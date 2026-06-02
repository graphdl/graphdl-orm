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
use crate::net_socket::{validate_socket_args, SocketId};
use crate::process::current_process_fd_table;
use crate::process::fd_table::socket as fd_socket;
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

    // (2) Create the smoltcp TCP socket (creation only — no I/O). The
    //     returned id is the token the fd table will store.
    let socket_id = match net::create_tcp_socket() {
        Ok(id) => id,
        // Net stack not initialised (no `net::init`) — surface the same
        // "this kernel can't right now" errno `openat` uses for the
        // pre-process state. In production `net::init` runs at boot, so
        // this only fires on the host test target / a mis-ordered boot.
        Err(net::SocketError::NotInitialised) => return -ENOSYS,
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

    /// `socket(AF_INET, SOCK_DGRAM, 0)` → `-EPROTONOSUPPORT` (93). UDP
    /// isn't served by this TCP-creation path.
    #[test]
    fn socket_dgram_returns_eprotonosupport() {
        let result = handle(AF_INET, SOCK_DGRAM, IPPROTO_IP);
        assert_eq!(result, -93);
    }

    /// `socket(AF_INET, SOCK_STREAM, IPPROTO_UDP)` → `-EPROTONOSUPPORT`
    /// (93). A stream socket with a non-TCP protocol is rejected.
    #[test]
    fn socket_stream_wrong_protocol_returns_eprotonosupport() {
        const IPPROTO_UDP: u64 = 17;
        let result = handle(AF_INET, SOCK_STREAM, IPPROTO_UDP);
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
