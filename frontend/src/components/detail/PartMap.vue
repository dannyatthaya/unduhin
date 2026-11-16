<script setup lang="ts">
// The part map: the file as one horizontal strip, one cell per part, each
// filling from the left as it downloads. A slow part is striped, outlined
// and carries a warning icon, so it never relies on colour alone.

import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { AlertTriangle } from "lucide-vue-next";

import { partWeight, type PartRow } from "@/lib/partRows";

/** Below this many parts a cell is wide enough to show its number. */
const NUMBERED_CELLS_MAX = 12;

const props = defineProps<{
  rows: PartRow[];
  /** The whole download is finished: draw every cell as done. */
  finished?: boolean;
}>();

const { t } = useI18n();

const showNumbers = computed(() => props.rows.length <= NUMBERED_CELLS_MAX);
const slowCount = computed(() => props.rows.filter((row) => row.isSlow).length);

const summary = computed(() =>
  props.finished
    ? t("detail.partMapAriaDone", { n: props.rows.length }, props.rows.length)
    : t("detail.partMapAria", { n: props.rows.length, slow: slowCount.value }, props.rows.length),
);

function cellRadius(index: number): string {
  const last = props.rows.length - 1;
  if (last === 0) return "rounded-lg";
  if (index === 0) return "rounded-l-lg rounded-r-sm";
  if (index === last) return "rounded-l-sm rounded-r-lg";
  return "rounded-sm";
}
</script>

<template>
  <div class="flex flex-col gap-2.5">
    <div role="img" :aria-label="summary" class="flex h-8 gap-0.5">
      <span
        v-for="(row, index) in rows"
        :key="row.index"
        class="relative block min-w-[3px] overflow-hidden"
        :class="[
          cellRadius(index),
          finished
            ? 'bg-dl-ok-soft shadow-[inset_0_0_0_1.5px_hsl(var(--success))]'
            : row.isSlow
              ? 'bg-dl-track shadow-[inset_0_0_0_1.5px_hsl(var(--warning))]'
              : 'bg-dl-track',
        ]"
        :style="{ flexGrow: partWeight(row), flexBasis: '0px' }"
      >
        <template v-if="!finished">
          <span
            class="absolute inset-y-0 left-0 transition-[width] duration-300 ease-out"
            :class="row.isSlow ? 'dl-stripes-warn' : 'bg-primary opacity-50'"
            :style="{ width: `${row.pct}%` }"
          />
          <span
            v-if="!row.isDone && row.pct > 0"
            class="absolute inset-y-0 w-0.5"
            :class="row.isSlow ? 'bg-warning' : 'bg-primary'"
            :style="{ left: `calc(${row.pct}% - 2px)` }"
          />
        </template>
        <span
          v-if="showNumbers"
          class="absolute left-1.5 top-2 font-mono text-xs font-medium leading-none text-foreground"
        >
          {{ row.index + 1 }}
        </span>
        <AlertTriangle
          v-if="row.isSlow && !finished && showNumbers"
          class="absolute right-[5px] top-[9px] h-[13px] w-[13px] text-warning"
          aria-hidden="true"
        />
      </span>
    </div>

    <div
      v-if="!finished"
      class="flex flex-wrap gap-x-3.5 gap-y-1.5 text-xs text-muted-foreground"
    >
      <span class="flex items-center gap-1.5">
        <span class="inline-block h-3 w-3 rounded-[3px] bg-primary opacity-50" />
        {{ t("detail.legendDownloaded") }}
      </span>
      <span class="flex items-center gap-1.5">
        <span class="inline-block h-3 w-[3px] rounded-[1px] bg-primary" />
        {{ t("detail.legendWriting") }}
      </span>
      <span class="flex items-center gap-1.5">
        <span
          class="dl-stripes-warn inline-block h-3 w-3 rounded-[3px] shadow-[inset_0_0_0_1px_hsl(var(--warning))]"
        />
        {{ t("detail.legendSlow") }}
      </span>
      <span class="flex items-center gap-1.5">
        <span class="inline-block h-3 w-3 rounded-[3px] bg-dl-track" />
        {{ t("detail.legendLeft") }}
      </span>
    </div>
  </div>
</template>
