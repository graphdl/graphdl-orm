// crates/arest/src/ring.rs
//
// Bounded ring buffer — one primitive for every append-only, depth-
// bounded, oldest-out-on-overflow structure AREST needs: the CPU
// audit log, the FPGA audit-log generator (#167), and the x86_64
// boot REPL keystroke history (#183). Expressing them all through
// one type keeps "bounded event store" a single concept rather than
// three near-copies.
//
// Semantics:
//   - push(item)     -> Option<evicted>. None when len < cap;
//                       Some(oldest) when the buffer is full.
//   - len / capacity self-explanatory.
//   - iter_oldest_first yields items in insertion order from the
//     earliest non-evicted entry to the most recent.
//
// The FPGA generator in #167 derives its on-chip ring from the same
// (capacity, element-shape) pair, so the source-level definition is
// the authority regardless of whether it runs in software or silicon.

use std::collections::VecDeque;

/// Append-only ring with a compile-time-unknown but fixed-per-instance
/// capacity. `cap == 0` is rejected at construction — a zero-capacity
/// ring would swallow every push and produce no observable state.
#[derive(Debug, Clone)]
pub struct RingBuffer<T> {
    buf: VecDeque<T>,
    cap: usize,
}

impl<T> RingBuffer<T> {
    /// New buffer of the given capacity. Panics when `cap == 0` —
    /// callers that want a dynamically-sized buffer should use `Vec`.
    pub fn with_capacity(cap: usize) -> Self {
        assert!(cap > 0, "RingBuffer capacity must be non-zero");
        Self { buf: VecDeque::with_capacity(cap), cap }
    }

    /// Push a new item. When the buffer is full the oldest item is
    /// evicted and returned — callers use this to forward the evicted
    /// entry to overflow-signal handling (SM transition, audit
    /// overflow counter, UI watermark).
    pub fn push(&mut self, item: T) -> Option<T> {
        let evicted = if self.buf.len() == self.cap {
            self.buf.pop_front()
        } else {
            None
        };
        self.buf.push_back(item);
        evicted
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn capacity(&self) -> usize {
        self.cap
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.buf.len() == self.cap
    }

    /// Walks the live entries from oldest to most recent.
    pub fn iter_oldest_first(&self) -> impl Iterator<Item = &T> {
        self.buf.iter()
    }

    /// Drain is useful for migration paths (freeze/thaw, tenant
    /// teardown) that need to consume every live entry once.
    pub fn drain_oldest_first(&mut self) -> impl Iterator<Item = T> + '_ {
        self.buf.drain(..)
    }
}

impl<T: Clone> RingBuffer<T> {
    /// Snapshot the buffer contents into a freshly allocated Vec.
    /// Useful for audit-log projections where the caller wants to
    /// read the full live set without borrowing the buffer.
    pub fn to_vec_oldest_first(&self) -> Vec<T> {
        self.buf.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_below_capacity_grows_len() {
        let mut r: RingBuffer<u32> = RingBuffer::with_capacity(4);
        assert_eq!(r.len(), 0);
        assert!(r.is_empty());
        assert!(!r.is_full());
        for i in 0..3 {
            assert_eq!(r.push(i), None, "no eviction below capacity");
        }
        assert_eq!(r.len(), 3);
        assert!(!r.is_full());
    }

    #[test]
    fn push_at_capacity_returns_evicted_oldest() {
        let mut r: RingBuffer<u32> = RingBuffer::with_capacity(3);
        r.push(10); r.push(20); r.push(30);
        assert!(r.is_full());
        // Next push evicts oldest (10).
        assert_eq!(r.push(40), Some(10));
        assert_eq!(r.len(), 3);
        assert!(r.is_full());
        // Subsequent pushes keep evicting the oldest live entry.
        assert_eq!(r.push(50), Some(20));
        assert_eq!(r.push(60), Some(30));
        // Live entries are 40, 50, 60.
        assert_eq!(r.to_vec_oldest_first(), vec![40, 50, 60]);
    }

    #[test]
    fn iter_oldest_first_respects_insertion_order() {
        let mut r: RingBuffer<&'static str> = RingBuffer::with_capacity(4);
        r.push("alpha");
        r.push("bravo");
        r.push("charlie");
        let collected: Vec<&str> = r.iter_oldest_first().copied().collect();
        assert_eq!(collected, vec!["alpha", "bravo", "charlie"]);
    }

    #[test]
    fn iter_after_wraparound_still_oldest_first() {
        let mut r: RingBuffer<u32> = RingBuffer::with_capacity(3);
        for i in 1..=7 {
            r.push(i);
        }
        // After seven pushes into a cap-3 ring, live entries are 5, 6, 7.
        assert_eq!(r.to_vec_oldest_first(), vec![5, 6, 7]);
    }

    #[test]
    fn drain_consumes_all_entries_oldest_first() {
        let mut r: RingBuffer<u32> = RingBuffer::with_capacity(5);
        for i in 1..=4 { r.push(i); }
        let taken: Vec<u32> = r.drain_oldest_first().collect();
        assert_eq!(taken, vec![1, 2, 3, 4]);
        assert!(r.is_empty());
    }

    #[test]
    #[should_panic(expected = "capacity must be non-zero")]
    fn zero_capacity_rejected_at_construction() {
        let _: RingBuffer<u32> = RingBuffer::with_capacity(0);
    }

    #[test]
    fn capacity_reported_verbatim() {
        let r: RingBuffer<u32> = RingBuffer::with_capacity(42);
        assert_eq!(r.capacity(), 42);
    }
}
