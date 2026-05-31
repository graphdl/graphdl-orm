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

use crate::syscall::arch_prctl;
use crate::syscall::brk;
use crate::syscall::close;
use crate::syscall::exit;
use crate::syscall::futex;
use crate::syscall::getrandom;
use crate::syscall::getpid;
use crate::syscall::identity;
use crate::syscall::ioctl;
use crate::syscall::openat;
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

/// Linux x86_64 syscall number for `close(fd)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_close` (= 3).
/// The vendored musl tree confirms the same value at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_close`. Routes to
/// `close::handle`, which releases the per-process fd-table slot.
pub const SYS_CLOSE: u64 = 3;

/// Linux x86_64 syscall number for `write(fd, buf, count)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_write`. The
/// vendored musl tree carries the same value at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_write` — the
/// kernel and libc agree by construction.
pub const SYS_WRITE: u64 = 1;

/// Linux x86_64 syscall number for `brk(unsigned long addr)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_brk` (= 12).
/// The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_brk`. There is no
/// separate SYS_SBRK on Linux x86_64 — `sbrk(3)` is a C-library
/// wrapper that issues two `brk` calls. Routes to `brk::handle`,
/// which queries or advances `Process::heap_break`; the real
/// page-table install (mapping new heap pages) is gated behind the
/// UEFI boot-integration track (#527 follow-up). Per #509.
pub const SYS_BRK: u64 = 12;

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

/// Linux x86_64 syscall number for
/// `openat(int dirfd, const char *pathname, int flags, mode_t mode)`.
/// Source: `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_openat`
/// (= 257). The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_openat`. Modern
/// libc (glibc 2.26+, musl 1.0.3+) implements `open(2)` as
/// `openat(AT_FDCWD, ...)` so this is the canonical open-side
/// surface. Routes to `openat::handle`, which resolves the path
/// against the synthetic-fs table (`/proc/*` etc) then the File-cell
/// graph (#398) and allocates a per-process fd.
pub const SYS_OPENAT: u64 = 257;

/// Linux x86_64 syscall number for `futex(uaddr, futex_op, val,
/// timeout, uaddr2, val3)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_futex` (= 202).
/// The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_futex`. The
/// foundational primitive for any glibc/musl-built threaded binary's
/// pthread_mutex / pthread_cond implementation — userspace does the
/// fast-path CAS, falls into the kernel only on contention. Routes
/// to `futex::handle`, which dispatches on the operation discriminant
/// (FUTEX_WAIT for the cornerstone block path, FUTEX_WAKE stubbed
/// pending #545, all others -ENOSYS). Per #544 (Track YYYYY).
pub const SYS_FUTEX: u64 = 202;

/// Linux x86_64 syscall number for `getrandom(buf, buflen, flags)`.
/// Source: `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_getrandom`
/// (= 318). The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_getrandom`. Routes
/// to `getrandom::handle`, which fills the userspace buffer from the
/// kernel-wide ChaCha20 CSPRNG (seeded at boot from `arest::entropy`
/// — RDSEED/RDRAND on UEFI x86_64 per #569, host CLI per #574). Caps
/// at 1 MiB per call (POSIX-conformant short read). Flags are ignored
/// — AREST has a single entropy stream. Per #576 (Track Rand-C2).
pub const SYS_GETRANDOM: u64 = 318;

/// Linux x86_64 syscall number for `getpid(void)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_getpid` (= 39).
/// Returns the calling process's pid. Per #501 (process-identity).
pub const SYS_GETPID: u64 = 39;

/// Linux x86_64 syscall number for `getuid(void)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_getuid` (= 102).
/// Tier-1 returns 0 (root uid — single-user kernel). Per #501.
pub const SYS_GETUID: u64 = 102;

/// Linux x86_64 syscall number for `getgid(void)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_getgid` (= 104).
/// Tier-1 returns 0 (root gid). Per #501.
pub const SYS_GETGID: u64 = 104;

/// Linux x86_64 syscall number for `geteuid(void)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_geteuid` (= 107).
/// Tier-1 returns 0. Effective uid == real uid in tier-1 (no setuid).
/// Per #501.
pub const SYS_GETEUID: u64 = 107;

/// Linux x86_64 syscall number for `getegid(void)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_getegid` (= 108).
/// Tier-1 returns 0. Effective gid == real gid in tier-1. Per #501.
pub const SYS_GETEGID: u64 = 108;

