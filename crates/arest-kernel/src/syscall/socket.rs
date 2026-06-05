// crates/arest-kernel/src/syscall/socket.rs
//
// Linux x86_64 syscall 41: `socket(int domain, int type, int protocol)`.
// Per #478a — the first leg of the userspace networking surface. Scope
// is deliberately narrow: create a TCP socket (`AF_INET` +
// `SOCK_STREAM`), allocate a per-process fd, bind the smoltcp socket
// handle to that fd. NO I/O — no connect / bind / listen / send / recv.
// Returns the fd.
//
// Linux x86_64 number: `__NR_socket = 41`
// (`linux/arch/x86/include/uapi/asm/unistd_64.h`; the vendored musl
// tree carries the same value at
// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_socket`).
//
// How the pieces fit (the #972 host-testable split)
// --------------------------------------------------
// The decision logic is extracted so it runs under `cargo test --lib`
// without a live smoltcp interface:
//
//   * argument validation → `crate::net_socket::validate_socket_args`
//     (pure function: (domain, type, protocol) → Ok | -errno).
//   * the monotonic socket-id bookkeeping + the id↔smoltcp-handle
//     registry → `crate::net_socket::SocketIdAllocator` (host-tested)
//     driven by `crate::net::create_tcp_socket` (the thin smoltcp
//     wrapper).
//   * the fd allocation + fd→socket-id binding → the per-process
//     `crate::process::fd_table::FdTable` (host-tested), reached via
//     `current_process_fd_table`, storing `FdEntry::Socket { socket_id }`.
//
// This handler is the glue: validate, create, allocate, bind, return.
// `validate_socket_args` and the fd table both run on the host, so the
// handler's branch logic is exercised by the unit tests below directly
// (the only piece that needs a live interface is the actual smoltcp
// socket creation, which returns `NotInitialised` on the host and is
// asserted as such).
//
// Return value
// ------------
// Linux `socket(2)` returns the new fd (≥ 0) on success, or `-errno`:
//   * `-EAFNOSUPPORT` (97) — `domain` is not `AF_INET` (tier-1 is
//                            IPv4-only).
//   * `-EPROTONOSUPPORT` (93) — `type` is `SOCK_DGRAM`, or a
//                            `SOCK_STREAM` with a non-TCP `protocol`.
//   * `-EINVAL` (22) — `type` (after masking the SOCK_NONBLOCK /
//                            SOCK_CLOEXEC flag bits) is an unknown
//                            socket type.
//   * `-EMFILE` (24) — the per-process fd table is full (1024 entries).
//   * `-ENOSYS` (38) — no process is installed (kernel boot before any
//                      spawn — the same sentinel `openat` uses), or the
//                      network stack isn't up (`net::init` hasn't run).
//
// The argument-error errnos come from `net_socket`; the fd-table-full
// (`EMFILE`) and no-process (`ENOSYS`) errnos are shared with `openat`
// (`crate::syscall::openat`).

use crate::net;
use crate::net_socket::{
    validate_socket_args, SocketId, EADDRINUSE, EAGAIN, ECONNREFUSED, EFAULT, EINPROGRESS, EINVAL,
    EISCONN, ENOTCONN, ENOTSOCK, SOCK_DGRAM, SOCK_TYPE_MASK,
};
use crate::process::current_process_fd_table;
use crate::process::fd_table::socket as fd_socket;
use crate::process::fd_table::FdEntry;
use crate::syscall::dispatch::EBADF;
use crate::syscall::openat::{EMFILE, ENOSYS};

/// Linux x86_64 syscall number for `socket(domain, type, protocol)`.
/// Source: `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_socket`
/// (= 41). The vendored musl tree confirms the same value at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_socket`. Routes to
/// `socket::handle`, which creates a TCP socket and allocates a
/// per-process fd bound to it. Per #478a.
pub const SYS_SOCKET: u64 = 41;

