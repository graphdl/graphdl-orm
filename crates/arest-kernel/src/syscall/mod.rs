// crates/arest-kernel/src/syscall/mod.rs
//
// Linux syscall dispatch surface (#473a, foundation for the userspace
// syscall epic #473). The trampoline (`process::trampoline::invoke`,
// pending #552 wires the SYSCALL MSR + the actual ring-3 gate) will,
// once it lands, route every `syscall` instruction issued from a Linux
// userspace process into `dispatch::dispatch`. The dispatcher matches
// on rax (the Linux x86_64 syscall number per
// `linux/arch/x86/include/uapi/asm/unistd_64.h`) and forwards the
// remaining argument registers (rdi / rsi / rdx / r10 / r8 / r9) to
// the right handler module.
//
// What this slice ships
// ---------------------
// Two handlers (the smallest viable surface for "hello world from a
// static Linux ELF"):
//
//   * `write` — syscall 1. Three-arg `write(fd, buf, count)`. For
//     `fd == 1` (stdout) the bytes route to the kernel's serial
//     console via `crate::print!`. For any other fd the handler
//     returns `-EBADF`. Read fd 0 (`#508`), open / openat (`#509`),
//     and the rest of the file-descriptor surface land in follow-up
//     tracks.
//
//   * `exit` — syscalls 60 (`exit`) + 231 (`exit_group`). Both mark
//     the calling Process's state machine `Exited` via the
//     `process::current_process` accessor + must never return to
//     userspace. Tier-1 doesn't have a scheduler so `exit_group`
//     leaves the kernel idling (`arch::halt_forever`); a real
//     scheduler (#530) will yield to the next runnable process
//     instead.
//
// Out of scope for this slice (intentionally — see #473 epic):
//
//   * brk / mmap / munmap (memory management — #497, #509-followups).
//   * SYSCALL MSR programming + the ring-3 gate (#552).
//   * Per-process fd table mutations beyond `Serial` stdin/stdout/
//     stderr (#560 vfs slice).
//   * 32-bit syscall arms (i386 / armv7-eabi compat — likely never).
//
// Why per-arch is x86_64-only
// ---------------------------
// Linux syscall ABIs differ per architecture (rax vs x8 vs r7, etc.)
// and the table of numbers is itself per-arch (the 1/60/231 numbers
// here are the x86_64 numbers; aarch64 has 64/93/94 for the same three
// syscalls). The dispatcher's signature accepts the x86_64 register
// names verbatim because that's the ABI the trampoline (#552) feeds
// it. The aarch64 / armv7 trampolines (#553 onward) will get their
// own dispatcher arms — different signatures, different numbers,
// different syscall instruction. For now the module compiles on
// every arch (the dispatch fn is `pub` and arch-neutral — pure u64
// arithmetic + match) so `cargo check --target aarch64-unknown-uefi`
// stays green; the actual SYSCALL entry that calls into it is
// x86_64-only and lives in #552.

#![allow(dead_code)]

pub mod close;
pub mod dispatch;
pub mod exit;
pub mod futex;
pub mod getrandom;
pub mod openat;
pub mod write;

// Re-export the dispatcher as the public surface — this is what the
// future #552 SYSCALL MSR entry (`arch::uefi::syscall_entry` or
// similar) will call once the ring-3 gate is wired. Keeping the
// import path short so the entry-side asm shim doesn't have to
// reach four modules deep.
pub use dispatch::{dispatch, EBADF, EFAULT, EINVAL};
pub use openat::{AT_FDCWD, EACCES, EMFILE, ENOENT, O_ACCMODE, O_RDONLY, O_RDWR, O_WRONLY};
