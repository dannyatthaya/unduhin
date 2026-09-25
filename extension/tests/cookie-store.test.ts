// `storeIdForTab` is the pure half of the incognito fix: a cookie lookup on
// behalf of a tab must read that tab's store, or an incognito tab is served
// the regular session's cookies.

import { describe, expect, it } from "vitest";

import { storeIdForTab } from "../src/background/cookie-forwarder.js";

const stores = [
  { id: "0", tabIds: [1, 2, 3] }, // regular profile
  { id: "1", tabIds: [7, 9] }, // incognito
];

describe("storeIdForTab", () => {
  it("finds the incognito store for an incognito tab", () => {
    expect(storeIdForTab(stores, 9)).toBe("1");
  });

  it("finds the regular store for a regular tab", () => {
    expect(storeIdForTab(stores, 2)).toBe("0");
  });

  it("returns undefined for a tab no store lists", () => {
    expect(storeIdForTab(stores, 42)).toBeUndefined();
  });
});
