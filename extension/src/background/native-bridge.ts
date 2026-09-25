// Native messaging bridge.
//
// Single long-lived `chrome.runtime.connectNative` port, with:
//   - lazy connect on first `send()`
//   - exponential reconnect (1/2/4/8/16/30s capped) on `onDisconnect`
//   - retry budget: 5 attempts within any rolling 60s window — once
//     exhausted, go quiet and only retry on the next user action (any
//     fresh `send()` call resets the budget)
//   - 30s ping via `chrome.alarms`; if no `pong` within 5s, tear the
//     port down and reconnect
//   - status broadcasts via `chrome.runtime.sendMessage({ kind:
//     "bridge-status", status })` — popup/options listen for these
//
// The bridge intentionally keeps "is the native side healthy?" as a
// runtime concept rather than persisting to storage: when the service
// worker comes back from sleep it tries to send first, sees the failure,
// and reconciles. Persisted health would lie after a host crash.

import { log } from "../shared/log.js";
import type {
  BridgeStatus,
  BridgeStatusMessage,
  Inbound,
  Outbound,
} from "../shared/types.js";

const HEALTH_ALARM = "unduhin-bridge-ping";
const HEALTH_PERIOD_MIN = 0.5; // 30 seconds
const PONG_TIMEOUT_MS = 5_000;
const BACKOFF_MS = [1_000, 2_000, 4_000, 8_000, 16_000, 30_000] as const;
const RETRY_WINDOW_MS = 60_000;
const RETRY_BUDGET = 5;

export interface NativeBridge {
  /** Send a message; resolves with the host's reply for round-trip variants. */
  send(msg: Inbound): Promise<Outbound>;
  /** Cheap synchronous read used by the interceptor: only `connected` means "go". */
  isHealthy(): boolean;
  /** Returns current status without forcing a connect. */
  status(): BridgeStatus;
  /** Cancel any pending reconnect and close the port. */
  shutdown(): void;
}

/**
 * Listener invoked for any unsolicited Outbound frame that doesn't
 * match a pending request — `settingsChanged`, `handoffDecision`, and the
 * other app-initiated pushes in `UNSOLICITED_TYPES`.
 * The handler runs synchronously; long work should be
 * dispatched off-thread.
 */
export type UnsolicitedHandler = (msg: Outbound) => void;

/** Set of `Outbound.type` strings the bridge routes to `UnsolicitedHandler`
 *  instead of the FIFO reply queue. `handoffDecision` arrives
 *  unsolicited because the user response is async — the original
 *  `askHandoff` `send()` has already resolved by then. `extensionUpdated`
 *  is the app's push (connection greeting / post-sync broadcast) telling
 *  us the canonical folder now holds a different version.
 *
 *  Only types the app NEVER sends as a reply belong here. `settings` is the
 *  reply to `setSettings` / `getSettings`; listing it here once left that
 *  waiter at the head of the FIFO forever, so every later reply was handed
 *  to the request before it. The app follows each `setSettings` with a
 *  `settingsChanged` broadcast, which is what applies the new shape. */
export const UNSOLICITED_TYPES: ReadonlySet<Outbound["type"]> = new Set<Outbound["type"]>([
  "settingsChanged",
  "handoffDecision",
  "extensionUpdated",
  // Both halves of the link refresh are app-initiated pushes with no reply on
  // this port: `armRefresh` is fire-and-forget, and `refreshCredentials` is
  // answered by a *separate* `credentialsRefreshed` send. Routing them here
  // keeps them out of the reply FIFO, where they would hijack a pending ack.
  "armRefresh",
  "refreshCredentials",
]);

/** How long a request may wait for its reply before the port is torn down.
 *
 *  Replies are matched to requests purely by order, so one reply that never
 *  comes (the app hung, or dropped the connection after the host forwarded
 *  the frame) would otherwise stall the waiter forever and hand every later
 *  reply to the wrong request. Dropping the whole port rejects every waiter
 *  and starts the next connection with an empty queue.
 *
 *  Generous on purpose: the slowest legitimate round-trip is a yt-dlp probe,
 *  capped at 30 s by `ytdlp_probe_timeout_ms` plus a 3 s impersonation check. */
const REPLY_TIMEOUT_MS = 90_000;

/** Resolved on every reply except `pong` (which is consumed by the health check). */
interface PendingReply {
  readonly resolve: (msg: Outbound) => void;
  readonly reject: (err: Error) => void;
  /** Backstop for a reply that never arrives; see `REPLY_TIMEOUT_MS`. */
  readonly timer: ReturnType<typeof setTimeout>;
}

