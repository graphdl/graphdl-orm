import type { Access } from 'payload'

/**
 * Read access for instance-level collections.
 * Users can read objects in their own domains + public domains.
 */
export const instanceReadAccess: Access = ({ req }) => {
  if (!req.user) return false
  return {
    or: [
      { 'domain.tenant': { equals: req.user.email } },
      { 'domain.visibility': { equals: 'public' } },
    ],
  }
}

/**
 * Write access for instance-level collections.
 * Users can only write to objects in their own domains.
 */
export const instanceWriteAccess: Access = ({ req }) => {
  if (!req.user) return false
  return { 'domain.tenant': { equals: req.user.email } }
}

/**
 * Read access for the Domains collection itself.
 * Users see their own domains + public domains.
 */
export const domainReadAccess: Access = ({ req }) => {
  if (!req.user) return false
  return {
    or: [
      { tenant: { equals: req.user.email } },
      { visibility: { equals: 'public' } },
    ],
  }
}

/**
 * Write access for the Domains collection.
 * Users can only modify their own domains.
 */
export const domainWriteAccess: Access = ({ req }) => {
  if (!req.user) return false
  return { tenant: { equals: req.user.email } }
}
