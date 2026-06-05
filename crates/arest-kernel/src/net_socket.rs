// crates/arest-kernel/src/net_socket.rs
//
// Host-testable decision logic for the `socket(2)` syscall (#478a —
// `socket(AF_INET, SOCK_STREAM)`: TCP socket creation + fd allocation).
//
// Why a separate, unconditional module
// ------------------------------------
// The kernel's smoltcp surface (`crate::net`) and the syscall dispatch
// (`crate::syscall`) both compile on the host test target, but the
// pieces that actually *touch* a live smoltcp interface are gated
// `#[cfg(all(target_os = "uefi", ...))]` — `net::create_tcp_socket`
// reaches the global `SocketSet`, which only exists after `net::init`
// runs at boot. That gated code can't run under `cargo test --lib`.
//
// So we split the `socket()` syscall the same way #972 split the
// virtio-tablet glue (`linuxkpi_virtio_tablet.rs`) and the way the
// `openat` / `read` handlers split their pure logic from the hardware
// fill: the *decisions* live here in a `pub mod` that compiles
// unconditionally and is unit-tested directly on the host, while the
// thin gated wrapper in `crate::net` does only the smoltcp socket
// creation + registry insert.
//
// Two host-testable concerns live here:
//
//   1. Argument validation — `validate_socket_args(domain, type_,
//      protocol)` maps the three `socket(2)` arguments onto either
//      `Ok(())` (this is a socket the kernel can create) or a negative
//      Linux errno (`-EAFNOSUPPORT` / `-EINVAL` / `-EPROTONOSUPPORT`).
//      Tier-1 supports exactly `AF_INET` + `SOCK_STREAM` + (`IPPROTO_IP`
//      default | `IPPROTO_TCP`); everything else is rejected with the
//      errno Linux returns for that case. Pure function — no state.
//
//   2. Socket-id bookkeeping — `SocketIdAllocator` hands out unique,
//      monotonically-increasing `SocketId`s. `crate::net` keeps a
//      registry mapping each `SocketId` to the smoltcp `SocketHandle`
//      the interface owns; the per-process fd table
//      (`process::fd_table::FdEntry::Socket { socket_id }`) stores the
//      id, NOT the smoltcp handle, so the fd-table module stays
//      smoltcp-free. The id is the stable cross-module token: an fd
//      resolves to a `SocketId`, the registry resolves the `SocketId`
//      to a live socket. We mint a kernel id rather than reuse
//      smoltcp's `SocketHandle(usize)` because that handle's inner
//      `usize` is private (no public accessor / constructor in
//      smoltcp 0.12), so it can't round-trip through the fd table; a
//      kernel-owned monotonic id sidesteps that and gives us a single
//      definition of "socket identity" the host tests can exercise
//      without a live interface.
//
// Linux ABI constant provenance
// -----------------------------
// The numeric values are the Linux x86_64 uapi values shared by musl,
// glibc, and the kernel:
//   * `AF_INET   = 2`   `<bits/socket.h>` (`vendor/musl/include/
//                       arpa/inet.h` pulls `sys/socket.h`)
//   * `AF_INET6  = 10`  same header (rejected — tier-1 is IPv4-only)
//   * `SOCK_STREAM = 1` `<bits/socket_type.h>`
//   * `SOCK_DGRAM  = 2` same header (rejected here — the UDP path is
//                       `net::udp_bind`, not the tier-1 `socket()`
//                       TCP surface)
//   * `IPPROTO_IP  = 0` `<netinet/in.h>` (the "default protocol for
//                       this (domain,type)" sentinel libc passes)
//   * `IPPROTO_TCP = 6` `<netinet/in.h>`
// Errno values from `<asm-generic/errno.h>`:
//   * `EAFNOSUPPORT  = 97` address family not supported
//   * `EPROTONOSUPPORT = 93` protocol not supported for this type
//   * `EINVAL        = 22` unknown socket type
//
// What this module does NOT do (intentionally — #478a scope is "create
// the socket + allocate the fd, no I/O"):
//   * No `connect` / `bind` / `listen` / `accept` — those are later
//     slices that look the `SocketId` up in `net`'s registry.
//   * No `SOCK_NONBLOCK` / `SOCK_CLOEXEC` flag handling — Linux ORs
//     those into the `type` argument; tier-1 masks them off (see
//     `SOCK_TYPE_MASK`) and ignores them. The fd-table `Socket` entry
//     grows a flags field when those land.
//   * No id recycling — the allocator never reuses an id even after the
//     socket closes. A `u64` monotonic counter can't realistically wrap
//     in a kernel's lifetime (2^64 socket() calls), so a free-list would
//     be complexity with no payoff; if a closed socket's id were reused
//     while a stale fd still referenced it, the fd would silently alias a
//     different socket — monotonic ids make that class of bug
//     impossible.

#![allow(dead_code)]

/// Linux `AF_INET` — IPv4 address family. The only address family
/// tier-1's `socket()` supports (the smoltcp interface binds IPv4).
/// Per `<bits/socket.h>`.
pub const AF_INET: u64 = 2;

