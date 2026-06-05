// crates/arest-kernel/src/syscall/sendto.rs
//
// Linux x86_64 syscall 44: `sendto(int sockfd, const void *buf, size_t
// len, int flags, const struct sockaddr *dest_addr, socklen_t addrlen)`.
// Per #532 (the send/recv leg of the userspace TCP socket cluster).
// `send(2)` is `sendto` with a null `dest_addr` (libc's `send` is exactly
// that), so this one handler serves both — the connection-mode (TCP)
// path requires the null-address form.
//
// Linux x86_64 number: `__NR_sendto = 44`
// (`linux/arch/x86/include/uapi/asm/unistd_64.h`; the vendored musl tree
// confirms the same value at
// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_sendto`).
//
// How the pieces fit (the #478a host-testable split, continued)
// -------------------------------------------------------------
//   * Pure decision — the connection-mode rule that a TCP `sendto` must
//     NOT name a per-call destination — is
//     `net_socket::validate_stream_sendto_dest` (host-unit-tested:
//     non-null dest pointer → `-EISCONN`). The fd→SocketId resolution +
//     the SocketError→errno mapping are the shared `syscall::socket`
//     helpers.
//   * The smoltcp side — enqueueing the bytes on the tx ring — is
//     `net::tcp_send` (gated wrapper; runs over the host loopback build,
//     `NotInitialised` before `net::init`).
//   * This handler is the glue: validate the dest, resolve the fd, form
//     the byte slice from userspace, call `tcp_send`, map the result.
//
// Buffer / pointer model (same as `write`)
// ----------------------------------------
// `buf` is a userspace virtual address; under tier-1's identity mapping
// it doubles as a kernel pointer (see `syscall::write`'s module header
// for the full rationale + the #527/#561 page-table follow-up). A
// zero-length `send` is a POSIX no-op returning 0 without dereferencing
// `buf`; a null `buf` with `len > 0` is `-EFAULT`. A `len` beyond
// `isize::MAX` is rejected (malformed call) before any slice is formed.
//
// `flags` handling for tier-1
// ---------------------------
// The `flags` argument (`MSG_DONTWAIT`, `MSG_NOSIGNAL`, `MSG_MORE`, …) is
// accepted but ignored. Tier-1 sockets are inherently non-blocking
// (there's no scheduler to park on, #530), so `MSG_DONTWAIT` is already
// the implicit behaviour; `MSG_NOSIGNAL` is moot (no `SIGPIPE` delivery
// surface yet); `MSG_OOB` / `MSG_MORE` are unmodelled. A future slice can
// honour them once the relevant machinery exists.
//
// Return value
// ------------
// Linux `sendto(2)` returns the number of bytes accepted (≥ 0; may be a
// short write) on success, or `-errno`:
//   * `-EBADF` (9) — `sockfd` isn't an open fd.
//   * `-EFAULT` (14) — `buf` is a null / bad pointer (with `len > 0`).
//   * `-ENOTSOCK` (88) — `sockfd` is open but not a socket.
//   * `-EISCONN` (106) — a non-null `dest_addr` on a connected stream
//                      socket.
//   * `-ENOTCONN` (107) — the socket has no established connection.
//   * `-EAGAIN` (11) — the tx ring is full; retry after a poll.
//   * `-ENOSYS` (38) — no process is installed, or the net stack isn't up.

use core::slice;

use crate::net;
use crate::net_socket::{
    parse_sockaddr_in, validate_dgram_sendto_dest, validate_stream_sendto_dest,
};
use crate::syscall::dispatch::{EBADF, ENOSYS, EFAULT};
use crate::syscall::socket::{
    read_sockaddr, resolve_socket_fd, socket_error_to_errno, SocketOp,
};

/// Linux x86_64 syscall number for `sendto(sockfd, buf, len, flags,
/// dest_addr, addrlen)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_sendto` (= 44). The
/// vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_sendto`. Routes to
/// `sendto::handle`, which enqueues the bytes on the socket's tx ring.
/// `send(2)` is this with a null `dest_addr`. Per #532.
pub const SYS_SENDTO: u64 = 44;

