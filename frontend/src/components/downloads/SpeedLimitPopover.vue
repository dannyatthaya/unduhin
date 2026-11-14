<script setup lang="ts">
// Toolbar button + popover for the global speed limit. Reads and writes
// the same `global_speed_limit_bps` setting as Settings → General
// (bytes per second, 0 = no limit), so both surfaces stay in step.

import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { ChevronDown, Gauge } from "lucide-vue-next";
import {
  PopoverContent,
  PopoverPortal,
  PopoverRoot,
  PopoverTrigger,
} from "reka-ui";

import Switch from "@/components/ui/Switch.vue";
import { useSettingsStore } from "@/stores/settings";
import { useToast } from "@/composables/useToast";
import { formatSpeedLimit } from "@/lib/format";

defineProps<{
  /** Narrow window: icon-only trigger. */
  iconOnly?: boolean;
}>();

const SETTING_KEY = "global_speed_limit_bps";
const KB = 1024;
const MB = KB * 1024;
const PRESETS_MB = [1, 5, 10, 25] as const;
const DEFAULT_LIMIT_MB = 10;

type Unit = "mb" | "kb";

const { t } = useI18n();
const settings = useSettingsStore();
const toast = useToast();

const open = ref(false);
const unit = ref<Unit>("mb");
const amount = ref<number>(DEFAULT_LIMIT_MB);

const limitBps = computed(() => {
  const v = settings.values[SETTING_KEY];
  return typeof v === "number" && v > 0 ? v : 0;
});
const enabled = computed(() => limitBps.value > 0);
const limitLabel = computed(() =>
  enabled.value ? formatSpeedLimit(limitBps.value) : t("downloads.speedLimitNone"),
);

function factorOf(u: Unit): number {
  return u === "mb" ? MB : KB;
}

// Follow the stored value (it can change from Settings too) but keep the
// last amount around while the limit is off, so re-enabling restores it.
watch(
  limitBps,
  (bps) => {
    if (bps <= 0) return;
    unit.value = bps >= MB ? "mb" : "kb";
    amount.value = Math.round((bps / factorOf(unit.value)) * 10) / 10;
  },
  { immediate: true },
);

async function save(bps: number) {
  try {
    await settings.set(SETTING_KEY, bps);
  } catch (e: unknown) {
    const detail = e instanceof Error ? e.message : String(e);
    toast.push(t("downloads.speedLimitSaveFailed", { error: detail }), "error");
  }
}

function currentBps(): number | null {
  if (!Number.isFinite(amount.value) || amount.value <= 0) return null;
  return Math.round(amount.value * factorOf(unit.value));
}

function setEnabled(next: boolean) {
  if (!next) {
    void save(0);
    return;
  }
  if (currentBps() == null) amount.value = DEFAULT_LIMIT_MB;
  const bps = currentBps();
  if (bps != null) void save(bps);
}

function onAmountChange(e: Event) {
  amount.value = Number((e.target as HTMLInputElement).value);
  const bps = currentBps();
  if (enabled.value && bps != null) void save(bps);
}

function onUnitChange(e: Event) {
  unit.value = (e.target as HTMLSelectElement).value === "kb" ? "kb" : "mb";
  const bps = currentBps();
  if (enabled.value && bps != null) void save(bps);
}

function applyPreset(mb: number) {
  unit.value = "mb";
  amount.value = mb;
  void save(mb * MB);
}

function isPresetActive(mb: number): boolean {
  return limitBps.value === mb * MB;
}

const FIELD =
  "h-9 rounded-[9px] border border-input bg-dl-field text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";
</script>

<template>
  <PopoverRoot v-model:open="open">
    <PopoverTrigger as-child>
      <button
        type="button"
        class="flex h-9 shrink-0 items-center gap-1.5 rounded-[9px] border font-medium text-foreground transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
        :class="[
          iconOnly ? 'px-2' : 'px-2.5',
          enabled
            ? 'border-dl-accent-ring bg-dl-accent-soft'
            : 'border-border bg-transparent hover:bg-accent',
        ]"
        :aria-label="t('downloads.speedLimitAria', { limit: limitLabel })"
        :title="t('downloads.speedLimit')"
      >
        <Gauge class="h-[15px] w-[15px]" aria-hidden="true" />
        <span v-if="!iconOnly" class="whitespace-nowrap">{{ limitLabel }}</span>
        <ChevronDown class="h-[11px] w-[11px] text-muted-foreground" aria-hidden="true" />
      </button>
    </PopoverTrigger>

    <PopoverPortal>
      <PopoverContent
        align="end"
        :side-offset="8"
        :aria-label="t('downloads.speedLimit')"
        class="dl-surface z-50 flex w-80 flex-col gap-3.5 rounded-[14px] border border-border bg-card p-4 text-foreground shadow-[0_18px_48px_rgba(0,0,0,0.35)] outline-none data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0"
      >
        <div class="flex items-center gap-3">
          <span class="flex-1 font-semibold">{{ t("downloads.speedLimitToggle") }}</span>
          <Switch
            :model-value="enabled"
            :aria-label="t('downloads.speedLimitToggle')"
            @update:model-value="setEnabled"
          />
        </div>

        <div class="flex items-end gap-2">
          <label class="flex flex-1 flex-col gap-1.5 text-xs text-muted-foreground">
            {{ t("downloads.speedLimitMax") }}
            <input
              type="number"
              min="0.1"
              step="0.1"
              :value="amount"
              :class="[FIELD, 'px-2.5 font-mono text-sm']"
              @change="onAmountChange"
            />
          </label>
          <select
            :value="unit"
            :aria-label="t('downloads.speedLimitUnit')"
            :class="[FIELD, 'px-2']"
            @change="onUnitChange"
          >
            <option value="mb">MB/s</option>
            <option value="kb">KB/s</option>
          </select>
        </div>

        <div role="group" :aria-label="t('downloads.speedLimitPresets')" class="flex gap-1.5">
          <button
            v-for="mb in PRESETS_MB"
            :key="mb"
            type="button"
            class="h-[30px] flex-1 rounded-lg border text-xs font-medium transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
            :class="
              isPresetActive(mb)
                ? 'border-primary bg-dl-accent-soft text-foreground'
                : 'border-border bg-transparent text-dl-soft hover:bg-accent'
            "
            :aria-pressed="isPresetActive(mb)"
            @click="applyPreset(mb)"
          >
            {{ mb }} MB/s
          </button>
        </div>

        <p class="text-xs leading-[1.45] text-muted-foreground">
          {{ t("downloads.speedLimitNote") }}
        </p>
      </PopoverContent>
    </PopoverPortal>
  </PopoverRoot>
</template>
