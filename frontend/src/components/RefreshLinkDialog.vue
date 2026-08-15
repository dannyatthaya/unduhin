<script setup lang="ts">
// Modal driven by the `useRefreshLink` composable. Mounted once at the app
// root so any surface can call `requestRefresh(id, filename)`.
//
// Two paths are live at the same time. The extension is armed the moment the
// dialog opens, so the user can simply go back to the site and click the link
// again. The paste field covers everything else: no extension, a dead service
// worker, or a URL obtained some other way.

import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";

import Dialog from "@/components/ui/Dialog.vue";
import Button from "@/components/ui/Button.vue";

import { useRefreshLink } from "@/composables/useRefreshLink";
import { formatBytes } from "@/lib/format";
import type { DownloadId, RefreshOutcome } from "@/types/tauri-bindings";

const { t } = useI18n();
const {
  pending,
  phase,
  error,
  mismatch,
  canForceRestart,
  lastOutcome,
  isOpen,
  submitUrl,
  confirmRestart,
  applyExternalOutcome,
  close,
} = useRefreshLink();

const url = ref("");

// Clear the field between openings so a previous attempt's URL never leaks
// into the next download's dialog.
watch(isOpen, (open) => {
  if (open) url.value = "";
});

const canSubmit = computed(() => {
  const v = url.value.trim();
  return phase.value !== "working" && (v.startsWith("http://") || v.startsWith("https://"));
});

/** True when arming failed, so no browser capture is coming. */
const browserPathUnavailable = computed(
  () => phase.value === "waiting" && (pending.value?.expiresAtMs ?? 0) === 0
);

const doneMessage = computed(() => {
  const o = lastOutcome.value;
  if (!o) return null;
  if (o.outcome === "resumed") {
    return t("downloads.refreshResumed", { bytes: formatBytes(o.downloaded_bytes) });
  }
  return t("downloads.refreshRestarted");
});

const mismatchSizes = computed(() => {
  const m = mismatch.value;
  if (!m) return { was: "—", now: "—" };
  return {
    was: m.oldBytes === null ? "—" : formatBytes(m.oldBytes),
    now: m.newBytes === null ? "—" : formatBytes(m.newBytes),
  };
});

function onSubmit() {
  if (!canSubmit.value) return;
  void submitUrl(url.value.trim());
}

// The extension path resolves out of band: the user re-clicks the link in the
// browser, the pipe handler applies the capture, and the app emits the result
// here. Without this listener the dialog would sit on "waiting" after the
// refresh had already succeeded.
let unlisten: UnlistenFn | null = null;

onMounted(async () => {
  unlisten = await listen<[DownloadId, RefreshOutcome, string]>(
    "unduhin:refresh-outcome",
    (event) => {
      const [id, outcome, capturedUrl] = event.payload;
      applyExternalOutcome(id, outcome, capturedUrl);
    },
  );
});

onBeforeUnmount(() => {
  if (unlisten) unlisten();
});
</script>

<template>
  <Dialog
    :open="isOpen"
    :title="t('downloads.refreshLinkTitle')"
    size="md"
    @close="close"
  >
    <p class="text-sm text-muted-foreground">
      {{ t("downloads.refreshLinkDescription", { filename: pending?.filename ?? "" }) }}
    </p>

    <!-- The size mismatch is the one case that needs a real decision. Show it
         alone so the paste field cannot distract from the question. -->
    <template v-if="phase === 'changed'">
      <p class="mt-4 text-sm text-foreground">{{ t("downloads.refreshChangedTitle") }}</p>
      <dl class="mt-2 grid grid-cols-2 gap-x-4 gap-y-1 text-xs">
        <dt class="text-muted-foreground">{{ t("downloads.refreshChangedWas") }}</dt>
        <dd class="text-foreground">{{ mismatchSizes.was }}</dd>
        <dt class="text-muted-foreground">{{ t("downloads.refreshChangedNow") }}</dt>
        <dd class="text-foreground">{{ mismatchSizes.now }}</dd>
      </dl>
      <p class="mt-3 text-xs text-muted-foreground">
        {{ t("downloads.refreshChangedHint") }}
      </p>
    </template>

    <template v-else-if="phase === 'done'">
      <p class="mt-4 text-sm text-foreground">{{ doneMessage }}</p>
    </template>

    <template v-else>
      <ol class="mt-4 space-y-2 text-sm text-muted-foreground">
        <li v-if="!browserPathUnavailable">
          <span class="font-medium text-foreground">1.</span>
          {{ t("downloads.refreshStepBrowser") }}
        </li>
        <li>
          <span class="font-medium text-foreground">{{ browserPathUnavailable ? "" : "2." }}</span>
          {{ t("downloads.refreshStepPaste") }}
        </li>
      </ol>

      <p v-if="browserPathUnavailable" class="mt-2 text-xs text-warning">
        {{ t("downloads.refreshNoExtension") }}
      </p>

      <input
        v-model="url"
        type="url"
        inputmode="url"
        spellcheck="false"
        :placeholder="t('downloads.refreshUrlPlaceholder')"
        class="mt-3 w-full rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:ring-2 focus:ring-ring"
        @keydown.enter.prevent="onSubmit"
      />
    </template>

    <p v-if="error" class="mt-3 text-xs text-danger">{{ error }}</p>

    <template #footer>
      <template v-if="phase === 'changed'">
        <Button variant="ghost" @click="close">{{ t("common.cancel") }}</Button>
        <Button variant="danger" :disabled="!canForceRestart" @click="confirmRestart">
          {{ t("downloads.refreshRestartAnyway") }}
        </Button>
      </template>
      <template v-else-if="phase === 'done'">
        <Button variant="secondary" @click="close">{{ t("common.close") }}</Button>
      </template>
      <template v-else>
        <Button variant="ghost" @click="close">{{ t("common.cancel") }}</Button>
        <Button variant="secondary" :disabled="!canSubmit" @click="onSubmit">
          {{ phase === "working" ? t("downloads.refreshWorking") : t("downloads.refreshApply") }}
        </Button>
      </template>
    </template>
  </Dialog>
</template>
