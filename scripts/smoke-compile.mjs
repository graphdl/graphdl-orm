// Replicate exactly what the MCP server does: loadReadingsFromDir →
// compileDomainReadings → system(h, 'debug', '').
// Expected: debug returns a JSON-parseable projection with nouns.
import { readdirSync, readFileSync, existsSync } from 'node:fs'
import { join } from 'node:path'
import { compileDomainReadings, system } from '../src/api/engine.ts'

const dir = process.argv[2] || new URL('../tutor/domains/', import.meta.url).pathname
console.error(`READINGS_DIR=${dir}`)
console.error(`exists=${existsSync(dir)}`)

function loadReadings(dir) {
  if (!dir || !existsSync(dir)) return []
  return readdirSync(dir)
    .filter(n => n.endsWith('.md'))
    .sort()
    .map(n => readFileSync(join(dir, n), 'utf-8'))
}

const readings = loadReadings(dir)
console.error(`${readings.length} readings, total ${readings.reduce((s, r) => s + r.length, 0)} bytes`)

try {
  const h = compileDomainReadings(...readings)
  console.error(`handle=${h}`)
  const debug = system(h, 'debug', '')
  console.error(`debug raw (first 200 chars): ${debug.slice(0, 200)}`)
  try {
    const parsed = JSON.parse(debug)
    console.error(`debug parsed: nouns=${parsed.nouns?.length} factTypes=${parsed.factTypes?.length}`)
  } catch (e) {
    console.error(`NOT JSON: ${e.message}`)
  }
} catch (e) {
  console.error(`FAIL: ${e.message}`)
}
