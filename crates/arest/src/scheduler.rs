// crates/arest/src/scheduler.rs
//
// Three-lane command queue. Callers enqueue work with a declared
// priority; pop() drains the Alethic lane first, then Deontic, then
// ReadOnly. Within a lane, ordering is strictly FIFO.
//
// The queue is a plain data structure — not thread-safe on its own.
// Callers that share it across threads wrap it in a Mutex or similar
// and signal via a Condvar / channel. Keeping this single-threaded
// means the same type works on every target (Cloudflare Workers,
// WASM in the browser, native multi-threaded), with the concurrency
// primitives chosen per deployment.
//
// Why these three lanes:
//   - Alethic: writes with alethic-constraint checks. If the result
//     of the op would violate an alethic constraint (`each`, `exactly
//     one`, value-range), AREST rejects it — so these carry the
//     strongest correctness invariants. Giving them highest priority
//     lets a flood of reads never delay a must-be-correct write.
//   - Deontic: writes that may surface deontic ("it is obligatory",
//     "it is forbidden") violations but still commit. Their value
//     is forward progress, not immediate correctness; queue behind
//     alethic work.
//   - ReadOnly: queries / projections / explain / audit. Never
//     change state; fill idle cycles behind everything else.
//
// The `classify_priority` helper is a conservative default keyed on
// the system-verb prefix; callers with per-op context (e.g. a
// transaction coordinator that knows this write enforces an alethic
// invariant) can override at enqueue time.

use alloc::collections::VecDeque;
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

/// Priority lane. Variants are declared in descending-priority order:
/// `Alethic` drains before `Deontic`, which drains before `ReadOnly`.
/// The derived `Ord` reflects that — `Alethic < Deontic < ReadOnly` —
/// so callers can compare priorities with standard operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    Alethic,
    Deontic,
    ReadOnly,
}

/// Three-lane FIFO queue. `T` is the command payload — typically a
/// struct carrying the handle, verb key, input, and a response
/// channel, but this type is agnostic.
#[derive(Debug)]
pub struct CommandQueue<T> {
    alethic: VecDeque<T>,
    deontic: VecDeque<T>,
    readonly: VecDeque<T>,
}

impl<T> Default for CommandQueue<T> {
    fn default() -> Self {
        Self {
            alethic: VecDeque::new(),
            deontic: VecDeque::new(),
            readonly: VecDeque::new(),
        }
    }
}

impl<T> CommandQueue<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enqueue an item on the given priority lane. O(1) amortized.
    pub fn push(&mut self, priority: Priority, item: T) {
        self.lane_mut(priority).push_back(item);
    }

    /// Dequeue the next item by priority: Alethic first, then Deontic,
    /// then ReadOnly. Within a lane, FIFO. Returns `None` when all
    /// three lanes are empty.
    pub fn pop(&mut self) -> Option<(Priority, T)> {
        if let Some(item) = self.alethic.pop_front() {
            return Some((Priority::Alethic, item));
        }
        if let Some(item) = self.deontic.pop_front() {
            return Some((Priority::Deontic, item));
        }
        self.readonly.pop_front().map(|item| (Priority::ReadOnly, item))
    }

    /// Peek at the next item without dequeuing. Same priority order
    /// as `pop`. Returns a reference that's valid until the queue is
    /// mutated.
    pub fn peek(&self) -> Option<(Priority, &T)> {
        if let Some(item) = self.alethic.front() {
            return Some((Priority::Alethic, item));
        }
        if let Some(item) = self.deontic.front() {
            return Some((Priority::Deontic, item));
        }
        self.readonly.front().map(|item| (Priority::ReadOnly, item))
    }

    pub fn is_empty(&self) -> bool {
        self.alethic.is_empty() && self.deontic.is_empty() && self.readonly.is_empty()
    }

    /// Total items across all three lanes.
    pub fn len(&self) -> usize {
        self.alethic.len() + self.deontic.len() + self.readonly.len()
    }

    /// Item count in a specific lane. Useful for back-pressure /
    /// observability checks.
    pub fn len_of(&self, priority: Priority) -> usize {
        self.lane(priority).len()
    }

    fn lane(&self, priority: Priority) -> &VecDeque<T> {
        match priority {
            Priority::Alethic => &self.alethic,
            Priority::Deontic => &self.deontic,
            Priority::ReadOnly => &self.readonly,
        }
    }

    fn lane_mut(&mut self, priority: Priority) -> &mut VecDeque<T> {
        match priority {
            Priority::Alethic => &mut self.alethic,
            Priority::Deontic => &mut self.deontic,
            Priority::ReadOnly => &mut self.readonly,
        }
    }
}

