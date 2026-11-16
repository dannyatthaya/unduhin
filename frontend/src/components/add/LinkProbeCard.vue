<script setup lang="ts">
// The card under the Link field in New download. It stays in one place and
// changes what it says: checking, ready, not a link, can't reach, or a
// torrent.

import { computed } from "vue";
import { useI18n } from "vue-i18n";
import {
  AlertCircle,
  AlertTriangle,
  CheckCircle2,
  Globe,
  Hourglass,
  Plug,
  RotateCw,
} from "lucide-vue-next";

import Button from "@/components/ui/Button.vue";
import KindBadge from "@/components/downloads/KindBadge.vue";

import type { LinkProbeState } from "@/composables/useLinkProbe";
import { formatBytes } from "@/lib/format";
import type { LinkProbe } from "@/types/tauri-bindings";

const props = defineProps<{
  state: LinkProbeState;
  result: LinkProbe | null;
  host: string;
  /** Name to show: the server's, or the link's last path segment. */
  name: string;
  /** Display name of a magnet / .torrent, when one is known. */
  torrentName?: string;
  /** The link is already in the list: swaps "Ready" for a warning tag. */
  duplicateLabel?: string;
  /** Id for `aria-describedby` on the Link field. */
  messageId?: string;
}>();

defineEmits<{ retry: [] }>();

const { t } = useI18n();

const isFailure = computed(
  () => props.state === "invalid" || props.state === "unreachable",
);

const sizeLabel = computed(() =>
  props.result?.total_bytes != null ? formatBytes(props.result.total_bytes) : "",
);
</script>

<template>
  <div
    v-if="state !== 'empty'"
    :id="messageId"
    :role="isFailure ? 'alert' : 'status'"
    class="flex h-14 items-center gap-3 rounded-[10px] border px-3"
    :class="
      isFailure
        ? 'border-danger bg-dl-fail-soft'
        : 'border-dl-line bg-dl-group'
    "
  >
    <template v-if="state === 'invalid'">
      <span
        class="flex h-8 w-8 shrink-0 items-center justify-center rounded-[9px] bg-dl-fail-soft text-danger"
      >
        <AlertCircle class="h-[17px] w-[17px]" aria-hidden="true" />
      </span>
      <span class="flex min-w-0 flex-1 flex-col gap-0.5">
        <span class="truncate font-medium">{{ t("addUrl.notALink") }}</span>
        <span class="truncate text-xs text-dl-soft">{{ t("addUrl.notALinkHint") }}</span>
      </span>
    </template>

    <template v-else-if="state === 'unreachable'">
      <span
        class="flex h-8 w-8 shrink-0 items-center justify-center rounded-[9px] bg-dl-fail-soft text-danger"
      >
        <Globe class="h-[17px] w-[17px]" aria-hidden="true" />
      </span>
      <span class="flex min-w-0 flex-1 flex-col gap-0.5">
        <span class="truncate font-medium">{{ t("addUrl.cantReach", { host }) }}</span>
        <span class="truncate text-xs text-dl-soft">{{ t("addUrl.cantReachHint") }}</span>
      </span>
      <Button size="xs" @click="$emit('retry')">
        <RotateCw class="h-[13px] w-[13px]" aria-hidden="true" />
        {{ t("addUrl.tryAgain") }}
      </Button>
    </template>

    <template v-else-if="state === 'checking'">
      <span
        class="flex h-8 w-8 shrink-0 items-center justify-center rounded-[9px] bg-muted text-dl-soft"
      >
        <Hourglass class="h-[17px] w-[17px]" aria-hidden="true" />
      </span>
      <span class="flex min-w-0 flex-1 flex-col gap-0.5">
        <span class="truncate font-medium">{{ t("addUrl.checking") }}</span>
        <span class="truncate text-xs text-dl-soft">{{ t("addUrl.checkingHint", { host }) }}</span>
      </span>
      <span
        class="flex h-[22px] shrink-0 items-center rounded-md bg-muted px-2 text-xs font-medium text-dl-soft"
      >
        {{ t("addUrl.canStartNow") }}
      </span>
    </template>

    <template v-else-if="state === 'torrent'">
      <KindBadge filename="" download-kind="torrent" />
      <span class="flex min-w-0 flex-1 flex-col gap-0.5">
        <span class="truncate font-medium" :title="torrentName">
          {{ torrentName || t("addUrl.torrentDetected") }}
        </span>
        <span class="truncate text-xs text-dl-soft">{{ t("addUrl.torrentHint") }}</span>
      </span>
    </template>

    <template v-else>
      <KindBadge :filename="name" />
      <span class="flex min-w-0 flex-1 flex-col gap-0.5">
        <span class="truncate font-medium" :title="name">
          {{ name || t("addUrl.nameUnknown") }}
        </span>
        <span class="flex min-w-0 items-center gap-3 overflow-hidden whitespace-nowrap text-xs">
          <span v-if="sizeLabel" class="font-mono text-dl-soft">{{ sizeLabel }}</span>
          <span v-else class="text-muted-foreground">{{ t("addUrl.sizeUnknown") }}</span>
          <!-- No answer from the server (a local link is not checked): say
               nothing about connections rather than guess. -->
          <span v-if="result" class="flex items-center gap-1 text-muted-foreground">
            <template v-if="result.resumable">
              <RotateCw class="h-3 w-3" aria-hidden="true" />
              {{ t("addUrl.canResume") }}
            </template>
            <template v-else>
              <Plug class="h-3 w-3" aria-hidden="true" />
              {{ t("addUrl.oneConnection") }}
            </template>
          </span>
        </span>
      </span>
      <span
        v-if="duplicateLabel"
        class="flex h-6 shrink-0 items-center gap-[5px] rounded-md bg-dl-warn-soft pl-1.5 pr-2 text-xs font-medium text-warning"
      >
        <AlertTriangle class="h-[13px] w-[13px]" aria-hidden="true" />
        {{ duplicateLabel }}
      </span>
      <span
        v-else
        class="flex h-6 shrink-0 items-center gap-[5px] rounded-md bg-dl-ok-soft pl-1.5 pr-2 text-xs font-medium text-success"
      >
        <CheckCircle2 class="h-[13px] w-[13px]" aria-hidden="true" />
        {{ t("addUrl.ready") }}
      </span>
    </template>
  </div>
</template>
