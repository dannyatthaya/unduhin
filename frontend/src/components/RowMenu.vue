<script setup lang="ts">
// Minimal teleported dropdown for the per-row "More" button.
//
// Hand-rolled for now — when the shared right-click context menu
// lands, we can lift both into a shared `Menu` primitive.

import { computed, nextTick, onBeforeUnmount, ref, watch, type Component } from "vue";
import { onClickOutside, useEventListener } from "@vueuse/core";
import { useI18n } from "vue-i18n";

import { useOverlaysStore } from "@/stores/overlays";

export interface RowMenuItem {
  label: string;
  danger?: boolean;
  icon?: Component;
  /** Keyboard shortcut shown at the right edge, e.g. "F2". */
  shortcut?: string;
  /** Draw a divider above this item. */
  separatorBefore?: boolean;
  onSelect: () => void;
}

const props = defineProps<{ items: RowMenuItem[] }>();

const { t } = useI18n();

const open = ref(false);
const triggerRef = ref<HTMLButtonElement | null>(null);
const menuRef = ref<HTMLDivElement | null>(null);
const menuStyle = ref<Record<string, string>>({});

const PLAIN_MENU_WIDTH = 200;
/** Menus with icons or shortcuts need the room the design gives them. */
const RICH_MENU_WIDTH = 248;
const isRich = computed(() => props.items.some((item) => item.icon || item.shortcut));
const menuWidth = computed(() => (isRich.value ? RICH_MENU_WIDTH : PLAIN_MENU_WIDTH));

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
  const left = `${Math.max(8, r.right - menuWidth.value)}px`;
  // Estimate menu height from item count; refined below once mounted.
  // 34px per item + 12px padding.
  const estimated = props.items.length * 34 + 12;
  const margin = 8;
  const placeAbove = r.bottom + estimated + margin > window.innerHeight;
  menuStyle.value = placeAbove
    ? {
        bottom: `${window.innerHeight - r.top + 4}px`,
        left,
        width: `${menuWidth.value}px`,
      }
    : { top: `${r.bottom + 4}px`, left, width: `${menuWidth.value}px` };

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
      width: `${menuWidth.value}px`,
    };
  } else if (placeAbove && !flipNow) {
    // Estimate said "above" but the real height fits below — switch
    // back so the menu doesn't float in empty space above the row.
    menuStyle.value = {
      top: `${trig.bottom + 4}px`,
      left,
      width: `${menuWidth.value}px`,
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
        class="dl-surface fixed z-50 overflow-hidden rounded-xl border border-border bg-card text-[13px] text-card-foreground shadow-dl-menu"
        :style="menuStyle"
      >
        <ul class="flex flex-col p-1.5">
          <li v-for="(item, i) in props.items" :key="i">
            <div
              v-if="item.separatorBefore"
              role="separator"
              class="my-1 h-px bg-dl-line"
            />
            <button
              type="button"
              role="menuitem"
              class="flex h-[34px] w-full items-center gap-2.5 rounded-[7px] px-2.5 text-left transition-colors hover:bg-accent focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring"
              :class="item.danger ? 'text-danger' : ''"
              @click="pick(item.onSelect)"
            >
              <component
                :is="item.icon"
                v-if="item.icon"
                class="h-[15px] w-[15px] shrink-0"
                aria-hidden="true"
              />
              <span class="min-w-0 flex-1 truncate">{{ item.label }}</span>
              <kbd
                v-if="item.shortcut"
                class="shrink-0 font-mono text-xs text-muted-foreground"
              >
                {{ item.shortcut }}
              </kbd>
            </button>
          </li>
        </ul>
      </div>
    </Transition>
  </Teleport>
</template>
