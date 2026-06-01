// Domain-rules export/import schema.
//
// The serialised JSON is versioned so a future shape change doesn't
// silently swallow older exports. `version: 1` is the current shape.

import type { HostRule } from "@/types/wire";

export interface RuleExportV1 {
  readonly version: 1;
  readonly exportedAt: string;
  readonly blocked: HostRule[];
  readonly always: HostRule[];
}

export interface RuleExportInput {
  readonly blocked: HostRule[];
  readonly always: HostRule[];
}

export function serializeRuleExport(input: RuleExportInput): string {
  const payload: RuleExportV1 = {
    version: 1,
    exportedAt: new Date().toISOString(),
    blocked: input.blocked.map((r) => ({ ...r })),
    always: input.always.map((r) => ({ ...r })),
  };
  return JSON.stringify(payload, null, 2);
}

/**
 * Parse the JSON body of a previously-exported rules file. Throws an
 * `Error` with a user-readable message on schema violations; caller is
 * responsible for surfacing the message.
 */
export function parseRuleExport(text: string): RuleExportV1 {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch (err) {
    throw new Error(`Could not parse JSON: ${(err as Error).message}`);
  }
  if (!raw || typeof raw !== "object") {
    throw new Error("Expected a JSON object at the top level.");
  }
  const obj = raw as {
    version?: unknown;
    blocked?: unknown;
    always?: unknown;
  };
  if (obj.version !== 1) {
    throw new Error(`Unsupported export version: ${String(obj.version)}.`);
  }
  if (!Array.isArray(obj.blocked) || !Array.isArray(obj.always)) {
    throw new Error("Both `blocked` and `always` must be arrays.");
  }
  return {
    version: 1,
    exportedAt: typeof raw === "object" && raw && "exportedAt" in raw
      ? String((raw as { exportedAt: unknown }).exportedAt)
      : new Date().toISOString(),
    blocked: obj.blocked.map(toRule),
    always: obj.always.map(toRule),
  };
}

function toRule(raw: unknown, index: number): HostRule {
  if (!raw || typeof raw !== "object") {
    throw new Error(`Rule at index ${index} must be an object.`);
  }
  const obj = raw as { pattern?: unknown; addedAt?: unknown };
  if (typeof obj.pattern !== "string" || obj.pattern.length === 0) {
    throw new Error(`Rule at index ${index} is missing a string \`pattern\`.`);
  }
  const addedAt =
    typeof obj.addedAt === "number" && Number.isFinite(obj.addedAt)
      ? Math.max(0, Math.trunc(obj.addedAt))
      : 0;
  return { pattern: obj.pattern, addedAt };
}
