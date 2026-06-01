//! Schedules: persistent rules that gate when a download is allowed to
//! start and when notifications are silenced.
//!
//! Three kinds share one table:
//!
//! - `start_at` (per-download) — the download stays out of the claim pass
//!   until `now >= start_iso`. `start_iso` is a full RFC3339 instant.
//! - `after_queue` (per-download) — the download is only picked when no
//!   other rows are in flight. Logically a low-priority bucket on top of
//!   the existing priority/created_at ordering.
//! - `quiet_hours` (global) — a recurring daily window during which
//!   completion / failure / queue-empty toasts and the tray badge are
//!   suppressed. Encoded as `start_iso` + `end_iso` = "HH:MM" local time
//!   plus a Mon..Sun `days_mask`. Downloads keep running by design.
//!
//! ## Cache
//!
//! [`SchedulesCache`] holds the in-memory copy used by hot paths (the
//! queue manager's `fill_capacity` and the notifications gate). It is
//! reloaded on construct and whenever `Core::*_schedule` mutates the
//! table. The queue manager does **not** reload per tick — the cache is
//! in-process, so the only drift source would be an external SQL writer
//! (out of scope for this app).

use std::collections::HashSet;

use chrono::{DateTime, Datelike, Local, NaiveTime, TimeZone, Utc, Weekday};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::download::DownloadId;
use crate::error::{CoreError, Result};

/// Database id for a schedule row.
pub type ScheduleId = i64;

/// Discriminant for the `schedules.kind` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleKind {
    StartAt,
    AfterQueue,
    QuietHours,
}

impl ScheduleKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ScheduleKind::StartAt => "start_at",
            ScheduleKind::AfterQueue => "after_queue",
            ScheduleKind::QuietHours => "quiet_hours",
        }
    }
}

impl std::str::FromStr for ScheduleKind {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "start_at" => ScheduleKind::StartAt,
            "after_queue" => ScheduleKind::AfterQueue,
            "quiet_hours" => ScheduleKind::QuietHours,
            other => {
                return Err(CoreError::InvalidArgument(format!(
                    "unknown schedule kind: {other}"
                )))
            }
        })
    }
}

/// Persisted view of one schedule row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub id: ScheduleId,
    pub kind: ScheduleKind,
    pub download_id: Option<DownloadId>,
    /// RFC3339 for `start_at`; `"HH:MM"` local for `quiet_hours`; `None`
    /// for `after_queue`.
    pub start_iso: Option<String>,
    /// `"HH:MM"` local for `quiet_hours`; `None` for other kinds.
    pub end_iso: Option<String>,
    /// Bit 0 = Mon … bit 6 = Sun.
    pub days_mask: u8,
    pub active: bool,
    pub created_at: DateTime<Utc>,
}

/// Inputs for `Core::add_schedule` / `Core::update_schedule`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewSchedule {
    pub kind: ScheduleKind,
    pub download_id: Option<DownloadId>,
    pub start_iso: Option<String>,
    pub end_iso: Option<String>,
    /// `None` → 127 (every day).
    pub days_mask: Option<u8>,
    /// `None` → true.
    pub active: Option<bool>,
}

/// Snapshot returned by `Core::quiet_hours_state` and the corresponding
/// Tauri command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuietHoursState {
    pub active: bool,
    /// RFC3339 instant when the active window ends. `None` when not
    /// currently active.
    pub until: Option<String>,
}

/// In-memory copy of the `schedules` table plus the "already fired"
/// bookkeeping for `start_at` rows.
pub(crate) struct SchedulesCache {
    rows: Vec<Schedule>,
    /// `start_at` rows are normally removed by the worker that consumed
    /// them, but `is_runnable` also tracks consumed ids so a race between
    /// the reload + delete doesn't keep gating the download.
    consumed_start_at: HashSet<DownloadId>,
}

impl SchedulesCache {
    pub(crate) async fn load(pool: &SqlitePool) -> Result<Self> {
        let rows = list_all(pool).await?;
        Ok(Self {
            rows,
            consumed_start_at: HashSet::new(),
        })
    }

    pub(crate) async fn reload(&mut self, pool: &SqlitePool) -> Result<()> {
        self.rows = list_all(pool).await?;
        // Discard `consumed` entries that no longer have a row — keeps the
        // set small over the app's lifetime.
        let live: HashSet<DownloadId> = self
            .rows
            .iter()
            .filter(|r| r.kind == ScheduleKind::StartAt)
            .filter_map(|r| r.download_id)
            .collect();
        self.consumed_start_at.retain(|id| live.contains(id));
        Ok(())
    }

