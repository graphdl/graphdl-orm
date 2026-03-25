/**
 * seed-core.ts
 *
 * Operational script that seeds the metamodel as EntityDB DOs by reading
 * all readings/*.md files and POSTing them to a live wrangler dev instance.
 *
 * Usage: npx tsx scripts/seed-core.ts
 *
 * Environment:
 *   GRAPHDL_URL - Base URL of the running wrangler dev instance
 *                 (default: http://localhost:8787)
 *
 * Pipeline: readings/*.md → POST /parse → POST /api/claims → EntityDB DOs
 */

import { readFileSync, readdirSync } from 'fs'
import { resolve } from 'path'

const READINGS_DIR = resolve(__dirname, '../readings')
const GRAPHDL_URL = process.env.GRAPHDL_URL || 'http://localhost:8787'

async function seedCore() {
  const files = readdirSync(READINGS_DIR).filter(f => f.endsWith('.md'))
  console.log(`Found ${files.length} reading files in ${READINGS_DIR}`)

  for (const file of files) {
    const text = readFileSync(resolve(READINGS_DIR, file), 'utf-8')
    const domain = file.replace('.md', '')
    console.log(`\nSeeding domain: ${domain} (${file})`)

    // Step 1: Parse FORML2 text
    console.log('  Parsing...')
    const parseRes = await fetch(`${GRAPHDL_URL}/parse`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ text, domain }),
    })
    if (!parseRes.ok) {
      console.error(`  Parse FAILED: ${parseRes.status} ${await parseRes.text()}`)
      continue
    }
    const claims = await parseRes.json()
    const nounCount = claims.nouns?.length || 0
    const readingCount = claims.readings?.length || 0
    const constraintCount = claims.constraints?.length || 0
    console.log(`  Parsed: ${nounCount} nouns, ${readingCount} readings, ${constraintCount} constraints`)

    // Step 2: Ingest claims
    console.log('  Ingesting...')
    const ingestRes = await fetch(`${GRAPHDL_URL}/api/claims`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ ...claims, domain }),
    })
    const result = await ingestRes.json()
    if (!ingestRes.ok) {
      console.error(`  Ingest FAILED: ${ingestRes.status}`)
      if (result.violations) {
        for (const v of result.violations) {
          console.error(`    ${v.type}: ${v.message}`)
          if (v.fix) console.error(`    Fix: ${v.fix}`)
        }
      }
    } else {
      console.log(`  OK: ${JSON.stringify(result)}`)
    }
  }

  console.log('\nDone.')
}

seedCore().catch(console.error)
