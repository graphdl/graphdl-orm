// crates/arest-kernel/src/ui_apps/breadcrumb.rs
//
// "You are here" breadcrumb + back/forward navigation history (#516,
// EPIC #496).
//
// VVVV's #511 landed cell-as-screen rendering and ZZZZ's #512 landed
// navigation actions as cells. The unified REPL now jumps between
// cells freely via `parse_cell_nav` (typed REPL command) or
// `on_navigation_target_selected` (clicked affordance) — but there is
// no surface for "where have I been?" or "let me return to the cell I
// was just on". This module supplies both, per the user's vision in
// #496:
//
//   "the repl should be easy to use. It should render the system as
//    the current screen of an app so that the user is never lost."
//
// The path through the cell graph IS itself a sequence of cells; the
// breadcrumb chrome simply visualises that sequence and exposes the
// classic browser-style back / forward stepping over it.
//
// # Conceptual model — the path IS a cell sequence
//
// Each navigation event pushes one `CrumbEntry` onto the history.
// Stepping back / forward walks the cursor through the existing path
// without adding new entries. When the user navigates somewhere new
// after stepping back, the forward stack clears (browser semantics).
//
// Bookmarks are facts of the implicit shape:
//
//     bookmark <Label> for <CellRef>
//
// We do not materialise these on the cell graph today — they live
// in-memory on `BreadcrumbState` and reset across reboots. A future
// task can wire `bookmark_has_target` cells through the SYSTEM verb
// surface so bookmarks become first-class facts the same way every
// other navigation primitive in this EPIC does.
//
// # Ring buffer + bounded history
//
// History uses a bounded `VecDeque<CurrentCell>` with the same
// semantics as `arest::ring::RingBuffer` (#188's bounded primitive).
// The `arest` crate's `ring` module is gated behind
// `not(feature = "no_std")` (see `arest/src/lib.rs` L107) so the
// kernel build (which uses `no_std`) hand-rolls the same shape here:
// `push` evicts the oldest on overflow and the cursor is clamped to
// the live range. Default capacity is 32 entries — large enough to
// span typical exploration sessions without unbounded growth. A
// future PR can lift the gate on `arest::ring` so this and the
// host-side audit log share one definition.
//
// # API contract
//
//   * `push(cell)`              — add a new entry, clearing forward
//                                 stack. Called from every navigation
//                                 surface (parse_cell_nav, click handler).
//   * `back()`                  — return the previous CurrentCell, if any.
//   * `forward()`               — return the next CurrentCell, if any.
//   * `bookmark(label, cell)`   — store `cell` under `label`.
//   * `goto_bookmark(label)`    — return the `CurrentCell` for `label`.
//   * `current_path()`          — return the live history slice
//                                 (oldest → current cursor) for the
//                                 breadcrumb strip.

#![allow(dead_code)]

use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::vec::Vec;

use crate::ui_apps::cell_renderer::CurrentCell;

/// Default ring-buffer capacity for navigation history. The buffer is
/// allocated lazily on construction; older entries beyond this depth
/// fall off the back of the ring per #188's semantics.
pub const DEFAULT_HISTORY_CAPACITY: usize = 32;

// ── CrumbEntry ─────────────────────────────────────────────────────

/// One step in the navigation history. Each entry is a cell the user
/// landed on at some point. `current_path()` returns these in
/// insertion order (oldest → most recent visible), with the cursor
/// position dictating which entry counts as "you are here".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrumbEntry {
    /// The cell that was visited. The breadcrumb strip renders this
    /// via `CurrentCell::label()`.
    pub cell: CurrentCell,
    /// Whether this entry is the cursor position ("you are here").
    /// Computed by `current_path()` from the cursor; `push` and the
    /// raw history vector do not carry this — it's a render-time
    /// projection.
    pub is_current: bool,
}

// ── BreadcrumbState ────────────────────────────────────────────────

