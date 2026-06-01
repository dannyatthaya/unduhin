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

function refresh(): void {
  chrome.storage.local.get({ verboseLogging: false }, (items) => {
    verbose = items.verboseLogging === true;
  });
}

refresh();

chrome.storage.onChanged.addListener((changes, area) => {
  if (area === "local" && changes.verboseLogging) {
    verbose = changes.verboseLogging.newValue === true;
  }
});

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
