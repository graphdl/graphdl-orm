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

use hashbrown::HashMap;
use std::net::SocketAddr;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
