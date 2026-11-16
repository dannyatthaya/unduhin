import { describe, expect, test } from "vitest";

import {
  editDistance,
  matchesQuery,
  splitByQuery,
  suggestQuery,
  type SearchableSetting,
} from "./settingsSearch";

const SETTINGS: SearchableSetting[] = [
  { id: "general/global-speed-limit", texts: ["Limit total speed", "Shared by every active download.", "General", "bandwidth"] },
  { id: "torrent/listen-port", texts: ["Listen port", "Port for incoming peers.", "Torrent", "bittorrent"] },
];

describe("matchesQuery", () => {
  test("matches any text of the setting, ignoring case and outer spaces", () => {
    expect(matchesQuery(SETTINGS[0], "  SPEED ")).toBe(true);
    expect(matchesQuery(SETTINGS[0], "bandwidth")).toBe(true);
    expect(matchesQuery(SETTINGS[0], "peers")).toBe(false);
  });

  test("an empty query matches everything", () => {
    expect(matchesQuery(SETTINGS[1], "   ")).toBe(true);
  });
});

describe("editDistance", () => {
  test("counts inserts, deletes and swaps", () => {
    expect(editDistance("torent", "torrent", 2)).toBe(1);
    expect(editDistance("speed", "speed", 2)).toBe(0);
    expect(editDistance("sped", "seed", 2)).toBe(1);
  });

  test("gives up past the limit", () => {
    expect(editDistance("theme", "bandwidth", 2)).toBe(3);
    expect(editDistance("abcdef", "uvwxyz", 2)).toBe(3);
  });
});

describe("suggestQuery", () => {
  test("offers the closest word a setting contains", () => {
    expect(suggestQuery("torent", SETTINGS)).toBe("torrent");
    expect(suggestQuery("Sped", SETTINGS)).toBe("speed");
  });

  test("returns null when nothing is close or the query is too short", () => {
    expect(suggestQuery("zzzzzz", SETTINGS)).toBeNull();
    expect(suggestQuery("prt", SETTINGS)).toBeNull();
  });

  test("never suggests the query itself", () => {
    expect(suggestQuery("speed", SETTINGS)).not.toBe("speed");
  });
});

describe("splitByQuery", () => {
  test("flags every match and keeps the original casing", () => {
    expect(splitByQuery("Speed limit: total speed", "speed")).toEqual([
      { text: "Speed", match: true },
      { text: " limit: total ", match: false },
      { text: "speed", match: true },
    ]);
  });

  test("returns the text whole when there is no query or no match", () => {
    expect(splitByQuery("Theme", "")).toEqual([{ text: "Theme", match: false }]);
    expect(splitByQuery("Theme", "port")).toEqual([{ text: "Theme", match: false }]);
  });
});
