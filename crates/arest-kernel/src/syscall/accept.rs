// crates/arest-kernel/src/syscall/accept.rs
//
// Linux x86_64 syscall 43: `accept(int sockfd, struct sockaddr *addr,
// socklen_t *addrlen)`. Per #530 (the accept leg of the userspace TCP
// socket cluster). `accept` pulls the next completed inbound connection
// off a LISTENing socket and returns a NEW fd for it; the listening
// socket stays listening for further connections.
//
// Linux x86_64 number: `__NR_accept = 43`
// (`linux/arch/x86/include/uapi/asm/unistd_64.h`; the vendored musl tree
// confirms the same value at
// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_accept`).
//
// How the pieces fit (the #478a host-testable split, continued)
// -------------------------------------------------------------
//   * `accept` carries no inbound sockaddr to decode, so it has no pure
//     parse step. Its host-testable surface is the fd→SocketId
//     resolution + the kind guard + the SocketError→errno mapping (the
//     shared `syscall::socket` helpers). The peer-address write into the
//     caller's `addr` reuses the same `sockaddr_in` layout
//     `recvfrom` writes.
//   * The smoltcp side — the listen→established hand-off that mints the
//     accepted socket and re-arms a fresh listener — is
//     `net::tcp_accept` (gated wrapper). Its HAPPY path (an actually-
//     established connection) needs a live peer + `poll()` cycles, so —
//     per the #478a methodology — it's exercised in production, while the
//     host tests cover the `WouldBlock` (nothing pending) / `NotListening`
//     / non-socket / UDP / dangling-fd errno surface.
//   * This handler is the glue: resolve the fd, require a TCP listener,
//     call `tcp_accept`, allocate a new fd for the accepted connection,
//     and report the peer address.
//
// accept4 / flags
// ---------------
// This is the bare `accept(2)` (no flags). `accept4(2)` (syscall 288),
// which adds a `SOCK_NONBLOCK` / `SOCK_CLOEXEC` flags argument, is a
// follow-up; tier-1 sockets are already non-blocking and there's no
// CLOEXEC bit yet (the fd-table `Socket` entry has no flags field), so
// `accept4` would land as a thin wrapper over this once those exist.
//
// Return value
// ------------
// Linux `accept(2)` returns the new connected fd (≥ 0) on success, or
// `-errno`:
//   * `-EAGAIN` (11) — no connection is waiting (non-blocking).
//   * `-EBADF` (9) — `sockfd` isn't an open fd.
//   * `-EINVAL` (22) — `sockfd` isn't listening.
//   * `-ENOTSOCK` (88) — `sockfd` is open but not a socket.
//   * `-EOPNOTSUPP` (95) — `sockfd` is a UDP socket (accept is stream-
//                      only).
//   * `-EMFILE` (24) — the fd table is full (the accepted connection is
//                      torn back down so it doesn't leak).
//   * `-ENOSYS` (38) — no process is installed, or the net stack isn't up.

use crate::net;
use crate::net_socket::{SockAddrIn, AF_INET, EOPNOTSUPP, SOCKADDR_IN_LEN};
use crate::process::current_process_fd_table;
use crate::process::fd_table::socket as fd_socket;
use crate::syscall::dispatch::{EBADF, ENOSYS};
use crate::syscall::openat::EMFILE;
use crate::syscall::socket::{resolve_socket_fd, socket_error_to_errno, SocketOp};

/// Linux x86_64 syscall number for `accept(sockfd, addr, addrlen)`.
/// Source: `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_accept`
/// (= 43). The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_accept`. Routes to
/// `accept::handle`, which pulls the next completed connection off a
/// listening socket and returns a new fd for it. Per #530.
pub const SYS_ACCEPT: u64 = 43;

