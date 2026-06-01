import { defineConfig } from "vitest/config";

// Vitest runs the pure-logic tests in `tests/`. Background-module tests
// (those that touch `chrome.*`) belong in 8g's manual smoke matrix — we
// deliberately don't mock the extension APIs here, on the principle that
// either we trust the runtime contract or we run it against a real
// browser, not somewhere in between.
export default defineConfig({
  test: {
    include: ["tests/**/*.test.ts"],
    environment: "node",
    pool: "threads",
  },
});
