// crates/arest/src/cluster/replication.rs
//
// Cluster-3: per-tenant state replication from primary → followers.
//
// Primary serialises the tenant's post-write state via `freeze::freeze`,
// stamps it with a monotonic generation counter, and hands the
// resulting `ReplMsg::State` to the Transport. Followers track the
// last-applied generation per tenant and accept only strictly higher
// generations; out-of-order / duplicate deltas get Nack'd.
//
// This module is the wire + state layer. Wiring the primary's "after
// commit" hook and the follower's "apply to CompiledState" side-effect
// to the Cluster-1 gossip transport is the Cluster-4 (failover) + the
// cluster runtime's job — keeping the apply callback as a closure on
// `Follower::receive` lets tests drive the full protocol without
// standing up real tenants.
//
// Generation semantics:
//   - Primary increments generation by 1 on every state broadcast.
//   - Follower accepts gen' > gen_applied; rejects gen' <= gen_applied.
//   - Rejection includes `have` so a catching-up follower can ask a
//     primary or peer for the delta range it's missing (Cluster-4
//     territory — not implemented here).
//
// Coarse-grained (full-state) replication today — every write ships
// the entire freeze image. A future optimisation can switch to cell-
// level deltas once the CompiledState commit path exposes the diff
// directly; wire format is versioned so that swap is additive.

#![cfg(all(feature = "cluster", not(feature = "no_std")))]

use crate::ast::Object;
use crate::freeze::{freeze, thaw};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use hashbrown::HashMap;

/// Wire messages for state replication.
///
/// `State` is primary → follower: "this is tenant h at generation g".
/// `Ack` is follower → primary: "I applied it."
/// `Nack` is follower → primary: "I already have ≥ `have`; send me
/// deltas from `have+1` onwards (Cluster-4 range-request lives here)."
/// `Reject` is follower → primary: "thaw succeeded but the state
/// failed alethic validation against its own def graph (D4 / #706);
/// don't replay this generation — fix the upstream commit." `reason`
/// carries the joined alethic violation messages so the primary can
/// surface them in logs / dashboards without a side-channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplMsg {
    State  { tenant: u32, generation: u64, bytes: Vec<u8> },
    Ack    { tenant: u32, generation: u64 },
    Nack   { tenant: u32, generation: u64, have: u64 },
    Reject { tenant: u32, generation: u64, have: u64, reason: String },
}

// Wire tags. u8 so decoder doesn't need to branch on alignment.
const TAG_STATE: u8 = 0x01;
const TAG_ACK: u8 = 0x02;
const TAG_NACK: u8 = 0x03;
const TAG_REJECT: u8 = 0x04;

/// Magic for the replication frame. Distinct from the Object freeze
/// magic so a decoder that receives the wrong message type fails fast
/// instead of spending CPU walking a bogus Object tree.
const REPL_MAGIC: &[u8] = b"REPL\x01";