/// Linux `AF_INET6` — IPv6 address family. Recognised so the handler
/// can return the *correct* errno (`-EAFNOSUPPORT`) rather than a
/// generic one, but not supported: tier-1's stack is IPv4-only
/// (smoltcp is built with `proto-ipv4` but not `proto-ipv6`).
/// Per `<bits/socket.h>`.
pub const AF_INET6: u64 = 10;

/// Linux `SOCK_STREAM` — a sequenced, reliable, two-way connection-
/// based byte stream, i.e. TCP for the `AF_INET` family. The only
/// socket type tier-1's `socket()` creates. Per `<bits/socket_type.h>`.
pub const SOCK_STREAM: u64 = 1;

/// Linux `SOCK_DGRAM` — a connectionless, unreliable datagram, i.e.
/// UDP for `AF_INET`. Recognised here so a `socket(AF_INET, SOCK_DGRAM)`
/// returns `-EPROTONOSUPPORT` (a sharp "this kernel can't make a UDP
/// socket through this path" rather than a vague `-EINVAL`). The
/// kernel's actual UDP surface is `net::udp_bind` (the Doom-multiplayer
/// scaffold), wired separately from the tier-1 `socket()` TCP path.
/// Per `<bits/socket_type.h>`.
pub const SOCK_DGRAM: u64 = 2;

/// Mask isolating the socket *type* from the `SOCK_NONBLOCK` /
/// `SOCK_CLOEXEC` flag bits Linux ORs into the `type` argument of
/// `socket(2)`. `SOCK_NONBLOCK = 0o4000` and `SOCK_CLOEXEC = 0o2000`
/// live well above the low type bits, so masking with `0xFF` recovers
/// the bare type. Tier-1 masks the flags off and ignores them (the
/// fd-table `Socket` entry has no flags field yet); applying the mask
/// here means `socket(AF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0)` is
/// accepted as a plain stream socket rather than rejected as an unknown
/// type. Per `<bits/socket_type.h>` (`SOCK_TYPE_MASK = 0xf` in the
/// kernel; we use `0xFF` to be tolerant of the full low byte, which
/// still excludes both flag bits).
pub const SOCK_TYPE_MASK: u64 = 0xFF;

/// Linux `IPPROTO_IP` (= 0) — the "default protocol for this
/// (domain, type) pair" sentinel. libc's `socket(AF_INET, SOCK_STREAM,
/// 0)` passes 0, and the kernel picks TCP. Per `<netinet/in.h>`.
pub const IPPROTO_IP: u64 = 0;

/// Linux `IPPROTO_TCP` (= 6) — explicit TCP. Accepted alongside the
/// `IPPROTO_IP` default for a `SOCK_STREAM` socket. Per `<netinet/in.h>`.
pub const IPPROTO_TCP: u64 = 6;

/// Linux errno `EAFNOSUPPORT` (= 97) — "Address family not supported by
/// protocol". Returned for any `domain` other than `AF_INET`. Per
/// `<asm-generic/errno.h>`.
pub const EAFNOSUPPORT: i64 = 97;

/// Linux errno `EPROTONOSUPPORT` (= 93) — "Protocol not supported".
/// Returned when the `protocol` argument is incompatible with the
/// (supported) (domain, type) — e.g. a non-TCP protocol on a
/// `SOCK_STREAM`, or `SOCK_DGRAM` (whose protocol family this path
/// doesn't serve). Per `<asm-generic/errno.h>`.
pub const EPROTONOSUPPORT: i64 = 93;

/// Linux errno `EINVAL` (= 22) — "Invalid argument". Returned when the
/// socket *type* itself is unrecognised (neither `SOCK_STREAM` nor
/// `SOCK_DGRAM` after masking the flag bits). Mirrors Linux, which
/// returns `-EINVAL` for an unknown type. Per `<asm-generic/errno.h>`.
pub const EINVAL: i64 = 22;

/// Linux errno `EFAULT` (= 14) — "Bad address". Returned when a
/// pointer argument (the `sockaddr` an `addrlen` claims is `n` bytes
/// long, the data buffer of send/recv) can't be dereferenced — null,
/// or — once #527 lands real page tables — outside the process's
/// address space. The `bind` / `connect` / `sendto` / `recvfrom`
/// handlers reject a null `sockaddr` pointer with this. Per
/// `<asm-generic/errno-base.h>`.
pub const EFAULT: i64 = 14;

/// Linux errno `ENOTSOCK` (= 88) — "Socket operation on non-socket".
/// Returned by `bind` / `listen` / `connect` / `accept` / `sendto` /
/// `recvfrom` when the fd resolves to a non-socket fd-table entry
/// (a `File` / `Synthetic` fd). The fd is open, just not a socket. Per
/// `<asm-generic/errno.h>`.
pub const ENOTSOCK: i64 = 88;

