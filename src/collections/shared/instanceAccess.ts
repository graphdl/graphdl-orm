import type { Access, Where } from 'payload'

const SERVICE_ACCOUNT = 'cicd@repo.do'

/** Look up user's organization IDs from their memberships */
async function getUserOrgIds(req: any): Promise<string[]> {
  const email = req.user?.email
  if (!email) return []
  try {
    const memberships = await req.payload.find({
      collection: 'org-memberships',
      where: { user: { equals: email } },
      pagination: false,
      depth: 0,
    })
    return memberships.docs.map((m: any) =>
      typeof m.organization === 'string' ? m.organization : m.organization?.id
    ).filter(Boolean)
  } catch {
    return []
  }
}

/**
 * Read access for schema-level collections (Nouns, Readings, GraphSchemas, Roles, Constraints, ConstraintSpans).
 * Public domain content is readable by anyone — including unauthenticated users (like schema.org).
 * Authenticated users also see their own private domain content (via tenant or org membership).
 */
export const schemaReadAccess: Access = async ({ req }) => {
  if (!req.user) {
    return { 'domain.visibility': { equals: 'public' } } as Where
  }
  if (req.user.email === SERVICE_ACCOUNT) return true
  const orgIds = await getUserOrgIds(req)
  const orClauses: Where[] = [
    { 'domain.tenant': { equals: req.user.email } },
    { 'domain.visibility': { equals: 'public' } },
  ]
  if (orgIds.length > 0) {
    orClauses.push({ 'domain.organization': { in: orgIds } })
  }
  return { or: orClauses } as Where
}

/**
 * Write access for schema-level collections.
 * Only the domain's tenant/org member (or service account) can create/update/delete.
 */
export const schemaWriteAccess: Access = async ({ req }) => {
  if (!req.user) return false
  if (req.user.email === SERVICE_ACCOUNT) return true
  const orgIds = await getUserOrgIds(req)
  const orClauses: Where[] = [
    { 'domain.tenant': { equals: req.user.email } },
  ]
  if (orgIds.length > 0) {
    orClauses.push({ 'domain.organization': { in: orgIds } })
  }
  return { or: orClauses } as Where
}

/**
 * Read access for instance-level collections.
 * Users can read objects in their own domains (tenant or org) + public domains.
 * Service account bypasses — the API proxy handles per-user scoping.
 */
export const instanceReadAccess: Access = async ({ req }) => {
  if (!req.user) return false
  if (req.user.email === SERVICE_ACCOUNT) return true
  const orgIds = await getUserOrgIds(req)
  const orClauses: Where[] = [
    { 'domain.tenant': { equals: req.user.email } } as Where,
    { 'domain.visibility': { equals: 'public' } } as Where,
  ]
  if (orgIds.length > 0) {
    orClauses.push({ 'domain.organization': { in: orgIds } } as Where)
  }
  return { or: orClauses }
}

/**
 * Write access for instance-level collections.
 * Users can write to objects in domains they own (tenant or org member).
 * Service account bypasses — the API proxy validates domain ownership.
 */
export const instanceWriteAccess: Access = async ({ req }) => {
  if (!req.user) return false
  if (req.user.email === SERVICE_ACCOUNT) return true
  const orgIds = await getUserOrgIds(req)
  const orClauses: Where[] = [
    { 'domain.tenant': { equals: req.user.email } },
  ]
  if (orgIds.length > 0) {
    orClauses.push({ 'domain.organization': { in: orgIds } })
  }
  return { or: orClauses }
}

/**
 * Read access for the Domains/Apps collections.
 * Users see their own domains (tenant or org) + public domains.
 * Service account bypasses — the API proxy handles per-user scoping.
 */
export const domainReadAccess: Access = async ({ req }) => {
  if (!req.user) return false
  if (req.user.email === SERVICE_ACCOUNT) return true
  const orgIds = await getUserOrgIds(req)
  const orClauses: Where[] = [
    { tenant: { equals: req.user.email } } as Where,
    { visibility: { equals: 'public' } } as Where,
  ]
  if (orgIds.length > 0) {
    orClauses.push({ organization: { in: orgIds } } as Where)
  }
  return { or: orClauses }
}

/**
 * Write access for the Domains/Apps collections.
 * Users can only modify domains they own (tenant or org member).
 * Service account bypasses — the API proxy validates domain ownership.
 */
export const domainWriteAccess: Access = async ({ req }) => {
  if (!req.user) return false
  if (req.user.email === SERVICE_ACCOUNT) return true
  const orgIds = await getUserOrgIds(req)
  const orClauses: Where[] = [
    { tenant: { equals: req.user.email } },
  ]
  if (orgIds.length > 0) {
    orClauses.push({ organization: { in: orgIds } })
  }
  return { or: orClauses }
}
