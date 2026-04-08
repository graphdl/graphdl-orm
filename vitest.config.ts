import { defineConfig } from 'vitest/config'
import { readFileSync } from 'fs'
import { resolve } from 'path'
import type { Plugin } from 'vite'

// Handle .wasm imports for Vitest:
// - For arest_bg.wasm: export the actual file bytes as a Uint8Array so initSync works.
// - For all other .wasm files: stub with {} (Cloudflare CompiledWasm convention).
function wasmStubPlugin(): Plugin {
  return {
    name: 'wasm-stub',
    enforce: 'pre',
    load(id) {
      if (id.endsWith('arest_bg.wasm')) {
        const bytes = readFileSync(resolve(id.replace(/\?.*$/, '')))
        const b64 = bytes.toString('base64')
        return `const bytes = Uint8Array.from(atob(${JSON.stringify(b64)}), c => c.charCodeAt(0)); export default bytes;`
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
  },
  resolve: {
    alias: {
      '@': './src',
    },
  },
})
