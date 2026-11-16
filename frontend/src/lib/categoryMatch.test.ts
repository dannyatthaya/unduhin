import { describe, expect, test } from "vitest";

import {
  categoryForFileName,
  isCategoryNameTaken,
  normalizeExtension,
  ownerOfExtension,
} from "./categoryMatch";
import type { Category } from "@/types/tauri-bindings";

function category(id: number, name: string, extensions: string[]): Category {
  return {
    id,
    name,
    icon: null,
    default_output_path: null,
    extension_rules: extensions,
    color: null,
  };
}

const VIDEO = category(1, "Video", ["mp4", "mkv"]);
const PROGRAMS = category(2, "Programs", ["exe", "iso"]);
const OTHER = category(3, "Other", []);
const ALL = [VIDEO, PROGRAMS, OTHER];

describe("normalizeExtension", () => {
  test("drops the dot, the case and the space", () => {
    expect(normalizeExtension(" .MP4 ")).toBe("mp4");
  });
});

describe("ownerOfExtension", () => {
  test("finds the category that has the extension", () => {
    expect(ownerOfExtension(".iso", ALL)).toBe(PROGRAMS);
  });

  test("skips the category being edited", () => {
    expect(ownerOfExtension("iso", ALL, PROGRAMS.id)).toBeNull();
  });

  test("returns null when no category has it", () => {
    expect(ownerOfExtension("epub", ALL)).toBeNull();
  });

  test("returns null for an empty extension", () => {
    expect(ownerOfExtension("  ", ALL)).toBeNull();
  });
});

describe("categoryForFileName", () => {
  test("files by extension", () => {
    expect(categoryForFileName("clip.MKV", ALL)).toBe(VIDEO);
  });

  test("falls back to Other for an unowned extension", () => {
    expect(categoryForFileName("notes.bin", ALL)).toBe(OTHER);
  });

  test("falls back to Other for a name with no extension", () => {
    expect(categoryForFileName("download", ALL)).toBe(OTHER);
  });

  test("returns null when there is no fallback category", () => {
    expect(categoryForFileName("notes.bin", [VIDEO])).toBeNull();
  });
});

describe("isCategoryNameTaken", () => {
  test("ignores capitals and surrounding space", () => {
    expect(isCategoryNameTaken("  video ", ALL)).toBe(true);
  });

  test("does not count the category being edited", () => {
    expect(isCategoryNameTaken("Video", ALL, VIDEO.id)).toBe(false);
  });

  test("an empty name is not taken", () => {
    expect(isCategoryNameTaken("", ALL)).toBe(false);
  });
});
