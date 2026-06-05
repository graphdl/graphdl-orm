// crates/arest-kernel/src/syscall/listen.rs
//
// Linux x86_64 syscall 50: `listen(int sockfd, int backlog)`. Per #529
// (the bind+listen leg of the userspace TCP socket cluster). `listen`
// marks a `bind`-ed socket as passive — willing to accept incoming
// connections — so a following `accept` (#530) can return connected
// sockets. It's the server-side companion to `bind`.
//
// Linux x86_64 number: `__NR_listen = 50`
// (`linux/arch/x86/include/uapi/asm/unistd_64.h`; the vendored musl tree
// confirms the same value at
// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_listen`).
//
// How the pieces fit (the #478a host-testable split, continued)
// -------------------------------------------------------------
// `listen` has no pure-decode step of its own — it carries no sockaddr,
// just the fd + an (ignored) backlog. Its host-testable surface is the
// fd→SocketId resolution + the SocketError→errno mapping, both the
// shared helpers in `crate::syscall::socket`. The smoltcp side —
// transitioning the socket into the LISTEN state on the port a prior
// `bind` recorded — is `crate::net::tcp_listen` (gated wrapper; runs over
// the host loopback build, returns `NotInitialised` before `net::init`).
// This handler is the thin glue.
//
// backlog handling for tier-1
// ---------------------------
// Linux's `backlog` caps the pending-connection queue. Tier-1's stack
// has a single smoltcp listen socket per `listen` call, so the effective
// backlog is one — the `backlog` argument is accepted (any value,
// including 0 or negative, which Linux itself clamps to the system max)
// but ignored. A multi-socket accept queue that honours `backlog` lands
// with the scheduler (#530, the `accept` slice). Documenting it as
// "accepted, ignored" keeps tier-1 honest: a server's `listen(fd, 128)`
// succeeds and works for one connection at a time.
//
// Return value
// ------------
// Linux `listen(2)` returns 0 on success, or `-errno`:
//   * `-EBADF` (9) — `sockfd` isn't an open fd.
//   * `-EINVAL` (22) — the socket has no bound port (no prior `bind`, or
//                      a zero port), so there's nothing to listen on.
//   * `-ENOTSOCK` (88) — `sockfd` is open but not a socket.
//   * `-EADDRINUSE` (98) — the socket is already open in a state that
//                      can't transition to LISTEN (e.g. already
//                      connected).
//   * `-ENOSYS` (38) — no process is installed, or the net stack isn't up.
//
// The fd / state errnos come from the shared helpers + `net::tcp_listen`.

use crate::net;
use crate::net_socket::EOPNOTSUPP;
use crate::syscall::dispatch::{EBADF, ENOSYS};
use crate::syscall::socket::{resolve_socket_fd, socket_error_to_errno, SocketOp};

/// Linux x86_64 syscall number for `listen(sockfd, backlog)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_listen` (= 50). The
/// vendored musl tree confirms the same value at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_listen`. Routes to
/// `listen::handle`, which transitions the socket into the LISTEN state
/// on its bound port. Per #529.
pub const SYS_LISTEN: u64 = 50;

