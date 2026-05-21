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
  // Local dev server. Same-origin proxy so the browser never makes a
  // cross-origin (CORS) request to the Wrangler worker: the SPA calls
  // /arest and /api on its own origin (localhost:5174) and Vite
  // forwards them to the worker on AREST_LOCAL_WORKER (default 8788).
  // Set VITE_AREST_BASE_URL=/arest in .env.local so the providers use
  // these proxied, same-origin paths. See LOCAL-DEV.md.
  server: {
    port: 5174,
    // Bind all interfaces (IPv4 + IPv6). On Windows, the default
    // 'localhost' bind can resolve to ::1 only, so a client hitting
    // 127.0.0.1 (e.g. a headless browser) gets ECONNREFUSED. host:true
    // makes both 127.0.0.1 and ::1 reachable.
    host: true,
    proxy: {
      // The local worker's first (cold) request can take ~10s while the
      // WASM engine boots, so give the proxy a generous timeout — the
      // default would surface a spurious 504/000 on the first call.
      '/arest': {
        target: process.env.AREST_LOCAL_WORKER ?? 'http://127.0.0.1:8788',
        changeOrigin: true,
        timeout: 60000,
        proxyTimeout: 60000,
      },
      '/api': {
        target: process.env.AREST_LOCAL_WORKER ?? 'http://127.0.0.1:8788',
        changeOrigin: true,
        timeout: 60000,
        proxyTimeout: 60000,
      },
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
    // @mdxui/admin's dist uses ESM directory imports ("./components/layout")
    // which Node's native resolver rejects. Inlining routes the import
    // through Vite's resolver (same path Vite uses at build time), so
    // jsdom sees the same module graph the browser does.
    server: { deps: { inline: [/@mdxui\//] } },
  },
}))
