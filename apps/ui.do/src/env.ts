/**
 * Runtime config for ui.do.
 *
 * `VITE_AREST_BASE_URL` points at the AREST worker's /arest/* surface.
 * In production that's https://ui.auto.dev/arest; in local dev callers
 * can point at a Wrangler-hosted worker. The default is hard-coded here
 * so `import.meta.env.VITE_AREST_BASE_URL` can be missing without
 * breaking the boot.
 */
export const AREST_BASE_URL: string =
  (import.meta.env.VITE_AREST_BASE_URL as string | undefined) ??
  'https://ui.auto.dev/arest'
