/**
 * Directory ResourceDefinition.
 *
 * Companion to file.ts — exposes the `Directory` noun via the standard
 * generator so generic CRUD remains available beside the bespoke
 * /files browser shipped in track #405.
 */
import type { ReactNode } from 'react'
import { createArestResource, type CreateArestResourceOptions } from './createArestResource'
import type { ResourceDefinition } from './resourceDefinition'

export interface CreateDirectoryResourceOptions extends Omit<CreateArestResourceOptions, 'icon'> {
  icon?: ReactNode
}

export function createDirectoryResource(options: CreateDirectoryResourceOptions): ResourceDefinition {
  return createArestResource('Directory', {
    baseUrl: options.baseUrl,
    app: options.app,
    label: options.label ?? 'Directories',
    icon: options.icon,
    hideFromMenu: options.hideFromMenu,
  })
}
