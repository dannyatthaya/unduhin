<script setup lang="ts">
// The card at the top of Overview when a download is waiting on something:
// a start time (queued) or the user (failed). A failed download whose link
// expired gets the fix right here — paste a fresh link, then Resume.

import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { AlertCircle, Calendar, Link, ListEnd, Pencil } from "lucide-vue-next";

import Button from "@/components/ui/Button.vue";
import Field from "@/components/ui/Field.vue";

import { useDownloadsStore } from "@/stores/downloads";
import { useSchedulesStore } from "@/stores/schedules";
import { useRefreshLink } from "@/composables/useRefreshLink";
import { useTimelineText } from "@/composables/useTimelineText";
import { useToast } from "@/composables/useToast";
import { classifyLink } from "@/lib/linkInput";
import { isSameDay, timeUntil, toTimeInput } from "@/lib/startPlan";
import type { DownloadRecord } from "@/types/tauri-bindings";

const props = defineProps<{ download: DownloadRecord }>();
const emit = defineEmits<{ "edit-schedule": [] }>();

const { t, locale } = useI18n();
const downloads = useDownloadsStore();
const schedules = useSchedulesStore();
const refreshLink = useRefreshLink();
const toast = useToast();
const { describe } = useTimelineText();

// ── Queued with a start time ────────────────────────────────────────────

const rows = computed(() => schedules.byDownload.get(props.download.id) ?? []);
const startRow = computed(() => rows.value.find((s) => s.kind === "start_at") ?? null);
const waitsForQueue = computed(() => rows.value.some((s) => s.kind === "after_queue"));
const isWaiting = computed(
  () => props.download.status === "queued" || props.download.status === "paused",
);

const startAt = computed(() => {
  const iso = startRow.value?.start_iso;
  const date = iso ? new Date(iso) : null;
  return date && !Number.isNaN(date.getTime()) ? date : null;
});

const startLine = computed(() => {
  const at = startAt.value;
  if (!at) return "";
  const now = new Date();
  const time = toTimeInput(at);
  if (isSameDay(at, now)) return t("detail.startsToday", { time });
  const date = at.toLocaleDateString(locale.value, { day: "numeric", month: "short" });
  return t("detail.startsOn", { date, time });
});

const startIn = computed(() => {
  const until = startAt.value ? timeUntil(startAt.value, new Date()) : null;
  if (!until) return "";
  if (until.days > 0) return t("start.inDaysHours", { days: until.days, hours: until.hours });
  if (until.hours > 0) {
    return t("start.inHoursMinutes", { hours: until.hours, minutes: until.minutes });
  }
  return t("start.inMinutes", { minutes: until.minutes });
});

async function clearSchedule() {
  try {
    for (const row of rows.value) await schedules.remove(row.id);
  } catch (e: unknown) {
    toast.push((e as { message?: string })?.message ?? t("downloads.actionFailed"), "error");
  }
}

// ── Failed ──────────────────────────────────────────────────────────────

const isFailed = computed(() => props.download.status === "failed");
/** Only a direct link can be swapped for a fresh one. */
const isExpiredLink = computed(
  () =>
    isFailed.value &&
    props.download.error_kind === "expired_auth" &&
    props.download.kind === "http",
);

const freshLink = ref("");
watch(
  () => props.download.id,
  () => {
    freshLink.value = "";
  },
);
const canResume = computed(() => classifyLink(freshLink.value) === "http");
const fieldRef = ref<InstanceType<typeof Field> | null>(null);

/** Hand the pasted link to the shared refresh flow, which also handles
 *  "this link points to a different file". */
async function resumeWithFreshLink() {
  if (!canResume.value) return;
  await refreshLink.requestRefresh(props.download.id, props.download.filename);
  await refreshLink.submitUrl(freshLink.value.trim());
  freshLink.value = "";
}

async function retry() {
  try {
    await downloads.retry(props.download.id);
  } catch (e: unknown) {
    const message = (e as { message?: string })?.message ?? String(e);
    toast.push(t("errors.restartDownload", { error: message }), "error");
  }
}

