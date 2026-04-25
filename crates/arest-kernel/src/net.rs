// crates/arest-kernel/src/net.rs
//
// Bare-metal TCP/IP stack via smoltcp (#261).
//
// AREST-as-OS doesn't reach for Linux's network stack — the kernel
// owns its own. smoltcp is the pure-Rust no_std stack that Redox
// and several unikernels already rely on, so the heavy lifting
// (packet parsing, TCP state machine, DHCP client, DNS, sockets)
// is vendored rather than re-written.
//
// This module is the surface the rest of the kernel sees. It hides
// smoltcp's internals and exposes four things:
//
//   init()            — create the interface + socket set at boot.
//   poll()            — drive the state machine (called from the
//                       timer IRQ / idle loop once the timer lands).
//   listen_tcp(port)  — bind a TCP listen socket and return a handle
//                       that the HTTP server (#264) reads / writes.
//   send_udp(...)     — bind a UDP socket for Doom multiplayer (#271).
//
// The backing `Device` trait impl is provided by the NIC driver.
// Until the virtio-net driver lands (#262) we plug in a `Loopback`
// device so smoltcp integration is verifiable end-to-end from day
// one — the HTTP server can be smoke-tested against `127.0.0.1`
// inside the kernel before any external packet flows exist.

use alloc::vec::Vec;
use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::phy::{self, Device, DeviceCapabilities, Loopback, Medium};
use smoltcp::socket::{dhcpv4, tcp};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr, Ipv4Cidr};
use spin::Mutex;

// `file_serve` / `file_upload` reach into `crate::block_storage`, which
// is `cfg(target_arch = "x86_64")`-gated (see main.rs L186-L197). The
// aarch64 + armv7 UEFI arms can compile `net` end-to-end without those
// helpers — `drive_http` simply skips the file_* intercept arms and
// falls straight through to the registered `Handler` chain. The cfg
// guards on the `try_serve` call sites in `drive_http` mirror this.
#[cfg(target_arch = "x86_64")]
use crate::file_serve::{self, ServeOutcome};
#[cfg(target_arch = "x86_64")]
use crate::file_upload::{self, ServeOutcome as UploadOutcome};
use crate::http;

// `VirtioPhy` source is arch-specific. On x86_64 we wrap the PCI-
// transport `crate::virtio::VirtioPhy` (BIOS + UEFI). On aarch64 +
// armv7 UEFI we wrap the MMIO-transport `crate::virtio_mmio::VirtioPhy`
// (#449's parallel adapter). The `KernelDevice::Virtio` arm threads
// through whichever flavour the cfg picks, so the rest of `net` is
// transport-agnostic.
#[cfg(target_arch = "x86_64")]
use crate::virtio::VirtioPhy;
#[cfg(all(target_os = "uefi", any(target_arch = "aarch64", target_arch = "arm")))]
use crate::virtio_mmio::VirtioPhy;

/// Monotonically-increasing timestamp for smoltcp's scheduler. Once
/// the TSC / HPET timer IRQ is wired we hand this a proper tick
/// counter; for now it's a raw counter bumped on every poll so the
/// TCP retransmit machinery at least progresses.
static MONOTONIC_MILLIS: spin::Mutex<i64> = spin::Mutex::new(0);

fn now() -> Instant {
    let mut t = MONOTONIC_MILLIS.lock();
    *t = t.saturating_add(1);
    Instant::from_millis(*t)
}

/// Global network state. `Option` so `init` can build it after the
/// heap is live. `Mutex` so every caller (IRQ, REPL, HTTP handler)
/// serialises on the same smoltcp instance.
static NET: Mutex<Option<NetState>> = Mutex::new(None);

/// smoltcp `phy::Device` backing the interface. Enum-dispatched so
/// the same NetState works regardless of whether the boot-time PCI
/// scan (#262) found a virtio-net NIC or not. Both variants
/// implement `phy::Device` by delegating through `KernelRxToken` /
/// `KernelTxToken`.
pub enum KernelDevice {
    /// Fallback path when the PCI scan finds no virtio-net. The
    /// interface still binds `127.0.0.1/8` so in-guest smoke tests
    /// (e.g. the http self_test) work without external packets.
    Loopback(Loopback),
    /// Real NIC — packets cross PCI into QEMU's user-mode NAT and
    /// on to the host via `-hostfwd=tcp::8080-:80` (#267).
    Virtio(VirtioPhy),
}

