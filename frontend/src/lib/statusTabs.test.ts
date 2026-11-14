import { describe, expect, it } from "vitest";

import { countByTab, matchesTab, statusTabOf } from "@/lib/statusTabs";
import { ALL_STATUSES } from "@/types/tauri-bindings";

describe("statusTabOf", () => {
  it("puts failed and paused rows under Needs attention", () => {
    expect(statusTabOf("failed")).toBe("attention");
    expect(statusTabOf("paused")).toBe("attention");
  });

  it("treats the merge phase as still downloading", () => {
    expect(statusTabOf("active")).toBe("downloading");
    expect(statusTabOf("muxing")).toBe("downloading");
  });

  it("maps queued and completed to their own tabs", () => {
    expect(statusTabOf("queued")).toBe("queued");
    expect(statusTabOf("completed")).toBe("done");
  });

  it("leaves cancelled rows out of every tab except All", () => {
    expect(statusTabOf("cancelled")).toBeNull();
    expect(matchesTab("cancelled", "all")).toBe(true);
    expect(matchesTab("cancelled", "done")).toBe(false);
  });
});

describe("matchesTab", () => {
  it("matches every status under All", () => {
    for (const status of ALL_STATUSES) {
      expect(matchesTab(status, "all")).toBe(true);
    }
  });

  it("matches only the statuses of the chosen tab", () => {
    expect(matchesTab("paused", "attention")).toBe(true);
    expect(matchesTab("paused", "queued")).toBe(false);
  });
});

describe("countByTab", () => {
  it("counts every row under All and each row under at most one other tab", () => {
    const counts = countByTab([
      { status: "active" },
      { status: "muxing" },
      { status: "queued" },
      { status: "paused" },
      { status: "failed" },
      { status: "completed" },
      { status: "completed" },
      { status: "cancelled" },
    ]);

    expect(counts).toEqual({
      all: 8,
      attention: 2,
      downloading: 2,
      queued: 1,
      done: 2,
    });
  });

  it("returns zeroes for an empty list", () => {
    expect(countByTab([])).toEqual({
      all: 0,
      attention: 0,
      downloading: 0,
      queued: 0,
      done: 0,
    });
  });
});
