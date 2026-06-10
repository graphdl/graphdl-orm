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

use crate::syscall::accept;
use crate::syscall::arch_prctl;
use crate::syscall::bind;
use crate::syscall::brk;
use crate::syscall::close;
use crate::syscall::connect;
use crate::syscall::exit;
use crate::syscall::futex;
use crate::syscall::getrandom;
use crate::syscall::getpid;
use crate::syscall::identity;
use crate::syscall::ioctl;
use crate::syscall::listen;
use crate::syscall::mmap;
use crate::syscall::openat;
use crate::syscall::read;
use crate::syscall::recvfrom;
use crate::syscall::robust_list;
use crate::syscall::rt_sigaction;
use crate::syscall::rt_sigprocmask;
use crate::syscall::rt_sigreturn;
use crate::syscall::sendto;
use crate::syscall::socket;
use crate::syscall::stat;
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

/// Linux x86_64 syscall number for `read(fd, buf, count)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_read` (= 0).
/// The vendored musl tree confirms the same value at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_read`. Routes to
/// `read::handle`, which drains decoded Unicode keystrokes from the
/// kernel PS/2 keyboard ring (`arch::uefi::keyboard`) into the
/// caller's buffer for fd 0 (stdin). Non-blocking: returns 0 when the
/// ring is empty, `-EBADF` for any fd != 0. Per #508.
pub const SYS_READ: u64 = 0;

/// Linux x86_64 syscall number for
/// `mmap(addr, len, prot, flags, fd, off)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_mmap` (= 9).
/// The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_mmap`. Routes to
/// `mmap::handle_mmap`, which implements a monotonic bump allocator for
/// the anonymous (MAP_ANONYMOUS) path; file-backed requests return
/// `-ENODEV`. Per #497.
pub const SYS_MMAP: u64 = 9;

/// Linux x86_64 syscall number for `munmap(addr, len)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_munmap` (= 11).
/// The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_munmap`. Routes to
/// `mmap::handle_munmap`, which is a documented no-op in tier-1 (no
/// per-mapping free list; the bump allocator never retreats). Per #497.
pub const SYS_MUNMAP: u64 = 11;

/// Linux x86_64 syscall number for `close(fd)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_close` (= 3).
/// The vendored musl tree confirms the same value at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_close`. Routes to
/// `close::handle`, which releases the per-process fd-table slot.
pub const SYS_CLOSE: u64 = 3;

/// Linux x86_64 syscall number for `stat(pathname, statbuf)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_stat` (= 4).
/// The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_stat`. Routes to
/// `stat::handle_stat`, which fills a stubbed `struct stat` (144-byte
/// Linux x86_64 ABI layout) at the caller's statbuf pointer. In tier-1
/// there is no VFS path-resolution layer; any non-null path returns a
/// char-device stat stub. Full path resolution is a follow-up (#500).
pub const SYS_STAT: u64 = 4;

/// Linux x86_64 syscall number for `fstat(fd, statbuf)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_fstat` (= 5).
/// The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_fstat`. Routes to
/// `stat::handle_fstat`, which fills a stubbed `struct stat` for the
/// known tier-1 file descriptors (0/1/2 as char devices with
/// `S_IFCHR | 0o666` and `st_blksize = 4096`); returns `-EBADF` for
/// any other fd. Per #500 (file-state surface).
pub const SYS_FSTAT: u64 = 5;

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
/// (FUTEX_WAIT for the cornerstone block path, FUTEX_WAKE for the
/// release path (#545), all others -ENOSYS). Per #544 (Track YYYYY).
pub const SYS_FUTEX: u64 = 202;

/// Linux x86_64 syscall number for `set_robust_list(struct
/// robust_list_head *head, size_t len)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_set_robust_list`
/// (= 273). The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:274`. Routes to
/// `robust_list::set_robust_list`, which records the calling thread's
/// robust-futex list head so the kernel can run owner-death recovery
/// when the thread exits. glibc/musl register it during thread
/// bring-up (`vendor/musl/src/thread/pthread_create.c`). Per #546.
pub const SYS_SET_ROBUST_LIST: u64 = 273;

