// Map the small palette of icon names persisted on `categories.icon` to
// lucide-vue-next components. Keeps the mapping in one place so the
// dialog picker, the table rows, and the rules editor agree.

import type { Component } from "vue";
import {
  FileText,
  Music,
  Video,
  Archive,
  LayoutGrid,
  HelpCircle,
  Book,
  Smile,
  Star,
} from "lucide-vue-next";

export interface CategoryIconOption {
  /** Persisted string. */
  key: string;
  label: string;
  /** Lucide component. */
  icon: Component;
  /** Tailwind text colour class for the icon. */
  tone: string;
  /** Tailwind bg colour class for the icon tile. */
  background: string;
}

export const CATEGORY_ICONS: CategoryIconOption[] = [
  { key: "document", label: "Document", icon: FileText, tone: "text-sky-500", background: "bg-sky-500/10" },
  { key: "music", label: "Music", icon: Music, tone: "text-emerald-500", background: "bg-emerald-500/10" },
  { key: "video", label: "Video", icon: Video, tone: "text-rose-500", background: "bg-rose-500/10" },
  { key: "archive", label: "Archive", icon: Archive, tone: "text-amber-600", background: "bg-amber-500/10" },
  { key: "app", label: "Programs", icon: LayoutGrid, tone: "text-amber-500", background: "bg-amber-400/10" },
  { key: "book", label: "Book", icon: Book, tone: "text-violet-500", background: "bg-violet-500/10" },
  { key: "smile", label: "Other", icon: Smile, tone: "text-pink-500", background: "bg-pink-500/10" },
  { key: "star", label: "Favorites", icon: Star, tone: "text-yellow-500", background: "bg-yellow-500/10" },
  { key: "other", label: "Other", icon: HelpCircle, tone: "text-violet-500", background: "bg-violet-500/10" },
];

const _byKey = new Map(CATEGORY_ICONS.map((opt) => [opt.key, opt]));

export function iconFor(key: string | null | undefined): CategoryIconOption {
  if (key) {
    const hit = _byKey.get(key);
    if (hit) return hit;
  }
  return _byKey.get("other") ?? CATEGORY_ICONS[0];
}

/** Options shown in the icon picker — excludes the "other" fallback. */
export const ICON_PICKER_OPTIONS: CategoryIconOption[] = CATEGORY_ICONS.filter(
  (o) => o.key !== "other",
);
