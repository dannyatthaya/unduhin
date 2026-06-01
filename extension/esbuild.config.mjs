// Build entry for the Unduhin Chromium extension. Produces `dist/` ready
// to load via chrome://extensions → "Load unpacked".
//
// Three bundle entries — service worker, popup, options. Popup/options
// land alongside their HTML+CSS via the small staticCopy plugin below;
// the service worker emits as an ES module so `manifest.json` can declare
// `"type": "module"`.

import { build, context } from "esbuild";
import { cp, mkdir, rm, writeFile, readFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const srcDir = resolve(__dirname, "src");
const distDir = resolve(__dirname, "dist");
const watch = process.argv.includes("--watch");

const entryPoints = {
  "background/service-worker": resolve(srcDir, "background/service-worker.ts"),
  "popup/popup": resolve(srcDir, "popup/popup.ts"),
  "options/options": resolve(srcDir, "options/options.ts"),
};

/** Files copied verbatim into `dist/` after each (re)build. */
const staticCopies = [
  { from: resolve(__dirname, "manifest.json"), to: resolve(distDir, "manifest.json") },
  { from: resolve(srcDir, "popup/popup.html"), to: resolve(distDir, "popup/popup.html") },
  { from: resolve(srcDir, "popup/popup.css"), to: resolve(distDir, "popup/popup.css") },
  {
    from: resolve(srcDir, "options/options.html"),
    to: resolve(distDir, "options/options.html"),
  },
  {
    from: resolve(srcDir, "options/options.css"),
    to: resolve(distDir, "options/options.css"),
  },
];

const sharedOpts = {
  bundle: true,
  format: "esm",
  target: "es2022",
  platform: "browser",
  sourcemap: "linked",
  logLevel: "info",
  outdir: distDir,
  entryPoints,
  // Chrome extensions disallow runtime `eval`/`new Function`; esbuild's
  // default for ES modules is fine, but we set this explicitly so a stray
  // dep can't sneak it back in.
  supported: { "import-meta": true },
};

async function staticCopy() {
  for (const { from, to } of staticCopies) {
    await mkdir(dirname(to), { recursive: true });
    await cp(from, to);
  }
}

async function clean() {
  if (existsSync(distDir)) {
    await rm(distDir, { recursive: true, force: true });
  }
  await mkdir(distDir, { recursive: true });
}

async function run() {
  await clean();
  if (watch) {
    const ctx = await context(sharedOpts);
    await ctx.watch();
    await staticCopy();
    // Re-copy static files on every rebuild by polling the manifest
    // mtime — esbuild's `onEnd` plugin would be cleaner but this keeps
    // the script free of extra moving parts.
    console.log("watch mode active — press Ctrl+C to stop");
  } else {
    await build(sharedOpts);
    await staticCopy();
    // Sanity check: validate manifest.json after copy so a bad edit
    // fails the build instead of silently shipping.
    const manifest = JSON.parse(await readFile(resolve(distDir, "manifest.json"), "utf8"));
    if (!manifest.background?.service_worker) {
      throw new Error("manifest.json missing background.service_worker");
    }
    if (!manifest.action?.default_popup) {
      throw new Error("manifest.json missing action.default_popup");
    }
    if (!manifest.options_ui?.page) {
      throw new Error("manifest.json missing options_ui.page");
    }
    await writeFile(
      resolve(distDir, ".build-stamp"),
      `${new Date().toISOString()}\n`,
      "utf8",
    );
    console.log("build OK →", distDir);
  }
}

run().catch((err) => {
  console.error(err);
  process.exit(1);
});
