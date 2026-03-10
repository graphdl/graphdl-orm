/**
 * Seed the GraphDL metamodel from FORML2 readings files.
 *
 * Reads each .md file in readings/, extracts claims via the apis extract endpoint,
 * then seeds them into the new graphdl-orm Worker.
 *
 * Usage: npx tsx scripts/seed-metamodel.ts
 */
import { readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'

const API_BASE = process.env.API_URL || 'https://api.auto.dev'
const API_KEY = process.env.AUTO_DEV_API_KEY

if (!API_KEY) {
  console.error('Set AUTO_DEV_API_KEY environment variable')
  process.exit(1)
}

const headers = {
  'X-API-Key': API_KEY,
  'Content-Type': 'application/json',
}

async function seedDomain(slug: string, readingsText: string): Promise<void> {
  console.log(`\n--- Seeding domain: ${slug} ---`)

  // Step 1: Extract claims via LLM
  console.log('  Extracting claims...')
  const extractRes = await fetch(`${API_BASE}/graphdl/extract/claims`, {
    method: 'POST',
    headers,
    body: JSON.stringify({ text: readingsText, seed: false }),
  })

  if (!extractRes.ok) {
    console.error(`  Extract failed: ${extractRes.status} ${await extractRes.text()}`)
    return
  }

  const { claims } = await extractRes.json() as any
  console.log(`  Extracted: ${claims.nouns.length} nouns, ${claims.readings.length} readings`)

  // Step 2: Ensure domain exists
  const domainRes = await fetch(`${API_BASE}/graphdl/raw/domains`, {
    method: 'POST',
    headers,
    body: JSON.stringify({ domainSlug: slug, name: slug, visibility: 'private' }),
  })
  const domain = await domainRes.json() as any
  const domainId = domain.doc?.id || domain.id

  // Step 3: Seed claims
  console.log('  Seeding claims...')
  const seedRes = await fetch(`${API_BASE}/graphdl/claims`, {
    method: 'POST',
    headers,
    body: JSON.stringify({ type: 'claims', claims, domainId }),
  })

  if (!seedRes.ok) {
    console.error(`  Seed failed: ${seedRes.status} ${await seedRes.text()}`)
    return
  }

  const result = await seedRes.json() as any
  console.log(`  Result: ${result.nouns} nouns, ${result.readings} readings, ${result.errors?.length || 0} errors`)
  if (result.errors?.length) {
    for (const err of result.errors.slice(0, 5)) {
      console.log(`    Error: ${err}`)
    }
  }
}

async function main() {
  const readingsDir = join(import.meta.dirname, '..', 'readings')
  const files = readdirSync(readingsDir).filter(f => f.endsWith('.md'))

  console.log(`Found ${files.length} readings files: ${files.join(', ')}`)

  for (const file of files) {
    const slug = file.replace('.md', '')
    const text = readFileSync(join(readingsDir, file), 'utf-8')
    await seedDomain(`graphdl-${slug}`, text)
  }

  // Verify
  console.log('\n--- Verification ---')
  const statsRes = await fetch(`${API_BASE}/graphdl/raw/nouns?limit=0`, { headers })
  const stats = await statsRes.json() as any
  console.log(`Total nouns: ${stats.totalDocs}`)
}

main().catch(console.error)
