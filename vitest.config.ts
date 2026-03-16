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

export default defineConfig({
  plugins: [wasmStubPlugin()],
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
