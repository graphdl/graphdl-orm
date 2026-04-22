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

pub mod failover;
pub mod placement;
pub mod replication;
pub mod transport;
pub mod wire;

use crate::ast::Object;
use crate::freeze::{freeze, thaw};
use alloc::sync::Arc;
use hashbrown::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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

/// Serialize a GossipMsg to bytes via freeze (#185). The cluster
/// protocol reuses AREST's canonical byte format — any consumer of
/// frozen Objects (the FPGA boot ROM, future mTLS framing in
/// Cluster-5) understands these bytes without a second codec.
pub fn encode(msg: &GossipMsg) -> Vec<u8> {
    freeze(&msg.to_object())
}

/// Deserialize bytes into a GossipMsg. Returns an error on bad
/// magic, truncated input, unknown tags, or a well-formed Object
/// that doesn't match any GossipMsg shape. The UDP transport drops
/// errors silently — gossip is lossy by design.
pub fn decode(bytes: &[u8]) -> Result<GossipMsg, String> {
    GossipMsg::from_object(&thaw(bytes)?)
}

// Encoding keys — one constant per field so typos fail at compile
// time, not at wire-decode time.
const K_TYPE: &str = "type";
const K_FROM: &str = "from";
const K_SEQ: &str = "seq";
const K_PIGGYBACK: &str = "piggyback";
const K_TARGET: &str = "target";
const K_ADDR: &str = "addr";
const K_INCARNATION: &str = "incarnation";
const K_MEMBERS: &str = "members";
const K_ID: &str = "id";
const K_STATE: &str = "state";

const T_PING: &str = "ping";
const T_ACK: &str = "ack";
const T_PING_REQ: &str = "pingreq";
const T_JOIN: &str = "join";
const T_JOIN_ACK: &str = "joinack";
const T_LEAVE: &str = "leave";

const S_ALIVE: &str = "alive";
const S_SUSPECT: &str = "suspect";
const S_LEFT: &str = "left";
const S_DEAD: &str = "dead";

impl State {
    fn wire_str(&self) -> &'static str {
        match self {
            State::Alive => S_ALIVE,
            State::Suspect => S_SUSPECT,
            State::Left => S_LEFT,
            State::Dead => S_DEAD,
        }
    }

    fn from_wire(s: &str) -> Result<Self, String> {
        match s {
            S_ALIVE => Ok(State::Alive),
            S_SUSPECT => Ok(State::Suspect),
            S_LEFT => Ok(State::Left),
            S_DEAD => Ok(State::Dead),
            other => Err(format!("unknown state on wire: {other:?}")),
        }
    }
}

impl Delta {
    fn to_object(&self) -> Object {
        let mut m: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        m.insert(K_ID.into(), Object::atom(&self.id));
        m.insert(K_ADDR.into(), Object::atom(&self.addr.to_string()));
        m.insert(K_INCARNATION.into(), Object::atom(&self.incarnation.to_string()));
        m.insert(K_STATE.into(), Object::atom(self.state.wire_str()));
        Object::Map(m)
    }

    fn from_object(obj: &Object) -> Result<Self, String> {
        let m = obj.as_map().ok_or_else(|| "Delta: expected Map".to_string())?;
        Ok(Delta {
            id: map_atom(m, K_ID)?.to_string(),
            addr: map_atom(m, K_ADDR)?
                .parse()
                .map_err(|e| format!("Delta.addr parse: {e}"))?,
            incarnation: map_atom(m, K_INCARNATION)?
                .parse()
                .map_err(|e| format!("Delta.incarnation parse: {e}"))?,
            state: State::from_wire(map_atom(m, K_STATE)?)?,
        })
    }
}

