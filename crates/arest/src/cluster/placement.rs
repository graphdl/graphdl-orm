// crates/arest/src/cluster/placement.rs
//
// Cluster-2: per-tenant primary placement via consistent-hash ring.
//
// Given a Membership (Cluster-1), compute deterministically which node
// owns any given tenant handle. The ring is the standard Karger et al.
// consistent hash: each Alive member is placed at `virtual_nodes`
// offsets on a 64-bit ring so load variance stays low with small
// clusters. For a tenant handle h, the primary is the first ring
// entry ≥ hash(h), wrapping to index 0 past the end.
//
// This module is pure state — it observes Membership but does not
// subscribe to its changes. The caller (placement driver, Cluster-3
// replication layer, request router) is responsible for calling
// `rebuild` after a membership transition it cares about. Full rebuild
// is O(N · V · log(N · V)) — sub-millisecond up to ~1k Alive nodes
// with V = 128, which is the scale we target.
//
// Why not incremental updates: a Suspect → Alive flap should produce
// the same ring as the equivalent rebuild; the simplicity pays for
// itself and rebuild cost is dwarfed by replication traffic anyway.

#![cfg(all(feature = "cluster", not(feature = "no_std")))]

use super::{Membership, NodeId, State};
use alloc::vec::Vec;

// FNV-1a 64-bit for variable-length byte inputs (vnode string ids).
const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = FNV_OFFSET;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// SplitMix64 (Stafford Mix 13). FNV-1a on a 4-byte u32 clusters the
/// output into a ~12-bit band of the 64-bit space — fine for string
/// dedup, terrible for hash-ring placement. SplitMix64 avalanches a
/// full 64-bit output from any 64-bit input in three rounds; we use
/// it anywhere the hash input is an integer or a fixed-width byte
/// stream so ring slots cover the whole 2^64 range.
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E3779B97F4A7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E7B5);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D209C6952B7E5F);
    x ^ (x >> 31)
}

/// Consistent-hash routing table mapping tenant handles → primary
/// NodeId. Only `State::Alive` members participate; Suspect / Dead /
/// Left all exit the ring so writes don't get routed to nodes that
/// might drop them.
///
/// See module docs for the pre-conditions on ring freshness.
pub struct Placement {
    /// (hash, node_id) sorted by hash. Binary-searchable in log time.
    ring: Vec<(u64, NodeId)>,
    /// Virtual nodes per NodeId. Larger → lower load variance, bigger
    /// ring, slower rebuild. 128 is a good default; tests use smaller
    /// values to make hash distribution easier to assert on.
    virtual_nodes: usize,
}

impl Placement {
    /// A reasonable default for production: load variance under 10%
    /// for clusters up to ~100 members.
    pub const DEFAULT_VNODES: usize = 128;

    /// An empty ring with the given virtual-node fan-out. Primary
    /// queries return None until `rebuild` is called.
    pub fn new(virtual_nodes: usize) -> Self {
        Self { ring: Vec::new(), virtual_nodes }
    }

    /// Build a ring directly from a membership snapshot.
    pub fn from_membership(m: &Membership, virtual_nodes: usize) -> Self {
        let mut p = Self::new(virtual_nodes);
        p.rebuild(m);
        p
    }

    /// Total ring slots (= alive_members × virtual_nodes).
    pub fn len(&self) -> usize { self.ring.len() }

    pub fn is_empty(&self) -> bool { self.ring.is_empty() }

    /// How many distinct Alive node IDs are on the ring. Cheap sweep —
    /// we don't keep a secondary index because rebuild amortises this
    /// out already and callers typically want `primaries_for` instead.
    pub fn alive_count(&self) -> usize {
        let mut seen: Vec<&NodeId> = Vec::new();
        for (_, id) in &self.ring {
            if !seen.iter().any(|s| *s == id) {
                seen.push(id);
            }
        }
        seen.len()
    }

    /// Discard the current ring and rebuild from `m`. Only Alive
    /// members participate.
    pub fn rebuild(&mut self, m: &Membership) {
        self.ring.clear();
        for (id, meta) in m.iter() {
            if meta.state != State::Alive { continue; }
            for v in 0..self.virtual_nodes {
                self.ring.push((Self::vnode_hash(id, v), id.clone()));
            }
        }
        self.ring.sort_by_key(|(h, _)| *h);
    }

    /// Hash the k-th virtual node of `id` into the ring. First FNV-1a
    /// the id string (variable-length input where FNV behaves), then
    /// SplitMix64-avalanche with the vnode index so 128 consecutive
    /// vnodes from the same id scatter across the full 64-bit space.
    fn vnode_hash(id: &NodeId, k: usize) -> u64 {
        let id_mix = fnv1a(id.as_bytes());
        splitmix64(id_mix ^ (k as u64).wrapping_mul(0x9E3779B97F4A7C15))
    }