    pub(crate) fn all(&self) -> &[Schedule] {
        &self.rows
    }

    /// True when this download is allowed to run *right now*. Takes:
    ///
    /// - `now` (UTC) for `start_at` gating.
    /// - `active_is_empty` so the manager can defer `after_queue` rows
    ///   until the running set drains.
    pub(crate) fn is_runnable(
        &self,
        id: DownloadId,
        now: DateTime<Utc>,
        active_is_empty: bool,
    ) -> bool {
        // start_at gate: any active, unconsumed start_at row whose
        // start_iso is still in the future blocks the row.
        for r in &self.rows {
            if r.kind != ScheduleKind::StartAt
                || !r.active
                || r.download_id != Some(id)
                || self.consumed_start_at.contains(&id)
            {
                continue;
            }
            let Some(iso) = r.start_iso.as_deref() else {
                continue;
            };
            match DateTime::parse_from_rfc3339(iso) {
                Ok(t) if t.with_timezone(&Utc) > now => return false,
                _ => {}
            }
        }
        // after_queue gate: must wait until no other download is active.
        if !active_is_empty && self.is_after_queue(id) {
            return false;
        }
        true
    }

    /// True when `id` has an active `after_queue` row.
    pub(crate) fn is_after_queue(&self, id: DownloadId) -> bool {
        self.rows
            .iter()
            .any(|r| r.kind == ScheduleKind::AfterQueue && r.active && r.download_id == Some(id))
    }

    /// Mark a `start_at` row as already fired. The queue manager calls
    /// this after a successful claim so the cache stays consistent even
    /// before the row is removed from the table.
    pub(crate) fn mark_start_at_consumed(&mut self, id: DownloadId) {
        self.consumed_start_at.insert(id);
    }

    /// True when at least one active `quiet_hours` row covers `now`.
    /// `now` should be in the user's local timezone so the `days_mask`
    /// matches their wall clock.
    pub fn quiet_hours_active(&self, now: DateTime<Local>) -> bool {
        for r in &self.rows {
            if r.kind != ScheduleKind::QuietHours || !r.active {
                continue;
            }
            if row_covers_local(r, now) {
                return true;
            }
        }
        false
    }

    /// First future moment quiet hours clear. Returns `None` when no
    /// quiet-hours row currently covers `now`. The result is an inclusive
    /// upper bound suitable for "suppress until X" UI copy.
    pub fn quiet_hours_active_until(&self, now: DateTime<Local>) -> Option<DateTime<Local>> {
        let mut soonest: Option<DateTime<Local>> = None;
        for r in &self.rows {
            if r.kind != ScheduleKind::QuietHours || !r.active {
                continue;
            }
            if !row_covers_local(r, now) {
                continue;
            }
            let Some(end) = row_end_today_or_tomorrow(r, now) else {
                continue;
            };
            soonest = Some(match soonest {
                Some(t) => t.min(end),
                None => end,
            });
        }
        soonest
    }
}

/// Does `r`'s daily window cover `now` (local)? Handles midnight wrap and
/// `days_mask` in the user's TZ.
fn row_covers_local(r: &Schedule, now: DateTime<Local>) -> bool {
    let (Some(s), Some(e)) = (
        parse_hhmm(r.start_iso.as_deref()),
        parse_hhmm(r.end_iso.as_deref()),
    ) else {
        return false;
    };
    let today_bit = weekday_bit(now.weekday());
    let yesterday_bit = weekday_bit(now.weekday().pred());
    let now_t = now.time();
    if s <= e {
        // Same-day window: must be enabled today.
        (r.days_mask & today_bit) != 0 && now_t >= s && now_t < e
    } else {
        // Crosses midnight: from `s` until midnight counts for the day
        // marked by `today_bit`; from midnight until `e` counts for the
        // day after, which from `now`'s perspective is "yesterday's"
        // window. So either:
        //   - today's mask bit is set AND now >= s, OR
        //   - yesterday's mask bit is set AND now < e.
        ((r.days_mask & today_bit) != 0 && now_t >= s)
            || ((r.days_mask & yesterday_bit) != 0 && now_t < e)
    }
}

