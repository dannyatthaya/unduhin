// When a download should start: now, at a set time, or once the queue is
// empty. Pure date helpers behind the Start control in New download and
// Schedule download, kept apart from Vue so they are unit-testable.

export type StartMode = "now" | "at" | "after_queue";

const MS_PER_MINUTE = 60_000;
const MINUTES_PER_HOUR = 60;
const MINUTES_PER_DAY = 24 * MINUTES_PER_HOUR;

function pad(n: number): string {
  return n.toString().padStart(2, "0");
}

/** `YYYY-MM-DD` in local time, the value of `<input type="date">`. */
export function toDateInput(date: Date): string {
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** `HH:MM` in local time, the value of `<input type="time">`. */
export function toTimeInput(date: Date): string {
  return `${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

/**
 * The local instant a date field and a time field describe together, or
 * null when either is empty or malformed.
 */
export function combineLocal(dateInput: string, timeInput: string): Date | null {
  const dateMatch = /^(\d{4})-(\d{2})-(\d{2})$/.exec(dateInput);
  const timeMatch = /^(\d{2}):(\d{2})$/.exec(timeInput);
  if (!dateMatch || !timeMatch) return null;
  const date = new Date(
    Number(dateMatch[1]),
    Number(dateMatch[2]) - 1,
    Number(dateMatch[3]),
    Number(timeMatch[1]),
    Number(timeMatch[2]),
    0,
    0,
  );
  return Number.isNaN(date.getTime()) ? null : date;
}

/** The same clock time one calendar day later. */
export function nextDay(date: Date): Date {
  const next = new Date(date);
  next.setDate(next.getDate() + 1);
  return next;
}

export interface TimeUntil {
  days: number;
  hours: number;
  minutes: number;
}

/**
 * Whole days, hours and minutes from `now` until `target`, rounded up to
 * the minute so "in 0 min" never shows for a time still ahead. Null when
 * the target is not in the future.
 */
export function timeUntil(target: Date, now: Date): TimeUntil | null {
  const ms = target.getTime() - now.getTime();
  if (ms <= 0) return null;
  const totalMinutes = Math.ceil(ms / MS_PER_MINUTE);
  return {
    days: Math.floor(totalMinutes / MINUTES_PER_DAY),
    hours: Math.floor((totalMinutes % MINUTES_PER_DAY) / MINUTES_PER_HOUR),
    minutes: totalMinutes % MINUTES_PER_HOUR,
  };
}

/** Whether two dates fall on the same local calendar day. */
export function isSameDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

/** A sensible first value for the time fields: the next full hour. */
export function defaultStartTime(now: Date): Date {
  const next = new Date(now);
  next.setMinutes(0, 0, 0);
  next.setHours(next.getHours() + 1);
  return next;
}