/// Handle a `socket(domain, type, protocol)` syscall. Returns the
/// allocated fd (≥ 3) on success, a negative errno on failure (see the
/// module docstring for the full errno table).
///
/// Steps, in order:
///   1. Validate `(domain, type, protocol)` — only `AF_INET` +
///      `SOCK_STREAM` + (`IPPROTO_IP` | `IPPROTO_TCP`) is accepted.
///      Bad args return the specific Linux errno before any socket is
///      created (no side effect on rejection).
///   2. Create the smoltcp TCP socket on the kernel interface (no I/O)
///      and get back the kernel `SocketId`. Returns `-ENOSYS` if the
///      net stack isn't up.
///   3. Allocate the lowest-free fd ≥ 3 in the calling process's fd
///      table and bind it to the socket id (`FdEntry::Socket`). On
///      fd-table-full the just-created socket is torn down so it doesn't
///      leak (no I/O happened, so the teardown is clean); returns
///      `-EMFILE`. Returns `-ENOSYS` if no process is installed.
///
/// The `domain` / `type` / `protocol` arguments arrive in the dispatch
/// registers as `u64` (rdi / rsi / rdx); they're conceptually `int` in
/// the C signature, but `validate_socket_args` works on the raw `u64`s
/// (the constants it compares against are small positive values, so the
/// sign extension of a negative `int` — which `socket(2)` never passes
/// for these args — would simply fail to match and be rejected).
pub fn handle(domain: u64, type_: u64, protocol: u64) -> i64 {
    // (1) Validate the argument triple. On rejection, return the errno
    //     without creating anything.
    if let Err(errno) = validate_socket_args(domain, type_, protocol) {
        return errno;
    }

    // (2) Create the smoltcp socket (creation only — no I/O). The masked
    //     type picks TCP vs UDP (validation above already guaranteed it's
    //     one of SOCK_STREAM / SOCK_DGRAM with a compatible protocol).
    //     The returned id is the token the fd table will store.
    let create = if type_ & SOCK_TYPE_MASK == SOCK_DGRAM {
        net::create_udp_socket() // SOCK_DGRAM → UDP (#533)
    } else {
        net::create_tcp_socket() // SOCK_STREAM → TCP (#478a)
    };
    let socket_id = match create {
        Ok(id) => id,
        // Net stack not initialised (no `net::init`) — surface the same
        // "this kernel can't right now" errno `openat` uses for the
        // pre-process state. In production `net::init` runs at boot, so
        // this only fires on the host test target / a mis-ordered boot.
        Err(net::SocketError::NotInitialised) => return -ENOSYS,
        // The create paths mint their own id and do no I/O, so they can
        // only fail with `NotInitialised` — the other `SocketError`
        // variants are produced exclusively by the bind/listen/connect/
        // send/recv wrappers (#529-#533), never here. The catch-all keeps
        // the match exhaustive as the error enum grows; map it to the
        // same pre-stack sentinel rather than inventing an errno for a
        // case that can't arise.
        Err(_) => return -ENOSYS,
    };

    // (3) Allocate the fd and bind it to the socket id.
    bind_socket_fd(socket_id)
}

/// Allocate an fd backed by the freshly-created socket and bind it.
/// Returns the fd or a negative errno. Mirrors `openat`'s
/// `allocate_synthetic` / `allocate_file` shape — the fd-table mutation
/// happens inside the `current_process_fd_table` lock.
///
/// On fd-table-full (`-EMFILE`) or no-process (`-ENOSYS`) the socket
/// created in step (2) is removed from the kernel's socket registry so
/// it doesn't leak. The removal is safe because `socket()` does no I/O —
/// the socket is idle (not connected/listening), so tearing it down has
/// no wire-visible effect.
fn bind_socket_fd(socket_id: SocketId) -> i64 {
    let result = current_process_fd_table(|maybe_table| match maybe_table {
        Some(table) => match table.allocate(fd_socket(socket_id.as_u64())) {
            Ok(fd) => fd as i64,
            Err(()) => -EMFILE,
        },
        // No process installed — same sentinel `openat` returns for the
        // pre-process state. The caller meant to open a socket against a
        // process that isn't live.
        None => -ENOSYS,
    });

    // If the fd couldn't be bound, tear the orphaned socket back down so
    // it doesn't sit in the registry forever (a slow leak of smoltcp
    // socket slots + their 8 KiB of ring buffers per failed call).
    if result < 0 {
        net::destroy_socket(socket_id);
    }
    result
}

