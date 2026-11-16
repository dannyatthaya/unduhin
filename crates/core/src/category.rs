//! Categories with extension-based auto-categorize rules.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::error::{CoreError, Result};

pub type CategoryId = i64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Category {
    pub id: CategoryId,
    pub name: String,
    pub icon: Option<String>,
    pub default_output_path: Option<PathBuf>,
    /// Lower-cased file extensions, no leading dot.
    pub extension_rules: Vec<String>,
    /// Named tint for the icon tile, one of [`CATEGORY_COLORS`]. `None` for
    /// rows predating the column; the UI then derives one from the icon.
    #[serde(default)]
    pub color: Option<String>,
}

/// The tints a category can carry. Names, not hex values: the UI maps each
/// to a light and a dark pair.
pub const CATEGORY_COLORS: &[&str] = &[
    "violet", "orange", "green", "blue", "pink", "yellow", "neutral",
];

/// Inputs to [`crate::Core::add_category`] / [`crate::Core::update_category`].
#[derive(Debug, Clone)]
pub struct NewCategory {
    pub name: String,
    pub icon: Option<String>,
    pub default_output_path: Option<PathBuf>,
    pub extension_rules: Vec<String>,
    /// One of [`CATEGORY_COLORS`], or `None` to leave the tint to the UI.
    pub color: Option<String>,
}

/// Reject a colour name the UI has no pair for, so the column never holds
/// free-form text.
fn validated_color(color: Option<&str>) -> Result<Option<&str>> {
    match color {
        None => Ok(None),
        Some(c) if CATEGORY_COLORS.contains(&c) => Ok(Some(c)),
        Some(other) => Err(CoreError::InvalidArgument(format!(
            "unknown category colour: {other}"
        ))),
    }
}

/// A category name must be unique ignoring case and surrounding space:
/// "video" and "Video" would be two rows the user cannot tell apart.
async fn ensure_name_free(pool: &SqlitePool, name: &str, except: Option<CategoryId>) -> Result<()> {
    let wanted = name.trim().to_lowercase();
    if wanted.is_empty() {
        return Err(CoreError::InvalidArgument(
            "a category needs a name".to_string(),
        ));
    }
    let taken = list(pool)
        .await?
        .into_iter()
        .any(|c| Some(c.id) != except && c.name.trim().to_lowercase() == wanted);
    if taken {
        return Err(name_taken(name));
    }
    Ok(())
}

fn name_taken(name: &str) -> CoreError {
    CoreError::InvalidArgument(format!(
        "there is already a category called {}",
        name.trim()
    ))
}

pub(crate) async fn list(pool: &SqlitePool) -> Result<Vec<Category>> {
    let rows = sqlx::query("SELECT * FROM categories ORDER BY display_order ASC, id ASC")
        .fetch_all(pool)
        .await?;
    rows.iter().map(category_from_row).collect()
}

pub(crate) async fn get(pool: &SqlitePool, id: CategoryId) -> Result<Category> {
    let row = sqlx::query("SELECT * FROM categories WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    let row = row.ok_or(CoreError::CategoryNotFound(id))?;
    category_from_row(&row)
}

pub(crate) async fn find_by_name(pool: &SqlitePool, name: &str) -> Result<Option<Category>> {
    let row = sqlx::query("SELECT * FROM categories WHERE name = ?")
        .bind(name)
        .fetch_optional(pool)
        .await?;
    row.as_ref().map(category_from_row).transpose()
}

pub(crate) async fn insert(pool: &SqlitePool, input: NewCategory) -> Result<CategoryId> {
    let color = validated_color(input.color.as_deref())?;
    ensure_name_free(pool, &input.name, None).await?;
    let exts = serde_json::to_string(&normalize_extensions(&input.extension_rules))?;
    let folder = input
        .default_output_path
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned());
    // `ensure_name_free` reads, then this writes. The `WHERE NOT EXISTS`
    // repeats the check inside the one statement, so two adds racing on the
    // same name cannot both land.
    let row = sqlx::query(
        "INSERT INTO categories \
         (name, icon, default_output_path, extension_rules, color, display_order) \
         SELECT ?, ?, ?, ?, ?, (SELECT COALESCE(MAX(display_order), 0) + 1 FROM categories) \
         WHERE NOT EXISTS \
         (SELECT 1 FROM categories WHERE lower(trim(name)) = lower(?)) \
         RETURNING id",
    )
    .bind(input.name.trim())
    .bind(&input.icon)
    .bind(folder)
    .bind(exts)
    .bind(color)
    .bind(input.name.trim())
    .fetch_optional(pool)
    .await?;
    let row = row.ok_or_else(|| name_taken(&input.name))?;
    Ok(row.get("id"))
}

