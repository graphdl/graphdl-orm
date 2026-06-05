// crates/arest-kernel/src/syscall/connect.rs
//
// Linux x86_64 syscall 42: `connect(int sockfd, const struct sockaddr
// *addr, socklen_t addrlen)`. Per #531 (the connect leg of the userspace
// TCP socket cluster, after socket() #478a / bind+listen #529). `connect`
// is the client-side active open: it initiates a TCP connection from
// `sockfd` to the peer named by `addr`.
//
// Linux x86_64 number: `__NR_connect = 42`
// (`linux/arch/x86/include/uapi/asm/unistd_64.h`; the vendored musl tree
// confirms the same value at
// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_connect`).
//
// How the pieces fit (the #478a host-testable split, continued)
// -------------------------------------------------------------
//   * Pure decisions — decoding the `sockaddr_in`
//     (`net_socket::parse_sockaddr_in`) AND validating it as a connect
//     *destination* (`net_socket::validate_connect_target`: a `0.0.0.0`
//     or port-0 target is `-EINVAL`). Both host-unit-tested.
//   * The smoltcp side — kicking off the active-open handshake — is
//     `net::tcp_connect` (gated wrapper). It picks the local port (a
//     prior bind's, else ephemeral) and calls smoltcp `connect`.
//   * This handler is the glue: read + parse + validate the sockaddr,
//     resolve the fd, call `tcp_connect`, map the result to a Linux
//     errno.
//
// Non-blocking connect semantics
// ------------------------------
// Tier-1 sockets are inherently non-blocking — there's no scheduler to
// park a blocking `connect` on until the handshake completes (#530). So
// `connect` returns `-EINPROGRESS` once the SYN is sent: smoltcp moves
// the socket to `SynSent` and the handshake finishes on later
// `net::poll()` ticks. This is exactly the contract libc expects from a
// non-blocking socket — the caller then `poll`s / `select`s for
// writability (or reads `getsockopt(SO_ERROR)`) to learn the outcome.
// Both of those observation surfaces are follow-ups; what this slice
// guarantees is that the handshake is correctly initiated and the
// in-progress / error states are reported with the right errno.
//
// Return value
// ------------
// Linux `connect(2)` returns 0 on success (blocking) or `-errno`:
//   * `-EINPROGRESS` (115) — the (non-blocking) handshake is underway
//                      (the tier-1 success-shaped outcome).
//   * `-EBADF` (9) — `sockfd` isn't an open fd.
//   * `-EFAULT` (14) — `addr` is a null / bad pointer.
//   * `-EINVAL` (22) — `addrlen` too short, a wildcard / port-0 target,
//                      or the socket is in a state smoltcp rejects.
//   * `-ENOTSOCK` (88) — `sockfd` is open but not a socket.
//   * `-EAFNOSUPPORT` (97) — the address family isn't `AF_INET`.
//   * `-EISCONN` (106) — the socket is already connected.
//   * `-ECONNREFUSED` (111) — (observed on a later poll, not at the
//                      initial call) the peer refused the connection.
//   * `-ENOSYS` (38) — no process is installed, or the net stack isn't up.

use crate::net;
use crate::net_socket::{parse_sockaddr_in, validate_connect_target};
use crate::syscall::socket::{read_sockaddr, resolve_socket_fd, socket_error_to_errno, SocketOp};

/// Linux x86_64 syscall number for `connect(sockfd, addr, addrlen)`.
/// Source: `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_connect`
/// (= 42). The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_connect`. Routes to
/// `connect::handle`, which initiates the active-open TCP handshake to
/// the peer named by `addr`. Per #531.
pub const SYS_CONNECT: u64 = 42;

