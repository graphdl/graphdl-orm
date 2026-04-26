// crates/arest-kernel/src/syscall/dispatch.rs
//
// The dispatch table for Linux x86_64 syscalls. Pure router — match
// on `rax` (the syscall number per
// `linux/arch/x86/include/uapi/asm/unistd_64.h`) and fan out to the
// per-syscall handler module. The result is returned in the Linux
// convention: a non-negative value is the syscall's success result;
// a negative value is `-errno` (per `<asm-generic/errno-base.h>` +
// `<asm-generic/errno.h>`).
//
// Why a fixed signature
// ---------------------
// Six register-passed arguments is the Linux x86_64 ABI maximum
// (`__syscall6` in `vendor/musl/arch/x86_64/syscall_arch.h:53` is
// the canonical reference). Keeping the dispatch fn at six u64s
// (rdi / rsi / rdx / r10 / r8 / r9, in that order) means the future
// #552 SYSCALL MSR entry (`arch::uefi::syscall_entry`) can pass the
// argument registers verbatim without an arity-by-arity branch.
// Handlers that take fewer arguments simply ignore the trailing
// registers — the cost of an unused-arg pass is one register's worth
// of stack vs. branching on the syscall number twice.
//
// Why i64 (not u64) return
// ------------------------
// Linux returns a `long`, which on x86_64 is 64-bit signed. The
// negative-errno convention requires the sign bit; libc unwraps via
// `if (ret < 0) { errno = -ret; ret = -1; }`. Returning u64 would
// force the caller to re-cast for that check on every syscall.
//
// errno value provenance
// ----------------------
// The numeric values come from `<asm-generic/errno-base.h>` (the
// Linux uapi header) which is the same set of numbers musl, glibc,
// and every other libc on Linux uses. The three constants exposed
// here are the only ones the tier-1 handlers need:
//
//   * `EBADF`  =  9   "Bad file descriptor"
//   * `EFAULT` = 14   "Bad address"
//   * `EINVAL` = 22   "Invalid argument"
//
// Future handlers will grow the constant set; intentionally leaving
// the table sparse keeps the surface honest about what's actually
// returned today.
//
// Unknown syscall behaviour
// -------------------------
// Returning `-ENOSYS` (38) lets a static binary compiled against musl
// detect "this kernel doesn't implement this syscall" via the standard
// `if (errno == ENOSYS)` test that musl/glibc both perform around
// optional syscalls (futex, getrandom, etc.). Eventually #530's
// scheduler will lock-step this against the trace surface so an
// unknown syscall is logged rather than silently failing — but for
// tier-1 the negative return is enough.

use crate::syscall::exit;
use crate::syscall::write;

/// Linux errno value for "Bad file descriptor". Returned by `write`
/// when the fd isn't open (anything other than 0/1/2 in tier-1) and
/// by `read` (#508) when the same condition holds.
pub const EBADF: i64 = 9;

/// Linux errno value for "Bad address". Returned when a syscall's
/// pointer argument can't be dereferenced — null, or pointing outside
/// the process's address space. Reserved for future use; tier-1
/// `write` accepts any non-null pointer (the trampoline's identity
/// mapping means kernel pointers and userspace pointers coincide;
/// see `process::process` line 241).
pub const EFAULT: i64 = 14;

/// Linux errno value for "Invalid argument". Returned when an enum-
/// shaped argument has a value outside the spec's allowed set
/// (e.g., `mmap` flags with both `MAP_PRIVATE` and `MAP_SHARED`).
/// Reserved for future use; tier-1 handlers don't yet need it.
pub const EINVAL: i64 = 22;

/// Linux errno for "Function not implemented". Returned for any
/// syscall number this dispatcher doesn't yet handle. Static binaries
/// linked against musl / glibc test for this on optional syscalls
/// (futex, getrandom, etc.) so the negative return propagates as a
/// clean "this kernel can't" rather than silent failure.
pub const ENOSYS: i64 = 38;

/// Linux x86_64 syscall number for `write(fd, buf, count)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_write`. The
/// vendored musl tree carries the same value at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_write` — the
/// kernel and libc agree by construction.
pub const SYS_WRITE: u64 = 1;

/// Linux x86_64 syscall number for `exit(status)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_exit`. Tier-1
/// treats `exit` and `exit_group` identically — there's no thread
/// model yet so the per-thread vs per-process distinction is moot;
/// both transition the calling Process to `Exited` and never return.
/// The distinction matters once #530's scheduler grows POSIX threads
/// (#560 onward).
pub const SYS_EXIT: u64 = 60;