impl GossipMsg {
    pub fn to_object(&self) -> Object {
        let mut m: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        match self {
            GossipMsg::Ping { from, seq, piggyback } => {
                m.insert(K_TYPE.into(), Object::atom(T_PING));
                m.insert(K_FROM.into(), Object::atom(from));
                m.insert(K_SEQ.into(), Object::atom(&seq.to_string()));
                m.insert(K_PIGGYBACK.into(), deltas_to_seq(piggyback));
            }
            GossipMsg::Ack { from, seq, piggyback } => {
                m.insert(K_TYPE.into(), Object::atom(T_ACK));
                m.insert(K_FROM.into(), Object::atom(from));
                m.insert(K_SEQ.into(), Object::atom(&seq.to_string()));
                m.insert(K_PIGGYBACK.into(), deltas_to_seq(piggyback));
            }
            GossipMsg::PingReq { from, target, seq } => {
                m.insert(K_TYPE.into(), Object::atom(T_PING_REQ));
                m.insert(K_FROM.into(), Object::atom(from));
                m.insert(K_TARGET.into(), Object::atom(target));
                m.insert(K_SEQ.into(), Object::atom(&seq.to_string()));
            }
            GossipMsg::Join { from, addr, incarnation } => {
                m.insert(K_TYPE.into(), Object::atom(T_JOIN));
                m.insert(K_FROM.into(), Object::atom(from));
                m.insert(K_ADDR.into(), Object::atom(&addr.to_string()));
                m.insert(K_INCARNATION.into(), Object::atom(&incarnation.to_string()));
            }
            GossipMsg::JoinAck { members } => {
                m.insert(K_TYPE.into(), Object::atom(T_JOIN_ACK));
                m.insert(K_MEMBERS.into(), deltas_to_seq(members));
            }
            GossipMsg::Leave { from, incarnation } => {
                m.insert(K_TYPE.into(), Object::atom(T_LEAVE));
                m.insert(K_FROM.into(), Object::atom(from));
                m.insert(K_INCARNATION.into(), Object::atom(&incarnation.to_string()));
            }
        }
        Object::Map(m)
    }

    pub fn from_object(obj: &Object) -> Result<Self, String> {
        let m = obj.as_map().ok_or_else(|| "GossipMsg: expected Map".to_string())?;
        match map_atom(m, K_TYPE)? {
            T_PING => Ok(GossipMsg::Ping {
                from: map_atom(m, K_FROM)?.to_string(),
                seq: map_atom(m, K_SEQ)?.parse().map_err(|e| format!("seq parse: {e}"))?,
                piggyback: seq_to_deltas(m.get(K_PIGGYBACK).ok_or("piggyback missing")?)?,
            }),
            T_ACK => Ok(GossipMsg::Ack {
                from: map_atom(m, K_FROM)?.to_string(),
                seq: map_atom(m, K_SEQ)?.parse().map_err(|e| format!("seq parse: {e}"))?,
                piggyback: seq_to_deltas(m.get(K_PIGGYBACK).ok_or("piggyback missing")?)?,
            }),
            T_PING_REQ => Ok(GossipMsg::PingReq {
                from: map_atom(m, K_FROM)?.to_string(),
                target: map_atom(m, K_TARGET)?.to_string(),
                seq: map_atom(m, K_SEQ)?.parse().map_err(|e| format!("seq parse: {e}"))?,
            }),
            T_JOIN => Ok(GossipMsg::Join {
                from: map_atom(m, K_FROM)?.to_string(),
                addr: map_atom(m, K_ADDR)?.parse().map_err(|e| format!("addr parse: {e}"))?,
                incarnation: map_atom(m, K_INCARNATION)?
                    .parse()
                    .map_err(|e| format!("incarnation parse: {e}"))?,
            }),
            T_JOIN_ACK => Ok(GossipMsg::JoinAck {
                members: seq_to_deltas(m.get(K_MEMBERS).ok_or("members missing")?)?,
            }),
            T_LEAVE => Ok(GossipMsg::Leave {
                from: map_atom(m, K_FROM)?.to_string(),
                incarnation: map_atom(m, K_INCARNATION)?
                    .parse()
                    .map_err(|e| format!("incarnation parse: {e}"))?,
            }),
            other => Err(format!("GossipMsg: unknown type {other:?}")),
        }
    }
}

fn map_atom<'a>(m: &'a hashbrown::HashMap<String, Object>, key: &str) -> Result<&'a str, String> {
    m.get(key)
        .ok_or_else(|| format!("missing key {key:?}"))?
        .as_atom()
        .ok_or_else(|| format!("key {key:?} not an Atom"))
}

fn deltas_to_seq(deltas: &[Delta]) -> Object {
    Object::Seq(deltas.iter().map(Delta::to_object).collect::<Vec<_>>().into())
}

