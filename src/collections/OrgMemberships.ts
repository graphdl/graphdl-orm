import type { CollectionConfig } from 'payload'

export const OrgMemberships: CollectionConfig = {
  slug: 'org-memberships',
  admin: { useAsTitle: 'user' },
  access: {
    read: () => true,
    create: ({ req }) => !!req.user,
    update: ({ req }) => !!req.user,
    delete: ({ req }) => !!req.user,
  },
  fields: [
    {
      name: 'user',
      type: 'text',
      required: true,
      index: true,
      label: 'User has OrgRole in Organization — UC(User, Organization) — user email',
    },
    {
      name: 'organization',
      type: 'relationship',
      relationTo: 'organizations',
      required: true,
      index: true,
      label: 'User has OrgRole in Organization — UC(User, Organization) — organization',
    },
    {
      name: 'role',
      type: 'select',
      required: true,
      defaultValue: 'member',
      options: [
        { label: 'Owner', value: 'owner' },
        { label: 'Member', value: 'member' },
      ],
      label: 'User has OrgRole in Organization — UC(User, Organization) — role',
    },
  ],
}
