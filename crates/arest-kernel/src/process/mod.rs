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
// Foundation slice scope (#518, this commit)
// ------------------------------------------
// Just `elf` — the parser. The next-slice loader (#472b) will land
// `process::loader`; the relocation engine #472c lands
// `process::relocate`; the syscall trampoline #472d lands
// `process::syscall`. Each is a sibling submodule that consumes the
// types the parser produces.
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

pub mod elf;

#[cfg(test)]
pub mod test_fixtures;
