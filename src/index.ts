/**
 * `arest` — npm entry point.
 *
 * Public surface:
 *
 *   Engine primitives:
 *     compileDomainReadings, compileDomainReadingsBare, system, release_domain
 *     (plus lower-level wasm exports: create, create_bare, parse_and_compile)
 *
 *   MCP server factory:
 *     createArestServer   — returns an unconnected McpServer with the
 *                           core AREST verbs registered. Attach any
 *                           transport (stdio, Streamable HTTP, custom).
 */

export {
  compileDomainReadings,
  compileDomainReadingsBare,
  release_domain,
  system,
  currentDomainHandle,
  evaluateConstraints,
  forwardChain,
  getTransitions,
  applyCommand,
  querySchema,
  getNounSchemas,
  computeRMAP,
} from './api/engine.js'

export { createArestServer } from './mcp/server-factory.js'
export type { CreateArestServerOptions } from './mcp/server-factory.js'
