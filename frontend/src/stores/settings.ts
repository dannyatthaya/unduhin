import { defineStore } from "pinia";
import { computed, ref } from "vue";

import { api } from "@/types/tauri-bindings";
import type { SettingValue } from "@/types/tauri-bindings";

export const useSettingsStore = defineStore("settings", () => {
  const values = ref<Record<string, SettingValue>>({});

  const get = computed(() => (key: string) => values.value[key]);

  async function refresh() {
    values.value = await api.getSettings();
  }

  async function set(key: string, value: SettingValue) {
    await api.setSetting({ key, value });
    values.value = { ...values.value, [key]: value };
  }

  return { values, get, refresh, set };
});
