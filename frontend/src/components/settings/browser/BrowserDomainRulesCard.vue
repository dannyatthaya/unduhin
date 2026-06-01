<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { VueDraggable } from "vue-draggable-plus";
import { Plus, Upload, Download } from "lucide-vue-next";

import Button from "@/components/ui/Button.vue";
import Input from "@/components/ui/Input.vue";
import Dialog from "@/components/ui/Dialog.vue";
import RuleRow from "@/components/settings/browser/RuleRow.vue";
import { useToast } from "@/composables/useToast";
import type { useBrowserSettings } from "@/composables/useBrowserSettings";
import { useRuleMetrics } from "@/composables/useRuleMetrics";
import type { HostRule } from "@/types/wire";

import {
  parseRuleExport,
  serializeRuleExport,
  type RuleExportV1,
} from "@/lib/ruleExport";

const props = defineProps<{
  settings: ReturnType<typeof useBrowserSettings>;
}>();

const { t } = useI18n();
const { push } = useToast();
const metrics = useRuleMetrics();

// Pending import — when set, the confirm dialog is open. The user's
// choice resolves the captured promise.
const pendingImport = ref<{
  parsed: RuleExportV1;
  resolve: (ok: boolean) => void;
} | null>(null);

function answerImport(ok: boolean): void {
  const pending = pendingImport.value;
  pendingImport.value = null;
  pending?.resolve(ok);
}

const importMessage = computed(() => {
  const p = pendingImport.value;
  if (!p) return "";
  return t("settings.browserRulesImportConfirm", {
    currentBlock: blockedDraggable.value.length,
    currentAllow: allowDraggable.value.length,
    nextBlock: p.parsed.blocked.length,
    nextAllow: p.parsed.always.length,
  });
});

type RuleKind = "block" | "allow";

interface Row {
  rule: HostRule;
  kind: RuleKind;
}

// Combined list — block rules first (matches the mockup's order). The
// drag-reorder commits each list independently so the kind stays
// pinned; cross-kind drag is not allowed.
const blockedDraggable = computed<HostRule[]>({
  get: () => [...props.settings.bindings.blockedHosts.value],
  set: (next) => props.settings.setRules("block", next),
});

const allowDraggable = computed<HostRule[]>({
  get: () => [...props.settings.bindings.alwaysInterceptHosts.value],
  set: (next) => props.settings.setRules("allow", next),
});

const draftKind = ref<RuleKind>("block");
const draftPattern = ref("");
const draftError = ref<string | null>(null);

const VALID_PATTERN = /^(\*\.)?[a-z0-9.-]{1,253}$/i;

function commitAdd(): void {
  const pattern = draftPattern.value.trim().toLowerCase();
  if (!pattern) {
    draftError.value = t("settings.browserRulePatternRequired");
    return;
  }
  if (!VALID_PATTERN.test(pattern)) {
    draftError.value = t("settings.browserRulePatternInvalid");
    return;
  }
  const existsBlock = blockedDraggable.value.some((r) => r.pattern === pattern);
  const existsAllow = allowDraggable.value.some((r) => r.pattern === pattern);
  if (existsBlock || existsAllow) {
    draftError.value = t("settings.browserRulePatternDuplicate");
    return;
  }
  const entry: HostRule = { pattern, addedAt: Date.now() };
  if (draftKind.value === "block") {
    props.settings.setRules("block", [entry, ...blockedDraggable.value]);
  } else {
    props.settings.setRules("allow", [entry, ...allowDraggable.value]);
  }
  draftPattern.value = "";
  draftError.value = null;
}

function removeRule(kind: RuleKind, pattern: string): void {
  if (kind === "block") {
    props.settings.setRules(
      "block",
      blockedDraggable.value.filter((r) => r.pattern !== pattern),
    );
  } else {
    props.settings.setRules(
      "allow",
      allowDraggable.value.filter((r) => r.pattern !== pattern),
    );
  }
}

// Browser-native I/O — the Tauri webview supports both. Avoids
// pulling in `tauri-plugin-fs` for a single round-trip pair.
const fileInput = ref<HTMLInputElement | null>(null);