/// Linux x86_64 syscall number for `get_robust_list(int pid, struct
/// robust_list_head **head_ptr, size_t *len_ptr)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_get_robust_list`
/// (= 274). The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:275`. Routes to
/// `robust_list::get_robust_list`, which reports the registered head +
/// len back through the out-pointers. Per #546.
pub const SYS_GET_ROBUST_LIST: u64 = 274;

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

/// Linux x86_64 syscall number for `rt_sigaction(signum, act, oldact,
/// sigsetsize)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_rt_sigaction`
/// (= 13). The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:14`. Routes to
/// `rt_sigaction::handle`, which installs/replaces the per-process
/// disposition for a signal (or SIG_DFL/SIG_IGN) and returns the old
/// action. The foundation for the signal family (#549/#550/#551). Per
/// #548.
pub const SYS_RT_SIGACTION: u64 = 13;

/// Linux x86_64 syscall number for `rt_sigprocmask(how, set, oldset,
/// sigsetsize)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_rt_sigprocmask`
/// (= 14). The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:15`. Routes to
/// `rt_sigprocmask::handle`, which blocks/unblocks/sets the thread's
/// signal mask (SIG_BLOCK/UNBLOCK/SETMASK) and returns the old mask.
/// Per #548.
pub const SYS_RT_SIGPROCMASK: u64 = 14;

/// Linux x86_64 syscall number for `rt_sigreturn()`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_rt_sigreturn`
/// (= 15). The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:16`. Routes to
/// `rt_sigreturn::handle`, which restores the saved context (signal
/// mask + — on the future #549+ delivery track — the interrupted
/// register frame) on return from a signal handler. Per #548.
pub const SYS_RT_SIGRETURN: u64 = 15;

/// Linux x86_64 syscall number for
/// `socket(int domain, int type, int protocol)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_socket` (= 41).
/// The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_socket`. Routes to
/// `socket::handle`, which creates a TCP socket (`AF_INET` +
/// `SOCK_STREAM`) on the kernel's smoltcp interface and allocates a
/// per-process fd bound to it — creation only, no I/O. Per #478a.
pub const SYS_SOCKET: u64 = 41;

/// Linux x86_64 syscall number for
/// `connect(int sockfd, const struct sockaddr *addr, socklen_t addrlen)`.
/// Source: `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_connect`
/// (= 42). The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_connect`. Routes to
/// `connect::handle`, which initiates the active-open TCP handshake to
/// the peer named by the `sockaddr_in`. Non-blocking: a started
/// handshake returns `-EINPROGRESS`. Per #531.
pub const SYS_CONNECT: u64 = 42;

/// Linux x86_64 syscall number for
/// `accept(int sockfd, struct sockaddr *addr, socklen_t *addrlen)`.
/// Source: `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_accept`
/// (= 43). The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_accept`. Routes to
/// `accept::handle`, which pulls the next completed inbound connection
/// off a listening socket and returns a new fd for it. Per #530.
pub const SYS_ACCEPT: u64 = 43;

/// Linux x86_64 syscall number for `sendto(int sockfd, const void *buf,
/// size_t len, int flags, const struct sockaddr *dest_addr, socklen_t
/// addrlen)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_sendto` (= 44). The
/// vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_sendto`. Routes to
/// `sendto::handle`, which enqueues the bytes on the socket's tx ring.
/// `send(2)` is this with a null `dest_addr`. Per #532.
pub const SYS_SENDTO: u64 = 44;

