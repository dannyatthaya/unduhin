// Real-browser end-to-end test for the extension's capture decisions and the
// app's capture rescue.
//
//   bun scripts/e2e-browser.mjs
//
// Windows-only, like the rest of the release tooling. Needs Brave (or pass
// E2E_BROWSER=<path to a Chromium browser>), `cargo`, and a built native
// host (`cargo build -p unduhin-native-host --release`).
//
// Nothing here touches an installed Unduhin or your own browser profile:
//
//   app         the headless harness in src-tauri/tests/browser_e2e_harness.rs,
//               in-memory database, on its own pipe
//   native host a copy registered as com.unduhin.host.e2e (key removed on exit)
//   extension   a copy of extension/dist whose default host name is patched
//               to com.unduhin.host.e2e, so it can never reach the real app
//   browser     a throwaway profile under the temp directory
//   file host   a local server on 127.0.0.1
//
// Downloads the app "takes" always fail at the file host on purpose, so no
// file is ever written outside the temp directory.

import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const REPO = path.resolve(import.meta.dir, "..");
const HOST_NAME = "com.unduhin.host";
const E2E_HOST_NAME = "com.unduhin.host.e2e";
const BIG = 1_300_000; // over the 1 MiB capture threshold
const WAIT_MS = 30_000;

const work = fs.mkdtempSync(path.join(os.tmpdir(), "unduhin-e2e-"));
const downloads = path.join(work, "downloads");
const profile = path.join(work, "profile");
const extDir = path.join(work, "extension");
const hostDir = path.join(work, "host");
const harnessLog = path.join(work, "harness.jsonl");
const stopFile = path.join(work, "stop");
const pipeName = `\\\\.\\pipe\\unduhin-e2e-${process.pid}`;

const browserPath =
  process.env.E2E_BROWSER ??
  "C:\\Program Files\\BraveSoftware\\Brave-Browser\\Application\\brave.exe";
const registryKeys = [
  `HKCU\\Software\\BraveSoftware\\Brave-Browser\\NativeMessagingHosts\\${E2E_HOST_NAME}`,
  `HKCU\\Software\\Google\\Chrome\\NativeMessagingHosts\\${E2E_HOST_NAME}`,
  `HKCU\\Software\\Microsoft\\Edge\\NativeMessagingHosts\\${E2E_HOST_NAME}`,
];

const cleanups = [];
const results = [];

function log(...args) {
  console.log("[e2e]", ...args);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function waitFor(what, probe, timeoutMs = WAIT_MS) {
  const start = Date.now();
  for (;;) {
    const value = await probe();
    if (value) return value;
    if (Date.now() - start > timeoutMs) throw new Error(`timed out waiting for ${what}`);
    await sleep(200);
  }
}

// --- staging ----------------------------------------------------------------

function stageExtension() {
  const dist = path.join(REPO, "extension", "dist");
  if (!fs.existsSync(path.join(dist, "manifest.json"))) {
    throw new Error("extension/dist is missing — run `bun run build` in extension/");
  }
  fs.cpSync(dist, extDir, { recursive: true });
  // Point the copy's DEFAULT host name at the e2e host. A fresh profile
  // connects the moment the service worker boots, so this has to be baked
  // in: set afterwards, the first connection would already have gone to
  // the real host and pushed this profile's default settings into it.
  let patched = 0;
  const walk = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) walk(full);
      else if (entry.name.endsWith(".js")) {
        const text = fs.readFileSync(full, "utf8");
        const next = text.replaceAll(`"${HOST_NAME}"`, `"${E2E_HOST_NAME}"`);
        if (next !== text) {
          fs.writeFileSync(full, next);
          patched++;
        }
      }
    }
  };
  walk(extDir);
  if (patched === 0) throw new Error("no host-name literal found to patch in the extension copy");
  const leftover = fs
    .readFileSync(path.join(extDir, "background", "service-worker.js"), "utf8")
    .includes(`"${HOST_NAME}"`);
  if (leftover) throw new Error("the extension copy still names the production host");
}

