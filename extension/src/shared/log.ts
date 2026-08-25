// Gated debug logger. Verbose output is opt-in via the `verboseLogging`
// settings toggle — until then the toggle lives in
// `chrome.storage.local.verboseLogging` so the SW can flip it at runtime
// from the devtools console without a settings page:
//
//   chrome.storage.local.set({ verboseLogging: true })
//
// Errors and warnings always print. Info / debug print only when the flag
// is true. The flag is cached in module scope so each log call doesn't
// hit `chrome.storage` — the cache reseeds via `chrome.storage.onChanged`.

let verbose = false;

// Reading the flag runs at import time, and this module is imported by
// modules that a unit test loads outside an extension — where there is no
// `chrome` at all. Bind against it defensively so importing a module that
// merely logs cannot throw. In the service worker `chrome` is always
// present, so this costs nothing there.
function bindToStorage(): void {
  const api = (globalThis as { chrome?: typeof chrome }).chrome;
  if (!api?.storage?.local) return;
  try {
    api.storage.local.get({ verboseLogging: false }, (items) => {
      verbose = items.verboseLogging === true;
    });
    api.storage.onChanged?.addListener((changes, area) => {
      if (area === "local" && changes.verboseLogging) {
        verbose = changes.verboseLogging.newValue === true;
      }
    });
  } catch {
    // A stubbed or partial `chrome` in a test. Logging still works; only
    // the verbose toggle is inert.
  }
}

bindToStorage();

function prefix(): string {
  return `[unduhin ${new Date().toISOString().slice(11, 23)}]`;
}

export const log = {
  error(...args: unknown[]): void {
    console.error(prefix(), ...args);
  },
  warn(...args: unknown[]): void {
    console.warn(prefix(), ...args);
  },
  info(...args: unknown[]): void {
    if (verbose) console.log(prefix(), ...args);
  },
  debug(...args: unknown[]): void {
    if (verbose) console.debug(prefix(), ...args);
  },
  isVerbose(): boolean {
    return verbose;
  },
};
