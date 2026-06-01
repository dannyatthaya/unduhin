<script setup lang="ts">
// Modal surface for `ask-first` mode. Listens for the
// `unduhin:ask-handoff` Tauri event the pipe server emits when the
// extension fires `Inbound::AskHandoff`. Renders the job's URL +
// filename + size, then sends the user's choice back via the
// `respond_handoff` Tauri command. The pipe server broadcasts the
// decision so the extension's pending waiter resolves.

import { onMounted, onBeforeUnmount, ref } from "vue";
import { useI18n } from "vue-i18n";

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import Dialog from "@/components/ui/Dialog.vue";
import Button from "@/components/ui/Button.vue";
import type { DownloadJob, HandoffDecision } from "@/types/wire";

interface AskHandoffPayload {
  readonly id: string;
  readonly job: DownloadJob;
}

const { t } = useI18n();

const open = ref(false);
const current = ref<AskHandoffPayload | null>(null);
// Queue extras while one prompt is already open — Tauri doesn't
// guarantee one-at-a-time, and we don't want to drop user-visible
// events on the floor.
const queue: AskHandoffPayload[] = [];

let unlisten: UnlistenFn | null = null;

function showNext(): void {
  const next = queue.shift();
  if (!next) {
    current.value = null;
    open.value = false;
    return;
  }
  current.value = next;
  open.value = true;
}

function enqueue(payload: AskHandoffPayload): void {
  if (open.value) {
    queue.push(payload);
    return;
  }
  current.value = payload;
  open.value = true;
}

async function decide(decision: HandoffDecision): Promise<void> {
  const payload = current.value;
  if (!payload) return;
  open.value = false;
  current.value = null;
  try {
    await invoke<void>("respond_handoff", { id: payload.id, decision });
  } catch (err) {
    console.warn("respond_handoff failed", err);
  }
  showNext();
}

function formatBytes(n: number | null): string {
  if (n == null) return t("settings.browserAskUnknownSize");
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

onMounted(async () => {
  unlisten = await listen<AskHandoffPayload>("unduhin:ask-handoff", (event) => {
    enqueue(event.payload);
  });
});

onBeforeUnmount(() => {
  if (unlisten) unlisten();
});
</script>

<template>
  <Dialog
    :open="open"
    :title="t('settings.browserAskTitle')"
    size="md"
    hide-close
    @close="decide('passthrough')"
  >
    <div v-if="current" class="flex flex-col gap-3 text-sm">
      <p class="text-muted-foreground">
        {{ t("settings.browserAskBody") }}
      </p>
      <dl class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
        <dt class="text-muted-foreground">{{ t("settings.browserAskFilename") }}</dt>
        <dd class="truncate font-mono">
          {{ current.job.filename ?? t("common.untitled") }}
        </dd>
        <dt class="text-muted-foreground">{{ t("settings.browserAskUrl") }}</dt>
        <dd class="truncate font-mono">{{ current.job.finalUrl }}</dd>
        <dt class="text-muted-foreground">{{ t("settings.browserAskSize") }}</dt>
        <dd>{{ formatBytes(current.job.size) }}</dd>
      </dl>
    </div>
    <template #footer>
      <Button variant="secondary" @click="decide('passthrough')">
        {{ t("settings.browserAskPassthrough") }}
      </Button>
      <Button @click="decide('capture')">
        {{ t("settings.browserAskCapture") }}
      </Button>
    </template>
  </Dialog>
</template>
