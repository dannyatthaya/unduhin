#!/usr/bin/env bun
/**
 * Add the macOS entries to an existing Tauri updater manifest.
 *
 * The Windows release is built locally by `scripts/release.ps1`, which
 * uploads a `latest-<channel>.json` carrying only `windows-x86_64`. The
 * macOS build runs later in CI, so it downloads that manifest, adds its
 * own keys, and re-uploads it. Windows tooling stays untouched.
 *
 * Two keys, not one. `darwin-universal` looks like the obvious choice for
 * a universal binary, but the updater never asks for it: the plugin builds
 * its lookup key from `cfg!(target_arch)` of the *running* slice, so the
 * same app asks for `darwin-aarch64` on Apple Silicon and `darwin-x86_64`
 * on Intel. Both keys therefore point at the same universal artifact with
 * the same signature.
 *
 * JavaScript rather than a shell script so it can be unit-tested on the
 * maintainer's Windows box; the macOS release path is otherwise only ever
 * exercised in CI.
 *
 * Usage:
 *   bun scripts/merge-macos-manifest.mjs <manifest.json> <version> <sig-file> <url>
 */

import { readFileSync, writeFileSync } from "node:fs";

/**
 * Insert the two darwin platform keys into a parsed manifest.
 *
 * Pure so the guards below are testable without touching the filesystem.
 * Throws rather than returning an error, because every failure here means
 * publishing a broken manifest.
 */
export function mergeDarwin(manifest, { version, signature, url }) {
  if (!manifest || typeof manifest !== "object") {
    throw new Error("manifest is not an object");
  }
  // Guard against merging into a manifest from a different release, which
  // would advertise a version whose artifacts do not exist.
  if (manifest.version !== version) {
    throw new Error(
      `manifest version ${manifest.version} does not match ${version}`,
    );
  }
  if (!signature) throw new Error("signature is empty");
  if (!url) throw new Error("url is empty");

  const platforms = { ...(manifest.platforms ?? {}) };
  platforms["darwin-aarch64"] = { signature, url };
  platforms["darwin-x86_64"] = { signature, url };

  // Dropping the Windows entry would silently break the updater for every
  // existing user, and nothing downstream would notice.
  if (!platforms["windows-x86_64"]?.url) {
    throw new Error("windows-x86_64 is missing from the merged manifest");
  }

  return { ...manifest, platforms };
}

// Run only when invoked directly, so the test can import `mergeDarwin`.
if (import.meta.main) {
  const [manifestPath, version, sigPath, url] = process.argv.slice(2);
  if (!manifestPath || !version || !sigPath || !url) {
    console.error(
      "usage: merge-macos-manifest.mjs <manifest.json> <version> <sig-file> <url>",
    );
    process.exit(1);
  }

  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  const signature = readFileSync(sigPath, "utf8").trim();
  const merged = mergeDarwin(manifest, { version, signature, url });

  writeFileSync(manifestPath, `${JSON.stringify(merged, null, 2)}\n`, "utf8");
  console.log(`merged darwin keys into ${manifestPath}`);
  for (const key of Object.keys(merged.platforms)) console.log(`  ${key}`);
}