fn seq_to_deltas(obj: &Object) -> Result<Vec<Delta>, String> {
    let items = obj.as_seq().ok_or_else(|| "deltas: expected Seq".to_string())?;
    items.iter().map(Delta::from_object).collect()
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

/// A probe currently in flight. The Gossiper holds at most one at a
/// time — SWIM deliberately serializes probes so failure-detection
/// latency is bounded by T_ack, not T_ack × n_peers.
#[derive(Clone, Debug)]
struct Probe {
    target: NodeId,
    /// Clock-ms when the current phase began. On direct-phase
    /// timeout we'd escalate to indirect PingReqs; with no other
    /// alive peers to ask, the Gossiper short-circuits to Suspect.
    phase_started_at: u64,
}

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
    probe: Option<Probe>,
    /// Map of NodeId → clock-ms when we marked it Suspect. On
    /// tick, any entry older than T_suspect flips to Dead.
    suspect_since: HashMap<NodeId, u64>,
    /// Bootstrap addresses we still need to Join. Drained when a
    /// JoinAck arrives. Tick retries until `bootstrap_attempts` hits
    /// MAX_JOIN_ATTEMPTS or at least one JoinAck is received.
    bootstrap_pending: Vec<SocketAddr>,
    bootstrap_attempts: u32,
}

