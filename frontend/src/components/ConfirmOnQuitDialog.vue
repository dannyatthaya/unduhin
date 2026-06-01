<script setup lang="ts">
// Styled confirm dialog driven by the Rust window handler. Mirrors the
// `DeleteConfirmDialog.vue` shape: mounted once at the app root, reacts
// to the singleton `useConfirmOnQuit` state, and reports the user's
// choice back through the same composable.

import { computed } from "vue";
import { useI18n } from "vue-i18n";

import Dialog from "@/components/ui/Dialog.vue";
import Button from "@/components/ui/Button.vue";

import { useConfirmOnQuit } from "@/composables/useConfirmOnQuit";

const { t } = useI18n();
const { pending, respond } = useConfirmOnQuit();

const open = computed(() => pending.value !== null);
const hasActive = computed(() => (pending.value?.active_count ?? 0) > 0);
const activeCount = computed(() => pending.value?.active_count ?? 0);
const askClose = computed(() => pending.value?.ask_close ?? true);

const title = computed(() =>
  askClose.value ? t("downloads.quitCloseTitle") : t("downloads.quitQuitTitle"),
);
const message = computed(() => {
  const n = activeCount.value;
  if (askClose.value) {
    return hasActive.value
      ? t("downloads.quitMessageCloseActive", { n })
      : t("downloads.quitMessageCloseIdle");
  }
  return t("downloads.quitMessageQuitActive", { n });
});

const confirmLabel = computed(() =>
  askClose.value ? t("downloads.quitButtonClose") : t("downloads.quitButtonQuit"),
);
const confirmVariant = computed<"primary" | "danger">(() =>
  askClose.value ? "primary" : "danger",
);
</script>

<template>
  <Dialog :open="open" :title="title" size="md" @close="respond(false)">
    <p class="text-sm text-muted-foreground">{{ message }}</p>

    <template #footer>
      <Button variant="secondary" @click="respond(false)">{{ t("common.cancel") }}</Button>
      <Button :variant="confirmVariant" @click="respond(true)">
        {{ confirmLabel }}
      </Button>
    </template>
  </Dialog>
</template>
