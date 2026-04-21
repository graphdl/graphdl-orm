// crates/arest/src/cluster/transport.rs
//
// Network layer for SWIM gossip. The state machine in mod.rs is
// transport-agnostic — it produces GossipMsgs and expects them to
// arrive, it doesn't care whether the wire is UDP, TCP, or an
// in-process channel.
//
// Two impls live here:
//   - InMemTransport: a mailbox-per-addr registry shared across
//     endpoints. Used by unit tests so convergence /
//     failure-detection behavior is verifiable without binding
//     sockets.
//   - UdpTransport:   send_to/recv_from on a std::net::UdpSocket
//     with freeze-bytes on the wire. Landed in a later commit —
//     the InMem path is enough for the acceptance #1 / #2 tests.

#![cfg(all(feature = "cluster", not(feature = "no_std")))]

use super::{GossipMsg, decode, encode};
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use hashbrown::HashMap;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Transport errors are intentionally coarse — gossip is lossy by
/// design and the state machine treats every send as best-effort.
#[derive(Debug)]
pub enum TransportError {
    /// Endpoint unknown to this in-memory registry. Production UDP
    /// transport never returns this — unreachable peers just drop.
    UnknownAddr(SocketAddr),
    /// Underlying socket I/O failure. Gossipers treat this as a
    /// dropped packet; the caller does not retry.
    Io(String),
}

pub trait Transport: Send {
    /// Enqueue a message for `to`. Never blocks on response.
    fn send(&mut self, to: SocketAddr, msg: &GossipMsg) -> Result<(), TransportError>;

    /// Drain every pending inbound message without blocking.
    /// The returned `SocketAddr` is the sender.
    fn recv_nonblocking(&mut self) -> Vec<(SocketAddr, GossipMsg)>;
}

/// Shared in-memory network used by unit tests. Each endpoint has a
/// mailbox keyed by its listening address; `send` pushes to the
/// target's mailbox, `recv_nonblocking` drains the sender's own.
///
/// The registry is behind an `Arc<Mutex<...>>` so tests can hold
/// multiple endpoints simultaneously and still observe a consistent
/// message order across them.
pub struct InMemNet {
    mailboxes: Mutex<HashMap<SocketAddr, VecDeque<(SocketAddr, GossipMsg)>>>,
}

impl InMemNet {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { mailboxes: Mutex::new(HashMap::new()) })
    }

    /// Register an endpoint at `addr` and return a Transport handle
    /// tied to that address. Subsequent calls with the same addr
    /// overwrite the mailbox — useful for restart simulations.
    pub fn endpoint(self: &Arc<Self>, addr: SocketAddr) -> InMemTransport {
        self.mailboxes.lock().unwrap().insert(addr, VecDeque::new());
        InMemTransport { me: addr, net: Arc::clone(self) }
    }
}

pub struct InMemTransport {
    me: SocketAddr,
    net: Arc<InMemNet>,
}

impl InMemTransport {
    pub fn addr(&self) -> SocketAddr { self.me }
}

impl Transport for InMemTransport {
    fn send(&mut self, to: SocketAddr, msg: &GossipMsg) -> Result<(), TransportError> {
        let mut mbx = self.net.mailboxes.lock().unwrap();
        match mbx.get_mut(&to) {
            Some(q) => { q.push_back((self.me, msg.clone())); Ok(()) }
            None => Err(TransportError::UnknownAddr(to)),
        }
    }

    fn recv_nonblocking(&mut self) -> Vec<(SocketAddr, GossipMsg)> {
        let mut mbx = self.net.mailboxes.lock().unwrap();
        mbx.get_mut(&self.me)
            .map(|q| q.drain(..).collect())
            .unwrap_or_default()
    }
}

// ── UDP transport ───────────────────────────────────────────────────
//
// A real `std::net::UdpSocket` bound to a user-supplied address.
// Sends are best-effort `send_to`; a dedicated reader thread drains
// inbound packets, decodes them via `cluster::decode`, and pushes
// well-formed messages into an mpsc channel. `recv_nonblocking`
// drains that channel. Malformed packets are silently dropped — UDP
// is lossy by design and gossip tolerates arbitrary packet loss.
//
// Shutdown: `Drop` flips the stop flag; the reader's `recv_from` has
// a short read timeout so it wakes up, observes the flag, and
// returns. Tests depend on this — if the reader thread leaked, test
// processes would never terminate.

pub struct UdpTransport {
    me: SocketAddr,
    socket: UdpSocket,
    inbox: mpsc::Receiver<(SocketAddr, GossipMsg)>,
    stop: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
}

