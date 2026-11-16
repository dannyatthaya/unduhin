import { describe, expect, test } from "vitest";

import {
  combineLocal,
  defaultStartTime,
  isSameDay,
  nextDay,
  timeUntil,
  toDateInput,
  toTimeInput,
} from "./startPlan";

describe("date and time fields", () => {
  test("formats a date as the date and time input values", () => {
    const date = new Date(2026, 10, 5, 7, 3);

    expect(toDateInput(date)).toBe("2026-11-05");
    expect(toTimeInput(date)).toBe("07:03");
  });

  test("combines a date field and a time field into one local instant", () => {
    const combined = combineLocal("2026-11-12", "23:00");

    expect(combined).toEqual(new Date(2026, 10, 12, 23, 0));
  });

  test("returns null when the time field is empty", () => {
    expect(combineLocal("2026-11-12", "")).toBeNull();
  });

  test("returns null when the date field is malformed", () => {
    expect(combineLocal("12/11/2026", "23:00")).toBeNull();
  });
});

describe("timeUntil", () => {
  const now = new Date(2026, 10, 12, 18, 5);

  test("splits the wait into hours and minutes", () => {
    const target = new Date(2026, 10, 12, 23, 0);

    expect(timeUntil(target, now)).toEqual({ days: 0, hours: 4, minutes: 55 });
  });

  test("counts whole days for a time on a later day", () => {
    const target = new Date(2026, 10, 14, 18, 35);

    expect(timeUntil(target, now)).toEqual({ days: 2, hours: 0, minutes: 30 });
  });

  test("rounds a partial minute up so a future time never reads as zero", () => {
    const target = new Date(now.getTime() + 20_000);

    expect(timeUntil(target, now)).toEqual({ days: 0, hours: 0, minutes: 1 });
  });

  test("returns null for a time that has already passed", () => {
    const target = new Date(2026, 10, 12, 17, 30);

    expect(timeUntil(target, now)).toBeNull();
  });
});

describe("day helpers", () => {
  test("nextDay keeps the clock time and crosses a month boundary", () => {
    const next = nextDay(new Date(2026, 10, 30, 17, 30));

    expect(next).toEqual(new Date(2026, 11, 1, 17, 30));
  });

  test("isSameDay ignores the time of day", () => {
    const morning = new Date(2026, 10, 12, 0, 1);
    const night = new Date(2026, 10, 12, 23, 59);

    expect(isSameDay(morning, night)).toBe(true);
    expect(isSameDay(morning, nextDay(morning))).toBe(false);
  });

  test("defaultStartTime is the next full hour", () => {
    const start = defaultStartTime(new Date(2026, 10, 12, 18, 5, 40));

    expect(start).toEqual(new Date(2026, 10, 12, 19, 0, 0));
  });
});
