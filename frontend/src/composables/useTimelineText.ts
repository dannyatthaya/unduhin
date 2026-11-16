// Words for a download's timeline entries. The store keeps each entry as
// facts; this turns one into a title and a detail line in the user's
// language. Shared by the History tab, its "Copy log" text and the
// "What happened" list on a failed download.

import { useI18n } from "vue-i18n";

import type { TimelineEntry } from "@/stores/downloads";
import type { DownloadSource, Status } from "@/types/tauri-bindings";

export interface TimelineText {
  title: string;
  detail: string;
}

export type TimelineTone = "muted" | "queue" | "accent" | "ok" | "fail";

const STATUS_TITLE_KEYS: Record<Status, string> = {
  queued: "detail.eventQueued",
  active: "detail.eventStarted",
  muxing: "detail.eventMuxing",
  paused: "detail.eventPaused",
  completed: "detail.eventFinished",
  failed: "detail.eventFailed",
  cancelled: "detail.eventCancelled",
};

const SOURCE_KEYS: Record<DownloadSource, string> = {
  manual: "detail.sourceManual",
  extension_pipe: "detail.sourceExtension",
  cli: "detail.sourceCli",
};

/** Dot colour for an entry: what kind of moment it was. */
export function timelineTone(entry: TimelineEntry): TimelineTone {
  switch (entry.kind) {
    case "completed":
      return "ok";
    case "failed":
      return "fail";
    case "retry":
      return "accent";
    case "status":
      if (entry.to === "active") return "accent";
      if (entry.to === "queued") return "queue";
      if (entry.to === "completed") return "ok";
      if (entry.to === "failed") return "fail";
      return "muted";
    default:
      return "muted";
  }
}

export function useTimelineText() {
  const { t } = useI18n();

  function sourceLabel(source: DownloadSource | undefined): string {
    return source ? t(SOURCE_KEYS[source]) : "";
  }

  function describe(entry: TimelineEntry): TimelineText {
    switch (entry.kind) {
      case "started":
        return { title: t("detail.eventAdded"), detail: sourceLabel(entry.source) };
      case "retry":
        return { title: t("detail.eventRetried"), detail: t("detail.eventRetriedDetail") };
      case "completed":
        return { title: t("detail.eventFinished"), detail: entry.filename ?? "" };
      case "failed":
        return { title: t("detail.eventFailed"), detail: entry.error ?? "" };
      case "status":
        return {
          title: entry.to ? t(STATUS_TITLE_KEYS[entry.to]) : "",
          detail: "",
        };
    }
  }

  return { describe, sourceLabel };
}