/// Linux x86_64 syscall number for `exit_group(status)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_exit_group`. The
/// glibc / musl `_exit(3)` typically issues this rather than `exit`
/// (60) so every thread in the calling process group exits in one
/// shot. For tier-1 (single-threaded model) it's the same as
/// `SYS_EXIT`; both route to `exit::handle`.
pub const SYS_EXIT_GROUP: u64 = 231;

/// The dispatch entry point. Match on `rax` and forward the argument
/// registers (rdi / rsi / rdx / r10 / r8 / r9) to the per-syscall
/// handler. Handlers that take fewer than six args simply ignore the
/// trailing slots.
///
/// Returns a Linux-convention `long`: non-negative = success result,
/// negative = `-errno`. Per `<asm-generic/errno.h>`. The future #552
/// SYSCALL MSR entry's asm shim writes this value back into rax
/// before `sysretq`.
///
/// `exit` and `exit_group` are special-cased — they MUST NOT return
/// to userspace. The handler function for those two diverges (returns
/// `!`); to satisfy the dispatcher's `i64` return type we wrap the
/// call in a `match` arm that calls the handler unconditionally.
/// Any caller that observed a return from this function for an exit
/// syscall would observe a `unreachable!()` panic (caught by the
/// kernel's panic handler — same path the trampoline's failure modes
/// take).
pub fn dispatch(
    rax: u64,
    rdi: u64,
    rsi: u64,
    rdx: u64,
    _r10: u64,
    _r8: u64,
    _r9: u64,
) -> i64 {
    match rax {
        SYS_WRITE => write::handle(rdi, rsi, rdx),
        SYS_EXIT | SYS_EXIT_GROUP => {
            // exit / exit_group both transition the Process state
            // machine to `Exited` and must never return. The handler's
            // signature is `! ` (diverges); calling through the match
            // arm gives the dispatcher the unreachable-after-handler
            // shape the i64 return type needs.
            exit::handle(rdi as i32)
        }
        _ => -ENOSYS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `EBADF` is 9 — matches `<asm-generic/errno-base.h>:EBADF`.
    /// Static check so a future "let's just use a different number"
    /// refactor surfaces in the test diff.
    #[test]
    fn ebadf_value_matches_linux_uapi() {
        assert_eq!(EBADF, 9);
    }

    /// `EFAULT` is 14 — matches `<asm-generic/errno-base.h>:EFAULT`.
    #[test]
    fn efault_value_matches_linux_uapi() {
        assert_eq!(EFAULT, 14);
    }

    /// `EINVAL` is 22 — matches `<asm-generic/errno-base.h>:EINVAL`.
    #[test]
    fn einval_value_matches_linux_uapi() {
        assert_eq!(EINVAL, 22);
    }

    /// `ENOSYS` is 38 — matches `<asm-generic/errno.h>:ENOSYS`.
    #[test]
    fn enosys_value_matches_linux_uapi() {
        assert_eq!(ENOSYS, 38);
    }

    /// `SYS_WRITE` is 1 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_write`.
    #[test]
    fn sys_write_number_matches_linux_uapi() {
        assert_eq!(SYS_WRITE, 1);
    }

    /// `SYS_EXIT` is 60 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_exit`.
    #[test]
    fn sys_exit_number_matches_linux_uapi() {
        assert_eq!(SYS_EXIT, 60);
    }

    /// `SYS_EXIT_GROUP` is 231 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_exit_group`.
    #[test]
    fn sys_exit_group_number_matches_linux_uapi() {
        assert_eq!(SYS_EXIT_GROUP, 231);
    }

    /// Unknown syscall numbers return `-ENOSYS`. musl + glibc both
    /// branch on this when probing optional syscalls (futex,
    /// getrandom, etc.).
    #[test]
    fn unknown_syscall_returns_minus_enosys() {
        // pick a number well outside the implemented set
        let result = dispatch(9999, 0, 0, 0, 0, 0, 0);
        assert_eq!(result, -ENOSYS);
    }

    /// `write(2, ...)` (stderr — currently unsupported) returns
    /// `-EBADF`. Verifies the dispatcher correctly routes to the
    /// write handler and the write handler's fd-validation arm fires.
    /// (Tier-1 only opens fd 1; fd 2 is reserved by the Process
    /// construction but the handler currently treats anything other
    /// than 1 as closed.)
    #[test]
    fn dispatch_write_to_unsupported_fd_returns_ebadf() {
        // fd 2 (stderr), arbitrary buf, zero count
        let result = dispatch(SYS_WRITE, 2, 0, 0, 0, 0, 0);
        assert_eq!(result, -EBADF);
    }
}
