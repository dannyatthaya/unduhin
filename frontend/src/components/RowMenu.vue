<script setup lang="ts">
// Minimal teleported dropdown for the per-row "More" button.
//
// Hand-rolled for now — when the shared right-click context menu
// lands, we can lift both into a shared `Menu` primitive.

import { nextTick, onBeforeUnmount, ref, watch } from "vue";
import { onClickOutside, useEventListener } from "@vueuse/core";
import { useI18n } from "vue-i18n";

import { useOverlaysStore } from "@/stores/overlays";

const props = defineProps<{ items: { label: string; danger?: boolean; onSelect: () => void }[] }>();

const { t } = useI18n();

const open = ref(false);
const triggerRef = ref<HTMLButtonElement | null>(null);
const menuRef = ref<HTMLDivElement | null>(null);
const menuStyle = ref<Record<string, string>>({});

const MENU_WIDTH = 200;

// Coordinate with other transient overlays — when something else
// claims the slot, close ourselves.
const overlays = useOverlaysStore();
const overlayId = Symbol("row-menu");
watch(
  () => overlays.activeId,
  (id) => {
    if (open.value && id !== overlayId) {
      open.value = false;
    }
  },
);

async function toggle() {
  if (open.value) {
    open.value = false;
    return;
  }
  open.value = true;
  overlays.claim(overlayId);
  await nextTick();
  if (!triggerRef.value) return;
  const r = triggerRef.value.getBoundingClientRect();
  const left = `${Math.max(8, r.right - MENU_WIDTH)}px`;
  // Estimate menu height from item count; refined below once mounted.
  // ~32px per item + 8px py-1 padding.
  const estimated = props.items.length * 32 + 8;
  const margin = 8;
  const placeAbove = r.bottom + estimated + margin > window.innerHeight;
  menuStyle.value = placeAbove
    ? {
        bottom: `${window.innerHeight - r.top + 4}px`,
        left,
        width: `${MENU_WIDTH}px`,
      }
    : { top: `${r.bottom + 4}px`, left, width: `${MENU_WIDTH}px` };

  // Once the actual menu is in the DOM, re-check with the real height
  // — fixes the case where item labels wrap and the estimate is off.
  await nextTick();
  if (!menuRef.value || !triggerRef.value) return;
  const real = menuRef.value.offsetHeight;
  const trig = triggerRef.value.getBoundingClientRect();
  const flipNow = trig.bottom + real + margin > window.innerHeight;
  if (flipNow) {
    menuStyle.value = {
      bottom: `${window.innerHeight - trig.top + 4}px`,
      left,
      width: `${MENU_WIDTH}px`,
    };
  } else if (placeAbove && !flipNow) {
    // Estimate said "above" but the real height fits below — switch
    // back so the menu doesn't float in empty space above the row.
    menuStyle.value = {
      top: `${trig.bottom + 4}px`,
      left,
      width: `${MENU_WIDTH}px`,
    };
  }
}

function close() {
  if (open.value) {
    open.value = false;
    overlays.release(overlayId);
  }
}

function pick(action: () => void) {
  close();
  action();
}

onClickOutside(menuRef, close);
useEventListener("keydown", (e: KeyboardEvent) => {
  if (e.key === "Escape") close();
});
useEventListener("scroll", close, true);
useEventListener("resize", close);

onBeforeUnmount(close);
</script>

<template>
  <button
    ref="triggerRef"
    type="button"
    class="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
    aria-haspopup="menu"
    :aria-expanded="open"
    :aria-label="t('downloads.rowMoreActions')"
    :title="t('downloads.rowMoreActions')"
    @click="toggle"
  >
    <slot />
  </button>
  <Teleport to="body">
    <Transition
      enter-active-class="transition duration-100 ease-out"
      leave-active-class="transition duration-75 ease-in"
      enter-from-class="opacity-0 scale-95"
      enter-to-class="opacity-100 scale-100"
      leave-to-class="opacity-0 scale-95"
    >
      <div
        v-if="open"
        ref="menuRef"
        role="menu"
        data-state="open"
        class="dl-surface fixed z-50 overflow-hidden rounded-lg border border-border bg-card text-card-foreground shadow-lg"
        :style="menuStyle"
      >
        <ul class="py-1">
          <li v-for="(item, i) in props.items" :key="i">
            <button
              type="button"
              class="flex w-full items-center px-3 py-1.5 text-left transition-colors hover:bg-accent"
              :class="item.danger ? 'text-danger hover:text-danger' : ''"
              @click="pick(item.onSelect)"
            >
              {{ item.label }}
            </button>
          </li>
        </ul>
      </div>
    </Transition>
  </Teleport>
</template>