impl ReplMsg {
    /// Encode to a flat byte frame: [REPL_MAGIC][tag][body]. Body
    /// layout depends on tag — documented on each branch.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(64);
        buf.extend_from_slice(REPL_MAGIC);
        match self {
            ReplMsg::State { tenant, generation, bytes } => {
                // TAG_STATE | u32 tenant | u64 generation | u32 len | bytes
                buf.push(TAG_STATE);
                buf.extend_from_slice(&tenant.to_le_bytes());
                buf.extend_from_slice(&generation.to_le_bytes());
                buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                buf.extend_from_slice(bytes);
            }
            ReplMsg::Ack { tenant, generation } => {
                buf.push(TAG_ACK);
                buf.extend_from_slice(&tenant.to_le_bytes());
                buf.extend_from_slice(&generation.to_le_bytes());
            }
            ReplMsg::Nack { tenant, generation, have } => {
                buf.push(TAG_NACK);
                buf.extend_from_slice(&tenant.to_le_bytes());
                buf.extend_from_slice(&generation.to_le_bytes());
                buf.extend_from_slice(&have.to_le_bytes());
            }
            ReplMsg::Reject { tenant, generation, have, reason } => {
                // TAG_REJECT | u32 tenant | u64 generation | u64 have | u32 len | reason
                buf.push(TAG_REJECT);
                buf.extend_from_slice(&tenant.to_le_bytes());
                buf.extend_from_slice(&generation.to_le_bytes());
                buf.extend_from_slice(&have.to_le_bytes());
                buf.extend_from_slice(&(reason.len() as u32).to_le_bytes());
                buf.extend_from_slice(reason.as_bytes());
            }
        }
        buf
    }

    /// Decode a frame produced by `encode`. Returns Err on bad magic,
    /// truncation, unknown tag, or trailing garbage.
    pub fn decode(frame: &[u8]) -> Result<Self, String> {
        if frame.len() < REPL_MAGIC.len() + 1
            || &frame[..REPL_MAGIC.len()] != REPL_MAGIC
        {
            return Err("bad magic — not a REPL frame".to_string());
        }
        let mut i = REPL_MAGIC.len();
        let tag = frame[i]; i += 1;
        match tag {
            TAG_STATE => {
                if frame.len() < i + 4 + 8 + 4 {
                    return Err("truncated State header".to_string());
                }
                let tenant = u32::from_le_bytes(frame[i..i+4].try_into().unwrap()); i += 4;
                let generation = u64::from_le_bytes(frame[i..i+8].try_into().unwrap()); i += 8;
                let len = u32::from_le_bytes(frame[i..i+4].try_into().unwrap()) as usize; i += 4;
                if frame.len() < i + len {
                    return Err("truncated State body".to_string());
                }
                let bytes = frame[i..i+len].to_vec();
                i += len;
                if i != frame.len() {
                    return Err("trailing bytes after State".to_string());
                }
                Ok(ReplMsg::State { tenant, generation, bytes })
            }
            TAG_ACK => {
                if frame.len() != i + 4 + 8 {
                    return Err("wrong Ack frame size".to_string());
                }
                let tenant = u32::from_le_bytes(frame[i..i+4].try_into().unwrap()); i += 4;
                let generation = u64::from_le_bytes(frame[i..i+8].try_into().unwrap());
                let _ = i;
                Ok(ReplMsg::Ack { tenant, generation })
            }
            TAG_NACK => {
                if frame.len() != i + 4 + 8 + 8 {
                    return Err("wrong Nack frame size".to_string());
                }
                let tenant = u32::from_le_bytes(frame[i..i+4].try_into().unwrap()); i += 4;
                let generation = u64::from_le_bytes(frame[i..i+8].try_into().unwrap()); i += 8;
                let have = u64::from_le_bytes(frame[i..i+8].try_into().unwrap());
                let _ = i;
                Ok(ReplMsg::Nack { tenant, generation, have })
            }
            TAG_REJECT => {
                if frame.len() < i + 4 + 8 + 8 + 4 {
                    return Err("truncated Reject header".to_string());
                }
                let tenant = u32::from_le_bytes(frame[i..i+4].try_into().unwrap()); i += 4;
                let generation = u64::from_le_bytes(frame[i..i+8].try_into().unwrap()); i += 8;
                let have = u64::from_le_bytes(frame[i..i+8].try_into().unwrap()); i += 8;
                let len = u32::from_le_bytes(frame[i..i+4].try_into().unwrap()) as usize; i += 4;
                if frame.len() < i + len {
                    return Err("truncated Reject reason".to_string());
                }
                let reason = core::str::from_utf8(&frame[i..i+len])
                    .map_err(|_| "Reject reason not UTF-8".to_string())?
                    .to_string();
                i += len;
                if i != frame.len() {
                    return Err("trailing bytes after Reject".to_string());
                }
                Ok(ReplMsg::Reject { tenant, generation, have, reason })
            }
            t => Err(alloc::format!("unknown repl tag {t:#x}")),
        }
    }
}

/// Primary-side generation tracker. Hands out monotonically-increasing
/// generation numbers per tenant so followers can detect out-of-order /
/// duplicate deliveries and the future range-request path has a total
/// order to reason about.
pub struct Replicator {
    generations: HashMap<u32, u64>,
}

impl Default for Replicator {
    fn default() -> Self { Self::new() }
}

impl Replicator {
    pub fn new() -> Self {
        Self { generations: HashMap::new() }
    }

    /// Current generation for `tenant` (0 if never written).
    pub fn generation(&self, tenant: u32) -> u64 {
        self.generations.get(&tenant).copied().unwrap_or(0)
    }

    /// Bump this tenant's generation and emit a `State` message
    /// carrying the frozen bytes. Caller is responsible for feeding
    /// the returned message into Transport — this method does not
    /// touch the wire.
    pub fn bump_and_encode(&mut self, tenant: u32, state: &Object) -> ReplMsg {
        let g = self.generations.entry(tenant).or_insert(0);
        *g += 1;
        ReplMsg::State {
            tenant,
            generation: *g,
            bytes: freeze(state),
        }
    }

