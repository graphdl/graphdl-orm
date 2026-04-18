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
use smoltcp::iface::{Config, Interface, SocketSet};
use smoltcp::phy::{Loopback, Medium};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr};
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

    let sockets = SocketSet::new(Vec::new());

    *NET.lock() = Some(NetState { device, iface, sockets });
}

/// Drive the stack forward. Call from the idle loop or timer IRQ.
/// Returns true if any socket woke up (i.e. caller has work to do).
pub fn poll() -> bool {
    use smoltcp::iface::PollResult;
    let mut guard = NET.lock();
    let Some(state) = guard.as_mut() else { return false; };
    let NetState { device, iface, sockets } = state;
    matches!(iface.poll(now(), device, sockets), PollResult::SocketStateChanged)
}

/// Report whether the stack is initialised — used by the banner so
/// `net:` only prints when `init` has run.
pub fn is_online() -> bool {
    NET.lock().is_some()
}