/// Linux x86_64 syscall number for `arch_prctl(int code, unsigned long
/// addr)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_arch_prctl`
/// (= 158). The foundational TLS-setup syscall: musl's `__init_tp`
/// calls `ARCH_SET_FS` in `_start`'s first instructions so that every
/// FS-relative access (errno, `pthread_self`, stack canary) resolves
/// through the correct thread pointer. Routes to `arch_prctl::handle`,
/// which stores `fs_base` in the Process struct and, on the real
/// x86_64-UEFI target, also programs the IA32_FS_BASE MSR (0xC0000100).
/// Per #501.
pub const SYS_ARCH_PRCTL: u64 = 158;

/// Linux x86_64 syscall number for
/// `ioctl(int fd, unsigned long request, ...)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_ioctl` (= 16).
/// The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_ioctl`. Routes to
/// `ioctl::handle`, which dispatches on the request code:
/// TIOCGWINSZ (0x5413) — fill `struct winsize` (24 rows × 80 cols);
/// TCGETS (0x5401) — fill a zeroed `struct termios`;
/// unknown → -ENOTTY. Per #502.
pub const SYS_IOCTL: u64 = 16;

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
    r10: u64,
    r8: u64,
    r9: u64,
) -> i64 {
    match rax {
        SYS_WRITE => write::handle(rdi, rsi, rdx),
        SYS_BRK => {
            // brk(addr) — heap-break management. `addr` = rdi.
            // Returns the resulting break (current or new) as a
            // non-negative i64 — never a negative errno per Linux
            // raw-syscall convention. Per #509.
            brk::handle(rdi)
        }
        SYS_CLOSE => close::handle(rdi as i32),
        SYS_OPENAT => openat::handle(rdi as i32, rsi, rdx as u32, r10 as u32),
        SYS_FUTEX => {
            // futex(uaddr, futex_op, val, timeout, uaddr2, val3) per
            // `vendor/musl/arch/x86_64/syscall_arch.h:__syscall6`.
            // Tier-1 only handles FUTEX_WAIT (block on value match) +
            // a FUTEX_WAKE stub; #544 (Track YYYYY) ships this slice,
            // #545 ships the real WAKE, #546+ ship REQUEUE / PI futex.
            futex::handle(rdi, rsi as u32, rdx as u32, r10, r8, r9 as u32)
        }
        SYS_GETRANDOM => {
            // getrandom(buf, buflen, flags) per Linux's
            // `linux/include/uapi/linux/random.h`. Three-arg syscall:
            // rdi = buf, rsi = buflen, rdx = flags. Caps at 1 MiB
            // per call (POSIX-conformant short read); flags are
            // accepted but ignored — AREST has one CSPRNG stream.
            getrandom::handle(rdi, rsi, rdx as u32)
        }
        SYS_GETPID => {
            // getpid() — no arguments; returns current pid as i64.
            // Per #501 (process-identity). Zero-arg: ignore all rdi..r9.
            getpid::handle()
        }
        SYS_GETUID | SYS_GETEUID => {
            // getuid() / geteuid() — tier-1 returns 0 (root uid).
            // No uid model yet; effective == real. Per #501.
            identity::handle_uid()
        }
        SYS_GETGID | SYS_GETEGID => {
            // getgid() / getegid() — tier-1 returns 0 (root gid).
            // Per #501.
            identity::handle_gid()
        }
        SYS_ARCH_PRCTL => {
            // arch_prctl(code, addr) — TLS setup. musl's `__init_tp`
            // calls ARCH_SET_FS (0x1002) in _start's first instructions
            // so that errno / pthread_self / stack-canary all work.
            // rdi = code (u64), rsi = addr (u64). Per #501.
            arch_prctl::handle(rdi, rsi)
        }
        SYS_IOCTL => {
            // ioctl(fd, request, arg) — terminal query stubs.
            // rdi = fd, rsi = request, rdx = arg (pointer to output
            // struct). TIOCGWINSZ (0x5413) fills winsize 24×80;
            // TCGETS (0x5401) fills a zeroed termios; unknown → -ENOTTY.
            // Per #502.
            ioctl::handle(rdi, rsi, rdx)
        }
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

    /// `SYS_OPENAT` is 257 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_openat`.
    #[test]
    fn sys_openat_number_matches_linux_uapi() {
        assert_eq!(SYS_OPENAT, 257);
    }

    /// `SYS_CLOSE` is 3 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_close`.
    #[test]
    fn sys_close_number_matches_linux_uapi() {
        assert_eq!(SYS_CLOSE, 3);
    }

    /// `SYS_FUTEX` is 202 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_futex`.
    #[test]
    fn sys_futex_number_matches_linux_uapi() {
        assert_eq!(SYS_FUTEX, 202);
    }

    /// `SYS_GETRANDOM` is 318 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_getrandom`.
    #[test]
    fn sys_getrandom_number_matches_linux_uapi() {
        assert_eq!(SYS_GETRANDOM, 318);
    }

    /// `SYS_GETPID` is 39 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_getpid`.
    #[test]
    fn sys_getpid_number_matches_linux_uapi() {
        assert_eq!(SYS_GETPID, 39);
    }

    /// `SYS_GETUID` is 102 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_getuid`.
    #[test]
    fn sys_getuid_number_matches_linux_uapi() {
        assert_eq!(SYS_GETUID, 102);
    }

    /// `SYS_GETGID` is 104 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_getgid`.
    #[test]
    fn sys_getgid_number_matches_linux_uapi() {
        assert_eq!(SYS_GETGID, 104);
    }

    /// `SYS_GETEUID` is 107 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_geteuid`.
    #[test]
    fn sys_geteuid_number_matches_linux_uapi() {
        assert_eq!(SYS_GETEUID, 107);
    }

    /// `SYS_GETEGID` is 108 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_getegid`.
    #[test]
    fn sys_getegid_number_matches_linux_uapi() {
        assert_eq!(SYS_GETEGID, 108);
    }

    /// `SYS_ARCH_PRCTL` is 158 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_arch_prctl`.
    #[test]
    fn sys_arch_prctl_number_matches_linux_uapi() {
        assert_eq!(SYS_ARCH_PRCTL, 158);
    }

    /// `SYS_BRK` is 12 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_brk`.
    #[test]
    fn sys_brk_number_matches_linux_uapi() {
        assert_eq!(SYS_BRK, 12);
    }

    /// `SYS_IOCTL` is 16 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_ioctl`.
    #[test]
    fn sys_ioctl_number_matches_linux_uapi() {
        assert_eq!(SYS_IOCTL, 16);
    }

    /// `dispatch(SYS_BRK, 0, ...)` (query form) returns 0 when no
    /// process is installed (the kernel boot state before any spawn).
    /// Verifies the dispatcher routes SYS_BRK (12) to brk::handle
    /// and that the "no current process" sentinel fires.
    #[test]
    fn dispatch_brk_zero_returns_zero_with_no_process() {
        assert_eq!(dispatch(SYS_BRK, 0, 0, 0, 0, 0, 0), 0);
    }

    /// `dispatch(SYS_GETUID, ...)` returns 0 — tier-1 root uid.
    #[test]
    fn dispatch_getuid_returns_zero() {
        assert_eq!(dispatch(SYS_GETUID, 0, 0, 0, 0, 0, 0), 0);
    }

    /// `dispatch(SYS_GETGID, ...)` returns 0 — tier-1 root gid.
    #[test]
    fn dispatch_getgid_returns_zero() {
        assert_eq!(dispatch(SYS_GETGID, 0, 0, 0, 0, 0, 0), 0);
    }

    /// `dispatch(SYS_GETEUID, ...)` returns 0 — effective == real uid.
    #[test]
    fn dispatch_geteuid_returns_zero() {
        assert_eq!(dispatch(SYS_GETEUID, 0, 0, 0, 0, 0, 0), 0);
    }

    /// `dispatch(SYS_GETEGID, ...)` returns 0 — effective == real gid.
    #[test]
    fn dispatch_getegid_returns_zero() {
        assert_eq!(dispatch(SYS_GETEGID, 0, 0, 0, 0, 0, 0), 0);
    }

    /// `dispatch(SYS_ARCH_PRCTL, unknown_code, ...)` returns -EINVAL.
    /// Verifies the dispatcher routes to arch_prctl::handle and the
    /// unknown-subcode guard fires.
    #[test]
    fn dispatch_arch_prctl_unknown_code_returns_einval() {
        assert_eq!(dispatch(SYS_ARCH_PRCTL, 0x0001, 0, 0, 0, 0, 0), -EINVAL);
    }

    /// `futex(NULL, FUTEX_WAIT, 0, ...)` returns -EFAULT — null
    /// uaddr is not a valid futex address. Verifies the dispatcher
    /// routes SYS_FUTEX (202) to the futex handler and the handler's
    /// null-pointer guard fires.
    #[test]
    fn dispatch_futex_null_uaddr_returns_efault() {
        // SYS_FUTEX = 202, uaddr = 0, op = FUTEX_WAIT (0), val = 0,
        // timeout = 0, uaddr2 = 0, val3 = 0. Handler should reject
        // before deref.
        let result = dispatch(SYS_FUTEX, 0, 0, 0, 0, 0, 0);
        assert_eq!(result, -14); // -EFAULT
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