/// Retry ceiling for bootstrap JOIN. At T_gossip = 1s, 10 attempts
/// = 10s of patience — plenty for a cold-started seed node, not so
/// much that we never give up.
const MAX_JOIN_ATTEMPTS: u32 = 10;

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
            probe: None,
            suspect_since: HashMap::new(),
            bootstrap_pending: Vec::new(),
            bootstrap_attempts: 0,
        }
    }

    /// Register bootstrap peers to contact on startup. The next few
    /// `tick()` calls will send `Join` messages; once a `JoinAck`
    /// arrives (or `MAX_JOIN_ATTEMPTS` pass) the pending list is
    /// cleared.
    pub fn set_bootstrap(&mut self, addrs: Vec<SocketAddr>) {
        self.bootstrap_pending = addrs;
        self.bootstrap_attempts = 0;
    }

    /// Announce a graceful exit: broadcast `Leave` to every Alive
    /// peer, then mark self as `Left` locally so the final snapshot
    /// reflects the departure. The caller should stop calling
    /// `tick()` after this returns; the peer-side transition to
    /// `Left` is driven by the broadcast.
    pub fn broadcast_leave(&mut self) {
        let self_id = self.membership.self_id().clone();
        let Some(me) = self.membership.get(&self_id).cloned() else { return };
        let leave = GossipMsg::Leave {
            from: self_id.clone(),
            incarnation: me.incarnation,
        };
        let alive: Vec<SocketAddr> = self
            .membership
            .iter()
            .filter(|(id, m)| **id != self_id && m.state == State::Alive)
            .map(|(_, m)| m.addr)
            .collect();
        for addr in alive {
            let _ = self.transport.send(addr, &leave);
        }
        self.membership.merge(Delta {
            id: self_id,
            addr: me.addr,
            incarnation: me.incarnation,
            state: State::Left,
        });
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

    /// One step of the state machine. Drains inbound messages, checks
    /// probe and suspect deadlines, retries pending bootstrap joins,
    /// and (if no probe is in flight and the gossip interval has
    /// elapsed) starts a fresh probe by sending a Ping to a random
    /// Alive peer with the current snapshot as piggyback.
    ///
    /// One probe in flight at a time: SWIM deliberately serializes
    /// probes so failure-detection latency is bounded by T_ack, and
    /// so a slow peer doesn't pile up probes and distort the
    /// cluster-wide gossip load.
    pub fn tick(&mut self) {
        let inbound = self.transport.recv_nonblocking();
        for (from_addr, msg) in inbound {
            self.handle(from_addr, msg);
        }
        self.retry_bootstrap();
        let now = self.clock.now_millis();
        self.check_probe_deadline(now);
        self.check_suspect_deadlines(now);
        if now >= self.next_gossip_at && self.probe.is_none() {
            self.start_probe(now);
            self.next_gossip_at = now + self.cfg.t_gossip_ms;
        }
    }

    fn retry_bootstrap(&mut self) {
        if self.bootstrap_pending.is_empty() || self.bootstrap_attempts >= MAX_JOIN_ATTEMPTS {
            return;
        }
        let self_id = self.membership.self_id().clone();
        let self_addr = self.self_addr();
        let incarnation = self
            .membership
            .get(&self_id)
            .map(|m| m.incarnation)
            .unwrap_or(0);
        let join = GossipMsg::Join { from: self_id, addr: self_addr, incarnation };
        let targets: Vec<SocketAddr> = self.bootstrap_pending.clone();
        for addr in targets {
            let _ = self.transport.send(addr, &join);
        }
        self.bootstrap_attempts += 1;
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
            GossipMsg::Ack { from, piggyback, .. } => {
                for d in piggyback { self.apply_delta(d); }
                // An Ack from our probe target clears the probe:
                // direct evidence the node is alive.
                if let Some(probe) = self.probe.as_ref() {
                    if probe.target == from {
                        self.probe = None;
                    }
                }
            }
            GossipMsg::Join { from, addr, incarnation } => {
                // The newbie has announced themselves. Add them to
                // our view and reply with the current snapshot so
                // they boot straight into a full membership list.
                self.apply_delta(Delta {
                    id: from,
                    addr,
                    incarnation,
                    state: State::Alive,
                });
                let ack = GossipMsg::JoinAck { members: self.membership.snapshot() };
                let _ = self.transport.send(addr, &ack);
            }
            GossipMsg::JoinAck { members } => {
                for d in members { self.apply_delta(d); }
                // Any JoinAck satisfies bootstrap — we've been
                // welcomed; no need to keep spamming the seed list.
                self.bootstrap_pending.clear();
            }
            GossipMsg::Leave { from, incarnation } => {
                // Honor the Leave only if it matches or exceeds the
                // incarnation we have. A stale Leave at a lower
                // incarnation cannot retire a restarted peer.
                if let Some(existing) = self.membership.get(&from).cloned() {
                    if incarnation >= existing.incarnation {
                        self.membership.merge(Delta {
                            id: from,
                            addr: existing.addr,
                            incarnation,
                            state: State::Left,
                        });
                    }
                }
            }
            // PingReq relay lands in a later commit — the acceptance
            // tests short-circuit to Suspect when no intermediaries
            // are available, which is the SWIM paper's own fallback.
            GossipMsg::PingReq { .. } => {}
        }
    }

    /// Start a new probe: pick a random Alive peer, send Ping, record
    /// the probe so the deadline checker can escalate to Suspect if
    /// no Ack arrives in time.
    fn start_probe(&mut self, now: u64) {
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
        self.probe = Some(Probe { target, phase_started_at: now });
    }

    fn check_probe_deadline(&mut self, now: u64) {
        let Some(probe) = self.probe.as_ref() else { return };
        if now.saturating_sub(probe.phase_started_at) <= self.cfg.t_ack_ms {
            return;
        }
        // Direct probe timed out. SWIM would escalate to indirect
        // PingReqs here; with no intermediaries (0 other Alive peers),
        // we short-circuit to Suspect. The PingReq relay path lives
        // in a follow-up — until then, even the 2-node case falls
        // through to Suspect correctly.
        let target = probe.target.clone();
        self.mark_suspect(&target, now);
        self.probe = None;
    }

    fn check_suspect_deadlines(&mut self, now: u64) {
        let expired: Vec<NodeId> = self
            .suspect_since
            .iter()
            .filter(|(_, &started)| now.saturating_sub(started) > self.cfg.t_suspect_ms)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            self.mark_dead(&id);
        }
    }

    fn mark_suspect(&mut self, id: &NodeId, now: u64) {
        let Some(existing) = self.membership.get(id).cloned() else { return };
        // Only transition Alive → Suspect at the current incarnation.
        // A peer can refute the Suspect by broadcasting Alive at a
        // higher incarnation (once the Gossiper implements self-
        // refutation — see follow-up commit).
        if existing.state == State::Alive {
            self.membership.merge(Delta {
                id: id.clone(),
                addr: existing.addr,
                incarnation: existing.incarnation,
                state: State::Suspect,
            });
            self.suspect_since.insert(id.clone(), now);
        }
    }

    fn mark_dead(&mut self, id: &NodeId) {
        let Some(existing) = self.membership.get(id).cloned() else { return };
        self.membership.merge(Delta {
            id: id.clone(),
            addr: existing.addr,
            incarnation: existing.incarnation,
            state: State::Dead,
        });
        self.suspect_since.remove(id);
    }
}

