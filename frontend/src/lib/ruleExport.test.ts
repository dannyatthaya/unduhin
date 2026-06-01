// Schema-validation tests for the domain-rules
// import/export round-trip.

import { describe, expect, it } from "vitest";

import {
  parseRuleExport,
  serializeRuleExport,
} from "@/lib/ruleExport";

describe("serializeRuleExport / parseRuleExport", () => {
  it("round-trips a non-empty set of rules", () => {
    const body = serializeRuleExport({
      blocked: [{ pattern: "*.tracker.example", addedAt: 1_700_000_000_000 }],
      always: [{ pattern: "files.example.com", addedAt: 1_700_000_001_000 }],
    });
    const parsed = parseRuleExport(body);
    expect(parsed.version).toBe(1);
    expect(parsed.blocked).toEqual([
      { pattern: "*.tracker.example", addedAt: 1_700_000_000_000 },
    ]);
    expect(parsed.always).toEqual([
      { pattern: "files.example.com", addedAt: 1_700_000_001_000 },
    ]);
  });

  it("rejects a missing top-level array", () => {
    const body = JSON.stringify({ version: 1, blocked: [] });
    expect(() => parseRuleExport(body)).toThrow(/arrays/i);
  });

  it("rejects a wrong-type rule entry", () => {
    const body = JSON.stringify({
      version: 1,
      blocked: [{ pattern: 42, addedAt: 0 }],
      always: [],
    });
    expect(() => parseRuleExport(body)).toThrow(/pattern/);
  });

  it("rejects an unsupported version", () => {
    const body = JSON.stringify({ version: 2, blocked: [], always: [] });
    expect(() => parseRuleExport(body)).toThrow(/version/i);
  });

  it("rejects non-JSON input", () => {
    expect(() => parseRuleExport("definitely not json")).toThrow(/parse/i);
  });

  it("falls back to addedAt=0 when the field is absent", () => {
    const body = JSON.stringify({
      version: 1,
      blocked: [{ pattern: "legacy.example.com" }],
      always: [],
    });
    const parsed = parseRuleExport(body);
    expect(parsed.blocked[0]).toEqual({
      pattern: "legacy.example.com",
      addedAt: 0,
    });
  });
});
