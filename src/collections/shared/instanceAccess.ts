import type { Access, Where } from 'payload'

const SERVICE_ACCOUNT = 'cicd@repo.do'

/**
 * Read access for schema-level collections (Nouns, Readings, GraphSchemas, Roles, Constraints, ConstraintSpans).
 * Public domain content is readable by anyone — including unauthenticated users (like schema.org).
 * Authenticated users also see their own private domain content.
 */
export const schemaReadAccess: Access = ({ req }) => {
  if (!req.user) {
    return { 'domain.visibility': { equals: 'public' } } as Where
  }
  if (req.user.email === SERVICE_ACCOUNT) return true
  return {
    or: [
      { 'domain.tenant': { equals: req.user.email } },
      { 'domain.visibility': { equals: 'public' } },
    ],
  } as Where
}

/**
 * Write access for schema-level collections.
 * Only the domain's tenant (or service account) can create/update/delete.
 */
export const schemaWriteAccess: Access = ({ req }) => {
  if (!req.user) return false
  if (req.user.email === SERVICE_ACCOUNT) return true
  return { 'domain.tenant': { equals: req.user.email } }
}

/**
 * Read access for instance-level collections.
 * Users can read objects in their own domains + public domains.
 * Service account bypasses — the API proxy handles per-user scoping.
 */
export const instanceReadAccess: Access = ({ req }) => {
  if (!req.user) return false
  if (req.user.email === SERVICE_ACCOUNT) return true
  return {
    or: [
      { 'domain.tenant': { equals: req.user.email } } as Where,
      { 'domain.visibility': { equals: 'public' } } as Where,
    ],
  }
}

/**
 * Write access for instance-level collections.
 * Users can only write to objects in their own domains.
 * Service account bypasses — the API proxy validates domain ownership.
 */
export const instanceWriteAccess: Access = ({ req }) => {
  if (!req.user) return false
  if (req.user.email === SERVICE_ACCOUNT) return true
  return { 'domain.tenant': { equals: req.user.email } }
}

/**
 * Read access for the Domains collection itself.
 * Users see their own domains + public domains.
 * Service account bypasses — the API proxy handles per-user scoping.
 */
export const domainReadAccess: Access = ({ req }) => {
  if (!req.user) return false
  if (req.user.email === SERVICE_ACCOUNT) return true
  return {
    or: [
      { tenant: { equals: req.user.email } } as Where,
      { visibility: { equals: 'public' } } as Where,
    ],
  }
}

/**
 * Write access for the Domains collection.
 * Users can only modify their own domains.
 * Service account bypasses — the API proxy validates domain ownership.
 */
export const domainWriteAccess: Access = ({ req }) => {
  if (!req.user) return false
  if (req.user.email === SERVICE_ACCOUNT) return true
  return { tenant: { equals: req.user.email } }
}
