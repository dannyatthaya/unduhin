// Which category a file name belongs to, and which category owns an
// extension. An extension belongs to one category only; these helpers are
// what the New download dialog and the category dialogs use to say so.

import { extensionOf } from "@/lib/linkInput";
import type { Category, CategoryId } from "@/types/tauri-bindings";

/** The seeded catch-all category. It cannot be deleted or reordered. */
export const FALLBACK_CATEGORY_NAME = "Other";

export function isFallbackCategory(category: Category): boolean {
  return category.name === FALLBACK_CATEGORY_NAME;
}

/** `.MP4`, `mp4 ` and `mp4` are the same extension. */
export function normalizeExtension(raw: string): string {
  return raw.trim().replace(/^\.+/, "").toLowerCase();
}

/**
 * The category that owns `extension`, skipping `exceptId` (the category
 * being edited). The first owner in list order wins, as it does when the
 * app files a new download.
 */
export function ownerOfExtension(
  extension: string,
  categories: readonly Category[],
  exceptId: CategoryId | null = null,
): Category | null {
  const wanted = normalizeExtension(extension);
  if (!wanted) return null;
  return (
    categories.find(
      (c) => c.id !== exceptId && c.extension_rules.some((e) => e === wanted),
    ) ?? null
  );
}

/**
 * The category a new download with this file name is filed into: the owner
 * of its extension, otherwise the fallback.
 */
export function categoryForFileName(
  fileName: string,
  categories: readonly Category[],
): Category | null {
  const fallback = categories.find(isFallbackCategory) ?? null;
  const extension = extensionOf(fileName);
  if (!extension) return fallback;
  return ownerOfExtension(extension, categories) ?? fallback;
}

/** Whether another category already uses this name. Names ignore capitals. */
export function isCategoryNameTaken(
  name: string,
  categories: readonly Category[],
  exceptId: CategoryId | null = null,
): boolean {
  const wanted = name.trim().toLowerCase();
  if (!wanted) return false;
  return categories.some(
    (c) => c.id !== exceptId && c.name.trim().toLowerCase() === wanted,
  );
}
