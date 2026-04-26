// crates/arest-kernel/src/process/mod.rs
//
// `process` — Linux-process foundation (#472 epic). First slice was
// #518: ELF64 header + program-header parsing into a `ParsedElf`
// value. Second slice (#519 + #520) added the in-memory
// `AddressSpace` model + `load_segments(&ParsedElf, &[u8])` that
// walks PT_LOAD entries into page-aligned heap allocations. Third
// slice (#521 — this commit) ships the `Process` struct + the
// initial-stack builder + the privilege-transition trampoline so a
// freshly-loaded binary can be brought to the doorstep of ring 3.
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
// Loader slice scope (#519 + #520)
// --------------------------------
// `elf` (the parser, foundation #518) plus `address_space` (the
// in-memory representation of a loaded process — owns one
// `LoadedSegment` per PT_LOAD, page-aligned heap allocation, drop-
// reclaims). `elf::load_segments(&ParsedElf, &[u8])` is the bridge:
// consumes a parsed header + the original blob, returns a populated
// `AddressSpace`. PT_INTERP detection (#520) is folded in — the
// loader rejects dynamic binaries up front with
// `LoaderError::DynamicLoaderRequired`.
//
// Spawn slice scope (#521 — this commit)
// --------------------------------------
// Three new submodules: `process` (the `Process` struct holding
// pid / address_space / fd_table / state, plus `Process::spawn` that
// orchestrates the spawn pipeline), `stack` (the System V AMD64 PSABI
// initial-stack builder — argv / envp / auxv layout per spec), and
// `trampoline` (the privilege-transition shim that sets up the iretq
// frame and would `iretq` to ring 3 once #526's GDT/TSS scaffolding
// + #527's page-table install land). The trampoline currently fails
// at the actual ring-3 jump because those prerequisites are pending;
// the entire SETUP path is exercised end-to-end + unit-tested.
//
// Gating
// ------
// `elf` and `address_space` are available unconditionally — pure
// byte-slice arithmetic. `process` and `stack` are also unconditional
// (no arch-specific intrinsics — the System V auxv numeric constants
// are arch-agnostic per the generic ABI supplement). `trampoline`
// has per-arch `cfg(target_arch = "...")` arms: x86_64 returns
// `NotYetImplemented` (waiting on #526 + #527); aarch64 / armv7
// return `UnsupportedArch`; the host-target arm (cargo test on
// Windows / Linux / Darwin) also returns `UnsupportedArch` so the
// crate compiles + the unit tests run cross-platform.

#![allow(dead_code)]

pub mod address_space;
pub mod elf;
pub mod process;
pub mod stack;
pub mod trampoline;

// Re-exports for the call site that wants the loader as a one-liner.
// Keeps the cross-module path short for the eventual #525 ld-musl:
//   use crate::process::{load_segments, AddressSpace, Process};
pub use address_space::{AddressSpace, LoadedSegment, LoaderError, SegmentPerm};
pub use elf::{load_segments, LoadOrParseError};
pub use process::{FdEntry, Process, ProcessState, SpawnError};
pub use stack::{AuxvEntry, AuxvType, InitialStack, StackBuilder, StackError};
pub use trampoline::{IretqFrame, TrampolineError};

#[cfg(test)]
pub mod test_fixtures;