/// Handle an `accept(sockfd, addr, addrlen)` syscall. Returns the new
/// connected fd (≥ 3) on success, or a negative errno (see the module
/// docstring).
///
/// Steps, in order:
///   1. Resolve `sockfd` to its `SocketId`. Unknown → `-EBADF`;
///      non-socket → `-ENOTSOCK`; no process → `-ENOSYS`.
///   2. Require a TCP socket — `accept` is stream-only, so a UDP fd →
///      `-EOPNOTSUPP` (this also guards against a wrong-type smoltcp
///      downcast in `tcp_accept`).
///   3. Pull the next connection via `net::tcp_accept`. Nothing pending
///      → `-EAGAIN`; not listening → `-EINVAL`.
///   4. Allocate a new fd bound to the accepted `SocketId`. Table full →
///      `-EMFILE` (the accepted connection is torn down so it doesn't
///      leak).
///   5. Write the peer address into `addr` / `*addrlen` when the caller
///      provided them.
///
/// `sockfd` is narrowed to `i32`; a value that doesn't fit surfaces as
/// `-EBADF`.
pub fn handle(sockfd: i32, addr: u64, addrlen: u64) -> i64 {
    // (1) Resolve the fd to a socket id.
    let socket_id = match resolve_socket_fd(sockfd) {
        Ok(id) => id,
        Err(errno) => return errno,
    };

    // (2) accept is TCP-only. Reject UDP with -EOPNOTSUPP (also guards a
    //     wrong-type smoltcp downcast in `tcp_accept`). No kind → dangling
    //     socket fd (net up) or stack down.
    match net::socket_kind(socket_id) {
        Some(net::SocketKind::Tcp) => {}
        Some(net::SocketKind::Udp) => return -EOPNOTSUPP,
        None => {
            return if net::is_online() { -EBADF } else { -ENOSYS };
        }
    }

    // (3) Pull the next completed connection.
    let (accepted_id, peer) = match net::tcp_accept(socket_id) {
        Ok(pair) => pair,
        Err(e) => return socket_error_to_errno(e, SocketOp::Accept),
    };

    // (4) Allocate a new fd for the accepted connection. On table-full,
    //     tear the accepted socket back down so it doesn't leak (it's a
    //     live connection — destroy_socket removes it from the set,
    //     dropping its rings; the peer will see the reset on its next
    //     send, which is the correct outcome for "the server couldn't
    //     accept you").
    let fd = current_process_fd_table(|maybe_table| match maybe_table {
        Some(table) => match table.allocate(fd_socket(accepted_id.as_u64())) {
            Ok(fd) => fd as i64,
            Err(()) => -EMFILE,
        },
        None => -ENOSYS,
    });
    if fd < 0 {
        net::destroy_socket(accepted_id);
        return fd;
    }

    // (5) Report the peer address (when the caller asked for it).
    write_peer_sockaddr(addr, addrlen, &peer);

    fd
}

