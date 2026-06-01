// Unit-test table for `shouldIntercept`. 12 rows covering each branch:
// size threshold, allowlist + blocklist precedence, host rules, non-HTTP
// schemes, and the global-enable gate.
//
// `intercept-rules.ts` has no `chrome.*` imports so the test runs in
// vanilla Node — no jsdom, no chrome mock.

import { describe, expect, it } from "vitest";

import {
  shouldIntercept,
  type InterceptInputs,
} from "../src/background/intercept-rules.js";
import type { Settings } from "../src/shared/types.js";

// Mirrored from `shared/settings.ts` so the test doesn't transitively
// import `shared/log.ts` (which hits `chrome.storage.local` at module
// load and would crash under vitest's vanilla Node runtime).
const DEFAULT_SETTINGS: Settings = {
  enabled: true,
  nativeHostName: "com.unduhin.host",
  minSizeMb: 1,
  extensionAllowlist: [],
  extensionBlocklist: ["html", "pdf", "txt", "json"],
  blockedHosts: [],
  alwaysInterceptHosts: [],
  detectHls: true,
  detectDash: true,
  verboseLogging: false,
  // Flat fields consumed by the mode gate and rules.
  mode: "catch-all",
  installContextMenu: true,
  hideShelf: true,
  forwardCookies: true,
  fileTypes: [],
};

interface MkOverrides {
  url?: string;
  filename?: string;
  size?: number;
  settings?: Partial<Settings>;
}

function rule(pattern: string, addedAt = 0): { pattern: string; addedAt: number } {
  return { pattern, addedAt };
}

function mk(overrides: MkOverrides): InterceptInputs {
  return {
    url: overrides.url ?? "https://example.com/file.zip",
    filename: overrides.filename ?? "file.zip",
    size: overrides.size ?? 5 * 1024 * 1024,
    settings: { ...DEFAULT_SETTINGS, ...overrides.settings },
  };
}