/// Linux errno `EOPNOTSUPP` (= 95) — "Operation not supported". Returned
/// by `listen` / `accept` when invoked on a socket that doesn't support
/// the operation — tier-1 only models TCP, so this is reserved for a
/// future `listen` on a UDP socket (#533) and similar mismatches. Per
/// `<asm-generic/errno.h>`.
pub const EOPNOTSUPP: i64 = 95;

/// Linux errno `EADDRINUSE` (= 98) — "Address already in use". Returned
/// by `bind` / `listen` when smoltcp refuses the local endpoint because
/// the port is already taken (or the socket was already open with a
/// different endpoint). Per `<asm-generic/errno.h>`.
pub const EADDRINUSE: i64 = 98;

/// Linux errno `EISCONN` (= 106) — "Transport endpoint is already
/// connected". Returned by `connect` when the socket is already
/// connected (or a `connect`/`listen` was already issued on it — smoltcp
/// reports `InvalidState` for "socket already open"). Per
/// `<asm-generic/errno.h>`.
pub const EISCONN: i64 = 106;

/// Linux errno `ENOTCONN` (= 107) — "Transport endpoint is not
/// connected". Returned by `send` / `recv` (the null-addr `sendto` /
/// `recvfrom` form) when the TCP socket has no established connection,
/// and by `getpeername`-shaped queries on an unconnected socket. Per
/// `<asm-generic/errno.h>`.
pub const ENOTCONN: i64 = 107;

/// Linux errno `ECONNREFUSED` (= 111) — "Connection refused". Returned by
/// `connect` when the peer actively refuses (RST to our SYN) and by a
/// `recv` that observes the connection was reset. Per
/// `<asm-generic/errno.h>`.
pub const ECONNREFUSED: i64 = 111;

/// Linux errno `EAGAIN` (= 11) — "Resource temporarily unavailable"
/// (a.k.a. `EWOULDBLOCK`, the same value on Linux x86_64). Returned by a
/// non-blocking `connect` still in progress (Linux uses `EINPROGRESS`
/// there, 115 — see [`EINPROGRESS`]), by a `send` whose tx ring is full,
/// and by a `recv` with no bytes pending on a still-open connection.
/// Tier-1's sockets are inherently non-blocking (there's no scheduler to
/// park on — #530), so this is the "try again on the next poll" signal.
/// Per `<asm-generic/errno-base.h>`.
pub const EAGAIN: i64 = 11;

/// Linux errno `EINPROGRESS` (= 115) — "Operation now in progress".
/// Returned by a non-blocking `connect` that has issued the SYN but not
/// yet completed the handshake — the canonical "connection is being
/// established, poll for writability" signal libc expects from a
/// non-blocking socket. Tier-1 sockets are inherently non-blocking, so
/// `connect` returns this immediately after kicking off the handshake.
/// Per `<asm-generic/errno.h>`.
pub const EINPROGRESS: i64 = 115;

/// Length in bytes of a Linux `struct sockaddr_in` (IPv4 socket
/// address). The ABI layout (`<netinet/in.h>` / `<linux/in.h>`):
///   * `sin_family`  `u16` host byte order — offset 0
///   * `sin_port`    `u16` network (big-endian) byte order — offset 2
///   * `sin_addr`    `u32` network (big-endian) byte order — offset 4
///   * `sin_zero`    8 bytes padding — offset 8
/// Total 16 bytes. `bind` / `connect` / `sendto` accept an `addrlen`
/// that must be *at least* this (Linux tolerates a longer buffer and
/// reads only the first 16 bytes); a shorter one is `-EINVAL`.
pub const SOCKADDR_IN_LEN: usize = 16;

/// A parsed Linux `struct sockaddr_in` (IPv4). The wire form carries
/// `sin_port` and `sin_addr` in network (big-endian) byte order; this
/// struct holds them already decoded to host integers, so the syscall
/// handlers and the gated smoltcp wrappers work in plain host values.
///
/// Pure data — no smoltcp types — so the parse/validate logic is
/// host-unit-testable without a live interface, exactly like
/// `validate_socket_args`. The gated `net` wrappers convert
/// `SockAddrIn` into smoltcp's `IpEndpoint` at the call boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SockAddrIn {
    /// `sin_port` decoded from network byte order to a host `u16`.
    /// A bound/connect to port 0 has special meaning (ephemeral / "any")
    /// that the gated wrapper handles; the parser passes it through.
    pub port: u16,
    /// `sin_addr` decoded from network byte order to a host `u32` — the
    /// IPv4 address as `(a << 24) | (b << 16) | (c << 8) | d` for
    /// `a.b.c.d`. `0.0.0.0` (INADDR_ANY) is passed through; the wrapper
    /// interprets it (bind-to-any vs an invalid connect target).
    pub addr: u32,
}

impl SockAddrIn {
    /// The four IPv4 octets in `a.b.c.d` order, recovered from the
    /// host-order `addr` word. Handy for building smoltcp's
    /// `Ipv4Address::new(a, b, c, d)` in the gated wrappers and for
    /// assertions in the host tests.
    pub fn octets(self) -> [u8; 4] {
        self.addr.to_be_bytes()
    }
}

