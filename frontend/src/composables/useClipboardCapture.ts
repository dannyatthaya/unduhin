// Opt-in clipboard watcher.
//
// Polls the OS clipboard every ~1.5 s. On a hit (HTTP(S) URL whose
// path tail matches the user's `fileTypes` allowlist), pushes a
// toast offering to capture the download. The toast carries a single
// "Capture" action; dismiss-or-ignore lets the toast time out and
// nothing happens.
//
// Gating:
//   - The `watch_clipboard` Tauri-canonical setting must be `true`.
//   - The `fileTypes` allowlist must be non-empty — an empty allowlist
//     would either be silent (nothing matches) or noisy (everything
//     matches), neither of which is useful as a default behavior.
//   - Already-prompted URLs are remembered for the rest of the session
//     so a sticky clipboard doesn't re-prompt every 1.5 s.
//
// Polling: Tauri's clipboard-manager plugin doesn't expose a `paste`
// event today, so polling is the canonical pattern. The interval is
// short enough that "I just copied this" feels live, but long enough
// that the OS clipboard read pressure is negligible.

import { onScopeDispose, watch } from "vue";
import { useI18n } from "vue-i18n";

import { readText } from "@tauri-apps/plugin-clipboard-manager";

import { useToast } from "@/composables/useToast";
import { useBrowserSettings } from "@/composables/useBrowserSettings";
import { useSettingsStore } from "@/stores/settings";
import { useDownloadsStore } from "@/stores/downloads";
import { matchClipboardPayload } from "@/lib/clipboardMatch";

/** Poll interval. 1.5 s feels responsive without busy-polling. */
const POLL_INTERVAL_MS = 1500;

/** Toast lifetime for the capture prompt. The user gets a few seconds
 *  to act; longer than the default info toast because the choice
 *  matters more than a status confirmation. */
const CAPTURE_TOAST_MS = 8000;

export function useClipboardCapture() {
  const settings = useSettingsStore();
  const browser = useBrowserSettings();
  const downloads = useDownloadsStore();
  const { push, pushAction } = useToast();
  const { t } = useI18n();

  let timer: ReturnType<typeof setInterval> | null = null;
  // Remember every clipboard payload we've already inspected during
  // this session so a sticky clipboard doesn't re-prompt every tick.
  // Bounded to the most recent N entries to keep the set small.
  const seen = new Set<string>();
  const SEEN_CAP = 32;

  function rememberSeen(url: string) {
    if (seen.has(url)) return;
    if (seen.size >= SEEN_CAP) {
      // Drop the oldest entry — Set preserves insertion order.
      const first = seen.values().next().value;
      if (first !== undefined) seen.delete(first);
    }
    seen.add(url);
  }

  function isEnabled(): boolean {
    const raw = settings.values["watch_clipboard"];
    return raw === true;
  }

  async function pollOnce() {
    if (!isEnabled()) return;
    const fileTypes = browser.view.value.fileTypes;
    if (!fileTypes || fileTypes.length === 0) return;
    let raw: string;
    try {
      raw = await readText();
    } catch {
      // Clipboard isn't available (no text payload, permission denied
      // mid-session, etc.). Silent — polling resumes on the next tick.
      return;
    }
    if (!raw) return;
    if (seen.has(raw)) return;
    const match = matchClipboardPayload(raw, fileTypes);
    if (!match) {
      // Still mark non-matches as seen so we don't re-evaluate the
      // same payload every 1.5 s. The set is bounded so this can't
      // grow without limit.
      rememberSeen(raw);
      return;
    }
    rememberSeen(raw);
    offerCapture(match.url);
  }

  function offerCapture(url: string) {
    pushAction(
      t("settings.clipboardCapturePrompt", { url: truncate(url, 64) }),
      {
        label: t("settings.clipboardCaptureAction"),
        run: () => {
          void downloads
            .add({
              url,
              filename: null,
              output_path: null,
              category_id: null,
              segments: null,
              priority: null,
            })
            .then(() => {
              push(t("settings.clipboardCaptureQueued"), "success");
            })
            .catch((err: unknown) => {
              const message =
                (err as { message?: string })?.message ??
                t("settings.clipboardCaptureFailed");
              push(message, "error");
            });
        },
      },
      "info",
      CAPTURE_TOAST_MS,
    );
  }

  function start() {
    if (timer) return;
    timer = setInterval(() => {
      void pollOnce();
    }, POLL_INTERVAL_MS);
  }

  function stop() {
    if (timer) {
      clearInterval(timer);
      timer = null;
    }
  }

  // React to the toggle: when the user turns on the watcher, start
  // polling immediately; when they turn it off, drop the timer and
  // forget the seen-set so a re-enable starts clean.
  watch(
    () => isEnabled(),
    (enabled) => {
      if (enabled) {
        start();
      } else {
        stop();
        seen.clear();
      }
    },
    { immediate: true },
  );

  onScopeDispose(() => {
    stop();
  });

  return { pollOnce };
}

function truncate(s: string, max: number): string {
  if (s.length <= max) return s;
  return `${s.slice(0, max - 1)}…`;
}
