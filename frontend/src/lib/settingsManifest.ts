// Static index used by the settings search/filter. Hand-maintained:
// every visible setting in the Settings views appears here so the
// sidebar filter can match against label + description + keywords and
// jump to the row on click.
//
// Adding a setting: drop a new entry below, then make sure the
// `<SettingRow>` in its section component uses the same `id`.

export type SettingsSectionKey =
  | "general"
  | "categories"
  | "behaviour"
  | "network"
  | "media"
  | "browser"
  | "about";

export interface SettingsIndexEntry {
  section: SettingsSectionKey;
  /** Stable anchor id; the SettingRow stamps this on `data-setting-id`. */
  id: string;
  label: string;
  description: string;
  keywords?: string[];
  route: string;
}

export const SECTION_LABELS: Record<SettingsSectionKey, string> = {
  general: "General",
  categories: "Categories",
  behaviour: "Behaviour",
  network: "Network",
  media: "Media",
  browser: "Browser",
  about: "About",
};

export const SECTION_ROUTES: Record<SettingsSectionKey, string> = {
  general: "/settings/general",
  categories: "/settings/categories",
  behaviour: "/settings/behaviour",
  network: "/settings/network",
  media: "/settings/media",
  browser: "/settings/browser",
  about: "/settings/about",
};

