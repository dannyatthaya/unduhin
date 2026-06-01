<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { ArrowRight, Check, Plus, Trash2 } from "lucide-vue-next";

import RuleRow from "@/components/settings/categories/RuleRow.vue";
import ChipInput from "@/components/settings/controls/ChipInput.vue";
import Select from "@/components/ui/Select.vue";
import { useCategoriesStore } from "@/stores/categories";
import { iconFor } from "@/lib/categoryIcons";
import type { Category, CategoryId, NewCategoryInput } from "@/types/tauri-bindings";

const { t } = useI18n();
const store = useCategoriesStore();

// "Rules" in the mockup map 1:1 to categories that have at least one
// extension. The fallback "Other" category is rendered as the locked
// "everything else" row at the bottom.
function isFallback(c: Category): boolean {
  return c.name === "Other";
}

const ruled = computed(() =>
  store.list.filter((c) => !isFallback(c) && c.extension_rules.length > 0),
);

const fallback = computed(() => store.list.find(isFallback) ?? null);

const targetableCategories = computed(() =>
  store.list.filter((c) => !isFallback(c)),
);

const targetOptions = computed(() =>
  targetableCategories.value.map((c) => ({ value: c.id, label: c.name })),
);

// Pending rule = the row the user is currently composing. It's local UI
// state, not persisted, until the user confirms — at which point the
// chips get merged into the chosen target category's extension_rules and
// the row disappears (the target's existing row, if any, picks them up).
interface PendingRule {
  extensions: string[];
  targetId: CategoryId | null;
}

const pending = ref<PendingRule | null>(null);

const pendingTarget = computed(() => {
  if (!pending.value?.targetId) return null;
  return store.byId.get(pending.value.targetId) ?? null;
});

const pendingIconOpt = computed(() =>
  iconFor(pendingTarget.value?.icon ?? "other"),
);

const canSavePending = computed(
  () =>
    pending.value != null &&
    pending.value.extensions.length > 0 &&
    pending.value.targetId != null,
);

function toInput(c: Category, overrides: Partial<NewCategoryInput> = {}): NewCategoryInput {
  return {
    name: c.name,
    icon: c.icon ?? null,
    default_output_path: c.default_output_path ?? null,
    extension_rules: c.extension_rules,
    ...overrides,
  };
}

async function updateExtensions(c: Category, extensions: string[]) {
  await store.update(c.id, toInput(c, { extension_rules: extensions }));
}

async function moveRuleTarget(from: Category, to: Category) {
  const merged = Array.from(new Set([...to.extension_rules, ...from.extension_rules]));
  await store.update(from.id, toInput(from, { extension_rules: [] }));
  await store.update(to.id, toInput(to, { extension_rules: merged }));
}

async function clearRule(c: Category) {
  await store.update(c.id, toInput(c, { extension_rules: [] }));
}

function startPending() {
  if (pending.value != null) return;
  const defaultTarget =
    targetableCategories.value.find((c) => c.extension_rules.length === 0) ??
    targetableCategories.value[0];
  pending.value = {
    extensions: [],
    targetId: defaultTarget?.id ?? null,
  };
}

function discardPending() {
  pending.value = null;
}

async function savePending() {
  if (!canSavePending.value || !pending.value) return;
  const target = store.byId.get(pending.value.targetId!);
  if (!target) {
    pending.value = null;
    return;
  }
  const merged = Array.from(
    new Set([...target.extension_rules, ...pending.value.extensions]),
  );
  await store.update(target.id, toInput(target, { extension_rules: merged }));
  pending.value = null;
}

function setPendingTarget(id: CategoryId) {
  if (pending.value) pending.value.targetId = id;
}
</script>

<template>
  <div class="overflow-hidden">
    <RuleRow
      v-for="cat in ruled"
      :key="cat.id"
      :category="cat"
      @update:extensions="(v) => updateExtensions(cat, v)"
      @update:category="(target) => moveRuleTarget(cat, target)"
      @remove="() => clearRule(cat)"
    />

    <!-- Pending (in-progress) rule the user is composing. Lives only in
         local state until they hit the check — at which point its chips
         merge into the selected target's existing rule. -->
    <div
      v-if="pending"
      class="flex items-center gap-3 border-t border-border/60 bg-primary/5 px-5 py-3 first:border-t-0"
    >
      <div class="flex flex-1 items-center gap-3">
        <ChipInput
          v-model="pending.extensions"
          :prefix="'.'"
          :placeholder="t('settings.categoriesRulePendingPlaceholder')"
        />
      </div>
      <ArrowRight class="h-4 w-4 shrink-0 text-muted-foreground" />
      <div class="w-48 shrink-0">
        <div class="flex items-center gap-2">
          <span
            class="flex h-7 w-7 items-center justify-center rounded-md"
            :class="pendingIconOpt.background"
          >
            <component
              :is="pendingIconOpt.icon"
              class="h-3.5 w-3.5"
              :class="pendingIconOpt.tone"
            />
          </span>
          <Select
            :model-value="pending.targetId ?? 0"
            :options="targetOptions"
            @update:model-value="(v) => setPendingTarget(Number(v))"
          />
        </div>
      </div>
      <button
        type="button"
        :disabled="!canSavePending"
        class="flex h-8 w-8 items-center justify-center rounded-md text-primary transition-colors hover:bg-primary/10 disabled:cursor-not-allowed disabled:opacity-30"
        :title="t('settings.categoriesRuleSave')"
        @click="savePending"
      >
        <Check class="h-3.5 w-3.5" />
      </button>
      <button
        type="button"
        class="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-danger"
        :title="t('settings.categoriesRuleDiscard')"
        @click="discardPending"
      >
        <Trash2 class="h-3.5 w-3.5" />
      </button>
    </div>

    <RuleRow
      v-if="fallback"
      :category="fallback"
      :locked="true"
      @remove="() => {}"
    />

    <button
      type="button"
      class="flex w-full items-center gap-1.5 border-t border-border/60 px-5 py-3 text-left text-xs text-primary transition-colors hover:bg-primary/5 disabled:cursor-not-allowed disabled:text-muted-foreground"
      :disabled="pending != null"
      @click="startPending"
    >
      <Plus class="h-3.5 w-3.5" />
      <span>{{ t("settings.categoriesRuleAdd") }}</span>
    </button>
  </div>
</template>
