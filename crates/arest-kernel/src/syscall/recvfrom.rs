// crates/arest-kernel/src/syscall/recvfrom.rs
//
// Linux x86_64 syscall 45: `recvfrom(int sockfd, void *buf, size_t len,
// int flags, struct sockaddr *src_addr, socklen_t *addrlen)`. Per #532
// (the send/recv leg of the userspace TCP socket cluster). `recv(2)` is
// `recvfrom` with a null `src_addr` (libc's `recv` is exactly that), so
// this one handler serves both.
//
// Linux x86_64 number: `__NR_recvfrom = 45`
// (`linux/arch/x86/include/uapi/asm/unistd_64.h`; the vendored musl tree
// confirms the same value at
// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_recvfrom`).
//
// How the pieces fit (the #478a host-testable split, continued)
// -------------------------------------------------------------
// `recvfrom` carries no inbound address to decode, so it has no pure
// sockaddr-parse step. Its host-testable surface is the fd→SocketId
// resolution + the SocketError→errno mapping (shared `syscall::socket`
// helpers). The smoltcp side — dequeueing bytes from the rx ring — is
// `net::tcp_recv` (gated wrapper). This handler is the glue.
//
// EOF vs would-block (the two zero-ish outcomes)
// ----------------------------------------------
// `recv` has two distinct "no payload" results POSIX keeps separate:
//   * Return 0  — END OF STREAM: the peer closed its send half and all
//     buffered bytes were delivered. `net::tcp_recv` returns `Ok(0)` for
//     this (smoltcp's `RecvError::Finished`); the handler returns 0.
//   * `-EAGAIN` — the connection is open but no bytes are pending right
//     now. `net::tcp_recv` returns `Err(WouldBlock)`; the handler maps
//     it to `-EAGAIN`. Tier-1 sockets are non-blocking (no scheduler to
//     park on, #530), so a recv with an empty rx ring returns `-EAGAIN`
//     rather than blocking; the caller retries after a `net::poll()`.
//
// src_addr / addrlen handling for tier-1
// --------------------------------------
// On a connection-mode (TCP) socket the source of every byte is the one
// connected peer, so `src_addr` is rarely used (libc programs typically
// pass NULL). Tier-1 does NOT report the peer address: when `addrlen` is
// a non-null pointer we write 0 to it (meaning "no address returned"),
// matching the kernel contract that `*addrlen` is updated to the length
// actually stored. Reporting the concrete peer endpoint needs the gated
// `remote_endpoint` plumbing and is a follow-up; the null-`src_addr`
// recv form — the primary contract of this slice — is fully served.
//
// Buffer / pointer model (same as `read`)
// ---------------------------------------
// `buf` is a userspace virtual address; under tier-1's identity mapping
// it doubles as a kernel pointer (see `syscall::read`'s header for the
// rationale + the #527/#561 follow-up). A zero-length recv returns 0
// (POSIX no-op) without touching `buf`; a null `buf` with `len > 0` is
// `-EFAULT`; an oversized `len` is `-EFAULT`.
//
// `flags` (`MSG_DONTWAIT`, `MSG_PEEK`, `MSG_WAITALL`, …) are accepted but
// ignored — the same tier-1 stance `sendto` takes (non-blocking is
// implicit; `MSG_PEEK` / `MSG_WAITALL` are unmodelled).
//
// Return value
// ------------
// Linux `recvfrom(2)` returns the number of bytes received (0 = orderly
// shutdown / EOF) on success, or `-errno`:
//   * `-EBADF` (9) — `sockfd` isn't an open fd.
//   * `-EFAULT` (14) — `buf` is a null / bad pointer (with `len > 0`).
//   * `-ENOTSOCK` (88) — `sockfd` is open but not a socket.
//   * `-ENOTCONN` (107) — the socket has no established connection.
//   * `-EAGAIN` (11) — no bytes pending on the (open) connection.
//   * `-ENOSYS` (38) — no process is installed, or the net stack isn't up.

use core::slice;

use crate::net;
use crate::syscall::dispatch::EFAULT;
use crate::syscall::socket::{resolve_socket_fd, socket_error_to_errno, SocketOp};