/// Token variants. One pair for each KernelDevice variant; the
/// phy::{RxToken,TxToken} impls below forward `consume` into the
/// inner smoltcp-native token.
pub enum KernelRxToken<'a> {
    Loopback(<Loopback as Device>::RxToken<'a>),
    #[cfg(target_arch = "x86_64")]
    Virtio(crate::virtio::VirtioRxToken<'a>),
    #[cfg(all(target_os = "uefi", any(target_arch = "aarch64", target_arch = "arm")))]
    Virtio(crate::virtio_mmio::VirtioRxToken<'a>),
}

pub enum KernelTxToken<'a> {
    Loopback(<Loopback as Device>::TxToken<'a>),
    #[cfg(target_arch = "x86_64")]
    Virtio(crate::virtio::VirtioTxToken<'a>),
    #[cfg(all(target_os = "uefi", any(target_arch = "aarch64", target_arch = "arm")))]
    Virtio(crate::virtio_mmio::VirtioTxToken<'a>),
}

impl<'a> phy::RxToken for KernelRxToken<'a> {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        match self {
            KernelRxToken::Loopback(t) => t.consume(f),
            KernelRxToken::Virtio(t) => t.consume(f),
        }
    }
}

impl<'a> phy::TxToken for KernelTxToken<'a> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        match self {
            KernelTxToken::Loopback(t) => t.consume(len, f),
            KernelTxToken::Virtio(t) => t.consume(len, f),
        }
    }
}

impl Device for KernelDevice {
    type RxToken<'a> = KernelRxToken<'a>;
    type TxToken<'a> = KernelTxToken<'a>;

    fn receive(&mut self, ts: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        match self {
            KernelDevice::Loopback(d) => d
                .receive(ts)
                .map(|(r, t)| (KernelRxToken::Loopback(r), KernelTxToken::Loopback(t))),
            KernelDevice::Virtio(d) => d
                .receive(ts)
                .map(|(r, t)| (KernelRxToken::Virtio(r), KernelTxToken::Virtio(t))),
        }
    }

    fn transmit(&mut self, ts: Instant) -> Option<Self::TxToken<'_>> {
        match self {
            KernelDevice::Loopback(d) => d.transmit(ts).map(KernelTxToken::Loopback),
            KernelDevice::Virtio(d) => d.transmit(ts).map(KernelTxToken::Virtio),
        }
    }

    fn capabilities(&self) -> DeviceCapabilities {
        match self {
            KernelDevice::Loopback(d) => d.capabilities(),
            KernelDevice::Virtio(d) => d.capabilities(),
        }
    }
}

struct NetState {
    /// The physical-layer device behind the interface — loopback by
    /// default, virtio-net when #262 discovered a real NIC.
    device: KernelDevice,
    iface: Interface,
    sockets: SocketSet<'static>,
    /// DHCPv4 client socket (#263). Registered at boot; `poll` picks
    /// up `Event::Configured` once the lease arrives and calls
    /// `apply_dhcp_config` to install the IP, netmask, and default
    /// gateway on the interface. Inactive over Loopback (no DHCP
    /// server), so `dhcp_lease()` returns None until virtio-net is
    /// live and a real DHCP server responds.
    dhcp_handle: SocketHandle,
    /// Cached lease info so the banner / status calls can report
    /// the assigned address without re-polling the socket.
    lease: Option<DhcpLease>,
    /// Registered HTTP listener (#264). `None` until `register_http`
    /// is called; single-listener for now because the kernel's only
    /// HTTP surface is the HATEOAS site. poll() drives it each tick.
    http_listener: Option<HttpListener>,
}

/// State for the kernel's single HTTP/1.1 listener (#264).
///
/// Owns the TCP socket handle, the handler fn, and the two byte
/// buffers we accumulate into between `poll`s:
///
///   rx_buf — bytes read off the socket but not yet enough to form
///            a complete request (headers + Content-Length body).
///   tx_buf — serialised response bytes; `tx_sent` advances through
///            them across polls so large responses handle smoltcp's
///            send-ring backpressure without dropping data.
///
/// Single connection at a time: after each response is fully written
/// we `close()`, the socket transitions to Closed, and the next poll
/// re-`listen`s on the same port.
struct HttpListener {
    handle: SocketHandle,
    handler: http::Handler,
    rx_buf: Vec<u8>,
    tx_buf: Vec<u8>,
    tx_sent: usize,
    port: u16,
}

/// Snapshot of a DHCPv4 lease. Populated when the DHCP socket
/// transitions to the Configured state. Cleared on Deconfigured.
#[derive(Debug, Clone)]
pub struct DhcpLease {
    pub address: Ipv4Cidr,
    pub router: Option<smoltcp::wire::Ipv4Address>,
    pub dns_servers: Vec<smoltcp::wire::Ipv4Address>,
}

