/**
 * Shared helper: ensure a Domain entity exists for a given slug.
 * Uses Registry to check for existing domain entity IDs, then EntityDB for storage.
 * Returns a record with { id } (the domain's UUID).
 */

import type { Env } from '../types'

export async function ensureDomain(env: Env, registry: any, slug: string, name?: string): Promise<Record<string, any>> {
  const existingIds: string[] = await registry.getEntityIds('Domain', slug)
  if (existingIds.length > 0) {
    const entityDO = env.ENTITY_DB.get(env.ENTITY_DB.idFromName(existingIds[0])) as any
    const entity = await entityDO.get()
    if (entity) return { id: entity.id, ...entity.data }
  }

  // Domain identity IS its slug (CSDP: reference scheme is the identity)
  const id = slug
  const data = { domainSlug: slug, name: name || slug, visibility: 'private' }
  const entityDO = env.ENTITY_DB.get(env.ENTITY_DB.idFromName(id)) as any
  await entityDO.put({ id, type: 'Domain', data })
  await registry.indexEntity('Domain', id, slug)
  return { id, ...data }
}
