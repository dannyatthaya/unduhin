<script setup lang="ts">
import { computed } from "vue";
import { cva, type VariantProps } from "class-variance-authority";
import { Primitive, type PrimitiveProps } from "reka-ui";

import { cn } from "@/lib/utils";

const buttonVariants = cva(
  "inline-flex shrink-0 items-center justify-center gap-2 whitespace-nowrap rounded-[9px] font-medium transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:pointer-events-none disabled:opacity-40",
  {
    variants: {
      variant: {
        // The solid yellow action: one per dialog or page.
        primary: "bg-dl-accent font-semibold text-dl-on-accent hover:brightness-95",
        secondary:
          "border border-input bg-dl-raised text-foreground hover:bg-accent",
        ghost: "text-foreground hover:bg-accent",
        danger: "bg-danger font-semibold text-dl-on-fail hover:bg-danger/90",
        // A destructive action that is not the dialog's main one.
        dangerGhost: "text-danger hover:bg-dl-fail-soft",
      },
      size: {
        xs: "h-7 rounded-[7px] px-2.5 text-xs",
        sm: "h-8 px-3 text-xs",
        md: "h-9 px-3.5 text-[13px]",
        icon: "h-9 w-9 p-0",
      },
    },
    defaultVariants: {
      variant: "secondary",
      size: "md",
    },
  },
);

type ButtonVariants = VariantProps<typeof buttonVariants>;

interface Props extends Pick<PrimitiveProps, "asChild" | "as"> {
  variant?: ButtonVariants["variant"];
  size?: ButtonVariants["size"];
  type?: "button" | "submit" | "reset";
  disabled?: boolean;
  title?: string;
  class?: string;
}

const props = withDefaults(defineProps<Props>(), {
  as: "button",
  type: "button",
  disabled: false,
});

const classes = computed(() =>
  cn(buttonVariants({ variant: props.variant, size: props.size }), props.class),
);
</script>

<template>
  <Primitive
    :as="as"
    :as-child="asChild"
    :type="as === 'button' ? type : undefined"
    :class="classes"
    :disabled="disabled"
    :title="title"
  >
    <slot />
  </Primitive>
</template>
