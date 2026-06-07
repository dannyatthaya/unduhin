<script setup lang="ts">
import { onBeforeUnmount, onMounted } from "vue";
import { useRouter } from "vue-router";
import type { UnlistenFn } from "@tauri-apps/api/event";

import AppTitleBar from "@/components/AppTitleBar.vue";
import Toaster from "@/components/ui/Toaster.vue";
import DeleteConfirmDialog from "@/components/DeleteConfirmDialog.vue";
import ConfirmOnQuitDialog from "@/components/ConfirmOnQuitDialog.vue";
import AskHandoffDialog from "@/components/settings/browser/AskHandoffDialog.vue";
import { useUnduhinEvents } from "@/composables/useUnduhinEvents";
import { useTheme } from "@/composables/useTheme";
import {
  installConfirmOnQuitBridge,
  uninstallConfirmOnQuitBridge,
} from "@/composables/useConfirmOnQuit";
import { useNotifications } from "@/composables/useNotifications";
import { useClipboardCapture } from "@/composables/useClipboardCapture";
import { onCheckUpdates } from "@/types/tauri-bindings";

const router = useRouter();

// Wire the theme at the app shell so the persisted `theme_mode` setting
// always propagates to the DOM, even when the user toggles it from the
// Settings route (where AppTopBar — the other caller — is unmounted).
useTheme();
// One subscription for the whole app, so cross-route navigation does not
// drop events (e.g. `setting_changed` while the user is in Settings).
useUnduhinEvents();
// Native OS toasts for completion / failure / queue-empty. Subscribes
// separately so its event handler is independent of the store reducer.
useNotifications();
// Clipboard watcher. Gated on the persisted `watch_clipboard`
// boolean — the composable polls only while the toggle is on, so this
// is cheap when the user hasn't opted in.
useClipboardCapture();

// Suppress the browser's default right-click context menu everywhere
// except in editable inputs (so paste, look-up, etc. still work in the
// Add URL field and the user-agent textarea). Row-level @contextmenu
// handlers continue to fire — preventDefault only blocks the browser's
// own action, not our custom menus.
function onGlobalContextMenu(e: MouseEvent) {
  const t = e.target as HTMLElement | null;
  if (
    t &&
    (t.tagName === "INPUT" ||
      t.tagName === "TEXTAREA" ||
      t.isContentEditable)
  ) {
    return;
  }
  e.preventDefault();
}

// The close-behavior policy lives in the Rust window handler now, so
// the only thing App.vue does on mount is install the confirm-on-quit
// bridge that connects the Rust prompt to <ConfirmOnQuitDialog/>.
let unlistenCheckUpdates: UnlistenFn | null = null;

onMounted(async () => {
  window.addEventListener("contextmenu", onGlobalContextMenu);
  await installConfirmOnQuitBridge();
  // Tray "Check for updates…" → land on Settings → About and auto-run the
  // check there (SettingsAbout reads the `?check=1` query on mount).
  unlistenCheckUpdates = await onCheckUpdates(() => {
    router.push({ name: "settings-about", query: { check: "1" } });
  });
});

onBeforeUnmount(() => {
  uninstallConfirmOnQuitBridge();
  window.removeEventListener("contextmenu", onGlobalContextMenu);
  if (unlistenCheckUpdates) unlistenCheckUpdates();
});
</script>

<template>
  <div class="flex h-screen w-screen flex-col bg-background text-foreground">
    <AppTitleBar />
    <RouterView />

    <ConfirmOnQuitDialog />
    <DeleteConfirmDialog />
    <AskHandoffDialog />
    <Toaster />
  </div>
</template>
