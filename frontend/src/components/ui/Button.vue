<script setup lang="ts">
import { computed } from "vue";
import { cva, type VariantProps } from "class-variance-authority";
import { Primitive, type PrimitiveProps } from "reka-ui";

import { cn } from "@/lib/utils";

const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50",
  {
    variants: {
      variant: {
        primary: "bg-primary text-primary-foreground hover:bg-primary/90",
        secondary:
          "border border-border bg-card text-foreground hover:bg-accent hover:text-accent-foreground",
        ghost: "text-foreground hover:bg-accent hover:text-accent-foreground",
        danger: "bg-danger text-white hover:bg-danger/90",
      },
      size: {
        sm: "h-8 px-3 text-xs",
        md: "h-9 px-3.5 text-sm",
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
