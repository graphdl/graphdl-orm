// crates/arest/src/cluster/mod.rs
//
// SWIM-style gossip membership (Cluster-1).
//
// Each node holds a Membership view mapping NodeId → NodeMeta
// (addr, incarnation, state). The Gossiper runs on a T_gossip ≈ 1s
// tick: pick a random peer, PING, expect ACK within T_ack. On
// timeout, ask K=3 other peers to PING-REQ; if none respond, mark
// Suspect, schedule Dead after T_suspect. Piggyback recent
// membership deltas on every PING/ACK.
//
// Joining: new node dials a bootstrap address, sends JOIN, receives
// the cluster's current snapshot, starts gossiping.
//
// Leaving (graceful): mark self Left, gossip that state, exit.
//
// The Membership type here is pure state — no time, no I/O. Timing
// concerns (suspect deadlines, gossip cadence) live in the Gossiper
// together with a Clock abstraction so tests drive synthetic time.

#![cfg(all(feature = "cluster", not(feature = "no_std")))]

pub mod transport;

use alloc::sync::Arc;
use hashbrown::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use transport::Transport;

/// Human-readable node identifier. Not a newtype yet — the SWIM paper
/// is agnostic about how IDs are minted; arest-cli uses the listening
/// socket's string form, which is stable across restarts at a fixed
/// bind address.
pub type NodeId = String;

/// Liveness state per SWIM. Ordered by "worseness" so same-incarnation
/// merges can resolve by `max(existing, incoming)` — Dead beats Left
/// beats Suspect beats Alive. The Left/Dead distinction matters for
/// placement (Cluster-2): a Left node drained cleanly and should not
/// have its tenants redistributed until manual re-add, whereas a Dead
/// node's tenants need immediate reassignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum State {
    Alive = 0,
    Suspect = 1,
    Left = 2,
    Dead = 3,
}

/// Per-node metadata in the membership view. `addr` is where this
/// node listens for gossip; `incarnation` is a monotonic generation
/// counter the owning node bumps when it refutes a stale Suspect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeMeta {
    pub addr: SocketAddr,
    pub incarnation: u64,
    pub state: State,
}

/// A single wire update — what one node tells another about some
/// node's current meta. Carried on the wire inside Ping/Ack/JoinAck.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delta {
    pub id: NodeId,
    pub addr: SocketAddr,
    pub incarnation: u64,
    pub state: State,
}

/// Eventually-consistent membership view. Holds self + every peer
/// we've ever been told about (including Dead / Left ones — the
/// corpses stick around so late-arriving deltas about them don't
/// resurrect as Alive).
pub struct Membership {
    self_id: NodeId,
    members: HashMap<NodeId, NodeMeta>,
}

impl Membership {
    /// Create a view containing only self at incarnation 0, Alive.
    pub fn new(self_id: NodeId, self_addr: SocketAddr) -> Self {
        let mut members = HashMap::new();
        members.insert(
            self_id.clone(),
            NodeMeta { addr: self_addr, incarnation: 0, state: State::Alive },
        );
        Self { self_id, members }
    }

    pub fn self_id(&self) -> &NodeId { &self.self_id }

    pub fn len(&self) -> usize { self.members.len() }

    pub fn is_empty(&self) -> bool { self.members.is_empty() }

    pub fn get(&self, id: &NodeId) -> Option<&NodeMeta> { self.members.get(id) }

    pub fn iter(&self) -> impl Iterator<Item = (&NodeId, &NodeMeta)> {
        self.members.iter()
    }

    /// Every known node's current state as a Vec of Deltas — what we
    /// hand to a JOIN-requester so they start with the full view.
    pub fn snapshot(&self) -> Vec<Delta> {
        self.members
            .iter()
            .map(|(id, m)| Delta {
                id: id.clone(),
                addr: m.addr,
                incarnation: m.incarnation,
                state: m.state,
            })
            .collect()
    }

    /// Merge a single incoming delta. Returns true iff this changed
    /// local state (i.e., the delta should be re-gossiped to peers
    /// so it propagates through the cluster).
    ///
    /// Merge rules (SWIM):
    ///   - Unknown node: insert.
    ///   - Higher incarnation: replace.
    ///   - Lower incarnation: ignore.
    ///   - Equal incarnation: replace only if incoming state is
    ///     "worse" (higher in the State ordering).
    ///
    /// Deltas about self with state != Alive require the Gossiper
    /// to refute via incarnation bump — this method applies the
    /// merge faithfully; the refutation lives in the Gossiper.
    pub fn merge(&mut self, delta: Delta) -> bool {
        let incoming = NodeMeta {
            addr: delta.addr,
            incarnation: delta.incarnation,
            state: delta.state,
        };
        match self.members.get(&delta.id) {
            None => {
                self.members.insert(delta.id, incoming);
                true
            }
            Some(existing) => {
                let supersedes = delta.incarnation > existing.incarnation
                    || (delta.incarnation == existing.incarnation
                        && delta.state > existing.state);
                if supersedes {
                    self.members.insert(delta.id, incoming);
                    true
                } else {
                    false
                }
            }
        }
    }
}