/// Linux x86_64 syscall number for `recvfrom(int sockfd, void *buf,
/// size_t len, int flags, struct sockaddr *src_addr, socklen_t
/// *addrlen)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_recvfrom` (= 45).
/// The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_recvfrom`. Routes to
/// `recvfrom::handle`, which dequeues bytes from the socket's rx ring (0
/// = EOF). `recv(2)` is this with a null `src_addr`. Per #532.
pub const SYS_RECVFROM: u64 = 45;

/// Linux x86_64 syscall number for
/// `bind(int sockfd, const struct sockaddr *addr, socklen_t addrlen)`.
/// Source: `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_bind`
/// (= 49). The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_bind`. Routes to
/// `bind::handle`, which records the socket's local endpoint (IPv4 addr
/// + port from the `sockaddr_in`) for a following `listen`. Per #529.
pub const SYS_BIND: u64 = 49;

/// Linux x86_64 syscall number for `listen(int sockfd, int backlog)`.
/// Source: `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_listen`
/// (= 50). The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_listen`. Routes to
/// `listen::handle`, which transitions the socket into the LISTEN state
/// on its bound port (the `backlog` is accepted but ignored — tier-1 has
/// an implicit backlog of one). Per #529.
pub const SYS_LISTEN: u64 = 50;

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
    let ret: i64 = match rax {
        SYS_READ => {
            // read(fd, buf, count) — keyboard ring → user buffer.
            // rdi = fd (must be 0 for stdin), rsi = buf pointer,
            // rdx = count (max bytes to fill). Non-blocking: returns 0
            // when the ring is empty, -EBADF for fd != 0. Per #508.
            read::handle(rdi, rsi, rdx)
        }
        SYS_WRITE => write::handle(rdi, rsi, rdx),
        SYS_MMAP => {
            // mmap(addr, len, prot, flags, fd, off) — anonymous mapping.
            // rdi=addr, rsi=len, rdx=prot, r10=flags, r8=fd, r9=off.
            // Fourth syscall arg is r10 (not rcx) per Linux x86_64 ABI
            // (`vendor/musl/arch/x86_64/syscall_arch.h:__syscall6`).
            // MAP_ANONYMOUS (flags & 0x20 != 0): bump-allocate a page-
            // aligned region; file-backed → -ENODEV. Per #497.
            mmap::handle_mmap(rdi, rsi, rdx, r10, r8, r9)
        }
        SYS_MUNMAP => {
            // munmap(addr, len) — rdi=addr, rsi=len.
            // Documented no-op in tier-1 (no per-mapping free list).
            // Returns 0 (success). Per #497.
            mmap::handle_munmap(rdi, rsi)
        }
        SYS_BRK => {
            // brk(addr) — heap-break management. `addr` = rdi.
            // Returns the resulting break (current or new) as a
            // non-negative i64 — never a negative errno per Linux
            // raw-syscall convention. Per #509.
            brk::handle(rdi)
        }
        SYS_CLOSE => close::handle(rdi as i32),
        SYS_STAT => {
            // stat(pathname, statbuf) — fill struct stat at statbuf.
            // rdi = pathname pointer (const char *), rsi = statbuf pointer.
            // Tier-1: no VFS — returns a char-device stub for any non-null
            // path. Returns -EFAULT for null pathname or statbuf. Per #500.
            stat::handle_stat(rdi, rsi)
        }
        SYS_FSTAT => {
            // fstat(fd, statbuf) — fill struct stat at statbuf.
            // rdi = fd (u64), rsi = statbuf pointer (struct stat *).
            // Known tier-1 fds (0/1/2) → char-device stub (S_IFCHR|0o666,
            // st_blksize=4096). Unknown fd → -EBADF. Null statbuf → -EFAULT.
            // Per #500 (file-state surface).
            stat::handle_fstat(rdi, rsi)
        }
        SYS_OPENAT => openat::handle(rdi as i32, rsi, rdx as u32, r10 as u32),
        SYS_FUTEX => {
            // futex(uaddr, futex_op, val, timeout, uaddr2, val3) per
            // `vendor/musl/arch/x86_64/syscall_arch.h:__syscall6`.
            // Tier-1 handles FUTEX_WAIT (block on value match) +
            // FUTEX_WAKE (release up to `val` waiters); #544 (Track
            // YYYYY) shipped WAIT, #545 ships WAKE, #546+ ship
            // REQUEUE / PI futex.
            futex::handle(rdi, rsi as u32, rdx as u32, r10, r8, r9 as u32)
        }
        SYS_SET_ROBUST_LIST => {
            // set_robust_list(head, len) — register the calling thread's
            // robust-futex list head. rdi = head (struct
            // robust_list_head *), rsi = len (size_t, must be 24). Per
            // #546.
            robust_list::set_robust_list(rdi, rsi)
        }
        SYS_GET_ROBUST_LIST => {
            // get_robust_list(pid, head_ptr, len_ptr) — report the
            // registered robust-list head + len. rdi = pid (0 = self),
            // rsi = head_ptr (struct robust_list_head **), rdx = len_ptr
            // (size_t *). Per #546.
            robust_list::get_robust_list(rdi, rsi, rdx)
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
        SYS_RT_SIGACTION => {
            // rt_sigaction(signum, act, oldact, sigsetsize) — install /
            // replace the per-process disposition for a signal. rdi =
            // signum (i32), rsi = act pointer (const k_sigaction *),
            // rdx = oldact pointer (k_sigaction *), r10 = sigsetsize
            // (must be 8). Fourth syscall arg is r10 (not rcx) per the
            // Linux x86_64 ABI. Returns the old action via oldact. Per
            // #548.
            rt_sigaction::handle(rdi as i32, rsi, rdx, r10)
        }
        SYS_RT_SIGPROCMASK => {
            // rt_sigprocmask(how, set, oldset, sigsetsize) — block /
            // unblock / set the thread signal mask. rdi = how
            // (SIG_BLOCK/UNBLOCK/SETMASK), rsi = set pointer
            // (const sigset_t *), rdx = oldset pointer (sigset_t *),
            // r10 = sigsetsize (must be 8). Per #548.
            rt_sigprocmask::handle(rdi as i32, rsi, rdx, r10)
        }
        SYS_RT_SIGRETURN => {
            // rt_sigreturn() — return from a signal handler, restoring
            // the saved context. Reads the rt_sigframe from the user
            // stack (rsp) on real Linux rather than from argument
            // registers; the tier-1 plumbing drives the per-process
            // saved-context slot instead. Args ignored. Per #548.
            rt_sigreturn::handle()
        }
        SYS_SOCKET => {
            // socket(domain, type, protocol) — create a TCP socket and
            // allocate a per-process fd bound to it. rdi = domain
            // (AF_INET), rsi = type (SOCK_STREAM), rdx = protocol
            // (IPPROTO_IP default | IPPROTO_TCP). Creation only — no
            // connect / bind / listen / I/O. Returns the fd (≥ 3) or a
            // negative errno (-EAFNOSUPPORT / -EPROTONOSUPPORT / -EINVAL
            // / -EMFILE / -ENOSYS). Per #478a.
            socket::handle(rdi, rsi, rdx)
        }
        SYS_CONNECT => {
            // connect(sockfd, addr, addrlen) — active-open TCP handshake
            // to the peer named by the sockaddr_in. rdi = sockfd (i32),
            // rsi = addr pointer (const struct sockaddr *), rdx = addrlen
            // (socklen_t). Non-blocking: a started handshake returns
            // -EINPROGRESS; other outcomes -EBADF / -EFAULT / -EINVAL /
            // -ENOTSOCK / -EAFNOSUPPORT / -EISCONN / -ENOSYS. Per #531.
            connect::handle(rdi as i32, rsi, rdx)
        }
        SYS_ACCEPT => {
            // accept(sockfd, addr, addrlen) — pull the next completed
            // connection off a listening socket. rdi = sockfd (i32), rsi
            // = addr pointer (struct sockaddr *, may be NULL), rdx =
            // addrlen pointer (socklen_t *). Returns the new connected fd
            // (≥ 3) or -EAGAIN (nothing pending) / -EBADF / -EINVAL (not
            // listening) / -ENOTSOCK / -EOPNOTSUPP (UDP) / -EMFILE /
            // -ENOSYS. Per #530.
            accept::handle(rdi as i32, rsi, rdx)
        }
        SYS_SENDTO => {
            // sendto(sockfd, buf, len, flags, dest_addr, addrlen) — send
            // bytes on a (connected) socket. rdi = sockfd (i32), rsi =
            // buf, rdx = len, r10 = flags, r8 = dest_addr, r9 = addrlen.
            // `send(2)` is this with dest_addr = 0. Fourth+ args are r10
            // / r8 / r9 per the Linux x86_64 ABI. Returns the byte count
            // (possibly short) or -EBADF / -EFAULT / -ENOTSOCK / -EISCONN
            // / -ENOTCONN / -EAGAIN / -ENOSYS. Per #532.
            sendto::handle(rdi as i32, rsi, rdx, r10 as u32, r8, r9)
        }
        SYS_RECVFROM => {
            // recvfrom(sockfd, buf, len, flags, src_addr, addrlen) —
            // receive bytes from a (connected) socket. rdi = sockfd
            // (i32), rsi = buf, rdx = len, r10 = flags, r8 = src_addr,
            // r9 = addrlen pointer. `recv(2)` is this with src_addr = 0.
            // Returns the byte count (0 = EOF) or -EBADF / -EFAULT /
            // -ENOTSOCK / -ENOTCONN / -EAGAIN / -ENOSYS. Per #532.
            recvfrom::handle(rdi as i32, rsi, rdx, r10 as u32, r8, r9)
        }
        SYS_BIND => {
            // bind(sockfd, addr, addrlen) — assign a local address to a
            // socket. rdi = sockfd (i32), rsi = addr pointer (const
            // struct sockaddr *), rdx = addrlen (socklen_t). Records the
            // IPv4 addr + port for a following listen. Returns 0 or a
            // negative errno (-EBADF / -EFAULT / -EINVAL / -ENOTSOCK /
            // -EAFNOSUPPORT / -EADDRINUSE / -ENOSYS). Per #529.
            bind::handle(rdi as i32, rsi, rdx)
        }
        SYS_LISTEN => {
            // listen(sockfd, backlog) — mark a bound socket passive.
            // rdi = sockfd (i32), rsi = backlog (i32, accepted but
            // ignored — tier-1 backlog is one). Returns 0 or a negative
            // errno (-EBADF / -EINVAL / -ENOTSOCK / -EADDRINUSE /
            // -ENOSYS). Per #529.
            listen::handle(rdi as i32, rsi as i32)
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
    };
    // #527 bring-up trace: one serial line per syscall. The guest's
    // own fd-1/2 writes also land on serial, so the interleaving
    // reads as a primitive strace. Bounded noise: tier-1 runs one
    // short-lived process at a time. Gated UEFI-only — host tests
    // call dispatch() in tight loops and don't want stdout spam.
    #[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
    crate::println!(
        "  sys:      #{rax}({rdi:#x}, {rsi:#x}, {rdx:#x}) = {ret:#x}"
    );
    ret
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

    /// `SYS_STAT` is 4 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_stat`.
    #[test]
    fn sys_stat_number_matches_linux_uapi() {
        assert_eq!(SYS_STAT, 4);
    }

    /// `SYS_FSTAT` is 5 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_fstat`.
    #[test]
    fn sys_fstat_number_matches_linux_uapi() {
        assert_eq!(SYS_FSTAT, 5);
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

    /// `SYS_SET_ROBUST_LIST` is 273 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_set_robust_list`
    /// (and `vendor/musl/arch/x86_64/bits/syscall.h.in:274`).
    #[test]
    fn sys_set_robust_list_number_matches_linux_uapi() {
        assert_eq!(SYS_SET_ROBUST_LIST, 273);
    }

    /// `SYS_GET_ROBUST_LIST` is 274 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_get_robust_list`
    /// (and `vendor/musl/arch/x86_64/bits/syscall.h.in:275`).
    #[test]
    fn sys_get_robust_list_number_matches_linux_uapi() {
        assert_eq!(SYS_GET_ROBUST_LIST, 274);
    }

    /// `set_robust_list(head, 8)` — a wrong length — routes through the
    /// dispatcher (syscall 273) to the handler and the length check
    /// fires → `-EINVAL` (-22), without needing a process installed
    /// (the length guard precedes the process touch).
    #[test]
    fn dispatch_set_robust_list_wrong_len_returns_einval() {
        let result = dispatch(SYS_SET_ROBUST_LIST, 0xdead_0000, 8, 0, 0, 0, 0);
        assert_eq!(result, -EINVAL);
    }

    /// `get_robust_list(0, NULL, NULL)` routes through the dispatcher
    /// (syscall 274) and the null-out-pointer guard fires → `-EFAULT`
    /// (-14).
    #[test]
    fn dispatch_get_robust_list_null_out_returns_efault() {
        let result = dispatch(SYS_GET_ROBUST_LIST, 0, 0, 0, 0, 0, 0);
        assert_eq!(result, -14);
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

    /// `SYS_SOCKET` is 41 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_socket`.
    #[test]
    fn sys_socket_number_matches_linux_uapi() {
        assert_eq!(SYS_SOCKET, 41);
    }

    /// `dispatch(SYS_SOCKET, AF_INET6, SOCK_STREAM, 0, ...)` routes to
    /// `socket::handle` and the address-family rejection fires →
    /// `-EAFNOSUPPORT` (-97). Verifies the dispatcher wires syscall 41
    /// to the socket handler. (AF_INET6 = 10, SOCK_STREAM = 1.) The
    /// rejection happens before socket creation, so no process / net
    /// stack is needed.
    #[test]
    fn dispatch_socket_af_inet6_returns_eafnosupport() {
        // domain = AF_INET6 (10), type = SOCK_STREAM (1), protocol = 0.
        let result = dispatch(SYS_SOCKET, 10, 1, 0, 0, 0, 0);
        assert_eq!(result, -97);
    }

    /// `dispatch(SYS_SOCKET, AF_INET, SOCK_DGRAM, IPPROTO_TCP, ...)` → -93
    /// (-EPROTONOSUPPORT). A datagram socket with a non-UDP protocol is
    /// rejected (#533 — SOCK_DGRAM itself is now accepted, but only with
    /// IPPROTO_IP / IPPROTO_UDP). Confirms the dispatcher routes 41 and
    /// the protocol-mismatch rejection fires before any creation (so no
    /// process / stack is needed). domain=AF_INET(2), type=SOCK_DGRAM(2),
    /// protocol=IPPROTO_TCP(6).
    #[test]
    fn dispatch_socket_dgram_wrong_protocol_returns_eprotonosupport() {
        let result = dispatch(SYS_SOCKET, 2, 2, 6, 0, 0, 0);
        assert_eq!(result, -93);
    }

    /// `SYS_BIND` is 49 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_bind`.
    #[test]
    fn sys_bind_number_matches_linux_uapi() {
        assert_eq!(SYS_BIND, 49);
    }

    /// `SYS_LISTEN` is 50 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_listen`.
    #[test]
    fn sys_listen_number_matches_linux_uapi() {
        assert_eq!(SYS_LISTEN, 50);
    }

    /// `dispatch(SYS_BIND, fd, NULL, 16, ...)` routes to `bind::handle`
    /// and the null-sockaddr rejection fires → `-EFAULT` (-14). Verifies
    /// the dispatcher wires syscall 49 to the bind handler. The rejection
    /// happens before any fd / net touch, so no process / stack is
    /// needed.
    #[test]
    fn dispatch_bind_null_addr_returns_efault() {
        // sockfd = 3, addr = NULL, addrlen = 16.
        let result = dispatch(SYS_BIND, 3, 0, 16, 0, 0, 0);
        assert_eq!(result, -14);
    }

    /// `SYS_CONNECT` is 42 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_connect`.
    #[test]
    fn sys_connect_number_matches_linux_uapi() {
        assert_eq!(SYS_CONNECT, 42);
    }

    /// `dispatch(SYS_CONNECT, fd, NULL, 16, ...)` routes to
    /// `connect::handle` and the null-sockaddr rejection fires →
    /// `-EFAULT` (-14). Verifies the dispatcher wires syscall 42. The
    /// rejection precedes any fd / net touch.
    #[test]
    fn dispatch_connect_null_addr_returns_efault() {
        let result = dispatch(SYS_CONNECT, 3, 0, 16, 0, 0, 0);
        assert_eq!(result, -14);
    }

    /// `SYS_ACCEPT` is 43 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_accept`.
    /// (Dispatch *routing* for accept is covered by the
    /// `syscall::accept` handler tests, which take the
    /// `CURRENT_PROCESS_TEST_LOCK` — a bare `dispatch(SYS_ACCEPT, ...)`
    /// here would observe a sibling test's installed process and flake,
    /// since accept resolves the fd before any process-independent
    /// check, the same reason the signal-syscall routing tests live in
    /// their handler modules.)
    #[test]
    fn sys_accept_number_matches_linux_uapi() {
        assert_eq!(SYS_ACCEPT, 43);
    }

    /// `SYS_SENDTO` is 44 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_sendto`.
    #[test]
    fn sys_sendto_number_matches_linux_uapi() {
        assert_eq!(SYS_SENDTO, 44);
    }

    /// `SYS_RECVFROM` is 45 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_recvfrom`.
    #[test]
    fn sys_recvfrom_number_matches_linux_uapi() {
        assert_eq!(SYS_RECVFROM, 45);
    }

    /// `dispatch(SYS_SENDTO, fd, buf, 0, ...)` routes to `sendto::handle`;
    /// a zero-length send is a POSIX no-op returning 0. Verifies the
    /// dispatcher wires syscall 44 (and threads the 6 args), with no
    /// process / stack needed (the count==0 short-circuit fires first).
    #[test]
    fn dispatch_sendto_zero_len_returns_zero() {
        // sockfd=3, buf=0, len=0, flags=0, dest_addr=0, addrlen=0.
        let result = dispatch(SYS_SENDTO, 3, 0, 0, 0, 0, 0);
        assert_eq!(result, 0);
    }

    /// `dispatch(SYS_SENDTO, fd, NULL, n>0, ...)` — a null buf with
    /// non-zero len → `-EFAULT` (-14). Verifies the dispatcher routes
    /// syscall 44 to the sendto handler (the buffer check fires before fd
    /// resolution, so no process is needed). The 6-arg threading itself
    /// is also covered by the zero-len no-op test above.
    #[test]
    fn dispatch_sendto_null_buf_returns_efault() {
        let result = dispatch(SYS_SENDTO, 3, 0, 16, 0, 0, 0);
        assert_eq!(result, -14);
    }

    /// `dispatch(SYS_RECVFROM, fd, NULL, n>0, ...)` routes to
    /// `recvfrom::handle`; a null buf with non-zero len → `-EFAULT`
    /// (-14). Verifies the dispatcher wires syscall 45.
    #[test]
    fn dispatch_recvfrom_null_buf_returns_efault() {
        let result = dispatch(SYS_RECVFROM, 3, 0, 16, 0, 0, 0);
        assert_eq!(result, -14);
    }

    /// `SYS_MMAP` is 9 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_mmap`.
    #[test]
    fn sys_mmap_number_matches_linux_uapi() {
        assert_eq!(SYS_MMAP, 9);
    }

    /// `SYS_MUNMAP` is 11 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_munmap`.
    #[test]
    fn sys_munmap_number_matches_linux_uapi() {
        assert_eq!(SYS_MUNMAP, 11);
    }

    /// `SYS_RT_SIGACTION` is 13 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_rt_sigaction`
    /// (and `vendor/musl/arch/x86_64/bits/syscall.h.in:14`).
    #[test]
    fn sys_rt_sigaction_number_matches_linux_uapi() {
        assert_eq!(SYS_RT_SIGACTION, 13);
    }

    /// `SYS_RT_SIGPROCMASK` is 14 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_rt_sigprocmask`
    /// (and `vendor/musl/arch/x86_64/bits/syscall.h.in:15`).
    #[test]
    fn sys_rt_sigprocmask_number_matches_linux_uapi() {
        assert_eq!(SYS_RT_SIGPROCMASK, 14);
    }

    /// `SYS_RT_SIGRETURN` is 15 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_rt_sigreturn`
    /// (and `vendor/musl/arch/x86_64/bits/syscall.h.in:16`).
    ///
    /// (Dispatch *routing* for the three signal syscalls is covered by
    /// the `dispatch_routes_*` tests in each handler module, which take
    /// the `CURRENT_PROCESS_TEST_LOCK` so they don't race the shared
    /// process singleton — a bare `dispatch(...)` here would observe a
    /// sibling test's installed process and flake.)
    #[test]
    fn sys_rt_sigreturn_number_matches_linux_uapi() {
        assert_eq!(SYS_RT_SIGRETURN, 15);
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

    /// `futex(uaddr, FUTEX_WAKE, n, ...)` on an empty queue routes
    /// through the dispatcher to the WAKE handler (#545) and returns 0
    /// — no waiters parked means zero woken. op = FUTEX_WAKE (1),
    /// uaddr = a valid aligned non-null address, n = 1. Verifies the
    /// dispatcher wires the WAKE op (not just WAIT) to futex::handle.
    #[test]
    fn dispatch_futex_wake_empty_queue_returns_zero() {
        // uaddr 0x4040 is non-null + 4-byte aligned; nothing is parked
        // there, so the wake count is 0.
        let result = dispatch(SYS_FUTEX, 0x4040, 1, 1, 0, 0, 0);
        assert_eq!(result, 0);
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

    /// `write(2, ...)` (stderr) now routes to the kernel serial console
    /// (same as stdout in tier-1). With zero count it returns 0 — the
    /// POSIX no-op short circuit fires before the fd check matters.
    /// Verifies the dispatcher routes SYS_WRITE (1) and that fd=2 is
    /// accepted after the #500 stderr-routing addition to write::handle.
    #[test]
    fn dispatch_write_to_stderr_returns_zero_for_zero_count() {
        // fd 2 (stderr), null buf, zero count — count=0 is a POSIX no-op
        // and returns 0 (the zero-count short-circuit fires before the
        // fd check; then fd=2 is accepted by the updated handle()).
        let result = dispatch(SYS_WRITE, 2, 0, 0, 0, 0, 0);
        assert_eq!(result, 0);
    }

    /// `write(5, ...)` (fd 5 — not a valid tier-1 fd) returns `-EBADF`.
    /// Verifies the dispatcher routes to the write handler and the
    /// fd-validation arm fires for fds beyond 0/1/2.
    #[test]
    fn dispatch_write_to_unknown_fd_returns_ebadf() {
        let payload = b"unused";
        let result = dispatch(SYS_WRITE, 5, payload.as_ptr() as u64, payload.len() as u64, 0, 0, 0);
        assert_eq!(result, -EBADF);
    }
}