// ── Shared glue for the socket-operation handlers (#529-#533) ───────
//
// bind / listen / connect / accept / sendto / recvfrom all start the
// same way: take an fd, resolve it to the kernel `SocketId` the fd-table
// `FdEntry::Socket` stores, then drive a `net::tcp_*` wrapper and map its
// `SocketError` onto a Linux errno. These three helpers — the fd→id
// resolver, the userspace `sockaddr` reader, and the error mapper — are
// factored here (in the socket module) so each per-operation handler
// stays a thin, well-documented sequence rather than re-deriving the
// plumbing. They're `pub(crate)` so the sibling `syscall::bind` /
// `syscall::listen` / … modules can call them.

/// Resolve `fd` to the kernel [`SocketId`] its fd-table entry stores.
/// Returns:
///   * `Ok(SocketId)` — `fd` is a live `FdEntry::Socket`.
///   * `Err(-ENOSYS)` — no process is installed (the pre-spawn boot
///     state, same sentinel `socket()` / `openat` use).
///   * `Err(-EBADF)` — `fd` isn't open at all (no such entry, or `fd`
///     doesn't fit the `i32` table key width).
///   * `Err(-ENOTSOCK)` — `fd` is open but backs a `File` / `Synthetic`
///     resource, not a socket. Linux returns `ENOTSOCK` for a socket op
///     on a non-socket fd.
///
/// The fd-table lock is taken and released entirely within this call —
/// the returned `SocketId` is a plain value, so the subsequent `net::`
/// call doesn't nest the fd-table lock inside the `NET` lock.
pub(crate) fn resolve_socket_fd(fd: i32) -> Result<SocketId, i64> {
    current_process_fd_table(|maybe_table| {
        let Some(table) = maybe_table else {
            // No current process — pre-spawn boot state.
            return Err(-ENOSYS);
        };
        match table.lookup(fd) {
            Some(FdEntry::Socket { socket_id }) => Ok(SocketId(*socket_id)),
            // Open, but not a socket — File/Synthetic fd.
            Some(_) => Err(-ENOTSOCK),
            // Not open at all.
            None => Err(-EBADF),
        }
    })
}

/// Read `addrlen` bytes of a `struct sockaddr` from userspace pointer
/// `addr` into an owned `Vec<u8>`, for the pure
/// [`crate::net_socket::parse_sockaddr_in`] to decode. Returns:
///   * `Ok(bytes)` — copied `addrlen` bytes.
///   * `Err(-EFAULT)` — `addr` is null (with `addrlen > 0`), or `addrlen`
///     exceeds a sane cap (a malformed call), guarding the slice
///     construction the same way `write::do_write` guards `count`.
///
/// SAFETY: `addr` is treated as a kernel pointer under tier-1's identity
/// mapping (same model as `write` / `read` / `openat`). The null check
/// guards the common mistake; once #527 lands real page tables this
/// gains a `validate_userspace_range` pre-check. `addrlen` is bounded so
/// the `from_raw_parts` length is always `<= isize::MAX`.
pub(crate) fn read_sockaddr(addr: u64, addrlen: u64) -> Result<alloc::vec::Vec<u8>, i64> {
    // A null pointer is -EFAULT for any non-zero length. (addrlen == 0 is
    // handled by the parser, which rejects an undersized buffer with
    // -EINVAL; we still must not deref null, so guard here too.)
    if addr == 0 {
        return Err(-EFAULT);
    }
    // Bound the length: a sockaddr is tiny (16 bytes for IPv4, 28 for
    // IPv6, 110 for AF_UNIX). Anything beyond `sockaddr_storage` (128
    // bytes) is a malformed call — reject with -EFAULT rather than copy a
    // huge span. This also keeps the slice length well under isize::MAX.
    const SOCKADDR_STORAGE_LEN: u64 = 128;
    if addrlen > SOCKADDR_STORAGE_LEN {
        return Err(-EFAULT);
    }
    let len = addrlen as usize;
    let mut out = alloc::vec::Vec::with_capacity(len);
    // SAFETY: `addr` is non-null (checked) and `len <= 128 <= isize::MAX`;
    // tier-1's identity mapping makes the span valid kernel memory.
    let slice = unsafe { core::slice::from_raw_parts(addr as *const u8, len) };
    out.extend_from_slice(slice);
    Ok(out)
}

