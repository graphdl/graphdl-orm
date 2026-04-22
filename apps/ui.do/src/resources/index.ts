/**
 * ResourceDefinition generator surface.
 *
 *   createArestResource(noun, opts)          — one definition per noun.
 *   useArestResources(opts)                  — auto-discover all nouns
 *                                              from the app's OpenAPI.
 *   createExternalSystemResource(sys, opts)  — #343: one definition
 *                                              per mounted External
 *                                              System (schema.org, …).
 *   useExternalSystems(opts)                 — auto-discover mounted
 *                                              External Systems.
 */
export { createArestResource, type CreateArestResourceOptions } from './createArestResource'
export {
  useArestResources,
  extractNounsFromDoc,
  type UseArestResourcesOptions,
  type UseArestResourcesResult,
} from './useArestResources'
export {
  createExternalSystemResource,
  extractExternalSystemsFromDoc,
  useExternalSystems,
  type CreateExternalSystemResourceOptions,
  type UseExternalSystemsOptions,
  type UseExternalSystemsResult,
} from './ExternalSystem'
export type { ResourceDefinition } from './resourceDefinition'