/// Parse + validate a Linux `struct sockaddr_in` out of the raw bytes a
/// `bind` / `connect` / `sendto` syscall copied from userspace. Pure
/// function — no global state, no smoltcp — so it's exhaustively
/// host-unit-testable, the same split `validate_socket_args` follows.
///
/// `addrlen` is the length the caller passed alongside the pointer.
/// Linux reads exactly the first `sizeof(struct sockaddr_in)` bytes and
/// tolerates a *longer* buffer (libc often passes `sizeof(struct
/// sockaddr_storage)`), so we require `addr.len() >= SOCKADDR_IN_LEN`
/// rather than `==`.
///
/// Returns the decoded [`SockAddrIn`] (port + addr in host byte order)
/// or `Err(-errno)` with the Linux errno for the specific rejection:
///
///   * `addr.len() < SOCKADDR_IN_LEN`           → `-EINVAL`
///       (the buffer is too short to hold a `sockaddr_in`)
///   * `sin_family != AF_INET`                  → `-EAFNOSUPPORT`
///       (tier-1 is IPv4-only — a `sockaddr_in6` or `AF_UNIX` address is
///       the wrong family for these sockets)
///
/// The byte layout is decoded explicitly (no `transmute`) so the parse
/// is endianness-correct on any host: `sin_family` is host-order (read
/// as native-endian `u16` from offset 0 — the family is the same small
/// integer on LE and BE because libc writes it in host order), while
/// `sin_port` (offset 2) and `sin_addr` (offset 4) are network
/// (big-endian) order and are decoded with `from_be_bytes`.
pub fn parse_sockaddr_in(addr: &[u8]) -> Result<SockAddrIn, i64> {
    // The buffer must be large enough to hold a full sockaddr_in. A
    // shorter buffer can't carry the family + port + addr the kernel
    // needs — Linux returns EINVAL for an undersized addrlen.
    if addr.len() < SOCKADDR_IN_LEN {
        return Err(-EINVAL);
    }

    // sin_family — host byte order (libc writes the family in native
    // endianness). Read the low 2 bytes as a native-endian u16. Only
    // AF_INET is supported; an IPv6 / unix address is the wrong family.
    let family = u16::from_ne_bytes([addr[0], addr[1]]) as u64;
    if family != AF_INET {
        return Err(-EAFNOSUPPORT);
    }

    // sin_port — network (big-endian) byte order, offset 2.
    let port = u16::from_be_bytes([addr[2], addr[3]]);

    // sin_addr — network (big-endian) byte order, offset 4. Decoded to a
    // host u32 so `a.b.c.d` is `(a<<24)|(b<<16)|(c<<8)|d`.
    let addr_word = u32::from_be_bytes([addr[4], addr[5], addr[6], addr[7]]);

    Ok(SockAddrIn {
        port,
        addr: addr_word,
    })
}

/// Validate a parsed [`SockAddrIn`] as a `connect(2)` *destination*
/// (#531). A connect target must name a concrete peer:
///
///   * `addr == 0` (INADDR_ANY, `0.0.0.0`)  → `-EINVAL`
///       — `0.0.0.0` is "any local address", not a routable destination;
///       you can't open a TCP connection *to* the wildcard. (Linux
///       returns `EADDRNOTAVAIL` here; tier-1's errno set doesn't expose
///       that, so `-EINVAL` is the closest "this address is wrong for a
///       connect" signal — the same errno the gated wrapper's
///       `Unaddressable` case maps to for connect.)
///   * `port == 0`                          → `-EINVAL`
///       — port 0 is the "pick any port" sentinel for bind, never a
///       valid connect target; smoltcp also rejects it.
///
/// `Ok(())` otherwise — the address + port name a concrete peer the
/// gated `net::tcp_connect` can hand to smoltcp. Pure function, no
/// state, host-unit-tested: lifts the connect-target sanity check out of
/// the gated wrapper so the accept/reject decision is verifiable without
/// a live interface (the same split `validate_socket_args` follows).
pub fn validate_connect_target(addr: &SockAddrIn) -> Result<(), i64> {
    if addr.addr == 0 {
        return Err(-EINVAL);
    }
    if addr.port == 0 {
        return Err(-EINVAL);
    }
    Ok(())
}

