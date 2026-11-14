import { describe, expect, it } from "vitest";

import { downloadLinkFromPaste } from "@/lib/pastedLink";

describe("downloadLinkFromPaste", () => {
  it("accepts http and https links, trimming surrounding whitespace", () => {
    expect(downloadLinkFromPaste("  https://cdn.example.com/a.zip\n")).toBe(
      "https://cdn.example.com/a.zip",
    );
    expect(downloadLinkFromPaste("http://example.com/file")).toBe(
      "http://example.com/file",
    );
  });

  it("accepts magnet links", () => {
    const magnet = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567";
    expect(downloadLinkFromPaste(magnet)).toBe(magnet);
  });

  it("rejects other schemes", () => {
    expect(downloadLinkFromPaste("javascript:alert(1)")).toBeNull();
    expect(downloadLinkFromPaste("file:///C:/secret.txt")).toBeNull();
    expect(downloadLinkFromPaste("mailto:someone@example.com")).toBeNull();
  });

  it("rejects prose, multiple links and empty text", () => {
    expect(downloadLinkFromPaste("see https://example.com/a.zip please")).toBeNull();
    expect(downloadLinkFromPaste("https://a.example/1 https://a.example/2")).toBeNull();
    expect(downloadLinkFromPaste("just some words")).toBeNull();
    expect(downloadLinkFromPaste("   ")).toBeNull();
  });
});
