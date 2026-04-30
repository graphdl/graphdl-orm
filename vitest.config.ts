import { defineConfig } from 'vitest/config'
import { readFileSync } from 'fs'
import { resolve } from 'path'
import type { Plugin } from 'vite'

// Handle .wasm imports for Vitest:
// - For arest_bg.wasm: export a compiled `WebAssembly.Module` so the
//   sanitizer-rewritten `arest.js` (which does
//   `new WebAssembly.Instance(wasm_module, ...)`) can load cleanly
//   under Node. Cloudflare's `CompiledWasm` rule supplies a Module at
//   runtime; vitest needs to mimic that. The previous Uint8Array
//   default broke the moment any .test.ts transitively imported
//   `crates/arest/pkg/arest.js` (#660 wired cell-encryption.ts to it).
// - For all other .wasm files: stub with {} (CompiledWasm convention).
function wasmStubPlugin(): Plugin {
  return {
    name: 'wasm-stub',
    enforce: 'pre',
    load(id) {
      if (id.endsWith('arest_bg.wasm')) {
        const bytes = readFileSync(resolve(id.replace(/\?.*$/, '')))
        const b64 = bytes.toString('base64')
        return `const bytes = Uint8Array.from(atob(${JSON.stringify(b64)}), c => c.charCodeAt(0));
const wasmModule = new WebAssembly.Module(bytes);
export default wasmModule;`
      }
      if (id.endsWith('.wasm')) {
        return 'export default {}'
      }
    },
  }
}

// Stub `cloudflare:workers` so modules importing DurableObject can be loaded
// in Vitest without the Cloudflare runtime.
function cloudflareStubPlugin(): Plugin {
  return {
    name: 'cloudflare-stub',
    enforce: 'pre',
    resolveId(id) {
      if (id === 'cloudflare:workers') {
        return '\0cloudflare:workers'
      }
    },
    load(id) {
      if (id === '\0cloudflare:workers') {
        return 'export class DurableObject {}'
      }
    },
  }
}

export default defineConfig({
  plugins: [wasmStubPlugin(), cloudflareStubPlugin()],
  test: {
    globals: true,
    include: ['src/**/*.test.ts', 'tests/**/*.test.ts', 'scripts/**/*.test.ts'],
    // Run each test file in a fresh Node fork, sequentially. We cannot use
    // singleFork (all-files-in-one-process) because some test files leave
    // behind unsettled timers / open Streams / dangling fetches that
    // accumulate across the run and eventually deadlock vitest somewhere
    // around the 15th file (observed: bisecting 37 files individually all
    // pass in <60s each, but a combined run hangs after verb-dispatcher).
    // We also can't use parallel forks because concurrent loads of
    // arest_bg.wasm intermittently trip "RuntimeError: unreachable" on
    // Windows under load. Sequential isolated forks gives us both: leak
    // containment per file + serialized WASM init.
    fileParallelism: false,
    pool: 'forks',
  },
  // Vitest 4 hoisted poolOptions out of `test.*` to the top level.
  poolOptions: {
    forks: {
      isolate: true,
      singleFork: false,
    },
  },
  resolve: {
    alias: {
      '@': './src',
    },
  },
})