/// Validate the `(domain, type_, protocol)` triple a `socket(2)` call
/// carries. Returns `Ok(())` when tier-1 can create the socket
/// (`AF_INET` + `SOCK_STREAM` + (`IPPROTO_IP` | `IPPROTO_TCP`)), or
/// `Err(-errno)` with the Linux errno for the specific rejection:
///
///   * `domain != AF_INET`                  → `-EAFNOSUPPORT`
///   * type (masked) not a known socket type → `-EINVAL`
///   * type is `SOCK_DGRAM`                  → `-EPROTONOSUPPORT`
///       (UDP goes through `net::udp_bind`, not this TCP path)
///   * type is `SOCK_STREAM` but protocol is
///     neither `IPPROTO_IP` nor `IPPROTO_TCP` → `-EPROTONOSUPPORT`
///
/// The `type_` argument is masked with `SOCK_TYPE_MASK` first so a
/// `SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC` request is accepted as a
/// plain stream socket (the flag bits are recognised but ignored in
/// tier-1 — there's no per-socket flag state yet).
///
/// Pure function — no global state, no allocation. This is the whole of
/// the `socket()` argument-acceptance decision, lifted out of the gated
/// syscall wrapper so it can be exhaustively unit-tested on the host.
pub fn validate_socket_args(domain: u64, type_: u64, protocol: u64) -> Result<(), i64> {
    // Address family first: tier-1 is IPv4-only.
    if domain != AF_INET {
        return Err(-EAFNOSUPPORT);
    }

    // Strip the SOCK_NONBLOCK / SOCK_CLOEXEC flag bits before matching
    // the bare socket type.
    let sock_type = type_ & SOCK_TYPE_MASK;
    match sock_type {
        SOCK_STREAM => {
            // TCP. Accept the libc default (IPPROTO_IP == 0, "pick the
            // default protocol for this type") and an explicit
            // IPPROTO_TCP; reject anything else.
            if protocol == IPPROTO_IP || protocol == IPPROTO_TCP {
                Ok(())
            } else {
                Err(-EPROTONOSUPPORT)
            }
        }
        // UDP is a recognised type but not served by this path — the
        // kernel's UDP surface is `net::udp_bind`. Return the protocol-
        // not-supported errno so libc sees a definite "no" for this
        // (domain, type) rather than a generic invalid-argument.
        SOCK_DGRAM => Err(-EPROTONOSUPPORT),
        // Any other type value is not a socket type Linux defines (after
        // the flag bits are masked off) — invalid argument.
        _ => Err(-EINVAL),
    }
}

/// A kernel-assigned socket identity. Wraps a `u64` rather than reusing
/// smoltcp's `SocketHandle` because that handle's inner `usize` is
/// private in smoltcp 0.12 (no public accessor or constructor), so it
/// can't be stored in / reconstructed from the per-process fd table.
/// The `SocketId` is the stable token: the fd table stores it
/// (`FdEntry::Socket { socket_id }`), and `crate::net`'s registry maps
/// it to the live smoltcp `SocketHandle`.
///
/// `Copy` + `Eq` + `Ord` + `Hash` so it can be a `BTreeMap` key in the
/// registry and compared freely in the fd-table entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SocketId(pub u64);

impl SocketId {
    /// The raw `u64` token — what the fd-table `FdEntry::Socket` stores
    /// and what the registry keys on.
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

/// Monotonic allocator for `SocketId`s. Hands out unique, strictly-
/// increasing ids; never recycles (see module docstring — a `u64`
/// counter can't realistically wrap, and monotonic ids make stale-fd
/// aliasing impossible).
///
/// `crate::net` owns one of these inside its `NetState` (or alongside
/// the socket registry). The allocator is split out here, separate from
/// the smoltcp-bearing `net` state, precisely so the "ids are unique +
/// monotonic" contract can be unit-tested on the host without a live
/// interface.
#[derive(Debug, Default)]
pub struct SocketIdAllocator {
    /// The id the *next* `allocate` call will return. Starts at 0;
    /// advances by 1 each allocation. `u64` so it never wraps in
    /// practice.
    next: u64,
}

impl SocketIdAllocator {
    /// A fresh allocator whose first `allocate` returns `SocketId(0)`.
    pub const fn new() -> Self {
        Self { next: 0 }
    }

    /// Hand out the next unique `SocketId` and advance the counter.
    /// Each call returns a value strictly greater than every previous
    /// return from the same allocator.
    pub fn allocate(&mut self) -> SocketId {
        let id = SocketId(self.next);
        // saturating_add so an (impossible) overflow pins at u64::MAX
        // rather than wrapping to 0 and colliding with a live id. At one
        // socket() per nanosecond this still takes ~585 years to reach.
        self.next = self.next.saturating_add(1);
        id
    }