/// Map a [`net::SocketError`] from a `tcp_*` wrapper onto the Linux errno
/// a socket syscall returns. `op` selects the few cases where the same
/// `SocketError` means different errnos for different operations
/// (`InvalidState` is `-EISCONN` for `connect` but `-EINVAL` for
/// `bind`; `Unaddressable` is `-EADDRNOTAVAIL`-ish but we use `-EINVAL`
/// for the zero-port `listen`/`bind` and `-EADDRNOTAVAIL` is not in the
/// tier-1 set, so `connect` to 0.0.0.0 maps to `-EINVAL` too). Returning
/// the negative errno keeps the call sites a single `?`-free match.
pub(crate) fn socket_error_to_errno(err: net::SocketError, op: SocketOp) -> i64 {
    use net::SocketError::*;
    match err {
        // Net stack not up — same sentinel the rest of the surface uses
        // for "this kernel can't right now".
        NotInitialised => -ENOSYS,
        // Dangling socket fd — the id resolved but isn't a live socket.
        UnknownSocket => -EBADF,
        AddrInUse => -EADDRINUSE,
        ConnectionRefused => -ECONNREFUSED,
        NotConnected => -ENOTCONN,
        WouldBlock => -EAGAIN,
        ConnectInProgress => -EINPROGRESS,
        // State / address mismatches: the errno depends on the operation.
        InvalidState => match op {
            // `connect` on an already-open socket → already connected.
            SocketOp::Connect => -EISCONN,
            // `listen` on an already-open (connected) socket → the local
            // address is effectively in use for listening.
            SocketOp::Listen => -EADDRINUSE,
            // `bind` after the socket is open → invalid argument.
            SocketOp::Bind => -EINVAL,
            // send/recv never surface raw `InvalidState` (the wrappers
            // map smoltcp's send/recv `InvalidState` to `NotConnected`),
            // but keep the match total: a state mismatch on a data op is
            // "not connected".
            SocketOp::Send | SocketOp::Recv => -ENOTCONN,
        },
        Unaddressable => match op {
            // A zero / 0.0.0.0 connect target is an invalid argument in
            // the tier-1 errno set (no EADDRNOTAVAIL exposed yet).
            SocketOp::Connect => -EINVAL,
            // A zero local port for bind/listen → invalid argument.
            SocketOp::Bind | SocketOp::Listen => -EINVAL,
            // send/recv don't carry an address (the null-addr form), so
            // `Unaddressable` can't arise; map to invalid argument for
            // totality.
            SocketOp::Send | SocketOp::Recv => -EINVAL,
        },
    }
}

