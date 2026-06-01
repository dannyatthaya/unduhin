<script setup lang="ts">
import { ContextMenuGroup, type ContextMenuGroupProps, useForwardProps } from "reka-ui";

const props = defineProps<ContextMenuGroupProps>();
const forwarded = useForwardProps(props);
</script>

<template>
  <ContextMenuGroup v-bind="forwarded">
    <slot />
  </ContextMenuGroup>
</template>