/// History ring + cursor + bookmarks. One instance lives on
/// `UnifiedReplState`; every navigation event flows through it.
///
/// Cursor semantics mirror a web browser:
///   * `push(cell)` advances the cursor to the new entry and discards
///     anything past the cursor (the "forward stack").
///   * `back()` walks the cursor toward index 0; returns the cell at
///     the new cursor or `None` when already at the oldest.
///   * `forward()` walks the cursor toward the most recent entry;
///     returns the cell at the new cursor or `None` at the tip.
///
/// The "forward stack" is implicit — entries past the cursor are still
/// in the ring, they're just hidden until `forward()` moves the
/// cursor back over them. A `push()` after a `back()` truncates them
/// out of the ring, mirroring browser behaviour.
#[derive(Debug, Clone)]
pub struct BreadcrumbState {
    /// Bounded ring of cells, oldest at front. Capacity is the
    /// constructor argument; pushes beyond that drop the oldest entry.
    /// Mirrors `arest::ring::RingBuffer` semantics; we hand-roll here
    /// because the `arest::ring` module is gated out under `no_std`
    /// (see `arest/src/lib.rs` L107).
    history: VecDeque<CurrentCell>,
    /// Maximum live entries before the oldest is evicted on push.
    capacity: usize,
    /// Position of "you are here" within `history`. `None` when the
    /// history is empty; `Some(i)` otherwise where `0 <= i < len`.
    cursor: Option<usize>,
    /// Named bookmarks. Persists across redraws (held on
    /// `UnifiedReplState`) but not across reboots — a future task can
    /// reify these as `bookmark_has_target` facts in the cell graph.
    bookmarks: BTreeMap<String, CurrentCell>,
}

impl Default for BreadcrumbState {
    fn default() -> Self {
        Self::new()
    }
}

