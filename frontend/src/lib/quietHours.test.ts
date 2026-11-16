import { describe, expect, test } from "vitest";

import {
  formatTime,
  hasDay,
  parseTime,
  snapMinutes,
  toggleDay,
  trackBlocks,
  windowLength,
  wrapsMidnight,
} from "./quietHours";

describe("parseTime / formatTime", () => {
  test("round-trips a time of day", () => {
    expect(parseTime("22:05")).toBe(1325);
    expect(formatTime(1325)).toBe("22:05");
    expect(parseTime("7:00")).toBe(420);
    expect(formatTime(420)).toBe("07:00");
  });

  test("rejects what is not a time", () => {
    expect(parseTime("24:00")).toBeNull();
    expect(parseTime("12:60")).toBeNull();
    expect(parseTime("soon")).toBeNull();
    expect(parseTime(null)).toBeNull();
  });

  test("formatTime wraps past midnight", () => {
    expect(formatTime(1440)).toBe("00:00");
    expect(formatTime(-15)).toBe("23:45");
  });
});

describe("snapMinutes", () => {
  test("snaps to quarter hours and stays inside the day", () => {
    expect(snapMinutes(1327)).toBe(1320);
    expect(snapMinutes(1333)).toBe(1335);
    expect(snapMinutes(-40)).toBe(0);
    expect(snapMinutes(5000)).toBe(1425);
  });
});

describe("window maths", () => {
  test("a night window wraps midnight and is two blocks", () => {
    const start = 22 * 60;
    const end = 7 * 60;
    expect(wrapsMidnight(start, end)).toBe(true);
    expect(windowLength(start, end)).toBe(9 * 60);
    expect(trackBlocks(start, end)).toEqual([
      { left: 0, width: 7 / 24 },
      { left: 22 / 24, width: 2 / 24 },
    ]);
  });

  test("a daytime window is one block", () => {
    const start = 13 * 60;
    const end = 15 * 60;
    expect(wrapsMidnight(start, end)).toBe(false);
    expect(windowLength(start, end)).toBe(120);
    expect(trackBlocks(start, end)).toEqual([{ left: 13 / 24, width: 2 / 24 }]);
  });

  test("a window ending at midnight has no empty leading block", () => {
    expect(trackBlocks(22 * 60, 0)).toEqual([{ left: 22 / 24, width: 2 / 24 }]);
  });

  test("equal times mean the whole day", () => {
    expect(windowLength(600, 600)).toBe(1440);
  });
});

describe("day mask", () => {
  test("bit 0 is Monday", () => {
    expect(hasDay(0b0011111, 0)).toBe(true);
    expect(hasDay(0b0011111, 5)).toBe(false);
  });

  test("toggleDay flips one day and returns a new mask", () => {
    expect(toggleDay(0b0011111, 5)).toBe(0b0111111);
    expect(toggleDay(0b0011111, 0)).toBe(0b0011110);
  });
});