function stageNativeHost() {
  const built = path.join(REPO, "target", "release", "unduhin-native-host.exe");
  if (!fs.existsSync(built)) {
    throw new Error("native host not built — run `cargo build -p unduhin-native-host --release`");
  }
  fs.mkdirSync(hostDir, { recursive: true });
  const exe = path.join(hostDir, "unduhin-native-host.exe");
  fs.copyFileSync(built, exe);
  const template = JSON.parse(
    fs.readFileSync(path.join(REPO, "src-tauri", "native-host", `${HOST_NAME}.json`), "utf8"),
  );
  const manifestPath = path.join(hostDir, `${E2E_HOST_NAME}.json`);
  fs.writeFileSync(
    manifestPath,
    JSON.stringify({
      name: E2E_HOST_NAME,
      description: "Unduhin native messaging host (e2e)",
      path: exe,
      type: "stdio",
      allowed_origins: template.allowed_origins,
    }),
  );
  for (const key of registryKeys) {
    const r = spawnSync("reg", ["add", key, "/ve", "/t", "REG_SZ", "/d", manifestPath, "/f"]);
    if (r.status !== 0) throw new Error(`reg add failed for ${key}`);
  }
  cleanups.push(() => {
    for (const key of registryKeys) spawnSync("reg", ["delete", key, "/f"]);
  });
}

function stageProfile() {
  fs.mkdirSync(path.join(profile, "Default"), { recursive: true });
  fs.mkdirSync(downloads, { recursive: true });
  fs.writeFileSync(
    path.join(profile, "Default", "Preferences"),
    JSON.stringify({
      download: { default_directory: downloads, prompt_for_download: false },
      savefile: { default_directory: downloads },
    }),
  );
}

// --- the app stand-in ---------------------------------------------------------

async function startHarness() {
  const child = spawn(
    "cargo",
    ["test", "-p", "unduhin-app", "--test", "browser_e2e_harness", "--", "--ignored", "--nocapture"],
    {
      cwd: REPO,
      env: {
        ...process.env,
        UNDUHIN_E2E_PIPE: pipeName,
        UNDUHIN_E2E_LOG: harnessLog,
        UNDUHIN_E2E_STOP: stopFile,
      },
      stdio: "ignore",
    },
  );
  cleanups.push(async () => {
    fs.writeFileSync(stopFile, "");
    await sleep(800);
    child.kill();
  });
  await waitFor("the app harness", () => appEvents().some((e) => e.event === "ready"), 600_000);
}

function appEvents() {
  if (!fs.existsSync(harnessLog)) return [];
  return fs
    .readFileSync(harnessLog, "utf8")
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line));
}

const appTook = (suffix) =>
  appEvents().some((e) => e.event === "added" && typeof e.url === "string" && e.url.endsWith(suffix));

// --- the file host ------------------------------------------------------------

const requests = []; // "METHOD /path"
const body = Buffer.alloc(BIG, 0x5a);

function file(name, type) {
  return new Response(body, {
    headers: {
      "Content-Type": type,
      "Content-Length": String(body.length),
      "Content-Disposition": `attachment; filename="${name}"`,
    },
  });
}

const countOf = (line) => requests.filter((r) => r === line).length;