impl BreadcrumbState {
    /// New state with the default 32-entry history ring.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_HISTORY_CAPACITY)
    }

    /// New state with a custom history capacity. Tests use this to
    /// exercise the ring-eviction path without pushing 32+ entries.
    /// Panics on `cap == 0` (mirrors `RingBuffer::with_capacity`'s
    /// assertion — a zero-capacity ring would swallow every push).
    pub fn with_capacity(cap: usize) -> Self {
        assert!(cap > 0, "BreadcrumbState capacity must be non-zero");
        Self {
            history: VecDeque::with_capacity(cap),
            capacity: cap,
            cursor: None,
            bookmarks: BTreeMap::new(),
        }
    }

    // ---- Navigation history ----------------------------------------

    /// Push a new entry. Mirrors browser-style semantics:
    ///
    ///   1. If the cursor is not at the tip (i.e. the user stepped
    ///      back and is now navigating somewhere new), drop every
    ///      entry past the cursor first — the forward stack clears.
    ///   2. Append `cell` to the history ring. If the ring is full,
    ///      the oldest entry falls off the back; the cursor is
    ///      clamped to the new live range.
    ///   3. Advance the cursor to the new entry's position.
    ///
    /// Returns the entry evicted from the ring (oldest), if any, so
    /// callers can forward it to overflow signals.
    pub fn push(&mut self, cell: CurrentCell) -> Option<CurrentCell> {
        // Step 1: if cursor is mid-history, truncate the forward stack.
        if let Some(i) = self.cursor {
            let live_len = self.history.len();
            if i + 1 < live_len {
                self.history.truncate(i + 1);
            }
        }

        // Step 2: append, evicting the oldest if at capacity. Mirrors
        // `RingBuffer::push` semantics — returns the evicted entry.
        let evicted = if self.history.len() == self.capacity {
            self.history.pop_front()
        } else {
            None
        };
        self.history.push_back(cell);

        // Step 3: advance cursor to the new tip. After eviction the
        // tip index is `len - 1` regardless.
        self.cursor = Some(self.history.len() - 1);

        evicted
    }

    /// Walk the cursor one step toward the oldest entry. Returns the
    /// cell at the new cursor position, or `None` when already at the
    /// oldest entry / the history is empty.
    pub fn back(&mut self) -> Option<CurrentCell> {
        let i = self.cursor?;
        if i == 0 {
            return None;
        }
        let new_i = i - 1;
        self.cursor = Some(new_i);
        self.entry_at(new_i)
    }

    /// Walk the cursor one step toward the newest entry. Returns the
    /// cell at the new cursor position, or `None` when already at the
    /// tip / the history is empty.
    pub fn forward(&mut self) -> Option<CurrentCell> {
        let i = self.cursor?;
        let live_len = self.history.len();
        if i + 1 >= live_len {
            return None;
        }
        let new_i = i + 1;
        self.cursor = Some(new_i);
        self.entry_at(new_i)
    }

    /// True iff `back()` would move (not currently at oldest).
    pub fn can_go_back(&self) -> bool {
        matches!(self.cursor, Some(i) if i > 0)
    }

    /// True iff `forward()` would move (not currently at tip).
    pub fn can_go_forward(&self) -> bool {
        match self.cursor {
            Some(i) => i + 1 < self.history.len(),
            None => false,
        }
    }

    /// Snapshot of the live history with the cursor flagged as the
    /// "you are here" entry. Returned in insertion order (oldest →
    /// most recent visible). The breadcrumb strip renders this
    /// directly. `is_current` is true exactly once across the slice
    /// (when the history is non-empty).
    pub fn current_path(&self) -> Vec<CrumbEntry> {
        let cursor = self.cursor;
        self.history
            .iter()
            .enumerate()
            .map(|(i, cell)| CrumbEntry {
                cell: cell.clone(),
                is_current: cursor == Some(i),
            })
            .collect()
    }

    /// Tail of the live history (most recent `n` entries) with the
    /// cursor flagged. Used by the Slint surface to render only the
    /// last few crumbs without overflowing the strip when the ring
    /// is full. When the cursor falls outside the tail window, the
    /// returned slice still shows the tail — the caller can still
    /// observe whether the cursor lies inside via `is_current`.
    pub fn current_path_tail(&self, n: usize) -> Vec<CrumbEntry> {
        let path = self.current_path();
        if path.len() <= n {
            return path;
        }
        path[path.len() - n..].to_vec()
    }

    /// Live history length. Capped at the ring capacity.
    pub fn len(&self) -> usize {
        self.history.len()
    }

    /// True iff no navigation has been recorded yet.
    pub fn is_empty(&self) -> bool {
        self.history.is_empty()
    }

    /// Ring capacity (max entries before eviction). Used by tests.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    // ---- Bookmarks -------------------------------------------------

    /// Store `cell` under `label`. Overwrites any existing bookmark
    /// with the same label. Empty labels are accepted but discouraged.
    pub fn bookmark(&mut self, label: String, cell: CurrentCell) {
        self.bookmarks.insert(label, cell);
    }

    /// Look up a bookmarked cell by label. Returns `None` when no
    /// bookmark with that label exists.
    pub fn goto_bookmark(&self, label: &str) -> Option<CurrentCell> {
        self.bookmarks.get(label).cloned()
    }

    /// True iff a bookmark with `label` is registered.
    pub fn has_bookmark(&self, label: &str) -> bool {
        self.bookmarks.contains_key(label)
    }

    /// Remove a bookmark by label. Returns the cell that was bound
    /// to it, or `None` if the label was unknown.
    pub fn remove_bookmark(&mut self, label: &str) -> Option<CurrentCell> {
        self.bookmarks.remove(label)
    }

    /// Snapshot of every (label, cell) pair, sorted by label for a
    /// stable render order in the Bookmark Card.
    pub fn bookmark_list(&self) -> Vec<(String, CurrentCell)> {
        self.bookmarks
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Number of registered bookmarks.
    pub fn bookmark_count(&self) -> usize {
        self.bookmarks.len()
    }

    // ---- Internal helpers ------------------------------------------

    /// Lookup the entry at logical index `i` in the live history.
    /// Returns `None` if `i` is out of range.
    fn entry_at(&self, i: usize) -> Option<CurrentCell> {
        self.history.get(i).cloned()
    }
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    fn root() -> CurrentCell {
        CurrentCell::Root
    }
    fn noun(n: &str) -> CurrentCell {
        CurrentCell::Noun { noun: n.to_string() }
    }
    fn instance(n: &str, i: &str) -> CurrentCell {
        CurrentCell::Instance {
            noun: n.to_string(),
            instance: i.to_string(),
        }
    }

    // ── Default constructor ────────────────────────────────────────

    #[test]
    fn new_state_starts_empty_with_default_capacity() {
        let s = BreadcrumbState::new();
        assert_eq!(s.len(), 0);
        assert!(s.is_empty());
        assert_eq!(s.capacity(), DEFAULT_HISTORY_CAPACITY);
        assert_eq!(s.current_path(), Vec::new());
        assert!(!s.can_go_back());
        assert!(!s.can_go_forward());
    }

    // ── push / back / forward symmetry ─────────────────────────────

    #[test]
    fn push_advances_cursor_to_new_tip() {
        let mut s = BreadcrumbState::new();
        s.push(root());
        s.push(noun("File"));
        s.push(instance("File", "f1"));
        let path = s.current_path();
        assert_eq!(path.len(), 3);
        // Only the last entry is current.
        assert_eq!(path.iter().filter(|e| e.is_current).count(), 1);
        assert!(path[2].is_current);
        assert_eq!(path[2].cell, instance("File", "f1"));
    }

    #[test]
    fn back_walks_cursor_one_step_toward_oldest() {
        let mut s = BreadcrumbState::new();
        s.push(root());
        s.push(noun("File"));
        s.push(instance("File", "f1"));

        assert_eq!(s.back(), Some(noun("File")));
        let path = s.current_path();
        assert!(path[1].is_current);
        assert!(!path[2].is_current);

        assert_eq!(s.back(), Some(root()));
        let path = s.current_path();
        assert!(path[0].is_current);

        // At oldest — back is a no-op.
        assert_eq!(s.back(), None);
    }

    #[test]
    fn forward_walks_cursor_back_toward_tip() {
        let mut s = BreadcrumbState::new();
        s.push(root());
        s.push(noun("File"));
        s.push(instance("File", "f1"));
        s.back();
        s.back();

        assert_eq!(s.forward(), Some(noun("File")));
        assert_eq!(s.forward(), Some(instance("File", "f1")));
        // At tip — forward is a no-op.
        assert_eq!(s.forward(), None);
    }

    #[test]
    fn back_then_forward_returns_to_starting_cursor() {
        // Symmetry property: walking back N then forward N restores
        // the cursor to where it started.
        let mut s = BreadcrumbState::new();
        s.push(root());
        s.push(noun("File"));
        s.push(instance("File", "f1"));
        let before_path = s.current_path();
        s.back();
        s.back();
        s.forward();
        s.forward();
        let after_path = s.current_path();
        assert_eq!(before_path, after_path);
    }

    #[test]
    fn back_on_empty_history_returns_none() {
        let mut s = BreadcrumbState::new();
        assert_eq!(s.back(), None);
        assert_eq!(s.forward(), None);
    }

    #[test]
    fn can_go_back_and_forward_track_cursor() {
        let mut s = BreadcrumbState::new();
        assert!(!s.can_go_back());
        assert!(!s.can_go_forward());

        s.push(root());
        // Single entry: nothing to go back to or forward from.
        assert!(!s.can_go_back());
        assert!(!s.can_go_forward());

        s.push(noun("File"));
        // At tip with two entries: can go back, not forward.
        assert!(s.can_go_back());
        assert!(!s.can_go_forward());

        s.back();
        // At index 0 with two entries: can go forward, not back.
        assert!(!s.can_go_back());
        assert!(s.can_go_forward());
    }

    // ── Navigate-after-back clears forward stack ───────────────────

    #[test]
    fn push_after_back_clears_forward_stack() {
        // Browser-style semantic: stepping back then navigating
        // somewhere new must drop the entries past the cursor.
        let mut s = BreadcrumbState::new();
        s.push(root());
        s.push(noun("File"));
        s.push(instance("File", "f1"));
        s.back(); // cursor now at noun("File")
        s.push(noun("Tag"));

        // Forward stack cleared — the path is Root, File, Tag.
        let path = s.current_path();
        let labels: Vec<String> = path.iter().map(|e| e.cell.label()).collect();
        assert_eq!(
            labels,
            vec!["Resources".to_string(), "File".to_string(), "Tag".to_string()]
        );
        assert!(path[2].is_current);
        // No forward — cursor is at the new tip.
        assert!(!s.can_go_forward());
    }

    #[test]
    fn push_after_multiple_back_drops_all_subsequent_entries() {
        let mut s = BreadcrumbState::new();
        s.push(root());
        s.push(noun("File"));
        s.push(noun("Tag"));
        s.push(instance("Tag", "t1"));
        s.back();
        s.back();
        s.back(); // cursor at root
        s.push(noun("Component"));

        let path = s.current_path();
        let labels: Vec<String> = path.iter().map(|e| e.cell.label()).collect();
        assert_eq!(labels, vec!["Resources".to_string(), "Component".to_string()]);
        assert!(path[1].is_current);
    }

    // ── Ring buffer eviction ───────────────────────────────────────

    #[test]
    fn ring_evicts_oldest_at_capacity() {
        let mut s = BreadcrumbState::with_capacity(3);
        s.push(root());
        s.push(noun("A"));
        s.push(noun("B"));
        // Ring is full. Next push evicts the oldest (Root).
        let evicted = s.push(noun("C"));
        assert_eq!(evicted, Some(root()));
        assert_eq!(s.len(), 3);

        let path = s.current_path();
        let labels: Vec<String> = path.iter().map(|e| e.cell.label()).collect();
        assert_eq!(labels, vec!["A".to_string(), "B".to_string(), "C".to_string()]);
        // Cursor stays at the tip after eviction.
        assert!(path[2].is_current);
    }

    #[test]
    fn ring_eviction_preserves_cursor_at_tip() {
        let mut s = BreadcrumbState::with_capacity(3);
        for c in ['A', 'B', 'C', 'D', 'E'] {
            s.push(noun(&c.to_string()));
        }
        assert_eq!(s.len(), 3);
        let path = s.current_path();
        let labels: Vec<String> = path.iter().map(|e| e.cell.label()).collect();
        assert_eq!(labels, vec!["C".to_string(), "D".to_string(), "E".to_string()]);
        assert!(path[2].is_current);
    }

    // ── Bookmarks ──────────────────────────────────────────────────

    #[test]
    fn bookmark_add_and_lookup() {
        let mut s = BreadcrumbState::new();
        let cell = instance("File", "f1");
        s.bookmark("home".to_string(), cell.clone());
        assert!(s.has_bookmark("home"));
        assert_eq!(s.goto_bookmark("home"), Some(cell));
    }

    #[test]
    fn bookmark_lookup_unknown_returns_none() {
        let s = BreadcrumbState::new();
        assert_eq!(s.goto_bookmark("anything"), None);
    }

    #[test]
    fn bookmark_overwrites_on_duplicate_label() {
        let mut s = BreadcrumbState::new();
        s.bookmark("x".to_string(), noun("File"));
        s.bookmark("x".to_string(), noun("Tag"));
        assert_eq!(s.goto_bookmark("x"), Some(noun("Tag")));
        assert_eq!(s.bookmark_count(), 1);
    }

    #[test]
    fn bookmark_remove_returns_prior_value() {
        let mut s = BreadcrumbState::new();
        s.bookmark("x".to_string(), noun("File"));
        assert_eq!(s.remove_bookmark("x"), Some(noun("File")));
        assert!(!s.has_bookmark("x"));
        assert_eq!(s.remove_bookmark("x"), None);
    }

    #[test]
    fn bookmark_list_sorted_by_label() {
        let mut s = BreadcrumbState::new();
        s.bookmark("zebra".to_string(), noun("Z"));
        s.bookmark("apple".to_string(), noun("A"));
        s.bookmark("mango".to_string(), noun("M"));
        let list = s.bookmark_list();
        let labels: Vec<&str> = list.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, vec!["apple", "mango", "zebra"]);
    }

    // ── current_path_tail ──────────────────────────────────────────

    #[test]
    fn current_path_tail_returns_last_n_entries() {
        let mut s = BreadcrumbState::new();
        for c in ['A', 'B', 'C', 'D', 'E'] {
            s.push(noun(&c.to_string()));
        }
        let tail = s.current_path_tail(3);
        let labels: Vec<String> = tail.iter().map(|e| e.cell.label()).collect();
        assert_eq!(labels, vec!["C".to_string(), "D".to_string(), "E".to_string()]);
    }

    #[test]
    fn current_path_tail_returns_full_when_shorter() {
        let mut s = BreadcrumbState::new();
        s.push(root());
        s.push(noun("File"));
        let tail = s.current_path_tail(5);
        assert_eq!(tail.len(), 2);
    }

    // ── Bookmark navigation interaction ────────────────────────────

    #[test]
    fn goto_bookmark_then_push_records_in_history() {
        // Typical flow: goto a bookmark, then push that cell into
        // history (the caller does both — `goto_bookmark` is a pure
        // lookup).
        let mut s = BreadcrumbState::new();
        s.bookmark("home".to_string(), noun("File"));
        let target = s.goto_bookmark("home").unwrap();
        s.push(target.clone());
        let path = s.current_path();
        assert_eq!(path.last().map(|e| &e.cell), Some(&target));
        assert!(path.last().map(|e| e.is_current).unwrap_or(false));
    }
}
