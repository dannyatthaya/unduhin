// Tracks which setting was just saved, so its row can show "Saved" for a
// moment. One flag at a time: saving another setting moves it.

import { onBeforeUnmount, ref } from "vue";

const FLASH_MS = 2000;

export function useSavedFlash() {
  const savedId = ref<string | null>(null);
  let timer: ReturnType<typeof setTimeout> | undefined;

  function flash(id: string): void {
    savedId.value = id;
    clearTimeout(timer);
    timer = setTimeout(() => {
      savedId.value = null;
    }, FLASH_MS);
  }

  function isSaved(id: string): boolean {
    return savedId.value === id;
  }

  onBeforeUnmount(() => clearTimeout(timer));

  return { flash, isSaved };
}