function startFileHost() {
  // /handback/file.zip plays a host that only serves a real browser. The
  // only thing telling the browser's requests from the app's is their order:
  //   1. browser GET            → 200 (the copy the extension cancels)
  //   2. app GET                → 403
  //   3. app probe HEAD         → 403   (credential refresh re-probe)
  //   4. app probe ranged GET   → 403
  //   5. browser GET            → 200 (the hand-back)
  let handbackState = 0;
  const server = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    fetch(req) {
      const url = new URL(req.url);
      const line = `${req.method} ${url.pathname}`;
      requests.push(line);
      switch (url.pathname) {
        case "/":
          return new Response("<!doctype html><title>e2e</title><p>e2e</p>", {
            headers: { "Content-Type": "text/html" },
          });
        case "/handback/file.zip": {
          const state = handbackState;
          if (state === 0 && req.method === "GET") {
            handbackState = 1;
            return file("handback.zip", "application/zip");
          }
          if (state >= 1 && state < 3) {
            if (req.method === "HEAD") handbackState = 2;
            else if (state === 2) handbackState = 3;
            return new Response("forbidden", { status: 403 });
          }
          if (state === 3 && req.method === "GET") {
            // Exactly one hand-back. A second fetch would mean the browser's
            // copy was captured again, and must not complete in the app.
            handbackState = 4;
            return file("handback.zip", "application/zip");
          }
          return new Response("gone", { status: 404 });
        }
        case "/capture/file.zip":
        case "/img/attached.jpg": {
          // Serve the browser once so the extension sees a download, then
          // 404 the app so nothing is written outside the temp directory.
          if (countOf(line) === 1 && req.method === "GET") {
            return url.pathname.endsWith(".zip")
              ? file("capture.zip", "application/zip")
              : file("attached.jpg", "image/jpeg");
          }
          return new Response("gone", { status: 404 });
        }
        case "/form":
          return new Response(
            `<!doctype html><form id="f" method="post" action="/export"><input name="q" value="1"></form>`,
            { headers: { "Content-Type": "text/html" } },
          );
        case "/export":
          if (req.method !== "POST") return new Response("use POST", { status: 405 });
          return file("export.zip", "application/zip");
        case "/pic/photo.png":
          return file("photo.png", "image/png");
        case "/gallery":
          return new Response(`<!doctype html><img id="i" src="/img/shown.jpg">`, {
            headers: { "Content-Type": "text/html" },
          });
        case "/img/shown.jpg":
          // Inline, as a page's own picture is.
          return new Response(body, {
            headers: { "Content-Type": "image/jpeg", "Content-Length": String(body.length) },
          });
        default:
          return new Response("not found", { status: 404 });
      }
    },
  });
  cleanups.push(() => server.stop(true));
  return `http://127.0.0.1:${server.port}`;
}

// --- the browser ----------------------------------------------------------------

class Cdp {
  constructor(ws) {
    this.ws = ws;
    this.nextId = 1;
    this.pending = new Map();
    /** Console output of every session that had `Runtime.enable` called. */
    this.console = [];
    ws.addEventListener("message", (ev) => {
      const msg = JSON.parse(ev.data);
      if (msg.method === "Runtime.consoleAPICalled") {
        const text = msg.params.args
          .map((a) => (a.value !== undefined ? String(a.value) : (a.description ?? a.type)))
          .join(" ");
        this.console.push(`${msg.params.type}: ${text}`);
        return;
      }
      const waiter = msg.id ? this.pending.get(msg.id) : null;
      if (!waiter) return;
      this.pending.delete(msg.id);
      if (msg.error) waiter.reject(new Error(`${waiter.method}: ${msg.error.message}`));
      else waiter.resolve(msg.result);
    });
  }

  send(method, params = {}, sessionId) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject, method });
      this.ws.send(JSON.stringify({ id, method, params, sessionId }));
    });
  }

  async attach(targetId) {
    const { sessionId } = await this.send("Target.attachToTarget", { targetId, flatten: true });
    return sessionId;
  }

  async evaluate(sessionId, expression) {
    const res = await this.send(
      "Runtime.evaluate",
      { expression, awaitPromise: true, returnByValue: true },
      sessionId,
    );
    if (res.exceptionDetails) {
      throw new Error(`evaluate failed: ${res.exceptionDetails.exception?.description ?? expression}`);
    }
    return res.result.value;
  }
}

