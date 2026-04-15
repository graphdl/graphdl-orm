#!/usr/bin/env bun
/**
 * Bun + V8 WASM benchmark — the production-equivalent side of the
 * comparison. Pair with:
 *
 *   cargo test --features wasm-lower --release --lib \
 *     bench_wasm_fixtures_and_write -- --ignored --nocapture
 *
 * That command writes target/wasm_fixtures/*.wasm. This script loads
 * each fixture into V8's WASM JIT via Bun, runs the identical tight
 * loop, and prints ns/op. Compare the numbers against the Rust and
 * wasmi columns the cargo test printed.
 *
 * V8 JITs the module on first call; the loop runs against native
 * code thereafter. Expect results within ~2× of the Rust native row
 * — that's the real "WASM+Bun vs Rust exe" story, not the
 * interpreter-tax that wasmi shows.
 *
 * If the fixture files are missing, this script exits with a clear
 * pointer to the cargo command that creates them — running Bun
 * without fixtures would otherwise surface as a cryptic ENOENT.
 */

import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'

const ITERATIONS = 10_000_000
const FIXTURES = [
  { name: 'id',             input: 42n },
  { name: 'constant_seven', input: 42n },
]

// Existence check before we start so the failure mode is actionable.
// The cargo test writes into crates/arest/target/wasm_fixtures; this
// script is expected to run from the repo root.
const FIXTURE_DIR = join('crates', 'arest', 'target', 'wasm_fixtures')
const missing = FIXTURES
  .map(fx => join(FIXTURE_DIR, `${fx.name}.wasm`))
  .filter(p => !existsSync(p))
if (missing.length > 0) {
  console.error('Missing WASM fixtures — generate them first:')
  console.error('  cargo test --features wasm-lower --release --lib \\')
  console.error('    bench_wasm_fixtures_and_write -- --ignored --nocapture')
  console.error('\nMissing paths:')
  for (const p of missing) console.error(`  ${p}`)
  process.exit(1)
}

console.log(`\n=== WASM+Bun benchmark — ${ITERATIONS.toLocaleString()} iterations ===`)
console.log(`${'case'.padEnd(18)} ${'bun-ns/op'.padStart(12)} ${'ops/sec'.padStart(16)}`)

for (const fx of FIXTURES) {
  const path = join(FIXTURE_DIR, `${fx.name}.wasm`)
  const bytes = readFileSync(path)
  const mod = new WebAssembly.Module(bytes)
  const instance = new WebAssembly.Instance(mod)
  // apply returns an i32 pointer into linear memory. Each call resets
  // the heap at entry, so the returned ptr is valid until the next call.
  // The bench treats the ptr as a black-box scalar for timing; the
  // memory export is unused here.
  const apply = instance.exports.apply as (x: bigint) => number

  // Warmup to let V8 JIT-optimise the call path.
  let warmup = 0
  for (let i = 0; i < 100_000; i++) {
    warmup += apply(fx.input)
  }
  // Black-box the warmup result so the compiler can't dead-code it.
  if (warmup === 999999999) console.log('should never hit')

  const t0 = Bun.nanoseconds()
  let acc = 0
  for (let i = 0; i < ITERATIONS; i++) {
    acc += apply(fx.input)
  }
  const dt = Bun.nanoseconds() - t0
  if (acc === 999999999) console.log('should never hit')

  const ns_per_op = dt / ITERATIONS
  const ops_per_sec = 1_000_000_000 / ns_per_op
  console.log(`${fx.name.padEnd(18)} ${ns_per_op.toFixed(1).padStart(12)} ${ops_per_sec.toExponential(2).padStart(16)}`)
}

console.log('\nRun the cargo test first to regenerate target/wasm_fixtures/*.wasm.')
