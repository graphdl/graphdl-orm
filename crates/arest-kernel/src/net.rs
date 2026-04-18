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
use smoltcp::socket::dhcpv4;
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr, Ipv4Cidr};
use spin::Mutex;

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
    changed
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