// ── Boot path ─────────────────────────────────────────────────────────
//
// `start` is the entry point main.rs (or any host) calls to turn a
// listen address plus a bootstrap list into a live, gossiping node.
// It binds a UdpTransport, constructs a Gossiper with production
// defaults (SystemClock + LcgPicker seeded from UNIX time), and
// spawns a background thread that drives tick() on a tight
// interval. The returned ClusterHandle exposes the latest
// membership snapshot and a blocking shutdown.

use std::io;

/// Driver-thread tick cadence. Much smaller than T_gossip so the
/// state machine reacts promptly to ack/suspect deadlines (20-40ms
/// granularity) without busy-looping.
const TICK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(20);

/// Handle to a gossiping node running on a background thread.
/// Dropping the handle (or calling `shutdown`) stops the thread,
/// broadcasts a Leave, and joins. Callers inspect the latest view
/// via `snapshot`.
pub struct ClusterHandle {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    local_addr: SocketAddr,
    shared: Arc<std::sync::RwLock<Vec<Delta>>>,
}

impl ClusterHandle {
    pub fn local_addr(&self) -> SocketAddr { self.local_addr }

    /// Latest membership snapshot. Updated on every tick (every
    /// TICK_INTERVAL), so two consecutive calls may yield
    /// different vectors.
    pub fn snapshot(&self) -> Vec<Delta> {
        self.shared.read().unwrap().clone()
    }

    /// Stop gossiping: set the flag, join the thread. The thread
    /// broadcasts Leave before exiting so peers transition us to
    /// `Left` promptly rather than waiting for T_suspect.
    pub fn shutdown(mut self) {
        self.stop_and_join();
    }

