<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { GripVertical, Trash2 } from "lucide-vue-next";

import type { HostRule } from "@/types/wire";

const props = defineProps<{
  rule: HostRule;
  kind: "block" | "allow";
  matchCount: number;
  lastMatchAt: number | null;
}>();

const emit = defineEmits<{
  (e: "remove"): void;
}>();

const { t, locale } = useI18n();

const dateFormatter = computed(
  () =>
    new Intl.DateTimeFormat(locale.value, {
      month: "short",
      day: "numeric",
      year: "numeric",
    }),
);

const addedLabel = computed(() => {
  if (props.rule.addedAt > 0) {
    return t("settings.browserRuleAdded", {
      when: dateFormatter.value.format(new Date(props.rule.addedAt)),
    });
  }
  return t("settings.browserRuleAddedUnknown");
});

const matchLabel = computed(() =>
  t("settings.browserRuleMatches", { n: props.matchCount }),
);
</script>

<template>
  <div
    class="flex items-center gap-3 border-b border-border px-4 py-2 last:border-b-0"
    :data-pattern="props.rule.pattern"
  >
    <button
      type="button"
      class="drag-handle cursor-grab text-muted-foreground hover:text-foreground"
      :aria-label="t('settings.browserRuleDragHandle')"
    >
      <GripVertical class="h-4 w-4" />
    </button>
    <span
      class="inline-flex shrink-0 items-center rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide"
      :class="
        props.kind === 'block'
          ? 'bg-destructive/10 text-destructive'
          : 'bg-success/10 text-success'
      "
    >
      {{
        props.kind === "block"
          ? t("settings.browserRulePillBlock")
          : t("settings.browserRulePillAllow")
      }}
    </span>
    <div class="flex min-w-0 flex-1 flex-col">
      <span class="truncate font-mono text-sm">{{ props.rule.pattern }}</span>
      <span class="text-xs text-muted-foreground">
        {{ matchLabel }} · {{ addedLabel }}
      </span>
    </div>
    <button
      type="button"
      class="rounded-md p-1.5 text-muted-foreground hover:bg-accent hover:text-foreground"
      :aria-label="t('settings.browserRuleRemove')"
      @click="emit('remove')"
    >
      <Trash2 class="h-4 w-4" />
    </button>
  </div>
</template>
