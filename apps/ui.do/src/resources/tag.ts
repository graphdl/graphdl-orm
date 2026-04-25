/**
 * Tag ResourceDefinition.
 *
 * Pairs with file.ts / directory.ts — exposes the `Tag` noun for the
 * generic admin shell. Track #405 consumes the Tag list directly via
 * useArestList for the FileBrowser's TagFilter chip strip.
 */
import type { ReactNode } from 'react'
import { createArestResource, type CreateArestResourceOptions } from './createArestResource'
import type { ResourceDefinition } from './resourceDefinition'

export interface CreateTagResourceOptions extends Omit<CreateArestResourceOptions, 'icon'> {
  icon?: ReactNode
}

export function createTagResource(options: CreateTagResourceOptions): ResourceDefinition {
  return createArestResource('Tag', {
    baseUrl: options.baseUrl,
    app: options.app,
    label: options.label ?? 'Tags',
    icon: options.icon,
    hideFromMenu: options.hideFromMenu,
  })
}
