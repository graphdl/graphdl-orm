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

use super::GossipMsg;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use hashbrown::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;

/// Transport errors are intentionally coarse — gossip is lossy by
/// design and the state machine treats every send as best-effort.
#[derive(Debug)]
pub enum TransportError {
    /// Endpoint unknown to this in-memory registry. Production UDP
    /// transport never returns this — unreachable peers just drop.
    UnknownAddr(SocketAddr),
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
