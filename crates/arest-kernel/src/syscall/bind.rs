// crates/arest-kernel/src/syscall/bind.rs
//
// Linux x86_64 syscall 49: `bind(int sockfd, const struct sockaddr *addr,
// socklen_t addrlen)`. Per #529 (the bind+listen leg of the userspace
// TCP socket cluster, continuing after `socket()` #478a). `bind` assigns
// a local address (IP + port) to a socket so a following `listen`
// (#529, `syscall::listen`) can accept connections on it.
//
// Linux x86_64 number: `__NR_bind = 49`
// (`linux/arch/x86/include/uapi/asm/unistd_64.h`; the vendored musl tree
// confirms the same value at
// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_bind`).
//
// How the pieces fit (the #478a host-testable split, continued)
// -------------------------------------------------------------
// `bind` follows the exact three-way split `socket()` established:
//
//   * The pure decision — decoding + validating the `struct sockaddr_in`
//     the caller passes — is `crate::net_socket::parse_sockaddr_in`
//     (host-unit-tested: (bytes) → SockAddrIn | -errno). The fd→SocketId
//     resolution + the userspace `sockaddr` read + the SocketError→errno
//     mapping are the shared helpers in `crate::syscall::socket`
//     (`resolve_socket_fd` / `read_sockaddr` / `socket_error_to_errno`),
//     also host-testable.
//   * The smoltcp side — recording the bound endpoint so `listen` can
//     use it — is `crate::net::tcp_bind` (the gated wrapper; on the host
//     loopback build it runs end-to-end, before `net::init` it returns
//     `NotInitialised`).
//   * This handler is the glue: read the sockaddr, parse it, resolve the
//     fd, call `tcp_bind`, map the result to a Linux errno.
//
// Why smoltcp `bind` is bookkeeping, not a wire op
// ------------------------------------------------
// smoltcp's `tcp::Socket` has no standalone bind step — a server supplies
// its local endpoint to `listen(endpoint)`, a client to `connect(..,
// local)`. POSIX `bind` therefore can't map 1:1 onto a smoltcp call at
// `bind` time, so `net::tcp_bind` records the (addr, port) in the
// NetState and `net::tcp_listen` consults it. The full rationale lives on
// `net::tcp_bind`.
//
// Return value
// ------------
// Linux `bind(2)` returns 0 on success, or `-errno`:
//   * `-EBADF` (9) — `sockfd` isn't an open fd.
//   * `-EFAULT` (14) — `addr` is a null / bad pointer.
//   * `-EINVAL` (22) — `addrlen` is too short for a `sockaddr_in`, or the
//                      socket is already bound/connected.
//   * `-ENOTSOCK` (88) — `sockfd` is open but not a socket.
//   * `-EAFNOSUPPORT` (97) — the address family isn't `AF_INET`.
//   * `-EADDRINUSE` (98) — the local port is already in use.
//   * `-ENOSYS` (38) — no process is installed, or the net stack isn't up.
//
// The argument-error errnos (`-EINVAL` short buffer, `-EAFNOSUPPORT`)
// come from `net_socket::parse_sockaddr_in`; the fd / state / address
// errnos come from the shared helpers + `net::tcp_bind`.

use crate::net;
use crate::net_socket::parse_sockaddr_in;
use crate::syscall::dispatch::ENOSYS;
use crate::syscall::dispatch::EBADF;
use crate::syscall::socket::{read_sockaddr, resolve_socket_fd, socket_error_to_errno, SocketOp};

/// Linux x86_64 syscall number for `bind(sockfd, addr, addrlen)`.
/// Source: `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_bind`
/// (= 49). The vendored musl tree confirms the same value at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_bind`. Routes to
/// `bind::handle`, which records the socket's local endpoint for a
/// following `listen`. Per #529.
pub const SYS_BIND: u64 = 49;

