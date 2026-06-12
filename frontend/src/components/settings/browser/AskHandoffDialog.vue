<script setup lang="ts">
// Config surface for `ask-first` mode. Listens for the
// `unduhin:ask-handoff` Tauri event the pipe server emits when the
// extension fires `Inbound::AskHandoff`, then renders a full download
// config dialog (filename, category, segments, output folder, schedule)
// pre-filled from the captured job.
//
// On confirm we start the download ourselves via `start_handoff_download`,
// which folds the captured cookies/referer/UA/headers back in server-side
// so authenticated downloads keep working. On cancel we simply abort — the
// extension already blocked the browser's own Save-As dialog and never
// re-issues the download, so nothing leaks to the browser (the only mode
// that does is `passthrough` = extension off).

import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";

import Dialog from "@/components/ui/Dialog.vue";
import Button from "@/components/ui/Button.vue";
import Input from "@/components/ui/Input.vue";

import { useCategoriesStore } from "@/stores/categories";
import { api } from "@/types/tauri-bindings";
import { formatBytes } from "@/lib/format";
import type { DownloadJob } from "@/types/wire";

interface AskHandoffPayload {
  readonly id: string;
  readonly job: DownloadJob;
}

const { t } = useI18n();
const categories = useCategoriesStore();

const open = ref(false);
const current = ref<AskHandoffPayload | null>(null);
// Queue extras while one prompt is already open — Tauri doesn't
// guarantee one-at-a-time, and we don't want to drop user-visible
// events on the floor.
const queue: AskHandoffPayload[] = [];

// ── Form state (re-seeded from the job each time a prompt opens) ──────────
const filename = ref("");
const categoryId = ref<number | null>(null);
const segments = ref<number | null>(null);
const outputPath = ref("");
const submitting = ref(false);
const errorMessage = ref<string | null>(null);

// Optional "Start at" — when filled, a `start_at` schedule row is attached
// after the download is created, mirroring the Add-URL dialog.
const scheduleEnabled = ref(false);
const scheduleStartAt = ref("");

let unlisten: UnlistenFn | null = null;

