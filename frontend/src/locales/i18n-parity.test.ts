// Shape parity: every key path in `en.json` must also exist in
// `id.json`, and vice versa. Catches the common drift where a string
// gets added to the English source-of-truth but the Indonesian
// translation never lands.

import { describe, expect, it } from "vitest";

import en from "./en.json";
import id from "./id.json";

type Json = string | number | boolean | null | { [key: string]: Json } | Json[];

function collectKeys(value: Json, prefix = ""): string[] {
  if (
    value === null ||
    typeof value !== "object" ||
    Array.isArray(value)
  ) {
    return [prefix];
  }
  const out: string[] = [];
  for (const [k, v] of Object.entries(value)) {
    const path = prefix ? `${prefix}.${k}` : k;
    out.push(...collectKeys(v as Json, path));
  }
  return out;
}

function collectStringValues(value: Json, prefix = ""): [string, string][] {
  if (typeof value === "string") return [[prefix, value]];
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return [];
  }
  const out: [string, string][] = [];
  for (const [k, v] of Object.entries(value)) {
    const path = prefix ? `${prefix}.${k}` : k;
    out.push(...collectStringValues(v as Json, path));
  }
  return out;
}

describe("i18n locale parity", () => {
  const enKeys = collectKeys(en as Json).sort();
  const idKeys = collectKeys(id as Json).sort();

  it("en.json and id.json have identical key sets", () => {
    const enSet = new Set(enKeys);
    const idSet = new Set(idKeys);
    const missingInId = enKeys.filter((k) => !idSet.has(k));
    const missingInEn = idKeys.filter((k) => !enSet.has(k));
    expect(missingInId, "keys in en.json missing from id.json").toEqual([]);
    expect(missingInEn, "keys in id.json missing from en.json").toEqual([]);
  });

  it("id.json has no empty string values", () => {
    const empty = collectStringValues(id as Json)
      .filter(([, v]) => v.trim() === "")
      .map(([k]) => k);
    expect(empty, "empty Indonesian translation values").toEqual([]);
  });

  it("en.json has no empty string values", () => {
    const empty = collectStringValues(en as Json)
      .filter(([, v]) => v.trim() === "")
      .map(([k]) => k);
    expect(empty, "empty English source-of-truth values").toEqual([]);
  });

  it("locked top-level namespace set is correct", () => {
    const expected = [
      "addUrl",
      "common",
      "detail",
      "downloads",
      "errors",
      "notify",
      "settings",
      "tray",
    ];
    expect(Object.keys(en).sort()).toEqual(expected);
    expect(Object.keys(id).sort()).toEqual(expected);
  });
});