/// When does the window containing `now` end? Same calendar day for the
/// non-wrapping case; next calendar day for the midnight-wrap case. The
/// caller must already know the row covers `now`.
fn row_end_today_or_tomorrow(r: &Schedule, now: DateTime<Local>) -> Option<DateTime<Local>> {
    let s = parse_hhmm(r.start_iso.as_deref())?;
    let e = parse_hhmm(r.end_iso.as_deref())?;
    let today = now.date_naive();
    let end_today = Local.from_local_datetime(&today.and_time(e)).single()?;
    if s <= e {
        Some(end_today)
    } else if now.time() < e {
        // We're in the pre-noon tail of yesterday's window; ends today.
        Some(end_today)
    } else {
        // We're after `s` today; window ends tomorrow at `e`.
        let tomorrow = today.succ_opt()?;
        Local.from_local_datetime(&tomorrow.and_time(e)).single()
    }
}

fn parse_hhmm(s: Option<&str>) -> Option<NaiveTime> {
    let s = s?;
    // Accept "HH:MM" or "HH:MM:SS". Reject anything else.
    NaiveTime::parse_from_str(s, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(s, "%H:%M:%S"))
        .ok()
}

fn weekday_bit(d: Weekday) -> u8 {
    1 << match d {
        Weekday::Mon => 0,
        Weekday::Tue => 1,
        Weekday::Wed => 2,
        Weekday::Thu => 3,
        Weekday::Fri => 4,
        Weekday::Sat => 5,
        Weekday::Sun => 6,
    }
}

/// Read all schedule rows. Order is `created_at ASC` so callers that
/// short-circuit on first match see the oldest rule first.
pub(crate) async fn list_all(pool: &SqlitePool) -> Result<Vec<Schedule>> {
    let rows = sqlx::query(
        "SELECT id, kind, download_id, start_iso, end_iso, days_mask, active, created_at \
         FROM schedules ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await?;
    rows.iter().map(record_from_row).collect()
}

pub(crate) async fn insert(pool: &SqlitePool, input: NewSchedule) -> Result<ScheduleId> {
    validate_input(&input)?;
    let kind = input.kind.as_str();
    let days_mask = input.days_mask.unwrap_or(127) as i64;
    let active = input.active.unwrap_or(true);
    let row = sqlx::query(
        "INSERT INTO schedules (kind, download_id, start_iso, end_iso, days_mask, active) \
         VALUES (?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(kind)
    .bind(input.download_id)
    .bind(input.start_iso.as_deref())
    .bind(input.end_iso.as_deref())
    .bind(days_mask)
    .bind(active as i64)
    .fetch_one(pool)
    .await?;
    Ok(row.get("id"))
}

pub(crate) async fn update(pool: &SqlitePool, id: ScheduleId, input: NewSchedule) -> Result<()> {
    validate_input(&input)?;
    let kind = input.kind.as_str();
    let days_mask = input.days_mask.unwrap_or(127) as i64;
    let active = input.active.unwrap_or(true);
    let res = sqlx::query(
        "UPDATE schedules SET kind = ?, download_id = ?, start_iso = ?, \
                              end_iso = ?, days_mask = ?, active = ? \
         WHERE id = ?",
    )
    .bind(kind)
    .bind(input.download_id)
    .bind(input.start_iso.as_deref())
    .bind(input.end_iso.as_deref())
    .bind(days_mask)
    .bind(active as i64)
    .bind(id)
    .execute(pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(CoreError::InvalidArgument(format!(
            "schedule id {id} not found"
        )));
    }
    Ok(())
}

pub(crate) async fn remove(pool: &SqlitePool, id: ScheduleId) -> Result<()> {
    let res = sqlx::query("DELETE FROM schedules WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(CoreError::InvalidArgument(format!(
            "schedule id {id} not found"
        )));
    }
    Ok(())
}

