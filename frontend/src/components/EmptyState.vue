<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import {
  Plus,
  ClipboardPaste,
  FolderOpen,
  ArrowDownUp,
  Puzzle,
  ArrowUpRight,
  CheckCircle2,
  PauseCircle,
  Clock,
  AlertTriangle,
  XCircle,
  Inbox,
  Filter,
} from "lucide-vue-next";
import Button from "./ui/Button.vue";
import type { Status } from "@/types/tauri-bindings";

// "welcome" is the first-launch full pitch with the three cards and the
// extension teaser. The status variants are short, honest messages that
// replace the bare "No downloads match the current filter." paragraph
// the per-status filter used to render. "filtered" is the catch-all when
// the search/category combination has no matches.
export type EmptyVariant = Status | "welcome" | "filtered";

const { t } = useI18n();

const props = withDefaults(
  defineProps<{ variant?: EmptyVariant }>(),
  { variant: "welcome" },
);

defineEmits<{ "add-url": [] }>();

const statusIcons: Record<Exclude<EmptyVariant, "welcome">, typeof CheckCircle2> = {
  active: ArrowDownUp,
  queued: Clock,
  paused: PauseCircle,
  completed: CheckCircle2,
  failed: AlertTriangle,
  cancelled: XCircle,
  muxing: ArrowDownUp,
  filtered: Filter,
};

const statusTitleKey: Record<Exclude<EmptyVariant, "welcome">, string> = {
  active: "downloads.emptyActiveTitle",
  queued: "downloads.emptyQueuedTitle",
  paused: "downloads.emptyPausedTitle",
  completed: "downloads.emptyCompletedTitle",
  failed: "downloads.emptyFailedTitle",
  cancelled: "downloads.emptyCancelledTitle",
  muxing: "downloads.emptyMuxingTitle",
  filtered: "downloads.emptyFilteredTitle",
};

const statusHintKey: Record<Exclude<EmptyVariant, "welcome">, string> = {
  active: "downloads.emptyActiveHint",
  queued: "downloads.emptyQueuedHint",
  paused: "downloads.emptyPausedHint",
  completed: "downloads.emptyCompletedHint",
  failed: "downloads.emptyFailedHint",
  cancelled: "downloads.emptyCancelledHint",
  muxing: "downloads.emptyMuxingHint",
  filtered: "downloads.emptyFilteredHint",
};

const status = computed(() => {
  if (props.variant === "welcome") return null;
  const key = props.variant as Exclude<EmptyVariant, "welcome">;
  return {
    icon: statusIcons[key] ?? Inbox,
    title: t(statusTitleKey[key] ?? "downloads.emptyFilteredTitle"),
    hint: t(statusHintKey[key] ?? "downloads.emptyFilteredHint"),
  };
});
</script>