// ── Wire protocol ────────────────────────────────────────────────────
//
// Every gossip message carries the sender's NodeId so the receiver
// can attribute the update without depending on transport-level
// source-addr reporting (which would not survive NAT or UDP spoofing
// once we add mTLS in Cluster-5).

/// Messages exchanged between gossiping peers.
///
/// Ping / Ack carry `piggyback` deltas — membership updates the
/// sender wants to spread. PingReq asks an intermediary to probe a
/// suspected-silent target. Join/JoinAck bootstrap a new node into
/// the cluster. Leave announces a graceful exit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GossipMsg {
    Ping { from: NodeId, seq: u64, piggyback: Vec<Delta> },
    Ack { from: NodeId, seq: u64, piggyback: Vec<Delta> },
    PingReq { from: NodeId, target: NodeId, seq: u64 },
    Join { from: NodeId, addr: SocketAddr, incarnation: u64 },
    JoinAck { members: Vec<Delta> },
    Leave { from: NodeId, incarnation: u64 },
}

/// Gossip timing configuration. `Default` yields the paper's
/// recommendations; tests override via `for_tests()` to keep
/// ticks cheap while preserving the ordering T_ack < T_gossip <
/// T_suspect.
#[derive(Clone, Debug)]
pub struct GossipConfig {
    pub t_gossip_ms: u64,
    pub t_ack_ms: u64,
    pub t_suspect_ms: u64,
    pub indirect_k: usize,
}

impl Default for GossipConfig {
    fn default() -> Self {
        Self { t_gossip_ms: 1000, t_ack_ms: 200, t_suspect_ms: 5000, indirect_k: 3 }
    }
}

impl GossipConfig {
    /// Tighter timings for unit tests — same ordering, smaller
    /// numbers so synthetic-clock arithmetic stays readable.
    pub fn for_tests() -> Self {
        Self { t_gossip_ms: 100, t_ack_ms: 20, t_suspect_ms: 500, indirect_k: 3 }
    }
}

// ── Clock abstraction ─────────────────────────────────────────────────

/// Millisecond clock. Production uses `SystemClock` (wraps
/// `std::time::Instant`); tests use `TestClock` for deterministic
/// timing.
pub trait Clock: Send + Sync {
    fn now_millis(&self) -> u64;
}

/// Wall-clock since the process started. Millisecond resolution
/// — coarser than SWIM's native Duration granularity but still
/// well under T_ack (200ms).
pub struct SystemClock {
    start: std::time::Instant,
}

impl SystemClock {
    pub fn new() -> Self { Self { start: std::time::Instant::now() } }
}

impl Default for SystemClock {
    fn default() -> Self { Self::new() }
}

impl Clock for SystemClock {
    fn now_millis(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }
}

/// Synthetic clock for tests. `advance_millis` mutates the backing
/// atomic so multiple Gossipers sharing the same `Arc<TestClock>`
/// see time jump in lockstep.
pub struct TestClock { t: AtomicU64 }

impl TestClock {
    pub fn new() -> Self { Self { t: AtomicU64::new(0) } }
    pub fn advance_millis(&self, d: u64) { self.t.fetch_add(d, Ordering::SeqCst); }
}

impl Default for TestClock {
    fn default() -> Self { Self::new() }
}

impl Clock for TestClock {
    fn now_millis(&self) -> u64 { self.t.load(Ordering::SeqCst) }
}

// ── Peer selection ────────────────────────────────────────────────────

/// Picks peers to gossip with. Production uses a seeded LCG
/// (`LcgPicker`); tests use `RoundRobinPicker` for determinism.
pub trait PeerPicker: Send {
    /// Pick one candidate, or None if the slice is empty.
    fn pick_one(&mut self, candidates: &[NodeId]) -> Option<NodeId>;

    /// Pick up to k distinct candidates, skipping `exclude`.
    /// Used for PingReq indirect probes.
    fn pick_k(&mut self, candidates: &[NodeId], k: usize, exclude: &NodeId) -> Vec<NodeId>;
}