/// Rewrite the `display_order` column to match the supplied order. The
/// supplied id set must equal the current id set, else this is rejected.
/// Runs in a single transaction so partial updates aren't observable.
pub(crate) async fn set_order(pool: &SqlitePool, ids: &[CategoryId]) -> Result<()> {
    let mut tx = pool.begin().await?;

    let current: Vec<CategoryId> = sqlx::query_scalar("SELECT id FROM categories")
        .fetch_all(&mut *tx)
        .await?;
    let mut want: Vec<CategoryId> = ids.to_vec();
    let mut have = current.clone();
    want.sort_unstable();
    have.sort_unstable();
    if want != have {
        return Err(CoreError::InvalidArgument(
            "id set does not match current categories".into(),
        ));
    }

    for (index, id) in ids.iter().enumerate() {
        sqlx::query("UPDATE categories SET display_order = ? WHERE id = ?")
            .bind(index as i64 + 1)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(())
}

pub(crate) async fn update(pool: &SqlitePool, id: CategoryId, input: NewCategory) -> Result<()> {
    let color = validated_color(input.color.as_deref())?;
    let current = get(pool, id).await?;
    // The catch-all is found by its name (`remove`, auto-categorize), so a
    // rename would leave the app without one.
    if current.name == FALLBACK_NAME && input.name.trim() != FALLBACK_NAME {
        return Err(CoreError::InvalidArgument(format!(
            "the {FALLBACK_NAME} category cannot be renamed"
        )));
    }
    ensure_name_free(pool, &input.name, Some(id)).await?;
    let exts = serde_json::to_string(&normalize_extensions(&input.extension_rules))?;
    let folder = input
        .default_output_path
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned());
    // Same single-statement name guard as `insert`.
    let res = sqlx::query(
        "UPDATE categories SET name = ?, icon = ?, default_output_path = ?, \
         extension_rules = ?, color = ? WHERE id = ? AND NOT EXISTS \
         (SELECT 1 FROM categories WHERE id != ? AND lower(trim(name)) = lower(?))",
    )
    .bind(input.name.trim())
    .bind(&input.icon)
    .bind(folder)
    .bind(exts)
    .bind(color)
    .bind(id)
    .bind(id)
    .bind(input.name.trim())
    .execute(pool)
    .await?;
    if res.rows_affected() == 0 {
        // The row was there a moment ago (`get` above), so the name guard
        // is what stopped the write.
        return Err(name_taken(&input.name));
    }
    Ok(())
}

