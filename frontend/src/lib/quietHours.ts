// Maths for the quiet-hours editor. A window is two times of day in
// minutes; when the end is not after the start it runs past midnight. The
// backend stores them as "HH:MM" and a day mask (bit 0 = Monday).

export const MINUTES_PER_DAY = 24 * 60;
/** Handles on the track move in quarter hours. */
export const QUIET_STEP_MINUTES = 15;
export const ALL_DAYS_MASK = 0b1111111;

export const DEFAULT_QUIET_START = "22:00";
export const DEFAULT_QUIET_END = "07:00";

/** "22:05" → 1325. `null` for anything that is not a time of day. */
export function parseTime(value: string | null | undefined): number | null {
  const match = /^(\d{1,2}):(\d{2})$/.exec(value?.trim() ?? "");
  if (!match) return null;
  const hours = Number(match[1]);
  const minutes = Number(match[2]);
  if (hours > 23 || minutes > 59) return null;
  return hours * 60 + minutes;
}

/** 1325 → "22:05". Wraps, so 1440 is "00:00". */
export function formatTime(minutes: number): string {
  const wrapped = ((Math.round(minutes) % MINUTES_PER_DAY) + MINUTES_PER_DAY) % MINUTES_PER_DAY;
  const hh = String(Math.floor(wrapped / 60)).padStart(2, "0");
  const mm = String(wrapped % 60).padStart(2, "0");
  return `${hh}:${mm}`;
}

/** Snap to the track's step and keep inside one day (00:00 to 23:45). */
export function snapMinutes(minutes: number): number {
  const snapped = Math.round(minutes / QUIET_STEP_MINUTES) * QUIET_STEP_MINUTES;
  return Math.min(MINUTES_PER_DAY - QUIET_STEP_MINUTES, Math.max(0, snapped));
}

/** Whether the window ends on the day after it starts. */
export function wrapsMidnight(start: number, end: number): boolean {
  return end <= start;
}

/** Length of the window in minutes; equal times mean a full day. */
export function windowLength(start: number, end: number): number {
  return wrapsMidnight(start, end) ? MINUTES_PER_DAY - start + end : end - start;
}

export interface TrackBlock {
  /** Left edge and width as fractions of the 24-hour track. */
  left: number;
  width: number;
}

/** The window as one block, or two when it runs past midnight. */
export function trackBlocks(start: number, end: number): TrackBlock[] {
  const fraction = (minutes: number) => minutes / MINUTES_PER_DAY;
  if (!wrapsMidnight(start, end)) {
    return [{ left: fraction(start), width: fraction(end - start) }];
  }
  const blocks: TrackBlock[] = [];
  if (end > 0) blocks.push({ left: 0, width: fraction(end) });
  blocks.push({ left: fraction(start), width: fraction(MINUTES_PER_DAY - start) });
  return blocks;
}

export function hasDay(mask: number, dayIndex: number): boolean {
  return (mask & (1 << dayIndex)) !== 0;
}

export function toggleDay(mask: number, dayIndex: number): number {
  return (mask ^ (1 << dayIndex)) & ALL_DAYS_MASK;
}
