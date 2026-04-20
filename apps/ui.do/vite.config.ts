import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { fileURLToPath, URL } from 'node:url'

/**
 * ui.do — Vite + React + TypeScript front-end for AREST.
 *
 * Direct-API-call config: mdxui providers call the AREST worker at
 *   VITE_AREST_BASE_URL (defaults to https://ui.auto.dev/arest)
 * without a Next.js API proxy in between. The /arest/* surface is the
 * authoritative HATEOAS entry point (per task #131 / #200).
 */
export default defineConfig(({ mode }) => ({
  plugins: [react()],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  define: {
    // Let code read the default without having to know Vite's env conventions.
    __AREST_DEFAULT_BASE_URL__: JSON.stringify('https://ui.auto.dev/arest'),
    __UI_DO_MODE__: JSON.stringify(mode),
  },
  test: {
    globals: true,
    environment: 'jsdom',
    include: ['src/**/*.test.ts', 'src/**/*.test.tsx'],
  },
}))
