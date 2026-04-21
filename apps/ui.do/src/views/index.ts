/**
 * Generic schema-driven views — each takes a noun name and
 * introspects the app's OpenAPI document to render fields.
 *
 *   GenericListView   — table of all rows
 *   GenericShowView   — detail dl of one row
 *   GenericEditView   — form for editing one row
 *   GenericCreateView — form for creating a new row
 *
 * Supporting widgets:
 *   SchemaInput, SchemaDisplay   — per-kind controls
 *   OverworldMenu, EntityOverworldMenu — SM + HATEOAS affordances
 *   ReferencePicker, ReferenceLabel    — reference-field widgets
 *   Pagination                         — prev/next list control
 *   ErrorBoundary                      — render-time error handling
 */
export { GenericListView, Pagination, type GenericListViewProps, type PaginationProps } from './GenericListView'
export { GenericShowView, type GenericShowViewProps } from './GenericShowView'
export { GenericEditView, type GenericEditViewProps } from './GenericEditView'
export { GenericCreateView, type GenericCreateViewProps } from './GenericCreateView'
export { SchemaInput, type SchemaInputProps } from './schemaInputs'
export { SchemaDisplay, type SchemaDisplayProps } from './schemaDisplay'
export { ReferencePicker, ReferenceLabel, type ReferencePickerProps, type ReferenceLabelProps } from './ReferencePicker'
export { ErrorBoundary, type ErrorBoundaryProps } from './ErrorBoundary'
export {
  mapViolationsToFields,
  extractViolations,
  type Violation,
} from './violationMapping'
export {
  OverworldMenu,
  EntityOverworldMenu,
  type OverworldMenuProps,
  type OverworldMenuSection,
  type OverworldMenuItem,
  type OverworldActionItem,
  type OverworldNavItem,
  type EntityOverworldMenuProps,
} from './OverworldMenu'