/** What led up to the failure, from this session's timeline. */
const log = computed(() =>
  downloads.timelineFor(props.download.id).map((entry, index) => {
    const text = describe(entry);
    const at = new Date(entry.at);
    return {
      key: `${entry.at}-${index}`,
      time: Number.isNaN(at.getTime())
        ? ""
        : at.toLocaleTimeString(locale.value, {
            hour: "2-digit",
            minute: "2-digit",
            second: "2-digit",
            hour12: false,
          }),
      text: text.detail ? `${text.title}: ${text.detail}` : text.title,
    };
  }),
);

function focusFreshLink() {
  fieldRef.value?.focus();
}

defineExpose({ focusFreshLink });
</script>

<template>
  <div
    v-if="isWaiting && (startAt || waitsForQueue)"
    class="flex flex-col gap-3 rounded-xl border border-dl-line bg-dl-queue-soft p-3.5"
  >
    <div class="flex items-center gap-2.5">
      <component
        :is="startAt ? Calendar : ListEnd"
        class="h-[18px] w-[18px] shrink-0 text-info"
        aria-hidden="true"
      />
      <span class="flex min-w-0 flex-1 flex-col gap-0.5">
        <span class="font-semibold">
          <template v-if="startAt">
            {{ startLine }}
            <span v-if="startIn" class="font-mono text-xs font-normal text-muted-foreground">
              ({{ startIn }})
            </span>
          </template>
          <template v-else>{{ t("detail.startsAfterQueue") }}</template>
        </span>
        <span class="text-xs leading-[1.45] text-muted-foreground">
          {{ startAt ? t("detail.startsAtHint") : t("start.afterQueueHint") }}
        </span>
      </span>
    </div>
    <div class="flex gap-2">
      <Button size="sm" @click="emit('edit-schedule')">
        <Pencil class="h-[13px] w-[13px]" aria-hidden="true" />
        {{ t("detail.changeTime") }}
      </Button>
      <Button size="sm" variant="ghost" @click="clearSchedule">
        {{ t("detail.clearTime") }}
      </Button>
    </div>
  </div>

  <template v-if="isFailed">
    <div
      role="alert"
      class="flex flex-col gap-3 rounded-xl border border-danger bg-dl-fail-soft p-3.5"
    >
      <div class="flex items-start gap-2.5">
        <component
          :is="isExpiredLink ? Link : AlertCircle"
          class="mt-px h-[18px] w-[18px] shrink-0 text-danger"
          aria-hidden="true"
        />
        <span class="flex min-w-0 flex-col gap-1">
          <span class="font-semibold">
            {{ isExpiredLink ? t("detail.linkExpiredTitle") : t("detail.failedTitle") }}
          </span>
          <span class="break-words text-xs leading-[1.45] text-dl-soft">
            {{ isExpiredLink ? t("detail.linkExpiredBody") : (download.error ?? t("detail.failedBody")) }}
          </span>
        </span>
      </div>
      <div v-if="isExpiredLink" class="flex gap-2">
        <Field
          ref="fieldRef"
          v-model="freshLink"
          size="sm"
          mono
          type="url"
          spellcheck="false"
          class="flex-1"
          :placeholder="t('detail.freshLinkPlaceholder')"
          :aria-label="t('downloads.refreshUrlAria')"
          @keydown.enter.prevent="resumeWithFreshLink"
        >
          <template #leading>
            <Link class="h-3.5 w-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
          </template>
        </Field>
        <Button size="sm" variant="primary" :disabled="!canResume" @click="resumeWithFreshLink">
          {{ t("detail.actionResume") }}
        </Button>
      </div>
      <div v-else class="flex gap-2">
        <Button size="sm" @click="retry">{{ t("addUrl.tryAgain") }}</Button>
      </div>
    </div>

    <section
      v-if="log.length > 0"
      :aria-label="t('detail.whatHappened')"
      class="flex flex-col gap-2"
    >
      <span class="text-xs font-semibold text-dl-soft">{{ t("detail.whatHappened") }}</span>
      <ol class="flex flex-col gap-1.5">
        <li
          v-for="row in log"
          :key="row.key"
          class="grid grid-cols-[68px_minmax(0,1fr)] items-baseline gap-2.5 text-xs"
        >
          <span class="whitespace-nowrap font-mono text-muted-foreground">{{ row.time }}</span>
          <span class="break-words">{{ row.text }}</span>
        </li>
      </ol>
    </section>
  </template>
</template>
