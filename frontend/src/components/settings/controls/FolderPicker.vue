<script setup lang="ts">
import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen } from "lucide-vue-next";

import Button from "@/components/ui/Button.vue";
import Input from "@/components/ui/Input.vue";

const props = defineProps<{
  modelValue: string;
  placeholder?: string;
  disabled?: boolean;
}>();
const emit = defineEmits<{ "update:modelValue": [value: string] }>();

async function pick() {
  if (props.disabled) return;
  const selected = await open({
    directory: true,
    multiple: false,
    defaultPath: props.modelValue || undefined,
  });
  if (typeof selected === "string") emit("update:modelValue", selected);
}
</script>

<template>
  <div class="flex w-[26rem] max-w-full gap-2">
    <Input
      :model-value="modelValue"
      :placeholder="placeholder ?? 'Pick a folder…'"
      @update:model-value="$emit('update:modelValue', $event)"
    />
    <Button
      variant="secondary"
      size="icon"
      :disabled="disabled"
      title="Browse"
      @click="pick"
    >
      <FolderOpen class="h-4 w-4" />
    </Button>
  </div>
</template>
