/**
 * PostCSS — feeds Tailwind + Autoprefixer to Vite.
 * Vite picks this up automatically; no extra wiring in vite.config.ts.
 */
export default {
  plugins: {
    tailwindcss: {},
    autoprefixer: {},
  },
}
