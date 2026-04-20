/**
 * Public exports for the hooks layer — one-noun, one-entity hooks that
 * wrap the AREST /arest/ surface in TanStack Query conventions. Higher-
 * level views (ListView / ShowView / EditView / CreateView) compose
 * these with @mdxui/admin primitives.
 */
export { useActions } from './useActions'
export type { ArestAction, UseActionsOptions, UseActionsResult } from './useActions'