/// Remove all `start_at` rows referencing a download — called by the
/// queue manager once a claim has fired so the gate doesn't trip again
/// after a manual pause + resume.
pub(crate) async fn delete_start_at_for(pool: &SqlitePool, id: DownloadId) -> Result<()> {
    sqlx::query("DELETE FROM schedules WHERE kind = 'start_at' AND download_id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

fn validate_input(input: &NewSchedule) -> Result<()> {
    match input.kind {
        ScheduleKind::StartAt => {
            input.download_id.ok_or_else(|| {
                CoreError::InvalidArgument("start_at requires a download_id".into())
            })?;
            let s = input.start_iso.as_deref().ok_or_else(|| {
                CoreError::InvalidArgument("start_at requires start_iso (RFC3339)".into())
            })?;
            DateTime::parse_from_rfc3339(s).map_err(|e| {
                CoreError::InvalidArgument(format!("start_at.start_iso is not RFC3339: {e}"))
            })?;
        }
        ScheduleKind::AfterQueue => {
            input.download_id.ok_or_else(|| {
                CoreError::InvalidArgument("after_queue requires a download_id".into())
            })?;
        }
        ScheduleKind::QuietHours => {
            if input.download_id.is_some() {
                return Err(CoreError::InvalidArgument(
                    "quiet_hours is global; download_id must be null".into(),
                ));
            }
            let s = input.start_iso.as_deref().ok_or_else(|| {
                CoreError::InvalidArgument("quiet_hours requires start_iso (HH:MM)".into())
            })?;
            let e = input.end_iso.as_deref().ok_or_else(|| {
                CoreError::InvalidArgument("quiet_hours requires end_iso (HH:MM)".into())
            })?;
            parse_hhmm(Some(s)).ok_or_else(|| {
                CoreError::InvalidArgument(format!("quiet_hours.start_iso must be HH:MM: {s}"))
            })?;
            parse_hhmm(Some(e)).ok_or_else(|| {
                CoreError::InvalidArgument(format!("quiet_hours.end_iso must be HH:MM: {e}"))
            })?;
        }
    }
    if let Some(mask) = input.days_mask {
        if mask == 0 {
            return Err(CoreError::InvalidArgument(
                "days_mask must include at least one day".into(),
            ));
        }
        if mask > 127 {
            return Err(CoreError::InvalidArgument(
                "days_mask is a 7-bit field (Mon..Sun)".into(),
            ));
        }
    }
    Ok(())
}

fn record_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Schedule> {
    let kind_str: String = row.get("kind");
    let kind: ScheduleKind = kind_str.parse()?;
    let created_at_str: String = row.get("created_at");
    let created_at = parse_created_at(&created_at_str)?;
    let active: i64 = row.get("active");
    let days_mask: i64 = row.get("days_mask");
    Ok(Schedule {
        id: row.get("id"),
        kind,
        download_id: row.get("download_id"),
        start_iso: row.get("start_iso"),
        end_iso: row.get("end_iso"),
        days_mask: days_mask.clamp(0, 255) as u8,
        active: active != 0,
        created_at,
    })
}

/// SQLite's `datetime('now')` default yields `YYYY-MM-DD HH:MM:SS` (no
/// `T`, no zone). Try RFC3339 first (in case a caller inserted that
/// shape) then fall back.
fn parse_created_at(s: &str) -> Result<DateTime<Utc>> {
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Ok(t.with_timezone(&Utc));
    }
    let naive = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .map_err(|e| CoreError::InvalidArgument(format!("bad created_at {s:?}: {e}")))?;
    Ok(Utc.from_utc_datetime(&naive))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, FixedOffset, Timelike};

    fn mk_quiet(start: &str, end: &str, days: u8) -> Schedule {
        Schedule {
            id: 1,
            kind: ScheduleKind::QuietHours,
            download_id: None,
            start_iso: Some(start.into()),
            end_iso: Some(end.into()),
            days_mask: days,
            active: true,
            created_at: Utc::now(),
        }
    }

    fn local_at(hour: u32, minute: u32, day_offset: i64) -> DateTime<Local> {
        let mut now = Local::now()
            .with_hour(hour)
            .unwrap()
            .with_minute(minute)
            .unwrap()
            .with_second(0)
            .unwrap()
            .with_nanosecond(0)
            .unwrap();
        if day_offset != 0 {
            now = now.checked_add_signed(Duration::days(day_offset)).unwrap();
        }
        now
    }

    #[test]
    fn quiet_hours_same_day_window() {
        let cache = SchedulesCache {
            rows: vec![mk_quiet("09:00", "17:00", 127)],
            consumed_start_at: HashSet::new(),
        };
        assert!(cache.quiet_hours_active(local_at(12, 0, 0)));
        assert!(!cache.quiet_hours_active(local_at(8, 59, 0)));
        // End is exclusive.
        assert!(!cache.quiet_hours_active(local_at(17, 0, 0)));
    }

    #[test]
    fn quiet_hours_midnight_wrap() {
        // 22:00 → 07:00 daily.
        let cache = SchedulesCache {
            rows: vec![mk_quiet("22:00", "07:00", 127)],
            consumed_start_at: HashSet::new(),
        };
        assert!(cache.quiet_hours_active(local_at(23, 30, 0)));
        assert!(cache.quiet_hours_active(local_at(2, 0, 0)));
        assert!(cache.quiet_hours_active(local_at(6, 59, 0)));
        assert!(!cache.quiet_hours_active(local_at(7, 0, 0)));
        assert!(!cache.quiet_hours_active(local_at(21, 59, 0)));
    }

    #[test]
    fn quiet_hours_days_mask_excludes_today() {
        // Mask = 0 for today: no day should match.
        let today_bit = weekday_bit(Local::now().weekday());
        let mask = !today_bit & 0x7F;
        let cache = SchedulesCache {
            rows: vec![mk_quiet("00:00", "23:59", mask)],
            consumed_start_at: HashSet::new(),
        };
        assert!(!cache.quiet_hours_active(local_at(12, 0, 0)));
    }

    #[test]
    fn is_runnable_blocks_future_start_at() {
        let future: DateTime<FixedOffset> = (Utc::now() + Duration::hours(1)).into();
        let cache = SchedulesCache {
            rows: vec![Schedule {
                id: 1,
                kind: ScheduleKind::StartAt,
                download_id: Some(42),
                start_iso: Some(future.to_rfc3339()),
                end_iso: None,
                days_mask: 127,
                active: true,
                created_at: Utc::now(),
            }],
            consumed_start_at: HashSet::new(),
        };
        assert!(!cache.is_runnable(42, Utc::now(), true));
    }

    #[test]
    fn is_runnable_admits_past_start_at() {
        let past: DateTime<FixedOffset> = (Utc::now() - Duration::seconds(5)).into();
        let cache = SchedulesCache {
            rows: vec![Schedule {
                id: 1,
                kind: ScheduleKind::StartAt,
                download_id: Some(42),
                start_iso: Some(past.to_rfc3339()),
                end_iso: None,
                days_mask: 127,
                active: true,
                created_at: Utc::now(),
            }],
            consumed_start_at: HashSet::new(),
        };
        assert!(cache.is_runnable(42, Utc::now(), true));
    }

    #[test]
    fn is_runnable_consumed_start_at_admits() {
        let future: DateTime<FixedOffset> = (Utc::now() + Duration::hours(1)).into();
        let mut cache = SchedulesCache {
            rows: vec![Schedule {
                id: 1,
                kind: ScheduleKind::StartAt,
                download_id: Some(42),
                start_iso: Some(future.to_rfc3339()),
                end_iso: None,
                days_mask: 127,
                active: true,
                created_at: Utc::now(),
            }],
            consumed_start_at: HashSet::new(),
        };
        assert!(!cache.is_runnable(42, Utc::now(), true));
        cache.mark_start_at_consumed(42);
        assert!(cache.is_runnable(42, Utc::now(), true));
    }

    #[test]
    fn is_runnable_after_queue_gates_on_active() {
        let cache = SchedulesCache {
            rows: vec![Schedule {
                id: 1,
                kind: ScheduleKind::AfterQueue,
                download_id: Some(7),
                start_iso: None,
                end_iso: None,
                days_mask: 127,
                active: true,
                created_at: Utc::now(),
            }],
            consumed_start_at: HashSet::new(),
        };
        // Other workers are running: defer.
        assert!(!cache.is_runnable(7, Utc::now(), false));
        // Nothing else active: claimable.
        assert!(cache.is_runnable(7, Utc::now(), true));
    }

    #[test]
    fn validate_start_at_requires_iso() {
        let input = NewSchedule {
            kind: ScheduleKind::StartAt,
            download_id: Some(1),
            start_iso: Some("not-a-date".into()),
            end_iso: None,
            days_mask: None,
            active: None,
        };
        assert!(validate_input(&input).is_err());
    }

    #[test]
    fn validate_quiet_hours_requires_global() {
        let input = NewSchedule {
            kind: ScheduleKind::QuietHours,
            download_id: Some(1),
            start_iso: Some("22:00".into()),
            end_iso: Some("07:00".into()),
            days_mask: None,
            active: None,
        };
        assert!(validate_input(&input).is_err());
    }

    #[test]
    fn validate_days_mask_must_have_at_least_one_day() {
        let input = NewSchedule {
            kind: ScheduleKind::QuietHours,
            download_id: None,
            start_iso: Some("22:00".into()),
            end_iso: Some("07:00".into()),
            days_mask: Some(0),
            active: None,
        };
        assert!(validate_input(&input).is_err());
    }
}