/// Conservative default priority for a system verb.
///
///   - Known-read verbs (list / get / query / debug / audit /
///     explain / verify_signature / snapshots) → `ReadOnly`.
///   - `snapshot` / `rollback` → `Alethic` — they mutate without
///     running validators, so their correctness is implicit in the
///     op itself rather than in constraint evaluation.
///   - Everything else (`compile`, `create:*`, `update:*`,
///     `transition:*`, user-defined defs) → `Deontic`. These run
///     validate and may surface deontic violations on commit.
///
/// Callers who know the op's semantic guarantees more precisely
/// (e.g. "this `update:Order` is being submitted as part of an
/// alethic-invariant commit") should pass their own `Priority` to
/// `CommandQueue::push` rather than relying on this default.
pub fn classify_priority(key: &str) -> Priority {
    if matches!(
        key,
        "debug" | "audit" | "verify_signature" | "snapshots"
    ) || key.starts_with("list:")
        || key.starts_with("get:")
        || key.starts_with("query:")
        || key.starts_with("explain:")
    {
        return Priority::ReadOnly;
    }
    if key == "snapshot" || key == "rollback" {
        return Priority::Alethic;
    }
    Priority::Deontic
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_ordering_follows_variant_declaration() {
        // Derived Ord uses declaration order; smaller = higher priority.
        assert!(Priority::Alethic < Priority::Deontic);
        assert!(Priority::Deontic < Priority::ReadOnly);
        assert!(Priority::Alethic < Priority::ReadOnly);
    }

    #[test]
    fn empty_queue_pops_none() {
        let mut q: CommandQueue<&'static str> = CommandQueue::new();
        assert!(q.is_empty());
        assert_eq!(q.len(), 0);
        assert!(q.pop().is_none());
        assert!(q.peek().is_none());
    }

    #[test]
    fn pop_drains_alethic_before_deontic_before_readonly() {
        let mut q: CommandQueue<&'static str> = CommandQueue::new();
        // Enqueue out of priority order to prove pop reorders.
        q.push(Priority::ReadOnly, "read-1");
        q.push(Priority::Deontic, "write-1");
        q.push(Priority::Alethic, "critical-1");
        q.push(Priority::ReadOnly, "read-2");
        q.push(Priority::Deontic, "write-2");
        q.push(Priority::Alethic, "critical-2");
        assert_eq!(q.pop(), Some((Priority::Alethic, "critical-1")));
        assert_eq!(q.pop(), Some((Priority::Alethic, "critical-2")));
        assert_eq!(q.pop(), Some((Priority::Deontic, "write-1")));
        assert_eq!(q.pop(), Some((Priority::Deontic, "write-2")));
        assert_eq!(q.pop(), Some((Priority::ReadOnly, "read-1")));
        assert_eq!(q.pop(), Some((Priority::ReadOnly, "read-2")));
        assert!(q.pop().is_none());
    }

    #[test]
    fn within_a_lane_items_are_fifo() {
        let mut q: CommandQueue<u32> = CommandQueue::new();
        for i in 0..5 {
            q.push(Priority::Deontic, i);
        }
        for expected in 0..5 {
            assert_eq!(q.pop(), Some((Priority::Deontic, expected)));
        }
    }

    #[test]
    fn peek_does_not_consume() {
        let mut q: CommandQueue<&'static str> = CommandQueue::new();
        q.push(Priority::Alethic, "first");
        q.push(Priority::Deontic, "later");
        // peek twice should return the same alethic item.
        assert_eq!(q.peek(), Some((Priority::Alethic, &"first")));
        assert_eq!(q.peek(), Some((Priority::Alethic, &"first")));
        // pop removes it; peek now sees the deontic item.
        assert_eq!(q.pop(), Some((Priority::Alethic, "first")));
        assert_eq!(q.peek(), Some((Priority::Deontic, &"later")));
    }

    #[test]
    fn len_and_len_of_track_lane_totals() {
        let mut q: CommandQueue<u32> = CommandQueue::new();
        q.push(Priority::Alethic, 1);
        q.push(Priority::Alethic, 2);
        q.push(Priority::Deontic, 3);
        q.push(Priority::ReadOnly, 4);
        q.push(Priority::ReadOnly, 5);
        q.push(Priority::ReadOnly, 6);
        assert_eq!(q.len(), 6);
        assert_eq!(q.len_of(Priority::Alethic), 2);
        assert_eq!(q.len_of(Priority::Deontic), 1);
        assert_eq!(q.len_of(Priority::ReadOnly), 3);
        q.pop();
        assert_eq!(q.len(), 5);
        assert_eq!(q.len_of(Priority::Alethic), 1);
    }

    #[test]
    fn classify_read_verbs_as_readonly() {
        assert_eq!(classify_priority("debug"), Priority::ReadOnly);
        assert_eq!(classify_priority("audit"), Priority::ReadOnly);
        assert_eq!(classify_priority("verify_signature"), Priority::ReadOnly);
        assert_eq!(classify_priority("snapshots"), Priority::ReadOnly);
        assert_eq!(classify_priority("list:Order"), Priority::ReadOnly);
        assert_eq!(classify_priority("get:Customer"), Priority::ReadOnly);
        assert_eq!(classify_priority("query:order_has_total"), Priority::ReadOnly);
        assert_eq!(classify_priority("explain:violation-42"), Priority::ReadOnly);
    }

    #[test]
    fn classify_snapshot_and_rollback_as_alethic() {
        // No validator runs; correctness is structural. Queue ahead
        // of anything that might produce violations on commit.
        assert_eq!(classify_priority("snapshot"), Priority::Alethic);
        assert_eq!(classify_priority("rollback"), Priority::Alethic);
    }

    #[test]
    fn classify_writes_and_unknown_as_deontic() {
        // compile + create / update / transition + unknown user defs
        // all run validate and may surface deontic violations.
        assert_eq!(classify_priority("compile"), Priority::Deontic);
        assert_eq!(classify_priority("create:Order"), Priority::Deontic);
        assert_eq!(classify_priority("update:Customer"), Priority::Deontic);
        assert_eq!(classify_priority("transition:Order"), Priority::Deontic);
        assert_eq!(classify_priority("user_def_with_side_effect"), Priority::Deontic);
    }

    #[test]
    fn queue_mirrors_real_dispatch_pattern() {
        // Simulate a realistic submission: reads, deontic writes, and
        // an urgent alethic compile, all interleaved. The alethic
        // compile jumps ahead; deontic writes drain before queries;
        // FIFO holds within each lane.
        let mut q: CommandQueue<&'static str> = CommandQueue::new();
        for step in [
            ("list:Order", "L1"),
            ("create:Order", "W1"),
            ("get:Order", "L2"),
            ("update:Order", "W2"),
            ("snapshot", "A1"),
            ("query:order_has_total", "L3"),
        ] {
            q.push(classify_priority(step.0), step.1);
        }
        // Expected drain order: A1 (alethic) → W1, W2 (deontic FIFO)
        // → L1, L2, L3 (readonly FIFO).
        let drained: Vec<&str> = core::iter::from_fn(|| q.pop().map(|(_, v)| v)).collect();
        assert_eq!(drained, vec!["A1", "W1", "W2", "L1", "L2", "L3"]);
    }
}
