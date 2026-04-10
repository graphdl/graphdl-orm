#!/usr/bin/env node
/**
 * Post-build sanitizer for the wasm-pack-generated arest_bg.wasm.d.ts.
 *
 * wasm-pack emits the WIT component exports with identifiers like
 *   `cabi_post_graphdl:arest/engine@0.1.0#parse-and-compile`
 * which are not valid TypeScript. Those raw exports are internal to the
 * WASM module and not consumed from TypeScript — we only need the high-
 * level API (create, system, release, parse_and_compile, etc.).
 *
 * This script filters out the invalid lines after each build, leaving a
 * clean .d.ts that tsc accepts. Invoked from package.json `build:wasm`.
 */

import { readFileSync, writeFileSync, existsSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const dtsPath = resolve(__dirname, '..', 'crates', 'arest', 'pkg', 'arest_bg.wasm.d.ts')

if (!existsSync(dtsPath)) {
  console.error(`sanitize-wasm-dts: ${dtsPath} not found, skipping`)
  process.exit(0)
}

const src = readFileSync(dtsPath, 'utf-8')
// Valid TS identifier before the colon in `export const <name>:`
const validIdent = /^[A-Za-z_$][\w$]*$/
const cleaned = src.split('\n').filter(line => {
  const trimmed = line.trim()
  const exportMatch = trimmed.match(/^export const ([^:]+):/)
  if (!exportMatch) return true
  return validIdent.test(exportMatch[1].trim())
}).join('\n')

if (cleaned !== src) {
  writeFileSync(dtsPath, cleaned, 'utf-8')
  const removed = src.split('\n').length - cleaned.split('\n').length
  console.log(`sanitize-wasm-dts: removed ${removed} invalid export line(s) from ${dtsPath}`)
} else {
  console.log(`sanitize-wasm-dts: no changes needed`)
}