/// Handle a `sendto(sockfd, buf, len, flags, dest_addr, addrlen)`
/// syscall (also the `send(2)` path, with `dest_addr == 0`). Returns the
/// number of bytes enqueued (a possibly-short non-negative count) or a
/// negative errno (see the module docstring for the full table).
///
/// Steps, in order:
///   1. `len == 0` → return 0 (POSIX no-op), without touching `buf`.
///   2. `buf == 0` (with `len > 0`) → `-EFAULT`; oversized `len` →
///      `-EFAULT`.
///   3. Resolve `sockfd` to its `SocketId`. Unknown → `-EBADF`;
///      non-socket → `-ENOTSOCK`; no process → `-ENOSYS`.
///   4. Route on the socket's transport kind (#533):
///        * TCP — a non-null `dest_addr` is `-EISCONN` (connection-mode
///          sockets don't name a per-call peer; the null-addr form is
///          plain `send`); then enqueue via `net::tcp_send`.
///        * UDP — a null `dest_addr` is `-EDESTADDRREQ` (a datagram needs
///          a destination); parse the `sockaddr_in` and enqueue via
///          `net::udp_sendto`.
///      Either path maps `NotConnected` → `-ENOTCONN`, `WouldBlock` →
///      `-EAGAIN`.
///
/// NOTE the dest-address check moved AFTER fd resolution (vs the #532
/// TCP-only handler): the rule is opposite for TCP vs UDP, so the socket
/// kind must be known first. A `sendto` with a bad dest on a non-existent
/// fd therefore now surfaces the fd error (`-EBADF`/`-ENOSYS`) rather
/// than the dest error — matching Linux, which validates the fd first.
///
/// SAFETY: `buf` is treated as a kernel pointer under tier-1's identity
/// mapping (same model as `write`). The null + oversized-`len` guards
/// keep the `from_raw_parts` span valid; tier-1's identity mapping makes
/// a non-null in-range address valid kernel memory.
pub fn handle(
    sockfd: i32,
    buf: u64,
    len: u64,
    _flags: u32,
    dest_addr: u64,
    addrlen: u64,
) -> i64 {
    // (1) Zero-length send is a POSIX no-op — return 0 without deref.
    if len == 0 {
        return 0;
    }

    // (2) Validate the buffer. Null with len > 0 → EFAULT; an oversized
    //     len would overflow the slice constructor → EFAULT (same guard
    //     `write::do_write` uses).
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

    // Form the byte slice from userspace.
    // SAFETY: `buf` is non-null (checked) and `len <= isize::MAX`
    // (checked); tier-1's identity mapping makes the span valid kernel
    // memory.
    let data: &[u8] = unsafe { slice::from_raw_parts(buf as *const u8, len as usize) };

    // (4) Route on the socket's transport kind (#533). The destination-
    //     address rule is OPPOSITE for the two: a TCP (connection-mode)
    //     sendto must NOT name a destination (the null-addr `send` form,
    //     else -EISCONN); a UDP (connectionless) sendto MUST name one
    //     (else -EDESTADDRREQ), which is then parsed and used per packet.
    match net::socket_kind(socket_id) {
        Some(net::SocketKind::Tcp) => {
            // Connection-mode rule (pure, host-tested), then enqueue.
            if let Err(errno) = validate_stream_sendto_dest(dest_addr) {
                return errno;
            }
            match net::tcp_send(socket_id, data) {
                Ok(n) => n as i64,
                Err(e) => socket_error_to_errno(e, SocketOp::Send),
            }
        }
        Some(net::SocketKind::Udp) => {
            // Connectionless rule: a destination is required.
            if let Err(errno) = validate_dgram_sendto_dest(dest_addr) {
                return errno;
            }
            // Parse the per-packet destination sockaddr_in.
            let dest_bytes = match read_sockaddr(dest_addr, addrlen) {
                Ok(b) => b,
                Err(errno) => return errno,
            };
            let dest = match parse_sockaddr_in(&dest_bytes) {
                Ok(sa) => sa,
                Err(errno) => return errno,
            };
            match net::udp_sendto(socket_id, dest.addr, dest.port, data) {
                Ok(n) => n as i64,
                Err(e) => socket_error_to_errno(e, SocketOp::Send),
            }
        }
        // No kind → dangling socket fd (net up) or stack down.
        None => {
            if net::is_online() {
                -EBADF
            } else {
                -ENOSYS
            }
        }
    }
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

    /// `SYS_SENDTO` is 44 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_sendto`.
    #[test]
    fn sys_sendto_number_matches_linux_uapi() {
        assert_eq!(SYS_SENDTO, 44);
    }

    fn install_test_process() {
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(7, address_space);
        current_process_install(proc);
    }

    // -- Edge cases / argument validation (no process / stack needed) --

    /// `sendto(fd, buf, 0, 0, NULL, 0)` — zero length is a POSIX no-op,
    /// returns 0 even with a null buf and no process installed.
    #[test]
    fn sendto_zero_len_returns_zero() {
        assert_eq!(handle(3, 0, 0, 0, 0, 0), 0);
    }

    /// `send(fd, NULL, n>0, 0)` with a null buf → `-EFAULT`. The buffer
    /// check fires before fd resolution, so no process is needed.
    #[test]
    fn sendto_null_buf_returns_efault() {
        assert_eq!(handle(3, 0, 16, 0, 0, 0), -14);
    }

    // -- fd-resolution errnos ------------------------------------------

    /// `send` with no process installed → `-ENOSYS`. A valid non-empty
    /// payload + null dest passes the early checks, so fd resolution is
    /// what fires.
    #[test]
    fn sendto_no_process_returns_enosys() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        current_process_uninstall();
        let payload = b"data";
        let result = handle(3, payload.as_ptr() as u64, payload.len() as u64, 0, 0, 0);
        assert_eq!(result, -38);
    }

    /// `send` of an unopened fd → `-EBADF`.
    #[test]
    fn sendto_unknown_fd_returns_ebadf() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        install_test_process();
        let payload = b"data";
        let result = handle(3, payload.as_ptr() as u64, payload.len() as u64, 0, 0, 0);
        assert_eq!(result, -9);
        current_process_uninstall();
    }

    /// `send` of a non-socket fd → `-ENOTSOCK`.
    #[test]
    fn sendto_non_socket_fd_returns_enotsock() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        install_test_process();
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_synthetic("/proc/cpuinfo")).expect("alloc")
        });
        let payload = b"data";
        let result = handle(fd, payload.as_ptr() as u64, payload.len() as u64, 0, 0, 0);
        assert_eq!(result, -88);
        current_process_uninstall();
    }

    // -- Socket-fd paths over the loopback net stack -------------------

    /// Serialises sendto tests that stand up `net`.
    static SENDTO_NET_TEST_LOCK: spin::Mutex<()> = spin::Mutex::new(());

    /// `send` on a freshly-created (UNconnected) socket → `-ENOTCONN`.
    /// The socket exists and is live in the registry, but its transmit
    /// half isn't open (never connected), so smoltcp's send fails with
    /// InvalidState, which the wrapper maps to NotConnected.
    #[test]
    fn sendto_unconnected_socket_returns_enotconn() {
        let _net_guard = SENDTO_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None);
        install_test_process();
        let id = net::create_tcp_socket().expect("create socket");
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_socket(id.as_u64())).expect("alloc fd")
        });
        let payload = b"data";
        let result = handle(fd, payload.as_ptr() as u64, payload.len() as u64, 0, 0, 0);
        assert_eq!(result, -107, "send on an unconnected socket must be -ENOTCONN");
        current_process_uninstall();
        net::destroy_socket(id);
    }

    /// A socket fd with a dangling id (`u64::MAX`) sends to `-EBADF` even
    /// with the net stack up — the id resolves to no live socket.
    #[test]
    fn sendto_dangling_socket_id_returns_ebadf() {
        let _net_guard = SENDTO_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None);
        install_test_process();
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_socket(u64::MAX)).expect("alloc fd")
        });
        let payload = b"data";
        let result = handle(fd, payload.as_ptr() as u64, payload.len() as u64, 0, 0, 0);
        assert_eq!(result, -9);
        current_process_uninstall();
    }

    /// `sendto` on a TCP socket WITH a non-null destination → `-EISCONN`.
    /// The connection-mode rule fires AFTER the kind is resolved (a TCP
    /// socket can't name a per-call peer). Uses an arbitrary non-null
    /// dest pointer; the rule trips before the dest is dereferenced.
    #[test]
    fn sendto_tcp_with_dest_addr_returns_eisconn() {
        let _net_guard = SENDTO_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None);
        install_test_process();
        let id = net::create_tcp_socket().expect("create tcp socket");
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_socket(id.as_u64())).expect("alloc fd")
        });
        let payload = b"hi";
        let dest = 0x5000_u64; // non-null stand-in for a dest_addr
        let result = handle(fd, payload.as_ptr() as u64, payload.len() as u64, 0, dest, 16);
        assert_eq!(result, -106, "TCP sendto with a destination must be -EISCONN");
        current_process_uninstall();
        net::destroy_socket(id);
    }

    // -- UDP (SOCK_DGRAM) sendto paths (#533) --------------------------

    /// Build a 16-byte `sockaddr_in` for `addr:port` (UDP destination).
    fn sockaddr_in(octets: [u8; 4], port: u16) -> [u8; crate::net_socket::SOCKADDR_IN_LEN] {
        use crate::net_socket::{AF_INET, SOCKADDR_IN_LEN};
        let mut buf = [0u8; SOCKADDR_IN_LEN];
        buf[0..2].copy_from_slice(&(AF_INET as u16).to_ne_bytes());
        buf[2..4].copy_from_slice(&port.to_be_bytes());
        buf[4..8].copy_from_slice(&octets);
        buf
    }

    /// Allocate an installed-process UDP socket fd over loopback. Returns
    /// (socket id, fd). Caller holds both test locks + has called
    /// `net::init(None)` + installed a process.
    fn udp_socket_fd() -> (crate::net_socket::SocketId, i32) {
        let id = net::create_udp_socket().expect("create udp socket");
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_socket(id.as_u64())).expect("alloc fd")
        });
        (id, fd)
    }

    /// `sendto` on a UDP socket with a NULL destination → `-EDESTADDRREQ`.
    /// A datagram socket needs a per-packet destination; the
    /// connectionless rule fires after the kind is resolved.
    #[test]
    fn sendto_udp_null_dest_returns_edestaddrreq() {
        let _net_guard = SENDTO_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None);
        install_test_process();
        let (id, fd) = udp_socket_fd();
        let payload = b"datagram";
        // dest_addr = 0 (null) on a UDP socket → EDESTADDRREQ (89).
        let result = handle(fd, payload.as_ptr() as u64, payload.len() as u64, 0, 0, 0);
        assert_eq!(result, -89, "UDP sendto with no destination must be -EDESTADDRREQ");
        current_process_uninstall();
        net::destroy_socket(id);
    }

    /// `sendto` on an UNBOUND UDP socket (with a valid destination) →
    /// `-EINVAL`. smoltcp refuses to send from a socket with no local
    /// port (Unaddressable), which the wrapper → handler maps to EINVAL.
    /// A UDP socket must be `bind`-ed before it can send.
    #[test]
    fn sendto_udp_unbound_returns_einval() {
        let _net_guard = SENDTO_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None);
        install_test_process();
        let (id, fd) = udp_socket_fd();
        let dest = sockaddr_in([127, 0, 0, 1], 5000);
        let payload = b"datagram";
        let result = handle(
            fd,
            payload.as_ptr() as u64,
            payload.len() as u64,
            0,
            dest.as_ptr() as u64,
            crate::net_socket::SOCKADDR_IN_LEN as u64,
        );
        assert_eq!(result, -22, "UDP sendto from an unbound socket must be -EINVAL");
        current_process_uninstall();
        net::destroy_socket(id);
    }

    /// `sendto` on a BOUND UDP socket to a concrete loopback peer
    /// succeeds, returning the full payload length. Binds a local port
    /// first (`net::udp_bind_socket`), then sends — the full UDP send
    /// contract end-to-end over the loopback stack.
    #[test]
    fn sendto_udp_bound_to_peer_succeeds() {
        let _net_guard = SENDTO_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None);
        install_test_process();
        let (id, fd) = udp_socket_fd();
        // Bind a local port so the socket can send.
        net::udp_bind_socket(id, 0, 6000).expect("bind udp local port");
        let dest = sockaddr_in([127, 0, 0, 1], 5000);
        let payload = b"datagram";
        let result = handle(
            fd,
            payload.as_ptr() as u64,
            payload.len() as u64,
            0,
            dest.as_ptr() as u64,
            crate::net_socket::SOCKADDR_IN_LEN as u64,
        );
        assert_eq!(
            result,
            payload.len() as i64,
            "bound UDP sendto must return the payload length, got {}",
            result
        );
        current_process_uninstall();
        net::destroy_socket(id);
    }
}