async function startBrowser() {
  if (!fs.existsSync(browserPath)) throw new Error(`browser not found at ${browserPath}`);
  const child = spawn(
    browserPath,
    [
      `--user-data-dir=${profile}`,
      `--load-extension=${extDir}`,
      "--disable-features=DisableLoadExtensionCommandLineSwitch",
      "--remote-debugging-port=0",
      "--no-first-run",
      "--no-default-browser-check",
      "--disable-sync",
      "about:blank",
    ],
    // The native host the browser spawns inherits this, which is what puts
    // it on the harness's pipe instead of the real app's.
    { env: { ...process.env, UNDUHIN_PIPE_NAME: pipeName }, stdio: "ignore", detached: false },
  );
  cleanups.push(async () => {
    child.kill();
    await sleep(1500);
  });
  const portFile = path.join(profile, "DevToolsActivePort");
  await waitFor("the browser's debugging port", () => fs.existsSync(portFile));
  await sleep(300);
  const [port, wsPath] = fs.readFileSync(portFile, "utf8").trim().split("\n");
  const ws = new WebSocket(`ws://127.0.0.1:${port}${wsPath}`);
  await new Promise((resolve, reject) => {
    ws.addEventListener("open", resolve);
    ws.addEventListener("error", () => reject(new Error("could not open the DevTools socket")));
  });
  return new Cdp(ws);
}

async function openPage(cdp, url) {
  const { targetId } = await cdp.send("Target.createTarget", { url });
  const session = await cdp.attach(targetId);
  await waitFor(`page ${url}`, async () => {
    // A new target starts on about:blank, which is "complete" too — wait
    // for the document we actually asked for.
    const state = await cdp
      .evaluate(session, "location.href + ' ' + document.readyState")
      .catch(() => "");
    return state === `${url} complete`;
  });
  return session;
}

/** A session on the extension's service worker, waking it if it is asleep. */
async function workerSession(cdp) {
  const target = await waitFor("the extension's service worker", async () => {
    const { targetInfos } = await cdp.send("Target.getTargets");
    return targetInfos.find(
      (t) => t.type === "service_worker" && t.url.endsWith("/background/service-worker.js"),
    );
  });
  const session = await cdp.attach(target.targetId);
  // Record the worker's log, and turn on its debug level, so a failing run
  // shows which rule decided each download.
  await cdp.send("Runtime.enable", {}, session);
  await cdp.evaluate(session, `chrome.storage.local.set({ verboseLogging: true })`);
  return { session, origin: new URL(target.url).origin };
}

/** How many copies of the staged e2e native host are running. */
function e2eHostProcesses() {
  const exe = path.join(hostDir, "unduhin-native-host.exe").replaceAll("'", "''");
  const r = spawnSync(
    "powershell",
    [
      "-NoProfile",
      "-Command",
      `@(Get-CimInstance Win32_Process -Filter "Name='unduhin-native-host.exe'" | ` +
        `Where-Object { $_.ExecutablePath -eq '${exe}' }).Count`,
    ],
    { encoding: "utf8" },
  );
  return Number.parseInt(r.stdout.trim(), 10);
}

const landed = (name) => fs.existsSync(path.join(downloads, name));

// --- scenarios ------------------------------------------------------------------

async function check(name, run) {
  try {
    await run();
    results.push({ name, ok: true });
    log("PASS", name);
  } catch (err) {
    results.push({ name, ok: false, error: err.message });
    log("FAIL", name, "—", err.message);
  }
}

