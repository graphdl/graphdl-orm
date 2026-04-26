// crates/arest-kernel/src/process/mod.rs
//
// `process` — Linux-process foundation (#472 epic). First slice is
// #518: ELF64 header + program-header parsing into a `ParsedElf`
// value. No memory mapping yet, no relocations yet, no syscall ABI
// yet — those land in #472b / #472c / #472d respectively.
//
// Why a top-level `process` module
// --------------------------------
// AAAA's `linuxkpi/` (#460) is the kernel-side surface that lets
// unmodified Linux kernel C drivers link against AREST primitives —
// it's about *driver* hosting. A *process* (an unmodified Linux
// userspace ELF binary like /bin/sh, /bin/true, or any libc-built
// program) is a different layer: the kernel needs a parser, a
// loader, an address-space carver, and a syscall trampoline. None
// of those concerns belong inside `linuxkpi/` (that module's docstring
// already calls itself "FreeBSD-style Linux kernel API shim"); they
// need their own module tree.
//
// Loader slice scope (#519 + #520, second commit)
// ------------------------------------------------
// `elf` (the parser, foundation #518) plus `address_space` (the
// in-memory representation of a loaded process — owns one
// `LoadedSegment` per PT_LOAD, page-aligned heap allocation, drop-
// reclaims). `elf::load_segments(&ParsedElf, &[u8])` is the bridge:
// consumes a parsed header + the original blob, returns a populated
// `AddressSpace`. PT_INTERP detection (#520) is folded in — the
// loader rejects dynamic binaries up front with
// `LoaderError::DynamicLoaderRequired`. The relocation engine #472c
// and the syscall trampoline #472d follow as separate sibling
// submodules.
//
// Gating
// ------
// Available unconditionally — the parser has no syscall surface, no
// arch-specific intrinsics, no Linux-only headers. Pure byte-slice
// arithmetic on a `&[u8]`. The downstream loader will need a
// per-arch carve path (page-table flags differ x86_64 vs aarch64),
// at which point that submodule will get its own `cfg(target_arch)`
// gate; the parser stays portable.

#![allow(dead_code)]

pub mod address_space;
pub mod elf;

// Re-exports for the call site that wants the loader as a one-liner.
// Keeps the cross-module path short for the eventual #521 trampoline:
//   use crate::process::{load_segments, AddressSpace};
pub use address_space::{AddressSpace, LoadedSegment, LoaderError, SegmentPerm};
pub use elf::{load_segments, LoadOrParseError};

#[cfg(test)]
pub mod test_fixtures;
