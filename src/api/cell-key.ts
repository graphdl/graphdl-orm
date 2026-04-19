/**
 * cell-key.ts — RMAP-derived cell naming for Durable Object routing.
 *
 * Paper anchor (§5.2, §5.4, Definition 2):
 *   D = ⟨⟨CELL, n₁, c₁⟩, ⟨CELL, n₂, c₂⟩, ...⟩
 *   "Each entity is a cell. RMAP assigns each entity to its own cell
 *    whose fold is independent."
 *
 * In the paper, the cell name `n` is the RMAP-derived identifier for
 * the 3NF row a command touches. In AREST's Cloudflare deployment,
 * cells map 1:1 to Durable Objects — one DO per cell per Definition 2
 * (Cell Isolation). Two writers on disjoint cells land on disjoint
 * DOs and run concurrently; two writers on the same cell serialise
 * through the single DO, which is exactly the isolation guarantee.
 *
 * This module is the narrow place where we compute the DO key from
 * (noun type, entity id). Having one helper makes the RMAP keying
 * explicit instead of scattering `${type}:${id}` concatenation across
 * the router and fan-out paths — and gives one place to extend when
 * we add compound reference schemes (§RMAP absorbs multi-role keys
 * into a single composite name).
 *
 * Format today: `{nounType}:{entityId}`. The prefix scopes the DO
 * namespace by type so two entities of different types with the same
 * id (possible when ids come from an external system) don't collide
 * onto the same DO. For entities with compound reference schemes
 * (Halpin Ch.5 — e.g. Order Line keyed by ⟨order, lineNum⟩) the
 * future form is `{nounType}:{canonicalConcat(roleValues)}`; callers
 * pass the canonical string as entityId today.
 *
 * Legacy paths that call `ENTITY_DB.idFromName(rawId)` directly still
 * work — DO names are opaque strings, so the prefix is additive, not
 * a schema change. New code should prefer `cellKey()`.
 */

/**
 * Compute the RMAP-derived cell name for an entity.
 *
 * @param nounType  The declared Noun (ORM 2 Entity Type) name, e.g. "Order".
 * @param entityId  The entity's id — the reference scheme value (or the
 *                  canonical-concatenated compound key).
 * @returns         Canonical cell name usable as a DO key.
 */
export function cellKey(nounType: string, entityId: string): string {
  // Defensive trim: callers sometimes pass pre-trimmed or URL-decoded
  // values; we don't want leading/trailing whitespace to shard writes
  // across two DOs for what is semantically the same cell.
  const type = nounType.trim()
  const id = entityId.trim()
  if (!type) return id
  return `${type}:${id}`
}

/**
 * Inverse of `cellKey` — split a cell name back into `(nounType, entityId)`.
 * Returns `null` when the key is not in cellKey format (e.g. legacy
 * raw-UUID keys from before cellKey was introduced).
 *
 * Used by the event demux (#220) and the federated-analytics binding
 * (#219) to route a post-commit event back to the right noun-typed
 * subscriber without re-querying the registry.
 */
export function parseCellKey(key: string): { nounType: string; entityId: string } | null {
  const sep = key.indexOf(':')
  if (sep <= 0 || sep === key.length - 1) return null
  return {
    nounType: key.slice(0, sep),
    entityId: key.slice(sep + 1),
  }
}