/// Delete a category. Its downloads are re-filed under the "Other" catch-all
/// in the same transaction, so nothing is left without a category; their
/// files on disk are not touched. The catch-all itself cannot be deleted.
pub(crate) async fn remove(pool: &SqlitePool, id: CategoryId) -> Result<()> {
    let target = get(pool, id).await?;
    if target.name == FALLBACK_NAME {
        return Err(CoreError::InvalidArgument(
            "the fallback category cannot be deleted".to_string(),
        ));
    }

    let mut tx = pool.begin().await?;
    let fallback_id = fallback_category_id(&mut tx).await?;
    sqlx::query("UPDATE downloads SET category_id = ? WHERE category_id = ?")
        .bind(fallback_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let res = sqlx::query("DELETE FROM categories WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    if res.rows_affected() == 0 {
        return Err(CoreError::CategoryNotFound(id));
    }
    tx.commit().await?;
    Ok(())
}

/// Id of the catch-all, putting it back when it is missing. An earlier
/// version let "Other" be renamed, so a database can be without one, and
/// the downloads of a deleted category still need somewhere to go.
async fn fallback_category_id(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>) -> Result<CategoryId> {
    let existing: Option<CategoryId> =
        sqlx::query_scalar("SELECT id FROM categories WHERE name = ?")
            .bind(FALLBACK_NAME)
            .fetch_optional(&mut **tx)
            .await?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let created: CategoryId = sqlx::query_scalar(
        "INSERT INTO categories (name, icon, extension_rules, display_order) \
         VALUES (?, 'other', '[]', (SELECT COALESCE(MAX(display_order), 0) + 1 FROM categories)) \
         RETURNING id",
    )
    .bind(FALLBACK_NAME)
    .fetch_one(&mut **tx)
    .await?;
    Ok(created)
}

/// Name of the seeded catch-all category.
const FALLBACK_NAME: &str = "Other";

/// Resolve a filename to a category id by extension match. Returns `None`
/// if no rules match — the caller falls back to "Other" (which always
/// exists from the seed migration).
pub(crate) async fn auto_categorize_for_filename(
    pool: &SqlitePool,
    filename: &str,
) -> Result<Option<CategoryId>> {
    let ext = match extract_extension(filename) {
        Some(e) => e,
        None => return Ok(other_category_id(pool).await.ok()),
    };
    let cats = list(pool).await?;
    for c in &cats {
        if c.extension_rules.iter().any(|r| r == &ext) {
            return Ok(Some(c.id));
        }
    }
    Ok(other_category_id(pool).await.ok())
}

/// One-time backfill of concrete per-category download folders, run at
/// startup right after migrations. Every category whose folder is still
/// unset gets `<Downloads>/<Name>` — plain `<Downloads>` for the "Other"
/// catch-all — so the settings UI shows real paths and downloads never
/// fall back to the process CWD (`C:\WINDOWS\system32` for an autostarted
/// app, where writes fail with os error 5).
///
/// Guarded by the `category_folders_seeded` settings flag so it runs
/// exactly once per database: a folder the user later clears on purpose
/// stays cleared instead of being re-filled on every launch.
pub(crate) async fn seed_default_folders(pool: &SqlitePool) -> Result<()> {
    let done: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'category_folders_seeded'")
            .fetch_optional(pool)
            .await?;
    if done.is_some() {
        return Ok(());
    }

    let downloads = crate::fallback_download_dir();
    for cat in list(pool).await? {
        let already_set = cat
            .default_output_path
            .as_ref()
            .is_some_and(|p| !p.as_os_str().is_empty());
        if already_set {
            continue;
        }
        let folder = if cat.name == "Other" {
            downloads.clone()
        } else {
            downloads.join(crate::download::sanitize_filename(&cat.name))
        };
        sqlx::query("UPDATE categories SET default_output_path = ? WHERE id = ?")
            .bind(folder.to_string_lossy().as_ref())
            .bind(cat.id)
            .execute(pool)
            .await?;
    }
    sqlx::query("INSERT INTO settings (key, value) VALUES ('category_folders_seeded', 'true')")
        .execute(pool)
        .await?;
    Ok(())
}

async fn other_category_id(pool: &SqlitePool) -> Result<CategoryId> {
    let row = sqlx::query("SELECT id FROM categories WHERE name = 'Other'")
        .fetch_optional(pool)
        .await?;
    let row = row.ok_or_else(|| CoreError::CategoryNameNotFound("Other".to_string()))?;
    Ok(row.get("id"))
}

fn category_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Category> {
    let exts: String = row.get("extension_rules");
    let exts: Vec<String> = serde_json::from_str(&exts)?;
    let folder: Option<String> = row.get("default_output_path");
    Ok(Category {
        id: row.get("id"),
        name: row.get("name"),
        icon: row.get("icon"),
        default_output_path: folder.map(PathBuf::from),
        extension_rules: exts,
        // `try_get`: a database that predates the column still reads.
        color: row.try_get::<Option<String>, _>("color").ok().flatten(),
    })
}

fn extract_extension(filename: &str) -> Option<String> {
    let dot = filename.rfind('.')?;
    let ext = &filename[dot + 1..];
    if ext.is_empty() || ext.contains('/') || ext.contains('\\') {
        None
    } else {
        Some(ext.to_lowercase())
    }
}

fn normalize_extensions(input: &[String]) -> Vec<String> {
    input
        .iter()
        .map(|e| e.trim_start_matches('.').to_lowercase())
        .filter(|e| !e.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqliteConnectOptions;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn fresh_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn set_order_rewrites_display_order() {
        let pool = fresh_pool().await;
        let initial = list(&pool).await.unwrap();
        assert!(initial.len() >= 2);

        let reversed: Vec<CategoryId> = initial.iter().rev().map(|c| c.id).collect();
        set_order(&pool, &reversed).await.unwrap();

        let after = list(&pool).await.unwrap();
        let after_ids: Vec<CategoryId> = after.iter().map(|c| c.id).collect();
        assert_eq!(after_ids, reversed);
    }

    #[tokio::test]
    async fn set_order_rejects_mismatched_id_set() {
        let pool = fresh_pool().await;
        let initial = list(&pool).await.unwrap();
        let mut ids: Vec<CategoryId> = initial.iter().map(|c| c.id).collect();
        ids.pop();
        let res = set_order(&pool, &ids).await;
        assert!(matches!(res, Err(CoreError::InvalidArgument(_))));
    }

    #[tokio::test]
    async fn insert_assigns_next_display_order() {
        let pool = fresh_pool().await;
        let before = list(&pool).await.unwrap();
        let max_before = before.len() as i64;
        let new_id = insert(
            &pool,
            NewCategory {
                name: "eBooks".into(),
                icon: Some("book".into()),
                default_output_path: None,
                extension_rules: vec!["epub".into(), "mobi".into()],
                color: Some("blue".into()),
            },
        )
        .await
        .unwrap();
        let order: i64 = sqlx::query_scalar("SELECT display_order FROM categories WHERE id = ?")
            .bind(new_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(order, max_before + 1);
    }

    fn new_category(name: &str, color: Option<&str>) -> NewCategory {
        NewCategory {
            name: name.into(),
            icon: Some("book".into()),
            default_output_path: None,
            extension_rules: vec![],
            color: color.map(str::to_string),
        }
    }

    #[tokio::test]
    async fn color_round_trips() {
        let pool = fresh_pool().await;

        let id = insert(&pool, new_category("eBooks", Some("blue")))
            .await
            .unwrap();

        assert_eq!(get(&pool, id).await.unwrap().color.as_deref(), Some("blue"));
    }

    #[tokio::test]
    async fn insert_rejects_an_unknown_color() {
        let pool = fresh_pool().await;

        let res = insert(&pool, new_category("eBooks", Some("#ff0000"))).await;

        assert!(matches!(res, Err(CoreError::InvalidArgument(_))));
    }

    #[tokio::test]
    async fn insert_rejects_a_name_that_differs_only_by_case() {
        let pool = fresh_pool().await;

        let res = insert(&pool, new_category("  video ", None)).await;

        assert!(matches!(res, Err(CoreError::InvalidArgument(_))));
    }

    #[tokio::test]
    async fn update_may_keep_its_own_name() {
        let pool = fresh_pool().await;
        let video = find_by_name(&pool, "Video").await.unwrap().unwrap();

        let res = update(&pool, video.id, new_category("Video", Some("violet"))).await;

        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn seeded_categories_get_a_color_from_their_icon() {
        let pool = fresh_pool().await;

        let video = find_by_name(&pool, "Video").await.unwrap().unwrap();

        assert_eq!(video.color.as_deref(), Some("violet"));
    }

    #[tokio::test]
    async fn remove_refiles_downloads_under_other() {
        let pool = fresh_pool().await;
        let video = find_by_name(&pool, "Video").await.unwrap().unwrap();
        let other = find_by_name(&pool, "Other").await.unwrap().unwrap();
        sqlx::query(
            "INSERT INTO downloads (url, filename, output_path, status, category_id, created_at) \
             VALUES ('https://example.com/a.mp4', 'a.mp4', '/tmp/a.mp4', 'completed', ?, \
             '2026-11-15T00:00:00Z')",
        )
        .bind(video.id)
        .execute(&pool)
        .await
        .unwrap();

        remove(&pool, video.id).await.unwrap();

        let filed: Option<i64> = sqlx::query_scalar("SELECT category_id FROM downloads")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(filed, Some(other.id));
    }

    #[tokio::test]
    async fn update_refuses_to_rename_the_fallback_category() {
        let pool = fresh_pool().await;
        let other = find_by_name(&pool, "Other").await.unwrap().unwrap();

        let res = update(&pool, other.id, new_category("Misc", None)).await;

        assert!(matches!(res, Err(CoreError::InvalidArgument(_))));
        assert!(find_by_name(&pool, "Other").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn update_may_change_the_fallback_category_but_not_its_name() {
        let pool = fresh_pool().await;
        let other = find_by_name(&pool, "Other").await.unwrap().unwrap();

        update(&pool, other.id, new_category("Other", Some("blue")))
            .await
            .unwrap();

        let after = get(&pool, other.id).await.unwrap();
        assert_eq!(after.color.as_deref(), Some("blue"));
    }

    #[tokio::test]
    async fn update_rejects_a_name_another_category_has() {
        let pool = fresh_pool().await;
        let video = find_by_name(&pool, "Video").await.unwrap().unwrap();
        let id = insert(&pool, new_category("Books", None)).await.unwrap();

        let res = update(&pool, id, new_category(" video ", None)).await;

        assert!(matches!(res, Err(CoreError::InvalidArgument(_))));
        assert_eq!(get(&pool, video.id).await.unwrap().name, "Video");
    }

    #[tokio::test]
    async fn remove_puts_the_fallback_back_when_an_old_version_renamed_it() {
        let pool = fresh_pool().await;
        let video = find_by_name(&pool, "Video").await.unwrap().unwrap();
        // What a database looks like after an older version renamed "Other".
        sqlx::query("UPDATE categories SET name = 'Misc' WHERE name = 'Other'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO downloads (url, filename, output_path, status, category_id, created_at) \
             VALUES ('https://example.com/a.mp4', 'a.mp4', '/tmp/a.mp4', 'completed', ?, \
             '2026-11-15T00:00:00Z')",
        )
        .bind(video.id)
        .execute(&pool)
        .await
        .unwrap();

        remove(&pool, video.id).await.unwrap();

        let other = find_by_name(&pool, "Other").await.unwrap().unwrap();
        let filed: Option<i64> = sqlx::query_scalar("SELECT category_id FROM downloads")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(filed, Some(other.id));
    }

    #[tokio::test]
    async fn remove_refuses_the_fallback_category() {
        let pool = fresh_pool().await;
        let other = find_by_name(&pool, "Other").await.unwrap().unwrap();

        let res = remove(&pool, other.id).await;

        assert!(matches!(res, Err(CoreError::InvalidArgument(_))));
    }

    #[test]
    fn extension_extraction() {
        assert_eq!(extract_extension("Foo.MP3").as_deref(), Some("mp3"));
        assert_eq!(extract_extension("archive.tar.gz").as_deref(), Some("gz"));
        assert_eq!(extract_extension("noext"), None);
        assert_eq!(extract_extension(".hidden").as_deref(), Some("hidden"));
    }

    #[test]
    fn extensions_normalized() {
        let v = normalize_extensions(&[".MP3".into(), "WAV".into(), "".into(), ".".into()]);
        assert_eq!(v, vec!["mp3", "wav"]);
    }

    /// First-run seeding fills every unset category folder with a concrete
    /// path under the user's Downloads directory — `Downloads/<Name>` for
    /// real categories, plain `Downloads` for the "Other" catch-all — and
    /// leaves folders the user already configured untouched.
    #[tokio::test]
    async fn seed_default_folders_fills_unset_categories() {
        let pool = fresh_pool().await;
        let downloads = crate::fallback_download_dir();

        // Pre-configure one category to prove seeding does not clobber it.
        let video = find_by_name(&pool, "Video").await.unwrap().unwrap();
        let custom = PathBuf::from("D:/My Videos");
        update(
            &pool,
            video.id,
            NewCategory {
                name: video.name.clone(),
                icon: video.icon.clone(),
                default_output_path: Some(custom.clone()),
                extension_rules: video.extension_rules.clone(),
                color: video.color.clone(),
            },
        )
        .await
        .unwrap();

        seed_default_folders(&pool).await.unwrap();

        for name in ["Documents", "Music", "Compressed", "Programs"] {
            let cat = find_by_name(&pool, name).await.unwrap().unwrap();
            assert_eq!(
                cat.default_output_path.as_deref(),
                Some(downloads.join(name).as_path()),
                "{name} should be seeded under Downloads"
            );
        }
        let other = find_by_name(&pool, "Other").await.unwrap().unwrap();
        assert_eq!(
            other.default_output_path.as_deref(),
            Some(downloads.as_path())
        );
        let video = find_by_name(&pool, "Video").await.unwrap().unwrap();
        assert_eq!(video.default_output_path, Some(custom));
    }

    /// Seeding runs exactly once: a folder the user clears afterwards stays
    /// cleared on the next startup instead of being re-filled.
    #[tokio::test]
    async fn seed_default_folders_runs_once() {
        let pool = fresh_pool().await;
        seed_default_folders(&pool).await.unwrap();

        let music = find_by_name(&pool, "Music").await.unwrap().unwrap();
        update(
            &pool,
            music.id,
            NewCategory {
                name: music.name.clone(),
                icon: music.icon.clone(),
                default_output_path: None,
                extension_rules: music.extension_rules.clone(),
                color: music.color.clone(),
            },
        )
        .await
        .unwrap();

        seed_default_folders(&pool).await.unwrap();
        let music = find_by_name(&pool, "Music").await.unwrap().unwrap();
        assert_eq!(
            music.default_output_path, None,
            "cleared folder must stay cleared"
        );
    }

    /// Regression for the yt-dlp finalize flow: an extensionless title
    /// falls into "Other" (matching what insert sees), but once yt-dlp
    /// has supplied the `.mp4`, the same filename routes to "Video".
    /// The queue worker uses this difference to flip the row's
    /// `category_id` on completion.
    #[tokio::test]
    async fn auto_categorize_routes_extensionless_to_other_and_mp4_to_video() {
        let pool = fresh_pool().await;
        let other = find_by_name(&pool, "Other").await.unwrap().unwrap();
        let video = find_by_name(&pool, "Video").await.unwrap().unwrap();

        let without_ext = auto_categorize_for_filename(&pool, "SECSUN Articulated Boom Lift")
            .await
            .unwrap();
        let with_mp4 = auto_categorize_for_filename(&pool, "SECSUN Articulated Boom Lift.mp4")
            .await
            .unwrap();

        assert_eq!(without_ext, Some(other.id));
        assert_eq!(with_mp4, Some(video.id));
        assert_ne!(without_ext, with_mp4);
    }
}