impl UdpTransport {
    /// Bind a UDP socket at `bind_addr` and start the reader thread.
    /// Pass a `0` port to let the OS pick — useful for tests that
    /// want to avoid port collisions; call `.addr()` to retrieve
    /// the resolved address.
    pub fn bind(bind_addr: SocketAddr) -> io::Result<Self> {
        let socket = UdpSocket::bind(bind_addr)?;
        // 50ms read timeout so the reader thread can observe the
        // stop flag without depending on a wake-on-drop of the
        // socket itself (some platforms don't unblock recv_from on
        // close from another thread).
        socket.set_read_timeout(Some(Duration::from_millis(50)))?;
        let me = socket.local_addr()?;
        let reader_socket = socket.try_clone()?;
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop);
        let reader = thread::Builder::new()
            .name(format!("cluster-udp-rx-{me}"))
            .spawn(move || udp_reader_loop(reader_socket, tx, stop_clone))?;
        Ok(Self { me, socket, inbox: rx, stop, reader: Some(reader) })
    }

    pub fn addr(&self) -> SocketAddr { self.me }
}

impl Drop for UdpTransport {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(r) = self.reader.take() {
            // The reader wakes up within 50ms of the stop flag
            // flipping; join is bounded and non-hanging.
            let _ = r.join();
        }
    }
}

fn udp_reader_loop(
    socket: UdpSocket,
    tx: mpsc::Sender<(SocketAddr, GossipMsg)>,
    stop: Arc<AtomicBool>,
) {
    // 65_507 is the maximum UDP payload over IPv4. SWIM piggyback
    // snapshots are tiny (a few deltas × ~80 bytes each) — the
    // buffer is oversized but one-shot allocation is cheap.
    let mut buf = [0u8; 65_507];
    while !stop.load(Ordering::Relaxed) {
        match socket.recv_from(&mut buf) {
            Ok((n, from)) => {
                if let Ok(msg) = decode(&buf[..n]) {
                    if tx.send((from, msg)).is_err() {
                        // Receiver dropped — Transport is gone, nothing to do.
                        break;
                    }
                }
                // Malformed packets drop silently.
            }
            Err(e)
                if e.kind() == io::ErrorKind::WouldBlock
                    || e.kind() == io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(_) => break,
        }
    }
}

impl Transport for UdpTransport {
    fn send(&mut self, to: SocketAddr, msg: &GossipMsg) -> Result<(), TransportError> {
        let bytes = encode(msg);
        self.socket
            .send_to(&bytes, to)
            .map(|_| ())
            .map_err(|e| TransportError::Io(e.to_string()))
    }

    fn recv_nonblocking(&mut self) -> Vec<(SocketAddr, GossipMsg)> {
        let mut out = Vec::new();
        while let Ok(m) = self.inbox.try_recv() { out.push(m); }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::State;

    fn loopback(port: u16) -> SocketAddr {
        format!("127.0.0.1:{port}").parse().unwrap()
    }

    /// Two real UDP sockets on loopback exchange one message.
    /// Proves the encode → send_to → recv_from → decode path works
    /// end-to-end, not just the InMem shortcut.
    #[test]
    fn udp_transport_roundtrip_over_loopback() {
        // Port 0 = let the OS pick. We discover the real port via .addr().
        let mut a = UdpTransport::bind(loopback(0)).expect("bind a");
        let mut b = UdpTransport::bind(loopback(0)).expect("bind b");
        let addr_a = a.addr();
        let addr_b = b.addr();

        let msg = GossipMsg::Ping {
            from: "alpha".into(),
            seq: 7,
            piggyback: vec![super::super::Delta {
                id: "beta".into(),
                addr: loopback(9000),
                incarnation: 1,
                state: State::Alive,
            }],
        };

        a.send(addr_b, &msg).expect("send a→b");

        // Wait up to 1s for the reader thread to deliver.
        let deadline = std::time::Instant::now() + Duration::from_millis(1000);
        let mut received = Vec::new();
        while std::time::Instant::now() < deadline && received.is_empty() {
            received = b.recv_nonblocking();
            if received.is_empty() {
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        assert_eq!(received.len(), 1, "b should have received one message");
        let (from, got) = &received[0];
        assert_eq!(*from, addr_a, "sender addr");
        assert_eq!(*got, msg, "roundtripped payload");
    }

    /// Stopping the Transport (via Drop) must not hang: the reader
    /// thread must honor the stop flag within the 50ms read timeout.
    #[test]
    fn udp_transport_drops_cleanly() {
        let before = std::time::Instant::now();
        {
            let _t = UdpTransport::bind(loopback(0)).expect("bind");
            // Drop happens at scope exit.
        }
        // 50ms read timeout + thread join overhead. 1s is a wide
        // ceiling that still catches a truly-hung thread.
        assert!(
            before.elapsed() < Duration::from_millis(1000),
            "Transport::drop took too long: {:?}",
            before.elapsed()
        );
    }
}

