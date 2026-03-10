/**
 * Seed the GraphDL metamodel from FORML2 readings files.
 *
 * Reads each .md file in readings/, extracts claims via the apis extract endpoint,
 * then seeds all domains in a single call to the graphdl-orm Worker.
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

async function extractClaims(text: string): Promise<any> {
  const res = await fetch(`${API_BASE}/graphdl/extract/claims`, {
    method: 'POST',
    headers,
    body: JSON.stringify({ text, seed: false }),
  })
  if (!res.ok) throw new Error(`Extract failed: ${res.status} ${await res.text()}`)
  return (await res.json() as any).claims
}

async function main() {
  const readingsDir = join(import.meta.dirname, '..', 'readings')
  const files = readdirSync(readingsDir).filter(f => f.endsWith('.md'))

  console.log(`Found ${files.length} readings files: ${files.join(', ')}`)

  // Extract claims from all files
  const domains = []
  for (const file of files) {
    const slug = `graphdl-${file.replace('.md', '')}`
    const text = readFileSync(join(readingsDir, file), 'utf-8')
    console.log(`Extracting claims from ${file}...`)
    const claims = await extractClaims(text)
    console.log(`  ${claims.nouns.length} nouns, ${claims.readings.length} readings`)
    domains.push({ slug, name: slug, claims })
  }

  // Seed all domains in one call
  console.log(`\nSeeding ${domains.length} domains...`)
  const seedRes = await fetch(`${API_BASE}/graphdl/claims`, {
    method: 'POST',
    headers,
    body: JSON.stringify({ type: 'claims', domains }),
  })

  if (!seedRes.ok) {
    console.error(`Seed failed: ${seedRes.status} ${await seedRes.text()}`)
    process.exit(1)
  }

  const result = await seedRes.json() as any
  for (const d of result.domains) {
    console.log(`  ${d.domain}: ${d.nouns} nouns, ${d.readings} readings, ${d.errors?.length || 0} errors`)
  }

  // Verify
  console.log('\n--- Verification ---')
  const statsRes = await fetch(`${API_BASE}/graphdl/raw/nouns?limit=0`, { headers })
  const stats = await statsRes.json() as any
  console.log(`Total nouns: ${stats.totalDocs}`)
}

main().catch(console.error)
