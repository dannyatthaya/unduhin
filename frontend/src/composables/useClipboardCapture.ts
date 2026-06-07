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
import { matchClipboardPayload, type ClipboardMatch } from "@/lib/clipboardMatch";
import type { TorrentMeta } from "@/types/tauri-bindings";

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
    // Magnets are downloads by definition, so they're capturable even with an
    // empty file-type allowlist (which only gates http(s) tail extensions).
    // Pass the allowlist through unchanged — `matchClipboardPayload` short-
    // circuits magnets before the allowlist gate and an empty list naturally
    // rejects every http URL, preserving the prior "empty = silent" behavior
    // for direct-file links.
    const fileTypes = browser.view.value.fileTypes ?? [];
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
    offerCapture(match);
  }

  function offerCapture(match: ClipboardMatch) {
    pushAction(
      t("settings.clipboardCapturePrompt", { url: truncate(match.url, 64) }),
      {
        label: t("settings.clipboardCaptureAction"),
        run: () => {
          const added =
            match.kind === "magnet"
              ? captureMagnet(match.url, match.infoHash)
              : downloads.add({
                  url: match.url,
                  filename: null,
                  output_path: null,
                  category_id: null,
                  segments: null,
                  priority: null,
                });
          void added
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

  /**
   * Capture a magnet straight from the clipboard without an add-time file
   * picker: `selected_files: null` means "all files" (the librqbit default),
   * and the file list / display name are reconciled by the backend once
   * librqbit resolves metadata. The info-hash (when recoverable) is the
   * de-dup key; the backend re-derives it from the magnet if we pass `""`.
   */
  function captureMagnet(uri: string, infoHash: string | null) {
    const torrent: TorrentMeta = {
      info_hash: infoHash ?? "",
      source: { kind: "magnet", uri },
      selected_files: null,
      files: null,
      swarm: null,
    };
    return downloads.add({
      url: uri,
      filename: null,
      output_path: null,
      category_id: null,
      segments: null,
      priority: null,
      kind: "torrent",
      torrent,
    });
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
