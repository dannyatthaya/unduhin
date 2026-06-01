<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { Boxes, RefreshCw } from "lucide-vue-next";

import Button from "@/components/ui/Button.vue";
import { useToast } from "@/composables/useToast";
import type { BrowserIntegrationStatus } from "@/composables/useBrowserStatus";

const props = defineProps<{
  status: BrowserIntegrationStatus | null;
  loading: boolean;
  testing: boolean;
}>();

const emit = defineEmits<{
  (e: "test-handoff"): void;
}>();

const { t } = useI18n();
const { push } = useToast();

const listening = computed(() => props.status?.pipe.listening ?? false);
const pipeName = computed(() => props.status?.pipe.name ?? "");
const handoffsThisWeek = computed(() => props.status?.handoffs_this_week ?? 0);
const lastHandoffAt = computed(() => props.status?.last_handoff_at ?? null);

function formatRelative(iso: string | null): string {
  if (!iso) return t("settings.browserHandoffNever");
  const then = Date.parse(iso);
  if (!Number.isFinite(then)) return t("settings.browserHandoffNever");
  const diffMs = Date.now() - then;
  if (diffMs < 60_000) return t("settings.browserHandoffJustNow");
  const mins = Math.floor(diffMs / 60_000);
  if (mins < 60) return t("settings.browserHandoffMinAgo", { n: mins });
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return t("settings.browserHandoffHourAgo", { n: hrs });
  const days = Math.floor(hrs / 24);
  return t("settings.browserHandoffDayAgo", { n: days });
}

const statusDotClass = computed(() =>
  listening.value ? "bg-success" : "bg-muted-foreground/40",
);

function handleTest() {
  if (!listening.value) {
    push(t("settings.browserTestNotListening"), "error");
    return;
  }
  emit("test-handoff");
}
</script>

<template>
  <article
    class="overflow-hidden rounded-lg border border-border bg-card text-card-foreground"
  >
    <div class="flex items-start justify-between gap-4 px-5 py-4">
      <div class="flex items-start gap-3">
        <div
          class="grid h-10 w-10 shrink-0 place-items-center rounded-md bg-primary/10 text-primary"
        >
          <Boxes class="h-5 w-5" />
        </div>
        <div class="flex flex-col gap-1">
          <div class="flex flex-wrap items-baseline gap-2">
            <span class="text-sm font-semibold">
              {{ t("settings.browserStatusHeading") }}
            </span>
            <span
              v-if="pipeName"
              class="font-mono text-xs text-muted-foreground"
            >
              {{ pipeName }}
            </span>
          </div>
          <div
            class="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground"
          >
            <span class="inline-flex items-center gap-1.5">
              <span class="h-1.5 w-1.5 rounded-full" :class="statusDotClass" />
              <span :class="listening ? 'text-success' : 'text-muted-foreground'">
                {{
                  listening
                    ? t("settings.browserStatusConnected")
                    : loading
                      ? t("settings.browserStatusStarting")
                      : t("settings.browserStatusDown")
                }}
              </span>
            </span>
            <span aria-hidden="true">·</span>
            <span>
              {{
                t("settings.browserCapturedThisWeek", { n: handoffsThisWeek })
              }}
            </span>
            <span aria-hidden="true">·</span>
            <span>
              {{
                t("settings.browserLastHandoff", {
                  when: formatRelative(lastHandoffAt),
                })
              }}
            </span>
          </div>
        </div>
      </div>
      <Button
        variant="secondary"
        size="sm"
        :disabled="testing || !listening"
        @click="handleTest"
      >
        <RefreshCw class="h-3.5 w-3.5" :class="testing ? 'animate-spin' : ''" />
        {{ t("settings.browserTestHandoff") }}
      </Button>
    </div>
  </article>
</template>