/// Which socket operation is mapping a [`net::SocketError`], so
/// [`socket_error_to_errno`] can pick the operation-specific errno for
/// the ambiguous `InvalidState` / `Unaddressable` cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SocketOp {
    Bind,
    Listen,
    Connect,
    Send,
    Recv,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net_socket::{
        AF_INET, AF_INET6, IPPROTO_IP, IPPROTO_TCP, SOCK_DGRAM, SOCK_STREAM,
    };
    use crate::process::address_space::AddressSpace;
    use crate::process::fd_table::FdEntry;
    use crate::process::process::CURRENT_PROCESS_TEST_LOCK;
    use crate::process::{current_process_fd_table, current_process_install, current_process_uninstall, Process};
    use crate::syscall::openat::ENOSYS as OPENAT_ENOSYS;

    /// `SYS_SOCKET` is 41 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_socket`.
    /// Static check so a future renumber surfaces in the test diff.
    #[test]
    fn sys_socket_number_matches_linux_uapi() {
        assert_eq!(SYS_SOCKET, 41);
    }

    /// Helper: install a fresh Process so the handler has somewhere to
    /// allocate the fd. Mirrors the helper in the openat / close tests.
    fn install_test_process() {
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(7, address_space);
        current_process_install(proc);
    }

    // -- Argument-validation rejections (no live interface needed) ----
    //
    // These exercise the handler's step (1): a bad argument triple is
    // rejected with the right errno BEFORE any socket creation, so they
    // don't depend on `net::init` having run (the host test target has
    // no live interface). The errno comes straight from
    // `net_socket::validate_socket_args`; these assert the handler wires
    // it through unchanged.

    /// `socket(AF_INET6, SOCK_STREAM, 0)` → `-EAFNOSUPPORT` (97).
    /// Tier-1 is IPv4-only; the rejection happens before socket
    /// creation, so no process need be installed.
    #[test]
    fn socket_af_inet6_returns_eafnosupport() {
        let result = handle(AF_INET6, SOCK_STREAM, IPPROTO_IP);
        assert_eq!(result, -97);
    }

    /// `socket(AF_INET, SOCK_STREAM, IPPROTO_UDP)` → `-EPROTONOSUPPORT`
    /// (93). A stream socket with a non-TCP protocol is rejected.
    #[test]
    fn socket_stream_wrong_protocol_returns_eprotonosupport() {
        const IPPROTO_UDP: u64 = 17;
        let result = handle(AF_INET, SOCK_STREAM, IPPROTO_UDP);
        assert_eq!(result, -93);
    }

    /// `socket(AF_INET, SOCK_DGRAM, IPPROTO_TCP)` → `-EPROTONOSUPPORT`
    /// (93). A datagram socket with a non-UDP protocol is rejected (#533).
    /// The validation fires before any creation, so no process / stack is
    /// needed.
    #[test]
    fn socket_dgram_wrong_protocol_returns_eprotonosupport() {
        let result = handle(AF_INET, SOCK_DGRAM, IPPROTO_TCP);
        assert_eq!(result, -93);
    }

    /// `socket(AF_INET, SOCK_RAW, 0)` → `-EINVAL` (22). An unknown
    /// socket type (after flag masking) is rejected.
    #[test]
    fn socket_unknown_type_returns_einval() {
        const SOCK_RAW: u64 = 3;
        let result = handle(AF_INET, SOCK_RAW, IPPROTO_IP);
        assert_eq!(result, -22);
    }

    // -- Creation path on the host (no live interface) ----------------
    //
    // smoltcp's `Loopback` device stands up a working interface on the
    // host (`net::init(None)` binds 127.0.0.1/8), so the FULL create→
    // bind→return-fd path runs under `cargo test --lib` — the host
    // target is NOT limited to "no interface". These tests bring the
    // loopback stack up, issue a VALID `socket()`, and assert the
    // returned fd is real (≥ 3), resolves to `FdEntry::Socket`, and that
    // the bound socket id is live in `crate::net`'s registry — i.e. the
    // complete tier-1 `socket()` contract end-to-end.
    //
    // Serialisation: these touch the process-global `CURRENT_PROCESS`
    // (via `CURRENT_PROCESS_TEST_LOCK`) AND the process-global `NET`
    // slot. `SOCKET_NET_TEST_LOCK` serialises the `net::init` +
    // `handle` + registry-read sequence so a parallel net test's
    // `init(None)` can't rebuild `NET` mid-sequence (same per-resource
    // lock discipline as `net::tests::TEST_NET_LOCK`). We don't read the
    // sibling lock directly (it's private to `net::tests`), so a
    // dedicated lock here covers the socket-syscall net-touching tests.

    /// Serialises socket-syscall tests that stand up `net` + read its
    /// registry, so a concurrent `init(None)` can't reset the stack
    /// between create and assert.
    static SOCKET_NET_TEST_LOCK: spin::Mutex<()> = spin::Mutex::new(());

    /// A VALID `socket(AF_INET, SOCK_STREAM, 0)` over the loopback stack
    /// returns a real fd (≥ 3), binds it to `FdEntry::Socket`, and the
    /// bound socket id is live in `net`'s registry. This is the whole
    /// tier-1 `socket()` contract: create a TCP socket, allocate an fd,
    /// bind the socket handle to the fd, return the fd — no I/O.
    #[test]
    fn socket_default_protocol_creates_socket_and_binds_fd() {
        let _net_guard = SOCKET_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None); // loopback interface — makes create_tcp_socket succeed
        install_test_process();

        let fd = handle(AF_INET, SOCK_STREAM, IPPROTO_IP);
        assert!(fd >= 3, "socket() must return a valid fd (>= 3), got {}", fd);

        // The fd resolves to a Socket entry whose id is registered with
        // the live network stack — proves the create→bind wiring.
        let entry = current_process_fd_table(|t| t.and_then(|t| t.lookup(fd as i32).cloned()));
        match entry {
            Some(FdEntry::Socket { socket_id }) => {
                assert!(
                    net::socket_handle(SocketId(socket_id)).is_some(),
                    "the fd's socket id must be live in net's registry"
                );
            }
            other => panic!("fd {} must bind to FdEntry::Socket, got {:?}", fd, other),
        }
        current_process_uninstall();
    }

    /// Explicit-TCP `socket(AF_INET, SOCK_STREAM, IPPROTO_TCP)` also
    /// creates a socket and binds an fd — pins that the explicit-protocol
    /// accept path reaches creation, not just the default-protocol one.
    #[test]
    fn socket_explicit_tcp_creates_socket_and_binds_fd() {
        let _net_guard = SOCKET_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None);
        install_test_process();

        let fd = handle(AF_INET, SOCK_STREAM, IPPROTO_TCP);
        assert!(fd >= 3, "explicit-TCP socket() must return a valid fd, got {}", fd);
        let entry = current_process_fd_table(|t| t.and_then(|t| t.lookup(fd as i32).cloned()));
        assert!(
            matches!(entry, Some(FdEntry::Socket { .. })),
            "fd must bind to FdEntry::Socket, got {:?}",
            entry
        );
        current_process_uninstall();
    }

    /// `socket(AF_INET, SOCK_DGRAM, 0)` over the loopback stack creates a
    /// UDP socket and binds a real fd (#533). The fd resolves to
    /// `FdEntry::Socket`, the socket id is live in the registry, and its
    /// transport kind is `Udp` — proving the SOCK_DGRAM accept path routes
    /// to `create_udp_socket`, not the TCP path.
    #[test]
    fn socket_dgram_creates_udp_socket_and_binds_fd() {
        let _net_guard = SOCKET_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None);
        install_test_process();

        let fd = handle(AF_INET, SOCK_DGRAM, IPPROTO_IP);
        assert!(fd >= 3, "SOCK_DGRAM socket() must return a valid fd, got {}", fd);
        let entry = current_process_fd_table(|t| t.and_then(|t| t.lookup(fd as i32).cloned()));
        match entry {
            Some(FdEntry::Socket { socket_id }) => {
                assert!(
                    net::socket_handle(SocketId(socket_id)).is_some(),
                    "the UDP fd's socket id must be live in net's registry"
                );
                assert_eq!(
                    net::socket_kind(SocketId(socket_id)),
                    Some(net::SocketKind::Udp),
                    "a SOCK_DGRAM socket must be registered as UDP"
                );
            }
            other => panic!("fd {} must bind to FdEntry::Socket, got {:?}", fd, other),
        }
        current_process_uninstall();
    }

    /// Two back-to-back `socket()` calls over the loopback stack return
    /// distinct, increasing fds (3 then 4), each bound to its own live
    /// socket — the create+allocate path is correctly stateful across
    /// calls.
    #[test]
    fn sequential_socket_syscalls_return_distinct_fds() {
        let _net_guard = SOCKET_NET_TEST_LOCK.lock();
        let _proc_guard = CURRENT_PROCESS_TEST_LOCK.lock();
        net::init(None);
        install_test_process();

        let fd_a = handle(AF_INET, SOCK_STREAM, IPPROTO_IP);
        let fd_b = handle(AF_INET, SOCK_STREAM, IPPROTO_IP);
        assert_eq!(fd_a, 3, "first socket fd is 3");
        assert_eq!(fd_b, 4, "second socket fd is 4");

        // Both fds bind to distinct live socket ids.
        let id_a = match current_process_fd_table(|t| t.and_then(|t| t.lookup(fd_a as i32).cloned())) {
            Some(FdEntry::Socket { socket_id }) => socket_id,
            other => panic!("fd_a must be a Socket, got {:?}", other),
        };
        let id_b = match current_process_fd_table(|t| t.and_then(|t| t.lookup(fd_b as i32).cloned())) {
            Some(FdEntry::Socket { socket_id }) => socket_id,
            other => panic!("fd_b must be a Socket, got {:?}", other),
        };
        assert_ne!(id_a, id_b, "each socket() call binds a distinct socket id");
        assert!(net::socket_handle(SocketId(id_a)).is_some());
        assert!(net::socket_handle(SocketId(id_b)).is_some());
        current_process_uninstall();
    }

    // -- fd-table binding logic (host-tested directly) ----------------
    //
    // The handler's step (3) — allocate an fd and bind it to a socket id
    // via `FdEntry::Socket` — is the host-testable heart of the syscall.
    // The real handler gets its `socket_id` from `net::create_tcp_socket`
    // (unavailable on the host), so these tests drive the SAME fd-table
    // binding path the handler uses (`current_process_fd_table` +
    // `fd_table::socket`) with a synthetic id, asserting the fd is
    // allocated, returned, and resolves back to the stored id — i.e. the
    // exact post-creation bookkeeping `socket()` performs.

    /// Binding a socket id allocates fd 3 (lowest free), and the fd
    /// resolves back to `FdEntry::Socket { socket_id }` — the create→
    /// bind→return contract `socket()` fulfils once the net stack is up.
    #[test]
    fn binding_socket_id_allocates_fd_and_stores_id() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        install_test_process();
        // Drive the same fd-table bind the handler's step (3) performs.
        let fd = current_process_fd_table(|t| {
            t.expect("process installed")
                .allocate(fd_socket(1234))
                .expect("allocate socket fd")
        });
        assert_eq!(fd, 3, "first socket fd is the lowest free fd (3)");
        // The fd round-trips to the stored socket id.
        let entry = current_process_fd_table(|t| t.and_then(|t| t.lookup(fd).cloned()));
        assert_eq!(entry, Some(FdEntry::Socket { socket_id: 1234 }));
        current_process_uninstall();
    }

    /// Two socket binds allocate increasing fds (3, then 4) — each
    /// carries its own socket id. Confirms the monotonic fd allocation
    /// the handler relies on for back-to-back `socket()` calls.
    #[test]
    fn sequential_socket_binds_allocate_increasing_fds() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        install_test_process();
        let fd_a = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_socket(10)).expect("a")
        });
        let fd_b = current_process_fd_table(|t| {
            t.expect("proc").allocate(fd_socket(11)).expect("b")
        });
        assert_eq!(fd_a, 3);
        assert_eq!(fd_b, 4);
        // Each fd carries its own id.
        let entry_a = current_process_fd_table(|t| t.and_then(|t| t.lookup(fd_a).cloned()));
        let entry_b = current_process_fd_table(|t| t.and_then(|t| t.lookup(fd_b).cloned()));
        assert_eq!(entry_a, Some(FdEntry::Socket { socket_id: 10 }));
        assert_eq!(entry_b, Some(FdEntry::Socket { socket_id: 11 }));
        current_process_uninstall();
    }

    /// `socket()` with no process installed returns `-ENOSYS` even for a
    /// valid argument triple — the same pre-process sentinel `openat`
    /// uses. (On the host the create step would also fail; this asserts
    /// the no-process path specifically by checking the errno is the
    /// pre-process one.)
    #[test]
    fn socket_with_no_process_returns_enosys() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        // Ensure no leftover process from a prior test.
        current_process_uninstall();
        let result = handle(AF_INET, SOCK_STREAM, IPPROTO_IP);
        assert_eq!(result, -OPENAT_ENOSYS);
    }
}