/// Handle a `bind(sockfd, addr, addrlen)` syscall. Returns 0 on success
/// or a negative errno (see the module docstring for the full table).
///
/// Steps, in order:
///   1. Read `addrlen` bytes of the `sockaddr` from userspace `addr`.
///      A null pointer / oversized length is `-EFAULT`.
///   2. Parse + validate the `sockaddr_in` (family must be `AF_INET`,
///      buffer must be ≥ 16 bytes). Bad family → `-EAFNOSUPPORT`; short
///      buffer → `-EINVAL`.
///   3. Resolve `sockfd` to its kernel `SocketId`. Unknown fd → `-EBADF`;
///      non-socket fd → `-ENOTSOCK`; no process → `-ENOSYS`.
///   4. Record the (addr, port) via `net::tcp_bind`; map any
///      `SocketError` to its errno.
///
/// `sockfd` arrives as the raw `u64` dispatch register; it's `int` in
/// the C signature, so it's narrowed to `i32` (the fd-table key width) —
/// a value that doesn't fit can't be an open fd and surfaces as `-EBADF`
/// through `resolve_socket_fd`.
pub fn handle(sockfd: i32, addr: u64, addrlen: u64) -> i64 {
    // (1) Copy the sockaddr bytes out of userspace.
    let bytes = match read_sockaddr(addr, addrlen) {
        Ok(b) => b,
        Err(errno) => return errno,
    };

    // (2) Decode + validate the sockaddr_in (pure, host-tested).
    let sockaddr = match parse_sockaddr_in(&bytes) {
        Ok(sa) => sa,
        Err(errno) => return errno,
    };

    // (3) Resolve the fd to a socket id.
    let socket_id = match resolve_socket_fd(sockfd) {
        Ok(id) => id,
        Err(errno) => return errno,
    };

    // (4) Bind, routing on the socket's transport kind (#533): a UDP
    //     socket binds its local port for real in smoltcp; a TCP socket
    //     records the endpoint for a following `listen`. An id with no
    //     recorded kind is a dangling socket fd → -EBADF.
    let result = match net::socket_kind(socket_id) {
        Some(net::SocketKind::Udp) => {
            net::udp_bind_socket(socket_id, sockaddr.addr, sockaddr.port)
        }
        Some(net::SocketKind::Tcp) => net::tcp_bind(socket_id, sockaddr.addr, sockaddr.port),
        // Net stack down → -ENOSYS; otherwise the id isn't a live socket
        // → -EBADF (the same UnknownSocket mapping the wrappers give).
        None => {
            if net::is_online() {
                return -EBADF;
            }
            return -ENOSYS;
        }
    };
    match result {
        Ok(()) => 0,
        Err(e) => socket_error_to_errno(e, SocketOp::Bind),
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

    /// `SYS_BIND` is 49 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_bind`.
    #[test]
    fn sys_bind_number_matches_linux_uapi() {
        assert_eq!(SYS_BIND, 49);
    }

    /// Build a 16-byte `sockaddr_in` for `addr:port` (addr as octets).
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

    // -- Argument-validation rejections (no live interface / process) --
    //
    // These exercise the handler's steps (1)+(2): a bad sockaddr is
    // rejected with the right errno BEFORE the fd is resolved or any
    // socket is touched, so they don't need a process or the net stack.

    /// `bind(fd, NULL, 16)` → `-EFAULT` (a null sockaddr pointer).
    #[test]
    fn bind_null_addr_returns_efault() {
        assert_eq!(handle(3, 0, SOCKADDR_IN_LEN as u64), -14);
    }

    /// `bind(fd, &short, 8)` → `-EINVAL` — an 8-byte addrlen is too short
    /// to hold a `sockaddr_in` (parser rejects before the fd is touched).
    #[test]
    fn bind_short_addrlen_returns_einval() {
        let buf = sockaddr_in([127, 0, 0, 1], 8080);
        // addrlen 8 < 16: the parser sees a too-short slice.
        let result = handle(3, buf.as_ptr() as u64, 8);
        assert_eq!(result, -22);
    }

    /// `bind(fd, &sockaddr_in6, 16)` → `-EAFNOSUPPORT` — wrong family.
    #[test]
    fn bind_af_inet6_returns_eafnosupport() {
        let mut buf = [0u8; SOCKADDR_IN_LEN];
        buf[0..2].copy_from_slice(&(AF_INET6 as u16).to_ne_bytes());
        let result = handle(3, buf.as_ptr() as u64, SOCKADDR_IN_LEN as u64);
        assert_eq!(result, -97);
    }

    // -- fd-resolution errnos (process installed, but fd wrong) --------

    /// `bind` with no process installed → `-ENOSYS` (pre-spawn sentinel),
    /// even for a well-formed sockaddr. The sockaddr passes parsing, so
    /// the fd-resolution step is what fires here.
    #[test]
    fn bind_no_process_returns_enosys() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        current_process_uninstall(); // ensure none leftover
        let buf = sockaddr_in([127, 0, 0, 1], 8080);
        let result = handle(3, buf.as_ptr() as u64, SOCKADDR_IN_LEN as u64);
        assert_eq!(result, -38);
    }

    /// `bind` of an unopened fd → `-EBADF`. A process is installed but fd
    /// 3 was never allocated.
    #[test]
    fn bind_unknown_fd_returns_ebadf() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        install_test_process();
        let buf = sockaddr_in([127, 0, 0, 1], 8080);
        let result = handle(3, buf.as_ptr() as u64, SOCKADDR_IN_LEN as u64);
        assert_eq!(result, -9);
        current_process_uninstall();
    }

    /// `bind` of a non-socket fd (a synthetic `/proc/cpuinfo` fd) →
    /// `-ENOTSOCK`. The fd is open but doesn't back a socket.
    #[test]
    fn bind_non_socket_fd_returns_enotsock() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        install_test_process();
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_synthetic("/proc/cpuinfo")).expect("alloc")
        });
        let buf = sockaddr_in([127, 0, 0, 1], 8080);
        let result = handle(fd, buf.as_ptr() as u64, SOCKADDR_IN_LEN as u64);
        assert_eq!(result, -88);
        current_process_uninstall();
    }

    // -- Socket-fd paths over the loopback net stack -------------------
    //
    // These bring up `net::init(None)` (the loopback interface) so a
    // socket fd binds against a real socket. They touch BOTH the
    // process-global `CURRENT_PROCESS` and the process-global `NET`
    // slot, so they hold the process test lock and a dedicated net lock
    // (the socket-module's `SOCKET_NET_TEST_LOCK` is private to that
    // module, so — exactly as `syscall::socket`'s tests note — we use a
    // local lock for the socket-syscall net-touching tests here).

    /// Serialises bind tests that stand up `net` so a concurrent
    /// `init(None)` can't reset the stack mid-test.
    static BIND_NET_TEST_LOCK: spin::Mutex<()> = spin::Mutex::new(());

    /// A `bind(fd, 0.0.0.0:8080, 16)` on a real socket fd over the
    /// loopback stack succeeds (returns 0). Creates the socket via
    /// `net::create_tcp_socket`, binds the fd to its id, then drives the
    /// handler — the full bind contract end-to-end.
    #[test]
    fn bind_socket_fd_wildcard_succeeds() {
        let _net_guard = BIND_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None); // loopback — makes the socket live
        install_test_process();
        let id = net::create_tcp_socket().expect("create tcp socket on loopback");
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_socket(id.as_u64())).expect("alloc socket fd")
        });
        let buf = sockaddr_in([0, 0, 0, 0], 8080);
        let result = handle(fd, buf.as_ptr() as u64, SOCKADDR_IN_LEN as u64);
        assert_eq!(result, 0, "bind to 0.0.0.0:8080 must succeed, got {}", result);
        current_process_uninstall();
        net::destroy_socket(id);
    }

    /// A socket fd whose stored id ISN'T in the net registry (here
    /// `u64::MAX`, which the monotonic allocator never reaches) binds to
    /// `-EBADF` — the fd is a `Socket` entry but resolves to no live
    /// socket. Brings up the net stack so the failure is specifically
    /// "unknown socket id", not "net not initialised".
    #[test]
    fn bind_dangling_socket_id_returns_ebadf() {
        let _net_guard = BIND_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None);
        install_test_process();
        let fd = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_socket(u64::MAX)).expect("alloc socket fd")
        });
        let buf = sockaddr_in([0, 0, 0, 0], 8080);
        let result = handle(fd, buf.as_ptr() as u64, SOCKADDR_IN_LEN as u64);
        assert_eq!(result, -9, "dangling socket id must be -EBADF");
        current_process_uninstall();
    }
}