/// Deterministic round-robin picker for tests. Sorts candidates
/// lexicographically so the sequence of picks is reproducible
/// across runs regardless of HashMap iteration order.
pub struct RoundRobinPicker { cursor: usize }

impl RoundRobinPicker {
    pub fn new() -> Self { Self { cursor: 0 } }
}

impl Default for RoundRobinPicker {
    fn default() -> Self { Self::new() }
}

impl PeerPicker for RoundRobinPicker {
    fn pick_one(&mut self, candidates: &[NodeId]) -> Option<NodeId> {
        if candidates.is_empty() { return None; }
        let mut sorted: Vec<&NodeId> = candidates.iter().collect();
        sorted.sort();
        let i = self.cursor % sorted.len();
        self.cursor = self.cursor.wrapping_add(1);
        Some(sorted[i].clone())
    }

    fn pick_k(&mut self, candidates: &[NodeId], k: usize, exclude: &NodeId) -> Vec<NodeId> {
        let mut sorted: Vec<&NodeId> = candidates.iter().filter(|id| *id != exclude).collect();
        sorted.sort();
        sorted.into_iter().take(k).cloned().collect()
    }
}

/// Production peer picker: tiny linear-congruential generator,
/// seeded from the process start time. Deterministic enough to be
/// harmless, random enough that three nodes don't always pick the
/// same victim.
pub struct LcgPicker { state: u64 }

impl LcgPicker {
    pub fn new(seed: u64) -> Self { Self { state: seed.max(1) } }

    fn next(&mut self) -> u64 {
        // Numerical Recipes constants — not crypto-grade, but SWIM
        // peer selection doesn't need to be.
        self.state = self.state.wrapping_mul(1664525).wrapping_add(1013904223);
        self.state
    }
}

impl PeerPicker for LcgPicker {
    fn pick_one(&mut self, candidates: &[NodeId]) -> Option<NodeId> {
        if candidates.is_empty() { return None; }
        let i = (self.next() as usize) % candidates.len();
        Some(candidates[i].clone())
    }

    fn pick_k(&mut self, candidates: &[NodeId], k: usize, exclude: &NodeId) -> Vec<NodeId> {
        let pool: Vec<&NodeId> = candidates.iter().filter(|id| *id != exclude).collect();
        if pool.is_empty() { return Vec::new(); }
        let mut out = Vec::with_capacity(k.min(pool.len()));
        let mut used = hashbrown::HashSet::new();
        let mut attempts = 0;
        while out.len() < k.min(pool.len()) && attempts < pool.len() * 4 {
            let i = (self.next() as usize) % pool.len();
            if used.insert(i) {
                out.push(pool[i].clone());
            }
            attempts += 1;
        }
        out
    }
}

// ── Gossiper ──────────────────────────────────────────────────────────

/// The SWIM state machine. Owns a Membership, a Transport, a Clock,
/// and a PeerPicker. `tick()` drives everything — there is no
/// background thread inside; the caller (a background thread in
/// production, the test harness in unit tests) loops tick().
pub struct Gossiper<T: Transport, C: Clock, P: PeerPicker> {
    membership: Membership,
    transport: T,
    clock: Arc<C>,
    picker: P,
    cfg: GossipConfig,
    next_gossip_at: u64,
    next_seq: u64,
}

impl<T: Transport, C: Clock, P: PeerPicker> Gossiper<T, C, P> {
    pub fn new(
        self_id: NodeId,
        self_addr: SocketAddr,
        transport: T,
        clock: Arc<C>,
        picker: P,
        cfg: GossipConfig,
    ) -> Self {
        Self {
            membership: Membership::new(self_id, self_addr),
            transport,
            clock,
            picker,
            cfg,
            next_gossip_at: 0,
            next_seq: 1,
        }
    }

    pub fn membership(&self) -> &Membership { &self.membership }

    pub fn self_addr(&self) -> SocketAddr {
        self.membership.get(self.membership.self_id()).unwrap().addr
    }

    /// Merge a delta into the local view. Used by the JOIN handler
    /// and by tests that set up synthetic pre-join state.
    pub fn apply_delta(&mut self, delta: Delta) -> bool {
        self.membership.merge(delta)
    }