async function main() {
  stageExtension();
  stageNativeHost();
  stageProfile();
  const base = startFileHost();
  log("work dir", work);
  log("file host", base);
  log("starting the app harness (first run compiles it)…");
  await startHarness();
  const cdp = await startBrowser();
  const worker = await workerSession(cdp);

  globalThis.e2eCdp = cdp;
  await check("the test profile is pointed at the e2e host, not the real app", async () => {
    // Give the service worker's boot-time connect a moment to finish; the
    // first capture below is what proves the bridge actually works.
    await sleep(4000);
    const hostName = await cdp.evaluate(
      worker.session,
      `chrome.storage.sync.get("settings").then((s) => s.settings?.nativeHostName ?? "default")`,
    );
    if (hostName === HOST_NAME) throw new Error("the profile is pointed at the production host");
  });

  await check("the extension opens exactly one native-host connection", async () => {
    const hosts = e2eHostProcesses();
    if (hosts !== 1) throw new Error(`${hosts} e2e native hosts are running, expected 1`);
  });

  // A navigation that turns into a download, as opening a link in a new tab
  // does. Each one gets its own tab: a tab whose navigation was just turned
  // into a (cancelled) download does not reliably take the next one.
  const go = (url) => cdp.send("Target.createTarget", { url });

  await check("a clicked download is captured by the app", async () => {
    await go(`${base}/capture/file.zip`);
    await waitFor("the app to take /capture/file.zip", () => appTook("/capture/file.zip"));
    await sleep(1500);
    if (landed("capture.zip")) throw new Error("the browser also kept its copy");
  });

  await check("a download the app cannot fetch is handed back to the browser", async () => {
    await go(`${base}/handback/file.zip`);
    await waitFor("the app to take /handback/file.zip", () => appTook("/handback/file.zip"));
    await waitFor("the hand-back to land in the browser", () => landed("handback.zip"));
    await sleep(2500);
    const gets = countOf("GET /handback/file.zip");
    const heads = countOf("HEAD /handback/file.zip");
    if (gets !== 4 || heads !== 1) {
      throw new Error(`unexpected request pattern: ${gets} GET, ${heads} HEAD`);
    }
    const again = appEvents().filter(
      (e) => e.event === "added" && e.url?.endsWith("/handback/file.zip"),
    ).length;
    if (again !== 1) throw new Error(`the hand-back was captured again (${again} rows)`);
  });

  await check("a form-POST download stays in the browser", async () => {
    const form = await openPage(cdp, `${base}/form`);
    await cdp.evaluate(form, `void document.getElementById("f").submit()`);
    await waitFor("export.zip to land in the browser", () => landed("export.zip"));
    if (appTook("/export")) throw new Error("the app took a POST download");
  });

  await check("a file type outside the default list stays in the browser", async () => {
    await go(`${base}/pic/photo.png`);
    await waitFor("photo.png to land in the browser", () => landed("photo.png"));
    if (appTook("/pic/photo.png")) throw new Error("the app took a .png with default settings");
  });

  // From here on, behave like an install that predates the default list:
  // an empty list means "every type", so only the save-as rule can keep an
  // image in the browser.
  await cdp.evaluate(
    worker.session,
    `chrome.storage.sync.get("settings").then(({ settings }) =>
       chrome.storage.sync.set({ settings: { ...settings, fileTypes: [] } }))`,
  );
  await sleep(1000);

  await check("an image the page only displays stays in the browser when saved", async () => {
    await openPage(cdp, `${base}/gallery`);
    // Stands in for "Save image as…", which no automation can click: a
    // browser-initiated download of the displayed image, with no navigation.
    await cdp.evaluate(
      worker.session,
      `chrome.downloads.download({ url: ${JSON.stringify(`${base}/img/shown.jpg`)} })`,
    );
    await waitFor("shown.jpg to land in the browser", () => landed("shown.jpg"));
    if (appTook("/img/shown.jpg")) throw new Error("the app took a saved image");
  });

  await check("an image the user navigated to is still captured", async () => {
    await go(`${base}/img/attached.jpg`);
    await waitFor("the app to take /img/attached.jpg", () => appTook("/img/attached.jpg"));
    await sleep(1500);
    if (landed("attached.jpg")) throw new Error("the browser also kept its copy");
  });
}

let failed = false;
try {
  await main();
} catch (err) {
  failed = true;
  log("ABORTED:", err.message);
} finally {
  for (const cleanup of cleanups.reverse()) {
    try {
      await cleanup();
    } catch {
      // Best effort: a cleanup that fails must not hide the result.
    }
  }
}

if (process.env.E2E_VERBOSE || failed || results.some((r) => !r.ok)) {
  for (const line of globalThis.e2eCdp?.console ?? []) log("  sw>", line);
}
log("requests seen by the file host:", JSON.stringify(requests));
log("app events:", JSON.stringify(appEvents()));
const bad = results.filter((r) => !r.ok);
log(`${results.length - bad.length}/${results.length} passed`);
if (failed || bad.length > 0 || results.length === 0) process.exit(1);
try {
  fs.rmSync(work, { recursive: true, force: true });
} catch {
  // The browser can hold the profile for a moment after exit.
}