    /// Observe a follower's Ack/Nack. No-op today but wired so
    /// Cluster-4 can hook failover / retry decisions here without the
    /// primary's caller needing to know about internal state.
    pub fn observe(&mut self, _msg: &ReplMsg) { /* reserved */ }

    /// Cluster-4 uses this to align the counter to a follower's HWM
    /// on promotion. Narrow crate-only API so external callers must
    /// go through `resume_from` (which enforces monotonicity).
    pub(crate) fn set_generation_raw(&mut self, tenant: u32, value: u64) {
        self.generations.insert(tenant, value);
    }
}

/// Follower-side apply loop.
///
/// Tracks last-applied generation per tenant. `receive` validates the
/// incoming State, calls the supplied apply closure on fresh deltas,
/// and returns the Ack/Nack that should go back to the primary. The
/// apply closure is where a real follower would `tenant_lock(h)` and
/// `st.replace_d(obj)` — tests can substitute a capture to verify what
/// got applied.
pub struct Follower {
    applied: HashMap<u32, u64>,
}

impl Default for Follower {
    fn default() -> Self { Self::new() }
}

impl Follower {
    pub fn new() -> Self {
        Self { applied: HashMap::new() }
    }

    /// Last-applied generation for `tenant` (0 if never applied).
    pub fn applied(&self, tenant: u32) -> u64 {
        self.applied.get(&tenant).copied().unwrap_or(0)
    }