/// Initialise the network stack. When `virtio` is Some the
/// interface binds the real NIC's MAC and talks over virtio-net;
/// otherwise it falls back to a loopback device at `127.0.0.1/8`
/// so in-guest smoke tests still run without packet flow.
///
/// Called once from `kernel_main` after `virtio::try_init_virtio_net`
/// has probed PCI.
pub fn init(virtio: Option<VirtioPhy>) {
    let (mut device, mac) = match virtio {
        Some(phy) => {
            let mac = phy.mac_address();
            (KernelDevice::Virtio(phy), mac)
        }
        None => {
            // Loopback needs a fake MAC — smoltcp only uses it to frame
            // Ethernet headers internally, and nothing on the wire cares.
            let mac = EthernetAddress([0x02, 0x00, 0x00, 0x00, 0x00, 0x01]);
            (KernelDevice::Loopback(Loopback::new(Medium::Ethernet)), mac)
        }
    };

    let config = Config::new(HardwareAddress::Ethernet(mac));
    let mut iface = Interface::new(config, &mut device, now());

    // On loopback we can statically assign 127.0.0.1/8 right away.
    // On virtio-net we leave the address empty — DHCP fills it in
    // on the first successful lease (Configured event in `poll`).
    if matches!(device, KernelDevice::Loopback(_)) {
        iface.update_ip_addrs(|addrs| {
            addrs
                .push(IpCidr::new(IpAddress::v4(127, 0, 0, 1), 8))
                .expect("loopback address push");
        });
    }

    let mut sockets = SocketSet::new(Vec::new());

    // DHCPv4 client socket (#263). Registered here so that on the
    // virtio-net path, `poll` DISCOVER / REQUESTs a lease automatically
    // — no extra wiring at the call site. Over Loopback the socket
    // simply times out and retries; harmless.
    let dhcp_socket = dhcpv4::Socket::new();
    let dhcp_handle = sockets.add(dhcp_socket);

    *NET.lock() = Some(NetState {
        device,
        iface,
        sockets,
        dhcp_handle,
        lease: None,
        http_listener: None,
    });
}

/// Register the kernel's HTTP listener on `port`. Adds a TCP listen
/// socket to the socket set and stores the handler fn pointer so
/// `poll` can dispatch every accepted request through it.
///
/// Panics if `net::init` has not run or if smoltcp rejects the
/// listen call (port out of range / already bound). Only one
/// listener is supported — a second call replaces the first.
///
/// The rx/tx buffers are 4 KiB each: enough for a hand-written
/// HTTP/1.1 request line + common headers, and any small HATEOAS
/// JSON response our handler returns. Responses larger than 4 KiB
/// stream across multiple polls via `drive_http`'s `tx_sent` cursor
/// so the ring can drain without dropping data.
pub fn register_http(port: u16, handler: http::Handler) {
    let mut guard = NET.lock();
    let state = guard.as_mut().expect("net::init() not called");

    let mut rx = Vec::new();
    rx.resize(4096, 0u8);
    let mut tx = Vec::new();
    tx.resize(4096, 0u8);
    let rx_buffer = tcp::SocketBuffer::new(rx);
    let tx_buffer = tcp::SocketBuffer::new(tx);
    let mut socket = tcp::Socket::new(rx_buffer, tx_buffer);
    socket.listen(port).expect("tcp listen");

    let handle = state.sockets.add(socket);
    state.http_listener = Some(HttpListener {
        handle,
        handler,
        rx_buf: Vec::new(),
        tx_buf: Vec::new(),
        tx_sent: 0,
        port,
    });
}