export function createNativeBridge(
  hostNameProvider: () => Promise<string>,
  onUnsolicited?: UnsolicitedHandler,
  /** Invoked right after a successful (re)connect, so callers can resync
   *  state the host may have missed while the port was down (e.g. settings
   *  edits). Runs synchronously; long work should be dispatched off-thread. */
  onConnected?: () => void,
): NativeBridge {
  let port: chrome.runtime.Port | null = null;
  let status: BridgeStatus = "disconnected";
  // FIFO of awaiters for non-ping replies. The native host responds in
  // order (confirmed) so a queue is correct without correlation
  // IDs. Ping/pong is handled separately because it can be in flight at
  // the same time as a user send.
  const replyQueue: PendingReply[] = [];
  let pongWaiter: ((msg: Outbound) => void) | null = null;
  let pongTimer: ReturnType<typeof setTimeout> | null = null;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let attempt = 0;
  // Sliding-window retry budget. Each `connect()` failure pushes a
  // timestamp; entries older than 60s are dropped before counting.
  const recentAttempts: number[] = [];
  let quiet = false; // exhausted budget → wait for the next user `send()`.

  function setStatus(next: BridgeStatus): void {
    if (status === next) return;
    status = next;
    const msg: BridgeStatusMessage = { kind: "bridge-status", status: next };
    // `sendMessage` rejects if no receiver is listening (popup closed).
    // That's normal — swallow it.
    try {
      chrome.runtime.sendMessage(msg).catch(() => {});
    } catch {
      // Channel can throw synchronously on Chrome shutdown.
    }
    log.info("bridge status →", next);
  }

  function rejectAllPending(err: Error): void {
    while (replyQueue.length > 0) {
      const next = replyQueue.shift();
      if (next) {
        clearTimeout(next.timer);
        next.reject(err);
      }
    }
    if (pongWaiter) {
      pongWaiter = null;
      if (pongTimer) {
        clearTimeout(pongTimer);
        pongTimer = null;
      }
    }
  }

  function teardown(reason: string): void {
    log.warn("bridge teardown:", reason);
    // `port.disconnect()` below does NOT fire our own `onDisconnect`, so
    // waiters on this port have to be failed here. Left pending, they would
    // sit at the head of the FIFO and swallow the next port's replies.
    rejectAllPending(new Error(`bridge torn down: ${reason}`));
    if (port) {
      try {
        port.disconnect();
      } catch {
        // Already disconnected — fine.
      }
      port = null;
    }
    if (pongTimer) {
      clearTimeout(pongTimer);
      pongTimer = null;
    }
    pongWaiter = null;
  }

  function scheduleReconnect(): void {
    if (quiet) {
      setStatus("disconnected");
      return;
    }
    if (reconnectTimer) return;
    const delay = BACKOFF_MS[Math.min(attempt, BACKOFF_MS.length - 1)] ?? 30_000;
    setStatus("reconnecting");
    log.info(`bridge reconnect in ${delay}ms (attempt ${attempt + 1})`);
    reconnectTimer = setTimeout(() => {
      reconnectTimer = null;
      attempt += 1;
      connect().catch((err) => {
        log.warn("reconnect failed", err);
        // `connect` already accounts for the retry budget and schedules
        // the next attempt if any budget remains.
      });
    }, delay);
  }

  function noteAttempt(): boolean {
    const now = Date.now();
    while (recentAttempts.length > 0) {
      const head = recentAttempts[0]!;
      if (head < now - RETRY_WINDOW_MS) recentAttempts.shift();
      else break;
    }
    recentAttempts.push(now);
    if (recentAttempts.length >= RETRY_BUDGET) {
      quiet = true;
      log.warn(
        `bridge gave up after ${RETRY_BUDGET} attempts in 60s — waiting for next user action`,
      );
      return false;
    }
    return true;
  }

  async function connect(): Promise<void> {
    if (port) return;
    const hostName = await hostNameProvider();
    log.info("bridge connectNative →", hostName);
    let opened: chrome.runtime.Port;
    try {
      opened = chrome.runtime.connectNative(hostName);
    } catch (err) {
      noteAttempt();
      throw err instanceof Error ? err : new Error(String(err));
    }

    opened.onMessage.addListener(handleMessage);
    opened.onDisconnect.addListener(handleDisconnect);
    port = opened;
    // Reset the attempt counter on every successful `connectNative` —
    // Chrome doesn't raise an error if the host isn't installed until
    // the first message round-trips, so we'll still see `onDisconnect`
    // fire and bump the counter then.
    attempt = 0;
    setStatus("connected");
    ensureHealthAlarm();
    // Let the owner resync anything the host missed while we were down.
    // Best-effort: a throwing callback must not break the connection.
    if (onConnected) {
      try {
        onConnected();
      } catch (err) {
        log.warn("bridge onConnected handler threw", err);
      }
    }
  }

  function handleMessage(raw: unknown): void {
    if (!raw || typeof raw !== "object" || !("type" in raw)) {
      log.warn("bridge: malformed message", raw);
      return;
    }
    const msg = raw as Outbound;
    if (msg.type === "pong") {
      if (pongTimer) {
        clearTimeout(pongTimer);
        pongTimer = null;
      }
      if (pongWaiter) {
        pongWaiter(msg);
        pongWaiter = null;
      }
      return;
    }
    // Unsolicited `settings` / `settingsChanged` frames come back
    // out of band from the Tauri pipe server (push-on-change). They
    // bypass the FIFO replyQueue so a pending `download` ack doesn't
    // get hijacked by a server-side broadcast.
    if (UNSOLICITED_TYPES.has(msg.type)) {
      if (onUnsolicited) {
        try {
          onUnsolicited(msg);
        } catch (err) {
          log.warn("unsolicited handler threw", err);
        }
      } else {
        log.debug("bridge: unsolicited frame with no handler", msg.type);
      }
      return;
    }
    const pending = replyQueue.shift();
    if (!pending) {
      log.warn("bridge: unexpected reply with no waiter", msg);
      return;
    }
    clearTimeout(pending.timer);
    pending.resolve(msg);
  }

  function handleDisconnect(): void {
    // `chrome.runtime.lastError` carries the reason on the SW global.
    // Reading it here clears the error state so other listeners don't
    // see a stale "Could not establish connection" later.
    const reason = chrome.runtime.lastError?.message ?? "host disconnected";
    noteAttempt();
    rejectAllPending(new Error(reason));
    teardown(reason);
    scheduleReconnect();
  }

  function ensureHealthAlarm(): void {
    chrome.alarms.get(HEALTH_ALARM, (existing) => {
      if (existing) return;
      chrome.alarms.create(HEALTH_ALARM, { periodInMinutes: HEALTH_PERIOD_MIN });
    });
  }

  // Health-check alarm runs even when the SW was just woken — that's
  // exactly the behaviour we want, so the bridge reconciles its state
  // on every wake.
  chrome.alarms.onAlarm.addListener((alarm) => {
    if (alarm.name !== HEALTH_ALARM) return;
    void pingHealth();
  });

  async function pingHealth(): Promise<void> {
    if (!port) {
      // Use the alarm as a heartbeat to reconnect when the budget
      // refilled (we left `quiet` true on exhaustion, but the next
      // user `send()` clears it — periodic recheck just nudges us
      // back into action sooner).
      if (!quiet && !reconnectTimer) scheduleReconnect();
      return;
    }
    if (pongWaiter) {
      // Previous ping never came back — tear down and retry.
      teardown("ping outstanding when next ping fired");
      scheduleReconnect();
      return;
    }
    try {
      port.postMessage({ type: "ping" } satisfies Inbound);
    } catch (err) {
      log.warn("ping postMessage threw", err);
      teardown("ping post failed");
      scheduleReconnect();
      return;
    }
    pongWaiter = () => {
      // No-op — the message handler already nulls the waiter.
    };
    pongTimer = setTimeout(() => {
      pongTimer = null;
      teardown("pong timeout");
      scheduleReconnect();
    }, PONG_TIMEOUT_MS);
  }

  async function send(msg: Inbound): Promise<Outbound> {
    // Any user-initiated send resets the quiet flag — the user wants us
    // to try again, so give them the full retry budget.
    if (quiet) {
      quiet = false;
      recentAttempts.length = 0;
    }
    if (!port) {
      await connect();
    }
    const live = port;
    if (!live) throw new Error("bridge: connect did not produce a port");
    return new Promise<Outbound>((resolve, reject) => {
      const pending: PendingReply = {
        resolve,
        reject,
        timer: setTimeout(() => {
          // Only meaningful while this port is still the live one; a
          // teardown in between has already rejected (and cleared) it.
          if (port !== live) return;
          teardown(`no reply to ${msg.type} within ${REPLY_TIMEOUT_MS}ms`);
          scheduleReconnect();
        }, REPLY_TIMEOUT_MS),
      };
      replyQueue.push(pending);
      try {
        live.postMessage(msg);
      } catch (err) {
        // postMessage threw — withdraw the waiter we just enqueued (it's
        // the last one) and reject. The disconnect handler will not also
        // fire on a sync throw.
        const idx = replyQueue.lastIndexOf(pending);
        if (idx >= 0) replyQueue.splice(idx, 1);
        clearTimeout(pending.timer);
        reject(err instanceof Error ? err : new Error(String(err)));
      }
    });
  }

  function isHealthy(): boolean {
    return status === "connected";
  }

  function shutdown(): void {
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
    rejectAllPending(new Error("bridge shutdown"));
    teardown("explicit shutdown");
    chrome.alarms.clear(HEALTH_ALARM);
    setStatus("disconnected");
  }

  return {
    send,
    isHealthy,
    status: () => status,
    shutdown,
  };
}