function onExport(): void {
  try {
    const body = serializeRuleExport({
      blocked: [...blockedDraggable.value],
      always: [...allowDraggable.value],
    });
    const blob = new Blob([body], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = "unduhin-rules.json";
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
    push(t("settings.browserRulesExportOk"), "success");
  } catch (err) {
    push(
      (err as { message?: string })?.message ??
        t("settings.browserRulesExportFailed"),
      "error",
    );
  }
}

function triggerImport(): void {
  fileInput.value?.click();
}

async function onImportFileChosen(event: Event): Promise<void> {
  const target = event.target as HTMLInputElement;
  const file = target.files?.[0];
  target.value = "";
  if (!file) return;
  try {
    const text = await file.text();
    let parsed: RuleExportV1;
    try {
      parsed = parseRuleExport(text);
    } catch (err) {
      push(
        (err as Error).message ?? t("settings.browserRulesImportInvalid"),
        "error",
      );
      return;
    }
    const ok = await new Promise<boolean>((resolve) => {
      pendingImport.value = { parsed, resolve };
    });
    if (!ok) return;
    props.settings.setRules("block", parsed.blocked);
    props.settings.setRules("allow", parsed.always);
    push(t("settings.browserRulesImportOk"), "success");
  } catch (err) {
    push(
      (err as { message?: string })?.message ??
        t("settings.browserRulesImportFailed"),
      "error",
    );
  }
}

const totalRules = computed(
  () => blockedDraggable.value.length + allowDraggable.value.length,
);
</script>

<template>
  <article
    class="overflow-hidden rounded-lg border border-border bg-card text-card-foreground"
  >
    <header class="flex items-start justify-between gap-3 px-5 py-4">
      <div class="flex flex-col gap-1">
        <h2 class="text-sm font-semibold">
          {{ t("settings.cardBrowserDomainRules") }}
        </h2>
        <p class="text-xs text-muted-foreground">
          {{ t("settings.cardBrowserDomainRulesDesc") }}
        </p>
      </div>
      <div class="flex shrink-0 items-center gap-2">
        <input
          ref="fileInput"
          type="file"
          accept="application/json,.json"
          class="hidden"
          @change="onImportFileChosen"
        />
        <Button variant="ghost" size="sm" @click="triggerImport">
          <Upload class="h-3.5 w-3.5" />
          {{ t("settings.browserRulesImport") }}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          :disabled="totalRules === 0"
          @click="onExport"
        >
          <Download class="h-3.5 w-3.5" />
          {{ t("settings.browserRulesExport") }}
        </Button>
      </div>
    </header>

    <div class="border-t border-border">
      <p
        v-if="totalRules === 0"
        class="px-5 py-6 text-center text-xs text-muted-foreground"
      >
        {{ t("settings.browserRulesEmpty") }}
      </p>

      <div v-if="blockedDraggable.length > 0">
        <p
          class="bg-muted/40 px-4 py-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground"
        >
          {{ t("settings.browserRulesBlockedHeading") }}
        </p>
        <VueDraggable
          v-model="blockedDraggable"
          handle=".drag-handle"
          :animation="150"
          tag="div"
        >
          <RuleRow
            v-for="rule in blockedDraggable"
            :key="`b-${rule.pattern}`"
            :rule="rule"
            kind="block"
            :match-count="metrics.getMatchCount(rule.pattern)"
            :last-match-at="metrics.getLastMatchAt(rule.pattern)"
            @remove="() => removeRule('block', rule.pattern)"
          />
        </VueDraggable>
      </div>

      <div v-if="allowDraggable.length > 0">
        <p
          class="bg-muted/40 px-4 py-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground"
        >
          {{ t("settings.browserRulesAllowHeading") }}
        </p>
        <VueDraggable
          v-model="allowDraggable"
          handle=".drag-handle"
          :animation="150"
          tag="div"
        >
          <RuleRow
            v-for="rule in allowDraggable"
            :key="`a-${rule.pattern}`"
            :rule="rule"
            kind="allow"
            :match-count="metrics.getMatchCount(rule.pattern)"
            :last-match-at="metrics.getLastMatchAt(rule.pattern)"
            @remove="() => removeRule('allow', rule.pattern)"
          />
        </VueDraggable>
      </div>
    </div>

    <Dialog
      :open="pendingImport !== null"
      :title="t('settings.browserRulesImportTitle')"
      size="md"
      @close="answerImport(false)"
    >
      <p class="text-sm text-muted-foreground">{{ importMessage }}</p>
      <template #footer>
        <Button variant="ghost" @click="answerImport(false)">
          {{ t("common.cancel") }}
        </Button>
        <Button @click="answerImport(true)">
          {{ t("common.continue") }}
        </Button>
      </template>
    </Dialog>

    <footer class="border-t border-border px-4 py-3">
      <div class="flex flex-wrap items-center gap-2">
        <select
          v-model="draftKind"
          class="h-8 rounded-md border border-input bg-background px-2 text-xs"
        >
          <option value="block">{{ t("settings.browserRulePillBlock") }}</option>
          <option value="allow">{{ t("settings.browserRulePillAllow") }}</option>
        </select>
        <Input
          v-model="draftPattern"
          class="h-8 flex-1 min-w-[180px] text-xs"
          :placeholder="t('settings.browserRulePatternPlaceholder')"
          @keydown.enter="commitAdd"
        />
        <Button size="sm" class="h-8" @click="commitAdd">
          <Plus class="h-3.5 w-3.5" />
          {{ t("settings.browserRuleAdd") }}
        </Button>
      </div>
      <p v-if="draftError" class="mt-2 text-xs text-destructive">
        {{ draftError }}
      </p>
    </footer>
  </article>
</template>
