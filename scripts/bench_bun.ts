#!/usr/bin/env bun
/**
 * Bun + V8 WASM benchmark — the production-equivalent side of the
 * comparison. Pair with:
 *
 *   cargo test --features wasm-lower --release --lib \
 *     bench_and_emit_wasm_fixtures -- --ignored --nocapture
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
 */

import { readFileSync } from 'node:fs'
import { join } from 'node:path'

const ITERATIONS = 10_000_000
const FIXTURES = [
  { name: 'id',             input: 42n },
  { name: 'constant_seven', input: 42n },
]

console.log(`\n=== WASM+Bun benchmark — ${ITERATIONS.toLocaleString()} iterations ===`)
console.log(`${'case'.padEnd(18)} ${'bun-ns/op'.padStart(12)} ${'ops/sec'.padStart(16)}`)

for (const fx of FIXTURES) {
  // Fixtures are written by the cargo test from crates/arest, so
  // they land in crates/arest/target/wasm_fixtures. Run this script
  // from the repo root.
  const path = join('crates', 'arest', 'target', 'wasm_fixtures', `${fx.name}.wasm`)
  const bytes = readFileSync(path)
  const mod = new WebAssembly.Module(bytes)
  const instance = new WebAssembly.Instance(mod)
  const apply = instance.exports.apply as (x: bigint) => bigint

  // Warmup to let V8 JIT-optimise the call path.
  let warmup = 0n
  for (let i = 0; i < 100_000; i++) {
    warmup += apply(fx.input)
  }
  // Black-box the warmup result so the compiler can't dead-code it.
  if (warmup === 999999999999n) console.log('should never hit')

  const t0 = Bun.nanoseconds()
  let acc = 0n
  for (let i = 0; i < ITERATIONS; i++) {
    acc += apply(fx.input)
  }
  const dt = Bun.nanoseconds() - t0
  if (acc === 999999999999n) console.log('should never hit')

  const ns_per_op = dt / ITERATIONS
  const ops_per_sec = 1_000_000_000 / ns_per_op
  console.log(`${fx.name.padEnd(18)} ${ns_per_op.toFixed(1).padStart(12)} ${ops_per_sec.toExponential(2).padStart(16)}`)
}

console.log('\nRun the cargo test first to regenerate target/wasm_fixtures/*.wasm.')