/// Linux x86_64 syscall number for `recvfrom(sockfd, buf, len, flags,
/// src_addr, addrlen)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_recvfrom` (= 45).
/// The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_recvfrom`. Routes to
/// `recvfrom::handle`, which dequeues bytes from the socket's rx ring.
/// `recv(2)` is this with a null `src_addr`. Per #532.
pub const SYS_RECVFROM: u64 = 45;

/// Handle a `recvfrom(sockfd, buf, len, flags, src_addr, addrlen)`
/// syscall (also the `recv(2)` path, with `src_addr == 0`). Returns the
/// number of bytes received (0 = EOF / orderly shutdown) or a negative
/// errno (see the module docstring for the full table).
///
/// Steps, in order:
///   1. `len == 0` → return 0 (POSIX no-op), without touching `buf`.
///   2. `buf == 0` (with `len > 0`) → `-EFAULT`; oversized `len` →
///      `-EFAULT`.
///   3. Resolve `sockfd` to its `SocketId`. Unknown → `-EBADF`;
///      non-socket → `-ENOTSOCK`; no process → `-ENOSYS`.
///   4. Dequeue bytes via `net::tcp_recv` into the caller's buffer; map
///      the result (`Ok(0)` = EOF → 0, `WouldBlock` → `-EAGAIN`,
///      `NotConnected` → `-ENOTCONN`).
///   5. On a successful receive, update `*addrlen` to 0 if `addrlen` is
///      non-null (tier-1 doesn't report the peer address).
///
/// SAFETY: `buf` is treated as a kernel pointer under tier-1's identity
/// mapping (same model as `read`). The null + oversized-`len` guards keep
/// the `from_raw_parts_mut` span valid; tier-1's identity mapping makes a
/// non-null in-range address valid kernel memory. `addrlen` is written
/// only after a non-null check.
pub fn handle(
    sockfd: i32,
    buf: u64,
    len: u64,
    _flags: u32,
    _src_addr: u64,
    addrlen: u64,
) -> i64 {
    // (1) Zero-length recv is a POSIX no-op — return 0 without deref.
    if len == 0 {
        return 0;
    }

    // (2) Validate the buffer (same guards as `read`).
    if buf == 0 {
        return -EFAULT;
    }
    if len > isize::MAX as u64 {
        return -EFAULT;
    }

    // (3) Resolve the fd to a socket id.
    let socket_id = match resolve_socket_fd(sockfd) {
        Ok(id) => id,
        Err(errno) => return errno,
    };

    // Form a mutable byte slice over the caller's buffer.
    // SAFETY: `buf` is non-null (checked) and `len <= isize::MAX`
    // (checked); tier-1's identity mapping makes the span valid kernel
    // memory. `tcp_recv` writes only up to `len` bytes.
    let data: &mut [u8] = unsafe { slice::from_raw_parts_mut(buf as *mut u8, len as usize) };

    // (4) Dequeue; map any failure to its errno. `Ok(0)` is EOF.
    let received = match net::tcp_recv(socket_id, data) {
        Ok(n) => n as i64,
        Err(e) => return socket_error_to_errno(e, SocketOp::Recv),
    };

    // (5) Report "no source address available" if the caller asked for
    //     one. tier-1 doesn't fill `src_addr`; the kernel contract is to
    //     update `*addrlen` to the length stored (0 here).
    if addrlen != 0 {
        // SAFETY: `addrlen` is non-null; tier-1's identity mapping makes
        // it valid kernel memory. `socklen_t` is a 32-bit unsigned int on
        // Linux x86_64.
        unsafe {
            core::ptr::write(addrlen as *mut u32, 0);
        }
    }

    received
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::address_space::AddressSpace;
    use crate::process::fd_table::{socket as fd_socket, synthetic as fd_synthetic};
    use crate::process::process::CURRENT_PROCESS_TEST_LOCK;
    use crate::process::{
        current_process_fd_table, current_process_install, current_process_uninstall, Process,
    };

    /// `SYS_RECVFROM` is 45 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_recvfrom`.
    #[test]
    fn sys_recvfrom_number_matches_linux_uapi() {
        assert_eq!(SYS_RECVFROM, 45);
    }

    fn install_test_process() {
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(7, address_space);
        current_process_install(proc);
    }

    // -- Edge cases / argument validation ------------------------------

    /// `recvfrom(fd, buf, 0, ...)` — zero length is a POSIX no-op,
    /// returns 0 even with a null buf and no process installed.
    #[test]
    fn recvfrom_zero_len_returns_zero() {
        assert_eq!(handle(3, 0, 0, 0, 0, 0), 0);
    }

    /// `recv(fd, NULL, n>0, ...)` — null buf with non-zero len →
    /// `-EFAULT`.
    #[test]
    fn recvfrom_null_buf_returns_efault() {
        assert_eq!(handle(3, 0, 16, 0, 0, 0), -14);
    }

    // -- fd-resolution errnos ------------------------------------------

    /// `recv` with no process installed → `-ENOSYS`.
    #[test]
    fn recvfrom_no_process_returns_enosys() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        current_process_uninstall();
        let mut buf = [0u8; 16];
        let result = handle(3, buf.as_mut_ptr() as u64, buf.len() as u64, 0, 0, 0);
        assert_eq!(result, -38);
    }

    /// `recv` of an unopened fd → `-EBADF`.
    #[test]
    fn recvfrom_unknown_fd_returns_ebadf() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        install_test_process();
        let mut buf = [0u8; 16];
        let result = handle(3, buf.as_mut_ptr() as u64, buf.len() as u64, 0, 0, 0);
        assert_eq!(result, -9);
        current_process_uninstall();
    }

    /// `recv` of a non-socket fd → `-ENOTSOCK`.
    #[test]
    fn recvfrom_non_socket_fd_returns_enotsock() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        install_test_process();
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_synthetic("/proc/cpuinfo")).expect("alloc")
        });
        let mut buf = [0u8; 16];
        let result = handle(fd, buf.as_mut_ptr() as u64, buf.len() as u64, 0, 0, 0);
        assert_eq!(result, -88);
        current_process_uninstall();
    }

    // -- Socket-fd paths over the loopback net stack -------------------

    /// Serialises recvfrom tests that stand up `net`.
    static RECVFROM_NET_TEST_LOCK: spin::Mutex<()> = spin::Mutex::new(());

    /// `recv` on a freshly-created (UNconnected) socket → `-ENOTCONN`.
    /// The receive half isn't open (never connected), so smoltcp's recv
    /// fails with InvalidState, mapped to NotConnected.
    #[test]
    fn recvfrom_unconnected_socket_returns_enotconn() {
        let _net_guard = RECVFROM_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None);
        install_test_process();
        let id = net::create_tcp_socket().expect("create socket");
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_socket(id.as_u64())).expect("alloc fd")
        });
        let mut buf = [0u8; 16];
        let result = handle(fd, buf.as_mut_ptr() as u64, buf.len() as u64, 0, 0, 0);
        assert_eq!(result, -107, "recv on an unconnected socket must be -ENOTCONN");
        current_process_uninstall();
        net::destroy_socket(id);
    }

    /// `recvfrom` with a non-null `addrlen` on an unconnected socket
    /// still returns the error (`-ENOTCONN`) WITHOUT writing `*addrlen`
    /// — the addrlen update only happens on a successful receive (step 5
    /// runs after the recv succeeds). Pins that the error path doesn't
    /// touch the addrlen out-pointer.
    #[test]
    fn recvfrom_error_does_not_write_addrlen() {
        let _net_guard = RECVFROM_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None);
        install_test_process();
        let id = net::create_tcp_socket().expect("create socket");
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_socket(id.as_u64())).expect("alloc fd")
        });
        let mut buf = [0u8; 16];
        let mut addrlen: u32 = 0xDEAD_BEEF; // sentinel — must stay untouched
        let result = handle(
            fd,
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
            0,
            0,
            (&mut addrlen as *mut u32) as u64,
        );
        assert_eq!(result, -107);
        assert_eq!(addrlen, 0xDEAD_BEEF, "addrlen must be untouched on the error path");
        current_process_uninstall();
        net::destroy_socket(id);
    }

    /// A socket fd with a dangling id (`u64::MAX`) recvs to `-EBADF` even
    /// with the net stack up.
    #[test]
    fn recvfrom_dangling_socket_id_returns_ebadf() {
        let _net_guard = RECVFROM_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None);
        install_test_process();
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_socket(u64::MAX)).expect("alloc fd")
        });
        let mut buf = [0u8; 16];
        let result = handle(fd, buf.as_mut_ptr() as u64, buf.len() as u64, 0, 0, 0);
        assert_eq!(result, -9);
        current_process_uninstall();
    }
}