    /// One step of the state machine. Drains inbound messages, and
    /// if the gossip interval has elapsed, picks a peer and sends a
    /// PING carrying the current snapshot as piggyback.
    pub fn tick(&mut self) {
        let inbound = self.transport.recv_nonblocking();
        for (from_addr, msg) in inbound {
            self.handle(from_addr, msg);
        }
        let now = self.clock.now_millis();
        if now >= self.next_gossip_at {
            self.send_periodic_ping();
            self.next_gossip_at = now + self.cfg.t_gossip_ms;
        }
    }

    fn handle(&mut self, from_addr: SocketAddr, msg: GossipMsg) {
        match msg {
            GossipMsg::Ping { seq, piggyback, .. } => {
                for d in piggyback { self.apply_delta(d); }
                // Acknowledge with our current snapshot so the
                // sender learns anything we know that they don't.
                let ack = GossipMsg::Ack {
                    from: self.membership.self_id().clone(),
                    seq,
                    piggyback: self.membership.snapshot(),
                };
                let _ = self.transport.send(from_addr, &ack);
            }
            GossipMsg::Ack { piggyback, .. } => {
                for d in piggyback { self.apply_delta(d); }
            }
            // PingReq / Join / JoinAck / Leave land in follow-up
            // commits — acceptance test #1 only exercises Ping/Ack.
            GossipMsg::PingReq { .. }
            | GossipMsg::Join { .. }
            | GossipMsg::JoinAck { .. }
            | GossipMsg::Leave { .. } => {}
        }
    }

