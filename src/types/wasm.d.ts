/**
 * Ambient module declarations for WASM imports.
 *
 * The generated `crates/arest/pkg/arest_bg.wasm.d.ts` contains WIT component
 * identifiers with characters invalid in TypeScript (`:`, `/`, `@`, `#`) and
 * is excluded from the project in tsconfig.json. This file supplies the
 * shape TypeScript needs for `import wasmModule from '...arest_bg.wasm'`.
 */

declare module '*.wasm' {
  const value: WebAssembly.Module
  export default value
}

declare module '*/arest_bg.wasm' {
  const value: WebAssembly.Module
  export default value
}
