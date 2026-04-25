/**
 * File ResourceDefinition.
 *
 * Wires the AREST `File` noun into the data provider via the standard
 * createArestResource generator so the @mdxui/admin Admin tree can
 * still render generic CRUD views for files alongside the dedicated
 * /files browser. Track #405 supplies the richer browser; this
 * resource keeps the noun discoverable in the sidebar/menus and
 * mounts the auto-generated /file/* routes for fallback access.
 */
import type { ReactNode } from 'react'
import { createArestResource, type CreateArestResourceOptions } from './createArestResource'
import type { ResourceDefinition } from './resourceDefinition'

export interface CreateFileResourceOptions extends Omit<CreateArestResourceOptions, 'icon'> {
  /** Optional sidebar icon override. Defaults to no icon (consumer wires Folder/File). */
  icon?: ReactNode
}

export function createFileResource(options: CreateFileResourceOptions): ResourceDefinition {
  return createArestResource('File', {
    baseUrl: options.baseUrl,
    app: options.app,
    label: options.label ?? 'Files',
    icon: options.icon,
    hideFromMenu: options.hideFromMenu,
  })
}
