/**
 * ResourceDefinition generator surface.
 *
 *   createArestResource(noun, opts)        — one definition per noun.
 *   useArestResources(opts)                — auto-discover all nouns
 *                                            from the app's OpenAPI.
 */
export { createArestResource, type CreateArestResourceOptions } from './createArestResource'
export {
  useArestResources,
  extractNounsFromDoc,
  type UseArestResourcesOptions,
  type UseArestResourcesResult,
} from './useArestResources'
export type { ResourceDefinition } from './resourceDefinition'
