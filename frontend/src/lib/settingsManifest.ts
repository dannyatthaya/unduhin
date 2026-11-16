// Static index used by the settings search. Hand-maintained: every setting
// a row shows appears here, pointing at the same strings the row renders,
// so search matches what is on screen in the current language.
//
// Adding a setting: add an entry below, then make sure the `<SettingRow>`
// in its section uses the same `id`.

export type SettingsSectionKey =
  | "general"
  | "categories"
  | "behaviour"
  | "network"
  | "torrent"
  | "media"
  | "browser"
  | "about";

export interface SettingsIndexEntry {
  section: SettingsSectionKey;
  /** Stable anchor id; the SettingRow stamps this on `data-setting-id`. */
  id: string;
  /** i18n key of the row's label. */
  labelKey: string;
  /** i18n key of the row's description. */
  descKey: string;
  /** Extra English words people search by that the strings do not contain. */
  keywords: string[];
}

/** i18n keys of the section names. */
export const SECTION_LABEL_KEYS: Record<SettingsSectionKey, string> = {
  general: "settings.sectionGeneral",
  categories: "settings.sectionCategories",
  behaviour: "settings.sectionBehaviour",
  network: "settings.sectionNetwork",
  torrent: "settings.sectionTorrent",
  media: "settings.sectionMedia",
  browser: "settings.sectionBrowser",
  about: "settings.sectionAbout",
};

export const SECTION_ROUTES: Record<SettingsSectionKey, string> = {
  general: "/settings/general",
  categories: "/settings/categories",
  behaviour: "/settings/behaviour",
  network: "/settings/network",
  torrent: "/settings/torrent",
  media: "/settings/media",
  browser: "/settings/browser",
  about: "/settings/about",
};

/** Sidebar order; search results follow it. */
export const SECTION_ORDER: SettingsSectionKey[] = [
  "general",
  "categories",
  "behaviour",
  "network",
  "torrent",
  "media",
  "browser",
  "about",
];

/** `[row id suffix, label key suffix, keywords, description key suffix]`.
 *  The description key defaults to the label key plus "Desc". */
type Row = [id: string, label: string, keywords: string[], desc?: string];

const ROWS: Record<SettingsSectionKey, Row[]> = {
  general: [
    ["default-output-path", "fileDefaultFolder", ["directory", "path", "location", "files"]],
    ["always-ask-filename", "fileAlwaysAskFilename", ["rename", "prompt"]],
    ["delete-default-action", "removeAction", ["delete", "trash", "disk"]],
    ["global-speed-limit", "bandwidthLimit", ["bandwidth", "throttle", "cap"]],
    ["default-segments", "fileDefaultSegments", ["parallel", "threads", "segments", "parts"]],
    ["max-concurrent", "fileMaxConcurrent", ["queue", "limit", "concurrent", "simultaneous"]],
    ["theme", "themeLabel", ["appearance", "dark mode", "light mode", "colour", "color"], "themeDesc"],
    ["language", "languageLabel", ["i18n", "translation", "locale", "bahasa"], "languageDesc"],
    ["clear-temp-data", "clearTempData", ["temp", "cache", "cleanup", "scratch", "disk space"]],
    ["history", "historyLabel", ["export", "csv", "json", "clear", "list"]],
  ],
  categories: [
    ["list", "sectionCategories", ["folder", "rules", "icon", "extension", "sort"]],
  ],
  behaviour: [
    ["autostart", "startupAutostart", ["startup", "boot", "login", "launch"]],
    ["start-minimized", "startupMinimized", ["tray", "background", "minimized"]],
    ["close-behavior", "closeBehaviour", ["close", "quit", "exit", "minimize", "tray"]],
    ["confirm-on-quit", "confirmOnQuit", ["confirm", "quit", "exit"]],
    ["notify-complete", "notifyComplete", ["notification", "toast", "done"]],
    ["notify-fail", "notifyFail", ["notification", "toast", "error"]],
    ["notify-queue-empty", "notifyQueueEmpty", ["notification", "queue", "summary"]],
    ["quiet-hours", "quietUse", ["mute", "silence", "schedule", "do not disturb", "night"]],
  ],
  network: [
    ["connect-timeout", "connectTimeout", ["timeout", "tcp", "tls", "handshake"]],
    ["read-timeout", "readTimeout", ["timeout", "stall"]],
    ["max-retries", "maxRetries", ["retry", "fail", "attempts"]],
    ["retry-backoff", "retryBackoff", ["backoff", "retry", "jitter"]],
    ["user-agent", "cardUserAgent", ["ua", "header", "browser", "identify"]],
  ],
  torrent: [
    ["listen-port", "torrentListenPort", ["port", "incoming", "peers", "bittorrent"]],
    ["enable-dht", "torrentEnableDht", ["dht", "magnet", "trackerless", "peers"]],
    ["enable-upnp", "torrentEnableUpnp", ["upnp", "port", "forward", "nat", "router"]],
    ["max-peers", "torrentMaxPeers", ["peers", "connections", "limit"]],
    ["download-dir", "torrentDownloadDir", ["folder", "directory", "path", "location"]],
    ["seed-ratio", "torrentSeedRatio", ["seed", "ratio", "upload", "share"]],
  ],
  media: [
    ["ytdlp-status", "cardYtdlp", ["youtube", "vimeo", "tiktok", "video", "yt-dlp", "ytdlp"]],
    ["ytdlp-custom-path", "ytdlpBinaryPath", ["override", "path", "binary", "exe"]],
    ["probe-timeout", "ytdlpProbeTimeout", ["timeout", "probe", "detect"]],
    ["default-format", "ytdlpDefaultFormat", ["format", "quality", "selector", "preset"]],
    ["impersonate", "ytdlpImpersonate", ["impersonate", "browser", "cloudflare"]],
    ["ffmpeg-status", "cardFfmpeg", ["ffmpeg", "merge", "mux", "audio", "video"]],
    ["ffmpeg-custom-path", "ffmpegBinaryPath", ["override", "path", "binary", "exe"]],
  ],
  browser: [
    ["install-context-menu", "browserContextMenu", ["right-click", "menu", "extension"]],
    ["hide-shelf", "browserHideShelf", ["shelf", "bubble", "chrome"]],
    ["forward-cookies", "browserForwardCookies", ["cookies", "login", "session"]],
    ["watch-clipboard", "browserClipboardToggle", ["clipboard", "copy", "paste"]],
    ["min-size", "browserMinSize", ["size", "minimum", "small files"]],
  ],
  about: [
    ["update-channel", "aboutUpdateChannel", ["channel", "release", "beta", "stable", "update"]],
    ["update-check-on-startup", "aboutCheckOnStartup", ["update", "auto", "startup"]],
    ["send-crash-reports", "aboutCrashReports", ["telemetry", "crash", "privacy"]],
    ["send-usage-stats", "aboutUsageStats", ["telemetry", "usage", "stats", "privacy"]],
  ],
};

export const SETTINGS_INDEX: SettingsIndexEntry[] = SECTION_ORDER.flatMap((section) =>
  ROWS[section].map(([id, label, keywords, desc]) => ({
    section,
    id: `${section}/${id}`,
    labelKey: `settings.${label}`,
    descKey: `settings.${desc ?? `${label}Desc`}`,
    keywords,
  })),
);
