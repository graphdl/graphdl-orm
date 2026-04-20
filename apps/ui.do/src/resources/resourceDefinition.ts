/**
 * Local re-export of @mdxui/admin's ResourceDefinition shape so our
 * code can import the type without pulling the whole admin runtime.
 * Keep this in sync with AdminContext.d.ts when upgrading mdxui.
 */
import type { ComponentType, ReactNode } from 'react'

export interface ResourceDefinition {
  /** Unique resource name (used in URL paths — plural slug). */
  name: string
  /** List view component. */
  list?: ComponentType
  /** Create form component. */
  create?: ComponentType
  /** Edit form component. */
  edit?: ComponentType
  /** Show/detail view component. */
  show?: ComponentType
  /** Icon for navigation. */
  icon?: ReactNode
  /** Display options. */
  options?: {
    /** Custom label for the resource (defaults to capitalized name). */
    label?: string
    /** Hide from sidebar menu. */
    hideFromMenu?: boolean
  }
}