export const SETTINGS_INDEX: SettingsIndexEntry[] = [
  // -- General -------------------------------------------------------------
  {
    section: "general",
    id: "general/default-output-path",
    label: "Default download folder",
    description: "Used when a category doesn't specify its own folder.",
    keywords: ["folder", "directory", "path", "location", "files"],
    route: SECTION_ROUTES.general,
  },
  {
    section: "general",
    id: "general/default-segments",
    label: "Default segment count",
    description:
      "How many parallel connections per download. More segments = faster on healthy servers, but rude on small ones.",
    keywords: ["parallel", "connections", "threads", "segments"],
    route: SECTION_ROUTES.general,
  },
  {
    section: "general",
    id: "general/max-concurrent",
    label: "Max concurrent downloads",
    description: "Anything past this number waits in queue until a slot frees up.",
    keywords: ["queue", "limit", "concurrent"],
    route: SECTION_ROUTES.general,
  },
  {
    section: "general",
    id: "general/global-speed-limit",
    label: "Global speed limit",
    description: "Caps the combined download rate across every active segment.",
    keywords: ["bandwidth", "throttle", "speed"],
    route: SECTION_ROUTES.general,
  },
  {
    section: "general",
    id: "general/theme",
    label: "Theme",
    description: "Light, dark, or follow the system.",
    keywords: ["appearance", "dark mode", "light mode", "colour"],
    route: SECTION_ROUTES.general,
  },
  {
    section: "general",
    id: "general/language",
    label: "Language",
    description: "Coming in a later release.",
    keywords: ["i18n", "translation", "locale"],
    route: SECTION_ROUTES.general,
  },

  // -- Categories ----------------------------------------------------------
  {
    section: "categories",
    id: "categories/list",
    label: "Categories",
    description: "Add, edit, reorder, and delete categories.",
    keywords: ["folder", "rules", "icon"],
    route: SECTION_ROUTES.categories,
  },
  {
    section: "categories",
    id: "categories/rules",
    label: "Auto-categorize rules",
    description:
      "If a download's file extension matches one of these, it's filed into that category.",
    keywords: ["extension", "match", "auto"],
    route: SECTION_ROUTES.categories,
  },

  // -- Behaviour -----------------------------------------------------------
  {
    section: "behaviour",
    id: "behaviour/autostart",
    label: "Launch at startup",
    description:
      "Start Unduhin when you sign in to Windows. Required if you want queued downloads to run unattended.",
    keywords: ["startup", "boot", "login"],
    route: SECTION_ROUTES.behaviour,
  },
  {
    section: "behaviour",
    id: "behaviour/start-minimized",
    label: "Start minimized to tray",
    description:
      "Skip showing the main window on startup. Persisted now; activates once the tray ships.",
    keywords: ["tray", "background"],
    route: SECTION_ROUTES.behaviour,
  },
  {
    section: "behaviour",
    id: "behaviour/close-behavior",
    label: "When I close the window",
    description:
      "What the × button does. Active downloads keep running either way.",
    keywords: ["close", "quit", "exit", "minimize"],
    route: SECTION_ROUTES.behaviour,
  },
  {
    section: "behaviour",
    id: "behaviour/confirm-on-quit",
    label: "Confirm before quitting with active downloads",
    description:
      'A short "3 downloads in progress — quit anyway?" dialog.',
    keywords: ["confirm", "quit"],
    route: SECTION_ROUTES.behaviour,
  },
  {
    section: "behaviour",
    id: "behaviour/notify-complete",
    label: "Notify when a download completes",
    description: 'Toast with the filename and an "Open folder" action.',
    keywords: ["notification", "toast", "complete"],
    route: SECTION_ROUTES.behaviour,
  },
  {
    section: "behaviour",
    id: "behaviour/notify-fail",
    label: "Notify when a download fails",
    description: 'Toast with the reason and a "Retry" action.',
    keywords: ["notification", "toast", "fail", "error"],
    route: SECTION_ROUTES.behaviour,
  },
  {
    section: "behaviour",
    id: "behaviour/notify-queue-empty",
    label: "Notify when the whole queue empties",
    description: "One summary toast after the last active download finishes.",
    keywords: ["notification", "queue", "summary"],
    route: SECTION_ROUTES.behaviour,
  },
  {
    section: "behaviour",
    id: "behaviour/quiet-hours",
    label: "Quiet hours",
    description:
      "Mute notifications during a recurring window. Coming in a later release.",
    keywords: ["mute", "silence", "schedule"],
    route: SECTION_ROUTES.behaviour,
  },

  // -- Network -------------------------------------------------------------
  {
    section: "network",
    id: "network/connect-timeout",
    label: "Connect timeout",
    description:
      "How long to wait for the initial TCP/TLS handshake before giving up on a host.",
    keywords: ["timeout", "tcp", "tls", "handshake"],
    route: SECTION_ROUTES.network,
  },
  {
    section: "network",
    id: "network/read-timeout",
    label: "Read timeout",
    description:
      "After connecting, how long a segment can stall with zero bytes before Unduhin retries.",
    keywords: ["timeout", "stall", "read"],
    route: SECTION_ROUTES.network,
  },
  {
    section: "network",
    id: "network/max-retries",
    label: "Max retries per segment",
    description:
      "A segment that fails this many times in a row is marked dead.",
    keywords: ["retry", "fail", "attempts"],
    route: SECTION_ROUTES.network,
  },
  {
    section: "network",
    id: "network/retry-backoff",
    label: "Retry backoff base",
    description:
      "Doubles after each failure with full jitter. Keep modest to avoid hammering servers.",
    keywords: ["backoff", "retry", "jitter"],
    route: SECTION_ROUTES.network,
  },
  {
    section: "network",
    id: "network/user-agent",
    label: "User-agent",
    description: "The identifier Unduhin sends with every HTTP request.",
    keywords: ["ua", "header", "browser", "identify"],
    route: SECTION_ROUTES.network,
  },

  // -- Media ---------------------------------------------------------------
  {
    section: "media",
    id: "media/ytdlp-status",
    label: "yt-dlp",
    description:
      "Install or update the yt-dlp binary used to download from YouTube, Vimeo, Twitter, TikTok and other media sites.",
    keywords: ["youtube", "vimeo", "twitter", "tiktok", "video", "media", "ytdlp"],
    route: SECTION_ROUTES.media,
  },
  {
    section: "media",
    id: "media/ytdlp-custom-path",
    label: "Custom yt-dlp path",
    description: "Point to an existing yt-dlp.exe instead of the managed install.",
    keywords: ["override", "path", "binary", "exe", "ytdlp"],
    route: SECTION_ROUTES.media,
  },
  {
    section: "media",
    id: "media/probe-timeout",
    label: "Probe timeout",
    description:
      "How long Unduhin waits for yt-dlp to recognize a URL before falling through to the regular engine.",
    keywords: ["timeout", "probe", "detect", "media"],
    route: SECTION_ROUTES.media,
  },
  {
    section: "media",
    id: "media/default-format",
    label: "Default format selector",
    description: "Default yt-dlp format expression used when the formats dialog opens.",
    keywords: ["format", "quality", "selector", "ytdlp", "preset"],
    route: SECTION_ROUTES.media,
  },
  {
    section: "media",
    id: "media/ffmpeg-status",
    label: "FFmpeg",
    description: "Install or update FFmpeg, used to combine separate video and audio streams.",
    keywords: ["ffmpeg", "merge", "mux", "audio", "video"],
    route: SECTION_ROUTES.media,
  },
  {
    section: "media",
    id: "media/ffmpeg-custom-path",
    label: "Custom FFmpeg path",
    description: "Point to an existing ffmpeg.exe instead of the managed install.",
    keywords: ["override", "path", "binary", "exe", "ffmpeg"],
    route: SECTION_ROUTES.media,
  },

  // -- Browser -------------------------------------------------------------
  {
    section: "browser",
    id: "browser/status",
    label: "Handoff bridge status",
    description: "Live pipe status and round-trip test.",
    keywords: ["bridge", "pipe", "handoff", "extension", "ping"],
    route: SECTION_ROUTES.browser,
  },
  {
    section: "browser",
    id: "browser/extensions",
    label: "Browser extensions",
    description:
      "Which browsers have the Unduhin extension registered.",
    keywords: ["chrome", "edge", "brave", "firefox", "extension"],
    route: SECTION_ROUTES.browser,
  },
  {
    section: "browser",
    id: "browser/behaviour",
    label: "Handoff behaviour",
    description:
      "How Unduhin reacts when the browser starts a download.",
    keywords: ["catch-all", "ask-first", "rules", "passthrough", "mode"],
    route: SECTION_ROUTES.browser,
  },
  {
    section: "browser",
    id: "browser/file-types",
    label: "File types to capture",
    description: "Which file extensions Unduhin intercepts.",
    keywords: ["extensions", "mime", "filter", "types"],
    route: SECTION_ROUTES.browser,
  },
  {
    section: "browser",
    id: "browser/domain-rules",
    label: "Domain rules",
    description: "Per-host allow/block list with drag-to-reorder.",
    keywords: ["domain", "rules", "allowlist", "blocklist", "host"],
    route: SECTION_ROUTES.browser,
  },

  // -- About ---------------------------------------------------------------
  {
    section: "about",
    id: "about/hero",
    label: "Version & build",
    description:
      "Version, build timestamp, commit hash, channel, and host OS.",
    keywords: ["version", "build", "commit", "channel", "os"],
    route: SECTION_ROUTES.about,
  },
  {
    section: "about",
    id: "about/update-channel",
    label: "Update channel",
    description:
      "Switch between stable and beta release channels.",
    keywords: ["channel", "release", "beta", "stable", "update"],
    route: SECTION_ROUTES.about,
  },
  {
    section: "about",
    id: "about/update-check-on-startup",
    label: "Check for updates on startup",
    description: "One quick check on launch.",
    keywords: ["update", "auto", "startup"],
    route: SECTION_ROUTES.about,
  },
  {
    section: "about",
    id: "about/send-crash-reports",
    label: "Send anonymous crash reports",
    description: "Stack trace + OS version; never URLs or filenames.",
    keywords: ["telemetry", "crash", "report", "privacy"],
    route: SECTION_ROUTES.about,
  },
  {
    section: "about",
    id: "about/send-usage-stats",
    label: "Send anonymous usage statistics",
    description: "Feature counts and timing.",
    keywords: ["telemetry", "usage", "stats", "privacy"],
    route: SECTION_ROUTES.about,
  },
];
