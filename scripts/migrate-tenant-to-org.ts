import { getPayload } from 'payload'

async function migrate() {
  const { default: configPromise } = await import('../src/payload.config')
  const payload = await getPayload({ config: configPromise })

  // Find all domains with a tenant email set
  const domains = await payload.find({
    collection: 'domains',
    pagination: false,
    where: { tenant: { exists: true } },
  })

  const tenantEmails = [...new Set(domains.docs.map((d) => d.tenant).filter(Boolean))]
  console.log(`Found ${tenantEmails.length} unique tenants`)

  for (const email of tenantEmails) {
    const slug = email.replace(/[@.]/g, '-').toLowerCase()
    const name = email.split('@')[0]

    // Find-or-create organization (idempotent)
    const existingOrg = await payload.find({
      collection: 'organizations',
      where: { slug: { equals: slug } },
      limit: 1,
    })

    let orgId: string
    if (existingOrg.docs.length > 0) {
      orgId = String(existingOrg.docs[0].id)
      console.log(`  Org exists: ${slug} (${orgId})`)
    } else {
      const created = await payload.create({
        collection: 'organizations',
        data: { slug, name: `${name}'s workspace` },
      })
      orgId = String(created.id)
      console.log(`  Created org: ${slug} (${orgId})`)
    }

    // Find-or-create owner membership (idempotent)
    const membership = await payload.find({
      collection: 'org-memberships',
      where: {
        and: [{ user: { equals: email } }, { organization: { equals: orgId } }],
      },
      limit: 1,
    })

    if (membership.docs.length === 0) {
      await payload.create({
        collection: 'org-memberships',
        data: { user: email, organization: orgId, role: 'owner' },
      })
      console.log(`  Created membership: ${email} -> ${slug}`)
    } else {
      console.log(`  Membership exists: ${email} -> ${slug}`)
    }

    // Update all domains for this tenant that don't yet have an organization
    const tenantDomains = domains.docs.filter((d) => d.tenant === email)
    for (const domain of tenantDomains) {
      if (!domain.organization) {
        await payload.update({
          collection: 'domains',
          id: domain.id,
          data: { organization: orgId },
        })
        console.log(`  Linked domain: ${domain.name || domain.domainSlug} -> ${slug}`)
      } else {
        console.log(`  Domain already linked: ${domain.name || domain.domainSlug}`)
      }
    }
  }

  // Same for apps — link tenant emails to their organizations
  const apps = await payload.find({
    collection: 'apps',
    pagination: false,
    where: { tenant: { exists: true } },
  })

  for (const app of apps.docs) {
    if (app.tenant && !app.organization) {
      const slug = app.tenant.replace(/[@.]/g, '-').toLowerCase()
      const org = await payload.find({
        collection: 'organizations',
        where: { slug: { equals: slug } },
        limit: 1,
      })
      if (org.docs.length > 0) {
        await payload.update({
          collection: 'apps',
          id: app.id,
          data: { organization: org.docs[0].id },
        })
        console.log(`  Linked app: ${app.name} -> ${slug}`)
      } else {
        console.log(`  No org found for app tenant: ${app.tenant}`)
      }
    } else if (app.tenant && app.organization) {
      console.log(`  App already linked: ${app.name}`)
    }
  }

  console.log('Migration complete')
  process.exit(0)
}

migrate().catch((err) => {
  console.error(err)
  process.exit(1)
})
