/**
 * shadcn-style class composition helper.
 *
 * `cn(...)` runs `clsx` (predicate-aware className join) followed by
 * `twMerge` (resolve Tailwind utility conflicts so the last wins —
 * e.g. `cn("p-2", "p-4") === "p-4"`). Used by every downstream
 * component that wants conditional Tailwind classes.
 */
import { clsx, type ClassValue } from 'clsx'
import { twMerge } from 'tailwind-merge'

export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs))
}