    /// Feed an incoming message. For `State` messages, calls
    /// `apply_state(tenant, obj)` when the generation advances and
    /// returns `Ack`; otherwise returns `Nack` with the current
    /// applied generation and does not invoke `apply_state`. For
    /// non-State messages returns them unchanged — a follower
    /// receiving an Ack or Nack is a bug, but we prefer surfacing it
    /// to the caller over panicking.
    ///
    /// Apply closure signature is `(u32, Object)` so a test can
    /// capture-by-move for assertion; production wires it to
    /// `crate::tenant_lock(...).write().replace_d(obj)`.
    pub fn receive<F: FnOnce(u32, Object)>(
        &mut self,
        msg: ReplMsg,
        apply_state: F,
    ) -> ReplMsg {
        match msg {
            ReplMsg::State { tenant, generation, bytes } => {
                let cur = self.applied(tenant);
                if generation <= cur {
                    return ReplMsg::Nack { tenant, generation, have: cur };
                }
                let obj = match thaw(&bytes) {
                    Ok(o) => o,
                    // Bytes are broken — Nack with our current high
                    // water mark so the primary retries / resnapshots.
                    Err(_) => return ReplMsg::Nack { tenant, generation, have: cur },
                };

                // D4 (#706): validate the thawed def state against
                // its own validate Func before applying. The wire is
                // mTLS-checked (Cluster-5), but a primary that itself
                // accepted bad facts (D1/D2/D3/T1 boundaries) would
                // happily replicate the bad state to every follower.
                // Run the same `validate` dispatch the local write
                // path uses; if any alethic violation surfaces, Reject
                // with the joined messages so the primary can pinpoint
                // the upstream commit. Validate Def absent = no-op:
                // boot snapshots and pre-validate states still apply.
                let ctx = crate::ast::encode_eval_context_state("", None, &obj);
                let violation_obj = crate::ast::apply(
                    &crate::ast::Func::Def("validate".to_string()),
                    &ctx,
                    &obj,
                );
                let violations = crate::ast::decode_violations(&violation_obj);
                let alethic: Vec<_> = violations.iter().filter(|v| v.alethic).collect();
                if !alethic.is_empty() {
                    let reason = alethic.iter()
                        .map(|v| v.constraint_text.as_str())
                        .collect::<Vec<_>>()
                        .join("; ");
                    return ReplMsg::Reject { tenant, generation, have: cur, reason };
                }

                apply_state(tenant, obj);
                self.applied.insert(tenant, generation);
                ReplMsg::Ack { tenant, generation }
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Object;
    use std::cell::RefCell;
    use std::rc::Rc;

    // Helper: capture what the follower applied so tests can inspect.
    fn capturing_apply() -> (Rc<RefCell<Vec<(u32, Object)>>>, impl Fn(u32, Object)) {
        let log = Rc::new(RefCell::new(Vec::<(u32, Object)>::new()));
        let log_cb = Rc::clone(&log);
        (log, move |t, o| log_cb.borrow_mut().push((t, o)))
    }

    #[test]
    fn wire_roundtrip_state() {
        let s = ReplMsg::State {
            tenant: 7,
            generation: 42,
            bytes: b"some frozen bytes".to_vec(),
        };
        let bytes = s.encode();
        let back = ReplMsg::decode(&bytes).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn wire_roundtrip_ack_and_nack() {
        let a = ReplMsg::Ack { tenant: 1, generation: 5 };
        assert_eq!(ReplMsg::decode(&a.encode()).unwrap(), a);

        let n = ReplMsg::Nack { tenant: 2, generation: 9, have: 7 };
        assert_eq!(ReplMsg::decode(&n.encode()).unwrap(), n);
    }

    #[test]
    fn wire_roundtrip_reject() {
        // Empty reason and non-empty reason both round-trip; non-ASCII
        // payload exercises the UTF-8 length-prefix path.
        let empty = ReplMsg::Reject {
            tenant: 4, generation: 11, have: 10, reason: String::new(),
        };
        assert_eq!(ReplMsg::decode(&empty.encode()).unwrap(), empty);

        let with_reason = ReplMsg::Reject {
            tenant: 4, generation: 11, have: 10,
            reason: "alethic: each Order has exactly one customer ✗".to_string(),
        };
        assert_eq!(ReplMsg::decode(&with_reason.encode()).unwrap(), with_reason);
    }

    #[test]
    fn wire_rejects_bad_magic() {
        let mut bytes = ReplMsg::Ack { tenant: 0, generation: 1 }.encode();
        bytes[0] = b'X';
        assert!(ReplMsg::decode(&bytes).is_err());
    }

    #[test]
    fn wire_rejects_truncated_state() {
        let bytes = ReplMsg::State {
            tenant: 0, generation: 1, bytes: b"xx".to_vec(),
        }.encode();
        assert!(ReplMsg::decode(&bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn replicator_generations_advance_per_tenant() {
        let mut r = Replicator::new();
        let obj = Object::atom("x");

        let m1 = r.bump_and_encode(10, &obj);
        let m2 = r.bump_and_encode(10, &obj);
        let m3 = r.bump_and_encode(20, &obj);

        match (m1, m2, m3) {
            (ReplMsg::State { tenant: 10, generation: 1, .. },
             ReplMsg::State { tenant: 10, generation: 2, .. },
             ReplMsg::State { tenant: 20, generation: 1, .. }) => {}
            other => panic!("unexpected generations: {other:?}"),
        }
        assert_eq!(r.generation(10), 2);
        assert_eq!(r.generation(20), 1);
        assert_eq!(r.generation(99), 0);
    }

    #[test]
    fn follower_applies_increasing_generations() {
        let mut f = Follower::new();
        let (log, apply) = capturing_apply();

        let obj_a = Object::atom("a");
        let obj_b = Object::atom("b");
        let obj_c = Object::atom("c");

        let m1 = ReplMsg::State { tenant: 5, generation: 1, bytes: freeze(&obj_a) };
        let m2 = ReplMsg::State { tenant: 5, generation: 2, bytes: freeze(&obj_b) };
        let m3 = ReplMsg::State { tenant: 5, generation: 3, bytes: freeze(&obj_c) };

        let r1 = f.receive(m1, &apply);
        let r2 = f.receive(m2, &apply);
        let r3 = f.receive(m3, &apply);

        assert!(matches!(r1, ReplMsg::Ack { tenant: 5, generation: 1 }));
        assert!(matches!(r2, ReplMsg::Ack { tenant: 5, generation: 2 }));
        assert!(matches!(r3, ReplMsg::Ack { tenant: 5, generation: 3 }));

        let applied = log.borrow();
        assert_eq!(applied.len(), 3);
        assert_eq!(applied[0], (5u32, obj_a));
        assert_eq!(applied[1], (5u32, obj_b));
        assert_eq!(applied[2], (5u32, obj_c));
        assert_eq!(f.applied(5), 3);
    }

    #[test]
    fn follower_nacks_stale_generation() {
        let mut f = Follower::new();
        let (log, apply) = capturing_apply();

        let obj = Object::atom("x");
        let m2 = ReplMsg::State { tenant: 1, generation: 2, bytes: freeze(&obj) };
        let stale = ReplMsg::State { tenant: 1, generation: 1, bytes: freeze(&obj) };

        f.receive(m2, &apply);
        assert_eq!(log.borrow().len(), 1);

        let r = f.receive(stale, &apply);
        match r {
            ReplMsg::Nack { tenant: 1, generation: 1, have: 2 } => {}
            other => panic!("expected Nack(have=2), got {other:?}"),
        }
        // No second apply.
        assert_eq!(log.borrow().len(), 1);
        assert_eq!(f.applied(1), 2);
    }

    #[test]
    fn follower_nacks_duplicate_generation() {
        let mut f = Follower::new();
        let (log, apply) = capturing_apply();

        let obj = Object::atom("x");
        let m = ReplMsg::State { tenant: 1, generation: 3, bytes: freeze(&obj) };
        let dup = m.clone();

        let _ = f.receive(m, &apply);
        let r = f.receive(dup, &apply);
        match r {
            ReplMsg::Nack { tenant: 1, generation: 3, have: 3 } => {}
            other => panic!("expected Nack(have=3), got {other:?}"),
        }
        assert_eq!(log.borrow().len(), 1, "duplicate must not re-apply");
    }

    #[test]
    fn follower_nacks_malformed_state_without_advancing() {
        let mut f = Follower::new();
        let (log, apply) = capturing_apply();

        let bad = ReplMsg::State {
            tenant: 1,
            generation: 1,
            bytes: b"not a valid freeze image".to_vec(),
        };
        let r = f.receive(bad, &apply);
        match r {
            ReplMsg::Nack { tenant: 1, generation: 1, have: 0 } => {}
            other => panic!("expected Nack(have=0), got {other:?}"),
        }
        assert!(log.borrow().is_empty());
        assert_eq!(f.applied(1), 0, "broken bytes must not advance the HWM");
    }

    #[test]
    fn follower_rejects_alethic_violation_without_advancing() {
        // D4 (#706): build a def state whose `validate` Func returns
        // a single alethic violation. The follower should refuse to
        // apply, return ReplMsg::Reject with the joined reason, and
        // leave its high water mark untouched.
        use crate::ast::{Func, Object as Obj, defs_to_state};

        let violation = Obj::seq(vec![Obj::seq(vec![
            Obj::atom("test_constraint"),
            Obj::atom("Customer must exist for every Order"),
            Obj::atom("Order o-1 has no Customer"),
        ])]);
        let validate_func = Func::constant(violation);
        let defs = vec![("validate".to_string(), validate_func)];
        let bad_state = defs_to_state(&defs, &Obj::phi());

        let mut f = Follower::new();
        let (log, apply) = capturing_apply();

        let msg = ReplMsg::State { tenant: 7, generation: 1, bytes: freeze(&bad_state) };
        let r = f.receive(msg, &apply);
        match r {
            ReplMsg::Reject { tenant: 7, generation: 1, have: 0, reason } => {
                assert!(
                    reason.contains("Customer must exist for every Order"),
                    "Reject reason should carry the violation text, got: {reason}",
                );
            }
            other => panic!("expected Reject, got {other:?}"),
        }
        assert!(log.borrow().is_empty(), "rejected state must not be applied");
        assert_eq!(f.applied(7), 0, "rejected state must not advance the HWM");
    }

    #[test]
    fn primary_to_follower_end_to_end() {
        let mut primary = Replicator::new();
        let mut follower = Follower::new();
        let (log, apply) = capturing_apply();

        let states = [
            Object::atom("v1"),
            Object::atom("v2"),
            Object::atom("v3"),
        ];

        for s in &states {
            let msg = primary.bump_and_encode(42, s);
            // Wire round-trip — simulates sending bytes over Transport.
            let bytes = msg.encode();
            let back = ReplMsg::decode(&bytes).unwrap();
            let ack = follower.receive(back, &apply);
            assert!(matches!(ack, ReplMsg::Ack { tenant: 42, .. }));
            primary.observe(&ack);
        }

        let applied = log.borrow();
        assert_eq!(applied.len(), 3);
        for (i, s) in states.iter().enumerate() {
            assert_eq!(applied[i].1, *s);
        }
        assert_eq!(primary.generation(42), 3);
        assert_eq!(follower.applied(42), 3);
    }

    #[test]
    fn follower_tenants_are_independent() {
        let mut f = Follower::new();
        let (log, apply) = capturing_apply();
        let obj = Object::atom("x");

        f.receive(
            ReplMsg::State { tenant: 1, generation: 5, bytes: freeze(&obj) },
            &apply,
        );
        // Tenant 2 starting from scratch — gen 1 must apply even
        // though tenant 1's HWM is 5.
        let r = f.receive(
            ReplMsg::State { tenant: 2, generation: 1, bytes: freeze(&obj) },
            &apply,
        );
        assert!(matches!(r, ReplMsg::Ack { tenant: 2, generation: 1 }));
        assert_eq!(log.borrow().len(), 2);
        assert_eq!(f.applied(1), 5);
        assert_eq!(f.applied(2), 1);
    }
}
