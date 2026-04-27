#!/usr/bin/env node
/**
 * Post-build sanitizer for the wasm-pack-generated arest_bg.wasm.d.ts.
 *
 * wasm-pack emits the WIT component exports with identifiers like
 *   `cabi_post_arest:engine/engine@0.1.0#parse-and-compile`
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
// (e.g. `cabi_post_arest:engine/engine@0.1.0#system`) which break tsc.
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

// The wasm pkg/ directory is .gitignored (regenerated every build) so the
// `.npmignore` that rescues it from .gitignore during `npm pack` can't be
// committed. Recreate it after every wasm build so fresh clones ship it too.
const npmIgnorePath = resolve(__dirname, '..', 'crates', 'arest', 'pkg', '.npmignore')
writeFileSync(npmIgnorePath, '', 'utf-8')

// Cloudflare Workers' WASM runtime doesn't expose `__wbindgen_start` on
// every instantiation path. wasm-pack's bundler target unconditionally
// invokes it at module load, which crashes the Worker on deploy with
// `TypeError: wasm2.__wbindgen_start is not a function`. Guard the call
// so the module loads whether or not the runtime surfaces the start hook.
const arestJsPath = resolve(__dirname, '..', 'crates', 'arest', 'pkg', 'arest.js')
if (existsSync(arestJsPath)) {
  const jsSrc = readFileSync(arestJsPath, 'utf-8')
  const guarded = jsSrc.replace(
    /^wasm\.__wbindgen_start\(\);$/m,
    'if (typeof wasm.__wbindgen_start === "function") wasm.__wbindgen_start();',
  )
  if (guarded !== jsSrc) {
    writeFileSync(arestJsPath, guarded, 'utf-8')
    console.log('sanitize-wasm-dts: guarded __wbindgen_start() call for CF Workers')
  }
}

// Build-time sanity check: confirm wasm-bindgen actually emitted its core
// allocator intrinsics into the export section. The bundler-target glue in
// `arest_bg.js` calls `wasm.__wbindgen_free` after every owned-string return
// (e.g. `system(...)`), so if that export is missing the deployed Worker
// fails on the first parse with `TypeError: wasm.__wbindgen_free is not a
// function`. We had this regression suspected after composing `wit` into the
// cloudflare feature — verify it's not happening again on every build.
//
// Read the WASM module's export section directly rather than parsing the
// `.d.ts`, since the .d.ts is post-processed above and contains export-name
// lines that don't map 1:1 to WASM exports for WIT-component identifiers.
const wasmPath = resolve(__dirname, '..', 'crates', 'arest', 'pkg', 'arest_bg.wasm')
if (existsSync(wasmPath)) {
  const wasm = readFileSync(wasmPath)
  const required = ['__wbindgen_free', '__wbindgen_malloc', '__wbindgen_realloc']
  const exports = readWasmExportNames(wasm)
  const missing = required.filter(n => !exports.includes(n))
  if (missing.length > 0) {
    console.error(
      `sanitize-wasm-dts: FATAL — wasm-bindgen intrinsics missing from arest_bg.wasm: ${missing.join(', ')}\n` +
      `  the bundler-target JS glue calls these on every system() invocation;\n` +
      `  a Worker built without them will throw TypeError on the first parse.\n` +
      `  Likely cause: cloudflare feature dropped 'dep:wasm-bindgen', or wit-bindgen\n` +
      `  output is being mistakenly post-processed as a Component module.`
    )
    process.exit(2)
  }
}

// Minimal LEB128 + WASM module walker — returns the names listed in the
// export section. Inlined here so the script stays dependency-free; the
// real wasm-tools / binaryen aren't on every contributor's PATH.
function readWasmExportNames(buf) {
  if (buf.length < 8 || buf.readUInt32LE(0) !== 0x6d736100) return []
  let i = 8
  while (i < buf.length) {
    const sectId = buf[i++]
    const [sectLen, after] = readULEB128(buf, i)
    i = after
    if (sectId === 7) {
      const [count, afterCount] = readULEB128(buf, i)
      let p = afterCount
      const names = []
      for (let e = 0; e < count; e++) {
        const [nameLen, afterNameLen] = readULEB128(buf, p)
        p = afterNameLen
        names.push(buf.toString('utf8', p, p + nameLen))
        p += nameLen
        p += 1                         // export kind byte
        const [, afterIdx] = readULEB128(buf, p)
        p = afterIdx                   // export index
      }
      return names
    } else {
      i += sectLen
    }
  }
  return []
}

function readULEB128(buf, offset) {
  let result = 0
  let shift = 0
  let i = offset
  while (true) {
    const b = buf[i++]
    result |= (b & 0x7f) << shift
    if ((b & 0x80) === 0) break
    shift += 7
  }
  return [result, i]
}
