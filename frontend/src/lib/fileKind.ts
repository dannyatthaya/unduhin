// Which family a download belongs to, for the coloured tile in the list.
// Pure function so the mapping is unit-testable and shared by the row and
// the detail panel.

import { extOf } from "@/lib/format";
import type { DownloadKind } from "@/types/tauri-bindings";

export type FileKind =
  | "video"
  | "audio"
  | "archive"
  | "app"
  | "doc"
  | "image"
  | "disc"
  | "torrent"
  | "file";

// `extOf` upper-cases and keeps at most four characters, so longer
// extensions appear here already cut (APPIMAGE → APPI).
const BY_EXTENSION: Record<Exclude<FileKind, "torrent" | "file">, readonly string[]> = {
  video: ["MP4", "MKV", "MOV", "AVI", "WEBM", "M4V", "FLV", "WMV"],
  audio: ["MP3", "FLAC", "WAV", "M4A", "OGG", "AAC", "OPUS"],
  archive: ["ZIP", "RAR", "7Z", "TAR", "GZ", "BZ2", "XZ", "ZST"],
  app: ["EXE", "MSI", "APK", "DMG", "DEB", "RPM", "APPI", "PKG"],
  doc: ["PDF", "DOC", "DOCX", "XLS", "XLSX", "PPT", "PPTX", "TXT", "EPUB", "CSV"],
  image: ["JPG", "JPEG", "PNG", "GIF", "WEBP", "SVG", "BMP"],
  disc: ["ISO", "IMG"],
};

export function fileKindOf(
  filename: string,
  downloadKind: DownloadKind = "http",
): FileKind {
  if (downloadKind === "torrent") return "torrent";
  const ext = extOf(filename);
  if (!ext) return "file";
  for (const [kind, extensions] of Object.entries(BY_EXTENSION)) {
    if (extensions.includes(ext)) return kind as FileKind;
  }
  return "file";
}
