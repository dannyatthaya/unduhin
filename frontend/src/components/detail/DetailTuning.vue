<script setup lang="ts">
// "This file only": the settings one download can override — its own speed
// limit, its connections, and whether finished connections help slow
// parts. They sit together on Overview so there is one place to look.

import { computed, ref, useId, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useDebounceFn } from "@vueuse/core";

import Select, { type SelectOption } from "@/components/ui/Select.vue";
import Switch from "@/components/ui/Switch.vue";
import NumberStepper from "@/components/settings/controls/NumberStepper.vue";

import { useDownloadsStore } from "@/stores/downloads";
import { useGeneralSettings } from "@/composables/useGeneralSettings";
import { useToast } from "@/composables/useToast";
import { formatSpeedLimit } from "@/lib/format";
import { api, type DownloadRecord } from "@/types/tauri-bindings";

const MIB = 1024 * 1024;
/** Limits offered in the list; a value set elsewhere is added to it. */
const LIMIT_PRESETS_BPS = [512 * 1024, MIB, 2 * MIB, 5 * MIB, 10 * MIB, 20 * MIB];
const NO_LIMIT = 0;
const MIN_CONNECTIONS = 1;
const MAX_CONNECTIONS = 32;
/** A burst of stepper clicks sends one request. */
const CONNECTIONS_DEBOUNCE_MS = 400;

const props = defineProps<{ download: DownloadRecord }>();

const { t } = useI18n();
const downloads = useDownloadsStore();
const general = useGeneralSettings();
const toast = useToast();

const limitId = useId();
const connectionsId = useId();
const helpId = useId();
const sectionId = useId();

/** Only the HTTP engine has connections to tune. */
const isHttp = computed(() => props.download.kind === "http");

function messageOf(e: unknown): string {
  return (e as { message?: string })?.message ?? String(e);
}

// ── Speed limit ─────────────────────────────────────────────────────────

const limit = computed(() => props.download.speed_limit_bps ?? NO_LIMIT);

const limitOptions = computed<SelectOption<number>[]>(() => {
  const values = LIMIT_PRESETS_BPS.includes(limit.value) || limit.value === NO_LIMIT
    ? LIMIT_PRESETS_BPS
    : [...LIMIT_PRESETS_BPS, limit.value].sort((a, b) => a - b);
  return [
    { value: NO_LIMIT, label: t("detail.noLimit") },
    ...values.map((bps) => ({ value: bps, label: formatSpeedLimit(bps) })),
  ];
});

async function setLimit(bps: number) {
  try {
    downloads.putRecord(
      await api.setDownloadSpeedLimit(props.download.id, bps === NO_LIMIT ? null : bps),
    );
  } catch (e: unknown) {
    toast.push(t("detail.tuningFailed", { error: messageOf(e) }), "error");
  }
}

// ── Connections ─────────────────────────────────────────────────────────

const connections = ref(props.download.segments);
watch(
  () => props.download.segments,
  (n) => {
    connections.value = n;
  },
);

const pushConnections = useDebounceFn(async (n: number) => {
  try {
    await api.setSegments(props.download.id, n);
  } catch (e: unknown) {
    toast.push(t("errors.setSegments", { error: messageOf(e) }), "error");
    connections.value = props.download.segments;
  }
}, CONNECTIONS_DEBOUNCE_MS);

function setConnections(n: number) {
  connections.value = n;
  void pushConnections(n);
}

// ── Help slow parts ─────────────────────────────────────────────────────

async function setHelpSlowParts(enabled: boolean) {
  try {
    downloads.putRecord(await api.setHelpSlowParts(props.download.id, enabled));
  } catch (e: unknown) {
    toast.push(t("detail.tuningFailed", { error: messageOf(e) }), "error");
  }
}

// ── Reset ───────────────────────────────────────────────────────────────

const isDefault = computed(
  () =>
    limit.value === NO_LIMIT &&
    (!isHttp.value ||
      (props.download.help_slow_parts &&
        props.download.segments === general.defaultSegments.value)),
);

async function resetToDefaults() {
  await setLimit(NO_LIMIT);
  if (!isHttp.value) return;
  if (!props.download.help_slow_parts) await setHelpSlowParts(true);
  if (props.download.segments !== general.defaultSegments.value) {
    setConnections(general.defaultSegments.value);
  }
}

const ROW = "flex items-center justify-between gap-4 border-b border-dl-line-soft py-3 last:border-b-0 last:pb-0";
const DESC = "text-xs leading-[1.45] text-muted-foreground";
</script>

<template>
  <section :aria-labelledby="sectionId" class="flex flex-col">
    <div class="flex min-h-6 items-center justify-between gap-2">
      <span :id="sectionId" class="text-xs font-semibold text-dl-soft">
        {{ t("detail.thisFileOnly") }}
      </span>
      <button
        v-if="!isDefault"
        type="button"
        class="-mr-1.5 flex h-6 items-center rounded-md px-1.5 text-xs font-medium text-primary transition-colors hover:bg-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
        @click="resetToDefaults"
      >
        {{ t("detail.resetToDefaults") }}
      </button>
    </div>

    <div :class="ROW">
      <span class="flex min-w-0 flex-col gap-0.5">
        <label :for="limitId" class="font-medium">{{ t("detail.speedLimit") }}</label>
        <span :class="DESC">
          {{ isHttp ? t("detail.speedLimitDesc") : t("detail.speedLimitDescMedia") }}
        </span>
      </span>
      <div class="w-[124px] shrink-0">
        <Select
          :id="limitId"
          :model-value="limit"
          :options="limitOptions"
          @update:model-value="setLimit"
        />
      </div>
    </div>

    <template v-if="isHttp">
      <div :class="ROW">
        <span class="flex min-w-0 flex-col gap-0.5">
          <span :id="connectionsId" class="font-medium">{{ t("addUrl.connectionsLabel") }}</span>
          <span :class="DESC">{{ t("detail.connectionsDesc") }}</span>
        </span>
        <NumberStepper
          :model-value="connections"
          size="sm"
          :min="MIN_CONNECTIONS"
          :max="MAX_CONNECTIONS"
          :labelledby="connectionsId"
          :decrease-label="t('addUrl.fewerConnections')"
          :increase-label="t('addUrl.moreConnections')"
          @update:model-value="setConnections"
        />
      </div>

      <div :class="ROW">
        <span class="flex min-w-0 flex-col gap-0.5">
          <span :id="helpId" class="font-medium">{{ t("detail.helpSlowParts") }}</span>
          <span :class="DESC">{{ t("detail.helpSlowPartsDesc") }}</span>
        </span>
        <Switch
          :model-value="download.help_slow_parts"
          :labelledby="helpId"
          @update:model-value="setHelpSlowParts"
        />
      </div>
    </template>
  </section>
</template>