    fn send_periodic_ping(&mut self) {
        let self_id = self.membership.self_id().clone();
        let peers: Vec<NodeId> = self
            .membership
            .iter()
            .filter(|(id, m)| **id != self_id && m.state == State::Alive)
            .map(|(id, _)| id.clone())
            .collect();
        let Some(target) = self.picker.pick_one(&peers) else { return };
        let Some(target_meta) = self.membership.get(&target).cloned() else { return };
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        let ping = GossipMsg::Ping {
            from: self_id,
            seq,
            piggyback: self.membership.snapshot(),
        };
        let _ = self.transport.send(target_meta.addr, &ping);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::transport::InMemNet;

    fn addr(port: u16) -> SocketAddr {
        format!("127.0.0.1:{}", port).parse().unwrap()
    }

    #[test]
    fn new_membership_contains_only_self_alive_at_incarnation_zero() {
        let m = Membership::new("a".into(), addr(1000));
        assert_eq!(m.len(), 1);
        let me = m.get(&"a".to_string()).unwrap();
        assert_eq!(me.incarnation, 0);
        assert_eq!(me.state, State::Alive);
        assert_eq!(me.addr, addr(1000));
    }

    #[test]
    fn merge_inserts_unknown_node_and_reports_change() {
        let mut m = Membership::new("a".into(), addr(1000));
        let changed = m.merge(Delta {
            id: "b".into(),
            addr: addr(2000),
            incarnation: 1,
            state: State::Alive,
        });
        assert!(changed);
        assert_eq!(m.len(), 2);
        assert_eq!(m.get(&"b".to_string()).unwrap().state, State::Alive);
    }

    #[test]
    fn merge_ignores_lower_incarnation() {
        let mut m = Membership::new("a".into(), addr(1000));
        m.merge(Delta { id: "b".into(), addr: addr(2000), incarnation: 5, state: State::Alive });
        let changed = m.merge(Delta { id: "b".into(), addr: addr(2000), incarnation: 3, state: State::Dead });
        assert!(!changed);
        // The stale Dead delta must NOT overwrite a fresher Alive record.
        assert_eq!(m.get(&"b".to_string()).unwrap().state, State::Alive);
        assert_eq!(m.get(&"b".to_string()).unwrap().incarnation, 5);
    }

    #[test]
    fn merge_replaces_on_higher_incarnation_even_if_state_less_severe() {
        let mut m = Membership::new("a".into(), addr(1000));
        m.merge(Delta { id: "b".into(), addr: addr(2000), incarnation: 5, state: State::Suspect });
        // Higher incarnation Alive wins over lower incarnation Suspect —
        // this is exactly how a node refutes a stale Suspect rumor.
        let changed = m.merge(Delta { id: "b".into(), addr: addr(2000), incarnation: 6, state: State::Alive });
        assert!(changed);
        assert_eq!(m.get(&"b".to_string()).unwrap().state, State::Alive);
        assert_eq!(m.get(&"b".to_string()).unwrap().incarnation, 6);
    }

    #[test]
    fn merge_prefers_worse_state_on_same_incarnation() {
        let mut m = Membership::new("a".into(), addr(1000));
        m.merge(Delta { id: "b".into(), addr: addr(2000), incarnation: 5, state: State::Alive });
        let changed = m.merge(Delta { id: "b".into(), addr: addr(2000), incarnation: 5, state: State::Suspect });
        assert!(changed);
        assert_eq!(m.get(&"b".to_string()).unwrap().state, State::Suspect);
    }

    #[test]
    fn merge_ignores_same_incarnation_less_severe_state() {
        let mut m = Membership::new("a".into(), addr(1000));
        m.merge(Delta { id: "b".into(), addr: addr(2000), incarnation: 5, state: State::Suspect });
        let changed = m.merge(Delta { id: "b".into(), addr: addr(2000), incarnation: 5, state: State::Alive });
        assert!(!changed);
        assert_eq!(m.get(&"b".to_string()).unwrap().state, State::Suspect);
    }

    #[test]
    fn state_ordering_is_alive_suspect_left_dead() {
        assert!(State::Alive < State::Suspect);
        assert!(State::Suspect < State::Left);
        assert!(State::Left < State::Dead);
    }

    #[test]
    fn snapshot_contains_self_and_all_known_members() {
        let mut m = Membership::new("a".into(), addr(1000));
        m.merge(Delta { id: "b".into(), addr: addr(2000), incarnation: 1, state: State::Alive });
        m.merge(Delta { id: "c".into(), addr: addr(3000), incarnation: 2, state: State::Suspect });
        let snap = m.snapshot();
        assert_eq!(snap.len(), 3);
        let ids: Vec<_> = snap.iter().map(|d| d.id.clone()).collect();
        assert!(ids.contains(&"a".to_string()));
        assert!(ids.contains(&"b".to_string()));
        assert!(ids.contains(&"c".to_string()));
    }

    // ── Gossiper tests ───────────────────────────────────────────────

    fn new_gossiper(
        id: &str,
        port: u16,
        net: &Arc<InMemNet>,
        clock: Arc<TestClock>,
        cfg: GossipConfig,
    ) -> Gossiper<transport::InMemTransport, TestClock, RoundRobinPicker> {
        let a = addr(port);
        Gossiper::new(
            id.to_string(),
            a,
            net.endpoint(a),
            clock,
            RoundRobinPicker::new(),
            cfg,
        )
    }

    /// Acceptance test #1 (handoff): two in-mem Membership instances
    /// converge on a three-node view within 5 gossip rounds after a
    /// synthetic join.
    ///
    /// Setup: A and B already know each other; C synthetically joins
    /// via A (both sides learn the A–C edge). B learns about C only
    /// through gossip from A, and vice versa.
    #[test]
    fn three_nodes_converge_within_five_rounds_after_synthetic_join() {
        let net = InMemNet::new();
        let clock = Arc::new(TestClock::new());
        let cfg = GossipConfig::for_tests();

        let mut a = new_gossiper("a", 1000, &net, clock.clone(), cfg.clone());
        let mut b = new_gossiper("b", 2000, &net, clock.clone(), cfg.clone());
        let mut c = new_gossiper("c", 3000, &net, clock.clone(), cfg.clone());

        // Synthetic pre-join state: A↔B know each other; A↔C know
        // each other; B and C do NOT know each other yet.
        a.apply_delta(Delta { id: "b".into(), addr: addr(2000), incarnation: 0, state: State::Alive });
        b.apply_delta(Delta { id: "a".into(), addr: addr(1000), incarnation: 0, state: State::Alive });
        a.apply_delta(Delta { id: "c".into(), addr: addr(3000), incarnation: 0, state: State::Alive });
        c.apply_delta(Delta { id: "a".into(), addr: addr(1000), incarnation: 0, state: State::Alive });

        // Five rounds: advance past T_gossip, then tick everyone
        // twice so in-flight messages (Ping sent this round, Ack
        // replied this round) get processed within the same round.
        for _ in 0..5 {
            clock.advance_millis(cfg.t_gossip_ms + 1);
            a.tick(); b.tick(); c.tick();
            a.tick(); b.tick(); c.tick();
        }

        for (name, g) in [
            ("a", &a.membership),
            ("b", &b.membership),
            ("c", &c.membership),
        ] {
            assert_eq!(g.len(), 3, "{name} view size = {}, want 3", g.len());
            for peer in ["a", "b", "c"] {
                let m = g.get(&peer.to_string())
                    .unwrap_or_else(|| panic!("{name} missing peer {peer}"));
                assert_eq!(m.state, State::Alive, "{name} sees {peer} as {:?}", m.state);
            }
        }
    }
}