/// Handle a `connect(sockfd, addr, addrlen)` syscall. Returns 0 on a
/// (blocking) success — which tier-1 never produces — or a negative
/// errno; the non-blocking success outcome is `-EINPROGRESS` (see the
/// module docstring).
///
/// Steps, in order:
///   1. Read `addrlen` bytes of the `sockaddr` from userspace. Null /
///      oversized → `-EFAULT`.
///   2. Parse the `sockaddr_in` (family `AF_INET`, ≥ 16 bytes), then
///      validate it as a connect target (no `0.0.0.0`, no port 0). Bad
///      family → `-EAFNOSUPPORT`; short buffer / wildcard / port-0 →
///      `-EINVAL`.
///   3. Resolve `sockfd` to its kernel `SocketId`. Unknown → `-EBADF`;
///      non-socket → `-ENOTSOCK`; no process → `-ENOSYS`.
///   4. Kick off the handshake via `net::tcp_connect`; map the result to
///      its errno (`ConnectInProgress` → `-EINPROGRESS`).
///
/// `sockfd` is narrowed to `i32`; a value that doesn't fit surfaces as
/// `-EBADF` through `resolve_socket_fd`.
pub fn handle(sockfd: i32, addr: u64, addrlen: u64) -> i64 {
    // (1) Copy the sockaddr bytes out of userspace.
    let bytes = match read_sockaddr(addr, addrlen) {
        Ok(b) => b,
        Err(errno) => return errno,
    };

    // (2) Decode + validate the sockaddr_in, then sanity-check it as a
    //     connect destination (pure, host-tested).
    let sockaddr = match parse_sockaddr_in(&bytes) {
        Ok(sa) => sa,
        Err(errno) => return errno,
    };
    if let Err(errno) = validate_connect_target(&sockaddr) {
        return errno;
    }

    // (3) Resolve the fd to a socket id.
    let socket_id = match resolve_socket_fd(sockfd) {
        Ok(id) => id,
        Err(errno) => return errno,
    };

    // (4) Start the handshake; map any outcome to its errno. A
    //     successfully-started non-blocking connect surfaces as
    //     `ConnectInProgress` → `-EINPROGRESS`.
    match net::tcp_connect(socket_id, sockaddr.addr, sockaddr.port) {
        // tier-1 never completes synchronously, so an `Ok` here would be
        // a blocking success (0). Kept for completeness / a future
        // blocking-connect mode.
        Ok(()) => 0,
        Err(e) => socket_error_to_errno(e, SocketOp::Connect),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net_socket::{AF_INET, AF_INET6, SOCKADDR_IN_LEN};
    use crate::process::address_space::AddressSpace;
    use crate::process::fd_table::{socket as fd_socket, synthetic as fd_synthetic};
    use crate::process::process::CURRENT_PROCESS_TEST_LOCK;
    use crate::process::{
        current_process_fd_table, current_process_install, current_process_uninstall, Process,
    };

    /// `SYS_CONNECT` is 42 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_connect`.
    #[test]
    fn sys_connect_number_matches_linux_uapi() {
        assert_eq!(SYS_CONNECT, 42);
    }

    /// Build a 16-byte `sockaddr_in` for `addr:port`.
    fn sockaddr_in(octets: [u8; 4], port: u16) -> [u8; SOCKADDR_IN_LEN] {
        let mut buf = [0u8; SOCKADDR_IN_LEN];
        buf[0..2].copy_from_slice(&(AF_INET as u16).to_ne_bytes());
        buf[2..4].copy_from_slice(&port.to_be_bytes());
        buf[4..8].copy_from_slice(&octets);
        buf
    }

    fn install_test_process() {
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(7, address_space);
        current_process_install(proc);
    }

    // -- Argument-validation rejections (no process / stack needed) ----

    /// `connect(fd, NULL, 16)` → `-EFAULT`.
    #[test]
    fn connect_null_addr_returns_efault() {
        assert_eq!(handle(3, 0, SOCKADDR_IN_LEN as u64), -14);
    }

    /// `connect(fd, &sockaddr_in6, 16)` → `-EAFNOSUPPORT`.
    #[test]
    fn connect_af_inet6_returns_eafnosupport() {
        let mut buf = [0u8; SOCKADDR_IN_LEN];
        buf[0..2].copy_from_slice(&(AF_INET6 as u16).to_ne_bytes());
        let result = handle(3, buf.as_ptr() as u64, SOCKADDR_IN_LEN as u64);
        assert_eq!(result, -97);
    }

    /// `connect(fd, 0.0.0.0:80, 16)` → `-EINVAL` — the wildcard address
    /// isn't a routable connect target. The connect-target validation
    /// (pure) fires before the fd is resolved, so no process is needed.
    #[test]
    fn connect_wildcard_addr_returns_einval() {
        let buf = sockaddr_in([0, 0, 0, 0], 80);
        assert_eq!(handle(3, buf.as_ptr() as u64, SOCKADDR_IN_LEN as u64), -22);
    }

    /// `connect(fd, 1.2.3.4:0, 16)` → `-EINVAL` — port 0 is never a valid
    /// connect destination.
    #[test]
    fn connect_zero_port_returns_einval() {
        let buf = sockaddr_in([1, 2, 3, 4], 0);
        assert_eq!(handle(3, buf.as_ptr() as u64, SOCKADDR_IN_LEN as u64), -22);
    }

    // -- fd-resolution errnos ------------------------------------------

    /// `connect` with no process installed → `-ENOSYS`, for a well-formed
    /// concrete target (so the fd-resolution step is what fires).
    #[test]
    fn connect_no_process_returns_enosys() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        current_process_uninstall();
        let buf = sockaddr_in([1, 2, 3, 4], 80);
        assert_eq!(handle(3, buf.as_ptr() as u64, SOCKADDR_IN_LEN as u64), -38);
    }

    /// `connect` of an unopened fd → `-EBADF`.
    #[test]
    fn connect_unknown_fd_returns_ebadf() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        install_test_process();
        let buf = sockaddr_in([1, 2, 3, 4], 80);
        assert_eq!(handle(3, buf.as_ptr() as u64, SOCKADDR_IN_LEN as u64), -9);
        current_process_uninstall();
    }

    /// `connect` of a non-socket fd → `-ENOTSOCK`.
    #[test]
    fn connect_non_socket_fd_returns_enotsock() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        install_test_process();
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_synthetic("/proc/cpuinfo")).expect("alloc")
        });
        let buf = sockaddr_in([1, 2, 3, 4], 80);
        assert_eq!(handle(fd, buf.as_ptr() as u64, SOCKADDR_IN_LEN as u64), -88);
        current_process_uninstall();
    }

    // -- Socket-fd paths over the loopback net stack -------------------
    //
    // Bring up `net::init(None)` so a connect runs against a real socket.
    // Hold the process lock + a local net lock (the socket module's net
    // lock is private), same discipline as bind/listen tests.

    /// Serialises connect tests that stand up `net`.
    static CONNECT_NET_TEST_LOCK: spin::Mutex<()> = spin::Mutex::new(());

    /// `connect` of a freshly-created socket to a concrete loopback peer
    /// returns `-EINPROGRESS` — tier-1's non-blocking active open kicks
    /// off the handshake (SYN sent) and reports "in progress". There's no
    /// listener on the other end, but smoltcp accepts the connect call
    /// and moves to SynSent regardless; the in-progress signal is the
    /// contract this slice guarantees.
    #[test]
    fn connect_started_returns_einprogress() {
        let _net_guard = CONNECT_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None);
        install_test_process();
        let id = net::create_tcp_socket().expect("create socket");
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_socket(id.as_u64())).expect("alloc fd")
        });
        let buf = sockaddr_in([127, 0, 0, 1], 9999);
        let result = handle(fd, buf.as_ptr() as u64, SOCKADDR_IN_LEN as u64);
        assert_eq!(result, -115, "non-blocking connect must report -EINPROGRESS");
        current_process_uninstall();
        net::destroy_socket(id);
    }

    /// A second `connect` on an already-connecting socket → `-EISCONN`.
    /// The first connect put the socket in SynSent (already open), so
    /// smoltcp rejects the second with InvalidState, which maps to
    /// already-connected for the connect op.
    #[test]
    fn connect_already_open_returns_eisconn() {
        let _net_guard = CONNECT_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None);
        install_test_process();
        let id = net::create_tcp_socket().expect("create socket");
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_socket(id.as_u64())).expect("alloc fd")
        });
        let buf = sockaddr_in([127, 0, 0, 1], 9999);
        // First connect → in progress (socket now open / SynSent).
        assert_eq!(handle(fd, buf.as_ptr() as u64, SOCKADDR_IN_LEN as u64), -115);
        // Second connect on the now-open socket → already connected.
        let result = handle(fd, buf.as_ptr() as u64, SOCKADDR_IN_LEN as u64);
        assert_eq!(result, -106, "connect on an open socket must be -EISCONN");
        current_process_uninstall();
        net::destroy_socket(id);
    }

    /// A socket fd with a dangling id (`u64::MAX`) connects to `-EBADF`
    /// even with the net stack up — the id resolves to no live socket.
    #[test]
    fn connect_dangling_socket_id_returns_ebadf() {
        let _net_guard = CONNECT_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None);
        install_test_process();
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_socket(u64::MAX)).expect("alloc fd")
        });
        let buf = sockaddr_in([127, 0, 0, 1], 9999);
        assert_eq!(handle(fd, buf.as_ptr() as u64, SOCKADDR_IN_LEN as u64), -9);
        current_process_uninstall();
    }
}
