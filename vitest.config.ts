import { defineConfig } from 'vitest/config'
import type { Plugin } from 'vite'

// Stub .wasm imports so Vitest can process modules that use Cloudflare's
// CompiledWasm import convention (e.g. `import mod from '...bg.wasm'`).
function wasmStubPlugin(): Plugin {
  return {
    name: 'wasm-stub',
    enforce: 'pre',
    load(id) {
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
