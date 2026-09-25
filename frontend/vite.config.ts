import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath, URL } from "node:url";

export default defineConfig({
  plugins: [vue(), tailwindcss()],
  clearScreen: false,
  define: {
    // Baked in at build time rather than read from a Tauri plugin at
    // runtime. Tauri always builds the frontend and the binary on the same
    // machine (`beforeBuildCommand`), so this is accurate, and being a
    // compile-time constant means the title bar cannot flash the wrong
    // window chrome before an async platform lookup resolves.
    // Read it through `@/lib/platform`, never directly.
    __PLATFORM__: JSON.stringify(process.platform),
    // vue-i18n's default message compiler builds each message with
    // `new Function`, which the app's Content-Security-Policy (no
    // 'unsafe-eval', see src-tauri/tauri.conf.json) refuses — the UI then
    // renders nothing. JIT compilation turns messages into an AST that is
    // interpreted without eval, the CSP-safe mode vue-i18n documents.
    __INTLIFY_JIT_COMPILATION__: true,
    __INTLIFY_DROP_MESSAGE_COMPILER__: false,
  },
  server: {
    port: 5173,
    strictPort: true,
    host: "127.0.0.1",
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  build: {
    rollupOptions: {
      input: {
        main: fileURLToPath(new URL("./index.html", import.meta.url)),
        "tray-popover": fileURLToPath(
          new URL("./tray-popover.html", import.meta.url),
        ),
      },
    },
  },
  test: {
    environment: "jsdom",
    globals: true,
    include: ["src/**/*.test.ts"],
  },
});
