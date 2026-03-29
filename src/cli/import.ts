#!/usr/bin/env node
/**
 * graphdl import — import an external API spec as FORML2 readings.
 *
 * Usage:
 *   npx graphdl import stripe                    # fetch from known registry
 *   npx graphdl import https://api.example.com/openapi.json
 *   npx graphdl import ./path/to/spec.json
 *   npx graphdl import ./schema.sql --format clickhouse
 *
 * Output: domains/<name>.md in the current directory
 */

import { fromOpenAPI, type OpenAPISpec } from '../generate/from-openapi.js'
import { parseClickHouseSQL, fromClickHouse } from '../generate/from-clickhouse.js'
import * as fs from 'fs'
import * as path from 'path'

// ── Known API registries ────────────────────────────────────────────
const KNOWN_SPECS: Record<string, string> = {
  stripe: 'https://raw.githubusercontent.com/stripe/openapi/master/openapi/spec3.json',
  github: 'https://raw.githubusercontent.com/github/rest-api-description/main/descriptions/api.github.com/api.github.com.json',
  twilio: 'https://raw.githubusercontent.com/twilio/twilio-oai/main/spec/json/twilio_api_v2010.json',
}

// ── CLI ─────────────────────────────────────────────────────────────

async function main() {
  const args = process.argv.slice(2)

  if (args[0] === 'import') {
    args.shift()
  }

  if (args.length === 0 || args[0] === '--help' || args[0] === '-h') {
    console.log(`graphdl import — import an external API spec as FORML2 readings

Usage:
  graphdl import <name-or-url-or-path> [options]

Arguments:
  <name>       Known API name: ${Object.keys(KNOWN_SPECS).join(', ')}
  <url>        URL to an OpenAPI 3.x JSON spec
  <path>       Local file path (.json for OpenAPI, .sql for ClickHouse)

Options:
  --format <type>   Force format: openapi (default) or clickhouse
  --domain <name>   Override the domain name (default: derived from source)
  --out <dir>       Output directory (default: ./domains)
  --stdout          Print to stdout instead of writing a file

Examples:
  graphdl import stripe
  graphdl import https://api.example.com/openapi.json
  graphdl import ./schema.sql --format clickhouse
  graphdl import ./petstore.json --domain pets --out ./readings`)
    process.exit(0)
  }

  const source = args[0]
  const format = getFlag(args, '--format') ?? detectFormat(source)
  const domainName = getFlag(args, '--domain') ?? deriveDomainName(source)
  const outDir = getFlag(args, '--out') ?? './domains'
  const toStdout = args.includes('--stdout')

  console.error(`Importing ${source} as ${format} → ${domainName}`)

  let readings: string

  if (format === 'clickhouse') {
    const sql = await loadSource(source)
    const tables = parseClickHouseSQL(sql)
    if (tables.length === 0) {
      console.error('No CREATE TABLE statements found')
      process.exit(1)
    }
    console.error(`Parsed ${tables.length} tables`)
    readings = fromClickHouse(tables, domainName)
  } else {
    const specText = await loadSource(source)
    let spec: OpenAPISpec
    try {
      spec = JSON.parse(specText)
    } catch {
      console.error('Failed to parse JSON. Is this a valid OpenAPI spec?')
      process.exit(1)
    }
    const schemaCount = Object.keys(spec.components?.schemas ?? {}).length
    const pathCount = Object.keys(spec.paths ?? {}).length
    console.error(`Parsed ${schemaCount} schemas, ${pathCount} paths`)
    readings = fromOpenAPI(spec, domainName)
  }

  if (toStdout) {
    console.log(readings)
  } else {
    if (!fs.existsSync(outDir)) {
      fs.mkdirSync(outDir, { recursive: true })
    }
    const outPath = path.join(outDir, `${domainName}.md`)
    fs.writeFileSync(outPath, readings)
    console.error(`Written to ${outPath}`)
  }
}

// ── Helpers ─────────────────────────────────────────────────────────

async function loadSource(source: string): Promise<string> {
  // Known registry
  if (KNOWN_SPECS[source.toLowerCase()]) {
    const url = KNOWN_SPECS[source.toLowerCase()]
    console.error(`Fetching from registry: ${url}`)
    const res = await fetch(url)
    if (!res.ok) throw new Error(`Failed to fetch ${url}: ${res.status}`)
    return res.text()
  }

  // URL
  if (source.startsWith('http://') || source.startsWith('https://')) {
    console.error(`Fetching from URL: ${source}`)
    const res = await fetch(source)
    if (!res.ok) throw new Error(`Failed to fetch ${source}: ${res.status}`)
    return res.text()
  }

  // Local file
  const resolved = path.resolve(source)
  if (!fs.existsSync(resolved)) {
    console.error(`File not found: ${resolved}`)
    process.exit(1)
  }
  console.error(`Reading file: ${resolved}`)
  return fs.readFileSync(resolved, 'utf-8')
}

function detectFormat(source: string): string {
  if (source.endsWith('.sql')) return 'clickhouse'
  return 'openapi'
}

function deriveDomainName(source: string): string {
  // Known name
  if (KNOWN_SPECS[source.toLowerCase()]) return source.toLowerCase()

  // URL → last path segment or hostname
  if (source.startsWith('http')) {
    try {
      const url = new URL(source)
      const lastSegment = url.pathname.split('/').filter(Boolean).pop()
      if (lastSegment) return lastSegment.replace(/\.(json|yaml|yml)$/, '')
      return url.hostname.split('.')[0]
    } catch {
      return 'imported'
    }
  }

  // File → basename without extension
  return path.basename(source).replace(/\.(json|yaml|yml|sql)$/, '')
}

function getFlag(args: string[], flag: string): string | undefined {
  const idx = args.indexOf(flag)
  if (idx >= 0 && idx + 1 < args.length) return args[idx + 1]
  return undefined
}

main().catch((e) => {
  console.error(e.message ?? e)
  process.exit(1)
})