/// Drive the stack forward. Call from the idle loop or timer IRQ.
/// Returns true if any socket woke up (i.e. caller has work to do).
///
/// Side effect: if the DHCPv4 socket transitioned to Configured or
/// Deconfigured since the last poll, the interface's IP address,
/// gateway, and DNS list are updated in place.
pub fn poll() -> bool {
    use smoltcp::iface::PollResult;
    let mut guard = NET.lock();
    let Some(state) = guard.as_mut() else { return false; };
    let changed = matches!(
        state.iface.poll(now(), &mut state.device, &mut state.sockets),
        PollResult::SocketStateChanged,
    );

    // Drain DHCP events every poll, regardless of whether socket
    // state "changed" — smoltcp reports on a different axis.
    let dhcp = state.sockets.get_mut::<dhcpv4::Socket>(state.dhcp_handle);
    if let Some(event) = dhcp.poll() {
        match event {
            dhcpv4::Event::Configured(config) => {
                state.lease = Some(DhcpLease {
                    address: config.address,
                    router: config.router,
                    dns_servers: config.dns_servers.iter().copied().collect(),
                });
                state.iface.update_ip_addrs(|addrs| {
                    addrs.clear();
                    let _ = addrs.push(IpCidr::Ipv4(config.address));
                });
                if let Some(router) = config.router {
                    let _ = state.iface.routes_mut()
                        .add_default_ipv4_route(router);
                } else {
                    state.iface.routes_mut().remove_default_ipv4_route();
                }
            }
            dhcpv4::Event::Deconfigured => {
                state.lease = None;
                state.iface.update_ip_addrs(|addrs| addrs.clear());
                state.iface.routes_mut().remove_default_ipv4_route();
            }
        }
    }

    // Advance the HTTP listener: read any pending request bytes,
    // dispatch through the handler when a full request has arrived,
    // and stream the response back until the socket has drained.
    if let Some(listener) = state.http_listener.as_mut() {
        drive_http(listener, &mut state.sockets);
    }

    changed
}

/// One tick of the HTTP listener state machine.
///
/// Four states, checked in order:
///
///   1. Socket is Closed — re-`listen` on the listener's port so the
///      next client can connect. Clear any leftover buffers from the
///      previous connection.
///   2. A response is in flight (`tx_sent < tx_buf.len()`) — push as
///      many remaining bytes as the send ring accepts; when the whole
///      response has been written, `close()` the socket so the client
///      sees EOF and the listener re-arms on the next poll.
///   3. Request bytes are arriving — `recv` whatever smoltcp has for
///      us and append to `rx_buf`.
///   4. A full request has been accumulated — parse it, call the
///      handler, serialise the response into `tx_buf`, and try an
///      immediate send to minimise round-trip latency.
fn drive_http(listener: &mut HttpListener, sockets: &mut SocketSet<'static>) {
    use smoltcp::socket::tcp::State;
    let socket = sockets.get_mut::<tcp::Socket>(listener.handle);

    // (1) Closed → re-arm listen.
    if socket.state() == State::Closed {
        let _ = socket.listen(listener.port);
        listener.rx_buf.clear();
        listener.tx_buf.clear();
        listener.tx_sent = 0;
        return;
    }

    // (2) Drain any in-flight response before accepting new input.
    if listener.tx_sent < listener.tx_buf.len() {
        if socket.can_send() {
            let remaining = &listener.tx_buf[listener.tx_sent..];
            if let Ok(n) = socket.send_slice(remaining) {
                listener.tx_sent += n;
            }
        }
        if listener.tx_sent >= listener.tx_buf.len() {
            socket.close();
        }
        return;
    }

    // (3) Accumulate request bytes.
    if socket.can_recv() {
        let _ = socket.recv(|chunk| {
            listener.rx_buf.extend_from_slice(chunk);
            (chunk.len(), ())
        });
    }

    if listener.rx_buf.is_empty() {
        return;
    }

    // (4) Try to parse; on success dispatch handler and queue response.
    //
    // Five dispatch arms, checked in order:
    //   a. `/file/{id}/content` (#403) — handled by `file_serve::try_serve`,
    //      which produces fully-serialised HTTP/1.1 wire bytes (200/206/
    //      404/405/416/500). Bypasses the `Handler` chain because the
    //      response carries a dynamic `Content-Type` (sourced from the
    //      File's `mime_type`) and may need `Content-Range` headers, both
    //      of which the static-`Content-Type` `http::Response` can't
    //      express.
    //   b. `POST /file` (#444) — handled by `file_upload::try_serve`,
    //      the write-side sibling of (a). Re-scans the raw rx_buf for
    //      the `Content-Type` header (multipart boundary lives there);
    //      the canonical `http::parse_request` doesn't capture it.
    //      Same wire-bytes-bypass story as (a) since 201 Created carries
    //      a dynamic `Location` header that the static `Response`
    //      builder can't emit. Now also handles chunked-mode init when
    //      the multipart body declares a `total` form field instead of
    //      shipping a `file` part.
    //   c. `PUT /file/{id}/chunk?offset=N` (#445) —
    //      `file_upload::try_serve_chunk`. Streams region-backed bytes
    //      onto the disk slot allocated at chunked-init time. Also
    //      reads the optional `Content-Range` header off the raw
    //      buffer (canonical `parse_request` ignores it).
    //   d. `GET /file/{id}/upload-state` (#445) —
    //      `file_upload::try_serve_upload_state`. Resume probe; reports
    //      the upload's highest contiguous byte for client-side resume.
    //   e. Anything else — falls through to the registered `Handler` fn,
    //      which serialises via `Response::to_wire()`.
    let parsed = match http::parse_request(&listener.rx_buf) {
        Ok(Some(req)) => Ok(req),
        Ok(None) => return, // need more bytes
        Err(msg) => Err(msg),
    };
    let wire = match parsed {
        Ok(req) => dispatch_request(&req, &listener.rx_buf, listener.handler),
        Err(msg) => http::Response::bad_request(msg).to_wire(),
    };
    listener.tx_buf = wire;
    listener.tx_sent = 0;
    listener.rx_buf.clear();

    // Opportunistic first send so the fast path finishes in one tick.
    if socket.can_send() {
        if let Ok(n) = socket.send_slice(&listener.tx_buf) {
            listener.tx_sent = n;
        }
    }
    if listener.tx_sent >= listener.tx_buf.len() {
        socket.close();
    }
}

