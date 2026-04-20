/**
 * mdxui-block-style content pages rendered from AREST ui-domain
 * entities (see readings/ui.md).
 *
 *   BlocksPage              — fetches rows + renders via a registry
 *   DEFAULT_BLOCK_REGISTRY  — plain-HTML fallbacks (hero, features,
 *                             text) that consumers can override with
 *                             mdxui primitives for production use.
 */
export {
  BlocksPage,
  DEFAULT_BLOCK_REGISTRY,
  type BlocksPageProps,
  type BlockRegistry,
  type BlockRow,
} from './BlocksPage'