/// Write the accepted connection's peer `peer` as a 16-byte `struct
/// sockaddr_in` into the caller's `addr` buffer and update `*addrlen` to
/// 16. A null `addr` means the caller doesn't want the peer (a common
/// `accept(fd, NULL, NULL)`), so nothing is written. Mirrors the source-
/// address write `recvfrom` performs for UDP: `sin_family` host-order at
/// offset 0, `sin_port` network-order at 2, `sin_addr` network-order at
/// 4, `sin_zero` (8 bytes) zeroed at 8.
///
/// SAFETY: each write is guarded by a non-null check; tier-1's identity
/// mapping makes a non-null pointer valid kernel memory. The 16 bytes
/// written match `SOCKADDR_IN_LEN`; a caller passing a non-null `addr` is
/// trusted to have provided a `sockaddr`-sized buffer (the libc
/// convention) — a stricter clamp to the incoming `*addrlen` is a
/// follow-up once `copy_to_user` (#561) validates the span.
fn write_peer_sockaddr(addr: u64, addrlen: u64, peer: &SockAddrIn) {
    if addr == 0 {
        return;
    }
    let mut buf = [0u8; SOCKADDR_IN_LEN];
    buf[0..2].copy_from_slice(&(AF_INET as u16).to_ne_bytes());
    buf[2..4].copy_from_slice(&peer.port.to_be_bytes());
    buf[4..8].copy_from_slice(&peer.octets());
    // SAFETY: `addr` is non-null (checked); identity mapping makes the
    // 16-byte span valid kernel memory.
    unsafe {
        core::ptr::copy_nonoverlapping(buf.as_ptr(), addr as *mut u8, SOCKADDR_IN_LEN);
    }
    if addrlen != 0 {
        // SAFETY: `addrlen` non-null; `socklen_t` is a 32-bit uint on
        // Linux x86_64.
        unsafe {
            core::ptr::write(addrlen as *mut u32, SOCKADDR_IN_LEN as u32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net_socket::SOCKADDR_IN_LEN as SA_LEN;
    use crate::process::address_space::AddressSpace;
    use crate::process::fd_table::{socket as fd_socket, synthetic as fd_synthetic};
    use crate::process::process::CURRENT_PROCESS_TEST_LOCK;
    use crate::process::{
        current_process_fd_table, current_process_install, current_process_uninstall, Process,
    };

    /// `SYS_ACCEPT` is 43 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_accept`.
    #[test]
    fn sys_accept_number_matches_linux_uapi() {
        assert_eq!(SYS_ACCEPT, 43);
    }

    fn install_test_process() {
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(7, address_space);
        current_process_install(proc);
    }

    // -- fd-resolution errnos (no live interface needed) ---------------

    /// `accept` with no process installed → `-ENOSYS`.
    #[test]
    fn accept_no_process_returns_enosys() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        current_process_uninstall();
        assert_eq!(handle(3, 0, 0), -38);
    }

    /// `accept` of an unopened fd → `-EBADF`.
    #[test]
    fn accept_unknown_fd_returns_ebadf() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        install_test_process();
        assert_eq!(handle(3, 0, 0), -9);
        current_process_uninstall();
    }

    /// `accept` of a non-socket fd → `-ENOTSOCK`.
    #[test]
    fn accept_non_socket_fd_returns_enotsock() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        install_test_process();
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_synthetic("/proc/cpuinfo")).expect("alloc")
        });
        assert_eq!(handle(fd, 0, 0), -88);
        current_process_uninstall();
    }

    // -- Socket-fd paths over the loopback net stack -------------------

    /// Serialises accept tests that stand up `net`.
    static ACCEPT_NET_TEST_LOCK: spin::Mutex<()> = spin::Mutex::new(());

    /// Build a 16-byte `sockaddr_in` for `0.0.0.0:port`.
    fn wildcard_sockaddr(port: u16) -> [u8; SA_LEN] {
        let mut buf = [0u8; SA_LEN];
        buf[0..2].copy_from_slice(&(AF_INET as u16).to_ne_bytes());
        buf[2..4].copy_from_slice(&port.to_be_bytes());
        buf
    }

    /// `accept` on a freshly-LISTENing socket with no pending connection
    /// → `-EAGAIN`. Sets up bind+listen over loopback, then accepts with
    /// nothing connected — the non-blocking "nothing waiting" signal.
    /// This is the deterministic, host-testable accept path (the happy
    /// path needs a live peer + poll cycles, which is the gated
    /// production path).
    #[test]
    fn accept_listening_no_pending_returns_eagain() {
        let _net_guard = ACCEPT_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None);
        install_test_process();
        let id = net::create_tcp_socket().expect("create socket");
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_socket(id.as_u64())).expect("alloc fd")
        });
        // bind + listen so the socket is in LISTEN.
        let buf = wildcard_sockaddr(8080);
        assert_eq!(
            crate::syscall::bind::handle(fd, buf.as_ptr() as u64, SA_LEN as u64),
            0,
            "bind must succeed"
        );
        assert_eq!(crate::syscall::listen::handle(fd, 128), 0, "listen must succeed");
        // accept with nothing pending → EAGAIN.
        let result = handle(fd, 0, 0);
        assert_eq!(result, -11, "accept with no pending connection must be -EAGAIN");
        current_process_uninstall();
        net::destroy_socket(id);
    }

    /// `accept` on a NON-listening socket (created but never `listen`-ed)
    /// → `-EINVAL`. Pins the not-listening precondition.
    #[test]
    fn accept_non_listening_returns_einval() {
        let _net_guard = ACCEPT_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None);
        install_test_process();
        let id = net::create_tcp_socket().expect("create socket");
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_socket(id.as_u64())).expect("alloc fd")
        });
        // No bind / listen — the socket is idle, not listening.
        let result = handle(fd, 0, 0);
        assert_eq!(result, -22, "accept on a non-listening socket must be -EINVAL");
        current_process_uninstall();
        net::destroy_socket(id);
    }

    /// `accept` on a UDP socket → `-EOPNOTSUPP` (accept is stream-only).
    #[test]
    fn accept_udp_socket_returns_eopnotsupp() {
        let _net_guard = ACCEPT_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None);
        install_test_process();
        let id = net::create_udp_socket().expect("create udp socket");
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_socket(id.as_u64())).expect("alloc fd")
        });
        let result = handle(fd, 0, 0);
        assert_eq!(result, -95, "accept on a UDP socket must be -EOPNOTSUPP");
        current_process_uninstall();
        net::destroy_socket(id);
    }

    /// A socket fd with a dangling id (`u64::MAX`) accepts to `-EBADF`
    /// even with the net stack up.
    #[test]
    fn accept_dangling_socket_id_returns_ebadf() {
        let _net_guard = ACCEPT_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None);
        install_test_process();
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_socket(u64::MAX)).expect("alloc fd")
        });
        assert_eq!(handle(fd, 0, 0), -9);
        current_process_uninstall();
    }
}
