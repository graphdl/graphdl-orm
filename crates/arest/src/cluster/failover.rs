// crates/arest/src/cluster/failover.rs
//
// Cluster-4: failover via ring-derived primary transitions.
//
// Given that Cluster-1 marks a failed node Suspect → Dead and the
// Placement ring excludes non-Alive members (Cluster-2), the set of
// tenants that need to fail over is exactly {h : Placement before had
// node=X, Placement now has node=Y}. Every running node runs a
// FailoverDetector locally; when its own `self_id` appears as the new
// primary for any tenant, the runtime promotes that tenant (resume
// replication from the follower's last-applied generation + 1).
//
// No quorum / leader election here — SWIM's eventually-consistent
// membership view + the deterministic consistent-hash ring mean two
// nodes that see the same membership land on the same primary. If
// they see different membership views briefly, both may think they
// own the same tenant; the replication layer's generation counter
// (Cluster-3) is the safety net: the node whose view converges first
// issues a higher generation, others' writes are Nack'd. This is the
// "split-brain tolerable because generation resolves it" strategy.
//
// The detector is pure state — no I/O, no timers. Callers feed it the
// current Placement + the list of tenants this node cares about
// (typically: everything the local CompiledState has rehydrated), and
// it returns the set of transitions since the last call. Hook the
// returned events to the runtime:
//   Promoted — call Replicator::resume_from(h, follower.applied(h))
//              and start broadcasting subsequent writes to the new
//              follower set.
//   Demoted  — stop owning tenant h; Follower takes over (the
//              erstwhile primary is now just another follower).

#![cfg(all(feature = "cluster", not(feature = "no_std")))]

use super::placement::Placement;
use super::{NodeId, Membership};
use super::replication::Replicator;
use alloc::vec::Vec;
use alloc::string::String;
use hashbrown::HashMap;

/// Transitions observed by `FailoverDetector::observe`.
///
/// Promoted: this node is the new primary for `tenant`. `from` is the
///   previous primary id (or `None` if the detector has no prior
///   record — e.g., first observation after boot).
/// Demoted: this node was the primary for `tenant`; `to` is the new
///   owner. Caller should stop issuing writes locally and start
///   following `to`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromotionEvent {
    Promoted { tenant: u32, from: Option<NodeId> },
    Demoted  { tenant: u32, to: NodeId },
}

/// Local failover observer.
///
/// Holds per-tenant "last seen primary" so it can diff against an
/// incoming Placement. Construct with the local node's id; feed
/// `observe(placement, tenants)` after every Placement rebuild the
/// runtime cares about.
pub struct FailoverDetector {
    self_id: NodeId,
    last_primary: HashMap<u32, NodeId>,
}

impl FailoverDetector {
    pub fn new(self_id: NodeId) -> Self {
        Self { self_id, last_primary: HashMap::new() }
    }

    pub fn self_id(&self) -> &NodeId { &self.self_id }

    /// Last-recorded primary for `tenant` (None if the detector has
    /// never observed this tenant).
    pub fn primary_for(&self, tenant: u32) -> Option<&NodeId> {
        self.last_primary.get(&tenant)
    }

    /// Compare current `placement` to the stored view across the given
    /// `tenants`. Returns a `PromotionEvent` for any transition whose
    /// "after" side involves self (promoted) or whose "before" side
    /// was self (demoted). Ignores transitions between two other
    /// nodes — this detector only reports what the local runtime must
    /// react to.
    ///
    /// Tenants whose current placement is None (empty ring) are
    /// ignored: there's no primary to promote/demote. A cluster with
    /// zero Alive members is down; that's a higher-level alarm.
    pub fn observe(&mut self, placement: &Placement, tenants: &[u32]) -> Vec<PromotionEvent> {
        let mut events = Vec::new();
        for &t in tenants {
            let now = match placement.primary_for(t) {
                Some(id) => id.clone(),
                None => continue,
            };
            let before = self.last_primary.get(&t).cloned();
            match &before {
                Some(prev) if *prev == now => { /* no change */ }
                Some(prev) if *prev == self.self_id && now != self.self_id => {
                    events.push(PromotionEvent::Demoted { tenant: t, to: now.clone() });
                }
                _ if now == self.self_id => {
                    events.push(PromotionEvent::Promoted {
                        tenant: t,
                        from: before,
                    });
                }
                _ => { /* transition between two other nodes */ }
            }
            self.last_primary.insert(t, now);
        }
        events
    }

    /// Convenience: rebuild `placement` from `membership` and observe
    /// in one call. The caller still supplies the tenant list.
    pub fn observe_membership(
        &mut self,
        membership: &Membership,
        virtual_nodes: usize,
        tenants: &[u32],
    ) -> Vec<PromotionEvent> {
        let p = Placement::from_membership(membership, virtual_nodes);
        self.observe(&p, tenants)
    }
}