function defaultStartAt(): string {
  // "1 minute from now" so the field is populated with something sensible
  // when the disclosure opens.
  const d = new Date(Date.now() + 60_000);
  const pad = (n: number) => n.toString().padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(
    d.getHours(),
  )}:${pad(d.getMinutes())}`;
}

function enqueue(payload: AskHandoffPayload): void {
  if (open.value) {
    queue.push(payload);
    return;
  }
  current.value = payload;
  open.value = true;
}

/** Close the current prompt and surface the next queued one (if any). */
function finish(): void {
  open.value = false;
  current.value = null;
  const next = queue.shift();
  if (next) {
    current.value = next;
    open.value = true;
  }
}

// Re-seed the form whenever a new job becomes current.
watch(current, (payload) => {
  if (!payload) return;
  filename.value = payload.job.filename ?? "";
  categoryId.value = null;
  segments.value = null;
  outputPath.value = "";
  errorMessage.value = null;
  submitting.value = false;
  scheduleEnabled.value = false;
  scheduleStartAt.value = defaultStartAt();
});

async function pickFolder(): Promise<void> {
  const picked = await openFileDialog({ directory: true, multiple: false });
  if (typeof picked === "string") outputPath.value = picked;
}

async function attachScheduleIfRequested(id: number): Promise<void> {
  if (scheduleEnabled.value && scheduleStartAt.value) {
    try {
      await api.addSchedule({
        kind: "start_at",
        download_id: id,
        start_iso: new Date(scheduleStartAt.value).toISOString(),
      });
    } catch (e) {
      // The download itself succeeded; surfacing a schedule-add failure as
      // an inline error would be confusing. Log and move on.
      console.warn("Failed to attach start_at schedule:", e);
    }
  }
}

async function confirm(): Promise<void> {
  const payload = current.value;
  if (!payload) return;
  submitting.value = true;
  errorMessage.value = null;
  try {
    const id = await api.startHandoffDownload(payload.job, {
      filename: filename.value.trim() || null,
      outputPath: outputPath.value.trim() || null,
      categoryId: categoryId.value,
      segments: segments.value,
    });
    await attachScheduleIfRequested(id);
    finish();
  } catch (e: unknown) {
    errorMessage.value =
      (e as { message?: string })?.message ?? t("errors.addDownload", { error: "" });
  } finally {
    submitting.value = false;
  }
}

/** Abort: drop this capture. The browser dialog stayed blocked, so nothing
 *  downloads — the user can re-trigger from the browser if they change their
 *  mind. */
function cancel(): void {
  finish();
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
  <Dialog :open="open" :title="t('settings.browserAskTitle')" @close="cancel">
    <div v-if="current" class="space-y-3">
      <p class="text-xs text-muted-foreground">
        {{ t("settings.browserAskBody") }}
      </p>

      <!-- Read-only context: what was captured. -->
      <dl class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
        <dt class="text-muted-foreground">{{ t("settings.browserAskUrl") }}</dt>
        <dd class="truncate font-mono" :title="current.job.finalUrl">
          {{ current.job.finalUrl }}
        </dd>
        <dt class="text-muted-foreground">{{ t("settings.browserAskSize") }}</dt>
        <dd>
          {{
            current.job.size == null
              ? t("settings.browserAskUnknownSize")
              : formatBytes(current.job.size)
          }}
        </dd>
      </dl>

      <div>
        <label class="mb-1 block text-xs font-medium text-muted-foreground">
          {{ t("addUrl.filenameLabel") }}
        </label>
        <Input v-model="filename" :placeholder="t('addUrl.filenameHint')" />
      </div>

      <div class="grid grid-cols-2 gap-3">
        <div>
          <label class="mb-1 block text-xs font-medium text-muted-foreground">
            {{ t("addUrl.categoryLabel") }}
          </label>
          <select
            v-model.number="categoryId"
            class="h-9 w-full rounded-md border border-input bg-background px-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <option :value="null">{{ t("addUrl.categoryAuto") }}</option>
            <option v-for="c in categories.list" :key="c.id" :value="c.id">
              {{ c.name }}
            </option>
          </select>
        </div>
        <div>
          <label class="mb-1 block text-xs font-medium text-muted-foreground">
            {{ t("addUrl.segmentsLabel") }}
          </label>
          <Input
            :model-value="segments?.toString() ?? ''"
            @update:model-value="
              (v: string) => (segments = v ? Math.max(1, parseInt(v, 10) || 1) : null)
            "
            :placeholder="t('addUrl.segmentsPlaceholder', { n: 8 })"
            type="number"
          />
        </div>
      </div>

      <div>
        <label class="mb-1 block text-xs font-medium text-muted-foreground">
          {{ t("addUrl.outputLabel") }}
        </label>
        <div class="flex gap-2">
          <Input v-model="outputPath" :placeholder="t('addUrl.outputPlaceholder')" />
          <Button variant="secondary" size="md" @click="pickFolder">
            {{ t("common.browse") }}
          </Button>
        </div>
      </div>

      <div>
        <button
          v-if="!scheduleEnabled"
          type="button"
          class="text-xs font-medium text-primary hover:underline"
          @click="scheduleEnabled = true"
        >
          {{ t("addUrl.startAtToggle") }}
        </button>
        <div v-else>
          <div class="mb-1 flex items-center justify-between">
            <label class="text-xs font-medium text-muted-foreground">
              {{ t("addUrl.startAtLabel") }}
            </label>
            <button
              type="button"
              class="text-xs text-muted-foreground hover:underline"
              @click="scheduleEnabled = false"
            >
              {{ t("addUrl.startAtClear") }}
            </button>
          </div>
          <Input v-model="scheduleStartAt" type="datetime-local" />
          <p class="mt-1 text-[11px] text-muted-foreground">
            {{ t("addUrl.startAtHint") }}
          </p>
        </div>
      </div>

      <p v-if="errorMessage" class="text-xs text-danger">{{ errorMessage }}</p>
    </div>

    <template #footer>
      <Button variant="ghost" :disabled="submitting" @click="cancel">
        {{ t("common.cancel") }}
      </Button>
      <Button variant="primary" :disabled="submitting" @click="confirm">
        {{ submitting ? t("addUrl.submitting") : t("addUrl.submit") }}
      </Button>
    </template>
  </Dialog>
</template>
