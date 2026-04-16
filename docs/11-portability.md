# 11 · Runtime Portability Contract

AREST runs on multiple targets without rewriting business logic. This document defines which
primitives are available on each target, what Cargo features to enable, and what breaks when
you leave the standard environment.

## Primitive target map

| Primitive          | Cloudflare Workers | Local (CLI) | x86_64 Kernel | WASM (browser) | FPGA        |
|--------------------|--------------------|-------------|---------------|----------------|-------------|
| `apply`            | supported          | supported   | supported     | supported      | planned     |
| `fetch`            | supported          | supported   | stub          | supported      | n/a         |
| `store`            | supported          | supported   | stub          | stub           | n/a         |
| `def`              | supported          | supported   | supported     | supported      | planned     |
| `compile`          | supported          | supported   | stub          | stub           | n/a         |
| `freeze` / `thaw`  | supported          | supported   | planned       | stub           | n/a         |
| `validate`         | supported          | supported   | supported     | supported      | planned     |
| `derive`           | supported          | supported   | supported     | supported      | planned     |
| `query`            | supported          | supported   | stub          | stub           | n/a         |
| `snapshot` / `rollback` | supported   | supported   | planned       | stub           | n/a         |

**Key:** `supported` = fully implemented, `stub` = returns a deterministic no-op or error,
`planned` = on the roadmap, `n/a` = architecturally excluded.

## Feature flags

Declare features in `Cargo.toml`. The recommended combinations are:

```toml
[features]
default  = ["wit", "debug-def", "std-deps"]

# Bare-metal targets (kernel module, FPGA soft-core)
no_std   = []

# Pull in the std-only dependency set
std-deps = ["serde", "regex", "crypto"]

# Target profiles
cloudflare  = ["wit", "std-deps"]
local       = ["wit", "std-deps", "debug-def"]
wasm-lower  = ["wit"]
parallel    = ["std-deps"]
```

| Feature      | Enables                                      | Requires |
|--------------|----------------------------------------------|----------|
| `wit`        | WIT interface types and component model ABI  | std      |
| `debug-def`  | Pretty-printed AST in error messages         | std      |
| `std-deps`   | `serde`, `regex`, `crypto` crates            | std      |
| `no_std`     | Disables the Rust standard library entirely  | —        |
| `cloudflare` | Workers-specific I/O bindings                | std      |
| `local`      | Filesystem and env-var I/O bindings          | std      |
| `wasm-lower` | Browser-safe WASM ABI without threads        | —        |
| `parallel`   | Rayon-backed parallel derivation passes      | std      |

Activate `no_std` by adding `#![no_std]` to the crate root **and** selecting the
`no_std` feature. Do not combine `no_std` with `std-deps`, `cloudflare`, or `local`.

## Target-specific constraints

### `no_std` (kernel module, FPGA soft-core)

- No `std::env` — configuration must be compiled in or passed through a platform register.
- No `std::fs` — all storage goes through the `Platform` trait; the kernel or FPGA backend
  implements it.
- No `std::time` — timestamps are unavailable; fact ordering uses a monotonic counter
  supplied by the platform.
- No threads — derivation passes run sequentially on a single logical core.
- No heap allocator by default — a future `no_alloc` sub-feature will target fixed-width
  fact tables on FPGA hardware.

### WASM (browser)

- `std::time::Instant` is not available in the browser ABI; use the JS `performance.now()`
  shim exposed through the `Platform` trait instead.
- No POSIX threads; use the `wasm-lower` feature which disables `parallel`.
- `crypto` requires the Web Crypto API injected through the `Platform` trait rather than
  the OS keyring.

### FPGA (future)

- The `no_alloc` sub-feature (planned) replaces `Vec` and `HashMap` with statically-sized
  arrays; all fact-type counts must be known at compile time.
- Only fixed-width integer and Boolean fact values are supported in the initial synthesis
  pass; floating-point and variable-length strings require a soft-core CPU.
- The Verilog generator (see [07-generators.md](07-generators.md)) is the intended
  compilation path; do not expect the AREST runtime binary to run directly on FPGA fabric.

## The portability guarantee

The paper states: *SYSTEM is one function — readings in, applications out.*

Every target runs the same `ast::apply`:

```
ast::apply(env: &Env, expr: &Expr) -> Value
```

The `Env` captures all fact bindings; `Expr` is the compiled AST. Neither mentions I/O,
time, or file systems. Variation is confined to two trait objects:

- **`Platform`** — resolves named functions the readings delegate to (e.g. `send_email`,
  `hash_password`). Each target provides its own `impl Platform`.
- **`Native`** — handles persistence and network I/O for `fetch`, `store`, `query`, and
  `snapshot`. Each target provides its own `impl Native`.

As long as a target can provide those two implementations, `ast::apply` produces identical
outputs for identical inputs. Business logic — constraints, derivations, state machines —
is tested once and ported for free.

## What's next

[Back to self-modification](10-self-modification.md) · [Generators](07-generators.md)
