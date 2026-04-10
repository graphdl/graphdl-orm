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
// Drop any export line whose declaration head (before the last `:`) contains
// WIT-invalid punctuation. WIT component names embed `#`, `@` and `/`
// (e.g. `cabi_post_graphdl:arest/engine@0.1.0#system`) which break tsc.
const cleaned = src.split('\n').filter(line => {
  const trimmed = line.trim()
  if (!trimmed.startsWith('export const ')) return true
  // Everything up to the last `:` is the declaration name + garbage.
  // A clean export has exactly `export const <ident>:` so a second `:`
  // or any of # / @ in the head is a red flag.
  const head = trimmed.slice('export const '.length).split(/:\s*\(/)[0].split(': ')[0]
  return !/[#@/]/.test(head) && !head.includes(':')
}).join('\n')

if (cleaned !== src) {
  writeFileSync(dtsPath, cleaned, 'utf-8')
  const removed = src.split('\n').length - cleaned.split('\n').length
  console.log(`sanitize-wasm-dts: removed ${removed} invalid export line(s) from ${dtsPath}`)
} else {
  console.log(`sanitize-wasm-dts: no changes needed`)
}