    /// The id the next `allocate` will return — for assertions / debug.
    pub fn peek_next(&self) -> u64 {
        self.next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Linux ABI constant value pins -------------------------------
    //
    // Static checks so a careless renumber surfaces in the test diff,
    // the same convention `syscall::dispatch` uses for its SYS_* /
    // errno constants.

    #[test]
    fn af_inet_value_matches_linux_uapi() {
        assert_eq!(AF_INET, 2);
    }

    #[test]
    fn af_inet6_value_matches_linux_uapi() {
        assert_eq!(AF_INET6, 10);
    }

    #[test]
    fn sock_stream_value_matches_linux_uapi() {
        assert_eq!(SOCK_STREAM, 1);
    }

    #[test]
    fn sock_dgram_value_matches_linux_uapi() {
        assert_eq!(SOCK_DGRAM, 2);
    }

    #[test]
    fn ipproto_constants_match_linux_uapi() {
        assert_eq!(IPPROTO_IP, 0);
        assert_eq!(IPPROTO_TCP, 6);
    }

    #[test]
    fn errno_constants_match_linux_uapi() {
        assert_eq!(EAFNOSUPPORT, 97);
        assert_eq!(EPROTONOSUPPORT, 93);
        assert_eq!(EINVAL, 22);
    }

    /// The networking errnos added for the bind/listen/connect/send/recv
    /// cluster match the Linux x86_64 uapi values (`<asm-generic/
    /// errno.h>` + `errno-base.h>`). Static pins so a renumber surfaces
    /// in the test diff.
    #[test]
    fn socket_op_errno_constants_match_linux_uapi() {
        assert_eq!(EFAULT, 14);
        assert_eq!(EAGAIN, 11);
        assert_eq!(ENOTSOCK, 88);
        assert_eq!(EOPNOTSUPP, 95);
        assert_eq!(EADDRINUSE, 98);
        assert_eq!(EISCONN, 106);
        assert_eq!(ENOTCONN, 107);
        assert_eq!(ECONNREFUSED, 111);
        assert_eq!(EINPROGRESS, 115);
    }

    // -- parse_sockaddr_in: the accept path ---------------------------

    /// A well-formed `sockaddr_in` for `1.2.3.4:80` parses to the
    /// host-order port + addr. Port 80 = `0x0050` big-endian = bytes
    /// `[0x00, 0x50]`; addr `1.2.3.4` = bytes `[1, 2, 3, 4]` big-endian.
    #[test]
    fn parse_sockaddr_in_accepts_well_formed_ipv4() {
        let mut buf = [0u8; SOCKADDR_IN_LEN];
        buf[0..2].copy_from_slice(&(AF_INET as u16).to_ne_bytes()); // family
        buf[2..4].copy_from_slice(&80u16.to_be_bytes()); // port 80, net order
        buf[4..8].copy_from_slice(&[1, 2, 3, 4]); // addr 1.2.3.4
        let parsed = parse_sockaddr_in(&buf).expect("well-formed sockaddr_in");
        assert_eq!(parsed.port, 80);
        assert_eq!(parsed.octets(), [1, 2, 3, 4]);
        assert_eq!(parsed.addr, 0x0102_0304);
    }

    /// Port decoding is byte-order correct: `0x1F90` = 8080. Big-endian
    /// on the wire is `[0x1F, 0x90]`; the parser must yield 8080, not the
    /// byte-swapped `0x901F`.
    #[test]
    fn parse_sockaddr_in_decodes_port_from_network_order() {
        let mut buf = [0u8; SOCKADDR_IN_LEN];
        buf[0..2].copy_from_slice(&(AF_INET as u16).to_ne_bytes());
        buf[2..4].copy_from_slice(&[0x1F, 0x90]); // 8080 big-endian
        buf[4..8].copy_from_slice(&[127, 0, 0, 1]);
        let parsed = parse_sockaddr_in(&buf).expect("parse");
        assert_eq!(parsed.port, 8080);
        assert_eq!(parsed.octets(), [127, 0, 0, 1]);
    }

    /// A buffer LONGER than `sockaddr_in` (e.g. `sockaddr_storage`, which
    /// libc commonly passes) is accepted — only the first 16 bytes are
    /// read, matching Linux's "reads sizeof(sockaddr_in), ignores the
    /// rest" behaviour.
    #[test]
    fn parse_sockaddr_in_accepts_oversized_buffer() {
        let mut buf = [0u8; 128]; // sockaddr_storage-sized
        buf[0..2].copy_from_slice(&(AF_INET as u16).to_ne_bytes());
        buf[2..4].copy_from_slice(&443u16.to_be_bytes());
        buf[4..8].copy_from_slice(&[10, 0, 0, 2]);
        let parsed = parse_sockaddr_in(&buf).expect("oversized ok");
        assert_eq!(parsed.port, 443);
        assert_eq!(parsed.octets(), [10, 0, 0, 2]);
    }

    /// `INADDR_ANY` (0.0.0.0) + port 0 parses cleanly — the parser
    /// passes the wildcard through; the gated wrapper decides what
    /// "any address / ephemeral port" means per-operation.
    #[test]
    fn parse_sockaddr_in_passes_wildcard_through() {
        let mut buf = [0u8; SOCKADDR_IN_LEN];
        buf[0..2].copy_from_slice(&(AF_INET as u16).to_ne_bytes());
        // port 0, addr 0.0.0.0 (all zero) — already zeroed.
        let parsed = parse_sockaddr_in(&buf).expect("wildcard ok");
        assert_eq!(parsed.port, 0);
        assert_eq!(parsed.addr, 0);
        assert_eq!(parsed.octets(), [0, 0, 0, 0]);
    }

    // -- parse_sockaddr_in: the reject paths --------------------------

    /// A buffer shorter than `sockaddr_in` (here 15 bytes) is rejected
    /// with `-EINVAL` — it can't hold the family + port + addr.
    #[test]
    fn parse_sockaddr_in_rejects_short_buffer_with_einval() {
        let buf = [0u8; SOCKADDR_IN_LEN - 1];
        assert_eq!(parse_sockaddr_in(&buf), Err(-EINVAL));
    }

    /// An empty buffer (addrlen 0) is `-EINVAL`.
    #[test]
    fn parse_sockaddr_in_rejects_empty_buffer_with_einval() {
        assert_eq!(parse_sockaddr_in(&[]), Err(-EINVAL));
    }

    /// A `sockaddr_in6` (sin6_family = AF_INET6 = 10) in a 16-byte buffer
    /// is rejected with `-EAFNOSUPPORT` — tier-1's sockets are IPv4-only.
    #[test]
    fn parse_sockaddr_in_rejects_af_inet6_with_eafnosupport() {
        let mut buf = [0u8; SOCKADDR_IN_LEN];
        buf[0..2].copy_from_slice(&(AF_INET6 as u16).to_ne_bytes());
        assert_eq!(parse_sockaddr_in(&buf), Err(-EAFNOSUPPORT));
    }

    /// The length check fires before the family check: a too-short
    /// buffer is `-EINVAL` even if its (truncated) family bytes happen to
    /// read as AF_INET. Pins the validation order (Linux checks the
    /// length first).
    #[test]
    fn parse_sockaddr_in_length_check_precedes_family_check() {
        // 4 bytes: family looks like AF_INET, but the buffer is far too
        // short to be a sockaddr_in.
        let mut buf = [0u8; 4];
        buf[0..2].copy_from_slice(&(AF_INET as u16).to_ne_bytes());
        assert_eq!(parse_sockaddr_in(&buf), Err(-EINVAL));
    }

    // -- validate_connect_target (#531) -------------------------------

    /// A concrete `1.2.3.4:80` target is accepted as a connect
    /// destination.
    #[test]
    fn validate_connect_target_accepts_concrete_peer() {
        let sa = SockAddrIn { addr: 0x0102_0304, port: 80 };
        assert_eq!(validate_connect_target(&sa), Ok(()));
    }

    /// `127.0.0.1:8080` (loopback) is a valid connect target.
    #[test]
    fn validate_connect_target_accepts_loopback() {
        let sa = SockAddrIn { addr: 0x7f00_0001, port: 8080 };
        assert_eq!(validate_connect_target(&sa), Ok(()));
    }

    /// A `0.0.0.0:80` target is rejected with `-EINVAL` — the wildcard
    /// address isn't a routable connect destination.
    #[test]
    fn validate_connect_target_rejects_wildcard_addr() {
        let sa = SockAddrIn { addr: 0, port: 80 };
        assert_eq!(validate_connect_target(&sa), Err(-EINVAL));
    }

    /// A `1.2.3.4:0` target is rejected with `-EINVAL` — port 0 is never
    /// a valid connect destination.
    #[test]
    fn validate_connect_target_rejects_zero_port() {
        let sa = SockAddrIn { addr: 0x0102_0304, port: 0 };
        assert_eq!(validate_connect_target(&sa), Err(-EINVAL));
    }

    // -- validate_socket_args: the accept path -----------------------

    /// `socket(AF_INET, SOCK_STREAM, 0)` — the canonical libc call (TCP,
    /// default protocol) — is accepted.
    #[test]
    fn validate_accepts_af_inet_stream_default_protocol() {
        assert_eq!(validate_socket_args(AF_INET, SOCK_STREAM, IPPROTO_IP), Ok(()));
    }

    /// `socket(AF_INET, SOCK_STREAM, IPPROTO_TCP)` — explicit TCP — is
    /// accepted.
    #[test]
    fn validate_accepts_af_inet_stream_explicit_tcp() {
        assert_eq!(validate_socket_args(AF_INET, SOCK_STREAM, IPPROTO_TCP), Ok(()));
    }

    /// `SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC` is accepted as a
    /// plain stream socket — the flag bits are masked off and ignored in
    /// tier-1. `SOCK_NONBLOCK = 0o4000`, `SOCK_CLOEXEC = 0o2000`.
    #[test]
    fn validate_accepts_stream_with_nonblock_and_cloexec_flags() {
        const SOCK_NONBLOCK: u64 = 0o4000;
        const SOCK_CLOEXEC: u64 = 0o2000;
        let type_ = SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC;
        assert_eq!(validate_socket_args(AF_INET, type_, IPPROTO_IP), Ok(()));
    }

    // -- validate_socket_args: the reject paths ----------------------

    /// A non-IPv4 address family (here `AF_INET6`) returns
    /// `-EAFNOSUPPORT` — tier-1's stack is IPv4-only.
    #[test]
    fn validate_rejects_af_inet6_with_eafnosupport() {
        assert_eq!(
            validate_socket_args(AF_INET6, SOCK_STREAM, IPPROTO_IP),
            Err(-EAFNOSUPPORT)
        );
    }

    /// `AF_UNIX` (1) — another common family — also returns
    /// `-EAFNOSUPPORT`. The check is "is it AF_INET", so every other
    /// family falls here.
    #[test]
    fn validate_rejects_af_unix_with_eafnosupport() {
        const AF_UNIX: u64 = 1;
        assert_eq!(
            validate_socket_args(AF_UNIX, SOCK_STREAM, IPPROTO_IP),
            Err(-EAFNOSUPPORT)
        );
    }

    /// The address-family check fires before the type / protocol checks:
    /// a bogus type on a bad family still surfaces as `-EAFNOSUPPORT`
    /// (the family is the first thing Linux validates).
    #[test]
    fn validate_family_check_precedes_type_check() {
        assert_eq!(
            validate_socket_args(0xDEAD, 0xBEEF, 0xF00D),
            Err(-EAFNOSUPPORT)
        );
    }

    /// `socket(AF_INET, SOCK_DGRAM, 0)` returns `-EPROTONOSUPPORT` — UDP
    /// is a recognised type but isn't served by the tier-1 `socket()`
    /// TCP path (the kernel's UDP surface is `net::udp_bind`).
    #[test]
    fn validate_rejects_dgram_with_eprotonosupport() {
        assert_eq!(
            validate_socket_args(AF_INET, SOCK_DGRAM, IPPROTO_IP),
            Err(-EPROTONOSUPPORT)
        );
    }

    /// An `AF_INET` + `SOCK_STREAM` socket with a nonsense protocol
    /// (neither IPPROTO_IP nor IPPROTO_TCP) returns `-EPROTONOSUPPORT`.
    /// Here `IPPROTO_UDP = 17` — valid protocol number, wrong for a
    /// stream socket.
    #[test]
    fn validate_rejects_stream_with_wrong_protocol() {
        const IPPROTO_UDP: u64 = 17;
        assert_eq!(
            validate_socket_args(AF_INET, SOCK_STREAM, IPPROTO_UDP),
            Err(-EPROTONOSUPPORT)
        );
    }

    /// An unrecognised socket type (after flag-masking) returns
    /// `-EINVAL`. `SOCK_RAW = 3` is a real Linux type but not one tier-1
    /// models, so it lands in the unknown-type arm.
    #[test]
    fn validate_rejects_unknown_type_with_einval() {
        const SOCK_RAW: u64 = 3;
        assert_eq!(
            validate_socket_args(AF_INET, SOCK_RAW, IPPROTO_IP),
            Err(-EINVAL)
        );
    }

    // -- SocketId / SocketIdAllocator --------------------------------

    /// A fresh allocator's first id is `SocketId(0)`.
    #[test]
    fn allocator_first_id_is_zero() {
        let mut a = SocketIdAllocator::new();
        assert_eq!(a.allocate(), SocketId(0));
    }

    /// Sequential allocations are strictly increasing and unique:
    /// 0, 1, 2, 3, ...
    #[test]
    fn allocator_ids_are_monotonic_and_unique() {
        let mut a = SocketIdAllocator::new();
        let ids: alloc::vec::Vec<SocketId> = (0..8).map(|_| a.allocate()).collect();
        // Strictly increasing.
        for pair in ids.windows(2) {
            assert!(pair[1].as_u64() > pair[0].as_u64(), "ids must strictly increase");
        }
        // The exact sequence.
        for (i, id) in ids.iter().enumerate() {
            assert_eq!(id.as_u64(), i as u64);
        }
    }

    /// Ids are never recycled: even across many allocations the values
    /// keep climbing — there's no free-list, so a closed socket's id is
    /// not handed back out (prevents stale-fd aliasing).
    #[test]
    fn allocator_does_not_recycle_ids() {
        let mut a = SocketIdAllocator::new();
        let first = a.allocate();
        let second = a.allocate();
        let third = a.allocate();
        assert_ne!(first, second);
        assert_ne!(second, third);
        assert_ne!(first, third);
        // peek_next reflects the count of allocations made so far.
        assert_eq!(a.peek_next(), 3);
    }

    /// Two independent allocators each start their own sequence at 0 —
    /// the counter is per-allocator state, not a process global. (The
    /// kernel keeps exactly one, but the type carries no hidden static.)
    #[test]
    fn independent_allocators_have_independent_sequences() {
        let mut a = SocketIdAllocator::new();
        let mut b = SocketIdAllocator::new();
        assert_eq!(a.allocate(), SocketId(0));
        assert_eq!(a.allocate(), SocketId(1));
        // `b` is untouched — its first id is still 0.
        assert_eq!(b.allocate(), SocketId(0));
    }

    /// `SocketId::as_u64` round-trips the wrapped value.
    #[test]
    fn socket_id_as_u64_round_trips() {
        assert_eq!(SocketId(12345).as_u64(), 12345);
    }
}