    /// SplitMix64 of the tenant handle. Exposed pub(crate) so
    /// Cluster-3/4 can reuse exactly the same key without refactoring.
    pub(crate) fn hash_handle(handle: u32) -> u64 {
        splitmix64(handle as u64)
    }

    /// Primary node for a tenant handle. None iff the ring is empty
    /// (no Alive members).
    pub fn primary_for(&self, handle: u32) -> Option<&NodeId> {
        if self.ring.is_empty() { return None; }
        let h = Self::hash_handle(handle);
        // First slot with hash > h; wrap to 0 past the end.
        let idx = self.ring.partition_point(|(rh, _)| *rh <= h);
        let idx = if idx == self.ring.len() { 0 } else { idx };
        Some(&self.ring[idx].1)
    }

    /// Replication fan-out: primary + up to k-1 distinct successors.
    /// Fewer than k are returned when the cluster has fewer Alive
    /// nodes than k (so a k=3 query on a 2-Alive cluster returns both
    /// nodes — the caller decides how to handle under-replication).
    pub fn primaries_for(&self, handle: u32, k: usize) -> Vec<&NodeId> {
        if self.ring.is_empty() || k == 0 { return Vec::new(); }
        let h = Self::hash_handle(handle);
        let start = self.ring.partition_point(|(rh, _)| *rh <= h);
        let start = if start == self.ring.len() { 0 } else { start };
        let mut out: Vec<&NodeId> = Vec::with_capacity(k);
        let mut i = start;
        loop {
            let id = &self.ring[i].1;
            if !out.iter().any(|x| *x == id) {
                out.push(id);
                if out.len() == k { break; }
            }
            i = (i + 1) % self.ring.len();
            if i == start { break; }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{NodeMeta, State};
    use std::net::SocketAddr;

    fn mk(self_id: &str) -> Membership {
        Membership::new(
            self_id.to_string(),
            "127.0.0.1:9000".parse::<SocketAddr>().unwrap(),
        )
    }

    fn add(m: &mut Membership, id: &str, port: u16, state: State) {
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        m.merge(super::super::Delta {
            id: id.to_string(),
            addr,
            incarnation: 0,
            state,
        });
    }

    /// 5000 tenants over a 3-node cluster with V=128 should all resolve
    /// to a node in the Alive set, and the distribution should be
    /// within ~30% of 1/3 per node. (V=128 variance bound.)
    #[test]
    fn primary_for_distributes_over_alive_nodes() {
        let mut m = mk("A");
        add(&mut m, "B", 9001, State::Alive);
        add(&mut m, "C", 9002, State::Alive);
        let p = Placement::from_membership(&m, 128);

        let mut counts: std::collections::HashMap<String, u32> =
            std::collections::HashMap::new();
        for h in 0u32..5000 {
            let id = p.primary_for(h).unwrap();
            *counts.entry(id.clone()).or_default() += 1;
        }
        assert_eq!(counts.len(), 3, "all 3 Alive nodes must own tenants");
        for (id, c) in &counts {
            let frac = *c as f64 / 5000.0;
            assert!((0.23..=0.44).contains(&frac),
                "node {id} owns {c}/{total} = {frac:.3}; expected 0.33±0.10",
                total = 5000);
        }
    }

    /// Same membership → same placement. This is the consistency
    /// guarantee the routing table depends on.
    #[test]
    fn primary_for_is_deterministic() {
        let mut m = mk("A");
        add(&mut m, "B", 9001, State::Alive);
        add(&mut m, "C", 9002, State::Alive);

        let p1 = Placement::from_membership(&m, 32);
        let p2 = Placement::from_membership(&m, 32);

        for h in 0u32..200 {
            assert_eq!(p1.primary_for(h), p2.primary_for(h));
        }
    }

    /// Dead and Left nodes must not own any tenants — they exit the
    /// ring so the router doesn't send writes to nodes that will drop
    /// them.
    #[test]
    fn non_alive_nodes_excluded_from_ring() {
        let mut m = mk("A");
        add(&mut m, "B", 9001, State::Dead);
        add(&mut m, "C", 9002, State::Left);
        add(&mut m, "D", 9003, State::Suspect);
        let p = Placement::from_membership(&m, 16);

        for h in 0u32..1000 {
            let owner = p.primary_for(h).unwrap();
            assert_eq!(owner, "A",
                "handle {h} routed to {owner}; only Alive node A should be on the ring");
        }
    }

    /// Empty ring ⇒ no primary. (Zero Alive members — cluster is
    /// effectively down.)
    #[test]
    fn empty_ring_has_no_primary() {
        let mut m = mk("A");
        // Mark self Dead — now zero Alive members.
        add(&mut m, "A", 9000, State::Dead);
        let p = Placement::from_membership(&m, 16);
        assert!(p.is_empty());
        assert_eq!(p.primary_for(42), None);
        assert_eq!(p.primaries_for(42, 3), Vec::<&NodeId>::new());
    }

    /// Adding a fourth node should move ≈ 1/4 of existing tenants,
    /// not all of them. This is the whole point of consistent hashing:
    /// minimal disruption on membership change.
    #[test]
    fn adding_node_reassigns_fraction_not_all() {
        let mut m = mk("A");
        add(&mut m, "B", 9001, State::Alive);
        add(&mut m, "C", 9002, State::Alive);
        let p3 = Placement::from_membership(&m, 128);

        add(&mut m, "D", 9003, State::Alive);
        let p4 = Placement::from_membership(&m, 128);

        let mut moved = 0u32;
        for h in 0u32..5000 {
            if p3.primary_for(h) != p4.primary_for(h) {
                moved += 1;
            }
        }
        let frac = moved as f64 / 5000.0;
        // Expected ≈ 0.25 (one new node out of four gets 1/4 of keys).
        // Tolerance ±0.10 for the hash variance at V=128.
        assert!((0.15..=0.35).contains(&frac),
            "adding 4th node moved {moved}/5000 = {frac:.3}; \
             expected 0.25±0.10 — consistent hashing is broken");
    }

    /// Removing a node should only re-home that node's tenants. The
    /// other two nodes' assignments must be unchanged.
    #[test]
    fn removing_node_only_affects_its_own_tenants() {
        let mut m = mk("A");
        add(&mut m, "B", 9001, State::Alive);
        add(&mut m, "C", 9002, State::Alive);
        let p3 = Placement::from_membership(&m, 128);

        // Mark C Dead.
        add(&mut m, "C", 9002, State::Dead);
        let p2 = Placement::from_membership(&m, 128);

        for h in 0u32..5000 {
            let before = p3.primary_for(h).unwrap();
            let after = p2.primary_for(h).unwrap();
            if before != "C" {
                assert_eq!(before, after,
                    "handle {h} owned by {before} before; must stay on {before} \
                     after C is Dead — got reassigned to {after}");
            } else {
                assert_ne!(after, "C",
                    "handle {h} was on dead node C; must have moved off");
            }
        }
    }

    /// primaries_for(h, 3) on a 3-node cluster must return 3 distinct
    /// node ids — one primary + two replicas.
    #[test]
    fn primaries_for_k_returns_k_distinct_nodes() {
        let mut m = mk("A");
        add(&mut m, "B", 9001, State::Alive);
        add(&mut m, "C", 9002, State::Alive);
        let p = Placement::from_membership(&m, 64);

        for h in 0u32..200 {
            let fan = p.primaries_for(h, 3);
            assert_eq!(fan.len(), 3);
            let mut unique = fan.clone();
            unique.sort();
            unique.dedup();
            assert_eq!(unique.len(), 3,
                "replication fan-out for handle {h} had duplicates: {fan:?}");
            assert_eq!(fan[0], p.primary_for(h).unwrap(),
                "first entry of fan-out must equal primary");
        }
    }

    /// primaries_for(h, k) on a k'-node cluster with k' < k returns k'
    /// entries — under-replicated, not crashed.
    #[test]
    fn primaries_for_k_caps_at_cluster_size() {
        let mut m = mk("A");
        add(&mut m, "B", 9001, State::Alive);
        // Only 2 Alive nodes.
        let p = Placement::from_membership(&m, 16);

        let fan = p.primaries_for(0, 5);
        assert_eq!(fan.len(), 2,
            "2-Alive cluster must cap fan-out at 2; got {fan:?}");
        let mut unique = fan.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), 2, "both entries should be distinct");
    }

    /// Rebuild is idempotent: building twice from the same membership
    /// yields the same ring byte-for-byte.
    #[test]
    fn rebuild_is_idempotent() {
        let mut m = mk("A");
        add(&mut m, "B", 9001, State::Alive);
        add(&mut m, "C", 9002, State::Alive);

        let mut p = Placement::from_membership(&m, 32);
        let snapshot: Vec<(u64, NodeId)> = p.ring.clone();
        p.rebuild(&m);
        assert_eq!(p.ring, snapshot);
    }

    /// alive_count reflects only Alive members, regardless of virtual
    /// node count.
    #[test]
    fn alive_count_ignores_virtual_nodes() {
        let mut m = mk("A");
        add(&mut m, "B", 9001, State::Alive);
        add(&mut m, "C", 9002, State::Dead);
        let p = Placement::from_membership(&m, 64);
        assert_eq!(p.alive_count(), 2);
        assert_eq!(p.len(), 2 * 64);
    }
}
