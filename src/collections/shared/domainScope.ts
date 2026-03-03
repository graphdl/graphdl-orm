import type { Field, Where } from 'payload'

export const domainField: Field = {
  name: 'domain',
  type: 'text',
  index: true,
  admin: {
    description: 'Domain this resource belongs to.',
  },
}

export function buildDomainFilter(domains?: string[] | null, domain?: string | null): Where {
  if (domains?.length) return { domain: { in: domains } }
  if (domain) return { domain: { equals: domain } }
  return {}
}