/// Handle a promotion by aligning the primary's generation counter to
/// the last-applied generation the node has as a follower. Prevents
/// split-brain where the newly-promoted primary reissues generation 1
/// while some other node still believes itself to be primary at a
/// higher gen — the follower-side HWM is the lower bound we have to
/// clear.
impl Replicator {
    /// Set `tenant`'s generation to `applied`; the next
    /// `bump_and_encode` emits `applied + 1`. Calling this with a
    /// value lower than the current generation is a no-op — we only
    /// ever advance the counter so previously-emitted Acks remain
    /// valid.
    pub fn resume_from(&mut self, tenant: u32, applied: u64) {
        let cur = self.generation(tenant);
        if applied > cur {
            self.set_generation_raw(tenant, applied);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{Delta, NodeMeta, State};
    use std::net::SocketAddr;

    fn mk_membership(self_id: &str) -> Membership {
        Membership::new(
            self_id.to_string(),
            "127.0.0.1:9000".parse::<SocketAddr>().unwrap(),
        )
    }

    fn merge_alive(m: &mut Membership, id: &str, port: u16) {
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        m.merge(Delta { id: id.to_string(), addr, incarnation: 0, state: State::Alive });
    }

    fn merge_dead(m: &mut Membership, id: &str, port: u16) {
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        m.merge(Delta { id: id.to_string(), addr, incarnation: 1, state: State::Dead });
    }

    /// Pick a tenant handle that lands on node `target` given this
    /// membership. Brute-force because our handle space is u32.
    fn tenant_on(placement: &Placement, target: &str) -> u32 {
        for h in 0u32..10_000 {
            if placement.primary_for(h).map(|s| s.as_str()) == Some(target) {
                return h;
            }
        }
        panic!("no handle in 0..10000 maps to {target}");
    }

    #[test]
    fn first_observation_emits_promoted_for_self_owned_tenants() {
        let mut m = mk_membership("A");
        merge_alive(&mut m, "B", 9001);
        let p = Placement::from_membership(&m, 128);
        let mut det = FailoverDetector::new("A".to_string());

        let h_a = tenant_on(&p, "A");
        let h_b = tenant_on(&p, "B");
        let ev = det.observe(&p, &[h_a, h_b]);

        // Only the A-owned tenant produces a Promoted event from A's
        // perspective. The B-owned tenant has nothing to do with A.
        assert!(ev.iter().any(|e| matches!(e,
            PromotionEvent::Promoted { tenant, from } if *tenant == h_a && from.is_none()
        )));
        assert!(!ev.iter().any(|e| matches!(e,
            PromotionEvent::Promoted { tenant, .. } if *tenant == h_b
        )));
    }

    #[test]
    fn idempotent_observe_emits_nothing_second_time() {
        let mut m = mk_membership("A");
        merge_alive(&mut m, "B", 9001);
        let p = Placement::from_membership(&m, 64);
        let mut det = FailoverDetector::new("A".to_string());

        let tenants: Vec<u32> = (0u32..20).collect();
        let first = det.observe(&p, &tenants);
        let second = det.observe(&p, &tenants);
        assert!(!first.is_empty(), "expect at least one self-owned promotion first time");
        assert!(second.is_empty(), "second observe with unchanged placement must be quiet");
    }

    #[test]
    fn promotion_on_primary_death() {
        // Setup: three-node cluster, A / B / C all Alive. Pick a
        // tenant that lands on B. Kill B. Re-observe — whichever node
        // now owns that tenant should see a Promoted event.
        let mut m = mk_membership("A");
        merge_alive(&mut m, "B", 9001);
        merge_alive(&mut m, "C", 9002);
        let p_before = Placement::from_membership(&m, 128);
        let h = tenant_on(&p_before, "B");

        let mut det_a = FailoverDetector::new("A".to_string());
        let mut det_c = FailoverDetector::new("C".to_string());
        // Prime both detectors with the pre-failure view.
        det_a.observe(&p_before, &[h]);
        det_c.observe(&p_before, &[h]);

        merge_dead(&mut m, "B", 9001);
        let p_after = Placement::from_membership(&m, 128);

        let new_owner = p_after.primary_for(h).unwrap().clone();
        assert!(new_owner == "A" || new_owner == "C",
            "dead B's tenants should re-home to A or C; got {new_owner}");

        let ev_a = det_a.observe(&p_after, &[h]);
        let ev_c = det_c.observe(&p_after, &[h]);

        // Whichever detector owns the now-owner must see a Promoted
        // event with `from = Some("B")`. The other sees nothing (it
        // wasn't the old primary, isn't the new primary).
        let expected_detector_events = if new_owner == "A" { &ev_a } else { &ev_c };
        let other_detector_events = if new_owner == "A" { &ev_c } else { &ev_a };

        assert!(expected_detector_events.iter().any(|e| matches!(e,
            PromotionEvent::Promoted { tenant, from: Some(prev) }
            if *tenant == h && prev == "B"
        )), "expected owner to see Promoted(from=B); got {expected_detector_events:?}");
        assert!(other_detector_events.is_empty(),
            "non-owner detector should be silent; got {other_detector_events:?}");
    }

    #[test]
    fn demotion_when_a_new_node_joins_and_takes_over() {
        // A runs alone → A owns every tenant. Add B → consistent
        // hashing moves ~half the tenants to B. A's detector should
        // see Demoted events for the tenants it lost.
        let mut m = mk_membership("A");
        let p1 = Placement::from_membership(&m, 128);
        let mut det_a = FailoverDetector::new("A".to_string());
        let tenants: Vec<u32> = (0u32..200).collect();
        // Prime: A is primary for every tenant.
        det_a.observe(&p1, &tenants);

        merge_alive(&mut m, "B", 9001);
        let p2 = Placement::from_membership(&m, 128);

        let ev = det_a.observe(&p2, &tenants);
        let demoted_count = ev.iter()
            .filter(|e| matches!(e, PromotionEvent::Demoted { to, .. } if to == "B"))
            .count();
        // ~half should move to B. Tolerance: between 10% and 90%.
        assert!((20..=180).contains(&demoted_count),
            "Demoted count {demoted_count}/200 out of expected band");
        // No Promoted events — A doesn't own anything new.
        assert!(!ev.iter().any(|e| matches!(e, PromotionEvent::Promoted { .. })),
            "A gains no tenants when B joins");
    }

    #[test]
    fn transitions_between_two_other_nodes_are_silent() {
        // A is the observer. Tenants owned by B move to C after B
        // dies — A is neither the old nor the new primary and should
        // not see events for those tenants.
        let mut m = mk_membership("A");
        merge_alive(&mut m, "B", 9001);
        merge_alive(&mut m, "C", 9002);
        let p1 = Placement::from_membership(&m, 128);
        let mut det = FailoverDetector::new("A".to_string());

        // Find a tenant that lands on B in p1 and on C in p2 (after B
        // dies). Use deterministic brute-force.
        merge_dead(&mut m, "B", 9001);
        let p2 = Placement::from_membership(&m, 128);
        let mut t_bc: Option<u32> = None;
        for h in 0u32..10_000 {
            if p1.primary_for(h).map(|s| s.as_str()) == Some("B")
                && p2.primary_for(h).map(|s| s.as_str()) == Some("C")
            {
                t_bc = Some(h);
                break;
            }
        }
        let t_bc = t_bc.expect("expected a B→C handoff tenant");

        det.observe(&p1, &[t_bc]);
        let ev = det.observe(&p2, &[t_bc]);
        assert!(ev.is_empty(),
            "A is neither old nor new primary for tenant {t_bc}; events {ev:?}");
    }

    #[test]
    fn resume_from_aligns_generation_to_applied() {
        use crate::cluster::replication::{Follower, Replicator, ReplMsg};
        use crate::ast::Object;

        // Simulate: old primary issued gens 1..5. New primary (this
        // node) has been tracking as a follower and has applied gen 5.
        // After promotion, its Replicator should emit gen 6 on the
        // next bump, not gen 1.
        let mut old_primary = Replicator::new();
        let mut follower = Follower::new();
        let obj = Object::atom("v");
        for _ in 0..5 {
            let msg = old_primary.bump_and_encode(42, &obj);
            let _ = follower.receive(msg, |_, _| {});
        }
        assert_eq!(follower.applied(42), 5);

        // Promote: the local node (follower until now) takes over and
        // calls resume_from with its own applied HWM.
        let mut new_primary = Replicator::new();
        new_primary.resume_from(42, follower.applied(42));
        assert_eq!(new_primary.generation(42), 5);

        let next = new_primary.bump_and_encode(42, &obj);
        match next {
            ReplMsg::State { tenant: 42, generation: 6, .. } => {}
            other => panic!("expected gen 6 after resume; got {other:?}"),
        }
    }

    #[test]
    fn resume_from_does_not_regress_counter() {
        use crate::cluster::replication::Replicator;
        use crate::ast::Object;

        let mut r = Replicator::new();
        let obj = Object::atom("v");
        for _ in 0..10 { let _ = r.bump_and_encode(1, &obj); }
        assert_eq!(r.generation(1), 10);

        // A lower applied value must not rewind.
        r.resume_from(1, 3);
        assert_eq!(r.generation(1), 10, "resume_from must be monotonic");
    }
}