/// Route one parsed request through the file-* intercept arms (when
/// available) before falling back to the registered `Handler` chain.
///
/// On x86_64 (BIOS + UEFI) the intercept arms cover the file_serve +
/// file_upload routes that need wire-byte responses (dynamic
/// Content-Type, Content-Range, Location headers) the static-typed
/// `http::Response` builder can't emit. On aarch64 + armv7 UEFI those
/// modules are gated out (see main.rs L186-L197 — they reach
/// `crate::block_storage` which is x86_64-only), so `dispatch_request`
/// shrinks to a direct handler call. Generic routes (`/api/*`, the
/// HATEOAS site, the SPA fallback) reach the `Handler` chain on every
/// arch.
#[cfg(target_arch = "x86_64")]
fn dispatch_request(
    req: &http::Request,
    rx_buf: &[u8],
    handler: http::Handler,
) -> alloc::vec::Vec<u8> {
    let range = file_serve::extract_range_header(rx_buf);
    match file_serve::try_serve(
        &req.method,
        &req.path,
        range.as_deref(),
        crate::system::state(),
    ) {
        ServeOutcome::Response(bytes) => bytes,
        ServeOutcome::NotApplicable => {
            let ct = file_upload::extract_content_type_header(rx_buf);
            // #453: extract the optional Idempotency-Key header once
            // and thread it through both upload entry points so a
            // retried POST /file or PUT /file/{id}/chunk returns the
            // cached response instead of duplicating work (per #446).
            let idem = file_upload::extract_idempotency_key_header(rx_buf);
            match file_upload::try_serve_idempotent(
                &req.method,
                &req.path,
                ct.as_deref(),
                &req.body,
                crate::system::state(),
                idem.as_deref(),
            ) {
                UploadOutcome::Response(bytes) => bytes,
                UploadOutcome::NotApplicable => {
                    // PUT /file/{id}/chunk?offset=N — second
                    // half of the resumable-upload protocol
                    // (#445).
                    let cr = file_upload::extract_content_range_header(rx_buf);
                    match file_upload::try_serve_chunk_idempotent(
                        &req.method,
                        &req.path,
                        &req.body,
                        cr.as_deref(),
                        idem.as_deref(),
                    ) {
                        UploadOutcome::Response(bytes) => bytes,
                        UploadOutcome::NotApplicable => {
                            // GET /file/{id}/upload-state — the
                            // resume probe (#445).
                            match file_upload::try_serve_upload_state(
                                &req.method,
                                &req.path,
                                crate::system::state(),
                            ) {
                                UploadOutcome::Response(bytes) => bytes,
                                UploadOutcome::NotApplicable => {
                                    handler(req).to_wire()
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(not(target_arch = "x86_64"))]
fn dispatch_request(
    req: &http::Request,
    _rx_buf: &[u8],
    handler: http::Handler,
) -> alloc::vec::Vec<u8> {
    handler(req).to_wire()
}

/// Most recent DHCP lease (address + gateway + DNS). `None` until
/// a real NIC comes up and a DHCP server responds.
pub fn dhcp_lease() -> Option<DhcpLease> {
    NET.lock().as_ref().and_then(|s| s.lease.clone())
}

/// Report whether the stack is initialised — used by the banner so
/// `net:` only prints when `init` has run.
pub fn is_online() -> bool {
    NET.lock().is_some()
}