describe("shouldIntercept", () => {
  it("intercepts a vanilla 5MB zip with defaults", () => {
    expect(shouldIntercept(mk({}))).toEqual({ kind: "intercept" });
  });

  it("passthrough when extension is globally disabled", () => {
    const result = shouldIntercept(mk({ settings: { enabled: false } }));
    expect(result).toEqual({ kind: "passthrough", reason: "extension disabled" });
  });

  it("passthrough for blob: scheme", () => {
    const result = shouldIntercept(mk({ url: "blob:https://example.com/abc-123" }));
    expect(result.kind).toBe("passthrough");
  });

  it("passthrough for data: scheme", () => {
    const result = shouldIntercept(mk({ url: "data:application/zip;base64,UEsDBA==" }));
    expect(result.kind).toBe("passthrough");
  });

  it("passthrough for file: scheme", () => {
    const result = shouldIntercept(mk({ url: "file:///C:/Users/x/y.zip" }));
    expect(result.kind).toBe("passthrough");
  });

  it("passthrough when below min size", () => {
    const result = shouldIntercept(mk({ size: 100 * 1024 })); // 100 KB
    expect(result).toEqual({ kind: "passthrough", reason: "below min size" });
  });

  it("intercepts when size is unknown (-1)", () => {
    expect(shouldIntercept(mk({ size: -1 }))).toEqual({ kind: "intercept" });
  });

  it("passthrough when filename ext is in default blocklist", () => {
    const result = shouldIntercept(
      mk({ url: "https://example.com/page.html", filename: "page.html" }),
    );
    expect(result).toEqual({ kind: "passthrough", reason: "extension in blocklist" });
  });

  it("allowlist gates everything not on it", () => {
    const result = shouldIntercept(
      mk({
        filename: "movie.mkv",
        url: "https://example.com/movie.mkv",
        settings: { extensionAllowlist: ["zip", "iso"] },
      }),
    );
    expect(result).toEqual({ kind: "passthrough", reason: "extension not in allowlist" });
  });

  it("blocked host overrides everything", () => {
    const result = shouldIntercept(
      mk({
        url: "https://cdn.example.com/file.zip",
        settings: { blockedHosts: [rule("*.example.com")] },
      }),
    );
    expect(result).toEqual({
      kind: "passthrough",
      reason: "blocked host",
      matchedPattern: "*.example.com",
    });
  });

  it("always-intercept host bypasses size + blocklist", () => {
    // 100KB html file would normally passthrough on both size and
    // blocklist; always-intercept short-circuits both.
    const result = shouldIntercept(
      mk({
        url: "https://files.example.com/index.html",
        filename: "index.html",
        size: 100 * 1024,
        settings: { alwaysInterceptHosts: [rule("files.example.com")] },
      }),
    );
    expect(result).toEqual({
      kind: "intercept",
      matchedPattern: "files.example.com",
    });
  });

  it("wildcard always-intercept matches subdomains AND apex", () => {
    expect(
      shouldIntercept(
        mk({
          url: "https://drive.example.com/x.zip",
          settings: { alwaysInterceptHosts: [rule("*.example.com")] },
        }),
      ),
    ).toEqual({ kind: "intercept", matchedPattern: "*.example.com" });
    expect(
      shouldIntercept(
        mk({
          url: "https://example.com/x.zip",
          settings: { alwaysInterceptHosts: [rule("*.example.com")] },
        }),
      ),
    ).toEqual({ kind: "intercept", matchedPattern: "*.example.com" });
  });

  it("malformed URL falls through cleanly", () => {
    const result = shouldIntercept(mk({ url: "not a url" }));
    expect(result.kind).toBe("passthrough");
  });

  it("mode: passthrough → never intercepts even on a vanilla zip", () => {
    const result = shouldIntercept(mk({ settings: { mode: "passthrough" } }));
    expect(result).toEqual({
      kind: "passthrough",
      reason: "mode: passthrough",
    });
  });

  it("mode: rules-only without host match → passthrough", () => {
    const result = shouldIntercept(mk({ settings: { mode: "rules-only" } }));
    expect(result).toEqual({
      kind: "passthrough",
      reason: "mode: rules-only (no host match)",
    });
  });

  it("mode: rules-only with always-intercept host → intercept", () => {
    const result = shouldIntercept(
      mk({
        url: "https://files.example.com/x.zip",
        settings: {
          mode: "rules-only",
          alwaysInterceptHosts: [rule("files.example.com")],
        },
      }),
    );
    expect(result).toEqual({
      kind: "intercept",
      matchedPattern: "files.example.com",
    });
  });

  it("mode: ask-first without always-intercept → ask", () => {
    const result = shouldIntercept(mk({ settings: { mode: "ask-first" } }));
    expect(result).toEqual({ kind: "ask" });
  });

  it("mode: ask-first with always-intercept host → intercept (no prompt)", () => {
    const result = shouldIntercept(
      mk({
        url: "https://drive.example.com/x.zip",
        settings: {
          mode: "ask-first",
          alwaysInterceptHosts: [rule("drive.example.com")],
        },
      }),
    );
    expect(result).toEqual({
      kind: "intercept",
      matchedPattern: "drive.example.com",
    });
  });

  it("fileTypes allowlist gates everything not on it", () => {
    const result = shouldIntercept(
      mk({
        filename: "movie.mkv",
        url: "https://example.com/movie.mkv",
        settings: { fileTypes: ["zip", "iso"] },
      }),
    );
    expect(result).toEqual({
      kind: "passthrough",
      reason: "file type not in capture list",
    });
  });

  it("fileTypes hit allows the download through", () => {
    expect(
      shouldIntercept(
        mk({
          filename: "movie.mkv",
          url: "https://example.com/movie.mkv",
          settings: { fileTypes: ["mkv"] },
        }),
      ),
    ).toEqual({ kind: "intercept" });
  });
});
