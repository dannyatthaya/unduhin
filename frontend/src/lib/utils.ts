import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/**
 * Conditionally compose Tailwind class names and dedupe conflicting
 * utilities. Standard shadcn-vue helper — keep the signature stable so
 * generated components can be dropped in without edits.
 */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
