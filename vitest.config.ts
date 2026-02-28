import { defineConfig } from 'vitest/config'
import path from 'path'

export default defineConfig({
  test: {
    globals: true,
    setupFiles: ['./test/vitest.setup.ts'],
    testTimeout: 60000,
    hookTimeout: 60000,
    pool: 'forks',
    maxWorkers: 1,
  },
  resolve: {
    alias: {
      '@': path.resolve(__dirname, 'src'),
      '@payload-config': path.resolve(__dirname, 'src/payload.config.ts'),
    },
  },
})
