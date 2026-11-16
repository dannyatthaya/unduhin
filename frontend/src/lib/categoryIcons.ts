// Map the icon names persisted on `categories.icon` to lucide components,
// and resolve a category's tint. One place, so the sidebar, the settings
// list and the category dialogs agree.

import type { Component } from "vue";
import {
  AppWindow,
  Book,
  Code,
  File,
  FileArchive,
  FileText,
  Film,
  Image,
  Music,
  Package,
  Smile,
  Star,
} from "lucide-vue-next";

import type { Category, CategoryColor } from "@/types/tauri-bindings";

export interface CategoryIconOption {
  /** Persisted string. */
  key: string;
  /** i18n key for the icon's name (its accessible label in the picker). */
  labelKey: string;
  icon: Component;
  /** Tint used when the category has no colour of its own. */
  defaultColor: CategoryColor;
}

export const CATEGORY_ICONS: CategoryIconOption[] = [
  { key: "document", labelKey: "settings.iconDocument", icon: FileText, defaultColor: "blue" },
  { key: "book", labelKey: "settings.iconBook", icon: Book, defaultColor: "blue" },
  { key: "music", labelKey: "settings.iconMusic", icon: Music, defaultColor: "pink" },
  { key: "video", labelKey: "settings.iconVideo", icon: Film, defaultColor: "violet" },
  { key: "archive", labelKey: "settings.iconArchive", icon: FileArchive, defaultColor: "orange" },
  { key: "app", labelKey: "settings.iconProgram", icon: AppWindow, defaultColor: "green" },
  { key: "image", labelKey: "settings.iconImage", icon: Image, defaultColor: "pink" },
  { key: "package", labelKey: "settings.iconPackage", icon: Package, defaultColor: "orange" },
  { key: "code", labelKey: "settings.iconCode", icon: Code, defaultColor: "green" },
  { key: "star", labelKey: "settings.iconStar", icon: Star, defaultColor: "yellow" },
  // Kept so rows saved with these older keys still render; not offered in
  // the picker.
  { key: "smile", labelKey: "settings.iconOther", icon: Smile, defaultColor: "neutral" },
  { key: "other", labelKey: "settings.iconOther", icon: File, defaultColor: "neutral" },
];

const LEGACY_KEYS: readonly string[] = ["smile", "other"];
const BY_KEY = new Map(CATEGORY_ICONS.map((option) => [option.key, option]));
const FALLBACK_ICON = BY_KEY.get("other") ?? CATEGORY_ICONS[0];

export function iconFor(key: string | null | undefined): CategoryIconOption {
  return (key ? BY_KEY.get(key) : undefined) ?? FALLBACK_ICON;
}

/** Options shown in the icon picker. */
export const ICON_PICKER_OPTIONS: CategoryIconOption[] = CATEGORY_ICONS.filter(
  (option) => !LEGACY_KEYS.includes(option.key),
);

/** A category's tint: its own colour, else the one that goes with its icon. */
export function colorFor(
  category: Pick<Category, "icon" | "color">,
): CategoryColor {
  return category.color ?? iconFor(category.icon).defaultColor;
}

/** Class that paints a tile in a tint (see `downloads-theme.css`). */
export function colorClass(color: CategoryColor): string {
  return `dl-cat-${color}`;
}