<template>
  <section
    v-if="status"
    class="mx-auto flex max-w-md flex-col items-center px-6 py-16 text-center"
  >
    <div
      class="flex h-12 w-12 items-center justify-center rounded-full bg-muted text-muted-foreground"
    >
      <component :is="status.icon" class="h-6 w-6" />
    </div>
    <h2 class="mt-4 font-serif text-xl font-bold tracking-tight">
      {{ status.title }}
    </h2>
    <p class="mt-1.5 max-w-xs text-sm text-muted-foreground">
      {{ status.hint }}
    </p>
  </section>

  <section v-else class="mx-auto max-w-6xl px-2">
    <header class="flex items-end justify-between gap-6">
      <div class="max-w-2xl">
        <p class="text-xs font-semibold uppercase tracking-[0.18em] text-primary">
          {{ t("downloads.emptyWelcomeTitle") }}
        </p>
        <h2 class="mt-2 font-serif text-4xl font-black tracking-tight">
          {{ t("downloads.emptyWelcomeSubtitle") }}
        </h2>
        <p class="mt-2 text-sm text-muted-foreground">
          {{ t("downloads.emptyWelcomeIntro") }}
        </p>
      </div>
      <Button variant="primary" size="md" class="shrink-0" @click="$emit('add-url')">
        <Plus class="h-4 w-4" />
        {{ t("downloads.emptyWelcomeAddFirst") }}
      </Button>
    </header>

    <div class="mt-8 grid grid-cols-1 gap-4 md:grid-cols-3">
      <!-- 01 -->
      <article class="relative overflow-hidden rounded-xl border border-border bg-card p-5">
        <span
          class="pointer-events-none absolute right-4 top-3 select-none font-serif text-5xl font-black text-primary/10"
        >
          01
        </span>
        <div
          class="flex h-10 w-10 items-center justify-center rounded-lg bg-primary/10 text-primary"
        >
          <ClipboardPaste class="h-5 w-5" />
        </div>
        <h3 class="mt-4 font-serif text-lg font-bold">{{ t("downloads.emptyWelcomeCard1Title") }}</h3>
        <p class="mt-1.5 text-sm leading-relaxed text-muted-foreground">
          {{ t("downloads.emptyWelcomeCard1Body") }}
        </p>
        <div class="mt-4 flex items-center gap-1.5 font-mono text-[11px] text-muted-foreground">
          <span>{{ t("downloads.emptyWelcomeCard1Shortcut") }}</span>
        </div>
      </article>

      <!-- 02 -->
      <article class="relative overflow-hidden rounded-xl border border-border bg-card p-5">
        <span
          class="pointer-events-none absolute right-4 top-3 select-none font-serif text-5xl font-black text-primary/10"
        >
          02
        </span>
        <div
          class="flex h-10 w-10 items-center justify-center rounded-lg bg-primary/10 text-primary"
        >
          <FolderOpen class="h-5 w-5" />
        </div>
        <h3 class="mt-4 font-serif text-lg font-bold">{{ t("downloads.emptyWelcomeCard2Title") }}</h3>
        <p class="mt-1.5 text-sm leading-relaxed text-muted-foreground">
          {{ t("downloads.emptyWelcomeCard2Body") }}
        </p>
        <div class="mt-4">
          <span
            class="inline-flex items-center rounded-md bg-primary/10 px-2 py-0.5 font-mono text-[11px] text-primary"
          >
            {{ t("downloads.emptyWelcomeCard2Hint") }}
          </span>
        </div>
      </article>

      <!-- 03 -->
      <article class="relative overflow-hidden rounded-xl border border-border bg-card p-5">
        <span
          class="pointer-events-none absolute right-4 top-3 select-none font-serif text-5xl font-black text-primary/10"
        >
          03
        </span>
        <div
          class="flex h-10 w-10 items-center justify-center rounded-lg bg-primary/10 text-primary"
        >
          <ArrowDownUp class="h-5 w-5" />
        </div>
        <h3 class="mt-4 font-serif text-lg font-bold">{{ t("downloads.emptyWelcomeCard3Title") }}</h3>
        <p class="mt-1.5 text-sm leading-relaxed text-muted-foreground">
          {{ t("downloads.emptyWelcomeCard3Body") }}
        </p>
        <div class="mt-4 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-success">
          <span class="font-medium">✓ {{ t("downloads.emptyWelcomeBadgeResumable") }}</span>
          <span class="text-muted-foreground">·</span>
          <span class="font-medium">{{ t("downloads.emptyWelcomeBadgeParallel") }}</span>
          <span class="text-muted-foreground">·</span>
          <span class="font-medium">{{ t("downloads.emptyWelcomeBadgeVerified") }}</span>
        </div>
      </article>
    </div>

    <aside
      class="mt-6 flex flex-col gap-3 rounded-xl bg-primary px-5 py-4 text-primary-foreground sm:flex-row sm:items-center"
    >
      <div
        class="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-white/15"
      >
        <Puzzle class="h-5 w-5" />
      </div>
      <div class="flex-1">
        <p class="text-[11px] font-semibold uppercase tracking-[0.18em] text-white/70">
          {{ t("downloads.emptyWelcomeAsideTitle") }}
        </p>
        <h3 class="font-serif text-base font-bold">
          {{ t("downloads.emptyWelcomeAsideBody") }}
        </h3>
        <p class="mt-0.5 text-xs text-white/85">
          {{ t("downloads.emptyWelcomeAsideHint") }}
        </p>
      </div>
      <button
        class="inline-flex shrink-0 items-center gap-1.5 rounded-md bg-background px-3 py-1.5 text-xs font-semibold text-foreground transition-colors hover:bg-background/90"
      >
        {{ t("downloads.emptyWelcomeNotifyMe") }}
        <ArrowUpRight class="h-3.5 w-3.5" />
      </button>
    </aside>
  </section>
</template>
