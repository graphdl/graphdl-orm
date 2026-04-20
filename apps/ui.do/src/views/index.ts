/**
 * Generic schema-driven views — each takes a noun name and
 * introspects the app's OpenAPI document to render fields.
 *
 *   GenericListView   — table of all rows
 *   GenericShowView   — detail dl of one row
 *   GenericEditView   — form for editing one row
 *   GenericCreateView — form for creating a new row
 */
export { GenericListView, type GenericListViewProps } from './GenericListView'
export { GenericShowView, type GenericShowViewProps } from './GenericShowView'
export { GenericEditView, type GenericEditViewProps } from './GenericEditView'
export { GenericCreateView, type GenericCreateViewProps } from './GenericCreateView'
export { SchemaInput, type SchemaInputProps } from './schemaInputs'
