import { describe, expect, test } from "bun:test";

import { mergeDarwin } from "./merge-macos-manifest.mjs";

const windowsOnly = () => ({
  version: "1.2.3",
  notes: "release notes",
  pub_date: "2026-08-15T00:00:00Z",
  platforms: {
    "windows-x86_64": { signature: "WSIG", url: "https://x/setup.exe" },
  },
});

const args = {
  version: "1.2.3",
  signature: "MACSIG",
  url: "https://x/Unduhin.app.tar.gz",
};

describe("mergeDarwin", () => {
  test("adds both darwin keys pointing at the same universal artifact", () => {
    const out = mergeDarwin(windowsOnly(), args);
    // The updater looks up the running slice's arch, never "universal",
    // so both keys must exist and both must resolve.
    expect(out.platforms["darwin-aarch64"]).toEqual({
      signature: "MACSIG",
      url: args.url,
    });
    expect(out.platforms["darwin-x86_64"]).toEqual({
      signature: "MACSIG",
      url: args.url,
    });
  });

  test("leaves the windows entry untouched", () => {
    const out = mergeDarwin(windowsOnly(), args);
    expect(out.platforms["windows-x86_64"]).toEqual({
      signature: "WSIG",
      url: "https://x/setup.exe",
    });
    expect(out.version).toBe("1.2.3");
    expect(out.notes).toBe("release notes");
  });

  test("does not mutate its input", () => {
    const original = windowsOnly();
    mergeDarwin(original, args);
    expect(Object.keys(original.platforms)).toEqual(["windows-x86_64"]);
  });

  test("rejects a manifest from a different release", () => {
    expect(() => mergeDarwin(windowsOnly(), { ...args, version: "9.9.9" }))
      .toThrow(/does not match/);
  });

  test("refuses to drop the windows entry", () => {
    const noWindows = { version: "1.2.3", platforms: {} };
    expect(() => mergeDarwin(noWindows, args)).toThrow(/windows-x86_64/);
  });

  test("rejects an empty signature", () => {
    expect(() => mergeDarwin(windowsOnly(), { ...args, signature: "" }))
      .toThrow(/signature/);
  });

  test("is idempotent across a re-run", () => {
    const once = mergeDarwin(windowsOnly(), args);
    const twice = mergeDarwin(once, args);
    expect(twice).toEqual(once);
  });
});
