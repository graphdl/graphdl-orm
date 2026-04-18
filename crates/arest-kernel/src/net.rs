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
use smoltcp::phy::{Loopback, Medium};
use smoltcp::socket::{dhcpv4, tcp};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr, Ipv4Cidr};
use spin::Mutex;

use crate::http;

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

struct NetState {
    /// Loopback device until #262 swaps in virtio-net. smoltcp
    /// drives its tx/rx over a `VecDeque<Vec<u8>>` inside Loopback,
    /// so packets originated by the kernel's HTTP server bounce
    /// right back through the interface's ingress path.
    device: Loopback,
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

/// Initialise the network stack with a loopback interface at
/// `127.0.0.1/8`. Replaced by a virtio-net-backed interface in
/// the NIC driver task (#262).
pub fn init() {
    let mut device = Loopback::new(Medium::Ethernet);

    // Loopback needs a fake MAC — smoltcp only uses it to frame
    // Ethernet headers internally, and nothing on the wire cares.
    let mac = EthernetAddress([0x02, 0x00, 0x00, 0x00, 0x00, 0x01]);
    let config = Config::new(HardwareAddress::Ethernet(mac));

    let mut iface = Interface::new(config, &mut device, now());
    iface.update_ip_addrs(|addrs| {
        addrs
            .push(IpCidr::new(IpAddress::v4(127, 0, 0, 1), 8))
            .expect("loopback address push");
    });

    let mut sockets = SocketSet::new(Vec::new());

    // DHCPv4 client socket (#263). Registered here so that the
    // moment a real Ethernet NIC (#262) drops in, `poll` will
    // DISCOVER/REQUEST a lease automatically — no extra wiring
    // needed at the call site. Over Loopback the socket simply
    // times out and retries; harmless.
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
    let resp = match http::parse_request(&listener.rx_buf) {
        Ok(Some(req)) => (listener.handler)(&req),
        Ok(None) => return, // need more bytes
        Err(msg) => http::Response::bad_request(msg),
    };
    listener.tx_buf = resp.to_wire();
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