    fn stop_and_join(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for ClusterHandle {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

/// Spawn a gossiping node. Binds `listen`, joins each `bootstrap`
/// address if non-empty, and returns a handle for inspection and
/// shutdown.
pub fn start(
    self_id: NodeId,
    listen: SocketAddr,
    bootstrap: Vec<SocketAddr>,
    cfg: GossipConfig,
) -> io::Result<ClusterHandle> {
    let transport = transport::UdpTransport::bind(listen)?;
    let actual_addr = transport.addr();
    let clock = Arc::new(SystemClock::new());
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1);
    let picker = LcgPicker::new(seed);
    let mut gossiper = Gossiper::new(self_id, actual_addr, transport, clock, picker, cfg);
    if !bootstrap.is_empty() {
        gossiper.set_bootstrap(bootstrap);
    }
    let initial = gossiper.membership().snapshot();
    let shared = Arc::new(std::sync::RwLock::new(initial));
    let stop = Arc::new(AtomicBool::new(false));

    let shared_rx = Arc::clone(&shared);
    let stop_rx = Arc::clone(&stop);
    let thread = std::thread::Builder::new()
        .name(format!("cluster-gossiper-{actual_addr}"))
        .spawn(move || {
            while !stop_rx.load(Ordering::Relaxed) {
                gossiper.tick();
                *shared_rx.write().unwrap() = gossiper.membership().snapshot();
                std::thread::sleep(TICK_INTERVAL);
            }
            // Graceful exit — tell everyone we're Leaving so they
            // transition us to Left instantly instead of discovering
            // it via T_suspect timeout.
            gossiper.broadcast_leave();
            *shared_rx.write().unwrap() = gossiper.membership().snapshot();
        })?;

    Ok(ClusterHandle {
        stop,
        thread: Some(thread),
        local_addr: actual_addr,
        shared,
    })
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

    // ── Wire roundtrip tests ─────────────────────────────────────────

    fn wire_roundtrip(msg: GossipMsg) {
        let bytes = encode(&msg);
        let decoded = decode(&bytes).unwrap_or_else(|e| panic!("decode({msg:?}): {e}"));
        assert_eq!(msg, decoded, "roundtrip of {:?}", decoded);
    }

    fn sample_delta(id: &str, port: u16, incarnation: u64, state: State) -> Delta {
        Delta { id: id.into(), addr: addr(port), incarnation, state }
    }

    #[test]
    fn wire_roundtrip_ping() {
        wire_roundtrip(GossipMsg::Ping {
            from: "a".into(),
            seq: 42,
            piggyback: vec![
                sample_delta("a", 1000, 0, State::Alive),
                sample_delta("b", 2000, 3, State::Suspect),
            ],
        });
    }

    #[test]
    fn wire_roundtrip_ack() {
        wire_roundtrip(GossipMsg::Ack {
            from: "b".into(),
            seq: 7,
            piggyback: vec![sample_delta("c", 3000, 1, State::Dead)],
        });
    }

    #[test]
    fn wire_roundtrip_ack_empty_piggyback() {
        wire_roundtrip(GossipMsg::Ack {
            from: "b".into(),
            seq: 0,
            piggyback: vec![],
        });
    }

    #[test]
    fn wire_roundtrip_ping_req() {
        wire_roundtrip(GossipMsg::PingReq {
            from: "a".into(),
            target: "c".into(),
            seq: 99,
        });
    }

    #[test]
    fn wire_roundtrip_join() {
        wire_roundtrip(GossipMsg::Join {
            from: "newbie".into(),
            addr: addr(4040),
            incarnation: 0,
        });
    }

    #[test]
    fn wire_roundtrip_join_ack() {
        wire_roundtrip(GossipMsg::JoinAck {
            members: vec![
                sample_delta("a", 1000, 0, State::Alive),
                sample_delta("b", 2000, 5, State::Alive),
                sample_delta("c", 3000, 2, State::Left),
            ],
        });
    }

    #[test]
    fn wire_roundtrip_leave() {
        wire_roundtrip(GossipMsg::Leave {
            from: "graceful".into(),
            incarnation: 12,
        });
    }

    #[test]
    fn decode_empty_bytes_errors() {
        assert!(decode(&[]).is_err());
    }

    #[test]
    fn decode_bad_magic_errors() {
        let garbage = vec![0xffu8; 32];
        assert!(decode(&garbage).is_err());
    }

    #[test]
    fn decode_unknown_type_errors() {
        // A well-formed frozen Map with an unrecognised "type" value.
        let mut m: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        m.insert("type".into(), Object::atom("nope"));
        let bytes = freeze(&Object::Map(m));
        let err = decode(&bytes).unwrap_err();
        assert!(err.contains("unknown type"), "error was {err:?}");
    }

    /// Acceptance test #2 (handoff): a node going silent is marked
    /// Suspect then Dead within T_suspect + T_gossip.
    ///
    /// Setup: A holds B in its view as Alive, but B has no Gossiper —
    /// its mailbox fills up and nothing ever drains it. A's probe of
    /// B times out on the direct phase; with no intermediaries in the
    /// cluster (only A and B), there is no one to PingReq, so A
    /// short-circuits directly to Suspect. After T_suspect elapses
    /// without refutation, A flips B to Dead.
    #[test]
    fn silent_node_transitions_alive_to_suspect_to_dead() {
        let net = InMemNet::new();
        let clock = Arc::new(TestClock::new());
        let cfg = GossipConfig::for_tests();

        let mut a = new_gossiper("a", 1000, &net, clock.clone(), cfg.clone());

        // Reserve B's mailbox so A's Pings don't error on send —
        // they just queue up in a mailbox nobody drains.
        let _silent_b = net.endpoint(addr(2000));
        a.apply_delta(Delta { id: "b".into(), addr: addr(2000), incarnation: 0, state: State::Alive });

        assert_eq!(a.membership().get(&"b".to_string()).unwrap().state, State::Alive);

        // Round 1: tick fires periodic gossip, A probes B.
        clock.advance_millis(cfg.t_gossip_ms + 1);
        a.tick();

        // Still Alive — the probe is in flight.
        assert_eq!(a.membership().get(&"b".to_string()).unwrap().state, State::Alive);

        // Past T_ack: probe times out, no intermediaries, B → Suspect.
        clock.advance_millis(cfg.t_ack_ms + 1);
        a.tick();

        assert_eq!(
            a.membership().get(&"b".to_string()).unwrap().state,
            State::Suspect,
            "after T_ack without reply, B should be Suspect"
        );

        // Past T_suspect: Suspect → Dead.
        clock.advance_millis(cfg.t_suspect_ms + 1);
        a.tick();

        assert_eq!(
            a.membership().get(&"b".to_string()).unwrap().state,
            State::Dead,
            "after T_suspect without refutation, B should be Dead"
        );
    }
}