/// Handle a `listen(sockfd, backlog)` syscall. Returns 0 on success or a
/// negative errno (see the module docstring for the full table).
///
/// Steps, in order:
///   1. Resolve `sockfd` to its kernel `SocketId`. Unknown fd → `-EBADF`;
///      non-socket fd → `-ENOTSOCK`; no process → `-ENOSYS`.
///   2. Require a TCP socket — `listen` is meaningless on a UDP
///      (connectionless) socket, so a `SOCK_DGRAM` fd → `-EOPNOTSUPP`
///      (#533). This guard ALSO prevents a wrong-type smoltcp downcast
///      (`tcp_listen` would `get_mut::<tcp::Socket>` a UDP handle and
///      panic).
///   3. Transition the socket to LISTEN via `net::tcp_listen` (which
///      reads the port a prior `bind` recorded); map any `SocketError`
///      to its errno. No bound port → `-EINVAL`.
///
/// `backlog` is accepted but ignored (tier-1 has an implicit backlog of
/// one — see the module docstring). `sockfd` is narrowed to `i32` (the
/// fd-table key width); a value that doesn't fit surfaces as `-EBADF`.
pub fn handle(sockfd: i32, _backlog: i32) -> i64 {
    // (1) Resolve the fd to a socket id.
    let socket_id = match resolve_socket_fd(sockfd) {
        Ok(id) => id,
        Err(errno) => return errno,
    };

    // (2) `listen` is a TCP-only operation. Reject UDP with -EOPNOTSUPP
    //     (also guards against a wrong-type smoltcp downcast in
    //     `tcp_listen`). An id with no kind is a dangling socket fd.
    match net::socket_kind(socket_id) {
        Some(net::SocketKind::Tcp) => {}
        Some(net::SocketKind::Udp) => return -EOPNOTSUPP,
        None => {
            return if net::is_online() { -EBADF } else { -ENOSYS };
        }
    }

    // (3) Transition to LISTEN; map any failure to its errno.
    match net::tcp_listen(socket_id) {
        Ok(()) => 0,
        Err(e) => socket_error_to_errno(e, SocketOp::Listen),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net_socket::{AF_INET, SOCKADDR_IN_LEN};
    use crate::process::address_space::AddressSpace;
    use crate::process::fd_table::{socket as fd_socket, synthetic as fd_synthetic};
    use crate::process::process::CURRENT_PROCESS_TEST_LOCK;
    use crate::process::{
        current_process_fd_table, current_process_install, current_process_uninstall, Process,
    };

    /// `SYS_LISTEN` is 50 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_listen`.
    #[test]
    fn sys_listen_number_matches_linux_uapi() {
        assert_eq!(SYS_LISTEN, 50);
    }

    fn install_test_process() {
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(7, address_space);
        current_process_install(proc);
    }

    // -- fd-resolution errnos (no live interface needed) ---------------

    /// `listen` with no process installed → `-ENOSYS` (pre-spawn
    /// sentinel).
    #[test]
    fn listen_no_process_returns_enosys() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        current_process_uninstall();
        assert_eq!(handle(3, 128), -38);
    }

    /// `listen` of an unopened fd → `-EBADF`.
    #[test]
    fn listen_unknown_fd_returns_ebadf() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        install_test_process();
        assert_eq!(handle(3, 128), -9);
        current_process_uninstall();
    }

    /// `listen` of a non-socket fd (synthetic `/proc/cpuinfo`) →
    /// `-ENOTSOCK`.
    #[test]
    fn listen_non_socket_fd_returns_enotsock() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        install_test_process();
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_synthetic("/proc/cpuinfo")).expect("alloc")
        });
        assert_eq!(handle(fd, 128), -88);
        current_process_uninstall();
    }

    // -- Socket-fd paths over the loopback net stack -------------------
    //
    // Bring up `net::init(None)` so a socket fd listens against a real
    // socket. Hold the process lock + a local net lock (the socket
    // module's net lock is private), same discipline as bind's tests.

    /// Serialises listen tests that stand up `net`.
    static LISTEN_NET_TEST_LOCK: spin::Mutex<()> = spin::Mutex::new(());

    /// Build a 16-byte `sockaddr_in` for `0.0.0.0:port`.
    fn wildcard_sockaddr(port: u16) -> [u8; SOCKADDR_IN_LEN] {
        let mut buf = [0u8; SOCKADDR_IN_LEN];
        buf[0..2].copy_from_slice(&(AF_INET as u16).to_ne_bytes());
        buf[2..4].copy_from_slice(&port.to_be_bytes());
        buf
    }

    /// A `bind` then `listen` over the loopback stack succeeds: bind to
    /// `0.0.0.0:8080`, then `listen(fd, 128)` returns 0. This is the full
    /// server-setup sequence — the bound port from `bind` is what
    /// `listen` transitions on.
    #[test]
    fn listen_after_bind_succeeds() {
        let _net_guard = LISTEN_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None);
        install_test_process();
        let id = net::create_tcp_socket().expect("create socket");
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_socket(id.as_u64())).expect("alloc fd")
        });
        // bind first so listen has a port.
        let buf = wildcard_sockaddr(8080);
        let bind_result = crate::syscall::bind::handle(fd, buf.as_ptr() as u64, SOCKADDR_IN_LEN as u64);
        assert_eq!(bind_result, 0, "bind must succeed first");
        // now listen.
        let result = handle(fd, 128);
        assert_eq!(result, 0, "listen after bind must succeed, got {}", result);
        current_process_uninstall();
        net::destroy_socket(id);
    }

    /// `listen` WITHOUT a prior `bind` → `-EINVAL` — the socket has no
    /// bound port, so there's nothing to listen on. Pins that the bound-
    /// port precondition is enforced (a fresh socket can't listen).
    #[test]
    fn listen_without_bind_returns_einval() {
        let _net_guard = LISTEN_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None);
        install_test_process();
        let id = net::create_tcp_socket().expect("create socket");
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_socket(id.as_u64())).expect("alloc fd")
        });
        // No bind — listen has no port.
        let result = handle(fd, 128);
        assert_eq!(result, -22, "listen without bind must be -EINVAL");
        current_process_uninstall();
        net::destroy_socket(id);
    }

    /// A socket fd with a dangling id (`u64::MAX`) listens to `-EBADF`
    /// even with the net stack up — the id resolves to no live socket.
    #[test]
    fn listen_dangling_socket_id_returns_ebadf() {
        let _net_guard = LISTEN_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None);
        install_test_process();
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_socket(u64::MAX)).expect("alloc fd")
        });
        assert_eq!(handle(fd, 128), -9);
        current_process_uninstall();
    }
}
