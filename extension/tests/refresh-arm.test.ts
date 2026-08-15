import { describe, expect, it } from "vitest";

import { baseName, createRefreshArmTable } from "../src/background/refresh-arm.js";

/** Fixed clock so expiry is deterministic. */
function clockFrom(start: number) {
  let now = start;
  return {
    now: () => now,
    advance: (ms: number) => {
      now += ms;
    },
  };
}

const T0 = 1_786_000_000_000;

function armed(over: Partial<Parameters<ReturnType<typeof createRefreshArmTable>["arm"]>[0]> = {}) {
  return {
    downloadId: 42,
    filename: "file.zip",
    sizeBytes: 123456,
    origin: "https://cdn.example.com",
    expiresAt: T0 + 60_000,
    ...over,
  };
}

describe("baseName", () => {
  it("strips a Windows path and lowercases", () => {
    expect(baseName("C:\\Users\\me\\Downloads\\File.ZIP")).toBe("file.zip");
  });

  it("strips a POSIX path", () => {
    expect(baseName("/home/me/downloads/file.zip")).toBe("file.zip");
  });

  it("passes a bare name through", () => {
    expect(baseName("file.zip")).toBe("file.zip");
  });
});

describe("refresh arm table", () => {
  it("matches a capture on name and size", () => {
    const c = clockFrom(T0);
    const table = createRefreshArmTable(c.now);
    table.arm(armed());

    // Chrome hands back a full path; the table compares base names.
    const hit = table.match("C:\\Users\\me\\Downloads\\file.zip", 123456);
    expect(hit?.downloadId).toBe(42);
  });

  it("matches on name alone when the row never learned a size", () => {
    const c = clockFrom(T0);
    const table = createRefreshArmTable(c.now);
    table.arm(armed({ sizeBytes: null }));

    expect(table.match("file.zip", 999)?.downloadId).toBe(42);
  });

  it("matches on name alone when the capture reports no size", () => {
    const c = clockFrom(T0);
    const table = createRefreshArmTable(c.now);
    table.arm(armed());

    expect(table.match("file.zip", null)?.downloadId).toBe(42);
  });

  it("refuses a same-named file of a different size", () => {
    // A different size means a different file. Adopting it would splice two
    // bodies together, which is exactly what the backend validator prevents —
    // no reason to send it in the first place.
    const c = clockFrom(T0);
    const table = createRefreshArmTable(c.now);
    table.arm(armed());

    expect(table.match("file.zip", 999)).toBeNull();
  });

  it("does not match a different name", () => {
    const c = clockFrom(T0);
    const table = createRefreshArmTable(c.now);
    table.arm(armed());

    expect(table.match("other.zip", 123456)).toBeNull();
  });

  it("prefers the size-confirmed entry when two share a name", () => {
    const c = clockFrom(T0);
    const table = createRefreshArmTable(c.now);
    table.arm(armed({ downloadId: 1, sizeBytes: null }));
    table.arm(armed({ downloadId: 2, sizeBytes: 500 }));

    expect(table.match("file.zip", 500)?.downloadId).toBe(2);
  });

  it("drops an entry once it expires", () => {
    const c = clockFrom(T0);
    const table = createRefreshArmTable(c.now);
    table.arm(armed({ expiresAt: T0 + 1_000 }));

    expect(table.match("file.zip", 123456)?.downloadId).toBe(42);
    c.advance(1_001);
    expect(table.match("file.zip", 123456)).toBeNull();
    expect(table.hasAny()).toBe(false);
  });

  it("caps how long an entry can live regardless of what the app asked", () => {
    const c = clockFrom(T0);
    const table = createRefreshArmTable(c.now);
    // A bad or hostile expiry must not pin an entry forever.
    table.arm(armed({ expiresAt: T0 + 365 * 24 * 3600 * 1000 }));

    c.advance(11 * 60 * 1000);
    expect(table.match("file.zip", 123456)).toBeNull();
  });

  it("re-arming the same row replaces rather than stacks", () => {
    const c = clockFrom(T0);
    const table = createRefreshArmTable(c.now);
    table.arm(armed({ filename: "old.zip" }));
    table.arm(armed({ filename: "new.zip" }));

    expect(table.match("old.zip", 123456)).toBeNull();
    expect(table.match("new.zip", 123456)?.downloadId).toBe(42);
  });

  it("bounds the table so a service worker cannot leak", () => {
    const c = clockFrom(T0);
    const table = createRefreshArmTable(c.now);
    for (let i = 0; i < 8; i++) {
      table.arm(armed({ downloadId: i, filename: `f${i}.zip`, expiresAt: T0 + 1_000 * (i + 1) }));
    }
    // Oldest expiry is evicted first, so the earliest arms are gone and the
    // most recent survive.
    expect(table.match("f0.zip", 123456)).toBeNull();
    expect(table.match("f7.zip", 123456)?.downloadId).toBe(7);
  });

  it("removes an entry after a successful adoption", () => {
    const c = clockFrom(T0);
    const table = createRefreshArmTable(c.now);
    table.arm(armed());
    table.remove(42);

    expect(table.match("file.zip", 123456)).toBeNull();
    expect(table.hasAny()).toBe(false);
  });

  it("reports nothing armed on an empty table", () => {
    const table = createRefreshArmTable(clockFrom(T0).now);
    expect(table.hasAny()).toBe(false);
    expect(table.match("file.zip", 1)).toBeNull();
  });
});
