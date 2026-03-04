import type { Field, Where } from 'payload'

export const domainField: Field = {
  name: 'domain',
  type: 'relationship',
  relationTo: 'domains',
  index: true,
  admin: {
    description: 'Domain this resource belongs to.',
  },
}

export function buildDomainFilter(domainIds?: string[] | null, domainId?: string | null): Where {
  if (domainIds?.length) return { domain: { in: domainIds } }
  if (domainId) return { domain: { equals: domainId } }
  return {}
}
