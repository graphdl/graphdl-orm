// crates/arest-kernel/src/main.rs
//
// AREST UEFI kernel binary entry point. Thin shim — the actual module
// tree (every `pub mod` declaration, the `arest_http_handler`, all the
// `#[cfg(target_os = "uefi")]`-gated entry harnesses with their
// `#[entry]`-macro–derived `_start` symbols) lives in `lib.rs`.
//
// Why the bin is so thin
// ----------------------
// #579 Track QQQQQ extracted the kernel's modules into a `[lib]`
// target so `cargo test --lib -p arest-kernel` runs the inline
// `#[cfg(test)]` blocks scattered through `process/` / `syscall/` /
// `synthetic_fs/` / `composer.rs` / `component_binding.rs` / etc.
// Pre-extract, the crate was bin-only with `[[bin]] test = false` and
// the inline tests silently never ran (#460 / #498 / #534 / et al.
// shipped "documentation-style" tests that the runner never reached).
//
// The `[lib]` target is the source of truth — this file's job is just
// to drag in the library so its `_start` symbol (defined in
// `entry_uefi*.rs`'s `#[entry] fn efi_main` expansion) lands in the
// linked PE32+ image the firmware loads.
//
// Why the entry harnesses live in the lib, not in the bin
// -------------------------------------------------------
// The `#[entry]` macro from uefi-rs expands to `#[no_mangle] pub
// extern "efiapi" fn efi_main(...) -> Status`. That `pub`-marked symbol
// gets exported from whichever compilation unit it lands in — the
// linker then resolves the PE32+ entry point against it regardless of
// bin vs lib origin. Keeping the harness in the lib (rather than
// duplicating it in the bin) means the lib carries the full kernel
// surface — the bin is a one-liner that pulls the lib's symbols
// through, and `cargo test --lib` exercises the same module bodies
// that `cargo build --target x86_64-unknown-uefi` ships.
//
// Why `#![no_main]` stays
// -----------------------
// UEFI binaries don't have a Rust `fn main` — the firmware probes for
// `_start` (the PE32+ AddressOfEntryPoint), which the lib's `#[entry]`
// macro expansion provides. `#![no_main]` tells rustc not to expect /
// generate a `main` symbol; without it the linker emits an
// "undefined reference to `main`" error. Same shape every UEFI Rust
// program (the rust-osdev "Writing an OS in Rust" book, redox-os) uses.

#![no_std]
#![no_main]
// Mirror of lib.rs — the abi_x86_interrupt feature is needed for any
// x86_64 build that installs an IDT with `extern "x86-interrupt" fn`
// handlers. The bin compiles against the same nightly toolchain as
// the lib, so the gate stays in sync.
#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]

// The bin links against the lib via this `use`. Cargo notices the
// dependency and pulls the lib's compilation unit into the binary's
// link line; the `_start` symbol from the lib's `entry_uefi*.rs`
// `#[entry]` macro expansion lands in the final PE32+ image.
//
// `as _` rather than `as arest_kernel` because we don't reference any
// items by name from this file — the `use` statement's only purpose is
// to force the link.
#[allow(unused_imports)]
use arest_kernel as _;
