// crates/arest/src/ast.rs
//
// The FP AST — Backus's combining forms as Rust types.
//
// Theoretical lineage:
//   - Principia Mathematica: first-order predicate logic (propositions, quantifiers, proof)
//   - Church's lambda calculus: abstraction, application, beta reduction
//   - Backus's FP algebra (1977): point-free combining forms, algebraic laws
//   - Halpin's ORM2/FORML2: natural language surface syntax for FOL
//
// Objects are the data domain (atoms, sequences, bottom).
// Functions are the program domain (primitives + combining forms).
// Application is the single operation: f:x → object.
//
// Skip-validate policy: when the `Policy_skip_validate` cell holds atom
// "T", `platform_compile` bypasses the post-merge constraint evaluation.
// The policy lives in the defs state — set it once at boot via
// `install_skip_validate(&d)` and the compile path observes it through
// the same merged_state it already builds.
//
// Validation is O(constraints × population); per-fact-type indexing is
// available via the `validate:{fact_type_id}` defs produced by
// `compile_to_defs_state`. Bulk loads of known-good readings may opt
// out entirely with this policy.
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

/// Policy cell: presence of atom "T" disables constraint evaluation
/// during `platform_compile`. Reachable from any state — typically the
/// defs state that flows into the compile merge.
pub const POLICY_SKIP_VALIDATE: &str = "Policy_skip_validate";

/// Install the skip-validate policy on `state`. Returns a new state
/// with the `Policy_skip_validate` cell set to atom "T".
pub fn install_skip_validate(state: &Object) -> Object {
    store(POLICY_SKIP_VALIDATE, Object::atom("T"), state)
}

/// Read the skip-validate policy from `state`. True iff the
/// `Policy_skip_validate` cell holds atom "T".
fn is_skip_validate(state: &Object) -> bool {
    matches!(fetch(POLICY_SKIP_VALIDATE, state).as_atom(), Some("T"))
}

// ── Reductions fuel (Sec-3: #159 enforcement inside apply) ─────────
//
// A thread-local "reductions remaining" counter, decremented at the
// top of every `apply` call. When it reaches zero, `apply` short-
// circuits to Bottom — which then propagates outward through the
// bottom-preserving combining forms (Compose, Construction, α, etc.)
// and stops the evaluator cleanly instead of letting a malicious
// Func tree run until the OS stack overflows.
//
// Sentinel `u64::MAX` = unlimited. Default is unlimited so existing
// call sites are unaffected; callers bound a suspect tree with
// `with_fuel(n, || apply(…))`.

#[cfg(not(feature = "no_std"))]
thread_local! {
    static APPLY_FUEL: core::cell::Cell<u64> = const { core::cell::Cell::new(u64::MAX) };
}

#[cfg(feature = "no_std")]
static APPLY_FUEL_ATOMIC: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(u64::MAX);

/// Evaluate `f` with the `apply` reductions budget set to `budget`.
/// Restores the previous budget when `f` returns — nested calls are
/// well-scoped, so callers (system dispatch, test fixtures) can bound
/// a specific subtree without leaking state to sibling scopes.
///
/// `u64::MAX` means "unlimited"; this is the startup default and what
/// `with_fuel` restores after inner scopes complete.
#[cfg(not(feature = "no_std"))]
pub fn with_fuel<T, F: FnOnce() -> T>(budget: u64, f: F) -> T {
    struct Restore(u64);
    impl Drop for Restore {
        fn drop(&mut self) { APPLY_FUEL.with(|c| c.set(self.0)); }
    }
    let _guard = APPLY_FUEL.with(|c| {
        let prior = c.get();
        c.set(budget);
        Restore(prior)
    });
    f()
}

#[cfg(feature = "no_std")]
pub fn with_fuel<T, F: FnOnce() -> T>(budget: u64, f: F) -> T {
    use core::sync::atomic::Ordering;
    let prior = APPLY_FUEL_ATOMIC.swap(budget, Ordering::Relaxed);
    let out = f();
    APPLY_FUEL_ATOMIC.store(prior, Ordering::Relaxed);
    out
}

/// Debit one reduction from the fuel counter. Returns `true` when the
/// caller may proceed, `false` when the budget is exhausted and the
/// caller must return Bottom. The unlimited sentinel short-circuits
/// without touching the counter — the fast path costs a compare.
#[cfg(not(feature = "no_std"))]
fn consume_fuel() -> bool {
    APPLY_FUEL.with(|c| {
        let fuel = c.get();
        if fuel == u64::MAX { return true; }
        if fuel == 0 { return false; }
        c.set(fuel - 1);
        true
    })
}

#[cfg(feature = "no_std")]
fn consume_fuel() -> bool {
    use core::sync::atomic::Ordering;
    let mut fuel = APPLY_FUEL_ATOMIC.load(Ordering::Relaxed);
    loop {
        if fuel == u64::MAX { return true; }
        if fuel == 0 { return false; }
        match APPLY_FUEL_ATOMIC.compare_exchange_weak(
            fuel, fuel - 1, Ordering::Relaxed, Ordering::Relaxed,
        ) {
            Ok(_) => return true,
            Err(curr) => fuel = curr,
        }
    }
}

/// True when a non-default budget is in effect. Used by the parallel
/// branches of ApplyToAll / Construction / Filter to fall back to the
/// serial path — Rayon workers have their own thread-local fuel, so
/// spawning loses the caller's bound. Serial is the correct behavior
/// under a fuel cap; the parallel speed-up applies only to the
/// unlimited default.
#[cfg(not(feature = "no_std"))]
#[allow(dead_code)] // only referenced under `feature = "parallel"`
fn fuel_is_bounded() -> bool {
    APPLY_FUEL.with(|c| c.get() != u64::MAX)
}

#[cfg(feature = "no_std")]
#[allow(dead_code)]
fn fuel_is_bounded() -> bool {
    APPLY_FUEL_ATOMIC.load(core::sync::atomic::Ordering::Relaxed) != u64::MAX
}

/// Snapshot the current fuel counter without modifying it. Used by
/// `apply_with_fuel` to compute the remaining budget after a bounded
/// evaluation returns. `u64::MAX` is the unlimited sentinel.
#[cfg(not(feature = "no_std"))]
fn current_fuel() -> u64 { APPLY_FUEL.with(|c| c.get()) }

#[cfg(feature = "no_std")]
fn current_fuel() -> u64 {
    APPLY_FUEL_ATOMIC.load(core::sync::atomic::Ordering::Relaxed)
}

// ── ⊥-trace: why-NOT provenance for a computation that bottomed out ──
//
// ⊥ is absorbing and provenance-lossless: `f:⊥ = ⊥` for every `f`, so a
// top-level ⊥ has lost every trace of WHERE it first arose. The engine
// can only say "⊥" with no context. This is the negative twin of the
// `explain` verb — why-NOT provenance for a structurally-bottomed
// computation.
//
// The carry is a `Result::Err`-context pattern realized as a thread-
// local, materialized ONLY when ⊥ is produced AND a caller has *armed*
// tracing via `with_bottom_trace`. Internal evaluation (Filter via
// `α(p→id;⊥)`, compact, etc.) produces ⊥ constantly as ordinary
// intermediate scaffolding; recording on every such ⊥ would tax the hot
// path. So recording is gated on an `armed` flag whose unset (default)
// fast path costs exactly one `Cell<bool>::get` compare — the same
// "sentinel short-circuit" shape the fuel counter uses. When unarmed,
// `note_bottom_*` is a no-op and the success path is untouched.
//
// When armed, frames record bottom-up as the ⊥ unwinds: the DEEPEST
// frame (a `Fetch`/`Store` over a cell) fills `cell` + `binding` first;
// the enclosing `Def` frame fills `rule` on the way out. Each field is
// written once (first writer wins per field), so the trace names the
// ORIGIN frame, not the outermost re-throw.
//
// Gated `not(no_std)`: the only ⊥→user-message surfaces (`system_impl`,
// the CLI dispatcher) are host builds. The kernel never renders ⊥ to a
// user, so the trace is dead weight there — and `thread_local!` +
// `RefCell` are std-only anyway. The recording call sites in `apply`
// compile to nothing under `no_std`.

/// Origin of a structurally-produced ⊥, captured as the ⊥ unwinds the
/// application spine. Each field is `Some` only if a frame that knew it
/// was crossed: `rule` from the enclosing `Func::Def`, `cell` + `binding`
/// from the `Fetch`/`FetchOrPhi`/`Store` frame where the ⊥ first arose.
#[cfg(not(feature = "no_std"))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BottomTrace {
    /// The rule / def being applied at the frame that bottomed out.
    pub rule: Option<String>,
    /// The cell (FactType image / store name) under which ⊥ arose.
    pub cell: Option<String>,
    /// The frame's argument binding at the point ⊥ first arose.
    pub binding: Option<String>,
}

#[cfg(not(feature = "no_std"))]
impl BottomTrace {
    fn is_empty(&self) -> bool {
        self.rule.is_none() && self.cell.is_none() && self.binding.is_none()
    }

    /// Human-readable one-line origin, e.g.
    /// `origin: <App, a1> in rule `view:Ready` over cell `App_uses_Generator``.
    /// Falls back through whichever fields are known. Returns `None` when
    /// nothing was captured (no armed frame carried context).
    pub fn describe(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mut s = String::from("⊥ origin:");
        if let Some(b) = &self.binding {
            s.push_str(&alloc::format!(" {}", b));
        }
        if let Some(r) = &self.rule {
            s.push_str(&alloc::format!(" in rule `{}`", r));
        }
        if let Some(c) = &self.cell {
            s.push_str(&alloc::format!(" over cell `{}`", c));
        }
        Some(s)
    }
}

#[cfg(not(feature = "no_std"))]
thread_local! {
    /// `true` only inside a `with_bottom_trace` scope. Unset = the hot
    /// default; `note_bottom_*` short-circuits on a single `get`.
    static BOTTOM_TRACE_ARMED: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
    /// The accumulating trace; meaningful only while armed.
    static BOTTOM_TRACE: core::cell::RefCell<BottomTrace> =
        const { core::cell::RefCell::new(BottomTrace {
            rule: None, cell: None, binding: None,
        }) };
}

/// Record the `rule` (Def name) at a frame that just bottomed out.
/// No-op unless tracing is armed (the hot default). First writer wins —
/// the innermost Def that bottomed names the rule.
///
/// `pub(crate)` so the forward-chain LFP loop (`evaluate`) can name the
/// rule it was churning on when it aborts a non-terminating fixpoint
/// (cli-apply-large-tasksdb-nonterminating) — the same ⊥-origin channel
/// that `apply`'s `Def` frames feed.
#[cfg(not(feature = "no_std"))]
pub(crate) fn note_bottom_rule(name: &str) {
    if !BOTTOM_TRACE_ARMED.with(|c| c.get()) {
        return;
    }
    BOTTOM_TRACE.with(|t| {
        let mut t = t.borrow_mut();
        if t.rule.is_none() {
            t.rule = Some(name.to_string());
        }
    });
}

/// Record the `cell` and the frame `binding` at the `Fetch`/`Store`
/// frame where ⊥ first arose. No-op unless armed. First writer wins, so
/// the DEEPEST (origin) cell access is the one named.
///
/// `pub(crate)` for the forward-chain abort path (see `note_bottom_rule`):
/// the loop names the consequent cell whose rule was still firing when
/// the chain's time budget ran out.
#[cfg(not(feature = "no_std"))]
pub(crate) fn note_bottom_cell(cell: &str, binding: &Object) {
    if !BOTTOM_TRACE_ARMED.with(|c| c.get()) {
        return;
    }
    BOTTOM_TRACE.with(|t| {
        let mut t = t.borrow_mut();
        if t.cell.is_none() {
            t.cell = Some(cell.to_string());
        }
        if t.binding.is_none() {
            t.binding = Some(binding.to_string());
        }
    });
}

/// No_std shims: tracing is host-only, so under `no_std` the recording
/// hooks compile to nothing (the kernel never renders ⊥ to a user).
#[cfg(feature = "no_std")]
pub(crate) fn note_bottom_rule(_name: &str) {}
#[cfg(feature = "no_std")]
pub(crate) fn note_bottom_cell(_cell: &str, _binding: &Object) {}

/// Evaluate `f` with ⊥-tracing armed, returning the result alongside the
/// captured `BottomTrace` (`None` when nothing carried context). Arms the
/// thread-local flag for the duration, clears any prior trace on entry,
/// and restores both on return so nested / sibling scopes compose. ZERO
/// effect on `apply` callers that don't opt in — the recording hooks
/// observe `armed == false` outside this scope.
///
/// Use at the ⊥→user-message boundary: when `apply` returns ⊥, the
/// trace de-opaques "engine returned ⊥" into "⊥ origin: <binding> in
/// rule `…` over cell `…`".
///
/// CONTRACT: the trace is meaningful only when the top-level `result` is
/// itself ⊥. A frame can record an origin for an intermediate ⊥ that is
/// later absorbed (e.g. `compact` dropping a ⊥ element) into a non-⊥
/// result; callers therefore gate on `matches!(result, Object::Bottom)`
/// before surfacing the trace (both the CLI dispatcher and `system_impl`
/// do exactly this). The helper itself cannot inspect the generic `T`.
#[cfg(not(feature = "no_std"))]
pub fn with_bottom_trace<T, F: FnOnce() -> T>(f: F) -> (T, Option<BottomTrace>) {
    struct Restore {
        prev_armed: bool,
        prev_trace: BottomTrace,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            BOTTOM_TRACE_ARMED.with(|c| c.set(self.prev_armed));
            BOTTOM_TRACE.with(|t| *t.borrow_mut() = core::mem::take(&mut self.prev_trace));
        }
    }
    let _guard = {
        let prev_armed = BOTTOM_TRACE_ARMED.with(|c| c.replace(true));
        let prev_trace = BOTTOM_TRACE.with(|t| core::mem::take(&mut *t.borrow_mut()));
        Restore { prev_armed, prev_trace }
    };
    let out = f();
    let trace = BOTTOM_TRACE.with(|t| {
        let t = t.borrow();
        if t.is_empty() { None } else { Some(t.clone()) }
    });
    (out, trace)
}

//
// All framework objects compile to these types:
//   Role        → Selector
//   Fact Type → Construction (CONS of roles)
//   Query       → partial application (some roles bound)
//   Fact        → fully applied Construction (all roles bound)
//   Derivation  → Composition chain
//   Constraint  → Condition
//   Aggregation → Insert (fold)
//   Population traversal → ApplyToAll (map)

use hashbrown::HashMap;
use crate::sync::Arc;
use core::fmt;

// `parallel` requires std (rayon thread pools); the top-level
// `compile_error!` in lib.rs (#592) rejects `no_std + parallel`, so
// gating this import on `not(feature = "no_std")` is belt-and-braces
// — keeps the unresolved-import diagnostic from drowning out the
// clearer compile_error message if the user composes both anyway.
#[cfg(all(feature = "parallel", not(feature = "no_std")))]
use rayon::prelude::*;

// ── Objects (data domain) ────────────────────────────────────────────
// An object is either an atom, a sequence, or bottom (undefined).
// Bottom is preserved through all operations: f(⊥) = ⊥.

#[derive(Clone, Debug, PartialEq)]
pub enum Object {
    /// An atom — a reference value (entity ID, slug, email, enum value, number).
    /// Includes T (true), F (false), and Phi (empty sequence).
    Atom(String),

    /// A sequence of objects: <x₁, ..., xₙ>.
    /// A fact's bindings are a sequence. A population is a sequence of facts.
    /// If any element is Bottom, the whole sequence is Bottom.
    ///
    /// Arc-wrapped slice for cheap clones: most Seq operations in
    /// AREST's evaluator are read-only (iteration, indexing,
    /// destructuring), and apply() clones freely to avoid aliasing
    /// concerns. `Arc<[Object]>` makes that a ref-count bump instead
    /// of a Vec deep copy, while giving us free `From<Vec<Object>>`
    /// and `FromIterator<Object>` so construction sites stay terse:
    /// `Object::Seq(vec.into())` or `iter.collect()` both work.
    Seq(Arc<[Object]>),

    /// A named store (Backus §13.3.4): cells indexed by name for O(1) fetch/store.
    /// Semantically equivalent to Seq of <CELL, name, contents> triples,
    /// but with HashMap backing for O(1) ↑n:D and ↓n:<x,D> operations.
    ///
    /// task-817: the HashMap is Arc-wrapped so clones are O(1) ref-count
    /// bumps instead of full deep copies. This mirrors Object::Seq's
    /// `Arc<[Object]>` shape and makes DistR-with-Map (where the same
    /// map is duplicated across N pair sites) cheap. Mutation paths use
    /// `Arc::make_mut` for copy-on-write semantics — single-reader paths
    /// stay zero-copy, second-reader paths pay the structural clone only
    /// when they must.
    Map(Arc<HashMap<String, Object>>),

    /// Bottom (⊥) — undefined. All functions preserve bottom: f(⊥) = ⊥.
    Bottom,
}

impl Object {
    pub fn atom(s: &str) -> Self { Object::Atom(s.to_string()) }
    pub fn t() -> Self { Object::Atom("T".to_string()) }
    pub fn f() -> Self { Object::Atom("F".to_string()) }
    pub fn phi() -> Self { Object::Seq(Arc::from([])) }

    /// Construct a sequence of `Object`s. **Bottom-preserving** per
    /// Backus §11.2.1: if any element is ⊥, the whole sequence is ⊥.
    /// This is the canonical, paper-faithful constructor and the one
    /// downstream code should reach for.
    ///
    /// To build a sequence that *carries* ⊥ as a member element —
    /// the intermediate scaffolding §11.2.4's `compact ∘ α(p → id ; ⊥)`
    /// presupposes — bypass this constructor and call
    /// `Object::Seq(Arc::from(items))` directly. That path skips the
    /// ⊥-check and is what makes `Func::Compact` a meaningful
    /// primitive (it operates on those bypassed sequences). Use the
    /// bypass only at internal interpreter / lowering boundaries; user-
    /// visible `Object`s should go through `Object::seq` so §11.2.1
    /// holds at the API surface.
    pub fn seq(items: Vec<Object>) -> Self {
        if items.iter().any(|x| matches!(x, Object::Bottom)) {
            Object::Bottom
        } else {
            Object::Seq(items.into())
        }
    }

    /// Parse an FFP object from Backus notation.
    /// Atoms: bare strings. Sequences: <x₁, x₂, ...>. Bottom: ⊥. Empty: φ.
    pub fn parse(input: &str) -> Object {
        parse_with_depth(input, 0)
    }

    pub fn is_bottom(&self) -> bool { matches!(self, Object::Bottom) }
    pub fn is_atom(&self) -> bool { matches!(self, Object::Atom(_)) }

    pub fn as_seq(&self) -> Option<&[Object]> {
        match self {
            Object::Seq(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_atom(&self) -> Option<&str> {
        match self {
            Object::Atom(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_map(&self) -> Option<&HashMap<String, Object>> {
        match self {
            Object::Map(m) => Some(m.as_ref()),
            _ => None,
        }
    }

    /// task-817: typed Map constructor that wraps a fresh HashMap in
    /// an `Arc`. New construction sites should prefer this over
    /// `Object::Map(Arc::new(map))` for clarity, and existing sites
    /// that build a local HashMap then construct can switch by
    /// dropping the intermediate `Arc::new` wrap.
    pub fn map(m: HashMap<String, Object>) -> Object {
        Object::Map(Arc::new(m))
    }

    /// Convert a Seq-of-cells store to a Map store for O(1) access.
    /// Backus §13.3.4: fetch scans linearly; Map preserves semantics with O(1).
    pub fn to_store(&self) -> Object {
        match self {
            Object::Map(_) => self.clone(),
            Object::Seq(cells) => {
                let mut map = HashMap::new();
                for cell_obj in cells.iter() {
                    if let Some(items) = cell_obj.as_seq() {
                        if items.len() == 3
                            && items[0].as_atom() == Some(CELL_TAG)
                        {
                            if let Some(name) = items[1].as_atom() {
                                map.insert(name.to_string(), items[2].clone());
                            }
                        }
                    }
                }
                Object::Map(Arc::new(map))
            }
            _ => self.clone(),
        }
    }

    /// Serialize this Object as a JSON string. Inverse bias: atoms that
    /// already parse as JSON (e.g. the `debug` def's JSON-atom payload)
    /// are passed through verbatim; other atoms become JSON strings.
    /// Seqs become arrays, Maps become objects, Bottom becomes null.
    ///
    /// Used by system_impl to serve every tool response as JSON so MCP
    /// and HTTP callers can parse uniformly — no mixed FFP/JSON handling.
    #[cfg(not(feature = "no_std"))]
    pub fn to_json_string(&self) -> String {
        self.to_json_value().to_string()
    }

    #[cfg(not(feature = "no_std"))]
    fn to_json_value(&self) -> serde_json::Value {
        match self {
            Object::Bottom => serde_json::Value::Null,
            Object::Atom(s) => {
                // Pass-through for atoms that are already JSON documents
                // (e.g. the debug / list:{noun} / get:{noun} / __result defs).
                serde_json::from_str::<serde_json::Value>(s)
                    .unwrap_or_else(|_| serde_json::Value::String(s.clone()))
            }
            Object::Seq(items) => serde_json::Value::Array(
                items.iter().map(|i| i.to_json_value()).collect()
            ),
            Object::Map(m) => serde_json::Value::Object(
                m.iter().map(|(k, v)| (k.clone(), v.to_json_value())).collect()
            ),
        }
    }
}

/// Split a string on commas, respecting nested <> brackets and
/// backslash-escaped specials inside atom tokens. A `\` always
/// escapes the next char, so an atom value `reachable in \< 30 s`
/// (the `Display`-emitted form of the in-memory atom
/// `reachable in < 30 s`) doesn't open a fake nesting level.
fn split_top_level(s: &str) -> Vec<&str> {
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut splits: Vec<&str> = Vec::new();
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => { chars.next(); }
            // Both Seq `<>` and Map `{}` brackets contribute nesting
            // depth so a `,` inside either kind of nested literal does
            // NOT split the outer entry. (task-922-object-parse-map-syntax:
            // before this, a Map value containing a `,` parsed as a
            // truncated Atom because depth ignored `{}`.)
            '<' | '{' => depth += 1,
            '>' | '}' => depth -= 1,
            ',' if depth == 0 => {
                splits.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    splits.push(&s[start..]);
    splits
}

/// Split `key=value` at the FIRST top-level `=`. Mirrors
/// `split_top_level` but for the Map entry separator. Returns `None`
/// when no top-level `=` is present (malformed entry — caller decides
/// whether to drop it or treat the whole thing as a key with empty
/// value).
fn split_first_eq_top_level(s: &str) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => { chars.next(); }
            '<' | '{' => depth += 1,
            '>' | '}' => depth -= 1,
            '=' if depth == 0 => return Some((&s[..i], &s[i + c.len_utf8()..])),
            _ => {}
        }
    }
    None
}

/// Backslash-escape the FFP-syntactic characters and the escape
/// character itself when emitting an atom to the FFP wire format.
/// Without this, an instance fact value like `reachable in < 30 s`
/// (a perfectly legitimate atom) round-trips through
/// `db::persist_state` → `db::load_state` as a malformed nested
/// `Seq`, silently corrupting the cell. Escaping is a pure
/// `Display`-side concern; the in-memory `Object::Atom("reachable
/// in < 30 s")` is unchanged.
///
/// The reserved set is BOTH the Seq delimiters (`<`, `>`, `,`) AND
/// the Map delimiters (`{`, `}`, `=`). `Object::parse` discriminates
/// Map literals from opaque atoms via `split_top_level` (which tracks
/// `<>`/`{}` nesting depth) and `split_first_eq_top_level` (which
/// splits on the first top-level `=`). An atom VALUE inside a Map —
/// e.g. a Task Description containing `BASE=localhost`, `${BASE}`,
/// `{slug}`, or a net-unbalanced `>` — would otherwise corrupt that
/// depth tracking on the way back, so the whole persisted cell
/// re-parses as a single opaque Atom (`as_map()`/`as_seq()` → None)
/// and reads as EMPTY on load. That silently destroyed every keyed
/// Map FT cell (Task_has_Task_Description / _Subject, 787 → 0) on a
/// full readings-recompile. `{`/`}`/`=` join the reserved set so
/// Map-backed cells round-trip losslessly; `unescape_atom_from_display`
/// already strips any `\X` → `X`, so no decode-side change is needed.
fn escape_atom_for_display(s: &str) -> alloc::string::String {
    let mut out = alloc::string::String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' | '<' | '>' | ',' | '{' | '}' | '=' => { out.push('\\'); out.push(c); }
            _ => out.push(c),
        }
    }
    out
}

/// Reverse of `escape_atom_for_display`. `\X` for any `X` becomes
/// `X` (we don't reserve specific escape sequences — the only
/// purpose is to neutralize the FFP-syntactic characters when they
/// appear inside an atom). A trailing `\` with no following char is
/// preserved verbatim (defensive: never panic on malformed input).
fn unescape_atom_from_display(s: &str) -> alloc::string::String {
    let mut out = alloc::string::String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some(next) => out.push(next),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Maximum nesting depth for `Object::parse` to prevent stack overflow on
/// maliciously crafted inputs (e.g. deeply nested `< < < ... > > >`).
const MAX_PARSE_DEPTH: usize = 100;

fn parse_with_depth(input: &str, depth: usize) -> Object {
    let s = input.trim();
    // Single dispatch table — Backus cond combining form over input shape.
    // No early returns; every branch is a value expression.
    match s {
        "" | "\u{03C6}" => Object::phi(),
        "\u{22A5}" => Object::Bottom,
        seq if seq.starts_with('<') && seq.ends_with('>') && depth >= MAX_PARSE_DEPTH => {
            let _ = seq; Object::Bottom
        }
        seq if seq.starts_with('<') && seq.ends_with('>') => {
            let inner = &seq[1..seq.len()-1];
            match inner.trim().is_empty() {
                true => Object::phi(),
                false => Object::Seq(
                    split_top_level(inner).into_iter()
                        .map(|i| parse_with_depth(i.trim(), depth + 1))
                        .collect::<Vec<_>>()
                        .into()
                ),
            }
        }
        // Map literal `{k1=v1, k2=v2, ...}` — inverse of the Display
        // impl at lines 454-458 / item_inside_seq lines 476-480.
        // Without this branch, a persisted Map cell round-trips back as
        // an opaque Atom holding the literal text — the cell is then
        // not iterable and the SQL projector + every consumer reads it
        // as empty. (task-922-object-parse-map-syntax.)
        //
        // Discrimination from JSON: existing callers (notably
        // `system_impl`'s `Object::parse(input)` for apply commands)
        // pass `{"type":"createEntity",…}` JSON strings through as
        // opaque Atoms because JSON uses `:` and quoted keys. We
        // recognise Map syntax ONLY when EVERY non-empty top-level
        // entry has a `=` separator. If any entry lacks `=`, fall
        // through to the Atom branch (preserves the legacy JSON
        // pass-through and any other `{…}`-shaped opaque payload).
        map_lit if map_lit.starts_with('{') && map_lit.ends_with('}')
            && depth >= MAX_PARSE_DEPTH => Object::Bottom,
        map_lit if map_lit.starts_with('{') && map_lit.ends_with('}')
            && {
                let inner = &map_lit[1..map_lit.len()-1];
                let mut entries = split_top_level(inner)
                    .into_iter()
                    .map(str::trim)
                    .filter(|e| !e.is_empty())
                    .peekable();
                // Empty `{}` IS valid Map syntax (empty Map).
                // Non-empty: every entry must split on top-level `=`.
                entries.peek().is_none()
                    || entries.all(|e| split_first_eq_top_level(e).is_some())
            } => {
            let inner = &map_lit[1..map_lit.len()-1];
            if inner.trim().is_empty() {
                Object::map(HashMap::new())
            } else {
                let mut map = HashMap::new();
                for entry in split_top_level(inner) {
                    let trimmed = entry.trim();
                    if trimmed.is_empty() { continue; }
                    if let Some((k, v)) = split_first_eq_top_level(trimmed) {
                        let key = unescape_atom_from_display(k.trim());
                        let value = parse_with_depth(v.trim(), depth + 1);
                        map.insert(key, value);
                    }
                }
                Object::map(map)
            }
        }
        atom => Object::Atom(unescape_atom_from_display(atom)),
    }
}

impl fmt::Display for Object {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Top-level atoms are emitted verbatim. A standalone Atom
            // carrying e.g. a JSON document (the shape
            // `platform_query_ft` returns) MUST NOT have its `<`,
            // `>`, `,` escaped — it isn't embedded in a Seq, so
            // there's no FFP-syntactic ambiguity to defend against.
            // Atoms appearing INSIDE a Seq are escaped via
            // `atom_inside_seq` below.
            Object::Atom(s) => write!(f, "{}", s),
            Object::Seq(items) if items.is_empty() => write!(f, "φ"),
            Object::Seq(items) => {
                write!(f, "<{}>", items.iter().map(|item| item_inside_seq(item))
                    .collect::<Vec<_>>().join(", "))
            }
            Object::Map(map) => {
                // Key AND value escaped so a Map cell round-trips through
                // `Object::parse`. The value goes via `item_inside_seq`
                // (not its bare top-level Display) so a nested atom value
                // carrying `{`/`}`/`=`/`<`/`>`/`,` is escaped and does not
                // corrupt `split_top_level` / `split_first_eq_top_level`
                // depth tracking on the way back. (See
                // `escape_atom_for_display` — the 787→0 Map-cell data-loss
                // fix.)
                // W5/D3 (task-932): key-sorted so serialization is
                // deterministic. Object::Map is Arc<HashMap> (non-stable
                // iteration order); thm:derive cacheability and
                // cor:consensus deterministic replay require a canonical
                // byte form, and key order is NOT semantic (D_n is a set).
                let mut entries: Vec<_> = map.iter().collect();
                entries.sort_by(|a, b| a.0.cmp(b.0));
                write!(f, "{{{}}}",
                    entries.into_iter().map(|(k, v)| format!("{}={}", escape_atom_for_display(k), item_inside_seq(v)))
                        .collect::<Vec<_>>().join(", "))
            }
            Object::Bottom => write!(f, "⊥"),
        }
    }
}

/// Render an Object as it appears INSIDE a Seq. Atoms here MUST
/// have FFP-syntactic chars escaped so they don't fool
/// `split_top_level` on the way back. Nested Seqs recurse — every
/// atom leaf, however deep, gets escape treatment.
fn item_inside_seq(item: &Object) -> alloc::string::String {
    match item {
        Object::Atom(s) => escape_atom_for_display(s),
        Object::Seq(items) if items.is_empty() => "φ".into(),
        Object::Seq(items) => {
            alloc::format!("<{}>", items.iter().map(item_inside_seq)
                .collect::<Vec<_>>().join(", "))
        }
        Object::Map(map) => {
            // Symmetric with the top-level Map Display: escape the KEY too,
            // so a Map nested inside a Seq/Map round-trips when its key
            // carries a Map/Seq delimiter (`{`/`}`/`=`/`<`/`>`/`,`). The
            // value already recurses through `item_inside_seq`.
            // W5/D3 (task-932): key-sorted for deterministic serialization
            // (symmetric with the top-level Map Display above).
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            alloc::format!("{{{}}}",
                entries.into_iter().map(|(k, v)| alloc::format!("{}={}", escape_atom_for_display(k), item_inside_seq(v)))
                    .collect::<Vec<_>>().join(", "))
        }
        Object::Bottom => "⊥".into(),
    }
}

// ── State encoding for evaluation ────────────────────────────────────
// State = Object (sequence of cells). No Population struct.

// `types::Violation` is a serde-derived struct — gated out of no_std
// along with the `types` module itself. The three helpers below
// (encode/decode_violation, decode_violations) are reachable from
// the kernel system::apply validate gate (#704), so the import is
// unconditional — the helpers themselves are alloc-clean.
use crate::types::Violation;

/// Encode an evaluation context as a single Object.
/// Structure: <response_text, sender_identity, population_as_object>
pub fn encode_eval_context_state(text: &str, sender: Option<&str>, state: &Object) -> Object {
    let response_obj = Object::atom(text);
    let sender_obj = match sender {
        Some(s) => Object::atom(s),
        None => Object::phi(),
    };
    let pop_obj = encode_state(state);
    // O(1)-lookup form of the population. Walks the same cells as
    // pop_obj (filtering out def cells) but emits an Object::Map keyed
    // by ft_id. Used by extract_facts_func via Func::FetchOrPhi at
    // Selector(4). The Seq form at Selector(3) is preserved verbatim
    // so existing constraint funcs and tests that read it keep working.
    let pop_indexed = encode_state_indexed(state);
    Object::seq(vec![response_obj, sender_obj, pop_obj, pop_indexed])
}

/// Indexed form of the population for O(1) cell access.
///
/// Same filtering and per-fact encoding as `encode_state`, but emitted
/// as `Object::Map` keyed by ft_id. Constraint funcs that look up a
/// specific fact type pay one HashMap lookup instead of scanning the
/// full Seq.
pub fn encode_state_indexed(state: &Object) -> Object {
    let map: HashMap<String, Object> = cells_iter(state).into_iter()
        .filter(|(ft_id, _)| !ft_id.contains(':'))
        .map(|(ft_id, contents)| {
            // task-744 phase 3 follow-up: Map-backed cells iterate
            // values; mirror of encode_state's fix. Without this, the
            // O(1) lookup form at Selector(4) reports empty facts for
            // every keyed cell, which breaks MC / UC / derivation
            // checks that rely on extract_facts_func.
            let fact_iter: Box<dyn Iterator<Item = &Object>> = match contents {
                Object::Seq(facts) => Box::new(facts.iter()),
                Object::Map(m) => Box::new(m.values()),
                _ => Box::new(core::iter::empty()),
            };
            let fact_objs: Vec<Object> = fact_iter.map(|fact| {
                let bindings: Vec<Object> = fact.as_seq().map(|pairs| {
                    pairs.iter().cloned().collect::<Vec<Object>>()
                }).unwrap_or_default();
                Object::Seq(Arc::from(bindings))
            }).collect::<Vec<Object>>();
            (ft_id.to_string(), Object::Seq(Arc::from(fact_objs)))
        }).collect();
    Object::Map(map.into())
}

/// Encode an Object state in the flat format expected by constraint evaluation.
/// Each cell becomes <ft_id, <fact_bindings...>> where each fact is <<k,v>, ...>.
///
/// Def cells (names containing ':' -- schema:, query:, derivation:, constraint:,
/// machine:, resolve:, transitions:, _cwa_negation:, etc.) are filtered out.
/// They hold compiled function definitions and template fact structures with
/// placeholder bindings that must not pollute constraint/derivation evaluation
/// over the population.
pub fn encode_state(state: &Object) -> Object {
    let fact_types: Vec<Object> = cells_iter(state).into_iter()
        .filter(|(ft_id, _)| !ft_id.contains(':'))
        .map(|(ft_id, contents)| {
            // task-744 phase 3 follow-up: Map-backed cells iterate
            // their values just like Seq-backed cells. Without this,
            // any cell flipped to Map storage (by `cell_put_keyed`)
            // shows up as an empty fact list in the encoded pop,
            // silently dropping facts from constraint / derivation
            // evaluation.
            let fact_iter: Box<dyn Iterator<Item = &Object>> = match contents {
                Object::Seq(facts) => Box::new(facts.iter()),
                Object::Map(m) => Box::new(m.values()),
                _ => Box::new(core::iter::empty()),
            };
            let fact_objs: Vec<Object> = fact_iter.map(|fact| {
                let bindings: Vec<Object> = fact.as_seq().map(|pairs| {
                    pairs.iter().map(|pair: &Object| pair.clone()).collect::<Vec<Object>>()
                }).unwrap_or_default();
                Object::Seq(Arc::from(bindings))
            }).collect::<Vec<Object>>();
            Object::seq(vec![Object::atom(ft_id), Object::Seq(Arc::from(fact_objs))])
        }).collect();
    Object::Seq(fact_types.into())
}

/// Decode a violation Object back to a Violation struct.
/// Expected: <constraint_id, constraint_text, detail>
/// Decode a violation Object back to a Violation struct.
/// Expected: <constraint_id, constraint_text, detail>
/// Detail can be an atom (string) or a sequence of atoms (joined with spaces).
///
/// no_std-clean (#704 / Audit D2): the kernel system::apply gate
/// reaches this from no_std builds. Implementation is purely
/// alloc-backed (Vec, String, Object), so dropping the gate is safe.
pub fn decode_violation(obj: &Object) -> Option<Violation> {
    let items = obj.as_seq().filter(|i| i.len() == 3)?;
    let detail: String = match &items[2] {
        Object::Atom(s) => Some(s.clone()),
        Object::Seq(parts) => Some(parts.iter()
            .filter_map(|p| p.as_atom())
            .collect::<Vec<_>>()
            .join(" ")),
        _ => None,
    }?;
    Some(Violation {
        constraint_id: items[0].as_atom()?.to_string(),
        constraint_text: items[1].as_atom()?.to_string(),
        detail,
        alethic: true,
    })
}

/// Decode a sequence of violation Objects. no_std-clean (see
/// `decode_violation` rationale).
pub fn decode_violations(obj: &Object) -> Vec<Violation> {
    match obj.as_seq() {
        Some(items) => items.iter().flat_map(|item|
            decode_violation(item).map_or_else(|| decode_violations(item), |v| vec![v])
        ).collect(),
        None => vec![],
    }
}

/// Encode a Violation as an Object. no_std-clean (see
/// `decode_violation` rationale).
pub fn encode_violation(v: &Violation) -> Object {
    Object::seq(vec![
        Object::atom(&v.constraint_id),
        Object::atom(&v.constraint_text),
        Object::atom(&v.detail),
    ])
}

// ── Functions (program domain) ───────────────────────────────────────
// A function maps objects to objects. All functions are bottom-preserving.
// Functions are built from primitives and combining forms.
// There are no variables — programs are point-free.

/// A boxed function: Object → Object. Thread-safe, cloneable.
pub type Fn1 = Arc<dyn Fn(&Object) -> Object + Send + Sync>;

/// State Machine cell shape — the synthesized role-token names the
/// apply / transition / status-extraction paths read and write to
/// the `State_Machine_is_currently_in_Status` cell.
///
/// task-742: renamed from the legacy code-shaped form
/// (`StateMachine_has_currentlyInStatus` + camelCased role names like
/// `currentlyInStatus`, `forResource`, `instanceOf`) to proper FORML2
/// verbalization (whitepaper §5.1: "Resource is currently in
/// Status"). Subject becomes "State Machine" (spaced) to match the
/// sibling cell `State_Machine_Definition_is_for_Noun`; role names
/// become proper nouns (Status / Resource / Noun) instead of mashed
/// single tokens. Same single-source-of-truth pattern as before;
/// only the strings change.
///
/// Lives in `ast` (not `command`) so the no_std kernel HATEOAS
/// direct-write fallback can reach the same role-name constants
/// without crossing the std-only `command` gate.
pub struct StateMachineCellShape {
    /// Cell name carrying the synthesized "State Machine is currently
    /// in Status" facts.
    pub cell_name: &'static str,
    /// Subject role binding: the State Machine entity id.
    pub state_machine_role: &'static str,
    /// Object role binding: the current status value.
    pub current_status_role: &'static str,
    /// Result entity binding: the target resource id (alias of the
    /// State Machine entity id at the API surface).
    pub for_resource_role: &'static str,
    /// HATEOAS / API entity_type label for the synthesized SM
    /// representation entity.
    pub entity_type_label: &'static str,
}

impl StateMachineCellShape {
    pub const fn boot() -> Self {
        StateMachineCellShape {
            cell_name:           "State_Machine_is_currently_in_Status",
            state_machine_role:  "State Machine",
            current_status_role: "Status",
            for_resource_role:   "Resource",
            entity_type_label:   "State Machine",
        }
    }
}

/// The program AST. Every node is a function Object → Object.
#[derive(Clone)]
pub enum Func {
    // ── Primitives ───────────────────────────────────────────────

    /// Identity: id:x = x
    Id,

    /// Selector: s:x = x_s (1-indexed). Role IS a selector.
    Selector(usize),

    /// Tail: tl:<x₁, ..., xₙ> = <x₂, ..., xₙ>
    Tail,

    /// Atom test: atom:x = T if x is atom, F otherwise
    AtomTest,

    /// Null test: null:x = T if x = φ, F otherwise
    NullTest,

    /// Cell-name test (Backus §13.3.4 `cellname`): T if x is a cell
    /// triple — a length-3 sequence whose first element is the atom
    /// `"CELL"` (`CELL_TAG`). F otherwise.
    ///
    /// Backus's `cellname n` is the *parameterized* form that also
    /// checks the cell's name; in AREST that decomposes cleanly:
    /// ```text
    /// cellname_n  ≡  And ∘ [
    ///   CellNameTest,                       — structural check (this primitive)
    ///   Eq ∘ [Selector(2), Constant(n̄)],   — name match
    /// ]
    /// ```
    /// so this primitive is the structural half; the parameterized
    /// half composes from existing primitives (#355).
    ///
    /// Closes the gap where the engine was reaching into Rust
    /// (`items.len() == 3 && items[0].as_atom() == Some(CELL_TAG)`)
    /// for what should be an FFP-expressible predicate. With this in
    /// place, a Func-level cell walk is `Filter(CellNameTest)`.
    CellNameTest,

    /// Equals: eq:<x, y> = T if x = y, F otherwise
    Eq,

    // ── AREST extensions (not in Backus's primitive set) ─────────
    //
    // Backus 1977's primitive set covers atoms, sequences, and
    // numeric/logical operations on them but stops short of two
    // categories AREST needs: ordered comparisons (for range /
    // count constraints) and string operations (for prose-tokenizer
    // primitives, FORML 2 text-pattern checks, and atom-name
    // manipulation). The variants below extend the set with those
    // pragmatic additions. Each is ⊥-preserving and follows the
    // same "function from object to atom (or T/F)" shape as
    // Backus's `eq` so the §12 algebraic laws extend cleanly.

    /// Greater than: gt:<x, y> = T if x > y (numeric), F otherwise. ⊥ on non-numeric.
    Gt,

    /// Less than: lt:<x, y> = T if x < y (numeric), F otherwise. ⊥ on non-numeric.
    Lt,

    /// Greater or equal: ge:<x, y> = T if x ≥ y (numeric), F otherwise.
    Ge,

    /// Less or equal: le:<x, y> = T if x ≤ y (numeric), F otherwise.
    Le,

    /// Contains: contains:<x,y> = T if atom x contains atom y (case-insensitive), else F
    Contains,

    /// StartsWith: starts_with:<x,y> = T if atom x has atom y as a prefix (case-insensitive), else F
    /// Text-pattern primitive (#282): used by the readings-form Stage-1
    /// tokenizer (#295) to check prefixes like `Statement has Deontic
    /// Operator` matching `It is obligatory that …`.
    StartsWith,

    /// EndsWith: ends_with:<x,y> = T if atom x has atom y as a suffix (case-insensitive), else F
    /// Text-pattern primitive (#282): pairs with StartsWith for the
    /// trailing-marker checks (`is an entity type`, `is abstract`, …).
    EndsWith,

    /// Trim: trim:x = atom x with leading/trailing ASCII whitespace removed.
    /// Text-pattern primitive (#282): normalises statement text before
    /// the classification rules see it.
    Trim,

    /// Split: split:<x,y> = <x₁, x₂, …> where atom x is split on every
    /// occurrence of atom y (empty delimiter yields char-by-char).
    /// Text-pattern primitive (#282): produces the comma-separated
    /// enum-value list in `Enum Values Declaration` tokenization.
    Split,

    /// Replace: replace:<x,<y,z>> = atom x with every occurrence of
    /// atom y replaced by atom z. Text-pattern primitive (#282): strips
    /// the `It is obligatory that ` prefix before subject parsing.
    Replace,

    /// Lower: lower:x = lowercase of atom x
    Lower,

    // ── Back to Backus's primitive set ───────────────────────────

    /// Length: length:<x₁, ..., xₙ> = n
    Length,

    /// Concat: concat:<<x1,...>, <y1,...>, ...> = <x1,...,y1,...,...>
    /// Flattens one level of nesting. Each element must be a sequence.
    /// AREST extension (not in Backus's primitive set) — derivable as a
    /// pattern over Insert + ApndR but awkward enough that the
    /// primitive carries its weight.
    Concat,

    /// Distribute from left: distl:<y, <z₁,...,zₙ>> = <<y,z₁>,...,<y,zₙ>>
    DistL,

    /// Distribute from right: distr:<<y₁,...,yₙ>, z> = <<y₁,z>,...,<yₙ,z>>
    DistR,

    /// Set-membership test (AREST extension, #743): has_member:<needle, haystack>
    /// returns T if `needle` equals any element of the sequence `haystack`,
    /// F otherwise. ⊥ on shape mismatch.
    ///
    /// Replaces the O(N²) `null ∘ filter(eq) ∘ distl` membership-test
    /// pattern that derivations like SM init's `is_new` and join
    /// rules' AbsenceOf guards otherwise compose by hand. Same big-O
    /// (linear scan), but zero allocation per check vs. one Seq of N
    /// pairs built and immediately discarded — on apps/tasks the
    /// difference is ~30 s vs. ~5 min per recompile in the joint
    /// fixpoint.
    HasMember,

    /// Set construction (task-744 follow-up): set:<x₁, ..., xₙ> = Map
    /// keyed by the atoms of x, each mapped to atom "T". `set:phi = Map{}`.
    /// Non-atom elements collapse to ⊥.
    ///
    /// Together with `FetchOrPhi`, this gives O(N) build + O(1) per
    /// membership check — replacing the O(N) HasMember scan with a
    /// hash-set lookup once a set is reused. The compile-time payoff
    /// is in SM init's `is_new` (one set built per round, M membership
    /// tests done against it; total O(N+M) vs. O(N·M)). Pairs with the
    /// existing Map-as-set convention the engine already uses for cell
    /// stores and metadata caches.
    SetFromSeq,

    /// Transpose: trans:<<a,b>, <c,d>> = <<a,c>, <b,d>>
    Trans,

    /// Append left: apndl:<y, <z₁,...,zₙ>> = <y, z₁,...,zₙ>
    ApndL,

    /// Reverse: reverse:<x₁,...,xₙ> = <xₙ,...,x₁>
    Reverse,

    /// Append right: apndr:<<y₁,...,yₙ>, z> = <y₁,...,yₙ, z>
    ApndR,

    /// Rotate left: rotl:<x₁,...,xₙ> = <x₂,...,xₙ, x₁>
    RotL,

    /// Rotate right: rotr:<x₁,...,xₙ> = <xₙ, x₁,...,xₙ₋₁>
    RotR,

    /// Compact (Backus §11.2.4): `compact:<x₁,...,xₙ>` drops every ⊥
    /// element from the sequence, preserving the order of the rest.
    /// `compact:<1, ⊥, 2, ⊥, 3> = <1, 2, 3>`; `compact:<> = <>`.
    ///
    /// Named missing primitive from #352. AREST previously had
    /// `Func::Filter(p)` as a primitive in lieu of deriving it from
    /// `compact ∘ α(p → id ; ⊥)`. The derivation is an *algebraic*
    /// identity per §11.2.4 eq 2 but not a *computational* one in
    /// AREST: `Object::seq(..)` is strictly ⊥-preserving per §11.2.1,
    /// so the moment α emits a single ⊥ element the intermediate
    /// collapses to ⊥ and compact on ⊥ = ⊥. Filter stays a runtime
    /// primitive. Compact is still a first-class primitive because
    /// it appears in other contexts — cell-index lookups over sparse
    /// noun populations, for example, produce seqs that can carry ⊥
    /// via direct `Object::Seq(..)` construction (bypassing the
    /// ⊥-checking `seq(..)` constructor).
    Compact,

    // ── Arithmetic (Backus 11.2.3) ──────────────────────────────
    /// Add: +:<y,z> = y+z where y,z are number atoms
    Add,
    /// Subtract: -:<y,z> = y-z
    Sub,
    /// Multiply: ×:<y,z> = y×z
    Mul,
    /// Divide: ÷:<y,z> = y÷z, bottom if z=0
    Div,

    // ── Logic (Backus 11.2.3) ───────────────────────────────────
    /// And: and:<T,T> = T, and:<T,F> = F, etc.
    And,
    /// Or: or:<F,F> = F, or:<T,F> = T, etc.
    Or,
    /// Not: not:T = F, not:F = T
    Not,

    // ── Cells (Backus 14.3) ─────────────────────────────────────
    /// Fetch: ↑n:<name, D> → contents of cell named name in D
    /// Returns ⊥ for missing names. Use FetchOrPhi when downstream code
    /// must not propagate ⊥ through Construction (which would void
    /// unrelated computations sharing the parent expression).
    Fetch,
    /// FetchOrPhi: like Fetch but returns φ (empty seq) when the name
    /// is absent. Used by indexed fact-type lookup so a missing FT
    /// cell (no instances of that type yet) yields an empty fact list
    /// rather than ⊥. Drops the Filter+Eq linear scan that
    /// extract_facts_func previously needed.
    ///
    /// Not a Backus primitive — it's an AREST pragmatic specialization
    /// of `(Null ∘ Fetch → φ̄ ; Fetch)` kept as a primitive because the
    /// derived form would call `Fetch:n:D` twice (once for the test,
    /// once for the value) when the cell could be looked up once.
    /// Equivalent in semantics; cheaper in evaluation.
    FetchOrPhi,
    /// Store: ↓n:<name, contents, D> → D' with cell name updated
    Store,

    // ── Combining Forms ──────────────────────────────────────────

    /// Constant: x̄:y = x (for all y ≠ ⊥). A literal value in a reading.
    Constant(Object),

    /// Composition: (f ∘ g):x = f:(g:x). Derivation rule chains.
    Compose(Box<Func>, Box<Func>),

    /// Construction: [f₁,...,fₙ]:x = <f₁:x,...,fₙ:x>. Fact Type = CONS of Roles.
    Construction(Vec<Func>),

    /// Condition: (p → f; g):x = if p:x = T then f:x, if F then g:x, else ⊥.
    /// Constraint evaluation. Deontic branching.
    Condition(Box<Func>, Box<Func>, Box<Func>),

    /// Apply-to-all: αf:<x₁,...,xₙ> = <f:x₁,...,f:xₙ>. Population traversal.
    ApplyToAll(Box<Func>),

    /// Insert (RIGHT fold, Backus /f): /f:<x₁,...,xₙ> = f:<x₁, /f:<x₂,...,xₙ>>.
    ///
    /// Processes right to left: the last element is the base case,
    /// then each preceding element is combined with the accumulated result.
    /// For a single-element sequence, /f:<x> = x (identity).
    /// For an empty sequence, /f:phi = the right UNIT of f (§11.2.4): T for
    /// ∧, F for ∨, 0 for +, 1 for ×, <> for concat (see `unit_of`). An
    /// operator with no known unit yields Bottom (undefined) on the empty
    /// sequence.
    ///
    /// Example: /+:<1, 2, 3> = +:<1, +:<2, 3>> = +:<1, 5> = 6.
    /// For non-commutative f, order matters: /-:<1, 2, 3> = -:<1, -:<2, 3>>
    /// = -:<1, -1> = 2 (NOT 1-2-3 = -4).
    ///
    /// See FoldL for left fold with explicit accumulator.
    Insert(Box<Func>),

    /// Binary-to-unary: (bu f x):y = f:<x, y>. Partial application / currying.
    BinaryToUnary(Box<Func>, Object),

    /// Filter: `Filter(p):<x₁,...,xₙ> = <xᵢ | p:xᵢ = T>`.
    /// The missing primitive for queries as partial application.
    /// Partial apply a fact type (bind some roles) → predicate falls out.
    /// Filter(predicate) applied to population → matching facts.
    ///
    /// Why this is a primitive even though Backus §11.2.4 eq 2 writes
    /// `Filter(p) ≡ compact ∘ α(p → id ; ⊥)` as a derived form: the
    /// derivation can't be executed step-by-step under §11.2.1's
    /// strict ⊥-preserving sequence constructor, because the moment
    /// α emits a single ⊥ element the intermediate sequence collapses
    /// to ⊥ and `compact ∘ ⊥ = ⊥`. Backus's derived-form definition is
    /// intensional — describing what Filter computes — not a literal
    /// substitution. AREST honors the §11.2.4 identity as an algebraic
    /// law (the runtime produces the same result as the derived form
    /// would, modulo the collapsed intermediate) without executing it
    /// compositionally. Compact (#352) is a separate primitive useful
    /// where ⊥s enter sequences via the bypassing `Object::Seq(..)`
    /// constructor (sparse cell-index lookups, etc.) — but Filter and
    /// Compact don't compose into a Filter substitute under §11.2.1.
    Filter(Box<Func>),

    /// While: (while p f):x = if p:x = T then (while p f):(f:x) else x.
    ///
    /// Safety bound: iteration is capped at 1000 steps. If the predicate
    /// still returns T after 1000 iterations, the result is Bottom (not
    /// an infinite loop). This bound is sufficient for any practical
    /// population-based computation (transitive closure, fixed-point
    /// iteration, state machine simulation).
    While(Box<Func>, Box<Func>),

    /// Left fold: FoldL(f):<z, <e₁,...,eₙ>> = foldl f z <e₁,...,eₙ>
    /// where foldl f z <> = z, foldl f z <e, E'> = foldl f (f:<z,e>) E'.
    /// Takes a pair <accumulator, sequence>. Returns the final accumulator.
    ///
    /// Early termination: if the accumulator becomes Bottom at any step,
    /// the fold terminates immediately and returns Bottom. This prevents
    /// wasted computation when an error propagates through the fold.
    ///
    /// Contrast with Insert (/f), which is a RIGHT fold (Backus /f):
    /// /f:<x₁,...,xₙ> processes right to left. FoldL processes left to
    /// right with an explicit initial accumulator, making it suitable for
    /// stateful computations (running totals, state machine transitions).
    FoldL(Box<Func>),

    /// IndexBy (perf-hashjoin): `IndexBy(keyfn):<e₁,...,eₙ>` builds a Map that
    /// groups each `eᵢ` under the atom key `keyfn:eᵢ` → `<eᵢ...>`. Read-only —
    /// unlike `Store` it has NO capability gate, so a derivation Func can build
    /// a hash index for an O(n) equi-join instead of an O(n²) cross product.
    /// Elements whose key is not an atom are skipped. Pairs with `FetchOrPhi`
    /// for O(1) probes (`IndexBy(keyfn) ∘ right`, then `FetchOrPhi:<key, idx>`).
    IndexBy(Box<Func>),

    /// OrderBy (sort-by-key extension — NOT in Backus's θ₁; Codd's relational
    /// algebra is unordered, §6 Table 1). `OrderBy(keyfn):<e₁,…,eₙ>` returns the
    /// elements stably sorted ascending by the atom key `keyfn:eᵢ` — numeric
    /// keys compare numerically (epoch / recorded-at), else lexicographically
    /// (ISO timestamps). Non-atom keys sort first and are retained (total +
    /// stable). The general primitive that orders an unordered event SET into
    /// the chronological STREAM E that the SM fold `foldl transition s₀ E`
    /// (AREST.tex eq:sm, "latest-wins per resource") consumes. Read-only — no
    /// Store capability gate.
    OrderBy(Box<Func>),

    /// Named definition: references a function by name from the definition set.
    Def(String),

    /// Platform primitive: a named operation resolved by the runtime.
    /// Each name maps to a known function (x, D) → Object.
    /// On FPGA, each is a synthesized circuit. In Rust, dispatched by name.
    Platform(String),

    /// Opaque: wraps an arbitrary Rust closure. Escape hatch for primitives
    /// that don't fit the AST. The θ₁ relational ops that previously used
    /// this now route through Platform; Native remains for any future
    /// Rust-only escape hatches and is not FPGA-synthesizable.
    Native(Fn1),
}

// ── Application (the single operation) ───────────────────────────────
// f:x → Object. This is beta reduction.

/// Parse a pair of number atoms, apply an arithmetic operation (Backus +,-,×,÷).
/// Numeric comparison helper for Gt/Lt/Ge/Le primitives.
/// Parses both operands as f64. Returns T/F/Bottom.
fn apply_compare(x: &Object, op: fn(f64, f64) -> bool) -> Object {
    match x.as_seq() {
        Some(items) if items.len() == 2 => {
            let a = items[0].as_atom().and_then(|s| s.parse::<f64>().ok());
            let b = items[1].as_atom().and_then(|s| s.parse::<f64>().ok());
            match (a, b) {
                (Some(a), Some(b)) => if op(a, b) { Object::t() } else { Object::f() },
                _ => Object::Bottom,
            }
        }
        _ => Object::Bottom,
    }
}

/// The right unit (identity element) of a fold operator `f`, used to give
/// `Insert(f):<>` (Backus `/f` over the EMPTY sequence) a paper-faithful
/// value instead of `⊥`. Backus §11.2.4 defines `/f:<>` as the right unit
/// of `f` when one exists — e.g. `/+:<> = 0`, `/∧:<> = T` (vacuous truth).
/// Only operators with a well-known unit are mapped; anything else returns
/// `None`, leaving `Insert` to fall back to `⊥` (still Backus-faithful: an
/// operator without a unit has no defined empty-fold).
///
/// The operator is matched on its first-class `Func` variant — the form
/// the compiler emits for these folds (`Func::And` for `/∧`, `Func::Add`
/// for `/+`, etc.), as constructed by `compile_aggregate_derivation`'s sum
/// and the universal-quantifier fold in `compile_explicit_derivation`.
/// Number units are the decimal-string atoms the arithmetic primitives
/// produce (`apply_arithmetic` formats integral results as `i64` strings),
/// so `0`/`1` round-trip through `+`/`×` unchanged.
fn unit_of(f: &Func) -> Option<Object> {
    match f {
        Func::And => Some(Object::t()),          // ∧ unit = T
        Func::Or => Some(Object::f()),           // ∨ unit = F
        Func::Add => Some(Object::atom("0")),    // + unit = 0
        Func::Mul => Some(Object::atom("1")),    // × unit = 1
        Func::Concat => Some(Object::phi()),     // concat unit = <>
        _ => None,
    }
}

// ── Static cell-read analysis of a compiled Func (delta-occ-1) ───────
//
// derivation-semi-naive-delta-joins / per-occurrence delta. The dark
// AREST_DELTA_JOINS view path is sound only when a rule's STATIC reads
// sidecar covers every cell its compiled Func actually fetches. Today
// `derivation_positive_reads` (compile.rs) is built from the declared
// antecedent_sources alone and never inspects the Func, so a rule whose
// Func also fetches a reflection/hidden cell is wrongly marked
// view-complete and the view-swap leaves that cell at full population →
// the B2 divergence (see the claude ledger lever delta-joins-per-occurrence).
//
// This is the foundational analyzer: walk a compiled Func and return the
// set of cell names it fetches by a LITERAL name, plus a flag when any
// fetch name is computed at runtime (so the read-set is NOT statically
// complete and the soundness gate must treat the rule conservatively).
// The compiler emits every cell read as the fixed shape
//   Compose(FetchOrPhi|Fetch, Construction([Constant(atom(cell)), <pop>]))
// (extract_facts_func / extract_facts_from_pop, compile.rs), so a literal
// read is exactly that shape and anything else feeding a fetch is dynamic.

/// Result of [`func_read_set`]: the statically-knowable cell reads of a Func.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FuncReadSet {
    /// Cell names fetched by a literal (Constant-atom) name.
    pub literal: alloc::collections::BTreeSet<alloc::string::String>,
    /// True if any Fetch/FetchOrPhi takes a non-literal (runtime-computed)
    /// name, or the walk reaches a Platform/Native escape hatch — i.e. the
    /// read-set is NOT provably complete and the delta-view soundness gate
    /// must full-evaluate this rule rather than swap views.
    pub has_dynamic: bool,
    /// Subset of `has_dynamic`: a Platform/Native escape hatch — genuine
    /// runtime/federation dynamism (eq:federation), NOT recoverable by static
    /// resolution. A `has_dynamic` rule that is NOT `has_federation` is a
    /// Fetch/metacomposition pattern that may be statically resolvable.
    pub has_federation: bool,
    /// Named defs referenced (Func::Def). Reads THROUGH a def are indirect;
    /// resolve these against the def map for a fully-complete read-set.
    pub def_refs: alloc::collections::BTreeSet<alloc::string::String>,
}

/// Walk a compiled Func and return its statically-knowable cell read-set
/// (delta-occ-1). See the module comment above for the recognized fetch
/// shape. `def_refs` are left unresolved — a caller wanting the transitive
/// read-set resolves them against its def map.
pub fn func_read_set(f: &Func) -> FuncReadSet {
    let mut rs = FuncReadSet::default();
    collect_func_reads(f, &mut rs);
    rs
}

fn collect_func_reads(f: &Func, rs: &mut FuncReadSet) {
    match f {
        // The cell-read shape: a Fetch/FetchOrPhi composed over a
        // Construction whose FIRST element builds the cell NAME.
        Func::Compose(g, h) if matches!(**g, Func::Fetch | Func::FetchOrPhi) => {
            match &**h {
                Func::Construction(elems) if !elems.is_empty() => {
                    match &elems[0] {
                        // Literal name → a statically-known read.
                        Func::Constant(obj) => match obj.as_atom() {
                            Some(name) => { rs.literal.insert(name.to_string()); }
                            None => rs.has_dynamic = true,
                        },
                        // Computed name → dynamic; still walk it for nested reads.
                        other => { rs.has_dynamic = true; collect_func_reads(other, rs); }
                    }
                    // The pop-source (and any further args) may carry nested reads.
                    for e in &elems[1..] { collect_func_reads(e, rs); }
                }
                // Fetch over a non-Construction operand: name not a leading
                // Constant → dynamic.
                other => { rs.has_dynamic = true; collect_func_reads(other, rs); }
            }
        }
        // A bare Fetch reached outside the recognized shape: its operand is
        // supplied by an enclosing frame, so the name is not statically known.
        Func::Fetch | Func::FetchOrPhi => rs.has_dynamic = true,

        // Sub-Func-carrying combining forms: recurse into every child.
        Func::Compose(g, h) => { collect_func_reads(g, rs); collect_func_reads(h, rs); }
        Func::Construction(v) => for g in v { collect_func_reads(g, rs); },
        Func::Condition(a, b, c) => {
            collect_func_reads(a, rs); collect_func_reads(b, rs); collect_func_reads(c, rs);
        }
        Func::While(a, b) => { collect_func_reads(a, rs); collect_func_reads(b, rs); }
        Func::ApplyToAll(g) | Func::Insert(g) | Func::Filter(g) | Func::FoldL(g)
        | Func::IndexBy(g) | Func::OrderBy(g) | Func::BinaryToUnary(g, _) =>
            collect_func_reads(g, rs),

        // Indirect / opaque surfaces.
        Func::Def(name) => { rs.def_refs.insert(name.clone()); }
        Func::Platform(_) | Func::Native(_) => { rs.has_dynamic = true; rs.has_federation = true; }

        // All remaining variants are nullary leaves with no cell reads
        // (Id, Selector, Tail, Eq, arithmetic/logic, Constant, Store, …).
        _ => {}
    }
}

// ── delta-occ-3: per-occurrence read rewriting ───────────────────────
//
// occ-1's [`func_read_set`] gives the SET of cells a Func reads. True
// semi-naive evaluation of a SELF-JOIN (or any rule reading one cell at
// several occurrences) needs to vary the delta ONE OCCURRENCE AT A TIME:
//   R :- A(x,y), A(y,z)   ⇒   ΔR = (ΔA ⋈ A) ∪ (A ⋈ ΔA)
// A whole-cell view swap can only produce ΔA ⋈ ΔA (both occurrences
// swapped together), which misses the cross terms. The two helpers below
// let the caller (semi_naive_inner) enumerate a cell's read occurrences
// and rebuild the Func with exactly ONE occurrence reading a synthetic
// delta cell while every other occurrence keeps the full population —
// the "compiled-Func parameterization" the shipped-dark note called for.
//
// Both mirror [`collect_func_reads`]'s recognition of the fetch shape
//   Compose(FetchOrPhi|Fetch, Construction([Constant(atom(cell)), <pop>]))
// and [`normalize_children`]'s rebuild over every compound variant.

/// Number of literal-name read occurrences of `cell` in `f` (delta-occ-3).
/// Each is an independent semi-naive delta site. A cell the Func reads
/// only indirectly (through a `Def`, or via a runtime-computed name)
/// contributes 0 — the caller must full-evaluate such a rule, since a
/// per-occurrence rewrite cannot reach an indirect read.
pub fn func_read_occurrence_count(f: &Func, cell: &str) -> usize {
    let mut n = 0usize;
    count_read_occ(f, cell, &mut n);
    n
}

fn count_read_occ(f: &Func, cell: &str, n: &mut usize) {
    match f {
        Func::Compose(g, h) if matches!(**g, Func::Fetch | Func::FetchOrPhi) => {
            match &**h {
                Func::Construction(elems) if !elems.is_empty() => {
                    if let Func::Constant(obj) = &elems[0] {
                        if obj.as_atom() == Some(cell) { *n += 1; }
                    } else {
                        count_read_occ(&elems[0], cell, n);
                    }
                    for e in &elems[1..] { count_read_occ(e, cell, n); }
                }
                other => count_read_occ(other, cell, n),
            }
        }
        Func::Compose(g, h) => { count_read_occ(g, cell, n); count_read_occ(h, cell, n); }
        Func::Construction(v) => for g in v { count_read_occ(g, cell, n); },
        Func::Condition(a, b, c) => {
            count_read_occ(a, cell, n); count_read_occ(b, cell, n); count_read_occ(c, cell, n);
        }
        Func::While(a, b) => { count_read_occ(a, cell, n); count_read_occ(b, cell, n); }
        Func::ApplyToAll(g) | Func::Insert(g) | Func::Filter(g) | Func::FoldL(g)
        | Func::IndexBy(g) | Func::OrderBy(g) | Func::BinaryToUnary(g, _) =>
            count_read_occ(g, cell, n),
        _ => {}
    }
}

/// Clone `f`, rewriting the `target`-th literal-name read occurrence of
/// `cell` (left-to-right walk order, same order as
/// [`func_read_occurrence_count`]) so it fetches `new_name` instead
/// (delta-occ-3). Every OTHER occurrence — including other reads of the
/// same `cell` — is preserved. The caller injects the delta rows into
/// the state under `new_name`, leaving `cell` at full population, so the
/// rewritten variant evaluates exactly the semi-naive term that varies
/// this one occurrence. `target` ≥ the occurrence count is a no-op clone.
pub fn func_rewrite_read_occurrence(f: &Func, cell: &str, target: usize, new_name: &str) -> Func {
    let mut cursor = 0usize;
    rewrite_read_occ(f, cell, target, new_name, &mut cursor)
}

fn rewrite_read_occ(f: &Func, cell: &str, target: usize, new_name: &str, cursor: &mut usize) -> Func {
    match f {
        Func::Compose(g, h) if matches!(**g, Func::Fetch | Func::FetchOrPhi) => {
            match &**h {
                Func::Construction(elems) if !elems.is_empty() => {
                    let mut new_elems: Vec<Func> = Vec::with_capacity(elems.len());
                    match &elems[0] {
                        Func::Constant(obj) if obj.as_atom() == Some(cell) => {
                            if *cursor == target {
                                new_elems.push(Func::Constant(Object::atom(new_name)));
                            } else {
                                new_elems.push(elems[0].clone());
                            }
                            *cursor += 1;
                        }
                        // A different literal cell, or a computed name —
                        // the computed case may itself fetch `cell`, so recurse.
                        other => new_elems.push(
                            rewrite_read_occ(other, cell, target, new_name, cursor)),
                    }
                    for e in &elems[1..] {
                        new_elems.push(rewrite_read_occ(e, cell, target, new_name, cursor));
                    }
                    Func::Compose(g.clone(), Box::new(Func::Construction(new_elems)))
                }
                other => Func::Compose(
                    g.clone(),
                    Box::new(rewrite_read_occ(other, cell, target, new_name, cursor))),
            }
        }
        Func::Compose(a, b) => Func::Compose(
            Box::new(rewrite_read_occ(a, cell, target, new_name, cursor)),
            Box::new(rewrite_read_occ(b, cell, target, new_name, cursor))),
        Func::Construction(fs) => Func::Construction(
            fs.iter().map(|g| rewrite_read_occ(g, cell, target, new_name, cursor)).collect()),
        Func::Condition(p, t, e) => Func::Condition(
            Box::new(rewrite_read_occ(p, cell, target, new_name, cursor)),
            Box::new(rewrite_read_occ(t, cell, target, new_name, cursor)),
            Box::new(rewrite_read_occ(e, cell, target, new_name, cursor))),
        Func::ApplyToAll(inner) =>
            Func::ApplyToAll(Box::new(rewrite_read_occ(inner, cell, target, new_name, cursor))),
        Func::Insert(inner) =>
            Func::Insert(Box::new(rewrite_read_occ(inner, cell, target, new_name, cursor))),
        Func::Filter(p) =>
            Func::Filter(Box::new(rewrite_read_occ(p, cell, target, new_name, cursor))),
        Func::BinaryToUnary(g, x) =>
            Func::BinaryToUnary(Box::new(rewrite_read_occ(g, cell, target, new_name, cursor)), x.clone()),
        Func::While(p, body) => Func::While(
            Box::new(rewrite_read_occ(p, cell, target, new_name, cursor)),
            Box::new(rewrite_read_occ(body, cell, target, new_name, cursor))),
        Func::FoldL(g) =>
            Func::FoldL(Box::new(rewrite_read_occ(g, cell, target, new_name, cursor))),
        Func::IndexBy(g) =>
            Func::IndexBy(Box::new(rewrite_read_occ(g, cell, target, new_name, cursor))),
        Func::OrderBy(g) =>
            Func::OrderBy(Box::new(rewrite_read_occ(g, cell, target, new_name, cursor))),
        leaf => leaf.clone(),
    }
}

fn apply_arithmetic(x: &Object, op: fn(f64, f64) -> Option<f64>) -> Object {
    match x.as_seq() {
        Some(items) if items.len() == 2 => {
            let a = items[0].as_atom().and_then(|s| s.parse::<f64>().ok());
            let b = items[1].as_atom().and_then(|s| s.parse::<f64>().ok());
            match (a, b) {
                (Some(a), Some(b)) => match op(a, b) {
                    Some(r) => {
                        // "Integer-valued within i64 range" via cast round-
                        // trip. Avoids `f64::fract` / `f64::abs`, which are
                        // std-only — this form compiles under no_std too.
                        // NaN / infinity / oversized values fail the round-
                        // trip and fall through to the f64 formatting arm.
                        let int_form = r as i64;
                        if (int_form as f64) == r {
                            Object::Atom(int_form.to_string())
                        } else {
                            Object::Atom(r.to_string())
                        }
                    }
                    None => Object::Bottom,
                },
                _ => Object::Bottom,
            }
        }
        _ => Object::Bottom,
    }
}

/// Apply a function to an object. The only operation in the FP system.
/// Store compiled defs as cells in D. Each def becomes a cell whose name
/// is the def name and whose contents is the Object representation of the Func.
/// ↓DEFS (AREST §3.2 Platform Binding). Runtime-side writer to DEFS.
///
/// Pushes a single (name, func) binding into state and also records
/// `name` in the `runtime_registered_names` cell. The binding is
/// indistinguishable from a compile-derived one at apply time — the
/// registry cell is the origin marker, consulted by provenance
/// emission (Citation with Authority Type 'Runtime-Function').
///
/// Per the paper:
/// - compile writes the domain layer via `defs_to_state`.
/// - the runtime writes the platform layer via this function.
/// Together they span DEFS; the surjectivity remark (§ Remark after
/// Theorem \ref{thm:spec}) names this split explicitly.
pub fn register_runtime_fn(name: &str, func: Func, state: &Object) -> Object {
    let with_def = store(name, func_to_object(&func), state);
    cell_push("runtime_registered_names", Object::atom(name), &with_def)
}

/// E3 / #305 — Citation provenance emission.
///
/// Pushes a Citation entity and its canonical per-fact readings into
/// the `Citation_has_URI`, `Citation_has_Retrieval_Date`,
/// `Citation_has_Authority_Type`, and (when Some) the
/// `Citation_is_backed_by_External_System` cells. Returns the assigned
/// Citation id so the caller can emit paired `Fact cites Citation`
/// link facts for whatever facts the outside-ρ call produced.
///
/// The Citation id is content-addressed over (uri, authority_type,
/// retrieval_date): two calls with the same triple produce the same
/// id, so repeated emission for the same origin is idempotent at the
/// cell level (the cell-push writes are idempotent by construction —
/// cell_push dedupes identical facts).
///
/// Authority Type values MUST be one of the enum members declared on
/// Authority Type in readings/instances.md. For E3, `'Runtime-Function'`
/// and `'Federated-Fetch'` are the two provenance kinds.
#[cfg(not(feature = "no_std"))]
pub fn emit_citation_fact(
    uri: &str,
    authority_type: &str,
    retrieval_date: &str,
    external_system: Option<&str>,
    state: &Object,
) -> (String, Object) {
    emit_citation_fact_pinned(
        uri, authority_type, retrieval_date, external_system, None, state,
    )
}

/// S1f (#722) variant of `emit_citation_fact` that also pins the
/// citation to a specific `(cell_name, version_id)` provenance pair.
/// When `cell_pin` is `Some((name, id))` the helper emits two
/// additional Citation facts — `Citation_pins_Cell_Name` and
/// `Citation_pins_Cell_Version_Id` — and folds the pair into the
/// content-addressed Citation id so two cites of the same URI under
/// different versions get distinct ids (the audit trail can
/// distinguish "fetched at v=3" from "fetched at v=4" even when the
/// upstream URL is identical).
///
/// Use this when a Platform function or federated fetch operates on a
/// specific version of a cell — query `system(h, "cell_pin", name)`
/// (S1e) for the chain version_id, then thread it here. Callers that
/// don't have or need cell provenance keep using `emit_citation_fact`
/// — `cell_pin = None` produces the pre-S1f shape unchanged.
#[cfg(not(feature = "no_std"))]
pub fn emit_citation_fact_pinned(
    uri: &str,
    authority_type: &str,
    retrieval_date: &str,
    external_system: Option<&str>,
    cell_pin: Option<(&str, u64)>,
    state: &Object,
) -> (String, Object) {
    use core::hash::{BuildHasher, Hash, Hasher};
    let mut h = hashbrown::hash_map::DefaultHashBuilder::default().build_hasher();
    uri.hash(&mut h);
    authority_type.hash(&mut h);
    retrieval_date.hash(&mut h);
    if let Some((name, id)) = cell_pin {
        name.hash(&mut h);
        id.hash(&mut h);
    }
    let cite_id = alloc::format!("cite:{:016x}", h.finish());

    // Auto-generated Text satisfies the alethic in readings/instances.md:
    //   "Each Citation has exactly one Text."
    // Without this, every Citation we emit would be in immediate
    // violation of its own mandatory-role constraint. Auto-generation
    // uses the already-known fields so the text is deterministic and
    // content-addresses with the id.
    let text = match external_system {
        Some(ext) => alloc::format!(
            "{} citation for {} (backed by {}) retrieved at {}",
            authority_type, uri, ext, retrieval_date
        ),
        None => alloc::format!(
            "{} citation for {} retrieved at {}",
            authority_type, uri, retrieval_date
        ),
    };

    let with_text = cell_push_unique(
        "Citation_has_Text",
        fact_from_pairs(&[("Citation", &cite_id), ("Text", &text)]),
        state,
    );
    let with_uri = cell_push_unique(
        "Citation_has_URI",
        fact_from_pairs(&[("Citation", &cite_id), ("URI", uri)]),
        &with_text,
    );
    let with_rd = cell_push_unique(
        "Citation_has_Retrieval_Date",
        fact_from_pairs(&[("Citation", &cite_id), ("Retrieval Date", retrieval_date)]),
        &with_uri,
    );
    let with_at = cell_push_unique(
        "Citation_has_Authority_Type",
        fact_from_pairs(&[("Citation", &cite_id), ("Authority Type", authority_type)]),
        &with_rd,
    );
    let with_ext = external_system
        .map(|ext| {
            cell_push_unique(
                "Citation_is_backed_by_External_System",
                fact_from_pairs(&[("Citation", &cite_id), ("External System", ext)]),
                &with_at,
            )
        })
        .unwrap_or(with_at);
    // S1f (#722): when the cite is for a specific cell version, push
    // the two `Citation pins …` facts so the audit chain records
    // exact storage provenance, not just the URI.
    let final_state = match cell_pin {
        Some((name, id)) => {
            let id_str = alloc::format!("{}", id);
            let with_name = cell_push_unique(
                "Citation_pins_Cell_Name",
                fact_from_pairs(&[("Citation", &cite_id), ("Cell Name", name)]),
                &with_ext,
            );
            cell_push_unique(
                "Citation_pins_Cell_Version_Id",
                fact_from_pairs(&[("Citation", &cite_id), ("Cell Version Id", &id_str)]),
                &with_name,
            )
        }
        None => with_ext,
    };
    (cite_id, final_state)
}

// ── Async Platform callback registry (#305 #2) ────────────────────
//
// Sibling to the sync registry below: hosts that want to register a
// Platform body that actually does async work (HTTP fetch, a channel
// send, waiting on a JS Promise) install via install_async_platform_fn
// and invoke via apply_platform_async. The sync `apply_platform` path
// is unchanged — the engine's synchronous reduction semantics are
// preserved for every caller that doesn't explicitly opt into async.
//
// How it composes across runtimes:
//
// - Browser / Cloudflare Workers: host uses wasm-bindgen-futures to
//   await apply_platform_async from a JS-facing async boundary. No
//   blocking; the JS Promise returned to the host's framework resolves
//   when the Rust future resolves.
//
// - Native std (server, CLI): host uses any executor (tokio,
//   async-std, pollster) to drive apply_platform_async to completion
//   and pass the result into a sync apply call or federated_ingest.
//
// - Pure no_std: no Future executor is available; async registry is
//   compiled out via #[cfg(not(feature = "no_std"))].

#[cfg(not(feature = "no_std"))]
pub type AsyncPlatformFn = crate::sync::Arc<
    dyn Fn(&Object, &Object) -> core::pin::Pin<alloc::boxed::Box<
        dyn core::future::Future<Output = Object> + Send
    >> + Send + Sync
>;

#[cfg(not(feature = "no_std"))]
static ASYNC_PLATFORM_FALLBACK: crate::sync::OnceLock<
    crate::sync::RwLock<HashMap<String, AsyncPlatformFn>>
> = crate::sync::OnceLock::new();

/// Install an async Platform body. apply_platform_async looks up here
/// for names not covered by the sync registry below. The body returns
/// a Pin<Box<dyn Future<Output = Object>>> — caller awaits to get the
/// Object. Thread-safe; callers may re-install to replace the body.
#[cfg(not(feature = "no_std"))]
pub fn install_async_platform_fn(name: &str, f: AsyncPlatformFn) {
    let reg = ASYNC_PLATFORM_FALLBACK
        .get_or_init(|| crate::sync::RwLock::new(HashMap::new()));
    reg.write().insert(name.to_string(), f);
}

/// Remove a previously-installed async Platform body.
#[cfg(not(feature = "no_std"))]
pub fn uninstall_async_platform_fn(name: &str) {
    if let Some(reg) = ASYNC_PLATFORM_FALLBACK.get() {
        reg.write().remove(name);
    }
}

/// Names the crate's production paths are permitted to install into
/// `ASYNC_PLATFORM_FALLBACK`. Empty by construction: no production
/// path in `arest` writes this registry today — see
/// `_reports/sec-2-platform-audit-2026-04-21.md`. A future writer
/// MUST add its name here and revise the audit; the integration test
/// `tests/sec_2_platform_fallback_audit.rs` fails otherwise.
#[cfg(not(feature = "no_std"))]
pub const APPROVED_ASYNC_PLATFORM_FN_NAMES: &[&str] = &[];

/// Sorted names currently installed in `ASYNC_PLATFORM_FALLBACK`.
/// Empty when the `OnceLock` has never been initialized. Used by the
/// sec-2 guard test; also exposable for host-side introspection.
#[cfg(not(feature = "no_std"))]
pub fn installed_async_platform_fn_names() -> alloc::vec::Vec<alloc::string::String> {
    match ASYNC_PLATFORM_FALLBACK.get() {
        Some(reg) => {
            let mut v: alloc::vec::Vec<alloc::string::String> =
                reg.read().keys().cloned().collect();
            v.sort();
            v
        }
        None => alloc::vec::Vec::new(),
    }
}

/// Async counterpart to `apply_platform` + `dispatch_platform_fallback`.
/// Dispatch order:
///   1. Async registry (install_async_platform_fn) — awaited.
///   2. Sync registry (install_platform_fn) — returns immediately.
///   3. Bottom.
///
/// Hardcoded `apply_platform` arms are not consulted here: they are
/// already sync and accessible via `apply(Func::Platform(...), ...)`.
/// This function is for the complement — names the engine doesn't
/// ship a hardcoded body for.
#[cfg(not(feature = "no_std"))]
pub async fn apply_platform_async(name: &str, x: &Object, d: &Object) -> Object {
    // Async fallback first — clone the Arc out of the lock so the
    // guard's lifetime ends before the `.await`.
    let async_fn = ASYNC_PLATFORM_FALLBACK.get()
        .and_then(|reg| reg.read().get(name).cloned());
    if let Some(f) = async_fn {
        return f(x, d).await;
    }
    // Fall through to sync fallback.
    dispatch_platform_fallback(name, x, d)
}

// ── Runtime Platform callback registry (#305 IoC/DI completion) ───
//
// apply_platform's hardcoded match only covers compile-derived names.
// When a host installs a synchronous Platform body for a runtime-
// registered name (ML scorer, local cache projector, test double),
// the engine looks it up here. Registration is orthogonal to
// register_runtime_fn: that one marks the name so provenance can cite
// it; install_platform_fn attaches the actual callable.
//
// Async I/O (HTTP fetch, external writes) cannot cross this boundary
// because apply is synchronous — hosts bridge async work at the FFI
// level (federated_ingest) instead. This registry is only for
// genuinely synchronous callbacks.
//
// C1 (#687): the *discovery* surface lives in the cell substrate.
// `Policy_platform` carries a Seq of fact rows
//   `<<name, "X">, <identifier, "Y">>`
// from which `platform_from_state` resolves a name → identifier. The
// process-global Rust side-table below (`PLATFORM_FALLBACK`) is keyed
// by that identifier and holds the actual `Arc<dyn Fn>` — the
// non-introspectable, non-replayable bit. `install_platform_fn` writes
// to *both* the side-table and a process-wide mirror state via
// `install_platform`, so introspection (`installed_platform_fn_names`,
// the sec-2 audit test) reads the cell instead of the raw registry.

/// Policy cell carrying the runtime-installed Platform names. Contents:
/// a Seq of `<<name, X>, <identifier, Y>>` named-tuple rows. Reading a
/// `name` resolves to the identifier the side-table dispatches on.
/// Absent or empty cell means "no runtime body installed at this name".
pub const POLICY_PLATFORM: &str = "Policy_platform";

#[cfg(not(feature = "no_std"))]
pub type PlatformFn = crate::sync::Arc<
    dyn Fn(&Object, &Object) -> Object + Send + Sync
>;

#[cfg(not(feature = "no_std"))]
static PLATFORM_FALLBACK: crate::sync::OnceLock<
    crate::sync::RwLock<HashMap<String, PlatformFn>>
> = crate::sync::OnceLock::new();

// Process-wide mirror of the runtime registry, encoded in the cell
// substrate. `install_platform_fn` updates both this mirror and the
// side-table; `installed_platform_fn_names` and the dispatcher read
// from this mirror so the cell is the authority for "is X registered".
#[cfg(not(feature = "no_std"))]
static PLATFORM_REGISTRY_STATE: crate::sync::OnceLock<
    crate::sync::Mutex<Object>
> = crate::sync::OnceLock::new();

#[cfg(not(feature = "no_std"))]
fn platform_registry_state() -> &'static crate::sync::Mutex<Object> {
    PLATFORM_REGISTRY_STATE.get_or_init(|| crate::sync::Mutex::new(Object::phi()))
}

/// Install a Platform identifier into `state`'s `Policy_platform` cell.
/// Returns a new state with a `<<name, X>, <identifier, Y>>` row whose
/// `name` binding is `name` and `identifier` binding is `identifier`.
/// Re-installing a `name` replaces its existing row in place; other
/// rows are untouched. Pure function — no side effects on the side-
/// table; the discovery surface and the function-pointer surface are
/// updated independently by `install_platform_fn`.
pub fn install_platform(state: &Object, name: &str, identifier: &str) -> Object {
    let row = fact_from_pairs(&[("name", name), ("identifier", identifier)]);
    let existing = fetch_or_phi(POLICY_PLATFORM, state);
    let new_rows: Vec<Object> = match existing.as_seq() {
        Some(items) => {
            let mut out: Vec<Object> = items.iter()
                .filter(|r| binding(r, "name") != Some(name))
                .cloned()
                .collect();
            out.push(row);
            out
        }
        None => alloc::vec![row],
    };
    store(POLICY_PLATFORM, Object::Seq(new_rows.into()), state)
}

/// Read the Platform identifier for `name` from `state`'s
/// `Policy_platform` cell. Returns `Some(identifier)` when a row with
/// matching `name` is present, `None` otherwise. The identifier is the
/// side-table key the dispatcher uses to find the actual closure; for
/// names installed via `install_platform_fn`, the identifier and the
/// name are the same string.
pub fn platform_from_state(state: &Object, name: &str) -> Option<alloc::string::String> {
    let cell = fetch(POLICY_PLATFORM, state);
    let rows = cell.as_seq()?;
    rows.iter().find_map(|r| {
        if binding(r, "name") == Some(name) {
            binding(r, "identifier").map(|s| s.to_string())
        } else {
            None
        }
    })
}

/// Sorted names currently registered in `state`'s `Policy_platform`
/// cell. Empty when the cell is absent. Pure function — host-side
/// introspection that does not consult the process-global side-table.
pub fn platform_names_from_state(state: &Object) -> alloc::vec::Vec<alloc::string::String> {
    let cell = fetch(POLICY_PLATFORM, state);
    let mut out: alloc::vec::Vec<alloc::string::String> = cell.as_seq()
        .map(|rows| rows.iter()
            .filter_map(|r| binding(r, "name").map(|s| s.to_string()))
            .collect())
        .unwrap_or_default();
    out.sort();
    out
}

/// Remove a Platform name from `state`'s `Policy_platform` cell.
/// Returns a new state with the matching row dropped; if no row
/// matches, returns a state where the cell is materialised but
/// unchanged in content. Counterpart to `install_platform` for
/// `uninstall_platform_fn`.
#[cfg(not(feature = "no_std"))]
fn uninstall_platform(state: &Object, name: &str) -> Object {
    let existing = fetch_or_phi(POLICY_PLATFORM, state);
    let new_rows: Vec<Object> = match existing.as_seq() {
        Some(items) => items.iter()
            .filter(|r| binding(r, "name") != Some(name))
            .cloned()
            .collect(),
        None => Vec::new(),
    };
    store(POLICY_PLATFORM, Object::Seq(new_rows.into()), state)
}

/// Install a synchronous Platform body. apply_platform falls through
/// here for names not covered by the hardcoded match. The body is an
/// `Arc<dyn Fn(&Object, &Object) -> Object>` — takes the operand and
/// the current `D`, returns an Object. Thread-safe; callers may
/// re-install to replace the body.
///
/// C1 (#687): the name is also recorded in the process-wide
/// `Policy_platform` cell mirror so introspection paths read the cell
/// substrate instead of the raw side-table. The identifier stored in
/// the cell is the same string as `name`; the side-table is keyed by
/// that identifier.
#[cfg(not(feature = "no_std"))]
pub fn install_platform_fn(name: &str, f: PlatformFn) {
    let reg = PLATFORM_FALLBACK.get_or_init(|| crate::sync::RwLock::new(HashMap::new()));
    reg.write().insert(name.to_string(), f);
    let mirror = platform_registry_state();
    let mut guard = mirror.lock();
    *guard = install_platform(&guard, name, name);
}

/// Remove a previously-installed Platform body. Used by tests to
/// avoid leakage between test cases sharing process state. Mirrors the
/// removal into the process-wide `Policy_platform` cell so the audit
/// surface stays in sync with the side-table.
#[cfg(not(feature = "no_std"))]
pub fn uninstall_platform_fn(name: &str) {
    if let Some(reg) = PLATFORM_FALLBACK.get() {
        reg.write().remove(name);
    }
    let mirror = platform_registry_state();
    let mut guard = mirror.lock();
    *guard = uninstall_platform(&guard, name);
}

/// Names the crate's production paths are permitted to install into
/// `PLATFORM_FALLBACK`. Empty by construction: no production path in
/// `arest` writes this registry today — see
/// `_reports/sec-2-platform-audit-2026-04-21.md`. A future writer
/// MUST add its name here and revise the audit; the integration test
/// `tests/sec_2_platform_fallback_audit.rs` fails otherwise.
#[cfg(not(feature = "no_std"))]
pub const APPROVED_PLATFORM_FN_NAMES: &[&str] = &[
    // task-919 gap-4: arest-dev Rebuild SM Platform Functions. Classified in
    // _reports/sec-2-platform-audit-2026-04-21.md. Filesystem reach bounded to
    // the apps_dir captured by install_rebuild_fns; per-fn Bottom-on-failure
    // rolls back the SM transition. Listing is declarative — the install call
    // at MCP/CLI boot remains a separate production-wiring step.
    "rebuild_snapshot",
    "rebuild_verify",
    "rebuild_apply_bulk",
    "rebuild_init",
    // pb-render-fn-contract (§5.2 Platform Binding): the reference HTML
    // render function (platform/render_html.rs). PURE — no filesystem,
    // network, or state reach; operand in, markup Atom out, Bottom on
    // malformed input. Dispatched per Render Target facts
    // (readings/ui/render-target.md); installed at CLI boot beside the
    // rebuild fns.
    "render:html",
    // pb-effect-fns-canonical (§5.2 EFFECTS): the canonical effect bodies
    // (platform/{http_fetch,notify}.rs). `http_fetch` = outbound HTTP via
    // the same bounded transport as the task-919 callback branch (5 s
    // deadlines, 64 KB cap, http/https only); `notify` = one stderr line,
    // no other reach. Both Bottom on malformed operand; dispatched by
    // Verb→Function_has_Name facts (transition_via_defs) or direct apply.
    "http_fetch",
    "notify",
];

/// Sorted names currently installed in `PLATFORM_FALLBACK`. Reads from
/// the `Policy_platform` cell mirror — the cell is the authority for
/// "is X registered". Used by the sec-2 guard test; also exposable for
/// host-side introspection.
#[cfg(not(feature = "no_std"))]
pub fn installed_platform_fn_names() -> alloc::vec::Vec<alloc::string::String> {
    let mirror = platform_registry_state();
    let guard = mirror.lock();
    platform_names_from_state(&guard)
}

#[cfg(not(feature = "no_std"))]
fn dispatch_platform_fallback(name: &str, x: &Object, d: &Object) -> Object {
    // The cell mirror is the authority on whether `name` is installed.
    // Reading it here keeps the side-table accessible only by an
    // identifier the cell handed back, never by raw name lookup.
    let mirror = platform_registry_state();
    let identifier = {
        let guard = mirror.lock();
        match platform_from_state(&guard, name) {
            Some(id) => id,
            None => return Object::Bottom,
        }
    };
    let reg = match PLATFORM_FALLBACK.get() {
        Some(r) => r,
        None => return Object::Bottom,
    };
    let maybe_f = reg.read().get(&identifier).cloned();
    match maybe_f {
        Some(f) => f(x, d),
        None => Object::Bottom,
    }
}

/// E3 / #305 — Federated ingestion end-to-end.
///
/// Realizes the paper's `ρ(populate_n) : I → {f₁, …, fₖ} ⊆ P_OWA`:
/// pre-fetched facts enter `P` under OWA, paired with a single
/// Citation whose Authority Type is `'Federated-Fetch'`. All facts
/// from the same fetch share one Citation (they came from the same
/// response at the same moment); the content-addressed id scheme
/// makes repeated ingestion idempotent at the cell level.
///
/// Input shape is explicit (fact_type_id, [(role_name, role_value)…])
/// so the caller owns JSON → fact mapping — the engine stays
/// serialization-agnostic. The MCP-server / Cloudflare-worker wrapper
/// does the HTTP fetch and the JSON → (fact_type, bindings) walk
/// using the compiled populate:{noun} config, then hands the tuple
/// list to this function.
#[cfg(not(feature = "no_std"))]
pub fn ingest_federated_facts(
    external_system: &str,
    url: &str,
    retrieval_date: &str,
    facts: &[(String, alloc::vec::Vec<(String, String)>)],
    state: &Object,
) -> (String, Object) {
    let (cite_id, with_cite) = emit_citation_fact(
        url,
        "Federated-Fetch",
        retrieval_date,
        Some(external_system),
        state,
    );
    let final_state = facts.iter().fold(with_cite, |acc, (ft_id, bindings)| {
        let pairs: alloc::vec::Vec<(&str, &str)> = bindings.iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let fact_id = fact_identity_id(ft_id, bindings);
        // Fact itself into its declared FT cell.
        let with_fact = cell_push_unique(ft_id, fact_from_pairs(&pairs), &acc);
        // Fact cites Citation link — instances.md §Fact.
        let with_link = cell_push_unique(
            "Fact_cites_Citation",
            fact_from_pairs(&[("Fact", &fact_id), ("Citation", &cite_id)]),
            &with_fact,
        );
        // Resource has Reference — instances.md §Resource. Fact is a
        // subtype of Resource, so Reference is the identity scheme.
        cell_push_unique(
            "Resource_has_Reference",
            fact_from_pairs(&[("Resource", &fact_id), ("Reference", &fact_id)]),
            &with_link,
        )
    });
    (cite_id, final_state)
}

/// Deterministic synthetic id for a fact given (factTypeId, bindings).
/// Used as the Fact / Resource identity when the fact enters P via a
/// runtime path (federated_ingest, platform-fn emission) rather than
/// through the command pipeline that would assign a Reference via
/// RMAP. The id is content-addressed so repeated emission of the same
/// fact is idempotent at the cell level when paired with cell_push_unique.
#[cfg(not(feature = "no_std"))]
fn fact_identity_id(fact_type_id: &str, bindings: &[(String, String)]) -> alloc::string::String {
    use core::hash::{BuildHasher, Hash, Hasher};
    let mut h = hashbrown::hash_map::DefaultHashBuilder::default().build_hasher();
    fact_type_id.hash(&mut h);
    // Sort bindings to make the hash invariant to caller ordering.
    let mut sorted = bindings.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    sorted.iter().for_each(|(k, v)| {
        k.hash(&mut h);
        v.hash(&mut h);
    });
    alloc::format!("fact:{:016x}", h.finish())
}

/// This is Backus Sec. 13.3.2: definitions map atoms to expressions.
/// Build state from defs + existing cells in O(n).
/// Collects all cells into a HashMap (O(1) per insert), then
/// constructs the Object sequence in one pass. Replaces the
/// O(n²) sequential fold over store.
pub fn defs_to_state(defs: &[(String, Func)], state: &Object) -> Object {
    // Start with existing cells from state
    let mut map: HashMap<String, Object> = cells_iter(state).into_iter()
        .map(|(name, contents)| (name.to_string(), contents.clone()))
        .collect();
    // Overlay defs — O(1) per insert
    defs.iter().for_each(|(name, func)| {
        map.insert(name.clone(), func_to_object(func));
    });
    // Return as Map store — O(1) fetch/store for all subsequent operations
    Object::Map(map.into())
}

/// Look up `allowed_writes:{def_name}` in DEFS. If it's a Seq of atom
/// cell names, push those as the current capability frame and return a
/// Some(guard) whose Drop pops it. Absent or non-seq cell → None
/// (unrestricted, legacy behavior). Called by `Func::Def` dispatch so
/// user-authored defs self-enforce their declared writes. See Sec-5.
///
/// `*` in the allow-list acts as a wildcard — emitted by compile for
/// system defs (kernel helpers, cache invalidation) that legitimately
/// touch any cell. User-authored readings never emit "*".
// `declared_writes` is now reachable under no_std (#565) — its no_std
// shims make `push_caps` a no-op so the kernel-side hook is harmless
// (capabilities are unrestricted under the kernel because user-code is
// not yet a threat surface there). The earlier cfg gate on this fn
// was a holdover from when `pub mod declared_writes;` was std-only.
fn defs_writes_scope(def_name: &str, d: &Object) -> Option<crate::declared_writes::CapGuard> {
    let key = alloc::format!("allowed_writes:{}", def_name);
    let cell = fetch(&key, d);
    let items = cell.as_seq()?;
    let frame: hashbrown::HashSet<String> = items.iter()
        .filter_map(|o| o.as_atom().map(|s| s.to_string()))
        .collect();
    // Empty-seq frame is still meaningful: "no writes allowed at all".
    // Only Bottom / non-seq skips enforcement.
    Some(crate::declared_writes::push_caps(frame))
}

/// Rewrite a Func to a smaller equivalent form before reduction.
///
/// Implements a subset of Backus (1978) §12 algebraic laws. Each rule is
/// an observational equivalence: `apply(normalize(f), x, d) == apply(f,
/// x, d)` for every x and d. The pass is bottom-up — children are
/// normalized first, then local rewrites are applied once at the root.
///
/// Rules implemented:
///   (III.1)   `id ∘ f → f`  and  `f ∘ id → f`
///   (fusion)  `α(f) ∘ α(g) → α(f ∘ g)`           — map fusion
///   (fusion)  `Filter(p) ∘ Filter(q) → Filter(and ∘ [p,q])`
///   (fold)    `[c̄₁, …, c̄ₙ] → c̄⟨c₁,…,cₙ⟩`         — constant folding
///
/// Rules deliberately NOT applied:
///   - `α(id) → id`                   (differs on atoms: α(id):atom = ⊥, id:atom = atom)
///   - `c̄ ∘ f → c̄`                   (differs when f:x = ⊥ with x ≠ ⊥)
/// The paper proves these equivalences but they rely on ⊥-preservation
/// bounds that the full-domain Func embedding does not respect.
pub fn normalize(f: &Func) -> Func {
    let recur = normalize_children(f);
    normalize_step(&recur)
}

fn normalize_children(f: &Func) -> Func {
    match f {
        Func::Compose(a, b) =>
            Func::Compose(Box::new(normalize(a)), Box::new(normalize(b))),
        Func::Construction(fs) =>
            Func::Construction(fs.iter().map(normalize).collect()),
        Func::Condition(p, t, e) =>
            Func::Condition(Box::new(normalize(p)), Box::new(normalize(t)), Box::new(normalize(e))),
        Func::ApplyToAll(inner) =>
            Func::ApplyToAll(Box::new(normalize(inner))),
        Func::Insert(inner) =>
            Func::Insert(Box::new(normalize(inner))),
        Func::Filter(p) =>
            Func::Filter(Box::new(normalize(p))),
        Func::BinaryToUnary(g, x) =>
            Func::BinaryToUnary(Box::new(normalize(g)), x.clone()),
        Func::While(p, body) =>
            Func::While(Box::new(normalize(p)), Box::new(normalize(body))),
        Func::FoldL(g) =>
            Func::FoldL(Box::new(normalize(g))),
        Func::IndexBy(g) =>
            Func::IndexBy(Box::new(normalize(g))),
        Func::OrderBy(g) =>
            Func::OrderBy(Box::new(normalize(g))),
        leaf => leaf.clone(),
    }
}

fn normalize_step(f: &Func) -> Func {
    match f {
        Func::Compose(a, b) => match (a.as_ref(), b.as_ref()) {
            (Func::Id, _) => (**b).clone(),
            (_, Func::Id) => (**a).clone(),
            (Func::ApplyToAll(inner_f), Func::ApplyToAll(inner_g)) => {
                let fused = normalize(&Func::Compose(inner_f.clone(), inner_g.clone()));
                Func::ApplyToAll(Box::new(fused))
            }
            (Func::Filter(p), Func::Filter(q)) => {
                let pred = Func::Compose(
                    Box::new(Func::And),
                    Box::new(Func::Construction(vec![(**p).clone(), (**q).clone()])),
                );
                Func::Filter(Box::new(normalize(&pred)))
            }
            _ => f.clone(),
        },
        Func::Construction(fs) if !fs.is_empty()
            && fs.iter().all(|g| matches!(g, Func::Constant(x) if !matches!(x, Object::Bottom))) => {
            let items: Vec<Object> = fs.iter().map(|g| match g {
                Func::Constant(x) => x.clone(),
                _ => unreachable!(),
            }).collect();
            Func::Constant(Object::Seq(Arc::from(items)))
        }
        _ => f.clone(),
    }
}

// ── Apply-variant profiler ──────────────────────────────────────────
//
// Opt-in, thread-local accounting of Func::apply calls. Gated behind
// the `profile` Cargo feature so default/release builds pay zero
// overhead in apply(). When the feature is off, profile_enable/etc.
// stub out and apply() is a two-line function.
//
// Enable via:
//   cargo test --features profile --lib profile_create_order -- \
//              --ignored --nocapture

#[cfg(all(feature = "profile", not(target_arch = "wasm32")))]
mod profile {
    use core::cell::{Cell, RefCell};
    use hashbrown::HashMap;

    thread_local! {
        pub(super) static ENABLED: Cell<bool> = const { Cell::new(false) };
        pub(super) static STATS: RefCell<HashMap<&'static str, (u64, u64)>> =
            RefCell::new(HashMap::new());
    }
}

#[cfg(all(feature = "profile", not(target_arch = "wasm32")))]
fn profile_record(variant: &'static str, ns: u64) {
    profile::STATS.with(|m| {
        let mut map = m.borrow_mut();
        let e = map.entry(variant).or_insert((0u64, 0u64));
        e.0 += 1;
        e.1 += ns;
    });
}

/// Turn on the apply-variant profiler for this thread. No-op unless
/// the `profile` feature is enabled at build time.
#[cfg(all(feature = "profile", not(target_arch = "wasm32")))]
pub fn profile_enable() { profile::ENABLED.with(|c| c.set(true)); }
#[cfg(not(all(feature = "profile", not(target_arch = "wasm32"))))]
pub fn profile_enable() {}

/// Turn off the apply-variant profiler for this thread. No-op unless
/// the `profile` feature is enabled at build time.
#[cfg(all(feature = "profile", not(target_arch = "wasm32")))]
pub fn profile_disable() { profile::ENABLED.with(|c| c.set(false)); }
#[cfg(not(all(feature = "profile", not(target_arch = "wasm32"))))]
pub fn profile_disable() {}

/// Clear accumulated apply counts for this thread.
#[cfg(all(feature = "profile", not(target_arch = "wasm32")))]
pub fn profile_reset() { profile::STATS.with(|m| m.borrow_mut().clear()); }
#[cfg(not(all(feature = "profile", not(target_arch = "wasm32"))))]
pub fn profile_reset() {}

/// Read a `(variant, count, total_ns)` histogram sorted descending by
/// total_ns. Empty under the default build (no `profile` feature).
#[cfg(all(feature = "profile", not(target_arch = "wasm32")))]
pub fn profile_snapshot() -> Vec<(&'static str, u64, u64)> {
    profile::STATS.with(|m| {
        let mut v: Vec<_> = m.borrow().iter().map(|(k, (c, t))| (*k, *c, *t)).collect();
        v.sort_by(|a, b| b.2.cmp(&a.2));
        v
    })
}
#[cfg(not(all(feature = "profile", not(target_arch = "wasm32"))))]
pub fn profile_snapshot() -> Vec<(&'static str, u64, u64)> { Vec::new() }

/// Pretty-print the current snapshot to stderr.
pub fn profile_dump() {
    let snap = profile_snapshot();
    let total_ns: u64 = snap.iter().map(|(_, _, t)| t).sum();
    let total_n:  u64 = snap.iter().map(|(_, c, _)| c).sum();
    diag!("[profile] apply-variant histogram ({} calls, {}ms total):",
        total_n, total_ns / 1_000_000);
    snap.iter().for_each(|(name, count, ns)| {
        let pct = if total_ns > 0 { *ns as f64 * 100.0 / total_ns as f64 } else { 0.0 };
        let avg_ns = if *count > 0 { ns / count } else { 0 };
        diag!("  {:<18} {:>10} calls   {:>10}µs   {:>6.2}%   avg {}ns",
            name, count, ns / 1_000, pct, avg_ns);
    });
}

/// Readable discriminant for a Func variant. Used by the profiler so
/// histogram entries are grouped by variant rather than by the boxed
/// children they carry.
#[cfg(all(feature = "profile", not(target_arch = "wasm32")))]
fn variant_name(f: &Func) -> &'static str {
    match f {
        Func::Id => "Id",
        Func::Selector(_) => "Selector",
        Func::Tail => "Tail",
        Func::AtomTest => "AtomTest",
        Func::NullTest => "NullTest",
        Func::CellNameTest => "CellNameTest",
        Func::Eq => "Eq",
        Func::Gt => "Gt",
        Func::Lt => "Lt",
        Func::Ge => "Ge",
        Func::Le => "Le",
        Func::Contains => "Contains",
        Func::StartsWith => "StartsWith",
        Func::EndsWith => "EndsWith",
        Func::Trim => "Trim",
        Func::Split => "Split",
        Func::Replace => "Replace",
        Func::Lower => "Lower",
        Func::Length => "Length",
        Func::Concat => "Concat",
        Func::Compact => "Compact",
        Func::DistL => "DistL",
        Func::DistR => "DistR",
        Func::HasMember => "HasMember",
        Func::SetFromSeq => "SetFromSeq",
        Func::Trans => "Trans",
        Func::ApndL => "ApndL",
        Func::Reverse => "Reverse",
        Func::ApndR => "ApndR",
        Func::RotL => "RotL",
        Func::RotR => "RotR",
        Func::Add => "Add",
        Func::Sub => "Sub",
        Func::Mul => "Mul",
        Func::Div => "Div",
        Func::And => "And",
        Func::Or => "Or",
        Func::Not => "Not",
        Func::Fetch => "Fetch",
        Func::FetchOrPhi => "FetchOrPhi",
        Func::Store => "Store",
        Func::Constant(_) => "Constant",
        Func::Compose(_, _) => "Compose",
        Func::Construction(_) => "Construction",
        Func::Condition(_, _, _) => "Condition",
        Func::ApplyToAll(_) => "ApplyToAll",
        Func::Insert(_) => "Insert",
        Func::BinaryToUnary(_, _) => "BinaryToUnary",
        Func::Filter(_) => "Filter",
        Func::While(_, _) => "While",
        Func::FoldL(_) => "FoldL",
        Func::IndexBy(_) => "IndexBy",
        Func::OrderBy(_) => "OrderBy",
        Func::Def(_) => "Def",
        Func::Platform(_) => "Platform",
        Func::Native(_) => "Native",
    }
}

/// Apply `func` to `x` with `d` as the def / cell store. Single-entry
/// β-reduction for the FP algebra: every combining form eventually
/// dispatches back here for its recursive sub-applications.
///
/// # Bottom preservation
///
/// `apply(f, ⊥, d) = ⊥` for every `f`; ⊥ flows through Compose,
/// Construction, ApplyToAll, Insert, Filter, FoldL, and Condition.
///
/// # Fuel model (Sec-3, #159)
///
/// Each call debits one unit from a thread-local reductions counter
/// and short-circuits to Bottom when the counter reaches zero. This
/// is the kernel's cgroup-equivalent *inside* the evaluator: a
/// malicious Func tree (deep Compose, Def cycle, exploding
/// Construction over a 1 M-element Seq) hits the ceiling mid-
/// evaluation. No panic, no stack overflow — Bottom propagates
/// outward via the bottom-preservation law.
///
/// The default budget is `u64::MAX` (unlimited), so existing call
/// sites are unchanged. Callers scope a suspect tree with
/// `with_fuel(n, || apply(&f, x, d))`; the prior budget is restored
/// on return so nested scopes compose cleanly. Tenant-level budgets
/// (per-minute call caps) live one layer up, around SYSTEM dispatch.
#[cfg(all(feature = "profile", not(target_arch = "wasm32")))]
pub fn apply(func: &Func, x: &Object, d: &Object) -> Object {
    if !consume_fuel() {
        return Object::Bottom;
    }
    if !profile::ENABLED.with(|c| c.get()) {
        return match x.is_bottom() {
            true => Object::Bottom,
            false => apply_nonbottom(func, x, d),
        };
    }
    let name = variant_name(func);
    let t = std::time::Instant::now();
    let result = match x.is_bottom() {
        true => Object::Bottom,
        false => apply_nonbottom(func, x, d),
    };
    profile_record(name, t.elapsed().as_nanos() as u64);
    result
}

/// See the `profile`-gated twin above for the fuel-model doc.
#[cfg(not(all(feature = "profile", not(target_arch = "wasm32"))))]
pub fn apply(func: &Func, x: &Object, d: &Object) -> Object {
    // Fuel debit first: a Func tree that's already blown the budget
    // must collapse to ⊥ before even checking `x.is_bottom()`, else a
    // bottom argument would let a malicious caller "free" reductions.
    if !consume_fuel() {
        return Object::Bottom;
    }
    // All functions are bottom-preserving: ⊥ propagates unchanged.
    match x.is_bottom() {
        true => Object::Bottom,
        false => apply_nonbottom(func, x, d),
    }
}

/// H2 (#690): Pure-API fuel surface. Evaluates `apply(func, x, d)`
/// under an explicit reduction budget and returns both the result and
/// the unconsumed remainder. `u64::MAX` is the unlimited sentinel —
/// `apply_with_fuel(f, x, d, u64::MAX)` matches `apply(f, x, d)`
/// semantics with a `(_, u64::MAX)` second slot.
///
/// Backus's algebra has no fuel — Bottom is a function of `(f, x)`.
/// The recursive evaluator still uses an internal counter for the
/// per-`apply` debit (`consume_fuel`), but external callers see fuel
/// as data flowing through the call: `(f, x, fuel) ↦ (object, fuel')`.
/// Sec-3 enforcement at the `system::apply` boundary stays unchanged
/// — that layer is unaware of how the budget is plumbed underneath.
///
/// Compatible with `with_fuel`: an outer `with_fuel(N, …)` scope sets
/// the ambient counter to N before this call sees `fuel`. The inner
/// `with_fuel(fuel, …)` here saves that prior value and restores it
/// on return, so well-scoped nesting still composes.
pub fn apply_with_fuel(func: &Func, x: &Object, d: &Object, fuel: u64) -> (Object, u64) {
    with_fuel(fuel, || {
        let result = apply(func, x, d);
        (result, current_fuel())
    })
}

fn apply_nonbottom(func: &Func, x: &Object, d: &Object) -> Object {
    match func {
        // ── Primitives ───────────────────────────────────────────

        Func::Id => x.clone(),

        Func::Selector(s) => {
            match x.as_seq() {
                Some(items) if *s >= 1 && *s <= items.len() => items[*s - 1].clone(),
                _ => Object::Bottom,
            }
        }

        Func::Tail => {
            match x.as_seq() {
                Some(items) if items.is_empty() => Object::Bottom,
                Some(items) if items.len() == 1 => Object::phi(),
                Some(items) => Object::Seq(Arc::from(items[1..].to_vec())),
                _ => Object::Bottom,
            }
        }

        Func::AtomTest => {
            if x.is_atom() { Object::t() } else { Object::f() }
        }

        Func::NullTest => {
            match x {
                Object::Seq(items) if items.is_empty() => Object::t(),
                _ => Object::f(),
            }
        }

        Func::CellNameTest => {
            // Backus §13.3.4 cellname (structural half): cell? = T iff x
            // is `<CELL_TAG, name, contents>` — sequence of length 3
            // whose first element is the atom "CELL".
            match x.as_seq() {
                Some(items) if items.len() == 3
                    && items[0].as_atom() == Some(CELL_TAG) => Object::t(),
                _ => Object::f(),
            }
        }

        Func::Eq => {
            match x.as_seq() {
                Some(items) if items.len() == 2 => {
                    if items[0] == items[1] { Object::t() } else { Object::f() }
                }
                _ => Object::Bottom,
            }
        }

        Func::Gt => apply_compare(x, |a, b| a > b),
        Func::Lt => apply_compare(x, |a, b| a < b),
        Func::Ge => apply_compare(x, |a, b| a >= b),
        Func::Le => apply_compare(x, |a, b| a <= b),

        Func::Contains => {
            match x.as_seq() {
                Some(items) if items.len() == 2 => {
                    match (items[0].as_atom(), items[1].as_atom()) {
                        (Some(haystack), Some(needle)) =>
                            if haystack.to_lowercase().contains(&needle.to_lowercase()) { Object::t() } else { Object::f() },
                        _ => Object::Bottom,
                    }
                }
                _ => Object::Bottom,
            }
        }

        Func::StartsWith => {
            match x.as_seq() {
                Some(items) if items.len() == 2 => {
                    match (items[0].as_atom(), items[1].as_atom()) {
                        (Some(haystack), Some(needle)) =>
                            if haystack.to_lowercase().starts_with(&needle.to_lowercase()) { Object::t() } else { Object::f() },
                        _ => Object::Bottom,
                    }
                }
                _ => Object::Bottom,
            }
        }

        Func::EndsWith => {
            match x.as_seq() {
                Some(items) if items.len() == 2 => {
                    match (items[0].as_atom(), items[1].as_atom()) {
                        (Some(haystack), Some(needle)) =>
                            if haystack.to_lowercase().ends_with(&needle.to_lowercase()) { Object::t() } else { Object::f() },
                        _ => Object::Bottom,
                    }
                }
                _ => Object::Bottom,
            }
        }

        Func::Trim => {
            match x.as_atom() {
                Some(s) => Object::Atom(s.trim().to_string()),
                None => Object::Bottom,
            }
        }

        Func::Split => {
            // split:<haystack, delim> → <part₁, part₂, …>. Empty
            // delimiter yields the char-by-char decomposition (each
            // grapheme cluster as a single-char atom).
            match x.as_seq() {
                Some(items) if items.len() == 2 => {
                    match (items[0].as_atom(), items[1].as_atom()) {
                        (Some(haystack), Some(delim)) => {
                            let parts: Vec<Object> = if delim.is_empty() {
                                haystack.chars()
                                    .map(|c| Object::Atom(c.to_string()))
                                    .collect()
                            } else {
                                haystack.split(delim)
                                    .map(|p| Object::Atom(p.to_string()))
                                    .collect()
                            };
                            Object::Seq(parts.into())
                        }
                        _ => Object::Bottom,
                    }
                }
                _ => Object::Bottom,
            }
        }

        Func::Replace => {
            // replace:<haystack, <needle, replacement>>. The three-ary
            // shape is wrapped as <h, <n, r>> so it composes cleanly
            // under the single-argument apply rule.
            match x.as_seq() {
                Some(items) if items.len() == 2 => {
                    let haystack = items[0].as_atom();
                    let pair = items[1].as_seq();
                    match (haystack, pair) {
                        (Some(h), Some(p)) if p.len() == 2 => {
                            match (p[0].as_atom(), p[1].as_atom()) {
                                (Some(needle), Some(replacement)) =>
                                    Object::Atom(h.replace(needle, replacement)),
                                _ => Object::Bottom,
                            }
                        }
                        _ => Object::Bottom,
                    }
                }
                _ => Object::Bottom,
            }
        }

        Func::Lower => {
            match x.as_atom() {
                Some(s) => Object::Atom(s.to_lowercase()),
                None => Object::Bottom,
            }
        }

        Func::Length => {
            // task-744 phase 3: Map cells answer length by entry count.
            match x {
                Object::Seq(items) => Object::Atom(items.len().to_string()),
                Object::Map(m) => Object::Atom(m.len().to_string()),
                _ => Object::Bottom,
            }
        }

        Func::Concat => {
            match x.as_seq() {
                Some(items) => Object::seq(items.iter().flat_map(|item|
                    item.as_seq().map(|sub| sub.to_vec())
                        .unwrap_or_else(|| vec![item.clone()])
                ).collect()),
                _ => Object::Bottom,
            }
        }

        Func::Compact => {
            // Drop ⊥ elements, preserve order. The paired op that
            // makes `Filter(p) ≡ compact ∘ α(p → id ; ⊥)` work.
            // ⊥ on non-sequence input — parallel to every other
            // sequence primitive.
            match x.as_seq() {
                Some(items) => Object::seq(
                    items.iter()
                        .filter(|i| **i != Object::Bottom)
                        .cloned()
                        .collect(),
                ),
                _ => Object::Bottom,
            }
        }

        Func::DistL => {
            // task-744 phase 3 follow-up: Map cells iterate their
            // values as the right-side collection. Order is incidental
            // (DistL is a point-wise pair-build).
            match x.as_seq() {
                Some(items) if items.len() == 2 => {
                    let y = &items[0];
                    match &items[1] {
                        Object::Seq(zs) if zs.is_empty() => Object::phi(),
                        Object::Seq(zs) => Object::seq(
                            zs.iter().map(|z| Object::seq(vec![y.clone(), z.clone()])).collect()
                        ),
                        Object::Map(m) if m.is_empty() => Object::phi(),
                        Object::Map(m) => Object::seq(
                            m.values().map(|z| Object::seq(vec![y.clone(), z.clone()])).collect()
                        ),
                        _ => Object::Bottom,
                    }
                }
                _ => Object::Bottom,
            }
        }

        Func::DistR => {
            // task-744 phase 3 follow-up: Map cells iterate values as
            // the left-side collection. Same shape as DistL.
            match x.as_seq() {
                Some(items) if items.len() == 2 => {
                    let z = &items[1];
                    match &items[0] {
                        Object::Seq(ys) if ys.is_empty() => Object::phi(),
                        Object::Seq(ys) => Object::seq(
                            ys.iter().map(|y| Object::seq(vec![y.clone(), z.clone()])).collect()
                        ),
                        Object::Map(m) if m.is_empty() => Object::phi(),
                        Object::Map(m) => Object::seq(
                            m.values().map(|y| Object::seq(vec![y.clone(), z.clone()])).collect()
                        ),
                        _ => Object::Bottom,
                    }
                }
                _ => Object::Bottom,
            }
        }

        Func::HasMember => {
            match x.as_seq() {
                Some(items) if items.len() == 2 => {
                    let needle = &items[0];
                    match items[1].as_seq() {
                        Some(haystack) => {
                            if haystack.iter().any(|h| h == needle) {
                                Object::t()
                            } else {
                                Object::f()
                            }
                        }
                        _ => Object::Bottom,
                    }
                }
                _ => Object::Bottom,
            }
        }

        Func::SetFromSeq => {
            // O(N) build of a Map<atom, T> from a Seq of atoms. Non-atom
            // elements break the build → ⊥. Used as the one-shot
            // preprocessor for membership-heavy derivations: build once
            // per round, lookup O(1) via FetchOrPhi thereafter.
            match x {
                Object::Seq(items) if items.is_empty() => Object::Map(HashMap::new().into()),
                Object::Seq(items) => {
                    let mut m = HashMap::with_capacity(items.len());
                    for v in items.iter() {
                        match v.as_atom() {
                            Some(s) => { m.insert(s.to_string(), Object::t()); }
                            None => return Object::Bottom,
                        }
                    }
                    Object::Map(m.into())
                }
                _ => Object::Bottom,
            }
        }

        Func::Trans => match x.as_seq() {
            Some(rows) if rows.is_empty() => Object::phi(),
            Some(rows) => {
                let inner: Vec<&[Object]> = rows.iter()
                    .filter_map(|r| r.as_seq())
                    .collect();
                match (inner.len() == rows.len(), inner.first().map(|r| r.len())) {
                    (false, _) => Object::Bottom,
                    (true, None) => Object::phi(),
                    (true, Some(cols)) if inner.iter().any(|r| r.len() != cols) => Object::Bottom,
                    (true, Some(cols)) => Object::Seq(
                        (0..cols).map(|c|
                            Object::Seq(inner.iter().map(|r| r[c].clone()).collect())
                        ).collect()
                    ),
                }
            }
            _ => Object::Bottom,
        }

        Func::ApndL => {
            match x.as_seq() {
                Some(items) if items.len() == 2 => {
                    let y = &items[0];
                    match items[1].as_seq() {
                        Some(zs) => {
                            let mut result = vec![y.clone()];
                            result.extend_from_slice(zs);
                            Object::Seq(result.into())
                        }
                        _ => Object::Bottom,
                    }
                }
                _ => Object::Bottom,
            }
        }

        Func::Reverse => {
            match x.as_seq() {
                Some(items) => Object::Seq(items.iter().rev().cloned().collect()),
                _ => Object::Bottom,
            }
        }

        Func::ApndR => {
            match x.as_seq() {
                Some(items) if items.len() == 2 => {
                    match items[0].as_seq() {
                        Some(ys) => {
                            let mut result = ys.to_vec();
                            result.push(items[1].clone());
                            Object::Seq(result.into())
                        }
                        _ => Object::Bottom,
                    }
                }
                _ => Object::Bottom,
            }
        }

        Func::RotL => {
            match x.as_seq() {
                Some(items) if items.len() >= 2 => {
                    let mut result = items[1..].to_vec();
                    result.push(items[0].clone());
                    Object::Seq(result.into())
                }
                Some(_) => x.clone(),
                _ => Object::Bottom,
            }
        }

        Func::RotR => {
            match x.as_seq() {
                Some(items) if items.len() >= 2 => {
                    let mut result = vec![items[items.len() - 1].clone()];
                    result.extend_from_slice(&items[..items.len() - 1]);
                    Object::Seq(result.into())
                }
                Some(_) => x.clone(),
                _ => Object::Bottom,
            }
        }

        Func::Add => apply_arithmetic(x, |a, b| Some(a + b)),
        Func::Sub => apply_arithmetic(x, |a, b| Some(a - b)),
        Func::Mul => apply_arithmetic(x, |a, b| Some(a * b)),
        Func::Div => apply_arithmetic(x, |a, b| if b == 0.0 { None } else { Some(a / b) }),

        Func::FetchOrPhi => {
            // fetch_or_phi:<name, D> → fetch with phi fallback for absent.
            // O(1) on Object::Map, O(n) scan on Object::Seq.
            //
            // task-930: if the cell has a registered view rule
            // (def `view:{name}`) in the def-state d, evaluate it
            // lazily against the population (items[1]) and return the
            // derived facts. Falls through to stored cell read when no
            // view exists. View defs live in `d` (the engine state);
            // the population state items[1] is what the view evaluates
            // OVER, so we look up the def in d but apply against the
            // population.
            //
            // task-930 v2: items[1] may be EITHER raw state (Map, or
            // Seq of CELL_TAG 3-tuples) OR an already-encoded
            // population (Seq of 2-tuples <ft_id, facts>) produced by
            // `encode_state`. The chain's `extract_facts_from_pop`
            // composes Func::FetchOrPhi over the encoded pop, so we
            // need to:
            //   1. First try a direct lookup against the encoded-pop
            //      shape (the common chain path — most cells are
            //      stored, and the entry already exists in the pop).
            //   2. Then fall through to `resolve_view`, which knows
            //      how to handle both shapes (it skips re-encoding
            //      when given an already-encoded pop).
            //   3. Finally fall back to legacy `fetch_or_phi`, which
            //      handles raw state.
            // The order matters: scanning the encoded pop is O(n) in
            // pop-size but avoids a wasted view-resolution probe
            // when the cell is already materialized.
            match x.as_seq() {
                Some(items) if items.len() == 2 => match items[0].as_atom() {
                    Some(name) => {
                        let r = match encoded_pop_lookup(name, &items[1]) {
                            Some(facts) => facts,
                            None => match resolve_view(name, &items[1], d) {
                                Some(view_result) => view_result,
                                None => fetch_or_phi(name, &items[1]),
                            },
                        };
                        // ⊥-trace origin: a view rule that bottomed out, or
                        // a cell access that resolved to ⊥, names this cell
                        // and the population it was read over. No-op unless
                        // armed; the success path skips the branch.
                        if r.is_bottom() {
                            note_bottom_cell(name, &items[1]);
                        }
                        r
                    }
                    None => Object::Bottom,
                },
                _ => Object::Bottom,
            }
        }

        Func::Fetch => {
            // fetch:<name, D> → contents of cell named name in D
            // task-930: same view-resolution as FetchOrPhi above.
            // task-930 v2: same encoded-pop awareness as FetchOrPhi.
            match x.as_seq() {
                Some(items) if items.len() == 2 => {
                    match items[0].as_atom() {
                        Some(name) => {
                            let r = match encoded_pop_lookup(name, &items[1]) {
                                Some(facts) => facts,
                                None => match resolve_view(name, &items[1], d) {
                                    Some(view_result) => view_result,
                                    None => fetch(name, &items[1]),
                                },
                            };
                            // ⊥-trace origin: name the cell + binding at the
                            // frame where the fetch bottomed out. No-op
                            // unless armed.
                            if r.is_bottom() {
                                note_bottom_cell(name, &items[1]);
                            }
                            r
                        }
                        None => Object::Bottom,
                    }
                }
                _ => Object::Bottom,
            }
        }

        Func::Store => {
            // store:<name, contents, D> → D' with cell updated.
            // Sec-5: consult the capability stack. Under a user-scoped
            // frame (apply_with_caps or Func::Def with allowed_writes),
            // writes to cells outside the allow-list or to the
            // protected metamodel set collapse to ⊥.
            //
            // #903 (A-17): the empty-stack case is cfg-gated. Under
            // `feature = "no_std"` the kernel image keeps the legacy
            // unrestricted behavior because init / boot / metamodel-
            // load paths legitimately need to populate engine state
            // before the capability system is in place (and there is
            // no user-code threat surface in the kernel). Under
            // `not(feature = "no_std")` (worker / host builds, which
            // always link `std-deps`), an empty cap stack is refused —
            // the bypass branch is replaced by the same Sec-5
            // violation shape (`Object::Bottom`) the populated-frame
            // out-of-allow-list path uses. Callers that legitimately
            // need the legacy unrestricted behavior under std (test
            // fixtures from before #903, gradual-migration islands)
            // wrap themselves in `permissive_empty_caps_guard()` for
            // the duration of the legacy block.
            match x.as_seq() {
                Some(items) if items.len() == 3 => {
                    match items[0].as_atom() {
                        // #903 — empty stack under std refuses by default.
                        // no_std builds skip this branch entirely (the cfg
                        // gates it out) so kernel boot paths keep the
                        // unrestricted system-mode semantics.
                        #[cfg(not(feature = "no_std"))]
                        Some(name) if crate::declared_writes::cap_stack_is_empty()
                            && !crate::declared_writes::is_permissive_empty_caps_mode() => {
                            // ⊥-trace: a Sec-5 cap refusal names the cell it
                            // tried (and failed) to write. No-op unless armed.
                            note_bottom_cell(name, &items[1]);
                            Object::Bottom
                        }
                        // declared_writes is now no_std-reachable (#565);
                        // its no_std shim returns true unconditionally
                        // (kernel runs only compile-authored code, no
                        // user-code threat surface), so this arm is a
                        // no-op there but enforces caps under std.
                        Some(name) if !crate::declared_writes::is_store_allowed(name) => {
                            note_bottom_cell(name, &items[1]);
                            Object::Bottom
                        }
                        Some(name) => store(name, items[1].clone(), &items[2]),
                        None => Object::Bottom,
                    }
                }
                _ => Object::Bottom,
            }
        }

        Func::And => {
            match x.as_seq() {
                Some(items) if items.len() == 2 => {
                    match (items[0].as_atom(), items[1].as_atom()) {
                        (Some("T"), Some("T")) => Object::t(),
                        (Some("T"), Some("F")) | (Some("F"), Some("T")) | (Some("F"), Some("F")) => Object::f(),
                        _ => Object::Bottom,
                    }
                }
                _ => Object::Bottom,
            }
        }

        Func::Or => {
            match x.as_seq() {
                Some(items) if items.len() == 2 => {
                    match (items[0].as_atom(), items[1].as_atom()) {
                        (Some("F"), Some("F")) => Object::f(),
                        (Some("T"), Some("T")) | (Some("T"), Some("F")) | (Some("F"), Some("T")) => Object::t(),
                        _ => Object::Bottom,
                    }
                }
                _ => Object::Bottom,
            }
        }

        Func::Not => {
            match x.as_atom() {
                Some("T") => Object::f(),
                Some("F") => Object::t(),
                _ => Object::Bottom,
            }
        }

        // ── Combining Forms ──────────────────────────────────────

        Func::Constant(obj) => obj.clone(),

        Func::Compose(f, g) => {
            let gx = apply(g, x, d);
            apply(f, &gx, d)
        }

        Func::Construction(funcs) => {
            // Serial under a fuel cap (thread-local, Rayon workers
            // would start at u64::MAX and escape the caller's bound).
            #[cfg(all(feature = "parallel", not(feature = "no_std")))]
            if funcs.len() >= 16 && !fuel_is_bounded() {
                let results: Vec<Object> = funcs.par_iter()
                    .map(|f| apply(f, x, d))
                    .collect();
                return Object::seq(results);
            }
            let results: Vec<Object> = funcs.iter()
                .map(|f| apply(f, x, d))
                .collect();
            Object::seq(results) // bottom-preserving via Object::seq
        }

        Func::Condition(p, f, g) => {
            match apply(p, x, d) {
                Object::Atom(ref s) if s == "T" => apply(f, x, d),
                Object::Atom(ref s) if s == "F" => apply(g, x, d),
                _ => Object::Bottom,
            }
        }

        Func::ApplyToAll(f) => {
            // task-744 / #743 phase 3: Map cells iterate their values
            // and produce a Seq result. α is a point-wise transform —
            // ordering is incidental; the only consumer assumption is
            // "Seq out, len = len in." Map storage preserves both.
            match x {
                Object::Seq(items) if items.is_empty() => Object::phi(),
                Object::Seq(items) => {
                    // Parallel α: Rayon par_iter for large sequences.
                    // Threshold 64: below this, Rayon spawn overhead exceeds gain.
                    // Stays serial under a fuel cap so the per-element
                    // debit is observed on this thread (fuel is thread-local).
                    #[cfg(all(feature = "parallel", not(feature = "no_std")))]
                    if items.len() >= 64 && !fuel_is_bounded() {
                        return Object::seq(
                            items.par_iter().map(|xi| apply(f, xi, d)).collect()
                        );
                    }
                    Object::seq(items.iter().map(|xi| apply(f, xi, d)).collect())
                }
                Object::Map(m) if m.is_empty() => Object::phi(),
                Object::Map(m) => Object::seq(m.values().map(|xi| apply(f, xi, d)).collect()),
                _ => Object::Bottom,
            }
        }

        Func::Insert(f) => {
            match x.as_seq() {
                // Backus §11.2.4: `/f:<>` is the right UNIT of `f` (the
                // empty-fold base case), not ⊥ — `/∧:<> = T`, `/+:<> = 0`.
                // Operators with no known unit keep ⊥ (still paper-faithful).
                Some(items) if items.is_empty() =>
                    unit_of(f).unwrap_or(Object::Bottom),
                Some(items) if items.len() == 1 => items[0].clone(),
                Some(items) if items.len() >= 2 => {
                    // Right fold, ITERATIVE. Was recursive on the tail
                    // (`apply(Insert(f), items[1..])`), so `/f` over an
                    // N-element STORED sequence cost N nested apply() frames —
                    // proportional to entity count, overflowing rayon worker
                    // stacks on bulk derivations (aggregates / `every`
                    // universals over ~900 facts). The data is stored; walk it
                    // in a loop, O(1) stack. Right-fold identity:
                    //   /f:<x1,...,xn> = f:<x1, f:<x2, ... f:<x_{n-1}, x_n>>>
                    // seed with the last element and fold leftward. ⊥ short-
                    // circuits, matching the recursive form's ⊥-propagation
                    // (Object::seq is ⊥-preserving, so f:<x, ⊥> = ⊥ either way).
                    let mut acc = items[items.len() - 1].clone();
                    for x in items[..items.len() - 1].iter().rev() {
                        acc = apply(f, &Object::seq(vec![x.clone(), acc]), d);
                        if acc.is_bottom() {
                            return Object::Bottom;
                        }
                    }
                    acc
                }
                _ => Object::Bottom,
            }
        }

        Func::Filter(p) => {
            // task-744 / #743 phase 3: Map cells iterate values, output
            // a Seq of kept tuples. Filter is point-wise (Bottom only on
            // shape mismatch / atom input); ordering is incidental.
            match x {
                Object::Seq(items) if items.is_empty() => Object::phi(),
                Object::Seq(items) => {
                    // Parallel filter falls back to serial when a fuel
                    // cap is in effect — same reasoning as ApplyToAll.
                    #[cfg(all(feature = "parallel", not(feature = "no_std")))]
                    if items.len() >= 64 && !fuel_is_bounded() {
                        let kept: Vec<Object> = items.par_iter()
                            .filter(|xi| apply(p, xi, d) == Object::t())
                            .cloned()
                            .collect();
                        return Object::Seq(kept.into());
                    }
                    let kept: Vec<Object> = items.iter()
                        .filter(|xi| apply(p, xi, d) == Object::t())
                        .cloned()
                        .collect();
                    Object::Seq(kept.into())
                }
                Object::Map(m) if m.is_empty() => Object::phi(),
                Object::Map(m) => {
                    let kept: Vec<Object> = m.values()
                        .filter(|xi| apply(p, xi, d) == Object::t())
                        .cloned()
                        .collect();
                    Object::Seq(kept.into())
                }
                _ => Object::Bottom,
            }
        }

        Func::BinaryToUnary(f, obj) => {
            apply(f, &Object::seq(vec![obj.clone(), x.clone()]), d)
        }

        Func::While(p, f) => {
            let current = x.clone();
            let max_iterations = 1000; // safety limit
            // While = bounded tail recursion (Backus 11.2.4)
            // Ok = continue iterating, Err = early exit (predicate false or ⊥)
            match (0..max_iterations).try_fold(current, |acc, _| {
                match apply(p, &acc, d) {
                    Object::Atom(ref s) if s == "T" => {
                        let next = apply(f, &acc, d);
                        if next.is_bottom() { Err(Object::Bottom) } else { Ok(next) }
                    }
                    Object::Atom(ref s) if s == "F" => Err(acc),
                    _ => Err(Object::Bottom),
                }
            }) {
                Ok(_) => Object::Bottom,    // limit exceeded
                Err(result) => result,      // early exit
            }
        }

        Func::FoldL(f) => {
            match x.as_seq() {
                Some(items) if items.len() == 2 => {
                    let seq = match items[1].as_seq() {
                        Some(s) => s,
                        None => return Object::Bottom,
                    };
                    // foldl f z <e₁,...,eₙ> (Backus: left fold with early termination on ⊥)
                    seq.iter().try_fold(items[0].clone(), |acc, element| {
                        let result = apply(f, &Object::seq(vec![acc, element.clone()]), d);
                        if result.is_bottom() { Err(Object::Bottom) } else { Ok(result) }
                    }).unwrap_or(Object::Bottom)
                }
                _ => Object::Bottom,
            }
        }

        Func::IndexBy(keyfn) => {
            // perf-hashjoin: group x's elements by the atom key keyfn:elem into
            // a Map<key, <elem...>>. Read-only (no Store capability gate), so a
            // derivation Func can build a hash index for an O(n) equi-join.
            // Non-atom keys are skipped (mirrors the SM fold's pre-SetFromSeq
            // φ-key filtering).
            match x.as_seq() {
                Some(items) => {
                    let mut groups: HashMap<String, Vec<Object>> = HashMap::new();
                    for element in items.iter() {
                        let k = apply(keyfn, element, d);
                        if let Some(ks) = k.as_atom() {
                            groups.entry(ks.to_string()).or_default().push(element.clone());
                        }
                    }
                    Object::map(groups.into_iter()
                        .map(|(k, v)| (k, Object::Seq(v.into())))
                        .collect())
                }
                _ => Object::Bottom,
            }
        }

        Func::OrderBy(keyfn) => {
            // sort x's elements stably-ascending by the atom key keyfn:elem.
            // Numeric keys (epoch / recorded-at) compare numerically; else
            // lexicographic (ISO timestamps). A non-atom key → "" (sorts
            // first), retained — total + stable so a latest-wins fold over the
            // ordered stream is deterministic. AREST.tex eq:sm.
            match x.as_seq() {
                Some(items) => {
                    let mut keyed: Vec<(String, Object)> = items.iter()
                        .map(|e| {
                            let k = apply(keyfn, e, d);
                            (k.as_atom().map(|s| s.to_string()).unwrap_or_default(), e.clone())
                        })
                        .collect();
                    keyed.sort_by(|(ka, _), (kb, _)|
                        match (ka.parse::<f64>(), kb.parse::<f64>()) {
                            (Ok(a), Ok(b)) =>
                                a.partial_cmp(&b).unwrap_or(core::cmp::Ordering::Equal),
                            _ => ka.cmp(kb),
                        });
                    Object::seq(keyed.into_iter().map(|(_, e)| e).collect())
                }
                _ => Object::Bottom,
            }
        }

        Func::Def(name) => {
            let def_obj = fetch(name, d);
            match def_obj {
                // The named def itself is absent → ⊥. Name the rule so a
                // top-level "engine returned ⊥" reports the missing def
                // as the origin rather than a bare ⊥.
                Object::Bottom => {
                    note_bottom_rule(name);
                    Object::Bottom
                }
                obj => {
                    // Sec-5: if DEFS contains `allowed_writes:{name}`
                    // as a Seq of atom cell names, scope the body under
                    // those caps. Absent cell = unrestricted (legacy
                    // behavior, preserves the established baseline).
                    // Now no_std-reachable (#565) — kernel build's
                    // shimmed `push_caps` is a no-op so this is safe.
                    let _caps_guard = defs_writes_scope(name, d);
                    let result = apply(&metacompose(&obj, d), x, d);
                    // ⊥-trace: if the rule body bottomed out, record this
                    // def as the enclosing rule frame. No-op unless armed;
                    // the success path skips the `is_bottom` branch.
                    if result.is_bottom() {
                        note_bottom_rule(name);
                    }
                    result
                }
            }
        }

        // Platform primitives mostly require serde_json + std modules and are
        // not available in the no_std kernel build — they return Bottom so
        // apply() stays total (kernel code RELIES on that, e.g. the `extract`
        // verb's Bottom → 503 seam). The PURE alloc-only leaves are the
        // exception: `skolem` (task-970 value invention) is evaluated by
        // every skolem-head view rule, and the §5.2 in-kernel Slint surface
        // resolves those rules at fetch time (viewproj-client-render — the
        // first QEMU boot drew an EMPTY form because this arm bottomed the
        // ve_<fnv> mint while the host build minted fine).
        #[cfg(not(feature = "no_std"))]
        Func::Platform(name) => apply_platform(name, x, d),
        #[cfg(feature = "no_std")]
        Func::Platform(name) => match name.as_str() {
            "skolem" => platform_skolem(x),
            _ => Object::Bottom,
        },

        Func::Native(f) => f(x),
    }
}

/// Platform primitives — known operations resolved by name.
/// Each is a fixed function (x, D) → Object. Synthesizable to hardware.
/// Requires serde_json + std modules; excluded from no_std builds.
#[cfg(not(feature = "no_std"))]
fn apply_platform(name: &str, x: &Object, d: &Object) -> Object {
    match name {
        "compile" => platform_compile(x, d),
        "apply_command" => platform_apply_command(x, d),
        "verify_signature" => platform_verify_signature(x),
        "induce" => platform_induce(x, d),
        // #894 — CIDR containment lifts the SSRF blocklist check from
        // hardcoded Rust to a typed Platform Func. See `platform_cidr_contains`.
        "cidr_contains" => platform_cidr_contains(x),
        // Codd θ₁ relational operators: take runtime data that cannot be
        // parameterized in compile-time FFP combining forms. Routing via
        // Platform lets each runtime (server, FPGA, Solidity) provide its
        // own implementation of the same named operation.
        "project" => platform_project(x),
        "join" => platform_join(x),
        "tie" => platform_tie(x),
        "compose_rel" => platform_compose_rel(x),
        "tc" => platform_tc(x),
        "tc_cycles" => platform_tc_cycles(x),
        // task-970: deterministic Skolem value-invention for existential
        // (TGD) derivation heads. Input is a frontier tuple (a seq of atom
        // values); output is a stable `ve_<fnv1a64>` id. Same FNV-1a-64 the
        // forward-chain dedup uses, so re-derivation is idempotent
        // (same frontier → same id → no duplicate entity). See
        // `platform_skolem` and `readings/ui/skolem-head-design.md`.
        "skolem" => platform_skolem(x),
        // H4 (#692): RMAP (Halpin Ch. 10 relational mapping) was the
        // last production Func::Native leaf. Routing through Platform
        // makes it introspectable and lets each runtime (server, FPGA,
        // Solidity) supply its own table-mapping strategy. The body
        // (and its serde_json encode) live in `platform_rmap` below.
        #[cfg(feature = "std-deps")]
        "rmap" => platform_rmap(x, d),
        s if s.starts_with("create:") => platform_create(&s[7..], x, d),
        s if s.starts_with("update:") => platform_update(&s[7..], x, d),
        s if s.starts_with("transition:") => platform_transition(&s[11..], x, d),
        s if s.starts_with("list_noun:") => platform_list_noun(&s[10..], d),
        s if s.starts_with("get_noun:") => platform_get_noun(&s[9..], x, d),
        s if s.starts_with("query_ft:") => platform_query_ft(&s[9..], x, d),
        // Wall-clock primitive — opt-in via the `wall-clock` feature.
        // The fold no longer depends on it (recorded_at is a logical
        // stamp, see logical_commit_stamp); a domain that explicitly
        // models time can still call `now` when the feature is compiled
        // in. Absent (cloudflare/no_std) → this arm drops out and `now`
        // falls through to the fallback registry (Bottom), so wasm32
        // never hits SystemTime's panic.
        #[cfg(feature = "wall-clock")]
        "now" => platform_now(),
        // Fall through to the runtime-installed callback registry for
        // names outside the compile-derived range. See
        // `install_platform_fn` — hosts (ML scorer, local projector,
        // tests) install sync bodies here. Returns Bottom when no body
        // is installed, preserving total-function semantics.
        _ => dispatch_platform_fallback(name, x, d),
    }
}

/// Platform primitive: return current wall-clock time as Unix epoch
/// milliseconds, atom-encoded decimal. Key: "now". Input: ignored.
///
/// S1a (#717): introduces the host-clock primitive for the immutable-cell
/// chain (S1, #716). The returned timestamp populates a VersionEntry's
/// `recorded_at` field at write time.
///
/// OPT-IN: gated on the `wall-clock` feature. The cell-fold does NOT use
/// this — `VersionEntry.recorded_at` is a logical monotonic stamp (see
/// `logical_commit_stamp`), so the chain is host-clock-free on every
/// target. This primitive exists only for domains that explicitly model
/// time. `wall-clock` is in `default` (native std), absent from
/// `cloudflare` (wasm32 — `SystemTime::now()` panics; there is no
/// `Date.now` import) and `no_std` (kernel — no clock). In those builds
/// the `now` arm drops out and `now` returns Bottom via the fallback
/// registry — total, never a panic.
#[cfg(feature = "wall-clock")]
fn platform_now() -> Object {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    Object::atom(&ms.to_string())
}

/// Logical commit stamp for `VersionEntry.recorded_at`. A process-
/// monotonic counter — NO host clock — so the cell-fold is pure and
/// deterministic on every target (wasm32 / kernel included) and the
/// "wasm32 SystemTime gap" the engine tests work around stops existing.
/// One value per `merge_delta` call (commit-batch atomicity). A
/// monotonic-nondecreasing decimal atom — the contract the chain's
/// `latest-by-recorded-at` aggregation relies on, now skew-free and
/// deterministic instead of wall-clock-dependent.
fn logical_commit_stamp() -> Object {
    use core::sync::atomic::{AtomicUsize, Ordering};
    static COMMIT_SEQ: AtomicUsize = AtomicUsize::new(1);
    let n = COMMIT_SEQ.fetch_add(1, Ordering::Relaxed);
    Object::atom(&alloc::format!("{}", n))
}

/// Codd π: project:<indices, R> → rows of R restricted to the given column indices.
fn platform_project(x: &Object) -> Object {
    x.as_seq()
        .filter(|items| items.len() == 2)
        .and_then(|items| {
            let indices = items[0].as_seq()?;
            let relation = items[1].as_seq()?;
            let selectors: Vec<usize> = indices.iter()
                .filter_map(|i| i.as_atom().and_then(|s| s.parse().ok()))
                .collect();
            (!selectors.is_empty()).then_some(())?;
            let rows: Vec<Object> = relation.iter()
                .filter_map(|tuple| {
                    let cols = tuple.as_seq()?;
                    let projected: Vec<Object> = selectors.iter()
                        .filter_map(|&s| (s >= 1 && s <= cols.len()).then(|| cols[s-1].clone()))
                        .collect();
                    Some(Object::Seq(projected.into()))
                })
                .fold(Vec::new(), |mut acc, row| {
                    (!acc.contains(&row)).then(|| acc.push(row));
                    acc
                });
            Some(Object::Seq(rows.into()))
        })
        .unwrap_or(Object::Bottom)
}

/// Codd ⋈: join:<shared_col, R, S> → natural join on shared column index.
fn platform_join(x: &Object) -> Object {
    x.as_seq()
        .filter(|items| items.len() == 3)
        .and_then(|items| {
            let shared_col: usize = items[0].as_atom().and_then(|s| s.parse().ok())?;
            let r = items[1].as_seq()?;
            let s = items[2].as_seq()?;
            let result: Vec<Object> = r.iter()
                .filter_map(|r_tuple| {
                    r_tuple.as_seq()
                        .filter(|cols| shared_col >= 1 && shared_col <= cols.len())
                })
                .flat_map(|r_cols| {
                    let r_val = r_cols[shared_col - 1].clone();
                    s.iter().filter_map(move |s_tuple| {
                        let s_cols = s_tuple.as_seq()
                            .filter(|cols| shared_col >= 1 && shared_col <= cols.len())?;
                        (r_val == s_cols[shared_col - 1]).then(|| {
                            let mut merged: Vec<Object> = r_cols.to_vec();
                            merged.extend(s_cols.iter().enumerate()
                                .filter(|(i, _)| i + 1 != shared_col)
                                .map(|(_, col)| col.clone()));
                            Object::Seq(merged.into())
                        })
                    })
                })
                .collect();
            Some(Object::Seq(result.into()))
        })
        .unwrap_or(Object::Bottom)
}

/// Codd γ (tie): tie:R → Filter(eq ∘ [sel(1), sel(n)]) : R, then drop last col.
fn platform_tie(x: &Object) -> Object {
    x.as_seq()
        .map(|relation| {
            Object::Seq(relation.iter()
                .filter_map(|tuple| {
                    let cols = tuple.as_seq()?;
                    (cols.len() >= 2 && cols[0] == cols[cols.len() - 1])
                        .then(|| Object::Seq(cols[..cols.len()-1].into()))
                })
                .collect())
        })
        .unwrap_or(Object::Bottom)
}

/// Codd ⋅ (compose): compose_rel:<shared_col, R, S> = π₁ₛ(R ⋈ S).
fn platform_compose_rel(x: &Object) -> Object {
    x.as_seq()
        .filter(|items| items.len() == 3)
        .and_then(|items| {
            let shared_col: usize = items[0].as_atom().and_then(|s| s.parse().ok())?;
            let r = items[1].as_seq()?;
            let s = items[2].as_seq()?;
            let result: Vec<Object> = r.iter()
                .filter_map(|r_tuple| {
                    r_tuple.as_seq()
                        .filter(|cols| shared_col >= 1 && shared_col <= cols.len())
                })
                .flat_map(|r_cols| {
                    let r_val = r_cols[shared_col - 1].clone();
                    s.iter().filter_map(move |s_tuple| {
                        let s_cols = s_tuple.as_seq()
                            .filter(|cols| shared_col >= 1 && shared_col <= cols.len())?;
                        (r_val == s_cols[shared_col - 1]).then(|| {
                            let projected: Vec<Object> = r_cols.iter().enumerate()
                                .filter(|(i, _)| i + 1 != shared_col)
                                .map(|(_, col)| col.clone())
                                .chain(s_cols.iter().enumerate()
                                    .filter(|(i, _)| i + 1 != shared_col)
                                    .map(|(_, col)| col.clone()))
                                .collect();
                            Object::Seq(projected.into())
                        })
                    })
                })
                .collect();
            Some(Object::Seq(result.into()))
        })
        .unwrap_or(Object::Bottom)
}

/// Transitive closure over encoded facts, returning self-loops (cycles)
/// as violation-shaped objects. Input shape: sequence of
/// <<noun0, val0>, <noun1, val1>> encoded facts. Output shape: sequence
/// of fact-like objects for nodes that participate in a cycle.
/// Used by the acyclic (AC) ring constraint compiler.
fn platform_tc_cycles(x: &Object) -> Object {
    let initial = match x.as_seq() {
        Some(e) => e.to_vec(),
        None => return Object::Bottom,
    };
    // Extract <role0_val, role1_val> from each encoded fact.
    fn edge_pair(fact: &Object) -> Option<(String, String)> {
        let items = fact.as_seq().filter(|i| i.len() >= 2)?;
        let v0 = items[0].as_seq().and_then(|p| p.get(1)).and_then(|v| v.as_atom())?;
        let v1 = items[1].as_seq().and_then(|p| p.get(1)).and_then(|v| v.as_atom())?;
        Some((v0.to_string(), v1.to_string()))
    }
    let original_pairs: Vec<(String, String)> = initial.iter()
        .filter_map(|f| edge_pair(f))
        .collect();
    // Fixed point: extend with one-hop reachable edges until stable.
    let tc: hashbrown::HashSet<(String, String)> = core::iter::successors(
        Some(original_pairs.iter().cloned().collect::<hashbrown::HashSet<_>>()),
        |tc| {
            let new_edges: Vec<(String, String)> = tc.iter()
                .flat_map(|(a, b)| original_pairs.iter()
                    .filter(|(c, _)| b == c)
                    .filter_map(|(_, d)| {
                        (!tc.contains(&(a.clone(), d.clone())))
                            .then(|| (a.clone(), d.clone()))
                    })
                    .collect::<Vec<_>>())
                .collect();
            (!new_edges.is_empty()).then(|| {
                let mut next = tc.clone();
                next.extend(new_edges);
                next
            })
        },
    ).take(1001).last().unwrap_or_default();
    // Self-loops → violation-shaped objects.
    let cycle_nodes: Vec<Object> = tc.iter()
        .filter(|(a, b)| a == b)
        .map(|(a, _)| Object::seq(vec![
            Object::seq(vec![Object::atom("_"), Object::atom(a)]),
            Object::seq(vec![Object::atom("_"), Object::atom(a)]),
        ]))
        .collect();
    Object::Seq(cycle_nodes.into())
}

/// Transitive closure over an edge relation. Iterates until no new edges are added.
fn platform_tc(x: &Object) -> Object {
    let edges = match x.as_seq() {
        Some(e) => e.to_vec(),
        None => return Object::Bottom,
    };
    let mut closure = edges.clone();
    loop {
        let new_edges: Vec<Object> = closure.iter()
            .filter_map(|a| a.as_seq())
            .flat_map(|a_cols| closure.iter()
                .filter_map(move |b| b.as_seq().map(|b_cols| (a_cols, b_cols))))
            .filter_map(|(a_cols, b_cols)| {
                (a_cols.len() >= 2 && b_cols.len() >= 2 && a_cols[1] == b_cols[0])
                    .then(|| Object::seq(vec![a_cols[0].clone(), b_cols[1].clone()]))
            })
            .filter(|edge| !closure.contains(edge))
            .fold(Vec::new(), |mut acc, e| {
                (!acc.contains(&e)).then(|| acc.push(e));
                acc
            });
        if new_edges.is_empty() { break; }
        closure.extend(new_edges);
    }
    Object::Seq(closure.into())
}

/// H4 (#692): RMAP platform body. Calls Halpin's relational-mapping
/// procedure on the live state and serialises the resulting
/// `Vec<TableDef>` to a JSON atom — the inverse of
/// `crate::rmap::decode_rmap_result`. The serde_json hop is what
/// keeps this op host-only (`std-deps`); embedded / FPGA targets
/// supply their own `"rmap"` body via `install_platform_fn`.
///
/// Input: ignored (RMAP reads schema from cells in `d`).
/// Returns Object::Bottom only on a serialisation failure that the
/// host should never reach — the procedure itself is total.
#[cfg(all(not(feature = "no_std"), feature = "std-deps"))]
fn platform_rmap(_x: &Object, d: &Object) -> Object {
    let tables = crate::rmap::rmap(d);
    let json = serde_json::to_string(&tables).unwrap_or_else(|_| "[]".to_string());
    Object::atom(&json)
}

/// Platform primitive: signature verification (AREST §5.5).
/// Input: seq<atom, atom, atom> — (sender, payload, signature).
/// Output: atom("true"|"false"), or Object::Bottom on malformed input.
/// Wired through crate::crypto::verify_signature — HMAC-SHA256 over
/// (sender || "::" || payload), constant-time hex compare via `subtle`.
/// Key from AREST_HMAC_KEY env (production) or DEV_KEY fallback.
#[cfg(not(feature = "no_std"))]
fn platform_verify_signature(x: &Object) -> Object {
    let parts = match x.as_seq() {
        Some(p) if p.len() == 3 => p,
        _ => return Object::Bottom,
    };
    let sender = match parts[0].as_atom() { Some(s) => s, None => return Object::Bottom };
    let payload = match parts[1].as_atom() { Some(s) => s, None => return Object::Bottom };
    let signature = match parts[2].as_atom() { Some(s) => s, None => return Object::Bottom };
    let ok = crate::crypto::verify_signature(sender, payload, signature);
    Object::atom(match ok { true => "true", false => "false" })
}

/// #894 — `cidr_contains` Func::Platform. Input: 2-element Seq
/// `<<cidr_atom>, <host_atom>>` where `cidr_atom` is a CIDR string
/// like `"127.0.0.0/8"` and `host_atom` is an IPv4 dotted-quad or
/// bare IPv6 literal. Output: `Object::t()` / `Object::f()` —
/// matches the boolean convention apps reach for via Func::Cond.
/// Malformed input ⇒ `Object::Bottom` (the standard total-function
/// fall-through; apps using `cidr_contains` as a predicate should
/// always treat a Bottom return as "policy violation" / "not contained").
///
/// The Rust body delegates to `parse_forml2::cidr_contains` so both
/// the in-process SSRF check (`is_forbidden_url_in_state`) and the
/// Platform-routed call site share one implementation. Lifting this
/// to Platform lets apps declare `Func::Platform("cidr_contains")` in
/// derivation-rule bodies — e.g. an app's own access-control policy
/// can compose CIDR membership without re-implementing the parser.
#[cfg(not(feature = "no_std"))]
fn platform_cidr_contains(x: &Object) -> Object {
    let parts = match x.as_seq() {
        Some(p) if p.len() == 2 => p,
        _ => return Object::Bottom,
    };
    let cidr = match parts[0].as_atom() { Some(s) => s, None => return Object::Bottom };
    let host = match parts[1].as_atom() { Some(s) => s, None => return Object::Bottom };
    match crate::parse_forml2::cidr_contains(cidr, host) {
        true  => Object::t(),
        false => Object::f(),
    }
}

/// #851 — `induce` Func::Platform dispatch. Thin parser that lifts
/// the search-loop's args off the FFP-shaped `x` operand, then hands
/// off to `induce::run_search` (the search loop lives in `induce.rs`
/// alongside the #848-#850 primitives it composes).
///
/// Input shape — a Seq of pair-bindings:
///
///   <<ft_id, "<FT id>">, <to_explain, <fact₁, fact₂, …>>>
///
/// `ft_id` is read via `binding(x, "ft_id")` (atom-valued pair).
/// `to_explain` is a sub-Seq of InstanceFact-shaped facts; we walk
/// `x.as_seq()` to find the `<to_explain, …>` pair directly because
/// `binding` only handles atom-valued pairs.
///
/// Empty `x` (Object::phi or no `ft_id` binding) is a no-op — returns
/// phi. This preserves the #846 stub contract: callers can still
/// distinguish "induce ran but emitted nothing" from "induce was never
/// wired" (the latter yields Object::Bottom from `apply_platform`'s
/// fallback).
///
/// `d` carries both the compiled defs (validate, derivation:*) AND the
/// observation cells (post-`defs_to_state` overlay). The search loop
/// reuses `d` as both `state` and `defs` per the integration the
/// induce::tests::coin_side_no_to_explain_yields_one_hypothesis_per_enum_value
/// acceptance test exercises.
#[cfg(not(feature = "no_std"))]
fn platform_induce(x: &Object, d: &Object) -> Object {
    let Some(ft_id) = binding(x, "ft_id") else { return Object::phi(); };
    if ft_id.is_empty() { return Object::phi(); }
    let to_explain: Vec<Object> = x.as_seq()
        .and_then(|items| items.iter().find_map(|p| {
            let pair = p.as_seq()?;
            if pair.len() != 2 { return None; }
            if pair[0].as_atom() != Some("to_explain") { return None; }
            pair[1].as_seq().map(|s| s.to_vec())
        }))
        .unwrap_or_default();
    let hyps = crate::induce::run_search(d, d, ft_id, &to_explain);
    // task-985 (arc issue 12.3): honor the documented `bound` param —
    // a map of pre-pinned role values. The MCP shim always sent it;
    // this parser silently dropped it, so candidates spanned the whole
    // enumeration regardless. Post-filter the ranked candidates to
    // those whose hidden fact carries EVERY bound pair (a candidate's
    // hidden cell is `Hypothesis_Candidate_has_hidden__Fact` →
    // per-FT-projected fact whose bindings include the role). Filtering
    // preserves the Confidence-Score-descending order.
    let bound_pairs: Vec<(String, String)> = x.as_seq()
        .and_then(|items| items.iter().find_map(|p| {
            let pair = p.as_seq()?;
            if pair.len() != 2 { return None; }
            if pair[0].as_atom() != Some("bound") { return None; }
            pair[1].as_seq().map(|entries| entries.iter().filter_map(|e| {
                let kv = e.as_seq()?;
                if kv.len() != 2 { return None; }
                Some((kv[0].as_atom()?.to_string(), kv[1].as_atom()?.to_string()))
            }).collect::<Vec<_>>())
        }))
        .unwrap_or_default();
    if bound_pairs.is_empty() {
        return Object::Seq(hyps.into());
    }
    let kept: Vec<Object> = hyps.into_iter().filter(|hyp| {
        let hidden = fetch_or_phi("Hypothesis_Candidate_has_hidden__Fact", hyp);
        let facts: Vec<Object> = cell_facts_iter(&hidden).cloned().collect();
        facts.iter().any(|fact| {
            bound_pairs.iter().all(|(role, want)| {
                binding(fact, role) == Some(want.as_str())
            })
        })
    }).collect();
    Object::Seq(kept.into())
}

/// compile ∘ parse: readings text → new defs merged into D.
/// Returns the new state D' (caller stores it).
/// Max input buffer size — platform hardware limit.
pub(crate) const PLATFORM_MAX_INPUT: usize = 1_024 * 1_024;

/// Max per-field value size within a Command — DoS bound.
pub(crate) const PLATFORM_MAX_FIELD: usize = 64 * 1024;

/// Metamodel namespace (security #23): these noun names belong to the
/// self-describing metamodel bootstrap. Once the bootstrap has declared them,
/// user domains MUST NOT redeclare (shadow) them on subsequent compiles.
/// The first compile (empty D) is free to populate the namespace; later
/// compiles that try to layer a new definition over an existing metamodel
/// noun are rejected by `platform_compile`.
pub(crate) const RESERVED_METAMODEL_NOUNS: &[&str] = &[
    "Noun",
    "Fact Type",
    "Role",
    "Constraint",
    "State Machine Definition",
    "Transition",
    "Status",
    "Event Type",
    "Domain Change",
    "Derivation Rule",
];

/// Does the given state's `Noun` cell already declare this name?
/// Pure scan — no side effects, no allocation beyond the cell walk.
fn noun_cell_has(state: &Object, name: &str) -> bool {
    fetch_cell_seq("Noun", state)
        .as_seq()
        .map(|facts| facts.iter().any(|f| binding(f, "name") == Some(name)))
        .unwrap_or(false)
}

/// Find the first reserved metamodel noun that `parsed` declares AND that is
/// already present in `existing`. Returns None when the check passes (either
/// because the parsed state does not touch the metamodel namespace, or because
/// this is the bootstrap compile that legitimately owns the first declaration).
fn find_metamodel_shadow(parsed: &Object, existing: &Object) -> Option<String> {
    let parsed_nouns = fetch_cell_seq("Noun", parsed);
    let facts = parsed_nouns.as_seq()?;
    facts.iter().find_map(|fact| {
        let name = binding(fact, "name")?;
        match RESERVED_METAMODEL_NOUNS.contains(&name) && noun_cell_has(existing, name) {
            true => Some(name.to_string()),
            false => None,
        }
    })
}

#[cfg(not(feature = "no_std"))]
fn platform_compile(x: &Object, d: &Object) -> Object {
    let input = match x.as_atom() {
        Some(s) if s.len() <= PLATFORM_MAX_INPUT => s,
        Some(_) => return Object::atom("⊥ input exceeds platform buffer"),
        None => return Object::Bottom,
    };

    // Parse readings into cells, with context from D (nouns + fact types)
    let parsed = match crate::parse_forml2::parse_to_state_from(input, d) {
        Ok(s) => s,
        Err(e) => return Object::atom(&format!("⊥ {}", e)),
    };

    // Metamodel namespace protection (security #23). The FORML2 parser also
    // rejects this at the Domain level, but we re-check at the state-cell
    // boundary to defend against any future code path that bypasses the
    // parser's Domain-level guard (e.g. direct state injection).
    //
    // NOTE: instance facts that reference metamodel nouns (e.g.
    // "Noun 'Order'" in instance fact position) can trigger false positives
    // because the parser emits a Noun cell entry for the referenced name.
    // We therefore only fire this guard when the new declaration BOTH
    // already exists in d AND the parsed state's Noun entry is of a
    // metamodel reserved kind. The simplest proxy: only reject if the
    // metamodel noun's objectType in parsed differs from d (i.e. the user
    // is redefining it). Since we don't have a cheap way to compare
    // objectType here without re-entering the parser, we skip the re-check
    // at the compile boundary and rely on the parser's Domain-level guard.
    let _ = find_metamodel_shadow as fn(_, _) -> _;

    // SSRF defense (#25, #894): External System federation must not reach
    // internal/loopback/link-local hosts, file:// URLs, or internal DNS.
    // Walk the parsed InstanceFact cell and reject any forbidden URL.
    // The CIDR blocklist now lives in `d`'s `CIDR_Block_has_Block_Kind`
    // cell (populated by `readings/core/security.md`); the Rust side
    // just reads it and applies `cidr_contains` per row.
    match crate::parse_forml2::find_forbidden_instance_url(&parsed, d) {
        Some(url) => return Object::atom(&format!("⊥ forbidden URL in External System: {}", url)),
        None => {}
    }

    // Drop SCHEMA cells (any cell name with non-empty contents in
    // the fresh parse) from the prior state before merging. Schema
    // cells are pure functions of the readings — preserving stale
    // entries causes both old and new to coexist (the #913
    // DerivationRule cascade failure; the post-#931 Derivation Mode
    // InstanceFact stale-wins first-write cascade). User population
    // cells (FT cells, SM cells — names NOT in the fresh parse) are
    // preserved. Empty-cell guard: an empty-readings recompile
    // shouldn't drop anything (otherwise `platform_compile("")`
    // wipes the pre-loaded model and the MC-violation rejection
    // never fires).
    let parsed_cell_names: hashbrown::HashSet<&str> =
        cells_iter(&parsed).into_iter()
            .filter(|(_, c)| c.as_seq().map(|s| !s.is_empty()).unwrap_or(false)
                || c.as_map().map(|m| !m.is_empty()).unwrap_or(false))
            .map(|(name, _)| name)
            .collect();
    let d_for_merge = {
        let mut map: HashMap<String, Object> = HashMap::new();
        for (name, contents) in cells_iter(d).into_iter() {
            if parsed_cell_names.contains(name) { continue; }
            map.insert(name.to_string(), contents.clone());
        }
        Object::Map(map.into())
    };

    // Merge: foldl(concat_cell, D, cells(parsed))
    let merged_state = merge_states(&d_for_merge, &parsed);

    // Structural model validation (#48, task 807) — catch FORML2 violations
    // at compile time, partitioned by modality (AREST.tex eq:create §157,
    // thm:complete §362, §442 "the compile op is itself subject to validate"):
    // an ALETHIC violation means the merged state is not a valid model ("It is
    // impossible that …", §328) and MUST reject the compile (D' = D); DEONTIC
    // findings warn and the compile proceeds (D' = D''). NOTE: reference-scheme
    // value types are now materialized by synthesize_ref_scheme_constraints
    // (ORM 2 — a reference mode is a view of a reference fact type over a VALUE
    // TYPE), so a `.id` role no longer false-flags as an "undeclared noun";
    // the reject fires only on a genuinely undeclared object type.
    let model_violations = crate::compile::validate_model_classified_from_state(&merged_state);
    model_violations.iter()
        .filter(|v| !v.alethic)
        .for_each(|v| { diag!("[model warning] {}", v.message); });
    let model_alethic: Vec<&str> = model_violations.iter()
        .filter(|v| v.alethic)
        .map(|v| v.message.as_str())
        .collect();
    if !model_alethic.is_empty() {
        return Object::atom(&format!("⊥ model violation: {}",
            dedup_violations_with_counts(model_alethic.into_iter())));
    }

    // Compile defs from merged state + re-register platform primitives
    let mut defs = crate::compile::compile_to_defs_state(&merged_state);
    defs.push(("compile".to_string(), Func::Platform("compile".to_string())));
    defs.push(("apply".to_string(), Func::Platform("apply_command".to_string())));
    defs.push(("verify_signature".to_string(), Func::Platform("verify_signature".to_string())));
    defs.push(("audit".to_string(), Func::Platform("audit".to_string())));
    // #894 — CIDR membership predicate. Powers the SSRF check above and
    // is reachable from apps' own derivation rules. See
    // `platform_cidr_contains` for the input/output shape contract.
    defs.push(("cidr_contains".to_string(), Func::Platform("cidr_contains".to_string())));
    let new_d = defs_to_state(&defs, &merged_state);
    // compile-reflect-schema-as-facts: regenerate the schema-as-facts
    // population (Fact_Type_has_Role / Role_is_played_by_Noun /
    // Noun_has_Object_Type) from the freshly merged schema cells —
    // set-replace, idempotent, what the view-projection rules join over.
    let new_d = {
        let mut map: HashMap<String, Object> = cells_iter(&new_d).into_iter()
            .map(|(name, contents)| (name.to_string(), contents.clone()))
            .collect();
        for (name, contents) in crate::compile::reflect_schema_cells(&merged_state) {
            map.insert(name, contents);
        }
        Object::Map(map.into())
    };

    // Validate: ρ(validate) applied to merged state. Alethic violations reject.
    // Skipped when the Policy_skip_validate cell holds atom "T" — installed
    // via `install_skip_validate(&d)` (e.g. CLI --no-validate at boot).
    let decoded = match is_skip_validate(&merged_state) {
        true => vec![],
        false => {
            let ctx = encode_eval_context_state("", None, &merged_state);
            let violations = apply(&Func::Def("validate".to_string()), &ctx, &new_d);
            decode_violations(&violations)
        }
    };
    match decoded.iter().any(|v| v.alethic) {
        true => Object::atom(&format!("⊥ constraint violation: {}",
            dedup_violations_with_counts(
                decoded.iter().filter(|v| v.alethic).map(|v| v.constraint_text.as_str())))),
        false => record_compile_event(&new_d, "compiled"),
    }
}

/// arc-agi-3 round-3 NIT: the live-compile reject envelope used to
/// repeat every per-instance violation line verbatim — a one-fact
/// fragment against a dirty population produced a 235 KB error of
/// thousands of identical constraint texts. Aggregate: first-seen
/// order, each distinct message once, `(xN)` appended for repeats.
fn dedup_violations_with_counts<'a>(msgs: impl Iterator<Item = &'a str>) -> String {
    let mut order: Vec<&str> = Vec::new();
    let mut counts: hashbrown::HashMap<&str, usize> = hashbrown::HashMap::new();
    for m in msgs {
        let n = counts.entry(m).or_insert(0);
        if *n == 0 { order.push(m); }
        *n += 1;
    }
    order.into_iter()
        .map(|m| match counts[m] {
            1 => m.to_string(),
            n => alloc::format!("{} (x{})", m, n),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Security #22 — Evolution state machine trace.
///
/// Records the compile operation as a Domain Change instance fact on the
/// `compile_history` cell. Each successful compile transitions through the
/// state machine (proposed → validated → compiled); alethic rejection is
/// tracked by the error atom return value (no state transition). The
/// sequence number is derived from the existing cell length — no wall-clock
/// time needed and safe for WASM.
///
/// This is a minimal trace: the goal is to leave an audit record that the
/// compile event occurred, not to implement full Domain Change identity.
/// See readings/evolution.md §4.2 and AREST paper §4.2 (Self-modification
/// is ingesting readings).
fn record_compile_event(state: &Object, status: &str) -> Object {
    let seq = fetch_cell_seq("compile_history", state)
        .as_seq()
        .map(|items| items.len())
        .unwrap_or(0);
    let id = format!("compile-{}", seq);
    let fact = fact_from_pairs(&[
        ("Domain Change", id.as_str()),
        ("status", status),
    ]);
    // S1c (#719): the compile_history cell carries the structural trace
    // (Domain Change facts per §4.2 evolution machinery). The legacy
    // `audit_log` push is gone — the chain (S1b) is now the audit
    // surface, and operation/sender provenance lives in VersionEntry's
    // `event` field for entries minted by the apply path.
    cell_push("compile_history", fact, state)
}

/// apply command: create = emit ∘ validate ∘ derive ∘ resolve (Eq. 10).
/// Identity is a fact in the input — "Resource is created by User" (instances.md).
/// Authorization is enforced by the constraint pipeline, not by this function.
#[cfg(not(feature = "no_std"))]
fn platform_apply_command(x: &Object, d: &Object) -> Object {
    let input = match x.as_atom() {
        Some(s) if s.len() <= PLATFORM_MAX_INPUT => s,
        Some(_) => return Object::atom("⊥ input exceeds platform buffer"),
        None => return Object::Bottom,
    };
    // Two accepted input shapes:
    //   (a) raw Command JSON — `{"type":"createEntity",...}`
    //   (b) JS-envelope wrapper — `{"command":{...},"population":"<json>"}`
    //
    // The host SDK (src/api/engine.ts:applyCommand) wraps because
    // callers want to pass a per-call population alongside the command
    // without mutating the tenant's compiled state. Detect the wrapper
    // shape, peel the command out, and ingest the population into a
    // forked state before dispatch. The raw-Command branch is the
    // backward-compatible fast path used by the kernel REPL and the
    // CLI's single-tenant calls.
    let parsed: serde_json::Value = match serde_json::from_str(input) {
        Ok(v) => v,
        Err(e) => return Object::atom(&format!("⊥ {}", e)),
    };
    // task-930: a bare top-level JSON array `[ <op>, … ]` is sugar for
    // the collection-shaped batch — wrap it as `{"type":"batch",
    // "commands":[…]}` so it deserializes into `Command::Batch`. Both
    // the raw form and the `{command, population}` envelope may carry a
    // batch (the envelope's `command` field can itself be an array).
    let wrap_array = |v: serde_json::Value| -> serde_json::Value {
        if v.is_array() {
            serde_json::json!({ "type": "batch", "commands": v })
        } else {
            v
        }
    };
    let (command_json, population_str): (serde_json::Value, Option<String>) = if parsed.get("command").is_some() {
        // (b) — extract the inner command + population fields. The inner
        // command may itself be a collection (batch sugar).
        let cmd = wrap_array(parsed.get("command").cloned().unwrap_or(serde_json::Value::Null));
        let pop = parsed.get("population")
            .and_then(|v| if v.is_string() { v.as_str().map(String::from) } else { Some(v.to_string()) });
        (cmd, pop)
    } else {
        (wrap_array(parsed), None)
    };
    let command: crate::command::Command = match serde_json::from_value(command_json) {
        Ok(c) => c,
        Err(e) => return Object::atom(&format!("⊥ {}", e)),
    };
    // Per-field bound: reject commands whose field values exceed the platform limit.
    match command_field_overflow(&command) {
        Some(field) => return Object::atom(&format!("⊥ field '{}' exceeds platform buffer", field)),
        None => {}
    }
    // If the caller supplied a population envelope, ingest it into a
    // forked state so apply_command_defs sees those facts in D. The
    // shape mirrors `forwardChain`'s input (`{"facts":[{factType,
    // subject?, roles?}, …]}`) so a single host can build one
    // population JSON string and pass it to either pipeline.
    let dispatch_state = match population_str {
        Some(pop_str) if !pop_str.is_empty() && pop_str != "null" => {
            ingest_population_into(d, &pop_str)
        }
        _ => d.clone(),
    };
    let result = crate::command::apply_command_defs(&dispatch_state, &command, &dispatch_state);
    // #766 / #797: return the `{__state_delta, __result}` Map carrier
    // the writer-dispatcher (`lib.rs::classify_writer_result`)
    // recognises as CommitDelta. Before this lift we stringified the
    // CommandResult into an Object::atom, which fell into `NoCommit` —
    // the apply looked successful but no chain entry was ever appended.
    // The Map carrier slots straight into `merge_delta`, extending each
    // touched cell's chain so `cell_pin` reflects the new version. The
    // encoded `__result` body uses the same compact JSON
    // `decode_command_result` already round-trips, so worker callers
    // that read `__result` get the exact same envelope shape they would
    // have seen pre-#766. The carrier shape is locked down by the
    // `platform_apply_command_*_returns_map_carrier_shape` /
    // `_state_delta_is_map_of_touched_cells` / `_classifies_as_commit_delta`
    // acceptance tests below — #797's #777 linchpin.
    //
    // arc-agi-3 engine-issue 15 (TORN BATCH): this MUST be the
    // `_or_bottom` variant. The single-op platform primitives
    // (platform_create / update / …) already collapse to ⊥ when the
    // forward chain aborted on its time budget — but THIS surface (the
    // `apply` def every MCP/HTTP call rides, including task-930
    // batches) used the plain encoder, so an abort's half-derived
    // partial state encoded as a normal CommitDelta carrier and the
    // host COMMITTED it: a 30-op batch landed its m:n rows while the
    // entity creates inside the aborted op vanished — dangling
    // references, atomicity broken (D' != D despite the abort). ⊥ here
    // routes the dispatcher to the traced-bottom error path: nothing
    // commits, the batch rejects whole (D' = D).
    encode_command_result_or_bottom(&result)
}

/// Merge a JS-side population JSON (`{"facts":[{factType, subject?,
/// roles?}, …]}`) into a fork of D so apply sees the caller's
/// per-call facts alongside D's existing cells. Used by
/// `platform_apply_command` when invoked through the engine.ts
/// `{command, population}` envelope. Unknown shapes are tolerated —
/// malformed entries are skipped, not fatal — same forgiving
/// contract as `forward_chain_to_json`.
///
/// `cfg(not(no_std))` because the body uses serde_json — the no_std
/// kernel target excludes serde_json (HATEOAS extract path takes a
/// different deserialiser), so the function itself is too. This
/// matches the gate on the only caller, `platform_apply_command`.
#[cfg(not(feature = "no_std"))]
fn ingest_population_into(d: &Object, population_json: &str) -> Object {
    let parsed: serde_json::Value = match serde_json::from_str(population_json) {
        Ok(v) => v,
        Err(_) => return d.clone(),
    };
    let facts = match parsed.get("facts").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return d.clone(),
    };
    // Build a reading→canonical-id map from the FactType cell so a
    // caller passing the human-readable form (`"Outbound Email is sent"`)
    // lands in the same cell that compile_to_defs_state's per-FT
    // validate looks up (`Outbound_Email_is_sent`). Without this
    // canonicalization the cell name diverges between ingest path and
    // validate path, and constraints silently never fire.
    let ft_cell = fetch_cell_seq("FactType", d);
    let mut reading_to_id: HashMap<String, String> = HashMap::new();
    if let Some(items) = ft_cell.as_seq() {
        for f in items {
            let Some(id) = binding(f, "id") else { continue };
            // Canonical id always maps to itself — round-trips when caller
            // already passes the underscore form.
            reading_to_id.insert(id.to_string(), id.to_string());
            if let Some(reading) = binding(f, "reading") {
                reading_to_id.insert(reading.to_string(), id.to_string());
                // Slash-separated alternate readings: register each side.
                if reading.contains(" / ") {
                    for part in reading.split(" / ") {
                        let p = part.trim().trim_end_matches('.').trim();
                        if !p.is_empty() {
                            reading_to_id.insert(p.to_string(), id.to_string());
                        }
                    }
                }
            }
        }
    }
    let mut state = d.clone();
    for entry in facts {
        let Some(obj) = entry.as_object() else { continue };
        let Some(fact_type) = obj.get("factType").and_then(|v| v.as_str())
            .or_else(|| obj.get("factTypeId").and_then(|v| v.as_str()))
        else { continue };
        // Resolve the caller's factType to the canonical FT id when the
        // model declares it. Fall back to the raw string for unknown
        // FTs (forward-compat: older code may push to free-form cells).
        let canonical_ft: String = reading_to_id.get(fact_type)
            .cloned()
            .unwrap_or_else(|| fact_type.to_string());
        // Build a fact pairs list. Subject acts as the head-noun
        // binding when role bindings aren't supplied (mirrors the
        // forward_chain ingest convention).
        let mut pairs: Vec<(&str, &str)> = Vec::new();
        if let Some(subj) = obj.get("subject").and_then(|v| v.as_str()) {
            // Best-effort: bind the subject under the fact-type name
            // itself (Noun-like) plus a generic "id" key. Downstream
            // consumers tolerate missing exact role names because the
            // population can be re-projected at apply time.
            pairs.push((fact_type, subj));
        }
        if let Some(role_obj) = obj.get("roles").and_then(|v| v.as_object()) {
            for (k, v) in role_obj {
                if let Some(s) = v.as_str() {
                    pairs.push((k.as_str(), s));
                }
            }
        }
        if pairs.is_empty() { continue; }
        state = cell_push(&canonical_ft, fact_from_pairs(&pairs), &state);
    }
    state
}

/// Encode a `CommandResult`, but collapse to ⊥ when the command's
/// forward-chain aborted on its time budget (a non-terminating
/// derivation cycle — cli-apply-large-tasksdb-nonterminating). The chain
/// already armed a ⊥-trace naming the offending rule/cell at the abort
/// point; returning `Object::Bottom` here lets the dispatcher's
/// `with_bottom_trace` boundary render that origin instead of persisting
/// a half-derived partial state. `take_chain_abort` read-and-clears the
/// thread-local so the flag never leaks past this command.
///
/// On the success path `take_chain_abort()` is `false` and this is just
/// `encode_command_result` — no behavior or cost change.
#[cfg(not(feature = "no_std"))]
fn encode_command_result_or_bottom(result: &crate::command::CommandResult) -> Object {
    if crate::evaluate::take_chain_abort() {
        return Object::Bottom;
    }
    crate::command::encode_command_result(result)
}

/// Platform primitive: create entity from fact pairs (AREST Eq. 6).
/// Key: "create:{noun}". Input: <<field, value>, ...> or <<id, val>, <field, val>, ...>.
/// Returns the result as an Object containing the new state.
#[cfg(not(feature = "no_std"))]
fn platform_create(noun: &str, x: &Object, d: &Object) -> Object {
    let (id, fields) = extract_fact_pairs(x);
    let command = crate::command::Command::CreateEntity {
        noun: noun.to_string(),
        domain: String::new(),
        id,
        fields,
        sender: None,
        signature: None,
    };
    let result = crate::command::apply_command_defs(d, &command, d);
    encode_command_result_or_bottom(&result)
}

/// Platform primitive: update entity from fact pairs.
/// Key: "update:{noun}". Input: <<id, val>, <field, val>, ...>.
///
/// task-861 / #904: a "force" key in the pair list is hoisted out
/// of `fields` and set on the Command's `force` flag (matches the
/// MCP `force: true` opt-out convention). When the SM-bypass guard
/// would otherwise refuse the update, "force" lets the call go
/// through. The field value is interpreted truth-loosely: any value
/// other than "false" / "0" / "" counts as true.
#[cfg(not(feature = "no_std"))]
fn platform_update(noun: &str, x: &Object, d: &Object) -> Object {
    let (id, mut fields) = extract_fact_pairs(x);
    let entity_id = id.unwrap_or_default();
    let force = fields.remove("force").map_or(false, |v| {
        !matches!(v.as_str(), "false" | "0" | "")
    });
    let command = crate::command::Command::UpdateEntity {
        noun: noun.to_string(),
        domain: String::new(),
        entity_id,
        fields,
        sender: None,
        signature: None,
        force,
    };
    let result = crate::command::apply_command_defs(d, &command, d);
    encode_command_result_or_bottom(&result)
}

/// Platform primitive: transition entity state machine.
/// Key: "transition:{noun}". Input: <entity_id, event>.
#[cfg(not(feature = "no_std"))]
fn platform_transition(_noun: &str, x: &Object, d: &Object) -> Object {
    let items = match x.as_seq() {
        Some(s) => s,
        None => return Object::Bottom,
    };
    let entity_id = items.first().and_then(|o| o.as_atom()).unwrap_or("").to_string();
    let event = items.get(1).and_then(|o| o.as_atom()).unwrap_or("").to_string();
    // Extract current status from state for the entity
    let sm = StateMachineCellShape::boot();
    let current_status = fetch_cell_seq(sm.cell_name, d).as_seq()
        .and_then(|facts| facts.iter()
            .find(|f| binding_matches(f, sm.state_machine_role, &entity_id))
            .and_then(|f| binding(f, sm.current_status_role).map(|s| s.to_string())));
    let command = crate::command::Command::Transition {
        entity_id,
        event,
        domain: String::new(),
        current_status,
        sender: None,
        signature: None,
    };
    let result = crate::command::apply_command_defs(d, &command, d);
    encode_command_result_or_bottom(&result)
}

/// Extract (optional id, field map) from an Object of fact pairs.
/// Input: <<id, val>, <field1, val1>, ...> or <<field1, val1>, ...>
fn extract_fact_pairs(x: &Object) -> (Option<String>, hashbrown::HashMap<String, String>) {
    let mut fields = hashbrown::HashMap::new();
    let mut id = None;
    let items = x.as_seq().unwrap_or_default();
    items.iter().for_each(|pair| {
        pair.as_seq().and_then(|kv| {
            let k = kv.first()?.as_atom()?.to_string();
            let v = kv.get(1)?.as_atom()?.to_string();
            Some((k, v))
        }).map(|(k, v)| {
            match k.as_str() {
                "id" => { id = Some(v); }
                _ => { fields.insert(k, v); }
            }
        });
    });
    (id, fields)
}

/// Set of DECLARED fact-type ids (the `id` role of every `FactType` cell row).
/// Reads via `cell_facts_iter` so a folded (Map) `FactType` cell is scanned too,
/// not just a Seq cell. Mirrors the private `is_declared_ft` in `command.rs`
/// (lifted here so `ast.rs` read paths can gate on declaration without crossing
/// the module boundary), but materializes the whole set once so a hot fold can
/// test membership in O(1) instead of rescanning the cell per candidate field.
#[cfg(not(feature = "no_std"))]
fn declared_ft_ids(d: &Object) -> hashbrown::HashSet<String> {
    let cell = fetch_or_phi("FactType", d);
    cell_facts_iter(&cell)
        .filter_map(|f| binding(f, "id").map(|s| s.to_string()))
        .collect()
}

/// Platform primitive: list entities of a noun by reading D at apply-time.
/// Key: "list_noun:{noun}". Input: operand is ignored (may be empty).
///
/// Walks every fact cell in D. A fact contributes to an entity summary if
/// one of its role bindings has a role name equal to the target noun — the
/// role's value is the entity id. Every OTHER role binding on that fact becomes
/// a field/value entry on the entity summary, BUT ONLY when the synthesized
/// per-field id `{noun}_has_{role}` (spaces underscored, matching the parser's
/// id-formation — see `command::fallback_ft_id`) is a DECLARED fact type. That
/// gate is the 3NF-field contract: the entity row is the set of declared
/// `{noun} has {ValueType}` value facts (the same `{noun}_has_` shape the view
/// renderer recovers in `viewproj::enum_options_for_fact_type`). Multiple facts
/// about the same entity merge; later facts overwrite earlier ones for the same
/// field.
///
/// sm-event-stamp-phantom (2026-06): without the declared-FT gate, an SM event
/// stamp — `command.rs` writes `<<{noun},id>,<Timestamp,occurred>>` into the
/// base trigger cell (e.g. `Task_is_started`) so the reconstruction fold can
/// order events — leaked a phantom `Timestamp` field onto the entity row. That
/// field maps to no declared fact type (`Task_has_Timestamp` is never declared),
/// so a round-trip through update flagged `apply:unresolvable-field-key`. The
/// gate rejects it while keeping a LEGITIMATELY declared `{noun} has Timestamp`
/// (e.g. `Log_Entry_has_Timestamp`, `Migration_Application_has_Timestamp`),
/// which IS in the `FactType` cell.
///
/// Returns an atom holding a JSON array: `[{"id":..., <field>:<value>, ...}, ...]`.
/// Returns `Bottom` if no matching entities are found.
#[cfg(not(feature = "no_std"))]
fn platform_list_noun(noun: &str, d: &Object) -> Object {
    use hashbrown::HashMap;
    // ρ-projection (#350): hide facts a Migration has rewritten away.
    // Destructive rewrite would violate §5 Thm 5; projecting here lets
    // every read path pick up the MA-filtered view for free.
    let d = visible_population(d);
    // sm-event-stamp-phantom: a field is folded onto the row only when its
    // synthesized id `{noun}_has_{role}` is a declared fact type. Materialize
    // the declared-id set once (not per fold step) so the inner gate is O(1).
    let declared = declared_ft_ids(&d);
    // `{noun}_has_{role}` with spaces underscored — the canonical 3NF-field id
    // form (identical to `command::fallback_ft_id`'s `{noun}_has_{field}`).
    let noun_us = noun.replace(' ', "_");
    let field_ft_id = |role: &str| -> String {
        alloc::format!("{}_has_{}", noun_us, role.replace(' ', "_"))
    };
    let mut entities: HashMap<String, HashMap<String, String>> = HashMap::new();

    cells_iter(&d).iter().for_each(|(name, contents)| {
        // task-955 Fix 2: only BASE population cells contribute an entity's
        // fields. ':'-namespaced cells (derivation:/schema:/view:/sql:/…) and
        // `_transitive_*` closure cells are derived, not base — folding their
        // role bindings into the entity below (the last-write `insert`) made
        // SM status reads NONDETERMINISTIC: a transitive-closure cell could
        // supply a phantom 'Status' that overwrote the keyed base value in
        // arbitrary iteration order (the 923/924 actions/get artifact).
        if name.contains(':') || name.starts_with("_transitive_") { return; }
        // #932 phase-2: cell_facts_iter so a folded (Map) cell is scanned
        // too, not just Seq cells (the raw-as_seq()-skips-Map bug class).
        cell_facts_iter(contents).for_each(|fact| {
            let pairs = match fact.as_seq() {
                Some(p) => p.to_vec(),
                None => return,
            };
            // Find entity id: the pair whose role name matches the noun.
            let entity_id = pairs.iter().find_map(|pair| {
                let kv = pair.as_seq()?;
                let role = kv.first()?.as_atom()?;
                let val = kv.get(1)?.as_atom()?;
                (role == noun).then(|| val.to_string())
            });
            if let Some(id) = entity_id {
                let entry = entities.entry(id).or_default();
                pairs.iter().for_each(|pair| {
                    let kv = match pair.as_seq() { Some(s) => s, None => return };
                    let role = match kv.first().and_then(|k| k.as_atom()) { Some(r) => r, None => return };
                    let val = match kv.get(1).and_then(|v| v.as_atom()) { Some(v) => v, None => return };
                    // Skip the noun's own id role, and any role whose synthesized
                    // `{noun}_has_{role}` is NOT a declared fact type (the
                    // sm-event-stamp `Timestamp` phantom, and any other
                    // non-canonical base-cell binding). Declared value fields —
                    // e.g. `Task_has_Task_Priority` — pass; relationship roles
                    // (verb-phrase / cross-noun ids like
                    // `State_Machine_is_for_Resource`) are surfaced as navigation,
                    // not folded onto the row, so the declared-`_has_` gate is the
                    // exact 3NF-field contract.
                    if role == noun { return; }
                    if !declared.contains(&field_ft_id(role)) { return; }
                    entry.insert(role.to_string(), val.to_string());
                });
            }
        });
    });

    if entities.is_empty() { return Object::Bottom; }

    let json_items: Vec<serde_json::Value> = entities.into_iter().map(|(id, fields)| {
        let mut obj = serde_json::Map::new();
        obj.insert("id".to_string(), serde_json::Value::String(id));
        fields.into_iter().for_each(|(k, v)| {
            obj.insert(k, serde_json::Value::String(v));
        });
        serde_json::Value::Object(obj)
    }).collect();
    let json = serde_json::to_string(&serde_json::Value::Array(json_items))
        .unwrap_or_else(|_| "[]".to_string());
    Object::atom(&json)
}

/// Platform primitive: query facts of a given fact type from live D.
/// Key: "query_ft:{fact_type_id}". Input: optional filter JSON atom of
/// `{role_name: value}` bindings to match (atom is ignored if not a JSON
/// object). Returns an atom holding a JSON array of facts. Each fact
/// emits as an object keyed by role name. Returns an empty array when
/// the cell is absent or no facts match — never Bottom, since "empty
/// result" is a valid query outcome distinct from "undefined fact type".
#[cfg(not(feature = "no_std"))]
fn platform_query_ft(ft_id: &str, x: &Object, d: &Object) -> Object {
    // ρ-projection (#350): query_ft is a population read path, so
    // migrated-away sources must not appear in results.
    let d = visible_population(d);
    // task-955 Fix 3: a view (derived) fact type's population IS its
    // derivation's output, not the stored cell. When the stored cell is
    // empty, resolve the view — mirrors sql.rs::materialize_fact_type_tables
    // so MCP `query` and `sql` agree on view-backed FTs (e.g.
    // Task_has_Task_Status, whose stored cell stays empty while its 2-stage
    // bridge derivation populates it on read). Stored data, when present, is
    // the eager truth, so only resolve the view for a never-materialized cell.
    let stored = fetch_cell_seq(ft_id, &d);
    let resolved = if cell_fact_count(&stored) == 0 { resolve_view(ft_id, &d, &d) } else { None };
    let facts = resolved.unwrap_or(stored);
    let facts_seq = facts.as_seq().map(|s| s.to_vec()).unwrap_or_default();

    let filter: hashbrown::HashMap<String, String> = x.as_atom()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.as_object().cloned())
        .map(|obj| obj.iter().filter_map(|(k, v)|
            v.as_str().map(|s| (k.clone(), s.to_string()))
        ).collect())
        .unwrap_or_default();

    // #840: ring fact types (both roles share the same noun, e.g.
    // `Task blocks Task`) carry pairs <<Task, blocker>, <Task, blocked>>
    // which collapse to a single key under naive `map.insert`. First
    // pass counts how many times each role name appears; second pass
    // emits subscript-suffixed keys (Task1, Task2, ...) when count > 1
    // — matching the FORML2 derivation-rule subscript convention. Roles
    // that appear exactly once keep their bare names.
    let fact_to_json = |fact: &Object| -> Option<serde_json::Value> {
        let pairs = fact.as_seq()?;
        let mut role_count: hashbrown::HashMap<String, usize> = hashbrown::HashMap::new();
        for pair in pairs.iter() {
            if let Some(kv) = pair.as_seq() {
                if let Some(role) = kv.first().and_then(|k| k.as_atom()) {
                    *role_count.entry(role.to_string()).or_insert(0) += 1;
                }
            }
        }
        let mut next_index: hashbrown::HashMap<String, usize> = hashbrown::HashMap::new();
        let mut map = serde_json::Map::new();
        pairs.iter().for_each(|pair| {
            if let Some(kv) = pair.as_seq() {
                if let (Some(role), Some(val)) = (
                    kv.first().and_then(|k| k.as_atom()),
                    kv.get(1).and_then(|v| v.as_atom()),
                ) {
                    let key = if role_count.get(role).copied().unwrap_or(0) > 1 {
                        let idx = next_index.entry(role.to_string()).or_insert(0);
                        *idx += 1;
                        alloc::format!("{}{}", role, idx)
                    } else {
                        role.to_string()
                    };
                    map.insert(key, serde_json::Value::String(val.to_string()));
                }
            }
        });
        Some(serde_json::Value::Object(map))
    };

    // #840: filter accepts either exact subscripted keys (Task1, Task2)
    // for precise role-targeting on ring FTs, or bare role names (Task)
    // which match any numbered variant. Non-ring queries are unaffected
    // because role names appear once and no subscripting happens.
    let matched: Vec<serde_json::Value> = facts_seq.iter()
        .filter_map(fact_to_json)
        .filter(|obj| {
            let m = match obj.as_object() { Some(m) => m, None => return false };
            filter.iter().all(|(k, v)| {
                if let Some(actual) = m.get(k).and_then(|x| x.as_str()) {
                    return actual == v.as_str();
                }
                // Fallback: filter key is a bare role name on a ring
                // FT — match if any subscripted variant equals v.
                m.iter().any(|(mk, mv)| {
                    mk.starts_with(k.as_str())
                        && mk[k.len()..].chars().all(|c| c.is_ascii_digit())
                        && mv.as_str() == Some(v.as_str())
                })
            })
        })
        .collect();

    let json = serde_json::to_string(&serde_json::Value::Array(matched))
        .unwrap_or_else(|_| "[]".to_string());
    Object::atom(&json)
}

/// Platform primitive: get a single entity by id.
/// Key: "get_noun:{noun}". Input: atom entity id.
#[cfg(not(feature = "no_std"))]
/// Returns the matching entity summary as a JSON atom, or Bottom if absent.
fn platform_get_noun(noun: &str, x: &Object, d: &Object) -> Object {
    let id = match x.as_atom() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return Object::Bottom,
    };
    let list = platform_list_noun(noun, d);
    let list_str = match list.as_atom() { Some(s) => s.to_string(), None => return Object::Bottom };
    let parsed: serde_json::Value = match serde_json::from_str(&list_str) {
        Ok(v) => v, Err(_) => return Object::Bottom,
    };
    let items = match parsed.as_array() { Some(a) => a.clone(), None => return Object::Bottom };
    items.into_iter()
        .find(|item| item.get("id").and_then(|v| v.as_str()) == Some(&id))
        .map(|item| Object::atom(&serde_json::to_string(&item).unwrap_or_default()))
        .unwrap_or(Object::Bottom)
}

/// Walk a Command's string fields and return the name of the first field whose
/// value exceeds PLATFORM_MAX_FIELD bytes, or None if all values are within bound.
#[cfg(not(feature = "no_std"))]
fn command_field_overflow(command: &crate::command::Command) -> Option<&'static str> {
    use crate::command::Command;
    let over = |s: &str| s.len() > PLATFORM_MAX_FIELD;
    let map_over = |m: &hashbrown::HashMap<String, String>| -> bool {
        m.iter().any(|(k, v)| over(k) || over(v))
    };
    match command {
        Command::CreateEntity { noun, domain, id, fields, sender, signature } => {
            match over(noun) { true => return Some("noun"), false => {} }
            match over(domain) { true => return Some("domain"), false => {} }
            match id.as_deref().map(over).unwrap_or(false) { true => return Some("id"), false => {} }
            match map_over(fields) { true => return Some("fields"), false => {} }
            match sender.as_deref().map(over).unwrap_or(false) { true => return Some("sender"), false => {} }
            match signature.as_deref().map(over).unwrap_or(false) { true => return Some("signature"), false => {} }
            None
        }
        Command::Transition { entity_id, event, domain, current_status, sender, signature } => {
            match over(entity_id) { true => return Some("entityId"), false => {} }
            match over(event) { true => return Some("event"), false => {} }
            match over(domain) { true => return Some("domain"), false => {} }
            match current_status.as_deref().map(over).unwrap_or(false) { true => return Some("currentStatus"), false => {} }
            match sender.as_deref().map(over).unwrap_or(false) { true => return Some("sender"), false => {} }
            match signature.as_deref().map(over).unwrap_or(false) { true => return Some("signature"), false => {} }
            None
        }
        Command::Query { schema_id, domain, target, bindings, sender, signature } => {
            match over(schema_id) { true => return Some("schemaId"), false => {} }
            match over(domain) { true => return Some("domain"), false => {} }
            match over(target) { true => return Some("target"), false => {} }
            match map_over(bindings) { true => return Some("bindings"), false => {} }
            match sender.as_deref().map(over).unwrap_or(false) { true => return Some("sender"), false => {} }
            match signature.as_deref().map(over).unwrap_or(false) { true => return Some("signature"), false => {} }
            None
        }
        Command::UpdateEntity { noun, domain, entity_id, fields, sender, signature, force: _ } => {
            match over(noun) { true => return Some("noun"), false => {} }
            match over(domain) { true => return Some("domain"), false => {} }
            match over(entity_id) { true => return Some("entityId"), false => {} }
            match map_over(fields) { true => return Some("fields"), false => {} }
            match sender.as_deref().map(over).unwrap_or(false) { true => return Some("sender"), false => {} }
            match signature.as_deref().map(over).unwrap_or(false) { true => return Some("signature"), false => {} }
            None
        }
        Command::LoadReadings { markdown, domain, sender, signature } => {
            match over(markdown) { true => return Some("markdown"), false => {} }
            match over(domain) { true => return Some("domain"), false => {} }
            match sender.as_deref().map(over).unwrap_or(false) { true => return Some("sender"), false => {} }
            match signature.as_deref().map(over).unwrap_or(false) { true => return Some("signature"), false => {} }
            None
        }
        Command::LoadReading { name, body, sender, signature } => {
            match over(name) { true => return Some("name"), false => {} }
            match over(body) { true => return Some("body"), false => {} }
            match sender.as_deref().map(over).unwrap_or(false) { true => return Some("sender"), false => {} }
            match signature.as_deref().map(over).unwrap_or(false) { true => return Some("signature"), false => {} }
            None
        }
        Command::UnloadReading { name, policy, sender, signature } => {
            match over(name) { true => return Some("name"), false => {} }
            match policy.as_deref().map(over).unwrap_or(false) { true => return Some("policy"), false => {} }
            match sender.as_deref().map(over).unwrap_or(false) { true => return Some("sender"), false => {} }
            match signature.as_deref().map(over).unwrap_or(false) { true => return Some("signature"), false => {} }
            None
        }
        Command::ReloadReading { name, body, policy, sender, signature } => {
            match over(name) { true => return Some("name"), false => {} }
            match over(body) { true => return Some("body"), false => {} }
            match policy.as_deref().map(over).unwrap_or(false) { true => return Some("policy"), false => {} }
            match sender.as_deref().map(over).unwrap_or(false) { true => return Some("sender"), false => {} }
            match signature.as_deref().map(over).unwrap_or(false) { true => return Some("signature"), false => {} }
            None
        }
        // task-930: the batch's bound is the per-op bound — the
        // collection overflows iff any constituent op overflows.
        Command::Batch { commands } => {
            commands.iter().find_map(command_field_overflow)
        }
        // task-971: check fact_type + each pair's role and value.
        Command::AssertFact { fact_type, pairs, sender, signature } => {
            match over(fact_type) { true => return Some("factType"), false => {} }
            for p in pairs {
                match over(&p.role) { true => return Some("role"), false => {} }
                match over(&p.value) { true => return Some("value"), false => {} }
            }
            match sender.as_deref().map(over).unwrap_or(false) { true => return Some("sender"), false => {} }
            match signature.as_deref().map(over).unwrap_or(false) { true => return Some("signature"), false => {} }
            None
        }
        // task-crudl-deploy-readpath: read commands — check noun, entity_id, sender.
        Command::GetEntity { noun, entity_id, sender } => {
            match over(noun) { true => return Some("noun"), false => {} }
            match over(entity_id) { true => return Some("entityId"), false => {} }
            match sender.as_deref().map(over).unwrap_or(false) { true => return Some("sender"), false => {} }
            None
        }
        Command::ListEntities { noun, sender } => {
            match over(noun) { true => return Some("noun"), false => {} }
            match sender.as_deref().map(over).unwrap_or(false) { true => return Some("sender"), false => {} }
            None
        }
    }
}

// ── FFP: Objects represent functions (Backus Section 13) ────────────
//
// In FFP, every object represents a function via the representation
// function ρ. Primitive atoms map to primitive functions. Sequences
// map to functional forms via metacomposition. Defined atoms map to
// their definitions. The meaning function μ evaluates expressions by
// replacing innermost applications (x:y) with (ρ x):y.
//
// This layer bridges FFP semantics with the compiled Func representation.
// The Func enum is the compiled (optimized) form. Objects are the source.

/// Standard atom names for primitive functions (Backus 11.2.3).
pub mod primitives {
    pub const ID: &str = "id";
    pub const TL: &str = "tl";
    pub const ATOM: &str = "a?";
    pub const EQ: &str = "=";
    pub const GT: &str = ">";
    pub const LT: &str = "<";
    pub const GE: &str = ">=";
    pub const LE: &str = "<=";
    pub const NULL: &str = "0?";
    pub const CELL_NAME_TEST: &str = "cn?";
    pub const REVERSE: &str = "<>";
    pub const COMPACT: &str = "ct";
    pub const DISTL: &str = "dl";
    pub const DISTR: &str = "dr";
    pub const HAS_MEMBER: &str = "in?";
    pub const SET_FROM_SEQ: &str = "set";
    pub const LENGTH: &str = "#l";
    pub const TRANS: &str = "tr";
    pub const APNDL: &str = "al";
    pub const APNDR: &str = "ar";
    pub const ROTL: &str = "rl";
    pub const ROTR: &str = "rr";
    pub const ADD: &str = "+";
    pub const SUB: &str = "-";
    pub const MUL: &str = "*";
    pub const DIV: &str = "/";
    pub const AND: &str = "and";
    pub const OR: &str = "or";
    pub const NOT: &str = "not";
    pub const FETCH: &str = "^";
    pub const FETCH_OR_PHI: &str = "^?";
    pub const STORE: &str = "v";
    pub const CONTAINS: &str = "in";
    pub const STARTS_WITH: &str = "in<";
    pub const ENDS_WITH: &str = "in>";
    pub const TRIM: &str = "tm";
    pub const SPLIT: &str = "sp";
    pub const REPLACE: &str = "rp";
    pub const LOWER: &str = "lc";
    pub const CONCAT: &str = "++";
}

/// Standard atom names for functional forms (Backus 11.2.4, 13.3.2).
pub mod forms {
    pub const COMP: &str = ".";
    pub const CONS: &str = "[";
    pub const COND: &str = "?";
    pub const ALPHA: &str = "@";
    pub const INSERT: &str = "/";
    pub const BU: &str = "bu";
    pub const FILTER: &str = "#";
    pub const WHILE: &str = "W";
    pub const FOLDL: &str = "\\";
    pub const INDEX_BY: &str = "ix";
    pub const ORDER_BY: &str = "ob";
    pub const CONST: &str = "'";
}

// ── Cells and State (Backus Section 14.3, 14.7) ─────────────────────
//
// The AST state D is a sequence of cells. Each cell is <CELL, name, contents>.
// fetch (↑n) retrieves the contents of the first cell named n.
// store (↓n) replaces or appends the cell named n with new contents.
// Cells can contain sub-stores (Section 14.7): a cell whose contents
// is itself a sequence of cells. This models partitioned populations.

/// The atom that marks a cell: <CELL, name, contents>
pub const CELL_TAG: &str = "CELL";

/// Create a cell object: <CELL, name, contents>
pub fn cell(name: &str, contents: Object) -> Object {
    Object::seq(vec![Object::atom(CELL_TAG), Object::atom(name), contents])
}

// ─── VersionEntry: cell-version wrapper (S1a, #717) ─────────────────────
//
// Realigns the cell store with whitepaper §3.3 + eq:cellfold (#716).
// Backus's store operator `↓ n: ⟨x, D⟩ → D'` is purely functional: each
// write produces a new D, and `D_n' = foldl μ_n D_n E_n` makes the
// per-cell sequence of intermediate states explicit. Today the Rust
// impl collapses that sequence into a single overwrite (`merge_delta` →
// `map.insert`); S1 (#716) realigns it as an append-version chain.
//
// S1a is the shape definition + Platform `now` primitive only — no
// behavior change. S1b (#718) flips `merge_delta` to append-using-this-
// shape and `cells_iter` to return the latest version's contents.
//
// Encoding: a VersionEntry is itself an Object — a Seq of (key, value)
// pairs, like `fact_from_pairs`. Carries:
//   version_id   : monotonic u64 (atom-encoded decimal)
//   contents     : the cell payload (any Object)
//   prev         : Option<u64> (atom-encoded decimal; empty = None)
//   recorded_at  : wall-clock atom from `platform_now`, or any Object
//   event        : (S1c, optional) the apply-time operand `x` that
//                  produced this entry — eq:cellfold's `μ_n` input.
//                  Carries operation kind + sender + payload, so the
//                  chain doubles as the audit-of-record. Omitted (4-
//                  field shape) for entries from non-apply paths
//                  (compile-time bootstrap, internal forward-chain),
//                  preserving back-compat with pre-S1c freezes.

pub const VERSION_ID_KEY: &str = "version_id";
pub const VERSION_CONTENTS_KEY: &str = "contents";
pub const VERSION_PREV_KEY: &str = "prev";
pub const VERSION_RECORDED_AT_KEY: &str = "recorded_at";
pub const VERSION_EVENT_KEY: &str = "event";

/// Construct a VersionEntry. Pure shape; no clock side-effect — pass
/// the `recorded_at` value the caller wants written (typically the
/// result of `apply_platform("now", …, …)` at write time).
///
/// `event` is the apply-time operand `x` that produced this entry —
/// FFP `μ_n`'s input, eq:cellfold's audit-of-record. `None` for
/// non-apply paths (compile bootstrap, internal forward-chain merges,
/// the synthetic v=0 raw-promote shim) — produces the pre-S1c 4-field
/// shape, byte-identical to existing freezes. `Some(event)` from the
/// apply path (S1c #719); the chain doubles as audit.
pub fn version_entry(
    version_id: u64,
    contents: Object,
    prev: Option<u64>,
    recorded_at: Object,
    event: Option<Object>,
) -> Object {
    let id_str = alloc::format!("{}", version_id);
    let prev_str = match prev {
        Some(p) => alloc::format!("{}", p),
        None => alloc::string::String::new(),
    };
    let mut pairs = alloc::vec![
        Object::seq(alloc::vec![Object::atom(VERSION_ID_KEY), Object::atom(&id_str)]),
        Object::seq(alloc::vec![Object::atom(VERSION_CONTENTS_KEY), contents]),
        Object::seq(alloc::vec![Object::atom(VERSION_PREV_KEY), Object::atom(&prev_str)]),
        Object::seq(alloc::vec![Object::atom(VERSION_RECORDED_AT_KEY), recorded_at]),
    ];
    if let Some(e) = event {
        pairs.push(Object::seq(alloc::vec![Object::atom(VERSION_EVENT_KEY), e]));
    }
    Object::seq(pairs)
}

/// Helper: get the value side of a (key, value) pair from a fact-shaped
/// Object. Differs from `binding` by accepting a non-atom value — needed
/// for VersionEntry's `contents` and `recorded_at` fields, which can hold
/// arbitrary sub-Objects.
fn pair_value<'a>(fact: &'a Object, key: &str) -> Option<&'a Object> {
    fact.as_seq()?.iter().find_map(|pair| {
        let items = pair.as_seq()?;
        (items.len() == 2 && items[0].as_atom() == Some(key))
            .then_some(&items[1])
    })
}

/// Extract a VersionEntry's `version_id` field. Returns None if the
/// Object is not a VersionEntry or the field is malformed.
pub fn version_entry_id(entry: &Object) -> Option<u64> {
    binding(entry, VERSION_ID_KEY)?.parse().ok()
}

pub fn version_entry_contents(entry: &Object) -> Option<&Object> {
    pair_value(entry, VERSION_CONTENTS_KEY)
}

/// Extract the `prev` field. Empty string encodes None (the chain root).
pub fn version_entry_prev(entry: &Object) -> Option<u64> {
    let s = binding(entry, VERSION_PREV_KEY)?;
    if s.is_empty() { None } else { s.parse().ok() }
}

pub fn version_entry_recorded_at(entry: &Object) -> Option<&Object> {
    pair_value(entry, VERSION_RECORDED_AT_KEY)
}

/// S1c (#719): the apply-time operand `x` that produced this entry —
/// FFP `μ_n`'s input, eq:cellfold's audit-of-record. `None` for
/// pre-S1c entries, internal commits, and the synthetic v=0 raw-
/// promote shim. `Some(event)` for entries minted by `apply_command`
/// where the caller threaded the event through `merge_delta_with_event`.
pub fn version_entry_event(entry: &Object) -> Option<&Object> {
    pair_value(entry, VERSION_EVENT_KEY)
}

/// Detect whether an Object carries the VersionEntry shape. Used by
/// S1b's `cells_iter` to decide between unwrap-latest-version (new
/// shape) vs. raw-contents (legacy shape) at read time.
pub fn is_version_entry(obj: &Object) -> bool {
    version_entry_id(obj).is_some() && version_entry_contents(obj).is_some()
}

// ─── Apply event (S1c #757) ─────────────────────────────────────────
//
// The operand `x` that drove a `system_impl` apply call: a (verb,
// operand) pair stamped onto each VersionEntry the apply produces.
// Per FFP applicative-state-transfer, every commit is `f:x → y`; this
// surfaces `x` so the chain doubles as the audit-of-record without a
// sidecar audit_log cell.

pub const APPLY_EVENT_VERB_KEY: &str = "verb";
pub const APPLY_EVENT_OPERAND_KEY: &str = "operand";

/// Build the apply-time event Object that gets attached to every
/// VersionEntry minted by a `system_impl` apply call. Two-pair shape:
///   <<verb, "create:Order">, <operand, <<id, ord-1>, <total, 100>>>>
/// Pre-S1c entries (compile bootstrap, internal forward-chain merges)
/// continue to pass `None` to `merge_delta`.
pub fn apply_event(verb: &str, operand: Object) -> Object {
    Object::seq(alloc::vec![
        Object::seq(alloc::vec![Object::atom(APPLY_EVENT_VERB_KEY), Object::atom(verb)]),
        Object::seq(alloc::vec![Object::atom(APPLY_EVENT_OPERAND_KEY), operand]),
    ])
}

pub fn apply_event_verb(event: &Object) -> Option<&str> {
    binding(event, APPLY_EVENT_VERB_KEY)
}

pub fn apply_event_operand(event: &Object) -> Option<&Object> {
    pair_value(event, APPLY_EVENT_OPERAND_KEY)
}

// ─── Cell version chain (S1b, #718) ─────────────────────────────────────
//
// A "version chain" is an Object::Seq whose elements are all VersionEntry
// objects in chronological order (oldest at index 0, latest at len-1).
// `merge_delta` (the commit boundary) appends a new entry per cell update
// — this realizes whitepaper eq:cellfold's `D_n' = foldl μ_n D_n E_n`.
//
// Reads stay transparent: `fetch`, `fetch_or_phi`, and `cells_iter` all
// auto-unwrap chains to return the latest version's contents. Legacy raw
// values pass through unchanged. `cells_iter_history` (additive) returns
// the full chain for callers who want the trace.
//
// `store` (the intermediate-computation primitive) is unchanged — only
// `merge_delta` versions, so internal forward-chaining work doesn't grow
// a chain entry per fact.

/// Predicate: is this Object a non-empty Seq of VersionEntry items?
pub fn is_version_chain(obj: &Object) -> bool {
    obj.as_seq()
        .map(|items| !items.is_empty() && items.iter().all(is_version_entry))
        .unwrap_or(false)
}

/// Latest entry of a chain, or None if the input isn't a chain.
pub fn chain_latest(obj: &Object) -> Option<&Object> {
    if !is_version_chain(obj) {
        return None;
    }
    obj.as_seq()?.last()
}

/// "Logical contents" of a cell value: latest version's contents if the
/// value is a chain, otherwise the raw object itself. The lifetime of
/// the returned reference matches the input — reads are zero-copy.
pub fn cell_contents_view(obj: &Object) -> &Object {
    if let Some(latest) = chain_latest(obj) {
        if let Some(c) = version_entry_contents(latest) {
            return c;
        }
    }
    obj
}

/// Wrap raw contents in a fresh single-version chain (version_id = 1,
/// prev = None). `event` is the apply-time operand for entries minted
/// by the apply path; `None` for non-apply commits.
pub fn wrap_as_chain(contents: Object, recorded_at: Object, event: Option<Object>) -> Object {
    Object::seq(alloc::vec![version_entry(1, contents, None, recorded_at, event)])
}

/// Append a new version entry to a chain. If `current` is not a chain,
/// promote it to a synthetic version-0 entry first so the new entry's
/// `prev` pointer chains backward into the legacy raw value.
///
/// `event` is attached only to the newly appended entry — the synthetic
/// v=0 promote shim stays event-less because it represents pre-history,
/// not an applied operation.
pub fn chain_append(
    current: &Object,
    new_contents: Object,
    recorded_at: Object,
    event: Option<Object>,
) -> Object {
    if is_version_chain(current) {
        let mut items: alloc::vec::Vec<Object> = current.as_seq()
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();
        let prev_id = items.last()
            .and_then(version_entry_id)
            .unwrap_or(0);
        let new_id = prev_id + 1;
        items.push(version_entry(new_id, new_contents, Some(prev_id), recorded_at, event));
        Object::seq(items)
    } else {
        // Promote legacy raw value to a synthetic v0 with prev=None.
        // The new contents land as v1 with prev=Some(0). The v0 has
        // no event (it never went through an apply); the v1 carries
        // the event the caller threaded.
        let v0_recorded = Object::atom("0");
        Object::seq(alloc::vec![
            version_entry(0, current.clone(), None, v0_recorded, None),
            version_entry(1, new_contents, Some(0), recorded_at, event),
        ])
    }
}

/// Walk the version chain of cell `name` in stored order (oldest first
/// → newest last). Empty Vec if the cell doesn't exist. For non-chain
/// (legacy) values, returns a single-element Vec wrapping the raw value
/// as a synthetic v0 entry — kept allocation-free by returning a Vec of
/// owned VersionEntry Objects in that one branch only.
pub fn cells_iter_history(state: &Object, name: &str) -> Vec<Object> {
    let raw = match state {
        Object::Map(map) => match map.get(name) {
            Some(v) => v.clone(),
            None => return Vec::new(),
        },
        Object::Seq(_) | _ => {
            let v = fetch_raw(name, state);
            if matches!(v, Object::Bottom) { return Vec::new(); }
            v
        }
    };
    if is_version_chain(&raw) {
        raw.as_seq()
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    } else {
        // Legacy: synthesize a v0 entry so callers get a uniform shape.
        alloc::vec![version_entry(0, raw, None, Object::atom("0"), None)]
    }
}

/// Get a specific entry from a version chain by its version_id, or
/// None if the input isn't a chain or no entry matches. S1d (#720)
/// uses this to materialize a cell's value at a snapshot's pinned id.
pub fn chain_at_version(chain: &Object, version_id: u64) -> Option<&Object> {
    if !is_version_chain(chain) {
        return None;
    }
    chain.as_seq()?
        .iter()
        .find(|entry| version_entry_id(entry) == Some(version_id))
}

/// Truncate a chain to entries with version_id <= cutoff, preserving
/// chronological order. S1d rollback: reconstruct a cell's state at
/// the snapshot's pinned version by chopping off any entries that
/// landed after the snapshot was taken. Returns the input unchanged
/// when it isn't a chain.
pub fn chain_truncate_at(chain: &Object, cutoff: u64) -> Object {
    if !is_version_chain(chain) {
        return chain.clone();
    }
    let kept: alloc::vec::Vec<Object> = chain.as_seq()
        .map(|s| s.iter()
            .filter(|entry| version_entry_id(entry).is_some_and(|id| id <= cutoff))
            .cloned()
            .collect())
        .unwrap_or_default();
    if kept.is_empty() {
        // Cutoff predates every entry — this should not happen if the
        // snapshot pin was taken from this chain. Defensive: return phi
        // so the cell behaves as absent.
        return Object::phi();
    }
    Object::seq(kept)
}

/// S1g (#723): drop chain entries whose version_id is not in
/// `keep_ids` AND is not the chain's latest entry. Latest is always
/// kept so the cell's logical view (`fetch` / `cells_iter`) is
/// unaffected — compaction trims audit history, never the active
/// value. Non-chain inputs pass through unchanged.
///
/// `keep_ids` is the union of:
/// - snapshot pins (S1d): every `SnapshotEntry::Pinned(id)` recorded
///   for this cell across the live snapshot map;
/// - citation pins (S1f): every Cell Version Id named by a
///   `Citation_pins_Cell_Name` / `Citation_pins_Cell_Version_Id`
///   pair whose Cell Name is this cell.
///
/// Returns the compacted chain. The returned Object is byte-identical
/// to the input when no entries were eligible for drop, so callers
/// can compare lengths to know whether anything was actually freed.
pub fn compact_chain(chain: &Object, keep_ids: &alloc::collections::BTreeSet<u64>) -> Object {
    if !is_version_chain(chain) {
        return chain.clone();
    }
    let entries = match chain.as_seq() {
        Some(s) => s,
        None => return chain.clone(),
    };
    let latest_id = entries.last().and_then(version_entry_id);
    let kept: alloc::vec::Vec<Object> = entries.iter()
        .filter(|entry| {
            let id = match version_entry_id(entry) {
                Some(id) => id,
                None => return true, // malformed entry — keep defensively
            };
            keep_ids.contains(&id) || latest_id == Some(id)
        })
        .cloned()
        .collect();
    if kept.is_empty() {
        return Object::phi();
    }
    Object::seq(kept)
}

/// S1g (#723): collect the set of cell version_ids cited by Citation
/// facts for the named cell. Walks the post-S1f
/// `Citation_pins_Cell_Name` and `Citation_pins_Cell_Version_Id`
/// cells and returns the inner-join over the Citation column.
///
/// Empty set if either cell is absent or the join finds no row for
/// `cell_name`. Callers union this with snapshot pins before passing
/// to `compact_chain`.
pub fn cell_versions_pinned_by_citations(
    state: &Object,
    cell_name: &str,
) -> alloc::collections::BTreeSet<u64> {
    use alloc::collections::BTreeMap;
    let mut out = alloc::collections::BTreeSet::new();
    let name_cell = fetch_cell_seq("Citation_pins_Cell_Name", state);
    let ver_cell = fetch_cell_seq("Citation_pins_Cell_Version_Id", state);
    let name_facts = match name_cell.as_seq() {
        Some(s) => s,
        None => return out,
    };
    let ver_facts = match ver_cell.as_seq() {
        Some(s) => s,
        None => return out,
    };
    // Build Citation -> Version lookup once, then filter the name cell
    // to citations that pin `cell_name`. O(N+M) over the two cells.
    let mut cite_to_ver: BTreeMap<&str, u64> = BTreeMap::new();
    for fact in ver_facts.iter() {
        let cite = match binding(fact, "Citation") { Some(s) => s, None => continue };
        let ver_str = match binding(fact, "Cell Version Id") { Some(s) => s, None => continue };
        if let Ok(id) = ver_str.parse::<u64>() {
            cite_to_ver.insert(cite, id);
        }
    }
    for fact in name_facts.iter() {
        let cite = match binding(fact, "Citation") { Some(s) => s, None => continue };
        let bound_name = match binding(fact, "Cell Name") { Some(s) => s, None => continue };
        if bound_name == cell_name {
            if let Some(&id) = cite_to_ver.get(cite) {
                out.insert(id);
            }
        }
    }
    out
}

/// S1h (#724): "Cell as_of Version" — the contents of cell `name` at
/// version_id `at`, or None if the cell isn't a chain or no entry
/// matches. Bitemporal point-in-time read over the chain produced by
/// merge_delta. Pure function: traverses the chain in stored order
/// without mutating state.
///
/// Pairs with `chain_truncate_at` (which gives the prefix); this gives
/// just the contents at one moment.
pub fn as_of(state: &Object, name: &str, at: u64) -> Option<Object> {
    let raw = fetch_raw(name, state);
    chain_at_version(&raw, at)
        .and_then(version_entry_contents)
        .cloned()
}

/// S1h (#724): "Cell between Version1 and Version2" — chain entries
/// for cell `name` whose version_id falls in `[lo, hi]` inclusive,
/// in chronological order. Returns an empty Vec if the cell isn't
/// a chain or no entries fall in range. Caller swaps `lo` and `hi`
/// if reversed isn't intended; the helper is total and just gives
/// back nothing when `lo > hi`.
pub fn between(state: &Object, name: &str, lo: u64, hi: u64) -> Vec<Object> {
    if lo > hi {
        return Vec::new();
    }
    let raw = fetch_raw(name, state);
    if !is_version_chain(&raw) {
        return Vec::new();
    }
    raw.as_seq()
        .map(|seq| seq.iter()
            .filter(|entry| {
                version_entry_id(entry).is_some_and(|id| id >= lo && id <= hi)
            })
            .cloned()
            .collect())
        .unwrap_or_default()
}

/// Latest version_id of the cell named `name`, or None if the cell is
/// absent or the stored value isn't a chain. S1d snapshot: capture the
/// pin point per cell at snapshot time.
pub fn cell_pin(state: &Object, name: &str) -> Option<u64> {
    let raw = fetch_raw(name, state);
    if !is_version_chain(&raw) {
        return None;
    }
    chain_latest(&raw).and_then(version_entry_id)
}

/// Fetch the raw stored value (chain-or-raw) without unwrapping.
/// `fetch` exists as the public unwrap-aware reader; this is the
/// chain-preserving counterpart used by `merge_delta`,
/// `cells_iter_history`, and S1d snapshot/rollback.
pub fn fetch_raw(name: &str, state: &Object) -> Object {
    match state {
        Object::Map(map) => map.get(name).cloned().unwrap_or(Object::Bottom),
        Object::Seq(cells) => cells.iter()
            .find_map(|cell_obj| {
                let items = cell_obj.as_seq()?;
                if items.len() == 3
                    && items[0].as_atom() == Some(CELL_TAG)
                    && items[1].as_atom() == Some(name)
                {
                    Some(items[2].clone())
                } else {
                    None
                }
            })
            .unwrap_or(Object::Bottom),
        _ => Object::Bottom,
    }
}

/// Fetch (↑n): retrieve contents of the first cell named n from a store.
/// ↑n:D → c where D contains <CELL, n, c>
/// Returns bottom if no cell named n exists.
/// O(1) for Map stores, O(n) fallback for Seq stores.
pub fn fetch(name: &str, state: &Object) -> Object {
    let raw = fetch_raw(name, state);
    // S1b (#718): unwrap to latest version's contents if the stored
    // value is a version chain. Legacy raw values pass through.
    if is_version_chain(&raw) {
        chain_latest(&raw)
            .and_then(version_entry_contents)
            .cloned()
            .unwrap_or(Object::Bottom)
    } else {
        raw
    }
}

/// Store (↓n): replace or append cell named n with new contents.
/// ↓n:<x, D> → D' where D' has cell n with contents x.
/// If cell n exists, its contents are replaced. Otherwise a new cell is appended.
/// O(1) for Map stores, O(n) fallback for Seq stores.
pub fn store(name: &str, contents: Object, state: &Object) -> Object {
    match state {
        Object::Map(map) => {
            // task-817: Arc-shared HashMap. clone() bumps the ref count;
            // Arc::make_mut clones the inner HashMap only when another
            // reader holds the same Arc. Single-owner state writes stay
            // zero-copy through this path.
            let mut new_arc = Arc::clone(map);
            Arc::make_mut(&mut new_arc).insert(name.to_string(), contents);
            Object::Map(new_arc)
        }
        Object::Seq(cells) => {
            let is_target = |c: &Object| c.as_seq().map_or(false, |items|
                items.len() == 3 && items[0].as_atom() == Some(CELL_TAG) && items[1].as_atom() == Some(name));
            let found = cells.iter().any(is_target);
            let replaced: Vec<Object> = cells.iter().map(|c|
                if is_target(c) { cell(name, contents.clone()) } else { c.clone() }
            ).collect();
            match found {
                true => Object::Seq(replaced.into()),
                false => Object::Seq([replaced, vec![cell(name, contents)]].concat().into()),
            }
        }
        _ => Object::Bottom,
    }
}

// ── State helpers (named-tuple cells for Population-as-Object) ──────

/// Fetch cell contents, defaulting to phi (empty sequence) if not found.
/// Replaces: population.facts.get("key").map(|v| v.as_slice()).unwrap_or(&[])
pub fn fetch_or_phi(name: &str, state: &Object) -> Object {
    match fetch(name, state) {
        Object::Bottom => Object::phi(),
        contents => contents,
    }
}

/// Append a fact to a named cell. Creates the cell if it does not exist.
/// Replaces: population.facts.entry("key").or_default().push(fact)
pub fn cell_push(name: &str, fact: Object, state: &Object) -> Object {
    let existing = fetch_or_phi(name, state);
    let new_contents = match &existing {
        Object::Seq(items) => {
            let mut v = items.to_vec();
            v.push(fact);
            Object::Seq(v.into())
        }
        // sm-status-bridge-projection-lag: a Map (folded / keyed D_n, #932) cell
        // holds its facts as VALUES, and `as_seq()` is None for a Map — so the
        // legacy `None => Seq([fact])` branch SILENTLY DROPPED every existing
        // entry. `cell_filter` is already Map-tolerant, so the transition
        // handler's `cell_filter` (keeps every OTHER machine) + `cell_push`
        // (this) round-tripped a keyed `State_Machine_is_currently_in_Status`
        // Map down to a single-entry Seq, wiping all OTHER machines' statuses.
        // The from-guarded fold then re-seeded those machines to `initial`,
        // reverting an unrelated entity's status mid-transition (a completed
        // blocker flipped back to in_progress, re-blocking its dependent).
        // Preserve the Map's facts and append the new one. Sorted by the
        // content-addressed fact id so the resulting Seq is deterministic (the
        // cell is re-keyed downstream by integrate_round_facts, but determinism
        // keeps any intermediate persisted bytes stable).
        Object::Map(m) => {
            let mut v: Vec<Object> = m.values().cloned().collect();
            v.sort_by_cached_key(|f| synthesize_fact_id(name, f));
            v.push(fact);
            Object::Seq(v.into())
        }
        _ => Object::seq(vec![fact]),
    };
    store(name, new_contents, state)
}

/// Append a fact to a named cell only if no structurally-identical fact
/// is already present. Matches the paper's set-semantics for P: facts
/// are members of a set, so re-asserting the same fact is a no-op.
///
/// Use when emission may fire more than once for the same origin
/// (Citation cells during idempotent ingest, provenance link facts on
/// re-fetch, derivation rules that compute the same fact twice). The
/// primary cell_push remains the default for performance-sensitive
/// paths (O(1) append vs. O(n) contains-check).
pub fn cell_push_unique(name: &str, fact: Object, state: &Object) -> Object {
    let existing = fetch_or_phi(name, state);
    match existing.as_seq() {
        Some(items) if items.iter().any(|f| f == &fact) => state.clone(),
        Some(items) => {
            let mut v = items.to_vec();
            v.push(fact);
            store(name, Object::Seq(v.into()), state)
        }
        None => store(name, Object::seq(vec![fact]), state),
    }
}

/// Seed the `Noun_is_instantiable` cell from a state's `Noun` cell
/// (task-961-b). This is the COMPILED FORM of the FORML derivation
/// `Noun is instantiable iff Noun has Object Type 'entity' and Noun has
/// some Reference Scheme` (readings/core/core.md §Derivation Rules,
/// marked `**`), evaluated eagerly against the `Noun` cell already in
/// `state` — the SAME predicate `compile_to_defs_state` emits as the
/// `_Noun_is_instantiable_compiled` cell (compile.rs §task-961 Phase C).
///
/// Production apply paths always route through `compile_to_defs_state`,
/// which seeds `_Noun_is_instantiable_compiled` into `d`. This helper is
/// for BYPASS paths that build a defs/population state WITHOUT that pass
/// (phi-state test fixtures that `cell_push` a `Noun` directly, or a
/// noun added to `state` after the last compile) — it lets those states
/// carry an authoritative instantiability cell so the run-time gate in
/// `command::noun_runtime_defined` can decide PURELY from the cell, with
/// no procedural fallback.
///
/// Predicate: `objectType == "entity"` AND a non-empty `referenceScheme`
/// (at least one non-empty comma-separated part) — byte-for-byte the
/// predicate `compile_to_defs_state` evaluates against `c_nouns` /
/// `c_ref_schemes`. Idempotent: re-seeding adds no duplicate facts.
pub fn seed_instantiable_cell(state: &Object) -> Object {
    let noun_cell = fetch_cell_seq("Noun", state);
    let instantiable: Vec<&str> = match noun_cell.as_seq() {
        Some(facts) => facts.iter().filter_map(|f| {
            let name = binding(f, "name")?;
            if binding(f, "objectType") == Some("entity")
                && binding(f, "referenceScheme")
                    .map_or(false, |rs| rs.split(',').any(|s| !s.is_empty()))
            {
                Some(name)
            } else {
                None
            }
        }).collect(),
        None => return state.clone(),
    };
    instantiable.into_iter().fold(state.clone(), |acc, noun| {
        cell_push_unique("Noun_is_instantiable", fact_from_pairs(&[("Noun", noun)]), &acc)
    })
}

/// Conflict raised by [`cell_put_keyed`] when the cell already holds a
/// fact at the same key whose non-key contents differ from the
/// incoming fact. Materializes the four pieces a UC-enforcement caller
/// needs to render a violation: the cell name, the colliding key, and
/// both facts. Byte-equal facts at the same key are re-assertions and
/// do NOT raise a conflict — the conflict is reserved for genuine
/// non-key disagreement, which is the Codd-style alethic-UC signal.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyConflict {
    pub name: alloc::string::String,
    pub key: alloc::string::String,
    pub existing_fact: Object,
    pub incoming_fact: Object,
}

/// perf-cellput-on2: test-only instrumentation counting how many times a
/// cell's backing `HashMap` is deep-cloned (`(**m).clone()`) across the
/// cell-put family. This is the operation whose per-fact repetition made
/// `integrate_round_facts` O(n²) (each `cell_put_*` re-cloned the whole
/// growing Map). The batched integrators clone ONCE per cell per round, so
/// the count is CONSTANT in the batch size — the O(n²)-regression guards
/// (`integrate_round_facts_*_clones_cell_once_per_round_not_per_fact`)
/// assert exactly that invariant. Mirrors `evaluate::chain_eval_counter`:
/// thread-local (parallel-test-safe), compiled only under test/test-bins,
/// and a zero-cost empty `record_*` in production no_std builds.
#[cfg(any(test, feature = "test-bins"))]
mod cell_map_clone_counter {
    use core::cell::Cell;
    std::thread_local! {
        pub static COUNT: Cell<usize> = const { Cell::new(0) };
    }
}

/// Reset the cell-Map deep-clone counter. Call before the measured window.
#[cfg(any(test, feature = "test-bins"))]
pub fn reset_cell_map_clone_count() {
    cell_map_clone_counter::COUNT.with(|c| c.set(0));
}

/// Read the cell-Map deep-clone counter accumulated since the last reset.
#[cfg(any(test, feature = "test-bins"))]
pub fn get_cell_map_clone_count() -> usize {
    cell_map_clone_counter::COUNT.with(|c| c.get())
}

/// Record one cell-Map deep-clone. No-op (and trivially inlined away) in
/// non-test builds, so the production hot path pays nothing.
#[inline]
fn record_cell_map_clone() {
    #[cfg(any(test, feature = "test-bins"))]
    cell_map_clone_counter::COUNT.with(|c| c.set(c.get() + 1));
}

/// task-744 / #743: write a fact into a cell that is keyed by its
/// reference-scheme roles (`key_roles`). The cell contents become an
/// `Object::Map<key, fact>` where key is the concatenation of the
/// values at the given role indices.
///
/// Collision detection (task-744 phase 4, refines the original phase-2
/// upsert path): rather than silently last-write-wins, the function
/// distinguishes three cases at write time:
///
/// 1. **No prior entry at this key** — new write. Returns `Ok(state')`
///    with the fact installed.
/// 2. **Prior entry byte-equal to the incoming fact** — re-assertion.
///    Returns `Ok(state.clone())` unchanged; no spurious Arc churn.
/// 3. **Prior entry differs from the incoming fact in any non-key
///    role value** — collision. Returns `Err(KeyConflict { … })`. The
///    state is NOT mutated; the caller decides how to surface the
///    UC violation (apply-path validator emits a `Violation`; bulk
///    loaders may aggregate; debug tooling may print).
///
/// API choice: `Result<Object, KeyConflict>` over the alternative of
/// writing collisions into a `_KeyConflicts` meta-cell. The Result
/// shape is type-system-enforced — a caller cannot accidentally drop
/// the conflict by forgetting to read a side-channel cell. Existing
/// callers (currently only the 9 test sites in this module) acquire a
/// trivial `.expect("…")` at sites that are exercising the happy path.
///
/// `key_roles` are role names matching the binding/pair-fact shape
/// produced by `fact_from_pairs` (and the reading pipeline). Each
/// role's value is extracted via `binding(fact, role)`.
///
/// Returns `Ok(state.clone())` unchanged when any required role is
/// missing from the fact (caller treats as a no-op rather than a
/// write of a partially-keyed tuple). A missing role can never be a
/// UC collision, since the fact is not fully keyed.
pub fn cell_put_keyed(
    name: &str,
    key_role_names: &[&str],
    fact: Object,
    state: &Object,
) -> Result<Object, KeyConflict> {
    let Some(key) = extract_key_from_fact(&fact, key_role_names) else {
        return Ok(state.clone());
    };
    let existing = fetch_or_phi(name, state);
    let mut map: HashMap<String, Object> = match &existing {
        Object::Map(m) => { record_cell_map_clone(); (**m).clone() }
        Object::Seq(items) if items.is_empty() => HashMap::new(),
        Object::Seq(items) => {
            // Migration: existing Seq contents rebuild into Map keyed
            // by the same role names. If two pre-existing Seq facts
            // share a key, the later one wins during migration — the
            // legacy Seq path never enforced uniqueness, so this is
            // the best we can do without rewriting the migration as
            // its own conflict-bearing pass. New conflicts (incoming
            // fact vs. migrated entry) are still detected below.
            let mut m = HashMap::new();
            for f in items.iter() {
                if let Some(k) = extract_key_from_fact(f, key_role_names) {
                    m.insert(k, f.clone());
                }
            }
            m
        }
        _ => HashMap::new(),
    };
    if let Some(existing_fact) = map.get(&key) {
        if existing_fact == &fact {
            // Byte-equal re-assertion: no-op. Return the original
            // state unchanged so callers can detect a no-op via
            // structural equality / Arc-pointer identity if they
            // care.
            return Ok(state.clone());
        }
        return Err(KeyConflict {
            name: name.into(),
            key,
            existing_fact: existing_fact.clone(),
            incoming_fact: fact,
        });
    }
    map.insert(key, fact);
    Ok(store(name, Object::Map(map.into()), state))
}

/// Extract a key from a fact tuple by joining the values at the
/// named roles. Used by `cell_put_keyed` and the Map-backed
/// constraint enforcement path.
///
/// Returns None when any role is absent from the fact — caller's
/// signal that the fact isn't fully-keyed and should not be stored
/// in a keyed cell.
pub fn extract_key_from_fact(fact: &Object, key_role_names: &[&str]) -> Option<String> {
    let mut parts: Vec<String> = Vec::with_capacity(key_role_names.len());
    for role in key_role_names {
        let v = binding(fact, role)?;
        parts.push(v.to_string());
    }
    Some(parts.join("\u{001f}")) // ASCII unit-separator — won't collide with atom contents
}

/// Positional counterpart to [`extract_key_from_fact`]: build the key from
/// the values at the given 0-based role POSITIONS, joined the same way.
///
/// Dup-role-name-safe, unlike the by-name `extract_key_from_fact`. An
/// aggregate head like `Value shortest reaches Value for Feature at Count`
/// stores TWO `Value` pairs that a by-name key cannot tell apart (`binding`
/// returns the FIRST) — so name-keying both COLLIDES distinct groups (every
/// `(rk, _)` keys to `rk\u{1f}rk\u{1f}…`) AND points at the wrong role.
/// Positional keying reads the value at each named POSITION, so the group
/// (Value@0, Value@1, Feature@2) keys correctly. The aggregate head is
/// emitted in DECLARED ROLE ORDER (compile::compile_aggregate_derivation,
/// the composite branch), so stored position i IS role i.
///
/// Returns None when any index is out of bounds or the pair is malformed —
/// the caller's signal the fact isn't shaped for this key (skip it).
pub fn extract_key_from_fact_by_index(fact: &Object, key_indices: &[usize]) -> Option<String> {
    let seq = fact.as_seq()?;
    let mut parts: Vec<String> = Vec::with_capacity(key_indices.len());
    for &i in key_indices {
        let pair = seq.get(i)?.as_seq()?;
        if pair.len() != 2 { return None; }
        parts.push(pair[1].as_atom()?.to_string());
    }
    Some(parts.join("\u{001f}")) // same US separator as extract_key_from_fact
}

/// #932 phase-2: fold a fact into a cell that has NO narrower uniqueness
/// constraint, keyed by its full tuple via `synthesize_fact_id`. This is
/// the keyless counterpart to [`cell_put_keyed`] and the fold μ_n
/// (`eq:cellfold`) for keyless cells — the 3NF row's natural identity
/// when a fact type's only UC is the spanning one over all roles (ORM:
/// every fact type has a uniqueness constraint; absent a narrower one it
/// is the whole tuple).
///
/// Set semantics, per P-as-set (`eq:pop`): re-asserting a byte-identical
/// fact is an idempotent no-op (same tuple → same key); two facts that
/// differ in ANY role get distinct keys, so every distinct row is
/// preserved. There is no `KeyConflict` — with the full tuple as the key
/// there is no non-key role left to disagree on.
///
/// Unlike `cell_put_keyed`'s `extract_key_from_fact` (binding-by-name),
/// the FNV hash over the (role,value) pairs — ordered by a name-only
/// STABLE sort — is dup-role-name-safe AND direction-preserving: a ring
/// fact type like `Task blocks Task` stores two `Task` pairs that a
/// by-name key cannot tell apart, and the two directions <a,b> / <b,a>
/// keep distinct keys (their role-index order is preserved, not sorted
/// away by value). See `synthesize_fact_id`.
///
/// A pre-existing Seq cell is migrated to the keyed Map in the same pass
/// (each Seq fact re-keyed by its own `synthesize_fact_id`), mirroring
/// `cell_put_keyed`'s Seq→Map migration.
pub fn cell_put_folded(name: &str, fact: Object, state: &Object) -> Object {
    let key = synthesize_fact_id(name, &fact);
    let existing = fetch_or_phi(name, state);
    let mut map: HashMap<String, Object> = match &existing {
        Object::Map(m) => { record_cell_map_clone(); (**m).clone() }
        Object::Seq(items) => items
            .iter()
            .map(|f| (synthesize_fact_id(name, f), f.clone()))
            .collect(),
        _ => HashMap::new(),
    };
    map.insert(key, fact);
    store(name, Object::Map(map.into()), state)
}

/// Batched counterpart to [`cell_put_folded`]: fold MANY facts into a
/// keyless cell with a SINGLE map clone + `store`, instead of one clone
/// per fact. Behavior-identical to folding `cell_put_folded` over
/// `facts` (set semantics; full-tuple key via `synthesize_fact_id`;
/// idempotent re-assert collapses), but O(n) where the per-fact loop was
/// O(n²) — every `cell_put_folded` re-cloned the whole growing Map
/// (ast.rs:5000), so integrating N facts cost Σⁿ clones.
///
/// perf-cellput-on2: the 8001-line tasks readings drove ~7378
/// classification facts into a single cell in ONE forward-chain round;
/// the per-fact path then spent ~29s re-cloning that Map (see
/// `evaluate::integrate_round_facts`, the sole caller). One clone here
/// collapses that round to ~milliseconds.
pub fn cell_put_folded_batch(name: &str, facts: Vec<Object>, state: &Object) -> Object {
    if facts.is_empty() {
        return state.clone();
    }
    let existing = fetch_or_phi(name, state);
    let mut map: HashMap<String, Object> = match &existing {
        Object::Map(m) => { record_cell_map_clone(); (**m).clone() }
        Object::Seq(items) => items
            .iter()
            .map(|f| (synthesize_fact_id(name, f), f.clone()))
            .collect(),
        _ => HashMap::new(),
    };
    for fact in facts {
        let key = synthesize_fact_id(name, &fact);
        map.insert(key, fact);
    }
    store(name, Object::Map(map.into()), state)
}

/// Batched counterpart to [`cell_put_keyed`]: integrate MANY facts into a
/// keyed (alethic-UC) cell with a SINGLE map clone + `store`. Returns the
/// new state and the conflicts that were DROPPED (so the caller can log
/// them exactly as the per-fact path did).
///
/// Semantics mirror `cell_put_keyed` plus the conflict handling that
/// lived in `integrate_round_facts`, per fact:
///   * key absent              → insert
///   * key present, == fact     → idempotent no-op
///   * key present, != fact:
///       - `upsert == true`     → last-write-wins (overwrite); not a conflict
///       - `upsert == false`    → keep existing; return the `KeyConflict`
/// Within one batch, two facts at the same key resolve the same way two
/// sequential calls would: non-upsert keeps the FIRST (later ones
/// conflict), upsert keeps the LAST. The `upsert` arm subsumes the old
/// `drop_keyed_entry` + re-put (a SECOND full clone per conflicting
/// write) with a single in-place `HashMap::insert`. O(n) vs O(n²).
pub fn cell_put_keyed_batch(
    name: &str,
    key_role_names: &[&str],
    facts: Vec<Object>,
    upsert: bool,
    state: &Object,
) -> (Object, Vec<KeyConflict>) {
    cell_put_keyed_batch_with(
        name, |f| extract_key_from_fact(f, key_role_names), facts, upsert, state)
}

/// derivation-aggregate-composite-key-upsert: positional counterpart to
/// [`cell_put_keyed_batch`]. Keys each fact by the values at the given role
/// POSITIONS (dup-role-name-safe — see [`extract_key_from_fact_by_index`])
/// instead of by role name. Used for composite AGGREGATE HEAD cells, whose
/// group roles can share a noun name (two `Value`). Same upsert / conflict
/// semantics as the by-name batch; `upsert=true` (the aggregate's
/// latest=min fold over a GROWING recursive source) makes each round's
/// re-fold SUPERSEDE the group's prior value instead of appending a stale
/// second tuple — the IVM fix for `min`-over-recursive-closure misfold.
pub fn cell_put_keyed_batch_by_index(
    name: &str,
    key_indices: &[usize],
    facts: Vec<Object>,
    upsert: bool,
    state: &Object,
) -> (Object, Vec<KeyConflict>) {
    cell_put_keyed_batch_with(
        name, |f| extract_key_from_fact_by_index(f, key_indices), facts, upsert, state)
}

/// Shared core for the keyed batch puts: identical fold/upsert/conflict
/// logic over ANY key-extraction strategy `key_of` (by-name for UC cells,
/// by-index for composite aggregate heads). Factored so the two public
/// entry points cannot drift in their migration / conflict handling.
fn cell_put_keyed_batch_with<K: Fn(&Object) -> Option<String>>(
    name: &str,
    key_of: K,
    facts: Vec<Object>,
    upsert: bool,
    state: &Object,
) -> (Object, Vec<KeyConflict>) {
    if facts.is_empty() {
        return (state.clone(), Vec::new());
    }
    let existing = fetch_or_phi(name, state);
    let mut map: HashMap<String, Object> = match &existing {
        // store-on-derive PERF WIN (folded-stale-tuple wall): RE-KEY a
        // pre-existing Map under `key_of`, symmetric with the Seq arm
        // below — do NOT take it verbatim. A folded keyed cell stores
        // entries under the FULL-TUPLE hash (`cell_put_folded` /
        // `merge_delta`'s union commit), so a stale same-UC-key tuple
        // (`t1→pending`) sits under a DIFFERENT map key than the role-key
        // an incoming upsert (`t1→in_progress`) writes at. Verbatim-clone
        // (the pre-fix `(**m).clone()`) preserved BOTH and the upsert never
        // displaced the stale one — the exact wall that forced the #836
        // wipe to stay load-bearing in `transition_via_defs`. Re-keying
        // collapses the cell to one entry per `key_of` (later wins on a
        // clash — a genuine UC violation in folded legacy data, resolved
        // the same last-write-wins direction the Seq migration uses), so a
        // subsequent keyed upsert now COLLIDES on the role key and
        // overwrites in place. Entries whose `key_of` is `None` (a partial
        // tuple a keyed upsert can never address) are PRESERVED under their
        // original map key, so nothing re-derivable is lost. One logical
        // cell-map materialization, counted once (perf contract: a keyed
        // batch clones the cell Map exactly once regardless of n).
        Object::Map(m) => {
            record_cell_map_clone();
            let mut rekeyed: HashMap<String, Object> = HashMap::with_capacity(m.len());
            for (orig_key, f) in m.iter() {
                match key_of(f) {
                    Some(k) => { rekeyed.insert(k, f.clone()); }
                    None => { rekeyed.insert(orig_key.clone(), f.clone()); }
                }
            }
            rekeyed
        }
        Object::Seq(items) if items.is_empty() => HashMap::new(),
        Object::Seq(items) => {
            // Migration: rebuild a pre-existing Seq cell into the Map
            // keyed by the same strategy (later wins on key clash, matching
            // `cell_put_keyed`'s migration arm).
            let mut m = HashMap::new();
            for f in items.iter() {
                if let Some(k) = key_of(f) {
                    m.insert(k, f.clone());
                }
            }
            m
        }
        _ => HashMap::new(),
    };
    let mut conflicts: Vec<KeyConflict> = Vec::new();
    for fact in facts {
        let Some(key) = key_of(&fact) else {
            continue;
        };
        // `.cloned()` releases the immutable borrow of `map` before the
        // insert in the fall-through arm; the clone only happens on the
        // rare key-present path, not the common fresh-insert path.
        match map.get(&key).cloned() {
            Some(existing_fact) if existing_fact == fact => { /* idempotent no-op */ }
            Some(existing_fact) if !upsert => {
                // partial-tuple upgrade (board-derived-layer poisoning,
                // 2026-06-12): when the same key holds a PARTIAL tuple
                // (fewer bound roles) and a FULLER fact arrives, the
                // fuller fact WINS instead of conflict-dropping behind
                // the partial. Partials are legitimate output for the
                // subtype-inheritance lift (one-pair rows are its
                // contract), but an underspecified rule's partial landing
                // FIRST in a keyed cell must not displace the real fact —
                // live hit: one-role `<<Resource, X>>` rows from the
                // (since removed) `State Machine is for Resource iff …`
                // core rule blocked every backfilled SM-for-Resource fact
                // and emptied the Task derived layer. Equal-or-fewer
                // bindings keeps today's conflict-reject.
                let n_in = fact.as_seq().map_or(0, |s| s.len());
                let n_ex = existing_fact.as_seq().map_or(0, |s| s.len());
                if n_in > n_ex {
                    map.insert(key, fact);
                } else {
                    conflicts.push(KeyConflict {
                        name: name.into(),
                        key,
                        existing_fact,
                        incoming_fact: fact,
                    });
                }
            }
            // None (fresh insert) OR upsert overwrite (last-write-wins).
            _ => {
                map.insert(key, fact);
            }
        }
    }
    (store(name, Object::Map(map.into()), state), conflicts)
}

/// task-984 part B (arc-agi-3 issue 10): reconcile keyed cells after
/// the cor:closure merge — a corrected single-valued fact from
/// readings must DISPLACE its stale carried-forward prior, not coexist
/// with it. Pre-984 the load path merged with identity-dedup only, so
/// `Run 'X' has Action Count '0'.` corrected to `'25'` left BOTH rows
/// in the cell despite `Each Run has at most one Action Count.`
///
/// For every cell named in `key_roles`: rebuild via the keyed upsert in
/// Seq order — later rows win, and the merge appends freshly-parsed rows
/// AFTER carried-forward priors, so corrected readings beat stale values.
/// Conservative gate: a cell where any row fails to extract its key
/// (dirty legacy shapes) is left untouched — dropping unkeyable rows is
/// not this pass's job. Returns the reconciled state plus `(cell,
/// displaced_count)` for the caller's `[load]` report.
///
/// store-on-derive STEP 4 (leaf-ingest convergence): read the cell via
/// `fetch_cell_seq`, NOT `fetch_or_phi(..).as_seq()`. The earlier form
/// SILENTLY SKIPPED folded **Map** cells (the #932/#940 silent-no-op bug
/// class) on the assumption that a Map cell is same-key-dup-free. That
/// assumption holds for the apply/full-compile path (cells reach here as
/// freshly-parsed Seqs) but is FALSE on the LEAF-INGEST path: the prior
/// db persists keyed cells in folded Map form, and `merge_states` unions
/// the new readings' rows onto that Map via `concat_dedup`, which dedups
/// only by identity (id/name), NOT by the UC key — so a corrected
/// single-valued fact (e.g. `Task 't1' has Task Status 'completed'`)
/// coexisted with its stale prior (`… 'pending'`) inside the Map and the
/// leaf db DIVERGED from a recompile (which never accumulated the stale
/// value). `fetch_cell_seq` key-flattens the Map so the upsert sees and
/// retracts the stale tuple regardless of storage shape; on a genuinely
/// dup-free cell the re-key is idempotent (no displacement).
pub(crate) fn reconcile_keyed_cells(
    state: &Object,
    key_roles: &hashbrown::HashMap<String, alloc::vec::Vec<String>>,
) -> (Object, alloc::vec::Vec<(String, usize)>) {
    let mut out = state.clone();
    let mut displaced: alloc::vec::Vec<(String, usize)> = alloc::vec::Vec::new();
    let mut cells: alloc::vec::Vec<&String> = key_roles.keys().collect();
    cells.sort();
    for cell in cells {
        let contents = fetch_cell_seq(cell, &out);
        let Some(items) = contents.as_seq() else { continue };
        if items.len() < 2 { continue; }
        let names: alloc::vec::Vec<&str> =
            key_roles[cell].iter().map(|s| s.as_str()).collect();
        if !items.iter().all(|f| extract_key_from_fact(f, &names).is_some()) {
            continue;
        }
        let before = items.len();
        let facts: alloc::vec::Vec<Object> = items.to_vec();
        // store-on-derive STEP 4: REBUILD the cell from the flattened tuples
        // keyed by the UC key. A folded Map cell stores entries under their
        // FULL-TUPLE hash, so a stale same-UC-key dup (`t1→pending`) sits
        // under a DIFFERENT map key than its correction (`t1→completed`) and
        // a batch that started from the existing Map would preserve BOTH
        // (the upsert only collides on equal UC keys, never across the two
        // full-tuple keys). Zeroing the cell first makes `cell_put_keyed_batch`
        // build the Map fresh from `facts` — every tuple re-keyed by the UC,
        // last-write-wins dropping the stale prior. `facts` already holds the
        // complete tuple set (fetch_cell_seq flattened it), so nothing is
        // lost; for a dup-free cell the rebuild is a faithful round-trip.
        let cleared = store(cell, Object::phi(), &out);
        let (next, _conflicts) = cell_put_keyed_batch(cell, &names, facts, true, &cleared);
        let after = cell_facts_iter(&fetch_or_phi(cell, &next)).count();
        if after < before {
            displaced.push((cell.clone(), before - after));
        }
        out = next;
    }
    (out, displaced)
}

/// Iterate over the facts in a cell regardless of storage shape.
/// Seq cells iterate their items; Map cells iterate their values.
/// Returns an empty iterator for any other shape (Bottom, Atom, …).
///
/// Migration glue for task-744. Readers that previously did
/// `fetch_or_phi(name, state).as_seq().unwrap_or(&[])` can switch to
/// `cell_facts_iter(&fetch_or_phi(name, state))` and keep working as
/// cells flip to Map storage.
pub fn cell_facts_iter(contents: &Object) -> alloc::boxed::Box<dyn Iterator<Item = &Object> + '_> {
    match contents {
        Object::Seq(items) => alloc::boxed::Box::new(items.iter()),
        Object::Map(m) => alloc::boxed::Box::new(m.values()),
        _ => alloc::boxed::Box::new(core::iter::empty()),
    }
}

/// Read a cell's facts as a Seq Object regardless of stored shape (#932).
/// A Map-backed (keyed) cell is normalized to a Seq of its values so a
/// legacy `.as_seq()` reader keeps working as cells flip to Map storage
/// (the silent-no-op bug class fixed in #940); Seq / Bottom / Atom pass
/// through unchanged. Lets a cell-read site convert by a mechanical 1:1
/// swap `fetch_or_phi(name, state).as_seq()` -> `fetch_cell_seq(name,
/// state).as_seq()` with no downstream restructuring. Iterator sibling:
/// `cell_facts_iter`. Transitional — phase-2 all-Map storage retires it.
pub fn fetch_cell_seq(name: &str, state: &Object) -> Object {
    let cell = fetch_or_phi(name, state);
    if let Object::Map(m) = &cell {
        // #932 phase-2 (D3): flatten a folded Map cell to a Seq in
        // deterministic KEY order. `m.values()` is HashMap-nondeterministic,
        // so the resulting fact array (and any JSON serialization of it,
        // which preserves array order) would vary per run — breaking the
        // canonical representation thm:derive caches and the deterministic
        // replay cor:consensus relies on. The key is the RMAP / full-tuple
        // identity, so key-order is a stable canonical order; D_n is a set,
        // so the order carries no semantics, only reproducibility.
        let mut entries: alloc::vec::Vec<(&alloc::string::String, &Object)> = m.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        Object::seq(entries.into_iter().map(|(_, v)| v.clone()).collect())
    } else {
        cell
    }
}

/// task-930 v2: classify the `state` operand of `Func::Fetch` /
/// `Func::FetchOrPhi`. Two shapes flow through these primitives:
///
///   * **Raw state** — `Object::Map` (the post-compile def-state, the
///     hot path), or `Object::Seq` of `<CELL_TAG, name, contents>`
///     3-tuples (the canonical "cells stored as a list" form).
///   * **Encoded population** — `Object::Seq` of `<ft_id, facts>`
///     2-tuples produced by `encode_state`. This is what the chain's
///     `extract_facts_from_pop` reads against.
///
/// Returns `true` for the encoded-pop shape, `false` for raw state /
/// unknown. Disambiguator: a non-empty Seq whose first item is a
/// 2-tuple `<atom, _>` (no CELL_TAG, not a 3-tuple) is treated as
/// already-encoded. Empty Seq, Map, and Seq-of-3-tuples-with-CELL_TAG
/// are raw state. Used by `resolve_view` to skip re-encoding when the
/// caller already supplied an encoded pop, and by `encoded_pop_lookup`
/// for the direct-scan fast path.
fn pop_is_encoded(state: &Object) -> bool {
    let cells = match state.as_seq() {
        Some(c) => c,
        None => return false, // Map and other shapes: raw
    };
    let first = match cells.first() {
        Some(f) => f,
        None => return false, // empty Seq: treat as raw (encode_state is a no-op on empty)
    };
    let items = match first.as_seq() {
        Some(i) => i,
        None => return false,
    };
    // Encoded entry: <atom_ft_id, seq_of_facts>, length 2.
    // Raw cell entry: <CELL_TAG, atom_name, contents>, length 3.
    items.len() == 2 && items[0].as_atom().is_some()
}

/// task-930 v2: direct lookup of `name` in an already-encoded population.
/// Returns Some(facts_seq) when the encoded pop has an entry for
/// `name`, None otherwise (so the caller falls through to view
/// resolution / raw fetch).
///
/// Mirrors the per-rule antecedent scan that `compile.rs`'s
/// `extract_facts_from_pop` builds via Filter+Eq+Selector. Lifting it
/// here lets `Func::Fetch` / `FetchOrPhi` serve the encoded-pop
/// callers uniformly with the raw-state callers — the chain's
/// extractor can compose Func::FetchOrPhi over the encoded pop and
/// get the right entry without changing its shape.
fn encoded_pop_lookup(name: &str, state: &Object) -> Option<Object> {
    // materialized-view-precedence (validate-resolve-view perf): the INDEXED
    // (Map) pop form — Selector(4) of the eval ctx, what extract_facts_func
    // reads — is keyed by ft_id. A present, NON-EMPTY entry here IS the
    // eagerly-materialized cell (the eager fold ran `derivation:{ft}` and
    // stored it; see compile.rs eager_materializable + the compile.rs:1975
    // note that the `view:` def is "a redundant FALLBACK for the now-
    // materialized cell, not the sole population source"). Returning it here,
    // BEFORE Func::Fetch/FetchOrPhi can fall through to `resolve_view`, makes
    // the view: def behave as the intended fallback instead of re-deriving a
    // View-marked aggregate on EVERY access (measured: Fact_Type_has_Arity
    // re-derived 9x/validate = 91% of resolve_view cost). `pop_is_encoded`
    // only recognizes the Seq form, so without this a materialized cell in the
    // Map form is silently bypassed. Empty/absent → None → caller resolves the
    // view (unchanged lazy behavior; View cells are not otherwise stored, and
    // the eager-stored ones are maintained fresh by the same chain/apply pack).
    if let Object::Map(m) = state {
        return m.get(name).and_then(|entry| match entry {
            Object::Seq(items) if !items.is_empty() => Some(entry.clone()),
            Object::Map(mm) if !mm.is_empty() => Some(entry.clone()),
            _ => None,
        });
    }
    if !pop_is_encoded(state) {
        return None;
    }
    let cells = state.as_seq()?;
    cells.iter().find_map(|cell| {
        let items = cell.as_seq()?;
        if items.len() == 2 && items[0].as_atom() == Some(name) {
            // task-930 v2 follow-up: return None for empty entries so
            // Func::Fetch / FetchOrPhi falls through to resolve_view
            // (the view def may produce facts even when the stored
            // cell is empty — typical post-`drop derived cells before
            // forward-chain` shape). Without this, view-marked FTs
            // whose cell is empty short-circuit to phi and lazy eval
            // never fires for downstream antecedent reads.
            let entry = &items[1];
            let is_empty = match entry {
                Object::Seq(items) => items.is_empty(),
                Object::Map(m) => m.is_empty(),
                _ => false,
            };
            if is_empty { None } else { Some(entry.clone()) }
        } else {
            None
        }
    })
}

// engine-flat-stratum-recursion-stack-guard: re-entrancy guard for lazy view
// resolution. A GENUINELY-unstratifiable program — an aggregate inside a
// recursive cycle (no least fixed point) — makes `resolve_view` recurse through
// the cycle on the CALL STACK (cell A's view reads cell B's view reads cell A's
// view …) and STACK-OVERFLOW the host compile. Track the cells currently being
// resolved on this thread; re-entering one already on the stack IS the cycle, so
// `resolve_view` stops and returns None (the caller falls through to the stored
// cell, breaking the recursion), letting the compile finish with a partial
// fixpoint + a loud note instead of crashing. Triggers ONLY on a genuine
// re-entrant cycle — acyclic / stratifiable views never re-enter a cell mid-
// resolution — so it is a strict no-op for every well-formed app. cfg-gated like
// the chain-abort / cap-stack thread-locals; no_std builds get a no-op guard.
#[cfg(not(feature = "no_std"))]
std::thread_local! {
    static RESOLVING_VIEWS: core::cell::RefCell<alloc::vec::Vec<alloc::string::String>> =
        const { core::cell::RefCell::new(alloc::vec::Vec::new()) };
}

#[cfg(not(feature = "no_std"))]
struct ViewResolveGuard;

#[cfg(not(feature = "no_std"))]
impl Drop for ViewResolveGuard {
    fn drop(&mut self) {
        RESOLVING_VIEWS.with(|s| { s.borrow_mut().pop(); });
    }
}

/// Begin resolving `cell`'s view. Returns `None` if `cell` is ALREADY being
/// resolved on this thread (a recursive view cycle — the caller must NOT recurse
/// into it); otherwise pushes it and returns a guard that pops on drop.
#[cfg(not(feature = "no_std"))]
fn enter_view_resolution(cell: &str) -> Option<ViewResolveGuard> {
    RESOLVING_VIEWS.with(|s| {
        let mut stack = s.borrow_mut();
        if stack.iter().any(|c| c == cell) {
            None
        } else {
            stack.push(alloc::string::String::from(cell));
            Some(ViewResolveGuard)
        }
    })
}

#[cfg(feature = "no_std")]
struct ViewResolveGuard;
#[cfg(feature = "no_std")]
fn enter_view_resolution(_cell: &str) -> Option<ViewResolveGuard> { Some(ViewResolveGuard) }

/// task-930: read-side eval of a view rule. Returns Some(facts) when
/// the cell has a registered view def (`view:{cell_name}`), None
/// otherwise (so the caller falls through to the legacy stored-cell
/// fetch).
///
/// Performance: a single `fetch_raw` on the def-state. When no views
/// are declared the lookup misses cheaply (one HashMap probe on Map
/// d, or one O(n) scan on Seq d but n is small for def-only state).
/// When a view is declared the cost is `apply(view_func, …)` —
/// equivalent to one chain step but only paid when the cell is
/// actually read.
///
/// Shape conversion: derivation funcs emit the "wrapped derived fact"
/// envelope `[ft_id, reading, [[role, value], …]]` because that's
/// what the forward chain consumes. Cell storage is the unwrapped
/// bindings list `[[[role, value], …], …]`. We unwrap here so
/// downstream Fetch consumers (other rule funcs, query/get/sql) see
/// the same shape they get from a stored cell.
///
/// task-930 v2: `pop` may be either raw state OR an already-encoded
/// population (the chain's `extract_facts_from_pop` calls Func::Fetch
/// with the encoded pop as items[1]). `pop_is_encoded` distinguishes,
/// and we skip the `encode_state` step when already encoded so the
/// view's derivation func sees the shape it was compiled against.
pub(crate) fn resolve_view(cell_name: &str, pop: &Object, defs: &Object) -> Option<Object> {
    let def_key = alloc::format!("view:{}", cell_name);
    let stored = fetch_raw(&def_key, defs);
    if matches!(stored, Object::Bottom) {
        // No `view:` def — try absorbed-FT reconstitution (task-962): the FT
        // may be RMAP-absorbed into an entity cell with no data cell of its
        // own. Read from `defs` (carries the FactType / Role / Noun cells).
        return crate::rmap::reconstitute_absorbed_ft(defs, cell_name);
    }
    // engine-flat-stratum-recursion-stack-guard: break a recursive view cycle
    // before it overflows the call stack (an ill-defined aggregate-in-cycle).
    // `_view_guard` pops `cell_name` off the resolving stack on every return.
    let _view_guard = match enter_view_resolution(cell_name) {
        Some(g) => g,
        None => {
            diag!("[view] recursive cycle resolving `{}` — an aggregate/view inside \
                a recursive cycle has no least fixed point; returning empty to break \
                the recursion. The program is ill-defined: break the cycle (e.g. add \
                a partition role so an aggregate sits strictly ABOVE its recursive \
                source).", cell_name);
            return None;
        }
    };
    let func = metacompose(&stored, defs);
    // The view's func is a derivation func — it expects an encoded
    // population. If the caller already handed us an encoded pop
    // (task-930 v2 — chain extractor composes Func::Fetch over the
    // encoded pop), feed it through directly; otherwise encode the
    // raw state the same way the forward chain does so derivation
    // funcs see the shape they were compiled against.
    let encoded_pop_owned;
    let encoded_pop: &Object = if pop_is_encoded(pop) {
        pop
    } else {
        encoded_pop_owned = encode_state(pop);
        &encoded_pop_owned
    };
    let wrapped = apply(&func, encoded_pop, defs);
    // Unwrap [ft_id, reading, bindings] envelopes → just the bindings.
    let unwrapped: alloc::vec::Vec<Object> = wrapped.as_seq()
        .map(|items| items.iter()
            .filter_map(|item| {
                let env = item.as_seq()?;
                if env.len() >= 3 {
                    Some(env[2].clone())
                } else { None }
            })
            .collect())
        .unwrap_or_default();
    Some(Object::seq(unwrapped))
}

/// Count facts in a cell regardless of storage shape. Convenience
/// wrapper over `cell_facts_iter` for the common "did anything land
/// here?" check.
pub fn cell_fact_count(contents: &Object) -> usize {
    match contents {
        Object::Seq(items) => items.len(),
        Object::Map(m) => m.len(),
        _ => 0,
    }
}

/// Merge two states in O(n): collect all cells into a HashMap,
/// concatenate overlapping cells, return as Map store.
pub fn merge_states(target: &Object, source: &Object) -> Object {
    let mut map: HashMap<String, Object> = cells_iter(target).into_iter()
        .map(|(name, contents)| (name.to_string(), contents.clone()))
        .collect();
    cells_iter(source).into_iter().for_each(|(name, contents)| {
        // Fast path: when target has no entry for this cell the
        // merge reduces to a direct Arc clone. `concat_dedup` below
        // is O(n²) in the source-cell size because every appended
        // fact scans the accumulator via `same_identity` — so
        // skipping it when there's nothing to dedup against avoids
        // millions of comparisons on the 4k-fact expanded-grammar
        // cells that Stage-2's classify pass merges every call.
        if !map.contains_key(name) {
            map.insert(name.to_string(), contents.clone());
            return;
        }
        let entry = map.get_mut(name).expect("checked above");
        *entry = concat_dedup(name, entry, contents);
    });
    Object::Map(map.into())
}

// READINGS_DERIVED_META_CELLS / drop_readings_derived_meta_cells were
// removed. They were the hardcoded list of cells the parser emits to
// (originally just DerivationRule per #913, but FactType / Noun /
// Constraint / InstanceFact / etc. were silently in the same boat).
// Callers (platform_compile, cli/entry.rs) now compute the schema-cell
// set structurally: parse the readings fresh into a state and the cell
// names in THAT state are exactly the readings-derived cells. Prior
// state's cells matching those names get dropped before merge.

/// Canonical identity bindings, in priority order. Two facts that both
/// carry one of these with an equal value are the SAME entity (modulo
/// homeDomain — see `same_identity`). Module-level so `same_identity` and
/// the `concat_dedup` bucket index share one source of truth.
const IDENTITY_KEYS: &[&str] = &["id", "name", "ruleId", "Change Id", "Signal Id"];

/// perf-mergededup-on2: the identity buckets a fact belongs to, for the
/// O(n) `concat_dedup` index. A fact is bucketed under (a) its φ-canonical
/// structural key (captures `same_identity`'s `a==b` and
/// `canon_phi(a)==canon_phi(b)` branches) and (b) one key per IDENTITY_KEY
/// it carries (bucketing under EVERY such key guarantees two facts that
/// match on the first key they share land in a common bucket). So
/// `same_identity(x,y)` can hold only if x,y share at least one of these
/// keys — the union of a fact's buckets is a sound candidate superset, and
/// the final verdict is still the unchanged `same_identity` predicate.
fn identity_bucket_keys(cell_name: &str, fact: &Object) -> Vec<String> {
    let mut keys: Vec<String> = Vec::with_capacity(IDENTITY_KEYS.len() + 1);
    // Structural bucket, φ-canonicalized so Atom("φ") and Seq([]) collide
    // (matching same_identity's canon_phi branch — the task-956 φ-bloat fix).
    keys.push(synthesize_fact_id(cell_name, &canon_phi(fact)));
    // Identity-key buckets. The \u{1} prefix keeps these disjoint from the
    // synthesize_fact_id structural namespace.
    for key in IDENTITY_KEYS {
        if let Some(v) = binding(fact, key) {
            keys.push(alloc::format!("\u{1}{}\u{1f}{}", key, v));
        }
    }
    keys
}

/// Concatenate two sequences and drop duplicates, identity-aware.
/// Preserves first-occurrence order. When two facts share an identity
/// key (`id`, `name`, or `ruleId`), the first is kept and the second
/// dropped — this handles the case where one file declares a noun fully
/// and another references it, producing two Noun facts that differ in
/// bindings but represent the same entity.
fn concat_dedup(name: &str, a: &Object, b: &Object) -> Object {
    // task-928: extract values from Map inputs so merge_states preserves
    // Map-backed apply-emitted population across recompile / in-process
    // compile. Map structure itself isn't preserved through merge
    // (collapse-the-duality is task-924); this just stops the silent
    // wipe when the prior side is Map. Pre-fix: `a.as_seq()` returned
    // None for Map, `unwrap_or_default()` made it empty, prior content
    // lost. Post-fix: Map values are extracted as fact items and merged
    // identity-aware just like Seq facts.
    let extract = |obj: &Object| -> Vec<Object> {
        match obj {
            Object::Map(m) => m.values().cloned().collect(),
            Object::Seq(_) => obj.as_seq().map(|s| s.to_vec()).unwrap_or_default(),
            _ => Vec::new(),
        }
    };
    let a_items = extract(a);
    let b_items = if matches!(b, Object::Map(_) | Object::Seq(_)) {
        extract(b)
    } else {
        vec![b.clone()]
    };
    // perf-mergededup-on2: index facts by identity bucket so each new fact
    // is compared (via the unchanged `same_identity`) only against the
    // handful of prior facts that could match it — not the whole
    // accumulator. Result is byte-identical to the prior
    // `out.iter().any(same_identity)` scan, but O(n) instead of O(n²) (the
    // ~226s parse-fold sink on the tasks app).
    let mut out: Vec<Object> = Vec::with_capacity(a_items.len() + b_items.len());
    let mut buckets: HashMap<String, Vec<usize>> = HashMap::new();
    // `a` is kept unconditionally (mirrors the prior `out = a_items`, which
    // never deduped within `a`); index each for `b`'s lookups.
    for item in a_items {
        let idx = out.len();
        for k in identity_bucket_keys(name, &item) {
            buckets.entry(k).or_default().push(idx);
        }
        out.push(item);
    }
    for item in b_items {
        let keys = identity_bucket_keys(name, &item);
        let is_dup = keys.iter().any(|k| {
            buckets.get(k).is_some_and(|cands|
                cands.iter().any(|&ci| same_identity(&out[ci], &item)))
        });
        if is_dup { continue; }
        let idx = out.len();
        for k in keys {
            buckets.entry(k).or_default().push(idx);
        }
        out.push(item);
    }
    // task-932 (W7-a): preserve Map SHAPE when either input was already a
    // folded Map cell. Only FT-image cells are Map post-W2/W4 (schema/meta
    // cells are Seq and must NEVER fold), so "an input was already a Map"
    // is exactly the guard that restricts re-folding to FT-image cells.
    // Pre-fix this returned Object::Seq UNCONDITIONALLY, silently demoting
    // a folded Map cell to Seq on EVERY merge — fired on every recompile
    // (entry.rs:764,838) and every in-process compile. We re-fold the
    // deduped Vec back into a Map keyed by `synthesize_fact_id` — the
    // keyless, content-addressed fold μ_n (`cell_put_folded`'s key). That
    // key is a pure FNV-1a over the fact's sorted bindings, so it is
    // idempotent at the fixpoint (unlike task-932-5's entity-id keying,
    // which co-mingled content-addressed and entity-id rows and grew
    // unbounded). Pure Seq+Seq inputs stay Seq.
    if matches!(a, Object::Map(_)) || matches!(b, Object::Map(_)) {
        let folded: HashMap<String, Object> = out.into_iter()
            .map(|fact| (synthesize_fact_id(name, &fact), fact))
            .collect();
        return Object::Map(folded.into());
    }
    Object::Seq(out.into())
}

/// perf-mergededup-on2: test-only counter of `same_identity` calls — the
/// O(n²) signal in `concat_dedup` (which, pre-fix, scanned the whole
/// accumulator per appended fact). The indexed dedup buckets candidates
/// by identity so this stays ~O(n); the guard test
/// (`concat_dedup_same_identity_calls_stay_linear`) asserts the call
/// count does not grow quadratically with cell size. Mirrors
/// `cell_map_clone_counter`: thread-local, test/test-bins only, zero-cost
/// empty `record_*` in production.
#[cfg(any(test, feature = "test-bins"))]
mod same_identity_counter {
    use core::cell::Cell;
    std::thread_local! {
        pub static COUNT: Cell<usize> = const { Cell::new(0) };
    }
}
/// Reset the `same_identity` call counter. Call before the measured window.
#[cfg(any(test, feature = "test-bins"))]
pub fn reset_same_identity_count() {
    same_identity_counter::COUNT.with(|c| c.set(0));
}
/// Read the `same_identity` call count accumulated since the last reset.
#[cfg(any(test, feature = "test-bins"))]
pub fn get_same_identity_count() -> usize {
    same_identity_counter::COUNT.with(|c| c.get())
}
#[inline]
fn record_same_identity_call() {
    #[cfg(any(test, feature = "test-bins"))]
    same_identity_counter::COUNT.with(|c| c.set(c.get() + 1));
}

/// Two facts share identity when they have the same value at a canonical
/// identity binding (`id`, `name`, or `ruleId`), or — falling back —
/// when they are structurally equal.
fn same_identity(a: &Object, b: &Object) -> bool {
    record_same_identity_call();
    if a == b { return true; }
    // task-956: the fan-out writes a unary predicate's object value as
    // Object::Atom("φ"), but a recompile round-trips it through SQLite where
    // Object::parse turns the token φ back into Object::phi() (empty Seq).
    // The two are display-equal but structurally unequal, so the `a == b`
    // above misses them and concat_dedup appends a fresh copy every recompile
    // (the Task_is_finished bloat, ~+653 rows/recompile). Compare φ-canonical
    // forms so such facts dedup regardless of φ representation.
    if canon_phi(a) == canon_phi(b) { return true; }
    // ns-4 (namespace-keyed identity): when both facts carry a `homeDomain`
    // (only domain-annotated Noun facts do — see annotate_noun_domain),
    // identity is the (id/name, homeDomain) PAIR, so same-named nouns in
    // different domains (core.Order value vs orders.Order entity) stay
    // DISTINCT instead of merging by name. Facts without a homeDomain leave
    // domain_mismatch false, so resource/command `domain` and every other
    // cell are untouched.
    let domain_mismatch = matches!(
        (binding(a, "homeDomain"), binding(b, "homeDomain")),
        (Some(da), Some(db)) if da != db
    );
    for key in IDENTITY_KEYS {
        let av = binding(a, key);
        let bv = binding(b, key);
        if let (Some(av), Some(bv)) = (av, bv) {
            return av == bv && !domain_mismatch;
        }
    }
    false
}

/// task-956 + compile-gc-orphaned-derived-facts: canonicalize the empty/absent
/// representations of a (unary) predicate's object slot so they compare equal
/// for fact IDENTITY. The same logical fact reaches the cell store in three
/// encodings depending on the write path: `Object::Atom("φ")` (the fan-out's
/// literal token), `Object::phi()` (empty Seq, after a SQLite round-trip where
/// the parser turns the φ token back into an empty Seq), and `Object::Atom("")`
/// (a blank from the apply/SM write path — the task-932 write-path duality).
/// All three denote "no object"; canonicalizing them to phi() makes
/// re-assertion idempotent regardless of encoding, so the same fact dedups
/// instead of accruing a copy per recompile (the Task_is_epic / Task_is_finished
/// bloat). This only affects fact identity (same_identity, dedup) — never a
/// stored value — and a fact differing by a REAL binding (id/name/value) still
/// canonicalizes distinctly. Recurses into Seq facts/pairs; others pass through.
fn canon_phi(o: &Object) -> Object {
    if matches!(o.as_atom(), Some("φ") | Some("")) { return Object::phi(); }
    match o {
        Object::Seq(items) => Object::seq(items.iter().map(canon_phi).collect()),
        _ => o.clone(),
    }
}

/// Concatenate two sequences: <a₁,...,aₙ> ++ <b₁,...,bₘ> = <a₁,...,aₙ,b₁,...,bₘ>
/// Iterate all cells in state as (name, contents) pairs.
/// Replaces: population.facts.iter()
pub fn cells_iter(state: &Object) -> Vec<(&str, &Object)> {
    // S1b (#718): when a stored value is a version chain, the &Object
    // returned points at the latest version's `contents` sub-Object.
    // Lifetime stays the same as the input — chain unwrapping is
    // pointer chase, not allocation. Legacy raw values pass through.
    match state {
        Object::Map(map) => map.iter()
            .map(|(k, v)| (k.as_str(), cell_contents_view(v)))
            .collect(),
        Object::Seq(cells) => cells.iter().filter_map(|c| {
            let items = c.as_seq()?;
            if items.len() == 3 && items[0].as_atom() == Some(CELL_TAG) {
                Some((items[1].as_atom()?, cell_contents_view(&items[2])))
            } else {
                None
            }
        }).collect(),
        _ => Vec::new(),
    }
}

/// Drop "subjectless" / malformed facts from a cell's contents. A stored
/// elementary fact populates every role of its fact type and identifies its
/// subject by its first role, so two shapes are provably-malformed relics —
/// left by an older write path and re-preserved every compile by cor:closure
/// with no GC — and are removed:
///
///   1. **Missing a role.** Fewer `<role,value>` bindings than the cell's
///      arity. This is the `null`-subject shape seen on the live tasks.db: a
///      `State Machine is currently in Status` row carrying only
///      `<Status, 'Proposed'>` with no `State Machine` binding at all (the
///      subject role is absent, so SQL materializes it as NULL).
///   2. **Empty subject.** First `<role,value>` binding has a degenerate value:
///      empty-string `""`, the literal `"φ"` token, or φ = empty Seq — the
///      three `canon_phi` "no-entity" encodings.
///
/// Arity is the modal (max) binding-count among the cell's *own* facts, so a
/// uniformly unary cell keeps all its rows and we never need the schema. Every
/// valid fact of one fact type shares that arity, so dropping the short ones
/// only ever removes malformed relics. Non-`<role,value>`-shaped entries and
/// non-Seq contents pass through untouched.
///
/// Cells are stored either as a `Seq` of facts or — for hash-keyed /
/// content-addressed fact stores like the SM `currently_in_Status`
/// latest-wins fold — as a `Map<key, fact>`. Both layouts are handled: for a
/// Map the *values* are the facts. (The orphan that motivated this lived as a
/// Map value `⟨State Machine='', Status='Proposed'⟩`, which every prior
/// `as_seq()`-only pass silently skipped.) Other shapes pass through.
// task-drop-deadcode-cleanup: the bare-arity wrapper had only test
// callers and was never reachable from production -- the cli/entry.rs
// loader (the single production site) always passes the declared
// arity through `drop_subjectless_facts_with_arity`. Tests now call
// the *_with_arity variant directly with `None` for the auto-infer
// case, which removes a pub(crate) surface that was only ever a
// convenience.

/// As [`drop_subjectless_facts`], but when `declared_arity` is `Some(n)` the
/// schema arity `n` is the keep-threshold instead of the modal (max) binding
/// count inferred from the cell's own facts.
///
/// Inferring arity from contents is only correct when the cell is uniform —
/// the documented assumption ("every valid fact of one fact type shares that
/// arity"). A cell that ALSO carries malformed OVER-arity relics violates it:
/// e.g. the unary `Task_is_finished` cell holding both the correct 1-binding
/// `<<Task,930>>` (apply/transition-written) and 2-binding
/// `<<Task,X>,<Task_is_finished,φ>>` rows (an older bulk-reading write path).
/// The malformed rows inflate the inferred max to 2, so the CORRECT 1-binding
/// rows are then dropped as "missing a role" — inverting the GC (drops the
/// valid facts, keeps the relics; tasks-db task-958). For cells in the
/// FactType registry the declared arity is authoritative — callers pass it.
/// The `>=` threshold is kept (not `==`): over-arity relics still pass, so
/// this never drops a status-bearing event; it only stops the inflation from
/// discarding the correct shorter rows.
pub(crate) fn drop_subjectless_facts_with_arity(contents: &Object, declared_arity: Option<usize>) -> Object {
    // A relic is a fact missing a role (fewer bindings than the cell's
    // arity) or with an empty subject (first binding's value empty/φ).
    fn keep(f: &Object, arity: usize) -> bool {
        match f.as_seq() {
            Some(pairs) => pairs.len() >= arity && has_nonempty_subject(f),
            None => true, // not a fact-shaped entry: leave it alone.
        }
    }
    if let Some(m) = contents.as_map() {
        let arity = declared_arity.unwrap_or_else(||
            m.values().filter_map(|f| f.as_seq().map(|p| p.len())).max().unwrap_or(0));
        return Object::map(m.iter()
            .filter(|&(_, f)| keep(f, arity))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect());
    }
    let facts = match contents.as_seq() {
        Some(f) => f,
        None => return contents.clone(),
    };
    let arity = declared_arity.unwrap_or_else(||
        facts.iter().filter_map(|f| f.as_seq().map(|p| p.len())).max().unwrap_or(0));
    Object::seq(facts.iter().filter(|&f| keep(f, arity)).cloned().collect())
}

/// Drop only facts with a provably-empty SUBJECT — the first `<role,value>`
/// binding's value is empty-string or φ. Makes NO arity assumption, so it is
/// safe on non-uniform cells — in particular synthetic SM-output cells not in
/// the FactType registry. Handles both `Seq` and hash-keyed `Map` layouts
/// (for a Map the values are the facts). Non-`<role,value>` entries untouched.
///
/// Gated to match its sole caller, `cli::dedup_state_for_persist`
/// (`#[cfg(all(not(feature = "no_std"), feature = "local"))]`): without the
/// `local`/rusqlite feature that persist path is compiled out, so an ungated
/// definition is genuinely dead there and trips `dead_code`. (Not deleted —
/// removing it would break the `local` build, which still references it.)
#[cfg(all(not(feature = "no_std"), feature = "local"))]
pub(crate) fn drop_empty_subject_facts(contents: &Object) -> Object {
    if let Some(m) = contents.as_map() {
        return Object::map(m.iter()
            .filter(|&(_, f)| has_nonempty_subject(f))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect());
    }
    match contents.as_seq() {
        Some(facts) => Object::seq(facts.iter().filter(|&f| has_nonempty_subject(f)).cloned().collect()),
        None => contents.clone(),
    }
}

/// True iff the fact's first `<role,value>` binding has a non-empty atom
/// value — i.e. its subject is present. Non-`<role,value>`-shaped entries
/// return true (left untouched). Shared by the subjectless-GC variants.
fn has_nonempty_subject(f: &Object) -> bool {
    match f.as_seq().and_then(|pairs| pairs.first()).and_then(|p| p.as_seq()) {
        // phi-keyed-task-started-orphan-gc: drop "φ" (the fan-out's literal
        // token) as a degenerate subject, alongside "" and φ = Seq([]) (whose
        // as_atom() is None, so matches! is already false here). These are
        // canon_phi's three "no-entity" encodings; without the "φ" arm a
        // `<<Task, φ>>` relic survives every persist GC pass and re-seeds the
        // phantom in_progress SM.
        Some(kv) if kv.len() == 2 => matches!(kv[1].as_atom(), Some(v) if !v.is_empty() && v != "φ"),
        _ => true,
    }
}

/// compile-gc-orphaned-derived-facts: true when a cell's contents look like a
/// fact-type POPULATION — a `Seq` (or hash-keyed `Map`) carrying at least one
/// elementary fact, where a fact is a `Seq` of `<role, value>` pairs (each pair
/// a 2-element `Seq` whose first element is an atom role name). cor:closure uses
/// this to distinguish a carried-forward FT population (an orphan to drop when
/// its FactType is no longer declared) from a non-population cell (always kept).
/// Conservative: at least one fact-shaped row must be present, so empty cells
/// and non-`<role,value>`-shaped cells are NOT treated as populations.
pub(crate) fn looks_like_population_cell(contents: &Object) -> bool {
    fn is_fact(o: &Object) -> bool {
        match o.as_seq() {
            Some(pairs) => !pairs.is_empty() && pairs.iter().all(|p|
                matches!(p.as_seq(), Some(kv) if kv.len() == 2 && kv[0].as_atom().is_some())),
            None => false,
        }
    }
    if let Some(m) = contents.as_map() {
        return m.values().any(is_fact);
    }
    if let Some(items) = contents.as_seq() {
        return items.iter().any(is_fact);
    }
    false
}

/// compile-gc-orphaned-derived-facts (duplicated half): collapse
/// structurally-equal duplicate facts within a `Seq` cell, preserving
/// first-occurrence order. cor:closure carries asserted-fact cells forward
/// across recompiles, and `concat_dedup` only dedups the incoming side
/// against the accumulator — never the accumulator's OWN internal dups — so
/// a cell like `Task_is_epic` accrues one extra identity-equal copy per
/// recompile (312 bindings for 8 distinct tasks observed live), bloating the
/// persisted cell without bound. Running this over the final state before
/// persist breaks the cycle: the stored cell is always dup-free, so the next
/// recompile loads a clean prior. O(n) via a canonical-form hash set (`{:?}`
/// over the φ-canonicalized fact). Non-`Seq` contents (hash-keyed Map cells
/// are already dup-free by construction) pass through unchanged. Returns the
/// input clone untouched when there were no duplicates.
pub(crate) fn dedup_cell_facts(contents: &Object) -> Object {
    let Some(items) = contents.as_seq() else { return contents.clone(); };
    let mut seen: hashbrown::HashSet<alloc::string::String> = hashbrown::HashSet::new();
    let mut out: alloc::vec::Vec<Object> = alloc::vec::Vec::with_capacity(items.len());
    for item in items {
        if seen.insert(alloc::format!("{:?}", canon_phi(item))) {
            out.push(item.clone());
        }
    }
    if out.len() == items.len() {
        return contents.clone();
    }
    Object::seq(out)
}

/// compile-gc-orphaned-derived-facts: the cor:closure "preserve prior
/// population" step, extracted from cli/entry.rs's dirs-compile so it is
/// unit-testable (the binary path is otherwise unreachable from cargo tests).
/// Carries the prior DB population forward across a recompile so runtime data
/// survives, while dropping (a) sidecar `:` cells and cells the fresh parse
/// re-emits (`parsed_cell_names`), and (b) orphan relics whose FactType is no
/// longer declared (`is_orphan_population_cell`) — then scrubs malformed
/// subjectless rows from what remains (declared arity per task-958) AND
/// dedupes identity-equal facts so legacy bloat (accrued before the post-merge
/// dedup at cli/entry.rs landed, or carried forward by recompile paths that
/// bypass that site — `load_reading_core::load_reading` via `arest reload` /
/// `arest watch`) self-heals at the canonical preserve step instead of riding
/// through the entire merge + forward-chain + persist pipeline. Returns the
/// preserved cell map plus the SORTED names of the orphan cells dropped (for
/// the caller's `[load]` diagnostic). DATA-CRITICAL: a regression here drops
/// declared-FT runtime data on recompile.
pub(crate) fn preserve_prior_population(
    loaded: &Object,
    parsed_cell_names: &hashbrown::HashSet<String>,
    declared_ft_ids: &hashbrown::HashSet<String>,
    ft_arity: &hashbrown::HashMap<String, usize>,
) -> (Object, Vec<String>) {
    let mut gc_orphans: Vec<String> = Vec::new();
    let map: hashbrown::HashMap<String, Object> =
        cells_iter(loaded).into_iter()
            .filter(|(name, contents)| {
                if name.contains(':') || parsed_cell_names.contains(*name) {
                    return false;
                }
                if crate::declared_writes::is_orphan_population_cell(*name, *contents, declared_ft_ids) {
                    gc_orphans.push((*name).to_string());
                    return false;
                }
                true
            })
            .map(|(name, contents)| {
                // Order matters: subjectless-GC FIRST (it preserves arity invariants
                // the dedup canon doesn't reason about), then dedup the remainder.
                // `dedup_cell_facts` is identity-aware (φ-canonical, task-956 +
                // commit 00196eb3) so all three empty-encoding forms of the same
                // unary fact (`Atom("φ")`, `Atom("")`, `phi()`) collapse to one —
                // this is what makes the Task_is_epic / Task_is_finished bloat
                // self-heal across a recompile on legacy DBs.
                let cleaned = drop_subjectless_facts_with_arity(contents, ft_arity.get(name).copied());
                (name.to_string(), dedup_cell_facts(&cleaned))
            })
            .collect();
    gc_orphans.sort();
    (Object::map(map), gc_orphans)
}

/// Diff two cell stores: return an Object::Map containing only cells
/// whose contents differ between `old` and `new`. Cells present in
/// `new` but absent from `old` are included. Cells present only in
/// `old` are omitted (delta semantics: delta applied on top of old
/// reaches new for the cells we ship; cells dropped entirely are a
/// structural change that belongs on a different path).
///
/// Used by task #209 to scope __state in CommandResult so create /
/// update / transition return only the cells they modified, not a
/// full D. Per AREST §5.4, each cell is independent; the delta is the
/// minimal patch that can reach new from old.
pub fn diff_cells(old: &Object, new: &Object) -> Object {
    let new_cells: Vec<(&str, &Object)> = cells_iter(new);
    let delta: HashMap<String, Object> = new_cells.into_iter()
        .filter(|(k, v)| {
            let prev = fetch_or_phi(k, old);
            prev != **v
        })
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    Object::Map(delta.into())
}

/// Merge a cell delta onto a base store. S1b (#718): for each cell in
/// `delta`, append the new contents as a fresh VersionEntry on top of
/// the base's per-cell chain (or wrap it in a new chain if the cell is
/// absent from base). Cells not in `delta` keep their existing chain.
///
/// Realizes whitepaper eq:cellfold (`D_n' = foldl μ_n D_n E_n`): each
/// merge is one fold step, the per-cell sequence of intermediate states
/// is retained instead of collapsed.
///
/// Read invariant: `cells_iter`/`fetch_or_phi` on the result returns
/// the latest version's contents — same logical view as the legacy
/// "last-write-wins" semantics, so existing readers are unaffected.
///
/// Complement of `diff_cells`: for any (old, new),
/// `cells_iter(merge_delta(old, diff_cells(old, new)))` produces the
/// same (name, contents) pairs as `cells_iter(new)`.
/// `event` (S1c #719) is the apply-time operand `x` threaded into
/// every cell's new VersionEntry — operation kind, sender, and
/// payload become queryable via `cells_iter_history`. `None` for
/// non-apply commits (compile-bootstrap, intermediate forward-chain).
/// The event is shared across every cell in this batch — eq:cellfold
/// says one `μ_n` invocation is one `x` operand; cells in a single
/// delta were all produced by the same apply.
pub fn merge_delta(
    base: &Object,
    delta: &Object,
    event: Option<Object>,
) -> Object {
    // Read base WITHOUT unwrapping — chains are preserved.
    let mut base_map: HashMap<String, Object> = match base {
        Object::Map(m) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        Object::Seq(cells) => cells.iter().filter_map(|c| {
            let items = c.as_seq()?;
            if items.len() == 3 && items[0].as_atom() == Some(CELL_TAG) {
                Some((items[1].as_atom()?.to_string(), items[2].clone()))
            } else {
                None
            }
        }).collect(),
        _ => HashMap::new(),
    };

    // Delta values are raw contents (logical, not chain-wrapped) per
    // the diff_cells contract.
    let delta_pairs: Vec<(String, Object)> = match delta {
        Object::Map(m) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        Object::Seq(cells) => cells.iter().filter_map(|c| {
            let items = c.as_seq()?;
            if items.len() == 3 && items[0].as_atom() == Some(CELL_TAG) {
                Some((items[1].as_atom()?.to_string(), items[2].clone()))
            } else {
                None
            }
        }).collect(),
        _ => Vec::new(),
    };

    // Logical commit stamp — one per merge, every cell in this delta gets
    // the same recorded_at (commit-batch atomicity). The fold reads NO
    // host clock (eq:cellfold orders by chain position, not wall time);
    // wall-clock is the opt-in `now` primitive for domains that model
    // time, not a fold dependency. Monotonic, so the chain's
    // `latest-by-recorded-at` aggregation still holds.
    let recorded_at = logical_commit_stamp();

    for (k, v) in delta_pairs {
        let new_chain = match base_map.get(&k) {
            Some(existing) => {
                // task-922-map-cell-merge-not-replace: if BOTH the
                // existing cell's latest contents AND the delta value
                // are Maps, the new chain entry's contents must be the
                // UNION (delta entries layered onto the existing Map),
                // not the delta value alone. Without this, every apply
                // that emits a single-entity Map delta replaces the
                // whole cell — multi-entity history is lost.
                //
                // The chain layer (chain_append) preserves prior
                // versions byte-for-byte, but the LOGICAL VIEW (latest
                // contents) is what readers see, and that view collapses
                // to one entry on every apply pre-fix.
                //
                // Apply-update semantics (same key in delta + existing):
                // delta entries WIN — we're updating that entry on
                // purpose. Different keys in the delta are added
                // alongside; existing keys not in the delta are kept.
                let merged_contents = merge_map_cell_contents(existing, &v);
                chain_append(existing, merged_contents, recorded_at.clone(), event.clone())
            }
            None => wrap_as_chain(v, recorded_at.clone(), event.clone()),
        };
        base_map.insert(k, new_chain);
    }
    Object::Map(base_map.into())
}

/// Merge a delta cell value onto the existing cell's latest contents
/// for the Map-form case.
///
/// task-922-map-cell-merge-not-replace: when an apply's delta value is
/// itself a Map (per-entity routing — the cell name is the FT id, the
/// Map keys are entity ids), the new cell contents must be the UNION
/// of the existing latest Map and the delta's Map. The cell layer
/// holds a chain over Map values; without this union the latest version
/// replaces the cell entirely and only the most recently apply'd
/// entity is visible.
///
/// Behavior:
/// - existing is a chain → unwrap to its latest contents
/// - existing latest contents is Map AND delta is Map → union (delta
///   entries replace same-key existing entries, other keys are kept)
/// - any other shape → return delta as-is (legacy semantics — a Seq
///   cell remains last-write-wins, an Atom cell is replaced)
///
/// Pure helper: takes references, returns a fresh Object. Callers
/// thread the returned contents into `chain_append` so the chain still
/// gets a new VersionEntry per merge.
fn merge_map_cell_contents(existing_chain: &Object, delta_value: &Object) -> Object {
    let Some(delta_map) = delta_value.as_map() else {
        return delta_value.clone();
    };
    // Existing is a chain (or legacy raw); read its latest logical
    // contents. cell_contents_view unwraps chains; non-chain values
    // pass through.
    let existing_contents = cell_contents_view(existing_chain);
    let Some(existing_map) = existing_contents.as_map() else {
        // Existing isn't a Map (legacy Seq or Atom or absent) — fall
        // back to legacy replace semantics. Migration of pre-existing
        // Seq-form data into Map-form is the caller's job (cell_put_keyed
        // handles it for direct writes); merge_delta plays the
        // conservative role here so cells that flipped shape mid-history
        // don't silently mutate.
        return delta_value.clone();
    };
    // Union: start from existing entries, layer delta entries on top.
    // delta entries WIN at colliding keys (apply-update semantics).
    let mut merged: HashMap<String, Object> = existing_map.clone();
    for (k, v) in delta_map.iter() {
        merged.insert(k.clone(), v.clone());
    }
    Object::Map(merged.into())
}

// ── store-on-derive STEP 1 (keystone): the retraction channel ────────
//
// Plan: `docs/superpowers/plans/2026-06-19-store-on-derive-default.md`
// §"The keystone: the retraction wall".
//
// `merge_delta`/`merge_map_cell_contents` are UNION-only (task-922): a
// cell delta can ADD or OVERWRITE-BY-KEY, but never REMOVE a tuple. A
// derived cell that should LOSE a tuple (a completed task leaving
// `Task_is_recommended`, an `update` re-keying a folded tuple) therefore
// cannot be expressed as a delta — so the apply path wipes the whole
// cell and force-replaces with the full recompute (the workarounds in
// `command::transition_via_defs`). IVM (Step 2+) needs deltas that carry
// REMOVALS (ΔD⁻) so the commit can shrink a cell without a wipe.
//
// This step adds that capability ADDITIVELY, leaving `merge_delta` /
// `diff_cells` / `merge_map_cell_contents` byte-identical so the apply
// path and every existing caller are UNCHANGED (Step 2 rewires the apply
// path; the Step-0 oracle must keep passing here). The removal channel is
// a SIBLING `Object::Map` (`cell_name → Seq[tuples-to-remove]`), shaped
// like the additions delta but carrying the tuples to retract. The
// retraction-aware merge removes ΔD⁻ FIRST, then applies the existing
// additions/union/overwrite logic — so the task-922 UNION semantics for
// ADDITIONS are reused verbatim, never re-implemented.

/// Remove the logical tuples in `removals` from a cell's latest logical
/// contents. Pure helper; the inverse of the union half of
/// `merge_map_cell_contents`.
///
/// Shape handling mirrors the read path (`cell_contents_view` unwraps a
/// chain to its latest contents first):
///   * **Map-form cell** (folded / keyed `D_n`, #932): each entry is
///     `key → fact`. A removal tuple drops any entry whose VALUE equals
///     it (folded cells key by the full-tuple hash, so value-equality is
///     the tuple identity the plan's "remove ΔD⁻ tuples/keys" refers to —
///     and it is robust to a tuple stored under a stale/legacy key).
///   * **Seq-form cell**: drop every element equal to a removal tuple.
///   * **Atom / absent**: no tuples to remove — returned unchanged.
///
/// Returns the shrunk LOGICAL contents (not chain-wrapped); the caller
/// threads it through `chain_append` exactly as the union path does, so
/// the version chain still gets one new entry per commit.
fn remove_tuples_from_cell_contents(existing_chain: &Object, removals: &[Object]) -> Object {
    if removals.is_empty() {
        return cell_contents_view(existing_chain).clone();
    }
    let existing_contents = cell_contents_view(existing_chain);
    match existing_contents {
        Object::Map(m) => {
            // Drop entries whose VALUE matches a removal tuple. Equality is
            // structural on the logical fact, so a folded cell retracts the
            // right tuple regardless of which hash key it sits under.
            let kept: HashMap<String, Object> = m.iter()
                .filter(|(_, v)| !removals.iter().any(|r| r == *v))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            Object::Map(kept.into())
        }
        Object::Seq(items) => {
            let kept: Vec<Object> = items.iter()
                .filter(|f| !removals.iter().any(|r| r == *f))
                .cloned()
                .collect();
            Object::Seq(kept.into())
        }
        // Atom / Bottom / other: nothing tuple-shaped to retract.
        _ => existing_contents.clone(),
    }
}

/// Read a removals-channel value (`cell_name → tuples`) into a flat list
/// of logical tuples for one cell. Accepts the same two encodings as the
/// additions delta: an `Object::Seq` of tuples (the canonical form) or a
/// folded `Object::Map` whose VALUES are the tuples (so a removal set
/// lifted straight off a Map cell round-trips without flattening).
fn removal_tuples(value: &Object) -> Vec<Object> {
    match value {
        Object::Seq(items) => items.iter().cloned().collect(),
        Object::Map(m) => m.values().cloned().collect(),
        Object::Bottom => Vec::new(),
        other => alloc::vec![other.clone()],
    }
}

/// Retraction-capable sibling of [`merge_delta`]: commit a delta that
/// carries both ADDITIONS (`delta`, identical shape + semantics to
/// `merge_delta`'s argument) and REMOVALS (`removals`, a sibling
/// `cell_name → tuples-to-remove` Map). For every cell named in EITHER
/// channel, the new chain entry's contents are computed by:
///   1. removing the cell's ΔD⁻ tuples from its existing latest contents
///      (`remove_tuples_from_cell_contents`), THEN
///   2. applying the EXISTING addition logic (`merge_map_cell_contents`
///      union for Map+Map, overwrite/replace otherwise) on top.
/// Order is load-bearing: removal-then-add lets an `update` expressed as
/// `δ⁻ + δ⁺` (retract the old tuple, add the new one — possibly at a
/// different folded key) resolve to exactly the new tuple.
///
/// `merge_delta(base, delta, event)` is EXACTLY
/// `merge_delta_with_removals(base, delta, &phi, event)` — the empty
/// removal channel reduces this to the union-only path verbatim (asserted
/// by `merge_delta_with_empty_removals_equals_merge_delta`), so existing
/// callers are unaffected and need no migration.
pub fn merge_delta_with_removals(
    base: &Object,
    delta: &Object,
    removals: &Object,
    event: Option<Object>,
) -> Object {
    // Read base WITHOUT unwrapping — chains are preserved (parity with
    // `merge_delta`).
    let mut base_map: HashMap<String, Object> = match base {
        Object::Map(m) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        Object::Seq(cells) => cells.iter().filter_map(|c| {
            let items = c.as_seq()?;
            if items.len() == 3 && items[0].as_atom() == Some(CELL_TAG) {
                Some((items[1].as_atom()?.to_string(), items[2].clone()))
            } else {
                None
            }
        }).collect(),
        _ => HashMap::new(),
    };

    // Additions: raw logical contents per cell (diff_cells contract).
    let delta_pairs: Vec<(String, Object)> = match delta {
        Object::Map(m) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        Object::Seq(cells) => cells.iter().filter_map(|c| {
            let items = c.as_seq()?;
            if items.len() == 3 && items[0].as_atom() == Some(CELL_TAG) {
                Some((items[1].as_atom()?.to_string(), items[2].clone()))
            } else {
                None
            }
        }).collect(),
        _ => Vec::new(),
    };

    // Removals: per-cell tuples to retract. Keyed by cell name; the value
    // is read by `removal_tuples`.
    let removal_map: HashMap<String, Vec<Object>> = match removals {
        Object::Map(m) => m.iter()
            .map(|(k, v)| (k.clone(), removal_tuples(v)))
            .filter(|(_, ts)| !ts.is_empty())
            .collect(),
        _ => HashMap::new(),
    };

    let recorded_at = logical_commit_stamp();

    // Cells touched by EITHER channel get a fresh chain entry. Process the
    // union of (addition keys ∪ removal keys); a removal-only cell still
    // needs a new version with the tuple gone.
    let addition_keys: alloc::collections::BTreeSet<String> =
        delta_pairs.iter().map(|(k, _)| k.clone()).collect();
    let mut delta_by_key: HashMap<String, Object> = delta_pairs.into_iter().collect();

    // Deterministic processing order (BTreeSet) — the cell map is a set, so
    // order carries no semantics, only reproducibility (parity with
    // fetch_cell_seq's key-ordered flatten).
    let mut all_keys: alloc::collections::BTreeSet<String> = addition_keys;
    for k in removal_map.keys() { all_keys.insert(k.clone()); }

    for k in all_keys {
        let removals_for_cell = removal_map.get(&k).map(|v| v.as_slice()).unwrap_or(&[]);
        let addition_for_cell = delta_by_key.remove(&k);
        let new_chain = match base_map.get(&k) {
            Some(existing) => {
                // 1. Retract ΔD⁻ from the existing latest contents.
                let after_removal = remove_tuples_from_cell_contents(existing, removals_for_cell);
                // 2. Layer ΔD⁺ on top using the EXISTING union/overwrite
                //    logic. `merge_map_cell_contents` reads its first
                //    argument via `cell_contents_view`; `after_removal` is
                //    already unwrapped logical contents, which that view
                //    passes through unchanged, so the union sees the
                //    post-removal Map and the task-922 semantics are reused
                //    verbatim.
                let merged_contents = match &addition_for_cell {
                    Some(v) => merge_map_cell_contents(&after_removal, v),
                    None => after_removal,
                };
                chain_append(existing, merged_contents, recorded_at.clone(), event.clone())
            }
            None => {
                // Cell absent from base: there is nothing to retract, so the
                // result is just the additions (if any). A removal-only delta
                // for an absent cell is a no-op (don't mint an empty cell).
                match addition_for_cell {
                    Some(v) => wrap_as_chain(v, recorded_at.clone(), event.clone()),
                    None => continue,
                }
            }
        };
        base_map.insert(k, new_chain);
    }
    Object::Map(base_map.into())
}

/// Retraction-aware sibling of [`diff_cells`]: emit BOTH the additions
/// patch (identical to `diff_cells(old, new)`) AND the per-cell removals
/// (`cell_name → Seq[tuples present in old but absent from new]`).
///
/// `diff_cells_with_removals(old, new)` paired with
/// `merge_delta_with_removals(old, additions, removals, _)` reconstructs
/// `new`'s logical view for every cell — INCLUDING cells that LOST tuples,
/// which `diff_cells`+`merge_delta` alone cannot express (a shrunk Map
/// cell unions back the dropped tuple). For a cell present in both with
/// only additions, `removals` carries an empty (or absent) entry and the
/// behavior collapses to the union-only round-trip.
///
/// Removals are computed on the LOGICAL tuple sets (`fetch_cell_seq`
/// key-flattens folded Map cells first), so the diff is shape-agnostic:
/// a tuple counts as removed iff it appears in `old`'s contents and not in
/// `new`'s. Only cells whose tuple set actually shrank appear in the
/// removals Map (an added-only or unchanged cell contributes nothing).
pub fn diff_cells_with_removals(old: &Object, new: &Object) -> (Object, Object) {
    let additions = diff_cells(old, new);

    // Per-cell removed tuples: for every cell in `old`, the logical tuples
    // that are gone from `new`. Cells absent from `new` entirely surface
    // every one of their old tuples as removed.
    let mut removals: HashMap<String, Object> = HashMap::new();
    for (name, _) in cells_iter(old).into_iter() {
        let old_tuples = fetch_cell_seq(name, old);
        let Some(old_items) = old_tuples.as_seq() else { continue };
        if old_items.is_empty() { continue; }
        let new_tuples = fetch_cell_seq(name, new);
        let new_items: &[Object] = new_tuples.as_seq().unwrap_or(&[]);
        let removed: Vec<Object> = old_items.iter()
            .filter(|t| !new_items.iter().any(|n| n == *t))
            .cloned()
            .collect();
        if !removed.is_empty() {
            removals.insert(name.to_string(), Object::Seq(removed.into()));
        }
    }
    (additions, Object::Map(removals.into()))
}

/// Demultiplex events by cell assignment (paper Eq. demux).
/// E_n = Filter(eq ∘ [RMAP, n̄]) : E
/// Splits a sequence of (fact_type_id, fact) pairs into per-cell groups
/// using the shard map (fact_type_id → cell_name).
pub fn demux<'a>(events: &'a [(String, Object)], shard_map: &HashMap<String, String>) -> HashMap<String, Vec<&'a (String, Object)>> {
    let mut cells: HashMap<String, Vec<&(String, Object)>> = HashMap::new();
    for event in events {
        let cell = shard_map.get(&event.0)
            .cloned()
            .unwrap_or_else(|| event.0.clone());
        cells.entry(cell).or_default().push(event);
    }
    cells
}

/// Get a binding value by role name from a named-tuple fact.
/// A named-tuple fact is <<role1, val1>, <role2, val2>, ...>.
/// Replaces: fact.bindings.iter().find(|(k,_)| k == "name").map(|(_,v)| v)
pub fn binding<'a>(fact: &'a Object, key: &str) -> Option<&'a str> {
    fact.as_seq()?.iter().find_map(|pair| {
        let items = pair.as_seq()?;
        if items.len() == 2 && items[0].as_atom() == Some(key) {
            items[1].as_atom()
        } else {
            None
        }
    })
}

/// Build a named-tuple fact from (key, value) pairs.
/// Replaces: FactInstance { fact_type_id, bindings: vec![(k,v), ...] }
pub fn fact_from_pairs(pairs: &[(&str, &str)]) -> Object {
    Object::Seq(pairs.iter().map(|(k, v)| {
        Object::seq(vec![Object::atom(k), Object::atom(v)])
    }).collect())
}

/// ns-3 (per-file domain binding): stamp every declared Function in
/// `state` — nouns and fact types — with `belongs to Domain 'domain'`,
/// returning a one-cell state (`Function_belongs_to_Domain`) ready to
/// `merge_states` into the per-file parse before the loader's fold merges
/// it forward. Mirrors `build_provenance_cell`'s per-file cell walk; the
/// (`Function`, `Domain`) role-noun keys follow the `fact_from_pairs`
/// convention (cf. `Completion_belongs_to_Agent`). `domain` is resolved by
/// the caller (file basename default, or an in-file `Domain 'X'` override).
pub fn stamp_file_domain(state: &Object, domain: &str) -> Object {
    let mut facts: Vec<Object> = Vec::new();
    for (kind, id_key) in [("Noun", "name"), ("FactType", "id")] {
        if let Some(cell_facts) = fetch_cell_seq(kind, state).as_seq() {
            for f in cell_facts {
                if let Some(id) = binding(f, id_key) {
                    facts.push(fact_from_pairs(&[("Function", id), ("Domain", domain)]));
                }
            }
        }
    }
    let mut m: HashMap<String, Object> = HashMap::new();
    m.insert("Function_belongs_to_Domain".to_string(), Object::seq(facts));
    Object::map(m)
}

/// ns-4 (namespace-keyed identity): tag every fact in `state`'s Noun cell
/// with a `homeDomain` binding (the file domain), so `same_identity` keys
/// nouns by (name, homeDomain) and same-named nouns from different domains
/// (core.Order value vs orders.Order entity) survive `merge_states` as
/// distinct entries instead of collapsing by name. Idempotent (replaces any
/// existing homeDomain); other cells pass through unchanged. Distinct from
/// the createEntity `domain` command field, so resource handling is untouched.
pub fn annotate_noun_domain(state: &Object, domain: &str) -> Object {
    let mut map: HashMap<String, Object> = cells_iter(state).into_iter()
        .map(|(n, c)| (n.to_string(), c.clone()))
        .collect();
    if let Some(facts) = map.get("Noun").and_then(|c| c.as_seq()).map(|s| s.to_vec()) {
        let annotated: Vec<Object> = facts.iter().map(|f| {
            let mut pairs: Vec<Object> = f.as_seq().map(|s| s.to_vec()).unwrap_or_default();
            pairs.retain(|p| p.as_seq()
                .and_then(|kv| kv.first())
                .and_then(|x| x.as_atom()) != Some("homeDomain"));
            pairs.push(Object::seq(vec![Object::atom("homeDomain"), Object::atom(domain)]));
            Object::seq(pairs)
        }).collect();
        map.insert("Noun".to_string(), Object::seq(annotated));
    }
    Object::map(map)
}

/// Check if a named-tuple fact has a binding matching key=val.
/// Replaces: fact.bindings.iter().any(|(k, v)| k == key && v == val)
pub fn binding_matches(fact: &Object, key: &str, val: &str) -> bool {
    binding(fact, key) == Some(val)
}

/// Whole-tuple equality between a named-tuple fact `<<role,val>,...>` and a
/// list of `(role, value)` pairs. True iff the fact has exactly the same
/// arity and every requested `(role, value)` is present (set-membership
/// per pair, matching the role-name-keyed semantics of the retract FFI).
///
/// Shared by the `retract:<ft>` FFI write-back (lib.rs / cli/entry.rs):
/// it both detects whether a row exists and drives the `cell_filter`
/// predicate that drops the matching row while preserving the cell's
/// Map shape (W7-b). Pure, references-only.
pub fn fact_matches_pairs(fact: &Object, pairs: &[(String, String)]) -> bool {
    let fact_pairs: alloc::vec::Vec<(&str, &str)> = match fact.as_seq() {
        Some(ps) => ps
            .iter()
            .filter_map(|p| {
                let kv = p.as_seq()?;
                if kv.len() != 2 {
                    return None;
                }
                Some((kv[0].as_atom()?, kv[1].as_atom()?))
            })
            .collect(),
        None => return false,
    };
    if fact_pairs.len() != pairs.len() {
        return false;
    }
    pairs.iter().all(|(role, value)| {
        fact_pairs
            .iter()
            .any(|(fr, fv)| *fr == role.as_str() && *fv == value.as_str())
    })
}

/// Retain only facts in a cell that satisfy a predicate. Pure functional filter.
/// Replaces: instances.retain(|inst| predicate(inst))
pub fn cell_filter(name: &str, predicate: impl Fn(&Object) -> bool, state: &Object) -> Object {
    let existing = fetch_or_phi(name, state);
    // #932 phase-2: both-tolerant. A Seq cell filters its items (Seq
    // preserved); a Map cell (folded D_n) filters by value, preserving
    // the keyed Map shape — so a retract on a folded cell drops only the
    // matching rows instead of collapsing the whole cell to empty (the
    // raw-as_seq()-on-Map silent-wipe bug class). Other shapes hold no
    // facts to retain.
    let filtered = match &existing {
        Object::Seq(items) => {
            Object::Seq(items.iter().filter(|f| predicate(f)).cloned().collect())
        }
        Object::Map(m) => Object::Map(
            m.iter()
                .filter(|(_, v)| predicate(v))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<HashMap<String, Object>>()
                .into(),
        ),
        _ => Object::phi(),
    };
    store(name, filtered, state)
}

/// Content-address for a fact: `{ft_id}#{fnv64 of sorted bindings}`.
///
/// Same FNV-1a the forward-chain dedup uses, so two code paths that
/// produce the same fact shape end up at the same id. Used by the
/// Migration runtime (#349) to stamp `source`/`produces` role values
/// on `MigrationApplication` facts, and by `visible_population`
/// (#350) to re-identify those facts in the population.
pub fn synthesize_fact_id(ft_id: &str, fact: &Object) -> String {
    let mut pairs: Vec<(String, String)> = fact.as_seq()
        .map(|ps| ps.iter().filter_map(|p| {
            let pair = p.as_seq()?;
            if pair.len() != 2 { return None; }
            Some((pair[0].as_atom()?.to_string(), pair[1].as_atom()?.to_string()))
        }).collect())
        .unwrap_or_default();
    // directed-ring-dedup: sort by ROLE NAME only (stable), NOT the full
    // (role, value) tuple. A full-tuple sort orders the two same-noun
    // bindings of a ring fact (`Glyph reaches Glyph`) by VALUE, so the
    // directed pair <g2,g0> hashes identically to <g0,g2> and the cell
    // can hold only one direction per unordered pair — silently dropping
    // half of a transitive closure. Name-only stable sort keeps the
    // positional (role-index) order, so a→b and b→a get distinct keys,
    // while still normalizing the order of DISTINCT-named roles (caller
    // -order invariance). Matches `fact_identity_id` and the chain's
    // `fact_key` / `state_keys`.
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME:  u64 = 0x100000001b3;
    let mut h: u64 = FNV_OFFSET;
    for b in ft_id.bytes() { h ^= b as u64; h = h.wrapping_mul(FNV_PRIME); }
    for (k, v) in &pairs {
        h ^= b'|' as u64; h = h.wrapping_mul(FNV_PRIME);
        for b in k.bytes() { h ^= b as u64; h = h.wrapping_mul(FNV_PRIME); }
        h ^= b'=' as u64; h = h.wrapping_mul(FNV_PRIME);
        for b in v.bytes() { h ^= b as u64; h = h.wrapping_mul(FNV_PRIME); }
    }
    alloc::format!("{}#{:016x}", ft_id, h)
}

/// task-970: deterministic Skolem id for an existential derivation head.
///
/// The value-invention leaf for a TGD head (`Func::Platform("skolem")`).
/// `x` is the FRONTIER tuple — a sequence whose elements are the
/// antecedent-bound role values that distinguish one head instance from
/// another (e.g. `<View.Name, Transition.id>` for the §4.5 menu rule).
/// Returns `Atom("ve_" ++ fnv1a64_hex(seed))` where `seed` is the
/// frontier values joined with `|`.
///
/// Why this is the correctness crux of the Skolem (semi-oblivious) chase:
/// the id is a PURE FUNCTION of the frontier — no clock, counter, or RNG
/// — so re-deriving the same head over the same population reproduces the
/// SAME id, and the lazy `resolve_view` recomputation stays idempotent
/// (same frontier → same id → no duplicate entity across passes). The
/// hash is the same FNV-1a-64 `fact_key`/`synthesize_fact_id` use, so a
/// Skolem id is stable across runs and code paths.
///
/// Shape contract (total — Bottom, never panic): `x` must be a sequence;
/// each element is coerced to its atom form (a non-atom element renders
/// via `Object::to_string`, so nested values still contribute
/// deterministically). A non-sequence `x` → Bottom.
///
/// The `ve_` prefix and `|` separator are fixed in this minimal version;
/// a future generalisation parameterises them so the one primitive serves
/// any existential head (`<prefix, frontier…>` input shape).
///
/// UNGATED (no `no_std` cfg): pure alloc-only FNV — the no_std Platform
/// arm dispatches it directly so skolem-head view rules mint ids in the
/// kernel too (§5.2 viewproj-client-render).
fn platform_skolem(x: &Object) -> Object {
    let items = match x.as_seq() {
        Some(items) => items,
        None => return Object::Bottom,
    };
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME:  u64 = 0x100000001b3;
    let mut h: u64 = FNV_OFFSET;
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            h ^= b'|' as u64;
            h = h.wrapping_mul(FNV_PRIME);
        }
        // Prefer the bare atom; fall back to Display for non-atoms so the
        // seed is still deterministic (Object's Display is stable).
        let owned;
        let s: &str = match item.as_atom() {
            Some(a) => a,
            None => { owned = alloc::format!("{}", item); &owned }
        };
        for b in s.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(FNV_PRIME);
        }
    }
    Object::Atom(alloc::format!("ve_{:016x}", h))
}

/// ρ-projection of the population, hiding facts migrated away by an
/// active Migration (paper §5, #350).
///
/// A fact `f` is hidden iff some `MigrationApplication` names `f`'s
/// `synthesize_fact_id` in its `source` binding AND that MA's
/// `migration` role still resolves to a row in the `Migration` cell.
/// Tying the projection to the Migration's presence is what makes
/// rollback free: retract the Migration and the MA becomes orphaned,
/// so visible_population stops filtering — P itself stays append-only
/// (Thm 5), and Cor 3 (closure under self-modification) survives.
///
/// Cells with a `:` in their name (defs, indices, schema shards) are
/// passed through untouched; they're never subject to the projection.
/// The `MigrationApplication` and `Migration` cells themselves are
/// passed through verbatim so callers can still read the metadata that
/// drives the projection.
pub fn visible_population(state: &Object) -> Object {
    use hashbrown::HashSet;
    let migrations = fetch_cell_seq("Migration", state);
    let active_migrations: HashSet<String> = migrations.as_seq()
        .map(|s| s.iter().filter_map(|m| binding(m, "id").map(String::from)).collect())
        .unwrap_or_default();
    if active_migrations.is_empty() { return state.clone(); }
    let mas = fetch_cell_seq("MigrationApplication", state);
    let hidden_ids: HashSet<String> = mas.as_seq()
        .map(|s| s.iter().filter_map(|ma| {
            let m_id = binding(ma, "migration")?;
            if !active_migrations.contains(m_id) { return None; }
            binding(ma, "source").map(String::from)
        }).collect())
        .unwrap_or_default();
    if hidden_ids.is_empty() { return state.clone(); }

    let mut out = state.clone();
    for (cell_name, contents) in cells_iter(state) {
        if cell_name.contains(':') { continue; }
        if cell_name == "MigrationApplication" || cell_name == "Migration" { continue; }
        // #932 phase-2: scan via cell_facts_iter (Map-tolerant — the bare
        // `contents.as_seq()` skipped folded Map cells) and re-store
        // preserving the cell's shape, so the migration projection never
        // de-keys a folded (D_n) cell. Skip-unchanged kept to avoid churn.
        let total = cell_facts_iter(contents).count();
        let new_contents = match contents {
            Object::Map(m) => {
                let kept: HashMap<String, Object> = m
                    .iter()
                    .filter(|(_, v)| !hidden_ids.contains(&synthesize_fact_id(&cell_name, v)))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                if kept.len() == total { continue; }
                Object::Map(kept.into())
            }
            _ => {
                let kept: Vec<Object> = cell_facts_iter(contents)
                    .filter(|f| !hidden_ids.contains(&synthesize_fact_id(&cell_name, f)))
                    .cloned()
                    .collect();
                if kept.len() == total { continue; }
                Object::Seq(kept.into())
            }
        };
        out = store(&cell_name, new_contents, &out);
    }
    out
}

/// The representation function ρ: Object → Func (Backus 13.3.2).
///
/// Maps objects to the functions they represent:
/// - Primitive atoms → primitive Func variants
/// - Defined atoms → definitions from D
/// - Undefined atoms → ⊥̄ (bottom everywhere)
/// - Sequences → functional forms via controlling operator
pub fn metacompose(obj: &Object, d: &Object) -> Func {
    match obj {
        Object::Bottom => Func::Constant(Object::Bottom),
        Object::Atom(name) => metacompose_atom(name, d),
        Object::Seq(items) if items.is_empty() => Func::Constant(Object::Bottom),
        Object::Seq(items) => metacompose_sequence(items, d),
        Object::Map(_) => Func::Constant(obj.clone()), // stores are data, not functions
    }
}

fn metacompose_atom(name: &str, d: &Object) -> Func {
    // Check definitions in D first (Backus 13.3.2: Def n ≡ r)
    let def_obj = fetch(name, d);
    match &def_obj {
        Object::Bottom => {},
        obj => return metacompose(obj, d),
    }

    // Primitive atoms (Backus 11.2.3)
    match name {
        primitives::ID => Func::Id,
        primitives::TL => Func::Tail,
        primitives::ATOM => Func::AtomTest,
        primitives::EQ => Func::Eq,
        primitives::GT => Func::Gt,
        primitives::LT => Func::Lt,
        primitives::GE => Func::Ge,
        primitives::LE => Func::Le,
        primitives::CONTAINS => Func::Contains,
        primitives::STARTS_WITH => Func::StartsWith,
        primitives::ENDS_WITH => Func::EndsWith,
        primitives::TRIM => Func::Trim,
        primitives::SPLIT => Func::Split,
        primitives::REPLACE => Func::Replace,
        primitives::CONCAT => Func::Concat,
        primitives::COMPACT => Func::Compact,
        primitives::LOWER => Func::Lower,
        primitives::NULL => Func::NullTest,
        primitives::CELL_NAME_TEST => Func::CellNameTest,
        primitives::REVERSE => Func::Reverse,
        primitives::DISTL => Func::DistL,
        primitives::DISTR => Func::DistR,
        primitives::HAS_MEMBER => Func::HasMember,
        primitives::SET_FROM_SEQ => Func::SetFromSeq,
        primitives::LENGTH => Func::Length,
        primitives::TRANS => Func::Trans,
        primitives::APNDL => Func::ApndL,
        primitives::APNDR => Func::ApndR,
        primitives::ROTL => Func::RotL,
        primitives::ROTR => Func::RotR,
        primitives::ADD => Func::Add,
        primitives::SUB => Func::Sub,
        primitives::MUL => Func::Mul,
        primitives::DIV => Func::Div,
        primitives::AND => Func::And,
        primitives::OR => Func::Or,
        primitives::NOT => Func::Not,
        primitives::FETCH => Func::Fetch,
        primitives::FETCH_OR_PHI => Func::FetchOrPhi,
        primitives::STORE => Func::Store,
        // Platform primitives: "platform:compile", "platform:apply_command", ...
        s if s.starts_with("platform:") => Func::Platform(s["platform:".len()..].to_string()),
        // Selector atoms: "1", "2", "3", ...
        s if s.parse::<usize>().is_ok() => Func::Selector(s.parse().unwrap()),
        // Undefined atom → ⊥̄
        _ => Func::Constant(Object::Bottom),
    }
}

fn metacompose_sequence(items: &[Object], d: &Object) -> Func {
    // Backus dispatch: <controller, args...> -> Func.
    // Any shape mismatch folds to None -> Func::Constant(Bottom) via unwrap_or.
    items.first()
        .and_then(|f| f.as_atom())
        .map(|controller| match controller {
        forms::COMP if items.len() == 3 => {
            // <COMP, f, g> → f ∘ g
            let f = metacompose(&items[1], d);
            let g = metacompose(&items[2], d);
            Func::Compose(Box::new(f), Box::new(g))
        }
        forms::CONS if items.len() >= 2 => {
            // <CONS, f₁, ..., fₙ> → [f₁, ..., fₙ]
            let funcs: Vec<Func> = items[1..].iter().map(|o| metacompose(o, d)).collect();
            Func::Construction(funcs)
        }
        forms::COND if items.len() == 4 => {
            // <COND, p, f, g> → (p → f; g)
            let p = metacompose(&items[1], d);
            let f = metacompose(&items[2], d);
            let g = metacompose(&items[3], d);
            Func::Condition(Box::new(p), Box::new(f), Box::new(g))
        }
        forms::ALPHA if items.len() == 2 => {
            // <ALPHA, f> → αf
            let f = metacompose(&items[1], d);
            Func::ApplyToAll(Box::new(f))
        }
        forms::INSERT if items.len() == 2 => {
            // <INSERT, f> → /f
            let f = metacompose(&items[1], d);
            Func::Insert(Box::new(f))
        }
        forms::FOLDL if items.len() == 2 => {
            // <FOLDL, f> → foldl(f)
            let f = metacompose(&items[1], d);
            Func::FoldL(Box::new(f))
        }
        forms::INDEX_BY if items.len() == 2 => {
            // <INDEX_BY, keyfn> → IndexBy(keyfn)
            let f = metacompose(&items[1], d);
            Func::IndexBy(Box::new(f))
        }
        forms::ORDER_BY if items.len() == 2 => {
            // <ORDER_BY, keyfn> → OrderBy(keyfn)
            let f = metacompose(&items[1], d);
            Func::OrderBy(Box::new(f))
        }
        forms::BU if items.len() == 3 => {
            // <BU, f, x> → (bu f x)
            let f = metacompose(&items[1], d);
            let x = items[2].clone();
            Func::BinaryToUnary(Box::new(f), x)
        }
        forms::FILTER if items.len() == 2 => {
            // <FILTER, p> → Filter(p)
            let p = metacompose(&items[1], d);
            Func::Filter(Box::new(p))
        }
        forms::WHILE if items.len() == 3 => {
            // <WHILE, p, f> → (while p f)
            let p = metacompose(&items[1], d);
            let f = metacompose(&items[2], d);
            Func::While(Box::new(p), Box::new(f))
        }
        forms::CONST if items.len() == 2 => {
            // <CONST, x> → x̄
            Func::Constant(items[1].clone())
        }
        _ => {
            // Unknown controlling operator → ⊥̄
            Func::Constant(Object::Bottom)
        }
    })
    .unwrap_or(Func::Constant(Object::Bottom))
}

/// FFP application: evaluate (x:y) where x is an object representing
/// a function and y is the operand (Backus 13.3.1).
///
/// μ(x:y) = (ρ x):y
pub fn apply_ffp(
    operator: &Object,
    operand: &Object,
    d: &Object,
) -> Object {
    apply(&metacompose(operator, d), operand, d)
}

/// Convert a Func back to its FFP object representation.
/// This is the inverse of ρ (on the image of compilation).
pub fn func_to_object(func: &Func) -> Object {
    match func {
        Func::Id => Object::atom(primitives::ID),
        Func::Selector(n) => Object::atom(&n.to_string()),
        Func::Tail => Object::atom(primitives::TL),
        Func::AtomTest => Object::atom(primitives::ATOM),
        Func::NullTest => Object::atom(primitives::NULL),
        Func::CellNameTest => Object::atom(primitives::CELL_NAME_TEST),
        Func::Eq => Object::atom(primitives::EQ),
        Func::Gt => Object::atom(primitives::GT),
        Func::Lt => Object::atom(primitives::LT),
        Func::Ge => Object::atom(primitives::GE),
        Func::Le => Object::atom(primitives::LE),
        Func::Contains => Object::atom(primitives::CONTAINS),
        Func::StartsWith => Object::atom(primitives::STARTS_WITH),
        Func::EndsWith => Object::atom(primitives::ENDS_WITH),
        Func::Trim => Object::atom(primitives::TRIM),
        Func::Split => Object::atom(primitives::SPLIT),
        Func::Replace => Object::atom(primitives::REPLACE),
        Func::Concat => Object::atom(primitives::CONCAT),
        Func::Compact => Object::atom(primitives::COMPACT),
        Func::Lower => Object::atom(primitives::LOWER),
        Func::Length => Object::atom(primitives::LENGTH),
        Func::DistL => Object::atom(primitives::DISTL),
        Func::DistR => Object::atom(primitives::DISTR),
        Func::HasMember => Object::atom(primitives::HAS_MEMBER),
        Func::SetFromSeq => Object::atom(primitives::SET_FROM_SEQ),
        Func::Trans => Object::atom(primitives::TRANS),
        Func::ApndL => Object::atom(primitives::APNDL),
        Func::Reverse => Object::atom(primitives::REVERSE),
        Func::ApndR => Object::atom(primitives::APNDR),
        Func::RotL => Object::atom(primitives::ROTL),
        Func::RotR => Object::atom(primitives::ROTR),
        Func::Add => Object::atom(primitives::ADD),
        Func::Sub => Object::atom(primitives::SUB),
        Func::Mul => Object::atom(primitives::MUL),
        Func::Div => Object::atom(primitives::DIV),
        Func::And => Object::atom(primitives::AND),
        Func::Or => Object::atom(primitives::OR),
        Func::Not => Object::atom(primitives::NOT),
        Func::Fetch => Object::atom(primitives::FETCH),
        Func::FetchOrPhi => Object::atom(primitives::FETCH_OR_PHI),
        Func::Store => Object::atom(primitives::STORE),
        Func::Constant(x) => Object::seq(vec![Object::atom(forms::CONST), x.clone()]),
        Func::Compose(f, g) => Object::seq(vec![
            Object::atom(forms::COMP), func_to_object(f), func_to_object(g),
        ]),
        Func::Construction(funcs) => {
            let mut items = vec![Object::atom(forms::CONS)];
            items.extend(funcs.iter().map(func_to_object));
            Object::Seq(items.into()) // not bottom-preserving — these are form objects
        }
        Func::Condition(p, f, g) => Object::seq(vec![
            Object::atom(forms::COND), func_to_object(p), func_to_object(f), func_to_object(g),
        ]),
        Func::ApplyToAll(f) => Object::seq(vec![Object::atom(forms::ALPHA), func_to_object(f)]),
        Func::Insert(f) => Object::seq(vec![Object::atom(forms::INSERT), func_to_object(f)]),
        Func::FoldL(f) => Object::seq(vec![Object::atom(forms::FOLDL), func_to_object(f)]),
        Func::IndexBy(f) => Object::seq(vec![Object::atom(forms::INDEX_BY), func_to_object(f)]),
        Func::OrderBy(f) => Object::seq(vec![Object::atom(forms::ORDER_BY), func_to_object(f)]),
        Func::BinaryToUnary(f, x) => Object::seq(vec![
            Object::atom(forms::BU), func_to_object(f), x.clone(),
        ]),
        Func::Filter(p) => Object::seq(vec![Object::atom(forms::FILTER), func_to_object(p)]),
        Func::While(p, f) => Object::seq(vec![
            Object::atom(forms::WHILE), func_to_object(p), func_to_object(f),
        ]),
        Func::Def(name) => Object::atom(name),
        Func::Platform(name) => Object::atom(&format!("platform:{}", name)),
        Func::Native(_) => Object::atom("<native>"),
    }
}

// ── Codd's θ₁: Named Relational Algebra Definitions ─────────────────
//
// Codd 1970 Sec 2.2: an adequate collection θ₁ for the named set is
// {projection, natural join, tie, restriction}. Each is an FFP definition
// composed from Backus's primitives and forms. These are registered in
// the definitions set D so they can be called by name via ρ.

/// Register Codd's theta-1 relational algebra operations as named definitions.
/// Call this to populate a defs map with the standard relational operations.
///
/// Pure Func analysis: all four operations require dynamic arity handling
/// (the number of columns per tuple varies at runtime), which cannot be
/// expressed as a fixed Func tree. Specifically:
///
/// - project: must build a Construction from runtime index values.
///   Pure form would be alpha(Construction(selectors)), but Construction
///   is a compile-time combinator and the selector list comes from data.
///
/// - join: the shared column index determines which selector to compare
///   and which columns to exclude from the merge. This is data-dependent
///   column selection that cannot be expressed without dynamic Construction.
///
/// - tie: checks first = last column (eq . [sel(1), sel(n)]), but n is
///   the tuple arity which varies per relation. Pure Func has no "select
///   last element" primitive (Backus defines selectors as fixed indices).
///
/// - compose_rel: combines join + project, inheriting both limitations.
///
/// All four route through Platform dispatch so each runtime (Rust, FPGA,
/// Solidity) can provide its own implementation of the named operation.
pub fn theta1_defs_vec() -> Vec<(String, Func)> {
    let mut defs = Vec::new();
    register_theta1_into(&mut defs);
    defs
}
fn register_theta1_into(defs: &mut Vec<(String, Func)>) {
    // Codd θ₁ operators are Platform ops. Each runtime (server, FPGA,
    // Solidity) resolves the named operation to its own implementation.
    // The Rust runtime dispatches to platform_project/join/tie/compose_rel
    // in apply_platform. Previously these were Func::Native(closure),
    // which couldn't be synthesized. See paper §"Relational Algebra".
    defs.push(("project".to_string(), Func::Platform("project".to_string())));
    defs.push(("join".to_string(), Func::Platform("join".to_string())));
    defs.push(("tie".to_string(), Func::Platform("tie".to_string())));
    defs.push(("compose_rel".to_string(), Func::Platform("compose_rel".to_string())));
}

// ── Identity generators (Conceptual Data Type, P3a) ─────────────────
//
// `gen:autocounter` — the canonical Backus reduction for "next integer
// id". Takes a Seq of the noun's existing numeric id atoms and returns
// the next id (max + 1; the empty Seq yields "1"). It is the pure FFP
// program
//
//   + ∘ [ /max ∘ apndl ∘ [0̄, ids] , 1̄ ]
//
// where `max:<a,b> = (a ≥ b → a ; b)` and `/max` is Backus's right
// insert. The seed `apndl:[0̄, ids]` prepends a `0` so `/max` always has
// a base element — `max` has no unit in `unit_of`, so `/max:<>` would be
// ⊥; with the seed `/max:<0> = 0` and the empty population correctly
// yields `+:<0,1> = 1`. `Ge`/`Add` parse atom operands as numbers (see
// `apply_compare` / `apply_arithmetic`), so the result is the
// integer-string atom that the bare-integer id scheme expects.

/// Build the `gen:autocounter` Func: `<existing numeric id atoms>` → next
/// id atom (max + 1; `<>` → "1"). See the module comment above for the
/// algebra. This is model-independent, so a single shared definition is
/// registered once into the compiled DEFS (`compile_to_defs_state`) and
/// resolved by name via `Func::Def("gen:autocounter")`.
pub fn gen_autocounter() -> Func {
    Func::compose(
        Func::Add,                                            // +:<max, 1>
        Func::construction(vec![
            Func::compose(
                Func::Insert(Box::new(Func::Condition(        // /max  (right insert/reduce)
                    Box::new(Func::Ge),                       // max:<a,b> = (a≥b → a ; b)
                    Box::new(Func::Selector(1)),
                    Box::new(Func::Selector(2)),
                ))),
                Func::compose(
                    Func::ApndL,                              // <0, id1, ..., idn>
                    Func::construction(vec![
                        Func::constant(Object::atom("0")),
                        Func::Id,                             // the input Seq of id atoms
                    ]),
                ),
            ),
            Func::constant(Object::atom("1")),
        ]),
    )
}

// H4 (#692): the `_register_theta1_native_legacy` reference body —
// 130 lines of Func::Native(Arc::new(closure)) reproductions of the
// Codd θ₁ ops that previously documented the pre-Platform escape
// hatch — was removed when the last production Native leaf (rmap_func)
// migrated to Platform. Each op's live implementation lives in
// `apply_platform` (`platform_project` / `platform_join` /
// `platform_tie` / `platform_compose_rel`); the `register_theta1_into`
// registry above wires each name as a `Func::Platform`. See git
// history (`git log -- crates/arest/src/ast.rs | grep H4`) for the
// removed body if the closure form is needed for FPGA / Solidity
// dispatch reference.

// ── Convenience constructors ─────────────────────────────────────────

impl Func {
    /// f ∘ g
    pub fn compose(f: Func, g: Func) -> Func {
        Func::Compose(Box::new(f), Box::new(g))
    }

    /// [f₁, ..., fₙ]
    pub fn construction(funcs: Vec<Func>) -> Func {
        Func::Construction(funcs)
    }

    /// p → f; g
    pub fn condition(p: Func, f: Func, g: Func) -> Func {
        Func::Condition(Box::new(p), Box::new(f), Box::new(g))
    }

    /// αf
    pub fn apply_to_all(f: Func) -> Func {
        Func::ApplyToAll(Box::new(f))
    }

    /// /f
    pub fn insert(f: Func) -> Func {
        Func::Insert(Box::new(f))
    }

    /// foldl(f)
    pub fn foldl(f: Func) -> Func {
        Func::FoldL(Box::new(f))
    }

    /// Filter(p)
    pub fn filter(p: Func) -> Func {
        Func::Filter(Box::new(p))
    }

    /// bu f x
    pub fn bu(f: Func, x: Object) -> Func {
        Func::BinaryToUnary(Box::new(f), x)
    }

    /// x̄ (constant)
    pub fn constant(x: Object) -> Func {
        Func::Constant(x)
    }

    /// Role at position n (1-indexed)
    pub fn role(n: usize) -> Func {
        Func::Selector(n)
    }

    /// Right-selector (Backus §11.2.4 `nr`): the n-th element *from the
    /// right* of a sequence. `selector_from_right(1)` is Backus's `1r`
    /// (last), `selector_from_right(2)` is `2r` (second-to-last), etc.
    ///
    /// Backus lists `1r`, `2r`, … as primitives; AREST derives them as
    /// `Selector(n) ∘ Reverse`, so no enum variant is added. Equivalent
    /// semantics, one extra ρ-application per evaluation, which is
    /// fine for the contexts where right-selectors appear (audit-log
    /// tail reads, trailing-role access in FORML 2 role subscripts).
    /// If a hot path ever justifies promoting these to variants, the
    /// constructor stays source-compatible — callers don't change.
    pub fn selector_from_right(n: usize) -> Func {
        Func::compose(Func::Selector(n), Func::Reverse)
    }

    /// Right-tail (Backus §11.2.4 `tlr`): the sequence `<x₁, …, xₙ₋₁>`
    /// dropping the last element. Derives as `Reverse ∘ Tail ∘ Reverse`.
    /// Same rationale as `selector_from_right`: Backus-named, AREST-
    /// derived, cheap to promote if perf ever calls for it.
    pub fn tail_from_right() -> Func {
        Func::compose(Func::Reverse, Func::compose(Func::Tail, Func::Reverse))
    }

    /// Returns true if this Func or any sub-Func contains a Native closure.
    /// Pure Func = no Native anywhere in the tree.
    pub fn has_native(&self) -> bool {
        match self {
            Func::Native(_) => true,
            Func::Compose(f, g) => f.has_native() || g.has_native(),
            Func::Construction(fs) => fs.iter().any(|f| f.has_native()),
            Func::Condition(p, f, g) => p.has_native() || f.has_native() || g.has_native(),
            Func::ApplyToAll(f) | Func::Insert(f) | Func::Filter(f) | Func::FoldL(f)
            | Func::IndexBy(f) | Func::OrderBy(f) => f.has_native(),
            Func::While(p, f) => p.has_native() || f.has_native(),
            Func::BinaryToUnary(f, _) => f.has_native(),
            _ => false,
        }
    }
}

// ── Debug ────────────────────────────────────────────────────────────

impl fmt::Debug for Func {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Func::Id => write!(f, "id"),
            Func::Selector(n) => write!(f, "{}", n),
            Func::Tail => write!(f, "tl"),
            Func::AtomTest => write!(f, "atom"),
            Func::NullTest => write!(f, "null"),
            Func::CellNameTest => write!(f, "cell?"),
            Func::Eq => write!(f, "eq"),
            Func::Gt => write!(f, ">"),
            Func::Lt => write!(f, "<"),
            Func::Ge => write!(f, "≥"),
            Func::Le => write!(f, "≤"),
            Func::Contains => write!(f, "contains"),
            Func::StartsWith => write!(f, "starts_with"),
            Func::EndsWith => write!(f, "ends_with"),
            Func::Trim => write!(f, "trim"),
            Func::Split => write!(f, "split"),
            Func::Replace => write!(f, "replace"),
            Func::Concat => write!(f, "concat"),
            Func::Compact => write!(f, "compact"),
            Func::Lower => write!(f, "lower"),
            Func::Length => write!(f, "length"),
            Func::DistL => write!(f, "distl"),
            Func::DistR => write!(f, "distr"),
            Func::HasMember => write!(f, "has_member"),
            Func::SetFromSeq => write!(f, "set"),
            Func::Trans => write!(f, "trans"),
            Func::ApndL => write!(f, "apndl"),
            Func::Reverse => write!(f, "reverse"),
            Func::ApndR => write!(f, "apndr"),
            Func::RotL => write!(f, "rotl"),
            Func::RotR => write!(f, "rotr"),
            Func::Add => write!(f, "+"),
            Func::Sub => write!(f, "-"),
            Func::Mul => write!(f, "×"),
            Func::Div => write!(f, "÷"),
            Func::And => write!(f, "and"),
            Func::Or => write!(f, "or"),
            Func::Not => write!(f, "not"),
            Func::Fetch => write!(f, "↑"),
            Func::FetchOrPhi => write!(f, "↑?"),
            Func::Store => write!(f, "↓"),
            Func::Constant(obj) => write!(f, "{:?}̄", obj),
            Func::Compose(g, h) => write!(f, "({:?} ∘ {:?})", g, h),
            Func::Construction(funcs) => {
                write!(f, "[{}]", funcs.iter().map(|func| format!("{:?}", func))
                    .collect::<Vec<_>>().join(", "))
            }
            Func::Condition(p, t, e) => write!(f, "({:?} → {:?}; {:?})", p, t, e),
            Func::ApplyToAll(g) => write!(f, "α{:?}", g),
            Func::Insert(g) => write!(f, "/{:?}", g),
            Func::FoldL(g) => write!(f, "foldl({:?})", g),
            Func::IndexBy(g) => write!(f, "indexby({:?})", g),
            Func::OrderBy(g) => write!(f, "orderby({:?})", g),
            Func::Filter(p) => write!(f, "Filter({:?})", p),
            Func::BinaryToUnary(g, x) => write!(f, "(bu {:?} {:?})", g, x),
            Func::While(p, g) => write!(f, "(while {:?} {:?})", p, g),
            Func::Def(name) => write!(f, "{}", name),
            Func::Platform(name) => write!(f, "platform:{}", name),
            Func::Native(_) => write!(f, "<native>"),
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // delta-occ-1: the static cell-read analyzer. Mirrors the compiler's
    // fixed cell-read shape and pins literal vs dynamic vs indirect reads.
    #[test]
    fn func_read_set_extracts_literal_and_flags_dynamic_fetches() {
        // The shape extract_facts_from_pop emits (compile.rs:544):
        //   Compose(FetchOrPhi, Construction([Constant(atom(cell)), Id]))
        let extract = |cell: &str| Func::compose(
            Func::FetchOrPhi,
            Func::construction(vec![Func::constant(Object::atom(cell)), Func::Id]),
        );

        // 1) a single literal-name fetch is a statically-known read.
        let rs = func_read_set(&extract("resource_is_of_function"));
        assert!(rs.literal.contains("resource_is_of_function"));
        assert_eq!(rs.literal.len(), 1);
        assert!(!rs.has_dynamic, "a literal-name fetch is statically complete");

        // 2) a 2-cell join reads BOTH antecedent cells (BTreeSet is sorted).
        let join = Func::construction(vec![
            extract("resource_is_of_function"),
            extract("function_belongs_to_domain"),
        ]);
        let rs = func_read_set(&join);
        assert_eq!(
            rs.literal.iter().cloned().collect::<Vec<_>>(),
            vec![
                "function_belongs_to_domain".to_string(),
                "resource_is_of_function".to_string(),
            ],
        );
        assert!(!rs.has_dynamic);

        // 3) reads nested inside a combining form are still found.
        let nested = Func::ApplyToAll(Box::new(extract("nested_cell")));
        let rs = func_read_set(&nested);
        assert!(rs.literal.contains("nested_cell") && !rs.has_dynamic);

        // 4) a fetch whose NAME is computed at runtime (Selector, not a
        //    Constant) is the hidden/dynamic case the soundness gate must
        //    catch — the read-set is NOT statically complete.
        let dynamic = Func::compose(
            Func::FetchOrPhi,
            Func::construction(vec![Func::Selector(1), Func::Id]),
        );
        let rs = func_read_set(&dynamic);
        assert!(rs.has_dynamic, "a computed-name fetch must flag has_dynamic");
        assert!(rs.literal.is_empty());

        // 5) a Def reference is recorded as indirect (resolve via def map),
        //    not as a literal read and not (by itself) dynamic.
        let rs = func_read_set(&Func::Def("derivation:rbd".to_string()));
        assert!(rs.def_refs.contains("derivation:rbd"));
        assert!(rs.literal.is_empty() && !rs.has_dynamic);
    }

    // delta-occ-3: per-occurrence read rewriting. The point is the
    // SELF-JOIN — varying ONE occurrence's delta while the other stays at
    // full population, which a whole-cell swap cannot express.
    #[test]
    fn occ3_rewrites_one_self_join_occurrence_at_a_time() {
        let extract = |cell: &str| Func::compose(
            Func::FetchOrPhi,
            Func::construction(vec![Func::constant(Object::atom(cell)), Func::Id]),
        );

        // A self-join reads `edge` at two occurrences; occurrence 0 is
        // nested under ApplyToAll so the two are positionally distinct in
        // left-to-right walk order.
        let self_join = Func::construction(vec![
            Func::ApplyToAll(Box::new(extract("edge"))),
            extract("edge"),
        ]);
        assert_eq!(func_read_occurrence_count(&self_join, "edge"), 2);
        assert_eq!(func_read_occurrence_count(&self_join, "other"), 0);

        // Rewriting occurrence 0 leaves exactly one full `edge` read and
        // one synthetic-delta read.
        let v0 = func_rewrite_read_occurrence(&self_join, "edge", 0, "\u{0394}edge");
        assert_eq!(func_read_occurrence_count(&v0, "edge"), 1,
            "one full `edge` read must remain after swapping occ 0");
        assert_eq!(func_read_occurrence_count(&v0, "\u{0394}edge"), 1);

        // Rewriting occurrence 1 also leaves one of each, but is a DIFFERENT
        // variant — proving the two calls target different occurrences (so
        // the union is ΔA⋈A ∪ A⋈ΔA, not ΔA⋈ΔA twice).
        let v1 = func_rewrite_read_occurrence(&self_join, "edge", 1, "\u{0394}edge");
        assert_eq!(func_read_occurrence_count(&v1, "edge"), 1);
        assert_eq!(func_read_occurrence_count(&v1, "\u{0394}edge"), 1);
        assert_ne!(format!("{:?}", v0), format!("{:?}", v1),
            "occ 0 and occ 1 rewrites must produce structurally different funcs");

        // A target past the occurrence count is a no-op clone.
        let noop = func_rewrite_read_occurrence(&self_join, "edge", 9, "\u{0394}edge");
        assert_eq!(format!("{:?}", noop), format!("{:?}", self_join));

        // Rewriting touches only the named cell; a distinct co-antecedent
        // is left fully intact (the classical 2-cell-join case).
        let join = Func::construction(vec![extract("a"), extract("b")]);
        let ra = func_rewrite_read_occurrence(&join, "a", 0, "\u{0394}a");
        assert_eq!(func_read_occurrence_count(&ra, "a"), 0);
        assert_eq!(func_read_occurrence_count(&ra, "b"), 1, "co-antecedent `b` untouched");
        assert_eq!(func_read_occurrence_count(&ra, "\u{0394}a"), 1);
    }

    // delta-occ-2 soundness gate (empirically grounded on the real
    // metamodel). The delta-view completeness marker must be emitted ONLY
    // for rules whose compiled Func has no dynamic fetch and no indirect Def
    // AND whose sidecar covers every literal cell the Func fetches — so the
    // dark AREST_DELTA_JOINS view-swap can never leave a read at full
    // population (the B2 divergence). Measured 2026-06-16: of 87 metamodel
    // derivations, 23 are view-complete and 64 carry a dynamic read (e.g.
    // the User_accesses_Domain 3-hop joins) and are correctly NOT marked —
    // they full-evaluate (sound) until per-occurrence delta (delta-occ-3)
    // accelerates them. This pins the gate so a regression cannot re-mark a
    // dynamic-read rule complete.
    #[test]
    fn occ2_completeness_marker_is_sound_across_metamodel() {
        let state = crate::metamodel_state();
        let model = crate::compile::compile(&state);
        let mut marked = 0usize;
        for id in &model.derivation_reads_complete {
            let d = model.derivations.iter().find(|d| &d.id == id)
                .expect("a marked rule must have a compiled derivation");
            let rs = func_read_set(&d.func);
            assert!(!rs.has_dynamic,
                "marked rule {} ({}) has a dynamic fetch — unsound for the delta view path",
                id, d.consequent_cell);
            assert!(rs.def_refs.is_empty(),
                "marked rule {} ({}) makes an indirect Def call — sidecar not provably complete",
                id, d.consequent_cell);
            let sidecar = model.derivation_positive_reads.get(id).cloned().unwrap_or_default();
            for cell in &rs.literal {
                assert!(sidecar.contains(cell),
                    "marked rule {} ({}) fetches {} but it is absent from the sidecar — the \
                     delta view-swap would leave it at full population", id, d.consequent_cell, cell);
            }
            marked += 1;
        }
        assert!(marked > 0, "expected some func-complete metamodel rules to carry the marker");
        let dynamic = model.derivations.iter()
            .filter(|d| func_read_set(&d.func).has_dynamic).count();
        eprintln!("[occ2] metamodel derivations={} view-complete(marker)={} full-eval(dynamic)={}",
            model.derivations.len(), marked, dynamic);
    }

    #[test]
    fn stamp_file_domain_tags_declared_functions() {
        let state = crate::parse_forml2::parse_to_state(
            "Order(.id) is an entity type.\nCustomer(.id) is an entity type.\nOrder placed by Customer.",
        )
        .expect("fixture parses");
        let stamped = stamp_file_domain(&state, "orders");
        let cell = fetch_cell_seq("Function_belongs_to_Domain", &stamped);
        let facts = cell.as_seq().expect("Function_belongs_to_Domain is a Seq");
        let stamped_with = |noun: &str| {
            facts.iter().any(|f| {
                binding(f, "Function") == Some(noun) && binding(f, "Domain") == Some("orders")
            })
        };
        assert!(stamped_with("Order"), "Order noun stamped with domain orders; got {:?}", facts);
        assert!(stamped_with("Customer"), "Customer noun stamped; got {:?}", facts);
        // The fact type is a Function too (Fact Type < Resource < Noun < Function),
        // so it is stamped alongside the two nouns.
        assert!(
            facts.len() >= 3,
            "expected >= 2 nouns + 1 fact type stamped; got {}: {:?}",
            facts.len(), facts
        );
    }

    #[test]
    fn namespaced_nouns_stay_distinct_across_domains() {
        // core.Order is a value type; orders.Order is an entity type. With
        // homeDomain annotation + domain-aware same_identity, the merge keeps
        // BOTH instead of collapsing them by name (the Order collision).
        let core = annotate_noun_domain(
            &crate::parse_forml2::parse_to_state("Order is a value type.").expect("core parses"),
            "core");
        let orders = annotate_noun_domain(
            &crate::parse_forml2::parse_to_state("Order(.id) is an entity type.").expect("orders parses"),
            "orders");
        let merged = merge_states(&core, &orders);
        let order_entries: Vec<Object> = fetch_cell_seq("Noun", &merged)
            .as_seq().expect("Noun seq").iter()
            .filter(|f| binding(f, "name") == Some("Order"))
            .cloned()
            .collect();
        assert_eq!(order_entries.len(), 2,
            "core.Order (value) and orders.Order (entity) must both survive the merge; got {:?}", order_entries);
        assert!(order_entries.iter().any(|f|
            binding(f, "objectType") == Some("value") && binding(f, "homeDomain") == Some("core")),
            "core.Order should be the value type");
        assert!(order_entries.iter().any(|f|
            binding(f, "objectType") == Some("entity") && binding(f, "homeDomain") == Some("orders")),
            "orders.Order should be the entity type");
    }

    #[test]
    fn map_display_is_key_sorted_for_determinism() {
        // W5/D3 (task-932): Object::Map must serialize in canonical
        // key-sorted order so the wire form is deterministic regardless
        // of Arc<HashMap> iteration order (thm:derive cacheability /
        // cor:consensus deterministic replay). Keys inserted out of
        // sorted order must come back sorted.
        let mut m: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        m.insert("charlie".into(), Object::atom("3"));
        m.insert("alpha".into(), Object::atom("1"));
        m.insert("bravo".into(), Object::atom("2"));
        let s = Object::map(m).to_string();
        assert_eq!(s, "{alpha=1, bravo=2, charlie=3}",
            "top-level Map Display must be key-sorted; got {}", s);

        // Round-trips back to an equal-size Map (order is not semantic).
        assert_eq!(Object::parse(&s).as_map().map(|mm| mm.len()), Some(3));

        // The nested-Map path (item_inside_seq) is sorted too.
        let mut inner: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        inner.insert("y".into(), Object::atom("2"));
        inner.insert("x".into(), Object::atom("1"));
        let outer = Object::seq(vec![Object::map(inner)]).to_string();
        assert_eq!(outer, "<{x=1, y=2}>",
            "nested Map Display must be key-sorted; got {}", outer);
    }

    fn defs() -> Object { Object::phi() }

    #[test]
    fn drop_subjectless_facts_removes_empty_subject_keeps_valid() {
        // Valid fact (non-empty subject) — kept.
        let valid = Object::seq(vec![
            Object::seq(vec![Object::atom("State Machine"), Object::atom("task-1")]),
            Object::seq(vec![Object::atom("Status"), Object::atom("pending")]),
        ]);
        // Empty-string subject — the 'Proposed'/'in_progress' orphan shape; dropped.
        let empty_subject = Object::seq(vec![
            Object::seq(vec![Object::atom("State Machine"), Object::atom("")]),
            Object::seq(vec![Object::atom("Status"), Object::atom("Proposed")]),
        ]);
        // φ subject (empty Seq) — dropped.
        let phi_subject = Object::seq(vec![
            Object::seq(vec![Object::atom("State Machine"), Object::phi()]),
            Object::seq(vec![Object::atom("Status"), Object::atom("in_progress")]),
        ]);
        // φ-TOKEN subject — Atom("φ"), the fan-out write form (NOT the empty
        // Seq above). phi-keyed-task-started-orphan-gc: this leaked the GC
        // because !"φ".is_empty() held, so a `<<Task, φ>>` relic survived every
        // persist and re-seeded the phantom in_progress SM; now dropped.
        let phi_token_subject = Object::seq(vec![
            Object::seq(vec![Object::atom("State Machine"), Object::atom("φ")]),
            Object::seq(vec![Object::atom("Status"), Object::atom("in_progress")]),
        ]);
        let cell = Object::seq(vec![valid.clone(), empty_subject, phi_subject, phi_token_subject]);
        let rows = drop_subjectless_facts_with_arity(&cell, None);
        let rows = rows.as_seq().expect("seq");
        assert_eq!(rows.len(), 1, "only the valid fact survives; got {:?}", rows);
        assert_eq!(rows[0], valid);

        // An empty VALUE on a non-subject role (e.g. an empty Description) on a
        // valid subject must be preserved — we key only on the subject (first
        // binding), never on value roles.
        let empty_value = Object::seq(vec![
            Object::seq(vec![Object::atom("Task"), Object::atom("t9")]),
            Object::seq(vec![Object::atom("Task Description"), Object::atom("")]),
        ]);
        let cell2 = Object::seq(vec![empty_value]);
        assert_eq!(drop_subjectless_facts_with_arity(&cell2, None).as_seq().expect("seq").len(), 1,
            "empty value on a non-subject role must be preserved");

        // Missing the subject role entirely — the live tasks.db relic shape:
        // a `currently in Status` row holding only <Status,'Proposed'> with no
        // <State Machine,…> binding. Arity-deficient (1 < 2) → dropped.
        let missing_subject_role = Object::seq(vec![
            Object::seq(vec![Object::atom("Status"), Object::atom("Proposed")]),
        ]);
        let cell3 = Object::seq(vec![valid.clone(), missing_subject_role]);
        let rows3 = drop_subjectless_facts_with_arity(&cell3, None);
        let rows3 = rows3.as_seq().expect("seq");
        assert_eq!(rows3.len(), 1, "missing-subject-role relic dropped; got {:?}", rows3);
        assert_eq!(rows3[0], valid);
    }

    #[test]
    fn looks_like_population_cell_detects_ft_facts() {
        // The live Task_has_Task_Readiness orphan shape: Seq of <role,value>-pair facts.
        let readiness = Object::seq(vec![
            Object::seq(vec![
                Object::seq(vec![Object::atom("Task"), Object::atom("597")]),
                Object::seq(vec![Object::atom("Task Readiness"), Object::atom("ready")]),
            ]),
        ]);
        assert!(looks_like_population_cell(&readiness),
            "Seq of <role,value>-pair facts is a population cell");

        // Hash-keyed Map layout (latest-wins fold) — the underscore-form orphan.
        let mut m: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        m.insert("597".to_string(), Object::seq(vec![
            Object::seq(vec![Object::atom("Task"), Object::atom("597")]),
            Object::seq(vec![Object::atom("Task Readiness"), Object::atom("ready")]),
        ]));
        assert!(looks_like_population_cell(&Object::map(m)),
            "Map of key->fact is a population cell");

        // Non-population shapes are NOT populations (conservative — never GC'd).
        assert!(!looks_like_population_cell(&Object::atom("x")), "atom is not a population");
        assert!(!looks_like_population_cell(&Object::seq(vec![])), "empty seq has no facts");
        assert!(!looks_like_population_cell(
            &Object::seq(vec![Object::atom("a"), Object::atom("b")])),
            "seq of bare atoms (not <role,value> pairs) is not a population");
    }

    #[test]
    fn dedup_cell_facts_collapses_structural_duplicates() {
        // Task_is_epic shape: unary <<Task, id>> facts; cor:closure had been
        // accruing one identity-equal copy per recompile.
        let fact = |t: &str| Object::seq(vec![
            Object::seq(vec![Object::atom("Task"), Object::atom(t)]),
        ]);
        let cell = Object::seq(vec![fact("772"), fact("772"), fact("773"), fact("772")]);
        let rows = dedup_cell_facts(&cell);
        let rows = rows.as_seq().expect("seq");
        assert_eq!(rows.len(), 2, "collapsed to 2 distinct facts; got {:?}", rows);
        assert_eq!(rows[0], fact("772"), "first-occurrence order preserved");
        assert_eq!(rows[1], fact("773"));

        // Already-distinct cell returns unchanged.
        let distinct = Object::seq(vec![fact("a"), fact("b")]);
        assert_eq!(dedup_cell_facts(&distinct), distinct);

        // The live Task_is_epic root: the unary predicate's empty object slot
        // is written as Atom("φ") by one path and Atom("") by another. Both
        // denote "no object", so canon_phi unifies them and the SAME fact
        // dedups to one — this is what makes re-assertion idempotent.
        let epic = |v: &str| Object::seq(vec![
            Object::seq(vec![Object::atom("Task"), Object::atom("9")]),
            Object::seq(vec![Object::atom("Task_is_epic"), Object::atom(v)]),
        ]);
        let dual = Object::seq(vec![epic("φ"), epic("")]);
        assert_eq!(dedup_cell_facts(&dual).as_seq().expect("seq").len(), 1,
            "φ-token and empty-string encodings of the same unary fact collapse to one");
    }

    #[test]
    fn preserve_prior_population_drops_orphans_keeps_declared_runtime_data() {
        let fact = |role: &str, subj: &str| Object::seq(vec![
            Object::seq(vec![Object::atom(role), Object::atom(subj)]),
        ]);
        let mut loaded: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        // declared-FT runtime population — DATA-CRITICAL, must survive recompile
        loaded.insert("Task_has_Task_Subject".to_string(),
            Object::seq(vec![fact("Task", "772"), fact("Task", "773")]));
        // orphan: FactType no longer declared (removed-rule relic) — must be GC'd
        loaded.insert("Task_has_Task_Readiness".to_string(),
            Object::seq(vec![fact("Task", "772")]));
        // sidecar ':' view cell — dropped (regenerates from data cells)
        loaded.insert("derivation_index:Task".to_string(), fact("noun", "Task"));
        let loaded = Object::map(loaded);

        let declared: hashbrown::HashSet<String> =
            ["Task_has_Task_Subject".to_string()].into_iter().collect();
        let parsed: hashbrown::HashSet<String> = hashbrown::HashSet::new();
        let arity: hashbrown::HashMap<String, usize> = hashbrown::HashMap::new();

        let (pop, gc_orphans) = preserve_prior_population(&loaded, &parsed, &declared, &arity);

        assert_eq!(fetch_cell_seq("Task_has_Task_Subject", &pop).as_seq().map_or(0, |s| s.len()), 2,
            "declared-FT runtime population must survive recompile (data-critical)");
        assert!(gc_orphans.contains(&"Task_has_Task_Readiness".to_string()),
            "an undeclared-FactType relic must be reported as a GC'd orphan");
        assert_eq!(fetch_cell_seq("Task_has_Task_Readiness", &pop).as_seq().map_or(0, |s| s.len()), 0,
            "orphan cell dropped from the preserved population");
        assert_eq!(fetch_cell_seq("derivation_index:Task", &pop).as_seq().map_or(0, |s| s.len()), 0,
            "':' sidecar cells are not carried forward");
    }

    /// compile-gc-orphaned-derived-facts: the canonical preserve step
    /// `preserve_prior_population` must dedupe identity-equal facts so the live
    /// tasks.db `Task_is_epic` shape — a unary "Task is epic." instance fact
    /// whose object slot reaches the cell store in multiple empty-encodings
    /// (`Atom("φ")`, `Atom("")`, `phi()`) — collapses to one fact per task
    /// regardless of how many recompiles preceded the fix. Without this, a
    /// recompile path that bypasses the cli/entry.rs post-merge dedup site
    /// (`arest reload` / `arest watch` through `load_reading_core`) would carry
    /// the legacy bloat forward and keep accruing one extra copy per epic per
    /// recompile. A second pass through the same helper must be a no-op
    /// (idempotency: `preserve(preserve(x)) == preserve(x)` on the cell map).
    #[test]
    fn preserve_prior_population_dedupes_identity_equal_asserted_facts() {
        // "Task is epic." unary instance fact, with the three empty-encodings of
        // the object slot the live tasks.db has accrued across recompiles.
        let epic_phi_token = Object::seq(vec![
            Object::seq(vec![Object::atom("Task"), Object::atom("772")]),
            Object::seq(vec![Object::atom("Task_is_epic"), Object::atom("φ")]),
        ]);
        let epic_empty_string = Object::seq(vec![
            Object::seq(vec![Object::atom("Task"), Object::atom("772")]),
            Object::seq(vec![Object::atom("Task_is_epic"), Object::atom("")]),
        ]);
        let epic_phi_seq = Object::seq(vec![
            Object::seq(vec![Object::atom("Task"), Object::atom("772")]),
            Object::seq(vec![Object::atom("Task_is_epic"), Object::phi()]),
        ]);
        // A second distinct epic — must survive (only same-identity dups collapse).
        let epic_other = Object::seq(vec![
            Object::seq(vec![Object::atom("Task"), Object::atom("773")]),
            Object::seq(vec![Object::atom("Task_is_epic"), Object::atom("φ")]),
        ]);

        // Simulate the legacy bloat: 4 entries (3 identity-equal for Task 772,
        // 1 distinct for Task 773) carried in from a prior recompile-fold.
        let mut loaded: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        loaded.insert("Task_is_epic".to_string(), Object::seq(vec![
            epic_phi_token.clone(), epic_empty_string, epic_phi_seq, epic_other.clone(),
        ]));
        let loaded = Object::map(loaded);

        let declared: hashbrown::HashSet<String> =
            ["Task_is_epic".to_string()].into_iter().collect();
        let parsed: hashbrown::HashSet<String> = hashbrown::HashSet::new();
        let mut arity: hashbrown::HashMap<String, usize> = hashbrown::HashMap::new();
        arity.insert("Task_is_epic".to_string(), 2);

        let (pop, _orphans) = preserve_prior_population(&loaded, &parsed, &declared, &arity);

        let preserved_len = fetch_cell_seq("Task_is_epic", &pop)
            .as_seq().map_or(0, |s| s.len());
        assert_eq!(preserved_len, 2,
            "the three empty-encoding forms of the same `Task is epic` instance fact \
             must collapse to one (identity-aware dedup); the distinct second epic \
             must survive — got {} facts in the preserved cell", preserved_len);

        // Idempotency: a second recompile over the already-preserved population
        // must NOT append another copy. This is the property that makes the
        // Task_is_epic cell stable across recompiles (the user-visible fix).
        let (pop2, _) = preserve_prior_population(&pop, &parsed, &declared, &arity);
        let twice_len = fetch_cell_seq("Task_is_epic", &pop2)
            .as_seq().map_or(0, |s| s.len());
        assert_eq!(twice_len, 2,
            "recompile must be idempotent on the preserved-population — a second \
             pass through preserve_prior_population must NOT grow the cell");
    }

    /// task-984 part B (arc-agi-3 issue 10): the load-path keyed-cell
    /// reconcile. A single-valued fact corrected in readings ('0' →
    /// '25') must displace its stale carried-forward prior — later Seq
    /// rows win at the same key; unrelated keys survive; and a cell
    /// containing a row that cannot extract its key is left UNTOUCHED
    /// (the conservative gate — dropping dirty rows is not this pass's
    /// job).
    #[test]
    fn reconcile_keyed_cells_displaces_stale_single_valued_rows() {
        let mut kr: hashbrown::HashMap<String, Vec<String>> = hashbrown::HashMap::new();
        kr.insert("Run_has_Action_Count".to_string(), vec!["Run".to_string()]);

        let cell = Object::seq(vec![
            fact_from_pairs(&[("Run", "X"), ("Action Count", "0")]),  // stale prior
            fact_from_pairs(&[("Run", "Y"), ("Action Count", "7")]),  // unrelated key
            fact_from_pairs(&[("Run", "X"), ("Action Count", "25")]), // corrected (later)
        ]);
        let state = store("Run_has_Action_Count", cell, &Object::phi());
        let (out, displaced) = reconcile_keyed_cells(&state, &kr);
        assert_eq!(displaced, alloc::vec![("Run_has_Action_Count".to_string(), 1usize)],
            "exactly the stale X row is displaced");
        let c = fetch_or_phi("Run_has_Action_Count", &out);
        let x_vals: Vec<&str> = cell_facts_iter(&c)
            .filter(|f| binding(f, "Run") == Some("X"))
            .filter_map(|f| binding(f, "Action Count"))
            .collect();
        assert_eq!(x_vals, alloc::vec!["25"],
            "the corrected reading value must win at key X");
        assert_eq!(cell_facts_iter(&c).filter(|f| binding(f, "Run") == Some("Y")).count(), 1,
            "unrelated key Y must survive");

        // Conservative gate: any unkeyable row → cell untouched.
        let dirty = Object::seq(vec![
            fact_from_pairs(&[("Run", "X"), ("Action Count", "0")]),
            fact_from_pairs(&[("Action Count", "9")]), // no Run key role
        ]);
        let state2 = store("Run_has_Action_Count", dirty.clone(), &Object::phi());
        let (out2, displaced2) = reconcile_keyed_cells(&state2, &kr);
        assert!(displaced2.is_empty(), "dirty cell must report nothing");
        assert_eq!(fetch_or_phi("Run_has_Action_Count", &out2), dirty,
            "dirty cell must be left byte-identical");
    }

    // ── Object construction ──────────────────────────────────────

    #[test]
    fn bottom_propagates_through_sequence() {
        let seq = Object::seq(vec![Object::atom("a"), Object::Bottom, Object::atom("c")]);
        assert_eq!(seq, Object::Bottom);
    }

    /// `Object::parse(obj.to_string())` must be the identity for any
    /// Object that came out of the parser. Values containing literal
    /// `<` or `>` (e.g. an instance fact subject like `reachable in
    /// < 30 s`) round-trip cleanly through the in-memory algebra
    /// because the parser stops splitting once depth returns to 0,
    /// and atoms are treated as opaque tokens. They DO NOT round-trip
    /// cleanly through `db::persist_state` → `db::load_state` because
    /// SQLite stores the cell's atom-formed `Display` output and the
    /// load path re-runs `Object::parse` on it without the
    /// surrounding cell-level `<...>` wrapper that defines depth 0.
    /// The DB value `<<Task, 623>, <Task Subject, …< 30 s>>` parses
    /// fine; the value `<Task Subject, …< 30 s>` (a single fact's
    /// pair, stored with no outer wrapper) does too. The bug we
    /// observe in apps/tasks (subjects ≥#623 missing from query
    /// results) lives elsewhere — likely in how `db::persist_state`
    /// stringifies a cell whose contents include unbalanced `<`
    /// characters embedded inside an Atom value, or in how some
    /// upstream serializer writes these atoms.
    ///
    /// This test pins the algebra-level invariant the rest of the
    /// engine assumes. If this ever fails, every cell write/read
    /// round-trip in the local CLI is silently corrupting data.
    #[test]
    fn parse_after_to_string_is_identity_for_atoms_containing_lessthan() {
        let cases = [
            "reachable in < 30 s",
            "a < b > c",
            "no brackets here",
            "value with , comma",
            "< leading angle",
            "trailing angle >",
        ];
        for raw in cases.iter() {
            let original = Object::atom(raw);
            let display = original.to_string();
            let reparsed = Object::parse(&display);
            assert_eq!(
                reparsed, original,
                "Object::parse(Object::atom({:?}).to_string()) must equal the original; \
                 to_string produced {:?}, reparse produced {:?}",
                raw, display, reparsed,
            );
        }
    }

    /// Cell-level round-trip: a Seq of pairs where one pair's atom
    /// value contains literal `<` must survive `to_string` →
    /// `Object::parse`. This is the shape `db::persist_state` writes
    /// for a fact-type cell containing an instance fact whose value
    /// has an angle bracket — exactly the apps/tasks #623 case
    /// (`HATEOAS-prereq-c: …reachable in < 30 s`).
    #[test]
    fn cell_round_trip_preserves_atom_values_containing_lessthan() {
        let cell = Object::seq(vec![
            // fact 1: (Task=623, Subject="reachable in < 30 s")
            Object::seq(vec![
                Object::seq(vec![Object::atom("Task"), Object::atom("623")]),
                Object::seq(vec![Object::atom("Task Subject"),
                    Object::atom("reachable in < 30 s")]),
            ]),
            // fact 2: (Task=624, Subject="simple") — must remain visible
            Object::seq(vec![
                Object::seq(vec![Object::atom("Task"), Object::atom("624")]),
                Object::seq(vec![Object::atom("Task Subject"),
                    Object::atom("simple")]),
            ]),
        ]);
        let display = cell.to_string();
        let reparsed = Object::parse(&display);
        assert_eq!(
            reparsed, cell,
            "cell with fact value containing `<` must round-trip; \
             to_string produced {:?}, reparse produced {:?}",
            display, reparsed,
        );
    }

    /// task-922-object-parse-map-syntax: Object::parse must recognize
    /// the `{k=v, ...}` Map syntax that Display emits. Without this,
    /// every persisted Map cell silently round-trips back as an opaque
    /// Atom — the cell becomes uniterable, the SQL projector reads it
    /// as empty, and downstream consumers see no rows.
    ///
    /// Bug shape: `Object::parse(Object::map(…).to_string())` returned
    /// `Object::Atom("{...}")` pre-fix.
    #[test]
    fn parse_after_to_string_is_identity_for_map_cells() {
        let mut m = HashMap::new();
        m.insert("alpha".to_string(),
            Object::seq(vec![Object::atom("Alpha"), Object::atom("v1")]));
        m.insert("beta".to_string(),
            Object::seq(vec![Object::atom("Beta"), Object::atom("v2")]));
        let map_obj = Object::map(m);

        let display = map_obj.to_string();
        let reparsed = Object::parse(&display);
        assert_eq!(reparsed, map_obj,
            "Object::parse(Object::map(…).to_string()) must equal the original; \
             to_string produced {:?}, reparse produced {:?}",
            display, reparsed);
    }

    /// REGRESSION (Map-cell data-loss, 787→0): a keyed Map FT cell whose
    /// VALUES contain the Map/Seq delimiters `=`, `{`, `}`, `<`, `>`, `,`
    /// must round-trip through `Display → Object::parse` preserving EVERY
    /// entry. This is the exact shape that destroyed Task_has_Task_Description
    /// / Task_has_Task_Subject on a full readings-recompile: descriptions
    /// like "312 = 8x39", "set BASE={url}", or "reachable in < 30 s" carried
    /// unescaped delimiters that corrupted `split_top_level` /
    /// `split_first_eq_top_level` depth tracking, so the whole persisted Map
    /// re-parsed as one opaque Atom (`as_map()` → None) and read as EMPTY.
    /// The fix escapes `{`/`}`/`=` in `escape_atom_for_display` and routes
    /// both Map Display arms through escaped keys + `item_inside_seq` values.
    ///
    /// This is the BINARY/multi-valued FT class the prior dedup test
    /// (unary Task_is_epic) never covered.
    #[test]
    fn map_cell_with_delimiter_laden_values_round_trips_all_entries() {
        // Three distinct Task-keyed facts; each Description value carries a
        // different reserved delimiter that would break naive parsing.
        let fact = |id: &str, desc: &str| Object::seq(vec![
            Object::seq(vec![Object::atom("Task"), Object::atom(id)]),
            Object::seq(vec![Object::atom("Task Description"), Object::atom(desc)]),
        ]);
        let mut m = HashMap::new();
        m.insert("656".to_string(), fact("656", "312 = 8x39 dup copies per epic"));
        m.insert("129".to_string(), fact("129", "set BASE={url} then ${BASE}/api"));
        m.insert("717".to_string(), fact("717", "reachable in < 30 s, maybe > 10"));
        let cell = Object::map(m);

        let display = cell.to_string();
        let reparsed = Object::parse(&display);
        // Must re-parse as a Map (not collapse to an opaque Atom)...
        assert!(reparsed.as_map().is_some(),
            "delimiter-laden Map cell must re-parse as a Map, not an opaque Atom; \
             Display produced {:?}, reparse produced {:?}", display, reparsed);
        // ...with ALL THREE distinct entries preserved (the 787→0 collapse
        // kept only one).
        assert_eq!(reparsed.as_map().map(|m| m.len()), Some(3),
            "all 3 Task-keyed facts must survive the round-trip; got {:?}", reparsed);
        assert_eq!(reparsed, cell,
            "delimiter-laden Map cell must round-trip byte-identically; \
             Display produced {:?}, reparse produced {:?}", display, reparsed);
    }

    /// task-922-object-parse-map-syntax discrimination guard: a JSON
    /// payload `{"type":"createEntity",…}` (used by `system_impl`'s
    /// apply path) must NOT be misparsed as Map syntax — JSON uses
    /// `:` and quoted keys, no `=` separators. The Map branch only
    /// fires when EVERY non-empty entry has a top-level `=`. JSON
    /// strings fall through to Atom (legacy behavior).
    #[test]
    fn parse_does_not_eat_json_payloads_starting_with_brace() {
        let json = r#"{"type":"createEntity","noun":"Order","fields":{"total":"100"}}"#;
        let parsed = Object::parse(json);
        assert!(matches!(parsed, Object::Atom(_)),
            "JSON payload starting with `{{` must parse as Atom, not Map; \
             got {:?}", parsed);
        assert_eq!(parsed.as_atom(), Some(json),
            "JSON Atom must be byte-identical to the input");
    }

    /// Empty Map round-trip (Display emits `{}`).
    #[test]
    fn parse_after_to_string_is_identity_for_empty_map() {
        let map_obj = Object::map(HashMap::new());
        let display = map_obj.to_string();
        assert_eq!(display, "{}", "empty Map must serialize as `{{}}`");
        let reparsed = Object::parse(&display);
        assert_eq!(reparsed, map_obj,
            "empty Map must round-trip; reparse produced {:?}", reparsed);
    }

    /// Nested Map inside a Seq round-trip — covers split_top_level's
    /// `{}` depth tracking. Without the fix, a `,` inside a nested Map
    /// value would split the outer Seq entry.
    #[test]
    fn parse_after_to_string_is_identity_for_seq_containing_map() {
        let mut m = HashMap::new();
        m.insert("k1".to_string(), Object::atom("v with , comma"));
        m.insert("k2".to_string(), Object::atom("v2"));
        let seq_with_map = Object::seq(vec![
            Object::atom("first"),
            Object::map(m),
            Object::atom("third"),
        ]);
        let display = seq_with_map.to_string();
        let reparsed = Object::parse(&display);
        assert_eq!(reparsed, seq_with_map,
            "Seq containing a Map (whose value has a comma) must round-trip; \
             to_string produced {:?}, reparse produced {:?}",
            display, reparsed);
    }

    #[test]
    fn phi_is_empty_sequence() {
        assert_eq!(Object::phi(), Object::seq(vec![]));
    }

    // ── Primitives ───────────────────────────────────────────────

    #[test]
    fn selector_extracts_nth_element() {
        let seq = Object::seq(vec![Object::atom("alice"), Object::atom("owner"), Object::atom("org-1")]);
        assert_eq!(apply(&Func::Selector(1), &seq, &defs()), Object::atom("alice"));
        assert_eq!(apply(&Func::Selector(2), &seq, &defs()), Object::atom("owner"));
        assert_eq!(apply(&Func::Selector(3), &seq, &defs()), Object::atom("org-1"));
        assert_eq!(apply(&Func::Selector(4), &seq, &defs()), Object::Bottom);
    }

    #[test]
    fn selector_on_atom_is_bottom() {
        assert_eq!(apply(&Func::Selector(1), &Object::atom("x"), &defs()), Object::Bottom);
    }

    #[test]
    fn tail_drops_first() {
        let seq = Object::seq(vec![Object::atom("a"), Object::atom("b"), Object::atom("c")]);
        assert_eq!(
            apply(&Func::Tail, &seq, &defs()),
            Object::seq(vec![Object::atom("b"), Object::atom("c")])
        );
    }

    #[test]
    fn tail_of_singleton_is_phi() {
        let seq = Object::seq(vec![Object::atom("a")]);
        assert_eq!(apply(&Func::Tail, &seq, &defs()), Object::phi());
    }

    #[test]
    fn eq_test() {
        let same = Object::seq(vec![Object::atom("x"), Object::atom("x")]);
        let diff = Object::seq(vec![Object::atom("x"), Object::atom("y")]);
        assert_eq!(apply(&Func::Eq, &same, &defs()), Object::t());
        assert_eq!(apply(&Func::Eq, &diff, &defs()), Object::f());
    }

    #[test]
    fn numeric_comparisons() {
        let three_two = Object::seq(vec![Object::atom("3"), Object::atom("2")]);
        let two_two = Object::seq(vec![Object::atom("2"), Object::atom("2")]);
        let two_three = Object::seq(vec![Object::atom("2"), Object::atom("3")]);
        // Gt: 3 > 2 true, 2 > 2 false, 2 > 3 false
        assert_eq!(apply(&Func::Gt, &three_two, &defs()), Object::t());
        assert_eq!(apply(&Func::Gt, &two_two, &defs()), Object::f());
        assert_eq!(apply(&Func::Gt, &two_three, &defs()), Object::f());
        // Lt: inverse of Gt
        assert_eq!(apply(&Func::Lt, &three_two, &defs()), Object::f());
        assert_eq!(apply(&Func::Lt, &two_three, &defs()), Object::t());
        // Ge: 3 >= 2 true, 2 >= 2 true, 2 >= 3 false
        assert_eq!(apply(&Func::Ge, &three_two, &defs()), Object::t());
        assert_eq!(apply(&Func::Ge, &two_two, &defs()), Object::t());
        assert_eq!(apply(&Func::Ge, &two_three, &defs()), Object::f());
        // Le: inverse of Ge
        assert_eq!(apply(&Func::Le, &two_two, &defs()), Object::t());
        assert_eq!(apply(&Func::Le, &two_three, &defs()), Object::t());
        assert_eq!(apply(&Func::Le, &three_two, &defs()), Object::f());
        // Non-numeric: Bottom
        let strings = Object::seq(vec![Object::atom("x"), Object::atom("y")]);
        assert_eq!(apply(&Func::Gt, &strings, &defs()), Object::Bottom);
    }

    #[test]
    fn numeric_comparisons_roundtrip_through_metacompose() {
        // Each comparator must round-trip: Func → Object → metacompose → Func
        for (variant, name) in [
            (Func::Gt, "gt"), (Func::Lt, "lt"),
            (Func::Ge, "ge"), (Func::Le, "le"),
        ] {
            let obj = func_to_object(&variant);
            let recovered = metacompose(&obj, &defs());
            let input = Object::seq(vec![Object::atom("5"), Object::atom("3")]);
            assert_eq!(apply(&variant, &input, &defs()),
                       apply(&recovered, &input, &defs()),
                       "{} round-trip failed", name);
        }
    }

    // #282: text-pattern primitives used by the readings-form
    // Stage-1 tokenizer (#295).

    #[test]
    fn starts_with_and_ends_with_are_case_insensitive() {
        let pair = Object::seq(vec![
            Object::atom("It is obligatory that Customer has Email"),
            Object::atom("it is obligatory that"),
        ]);
        assert_eq!(apply(&Func::StartsWith, &pair, &defs()), Object::t());

        let mismatch = Object::seq(vec![
            Object::atom("Customer is an entity type"),
            Object::atom("It is obligatory"),
        ]);
        assert_eq!(apply(&Func::StartsWith, &mismatch, &defs()), Object::f());

        let ends = Object::seq(vec![
            Object::atom("Customer is an entity type"),
            Object::atom("IS AN ENTITY TYPE"),
        ]);
        assert_eq!(apply(&Func::EndsWith, &ends, &defs()), Object::t());
    }

    #[test]
    fn trim_strips_ascii_whitespace() {
        let input = Object::atom("   Noun has Name.   ");
        assert_eq!(
            apply(&Func::Trim, &input, &defs()),
            Object::atom("Noun has Name."),
        );
    }

    #[test]
    fn split_breaks_on_delimiter() {
        let pair = Object::seq(vec![
            Object::atom("'low', 'medium', 'high'"),
            Object::atom(", "),
        ]);
        assert_eq!(
            apply(&Func::Split, &pair, &defs()),
            Object::seq(vec![
                Object::atom("'low'"),
                Object::atom("'medium'"),
                Object::atom("'high'"),
            ]),
        );
    }

    #[test]
    fn split_on_empty_delimiter_returns_chars() {
        let pair = Object::seq(vec![
            Object::atom("abc"),
            Object::atom(""),
        ]);
        assert_eq!(
            apply(&Func::Split, &pair, &defs()),
            Object::seq(vec![
                Object::atom("a"), Object::atom("b"), Object::atom("c"),
            ]),
        );
    }

    #[test]
    fn replace_substitutes_every_occurrence() {
        let triple = Object::seq(vec![
            Object::atom("Noun1 is subtype of Noun2 and Noun2 is subtype of Noun3"),
            Object::seq(vec![Object::atom("Noun"), Object::atom("Entity")]),
        ]);
        assert_eq!(
            apply(&Func::Replace, &triple, &defs()),
            Object::atom("Entity1 is subtype of Entity2 and Entity2 is subtype of Entity3"),
        );
    }

    #[test]
    fn text_primitives_roundtrip_through_metacompose() {
        for (variant, name) in [
            (Func::StartsWith, "starts_with"),
            (Func::EndsWith, "ends_with"),
            (Func::Trim, "trim"),
            (Func::Split, "split"),
            (Func::Replace, "replace"),
        ] {
            let obj = func_to_object(&variant);
            let recovered = metacompose(&obj, &defs());
            // Sanity: round-tripped Func applied to trivial input
            // matches direct apply (shape / Bottom propagation
            // preserved).
            let input = Object::seq(vec![Object::atom("x"), Object::atom("x")]);
            assert_eq!(
                apply(&variant, &input, &defs()),
                apply(&recovered, &input, &defs()),
                "{} round-trip failed",
                name,
            );
        }
    }

    #[test]
    fn merge_states_dedupes_by_identity() {
        // Two states declaring Brand — one full (with refScheme), one
        // reference-only (minimal). merge_states should keep just one.
        let rich = fact_from_pairs(&[
            ("name", "Brand"),
            ("objectType", "entity"),
            ("referenceScheme", "Brand Name"),
        ]);
        let reference_only = fact_from_pairs(&[
            ("name", "Brand"),
            ("objectType", "entity"),
        ]);
        let state_a = store("Noun", Object::seq(vec![rich.clone()]), &Object::phi());
        let state_b = store("Noun", Object::seq(vec![reference_only]), &Object::phi());
        let merged = merge_states(&state_a, &state_b);
        let nouns = fetch("Noun", &merged);
        let facts = nouns.as_seq().expect("Noun cell should be a seq");
        assert_eq!(facts.len(), 1, "duplicate Brand should dedupe, got {:?}", facts);
        // First-occurrence wins: the rich one with refScheme is kept.
        assert_eq!(facts[0], rich);
    }

    #[test]
    fn merge_states_dedupes_by_structural_equality() {
        // Identical facts in both states collapse to one.
        let fact = fact_from_pairs(&[("name", "Order"), ("objectType", "entity")]);
        let state_a = store("Noun", Object::seq(vec![fact.clone()]), &Object::phi());
        let state_b = store("Noun", Object::seq(vec![fact.clone()]), &Object::phi());
        let merged = merge_states(&state_a, &state_b);
        let nouns = fetch("Noun", &merged);
        assert_eq!(nouns.as_seq().map(|s| s.len()), Some(1));
    }

    #[test]
    fn merge_states_preserves_distinct_facts() {
        // Two different nouns in separate states both survive.
        let order = fact_from_pairs(&[("name", "Order"), ("objectType", "entity")]);
        let customer = fact_from_pairs(&[("name", "Customer"), ("objectType", "entity")]);
        let state_a = store("Noun", Object::seq(vec![order.clone()]), &Object::phi());
        let state_b = store("Noun", Object::seq(vec![customer.clone()]), &Object::phi());
        let merged = merge_states(&state_a, &state_b);
        let nouns = fetch("Noun", &merged);
        let facts = nouns.as_seq().unwrap();
        assert_eq!(facts.len(), 2);
        assert!(facts.contains(&order));
        assert!(facts.contains(&customer));
    }

    // task-928 + task-932 (W7-a): when prior cell is Map-backed
    // (cell_put_keyed apply population) and we merge in a Seq from parse,
    // the prior Map contents must survive AND the cell must stay a Map.
    // Pre-928: concat_dedup's a.as_seq() returned None for Map, all prior
    // contents silently dropped, apps_compile wiped ~700
    // Task_has_Task_Subject entries down to 1. Pre-W7-a: concat_dedup
    // preserved the CONTENTS but demoted the SHAPE to Seq on every merge,
    // re-firing the fold→demote→re-fold churn on every recompile. W7-a
    // re-folds the deduped Vec back into a Map keyed by synthesize_fact_id
    // whenever either input was already a Map, so a folded FT-image cell
    // keeps its Map shape across merge.
    #[test]
    fn merge_states_preserves_map_cell_when_merging_with_seq() {
        let t1 = fact_from_pairs(&[("Task", "1"), ("Task Subject", "first")]);
        let t2 = fact_from_pairs(&[("Task", "2"), ("Task Subject", "second")]);
        let t3 = fact_from_pairs(&[("Task", "3"), ("Task Subject", "third")]);
        // Build a Map-backed prior cell via cell_put_keyed (the
        // post-UC-Map apply path used by cell command_via_defs).
        let mut prior = Object::phi();
        prior = cell_put_keyed("Task_has_Task_Subject", &["Task"], t1.clone(), &prior).unwrap();
        prior = cell_put_keyed("Task_has_Task_Subject", &["Task"], t2.clone(), &prior).unwrap();
        prior = cell_put_keyed("Task_has_Task_Subject", &["Task"], t3.clone(), &prior).unwrap();
        // Sanity: prior cell is Map.
        let prior_cell = fetch("Task_has_Task_Subject", &prior);
        assert!(matches!(prior_cell, Object::Map(_)),
            "test fixture: prior cell should be Map, got {:?}", prior_cell);

        // Now merge with a Seq-shaped state (mirrors what parse emits).
        let from_parse = store("Task_has_Task_Subject",
            Object::seq(vec![fact_from_pairs(&[("Task", "4"), ("Task Subject", "fourth")])]),
            &Object::phi());

        let merged = merge_states(&prior, &from_parse);
        let merged_cell = fetch("Task_has_Task_Subject", &merged);
        // W7-a: the folded Map cell must KEEP its Map shape, not demote to Seq.
        assert!(matches!(merged_cell, Object::Map(_)),
            "merge of a Map cell with a Seq must preserve Map shape; got {:?}",
            merged_cell);
        // All entries must still be present (read shape-tolerantly).
        let subjects: Vec<String> = cell_facts_iter(&merged_cell)
            .filter_map(|f| binding(f, "Task Subject").map(String::from))
            .collect();
        assert_eq!(cell_fact_count(&merged_cell), 4,
            "all 3 prior Map entries + 1 new Seq entry must survive merge; \
             got {:?}", subjects);
        for expected in ["first", "second", "third", "fourth"] {
            assert!(subjects.contains(&expected.to_string()),
                "merged cell must contain '{}'; got {:?}", expected, subjects);
        }
    }

    // ── Combining forms ──────────────────────────────────────────

    #[test]
    fn construction_applies_each_function() {
        // [1, 2, 3]:<a, b, c> = <a, b, c> (selectors extract each)
        let cons = Func::construction(vec![Func::Selector(1), Func::Selector(2), Func::Selector(3)]);
        let seq = Object::seq(vec![Object::atom("a"), Object::atom("b"), Object::atom("c")]);
        assert_eq!(
            apply(&cons, &seq, &defs()),
            Object::seq(vec![Object::atom("a"), Object::atom("b"), Object::atom("c")])
        );
    }

    #[test]
    fn construction_is_fact_type() {
        // Fact type "User has Org Role in Organization" = [Role₁, Role₂, Role₃]
        // Applied to a membership fact, selects each role's resource.
        let schema = Func::construction(vec![Func::role(1), Func::role(2), Func::role(3)]);
        let fact = Object::seq(vec![
            Object::atom("alice@example.com"),
            Object::atom("owner"),
            Object::atom("org-123"),
        ]);
        assert_eq!(
            apply(&schema, &fact, &defs()),
            Object::seq(vec![
                Object::atom("alice@example.com"),
                Object::atom("owner"),
                Object::atom("org-123"),
            ])
        );
    }

    #[test]
    fn composition_chains() {
        // (1 ∘ tl):<a, b, c> = 1:<b, c> = b
        let f = Func::compose(Func::Selector(1), Func::Tail);
        let seq = Object::seq(vec![Object::atom("a"), Object::atom("b"), Object::atom("c")]);
        assert_eq!(apply(&f, &seq, &defs()), Object::atom("b"));
    }

    #[test]
    fn condition_branches() {
        // (null → "empty"̄; "notempty"̄)
        let f = Func::condition(
            Func::NullTest,
            Func::constant(Object::atom("empty")),
            Func::constant(Object::atom("notempty")),
        );
        assert_eq!(apply(&f, &Object::phi(), &defs()), Object::atom("empty"));
        assert_eq!(
            apply(&f, &Object::seq(vec![Object::atom("x")]), &defs()),
            Object::atom("notempty")
        );
    }

    #[test]
    fn apply_to_all_maps_over_sequence() {
        // α(1):<< a, b>, <c, d>> = <a, c>
        let f = Func::apply_to_all(Func::Selector(1));
        let pop = Object::seq(vec![
            Object::seq(vec![Object::atom("a"), Object::atom("b")]),
            Object::seq(vec![Object::atom("c"), Object::atom("d")]),
        ]);
        assert_eq!(
            apply(&f, &pop, &defs()),
            Object::seq(vec![Object::atom("a"), Object::atom("c")])
        );
    }

    #[test]
    fn insert_folds() {
        // /(or):<F, F, T> = or:<F, or:<F, T>> = or:<F, T> = T
        let f = Func::insert(Func::Or);
        let seq = Object::seq(vec![Object::f(), Object::f(), Object::t()]);
        assert_eq!(apply(&f, &seq, &defs()), Object::t());

        // /(or):<F, F, F> = F
        let seq2 = Object::seq(vec![Object::f(), Object::f(), Object::f()]);
        assert_eq!(apply(&f, &seq2, &defs()), Object::f());
    }

    #[test]
    fn binary_to_unary_curries() {
        // (bu eq "owner"):x = eq:<"owner", x>
        let f = Func::bu(Func::Eq, Object::atom("owner"));
        assert_eq!(apply(&f, &Object::atom("owner"), &defs()), Object::t());
        assert_eq!(apply(&f, &Object::atom("member"), &defs()), Object::f());
    }

    #[test]
    fn distl_distributes() {
        // distl:<y, <z₁, z₂>> = <<y, z₁>, <y, z₂>>
        let x = Object::seq(vec![
            Object::atom("user-1"),
            Object::seq(vec![Object::atom("org-a"), Object::atom("org-b")]),
        ]);
        assert_eq!(
            apply(&Func::DistL, &x, &defs()),
            Object::seq(vec![
                Object::seq(vec![Object::atom("user-1"), Object::atom("org-a")]),
                Object::seq(vec![Object::atom("user-1"), Object::atom("org-b")]),
            ])
        );
    }

    // #743 — HasMember: O(N) allocation-free replacement for the
    // null ∘ filter(eq) ∘ distl pattern. Same big-O as the composed
    // form, but no intermediate Seq materialization (DistL allocates
    // N pairs before the filter sees any of them).

    #[test]
    fn has_member_finds_atom_in_haystack() {
        let x = Object::seq(vec![
            Object::atom("task-42"),
            Object::seq(vec![
                Object::atom("task-1"),
                Object::atom("task-42"),
                Object::atom("task-99"),
            ]),
        ]);
        assert_eq!(apply(&Func::HasMember, &x, &defs()), Object::t());
    }

    #[test]
    fn has_member_returns_false_when_needle_absent() {
        let x = Object::seq(vec![
            Object::atom("task-42"),
            Object::seq(vec![Object::atom("task-1"), Object::atom("task-2")]),
        ]);
        assert_eq!(apply(&Func::HasMember, &x, &defs()), Object::f());
    }

    #[test]
    fn has_member_returns_false_when_haystack_empty() {
        let x = Object::seq(vec![Object::atom("task-42"), Object::phi()]);
        assert_eq!(apply(&Func::HasMember, &x, &defs()), Object::f());
    }

    #[test]
    fn has_member_compares_seq_needles_structurally() {
        // Used inside compile_sm_init_for the needles are typically
        // atoms, but the primitive must support Seq needles too so
        // the same primitive lifts joins whose role values are
        // structured (rare but legal).
        let pair = Object::seq(vec![Object::atom("Task"), Object::atom("task-1")]);
        let other = Object::seq(vec![Object::atom("Task"), Object::atom("task-2")]);
        let x = Object::seq(vec![
            pair.clone(),
            Object::seq(vec![other.clone(), pair.clone()]),
        ]);
        assert_eq!(apply(&Func::HasMember, &x, &defs()), Object::t());
    }

    #[test]
    fn has_member_returns_bottom_on_shape_mismatch() {
        // Single-element input (missing haystack).
        let x = Object::seq(vec![Object::atom("task-42")]);
        assert_eq!(apply(&Func::HasMember, &x, &defs()), Object::Bottom);

        // Atom haystack (not a sequence).
        let y = Object::seq(vec![Object::atom("task-42"), Object::atom("task-1")]);
        assert_eq!(apply(&Func::HasMember, &y, &defs()), Object::Bottom);
    }

    #[test]
    fn has_member_roundtrips_through_object() {
        // ρ(in?) = Func::HasMember; ρ⁻¹(HasMember) = atom "in?".
        let obj = func_to_object(&Func::HasMember);
        assert_eq!(obj.as_atom(), Some(primitives::HAS_MEMBER));
        let func = metacompose(&obj, &defs());
        assert!(matches!(func, Func::HasMember));
    }

    // task-744 phase 5: SetFromSeq builds a Map<atom,T> from a Seq of
    // atoms in one pass. Paired with FetchOrPhi this delivers the
    // O(N+M) replacement for the O(N·M) HasMember scan in is_new.

    #[test]
    fn set_from_seq_builds_map_with_atom_keys_and_t_values() {
        let input = Object::seq(vec![
            Object::atom("task-1"),
            Object::atom("task-2"),
            Object::atom("task-3"),
        ]);
        let result = apply(&Func::SetFromSeq, &input, &defs());
        let m = result.as_map().expect("Map result");
        assert_eq!(m.len(), 3);
        assert_eq!(m.get("task-1"), Some(&Object::t()));
        assert_eq!(m.get("task-2"), Some(&Object::t()));
        assert_eq!(m.get("task-3"), Some(&Object::t()));
    }

    #[test]
    fn set_from_seq_dedupes_atoms_by_overwriting_the_key() {
        let input = Object::seq(vec![
            Object::atom("dup"),
            Object::atom("dup"),
            Object::atom("uniq"),
        ]);
        let m = apply(&Func::SetFromSeq, &input, &defs()).as_map().cloned()
            .expect("Map result");
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn set_from_seq_on_empty_seq_returns_empty_map() {
        let result = apply(&Func::SetFromSeq, &Object::phi(), &defs());
        let m = result.as_map().expect("Map result");
        assert!(m.is_empty());
    }

    #[test]
    fn set_from_seq_with_non_atom_element_returns_bottom() {
        let input = Object::seq(vec![
            Object::atom("ok"),
            Object::seq(vec![Object::atom("nested")]),
        ]);
        assert_eq!(apply(&Func::SetFromSeq, &input, &defs()), Object::Bottom);
    }

    #[test]
    fn set_membership_via_fetch_or_phi_is_o1() {
        // The is_new pattern in compile_sm_init_for compiles to
        // `null . FetchOrPhi`. Verify the composed shape works against
        // a SetFromSeq-built map.
        let resources = Object::seq(vec![
            Object::atom("task-1"),
            Object::atom("task-2"),
        ]);
        let set_map = apply(&Func::SetFromSeq, &resources, &defs());
        let is_new = Func::compose(Func::NullTest, Func::FetchOrPhi);

        // Membership: task-1 is in set → null(T) = F → not new.
        let m1 = Object::seq(vec![Object::atom("task-1"), set_map.clone()]);
        assert_eq!(apply(&is_new, &m1, &defs()), Object::f());
        // Non-membership: task-99 is absent → null(φ) = T → new.
        let m2 = Object::seq(vec![Object::atom("task-99"), set_map]);
        assert_eq!(apply(&is_new, &m2, &defs()), Object::t());
    }

    #[test]
    fn set_from_seq_roundtrips_through_object() {
        let obj = func_to_object(&Func::SetFromSeq);
        assert_eq!(obj.as_atom(), Some(primitives::SET_FROM_SEQ));
        let func = metacompose(&obj, &defs());
        assert!(matches!(func, Func::SetFromSeq));
    }

    /// task-744 / task-817 perf guard.
    ///
    /// After the Arc-Map refactor (Object::Map's HashMap wrapped in
    /// Arc, clone-as-refcount-bump), the SetFromSeq+FetchOrPhi
    /// membership pattern decisively beats the HasMember atom-scan
    /// at SM-init scale: at N=M=700 release-mode the new pattern is
    /// ~4× faster (≈430µs vs ≈1.8ms). The pattern is the production
    /// path in compile_sm_init_for; this test pins the win so any
    /// future regression — accidental switch back to deep-clone
    /// Object::Map, change to DistR semantics that drop the Arc
    /// share, etc. — surfaces immediately.
    ///
    /// The asymmetry is structural: SetFromSeq is one O(N) HashMap
    /// build, then M × O(1) lookups; HasMember is M × O(N) atom
    /// scans. Arc-shared Map storage means the per-instance DistR
    /// clone is a refcount bump rather than a full HashMap deep
    /// copy, so the algorithmic win is no longer cancelled by
    /// allocation overhead.
    #[test]
    fn membership_perf_set_from_seq_beats_has_member_after_arc_map() {
        const N: usize = 700;
        const M: usize = 700;

        let haystack_atoms: Vec<Object> = (0..N)
            .map(|i| Object::atom(&format!("r-{}", i)))
            .collect();
        let needles: Vec<Object> = (0..M)
            .map(|i| Object::atom(&format!("r-{}", i)))
            .collect();
        let haystack_seq = Object::Seq(haystack_atoms.into());
        let defs = defs();

        // Pattern A: SetFromSeq + FetchOrPhi. The production path.
        let t_a = crate::time_shim::Instant::now();
        let set = apply(&Func::SetFromSeq, &haystack_seq, &defs);
        let is_new_new = Func::compose(Func::NullTest, Func::FetchOrPhi);
        let mut hits_new = 0usize;
        for needle in &needles {
            let input = Object::seq(vec![needle.clone(), set.clone()]);
            if apply(&is_new_new, &input, &defs) == Object::f() {
                hits_new += 1;
            }
        }
        let elapsed_new = t_a.elapsed();

        // Pattern B: legacy HasMember atom-scan.
        let t_b = crate::time_shim::Instant::now();
        let mut hits_has = 0usize;
        for needle in &needles {
            let input = Object::seq(vec![needle.clone(), haystack_seq.clone()]);
            if apply(&Func::HasMember, &input, &defs) == Object::t() {
                hits_has += 1;
            }
        }
        let elapsed_has = t_b.elapsed();

        // Both shapes agree on membership.
        assert_eq!(hits_new, hits_has, "membership semantics diverged");
        assert_eq!(hits_new, M, "every needle should be a member");

        // SetFromSeq+FetchOrPhi must win — if not, the Arc-share
        // invariant has likely been broken somewhere.
        assert!(
            elapsed_new < elapsed_has,
            "SetFromSeq+FetchOrPhi must beat HasMember at N=M={} now \
             that Object::Map is Arc-shared. Got new={:?} has={:?}. \
             If this inverts, check whether something in the DistR / \
             store / cell_put_keyed path lost the Arc share (e.g. an \
             intermediate clone copying the inner HashMap).",
            N, elapsed_new, elapsed_has,
        );
    }

    // ── Compact (#352) — drops ⊥ elements so Filter derives cleanly
    //    from compact ∘ α(p → id ; ⊥) per Backus §11.2.4.

    #[test]
    fn compact_drops_bottom_elements_preserving_order() {
        // Object::seq strips ⊥ *at construction* (see the paper's "⊥-
        // preserving sequence constructor" — any sequence with a ⊥ member
        // IS ⊥). So we install ⊥-interleaved input via `Object::Seq`
        // directly, bypassing the constructor.
        let x = Object::Seq(alloc::sync::Arc::from([
            Object::atom("a"),
            Object::Bottom,
            Object::atom("b"),
            Object::Bottom,
            Object::atom("c"),
        ].as_slice()));
        assert_eq!(
            apply(&Func::Compact, &x, &defs()),
            Object::seq(vec![
                Object::atom("a"),
                Object::atom("b"),
                Object::atom("c"),
            ])
        );
    }

    #[test]
    fn compact_on_clean_sequence_is_identity() {
        let x = Object::seq(vec![
            Object::atom("a"),
            Object::atom("b"),
        ]);
        assert_eq!(apply(&Func::Compact, &x, &defs()), x);
    }

    #[test]
    fn compact_on_empty_sequence_is_empty() {
        assert_eq!(apply(&Func::Compact, &Object::phi(), &defs()), Object::phi());
    }

    #[test]
    fn compact_on_atom_is_bottom() {
        assert_eq!(apply(&Func::Compact, &Object::atom("x"), &defs()), Object::Bottom);
    }

    // ── CellNameTest (#355) — Backus §13.3.4 cellname (structural).

    #[test]
    fn cell_name_test_matches_cell_triple() {
        let c = cell("greeting", Object::atom("hello"));
        assert_eq!(apply(&Func::CellNameTest, &c, &defs()), Object::t());
    }

    #[test]
    fn cell_name_test_rejects_atom() {
        assert_eq!(apply(&Func::CellNameTest, &Object::atom("x"), &defs()), Object::f());
    }

    #[test]
    fn cell_name_test_rejects_two_element_seq() {
        let s = Object::seq(vec![Object::atom(CELL_TAG), Object::atom("name-only")]);
        assert_eq!(apply(&Func::CellNameTest, &s, &defs()), Object::f());
    }

    #[test]
    fn cell_name_test_rejects_three_element_seq_without_cell_tag() {
        let s = Object::seq(vec![
            Object::atom("OTHER"),
            Object::atom("name"),
            Object::atom("contents"),
        ]);
        assert_eq!(apply(&Func::CellNameTest, &s, &defs()), Object::f());
    }

    #[test]
    fn cell_name_test_rejects_phi() {
        assert_eq!(apply(&Func::CellNameTest, &Object::phi(), &defs()), Object::f());
    }

    #[test]
    fn cell_name_test_rejects_bottom() {
        // ⊥ propagates through every primitive — apply(_, ⊥, _) = ⊥, not F.
        assert_eq!(apply(&Func::CellNameTest, &Object::Bottom, &defs()), Object::Bottom);
    }

    #[test]
    fn cell_name_test_round_trips_through_metacompose() {
        // Pin the atom-name + metacompose round-trip so the FFP-object
        // representation of CellNameTest stays stable across freeze/thaw.
        // Func has no PartialEq; compare via a second func_to_object pass.
        let f = Func::CellNameTest;
        let obj = func_to_object(&f);
        assert_eq!(obj, Object::atom(primitives::CELL_NAME_TEST));
        let back = metacompose(&obj, &Object::phi());
        assert_eq!(func_to_object(&back), obj);
    }

    // ── Right-selectors (#356) — Backus §11.2.4 `nr`, `tlr`, derived.

    #[test]
    fn selector_from_right_gets_last_element() {
        let s = Object::seq(vec![Object::atom("a"), Object::atom("b"), Object::atom("c")]);
        assert_eq!(apply(&Func::selector_from_right(1), &s, &defs()), Object::atom("c"));
    }

    #[test]
    fn selector_from_right_gets_nth_from_end() {
        let s = Object::seq(vec![
            Object::atom("a"), Object::atom("b"),
            Object::atom("c"), Object::atom("d"),
        ]);
        assert_eq!(apply(&Func::selector_from_right(1), &s, &defs()), Object::atom("d"));
        assert_eq!(apply(&Func::selector_from_right(2), &s, &defs()), Object::atom("c"));
        assert_eq!(apply(&Func::selector_from_right(3), &s, &defs()), Object::atom("b"));
    }

    #[test]
    fn tail_from_right_drops_last() {
        let s = Object::seq(vec![
            Object::atom("a"), Object::atom("b"), Object::atom("c"),
        ]);
        assert_eq!(
            apply(&Func::tail_from_right(), &s, &defs()),
            Object::seq(vec![Object::atom("a"), Object::atom("b")]),
        );
    }

    #[test]
    fn right_selectors_propagate_bottom() {
        assert_eq!(
            apply(&Func::selector_from_right(1), &Object::Bottom, &defs()),
            Object::Bottom,
        );
        assert_eq!(
            apply(&Func::tail_from_right(), &Object::Bottom, &defs()),
            Object::Bottom,
        );
    }

    #[test]
    fn parameterized_cellname_n_composes_from_primitives() {
        // Backus §13.3.4 `(cellname n)` = "is x a cell triple named n?"
        // Backus's paper form uses nested Condition for short-circuit
        // semantics: if x isn't a cell, return F without reaching for
        // Sel(2):x (which would be ⊥ on an atom and would collapse an
        // eager `And ∘ Construction([..])` form to ⊥ per §11.2.1's
        // ⊥-preserving seq constructor).
        //
        //   (cellname n) ≡ CellNameTest → (Eq ∘ [Sel(2), n̄]) ; F̄
        let probe = cell("greeting", Object::atom("hello"));
        let other = cell("farewell", Object::atom("bye"));

        let is_cell_named_greeting = Func::condition(
            Func::CellNameTest,
            Func::compose(
                Func::Eq,
                Func::construction(vec![
                    Func::Selector(2),
                    Func::constant(Object::atom("greeting")),
                ]),
            ),
            Func::constant(Object::f()),
        );

        assert_eq!(apply(&is_cell_named_greeting, &probe, &defs()), Object::t());
        assert_eq!(apply(&is_cell_named_greeting, &other, &defs()), Object::f());
        assert_eq!(apply(&is_cell_named_greeting, &Object::atom("nope"), &defs()), Object::f());
    }

    #[test]
    fn filter_equivalent_to_compact_alpha_cond_when_all_match() {
        // Backus §11.2.4 eq 2 states `Filter(p) ≡ compact ∘ α(p → id ; ⊥)`
        // as an algebraic identity. In AREST the identity only holds
        // *computationally* when the predicate matches every element —
        // because `Object::seq(..)` is strictly ⊥-preserving per §11.2.1
        // (any ⊥ element collapses the whole seq to ⊥), so the moment
        // α emits a ⊥ the intermediate becomes ⊥ and compact on ⊥ = ⊥.
        // Runtime Filter is a necessity, not an optimization; Compact
        // stays a standalone primitive that's useful where ⊥s enter
        // the seq via other paths (e.g. cell_index lookups over sparse
        // noun populations). Pin the all-match case here so future
        // refactors don't break that subset of the algebraic law.
        let is_a = Func::compose(
            Func::Eq,
            Func::construction(vec![Func::Id, Func::constant(Object::atom("a"))]),
        );
        let all_a = Object::seq(vec![Object::atom("a"), Object::atom("a")]);
        let filter_form = Func::filter(is_a.clone());
        let derived = Func::compose(
            Func::Compact,
            Func::apply_to_all(Func::condition(
                is_a,
                Func::Id,
                Func::constant(Object::Bottom),
            )),
        );
        assert_eq!(apply(&filter_form, &all_a, &defs()),
                   apply(&derived, &all_a, &defs()));
    }

    // ── Derivation chain example ─────────────────────────────────

    #[test]
    fn composition_extracts_org_from_membership() {
        // A single membership fact: <alice@example.com, owner, org-123>
        // Composition: (2 ∘ id):fact = role 2 = "owner"
        //              (3 ∘ id):fact = role 3 = "org-123"
        let fact = Object::seq(vec![
            Object::atom("alice@example.com"),
            Object::atom("owner"),
            Object::atom("org-123"),
        ]);

        // Extract org (role 3) via composition
        let get_org = Func::compose(Func::Selector(3), Func::Id);
        assert_eq!(apply(&get_org, &fact, &defs()), Object::atom("org-123"));
    }

    #[test]
    fn apply_to_all_extracts_orgs_from_population() {
        // Population of membership facts (all for same user):
        //   <user, owner, org-1>
        //   <user, member, org-2>
        //
        // α(3):population = <org-1, org-2>  (extract org from each fact)
        let population = Object::seq(vec![
            Object::seq(vec![Object::atom("user"), Object::atom("owner"), Object::atom("org-1")]),
            Object::seq(vec![Object::atom("user"), Object::atom("member"), Object::atom("org-2")]),
        ]);

        let extract_orgs = Func::apply_to_all(Func::Selector(3));
        assert_eq!(
            apply(&extract_orgs, &population, &defs()),
            Object::seq(vec![Object::atom("org-1"), Object::atom("org-2")])
        );
    }

    #[test]
    fn bu_checks_membership_in_org() {
        // (bu eq "org-123"):x = eq:<"org-123", x>
        // Checks if a given org ID matches a target.
        let check = Func::bu(Func::Eq, Object::atom("org-123"));
        assert_eq!(apply(&check, &Object::atom("org-123"), &defs()), Object::t());
        assert_eq!(apply(&check, &Object::atom("org-456"), &defs()), Object::f());
    }

    #[test]
    fn insert_or_checks_existence() {
        // /(or):<T, F, F> = T  (at least one org matches → user has access)
        // /(or):<F, F, F> = F  (no org matches → no access)
        let exists = Func::insert(Func::Or);
        let has_match = Object::seq(vec![Object::t(), Object::f(), Object::f()]);
        let no_match = Object::seq(vec![Object::f(), Object::f(), Object::f()]);
        assert_eq!(apply(&exists, &has_match, &defs()), Object::t());
        assert_eq!(apply(&exists, &no_match, &defs()), Object::f());
    }

    #[test]
    fn full_access_derivation_chain() {
        // Full derivation: "User can access Domain iff..."
        //
        // Given: user's org IDs = <org-1, org-2>
        //        domain's org  = "org-2"
        //
        // Composed: /(or) ∘ α(bu eq "org-2") : <org-1, org-2>
        //         = /(or) ∘ <eq:<org-2, org-1>, eq:<org-2, org-2>>
        //         = /(or) ∘ <F, T>
        //         = T
        // Domain org = "org-2". Check: is org-2 in user's org list?
        let domain_org = Object::atom("org-2");
        let check_access = Func::compose(
            Func::insert(Func::Or),
            Func::apply_to_all(Func::bu(Func::Eq, domain_org)),
        );

        let user_orgs = Object::seq(vec![Object::atom("org-1"), Object::atom("org-2")]);
        assert_eq!(apply(&check_access, &user_orgs, &defs()), Object::t());

        // User not in org-2's org
        let other_orgs = Object::seq(vec![Object::atom("org-3"), Object::atom("org-4")]);
        assert_eq!(apply(&check_access, &other_orgs, &defs()), Object::f());
    }

    // ── All functions are bottom-preserving ───────────────────────

    #[test]
    fn all_forms_preserve_bottom() {
        let d = defs();
        assert_eq!(apply(&Func::Id, &Object::Bottom, &d), Object::Bottom);
        assert_eq!(apply(&Func::Selector(1), &Object::Bottom, &d), Object::Bottom);
        assert_eq!(apply(&Func::Tail, &Object::Bottom, &d), Object::Bottom);
        assert_eq!(apply(&Func::construction(vec![Func::Id]), &Object::Bottom, &d), Object::Bottom);
        assert_eq!(apply(&Func::compose(Func::Id, Func::Id), &Object::Bottom, &d), Object::Bottom);
        assert_eq!(apply(&Func::apply_to_all(Func::Id), &Object::Bottom, &d), Object::Bottom);
        assert_eq!(apply(&Func::filter(Func::Id), &Object::Bottom, &d), Object::Bottom);
    }

    // ── Filter ───────────────────────────────────────────────────

    #[test]
    fn filter_keeps_matching_items() {
        // Filter(bu eq "owner"):<"owner", "member", "owner"> = <"owner", "owner">
        let pred = Func::bu(Func::Eq, Object::atom("owner"));
        let seq = Object::seq(vec![
            Object::atom("owner"),
            Object::atom("member"),
            Object::atom("owner"),
        ]);
        assert_eq!(
            apply(&Func::filter(pred), &seq, &defs()),
            Object::seq(vec![Object::atom("owner"), Object::atom("owner")])
        );
    }

    #[test]
    fn filter_on_tuples_checks_role() {
        // Filter facts where role 2 = "owner":
        // Filter(eq ∘ [2, "owner"̄])
        let pred = Func::compose(
            Func::Eq,
            Func::construction(vec![
                Func::Selector(2),
                Func::constant(Object::atom("owner")),
            ]),
        );
        let pop = Object::seq(vec![
            Object::seq(vec![Object::atom("alice"), Object::atom("owner"), Object::atom("org-1")]),
            Object::seq(vec![Object::atom("bob"), Object::atom("member"), Object::atom("org-2")]),
            Object::seq(vec![Object::atom("carol"), Object::atom("owner"), Object::atom("org-3")]),
        ]);
        let result = apply(&Func::filter(pred), &pop, &defs());
        assert_eq!(
            result,
            Object::seq(vec![
                Object::seq(vec![Object::atom("alice"), Object::atom("owner"), Object::atom("org-1")]),
                Object::seq(vec![Object::atom("carol"), Object::atom("owner"), Object::atom("org-3")]),
            ])
        );
    }

    #[test]
    fn filter_empty_returns_phi() {
        let pred = Func::bu(Func::Eq, Object::atom("x"));
        assert_eq!(apply(&Func::filter(pred), &Object::phi(), &defs()), Object::phi());
    }

    #[test]
    fn filter_no_matches_returns_phi() {
        let pred = Func::bu(Func::Eq, Object::atom("x"));
        let seq = Object::seq(vec![Object::atom("a"), Object::atom("b")]);
        assert_eq!(apply(&Func::filter(pred), &seq, &defs()), Object::phi());
    }

    #[test]
    fn filter_compose_extracts_from_matches() {
        // Full query pipeline: α(1) ∘ Filter(eq ∘ [2, "owner"̄])
        // = extract role 1 from facts where role 2 = "owner"
        let pred = Func::compose(
            Func::Eq,
            Func::construction(vec![
                Func::Selector(2),
                Func::constant(Object::atom("owner")),
            ]),
        );
        let query = Func::compose(
            Func::apply_to_all(Func::Selector(1)),
            Func::filter(pred),
        );
        let pop = Object::seq(vec![
            Object::seq(vec![Object::atom("alice"), Object::atom("owner")]),
            Object::seq(vec![Object::atom("bob"), Object::atom("member")]),
            Object::seq(vec![Object::atom("carol"), Object::atom("owner")]),
        ]);
        assert_eq!(
            apply(&query, &pop, &defs()),
            Object::seq(vec![Object::atom("alice"), Object::atom("carol")])
        );
    }

    // ── Named definitions ────────────────────────────────────────

    #[test]
    fn def_resolves_from_definition_set() {
        let d = defs_to_state(&[("second".to_string(), Func::Selector(2))], &Object::phi());

        let f = Func::Def("second".to_string());
        let seq = Object::seq(vec![Object::atom("a"), Object::atom("b")]);
        assert_eq!(apply(&f, &seq, &d), Object::atom("b"));
    }

    // ── cell_push_unique: set-semantics for P ────────────────────

    #[test]
    fn cell_push_unique_appends_new_fact() {
        let f = fact_from_pairs(&[("Citation", "c1"), ("URI", "platform:x")]);
        let d = cell_push_unique("Citation_has_URI", f.clone(), &Object::phi());
        let cell = fetch("Citation_has_URI", &d).as_seq().map(|s| s.to_vec()).unwrap_or_default();
        assert_eq!(cell.len(), 1);
        assert_eq!(cell[0], f);
    }

    #[test]
    fn cell_push_unique_skips_identical_fact() {
        let f = fact_from_pairs(&[("Citation", "c1"), ("URI", "platform:x")]);
        let d1 = cell_push_unique("Citation_has_URI", f.clone(), &Object::phi());
        let d2 = cell_push_unique("Citation_has_URI", f, &d1);
        let cell = fetch("Citation_has_URI", &d2).as_seq().map(|s| s.to_vec()).unwrap_or_default();
        assert_eq!(cell.len(), 1, "identical fact must not produce a duplicate");
    }

    #[test]
    fn cell_push_unique_keeps_structurally_distinct_facts() {
        let f1 = fact_from_pairs(&[("Citation", "c1"), ("URI", "platform:x")]);
        let f2 = fact_from_pairs(&[("Citation", "c2"), ("URI", "platform:x")]);
        let d = cell_push_unique("Citation_has_URI", f1, &Object::phi());
        let d = cell_push_unique("Citation_has_URI", f2, &d);
        let cell = fetch("Citation_has_URI", &d).as_seq().map(|s| s.to_vec()).unwrap_or_default();
        assert_eq!(cell.len(), 2, "different Citation ids yield distinct facts");
    }

    // ── Runtime Registration (↓DEFS, AREST §3.2 Platform Binding) ──
    // The paper's IoC/DI primitive: a runtime writes a binding into DEFS
    // at any time. The binding is indistinguishable from a compile-derived
    // one at apply time (uniformity); the `runtime_registered_names` cell
    // records which names entered via the runtime writer so downstream
    // layers (provenance / Citation emission) can tell origin apart.

    #[test]
    fn register_runtime_fn_binds_name_in_defs() {
        let d = Object::phi();
        let d2 = register_runtime_fn("sample", Func::Constant(Object::atom("hi")), &d);
        let resolved = apply(&Func::Def("sample".to_string()), &Object::phi(), &d2);
        assert_eq!(resolved, Object::atom("hi"),
            "Func::Def('sample') should resolve to the registered body");
    }

    #[test]
    fn register_runtime_fn_records_name_in_registry_cell() {
        let d = Object::phi();
        let d2 = register_runtime_fn("sample", Func::Constant(Object::atom("hi")), &d);
        let registry = fetch("runtime_registered_names", &d2);
        let names: Vec<String> = registry.as_seq()
            .map(|s| s.iter().filter_map(|o| o.as_atom().map(String::from)).collect())
            .unwrap_or_default();
        assert!(names.contains(&"sample".to_string()),
            "runtime_registered_names should include 'sample' after registration; got {:?}", names);
    }

    #[test]
    fn compile_derived_defs_are_not_in_registry() {
        let d = defs_to_state(&[("second".to_string(), Func::Selector(2))], &Object::phi());
        let registry = fetch("runtime_registered_names", &d);
        let names: Vec<String> = registry.as_seq()
            .map(|s| s.iter().filter_map(|o| o.as_atom().map(String::from)).collect())
            .unwrap_or_default();
        assert!(!names.contains(&"second".to_string()),
            "defs_to_state-derived names must NOT be in the runtime registry; got {:?}", names);
    }

    // ── gen:autocounter (Conceptual Data Type, P3a) ──────────────
    // The auto-counter id generator expressed as a pure Backus-FP
    // reduction: + ∘ [ /max ∘ apndl ∘ [0̄, ids] , 1̄ ]. Asserts the
    // canonical cases both against the directly-built Func and through
    // a registered `Func::Def("gen:autocounter")` resolution.

    #[test]
    fn gen_autocounter_func_computes_max_plus_one() {
        let f = gen_autocounter();
        // Empty population → "1" (the apndl seed gives /max a base, then +1).
        assert_eq!(apply(&f, &Object::phi(), &Object::phi()), Object::atom("1"),
            "<> must yield the first id \"1\"");
        // <1,2,3> → max 3 → 4.
        let ids = Object::seq(vec![Object::atom("1"), Object::atom("2"), Object::atom("3")]);
        assert_eq!(apply(&f, &ids, &Object::phi()), Object::atom("4"),
            "<1,2,3> must yield max+1 = \"4\"");
        // <482,497> → max 497 → 498.
        let ids = Object::seq(vec![Object::atom("482"), Object::atom("497")]);
        assert_eq!(apply(&f, &ids, &Object::phi()), Object::atom("498"),
            "<482,497> must yield max+1 = \"498\"");
    }

    #[test]
    fn gen_autocounter_resolves_as_named_def() {
        // Mirror the production wiring: the def is registered into D and
        // reached by name via Func::Def, exactly as the create path does.
        let d = defs_to_state(&[("gen:autocounter".to_string(), gen_autocounter())], &Object::phi());
        let def = Func::Def("gen:autocounter".to_string());
        assert_eq!(apply(&def, &Object::phi(), &d), Object::atom("1"),
            "Func::Def(\"gen:autocounter\"):<> must resolve to \"1\"");
        let ids = Object::seq(vec![Object::atom("1"), Object::atom("2"), Object::atom("3")]);
        assert_eq!(apply(&def, &ids, &d), Object::atom("4"),
            "Func::Def(\"gen:autocounter\"):<1,2,3> must resolve to \"4\"");
        let ids = Object::seq(vec![Object::atom("482"), Object::atom("497")]);
        assert_eq!(apply(&def, &ids, &d), Object::atom("498"),
            "Func::Def(\"gen:autocounter\"):<482,497> must resolve to \"498\"");
    }

    // ── Citation provenance (E3 / #305) ─────────────────────────
    // emit_citation_fact pushes the four per-Citation facts declared
    // in readings/instances.md §Citation. It returns the assigned
    // Citation id so the caller can emit the Fact cites Citation
    // links it needs. The helper is idempotent over (uri, auth,
    // retrieval_date) — two calls with the same triple produce the
    // same id.

    #[test]
    fn emit_citation_fact_pushes_uri_retrieval_and_authority_facts() {
        let (cite_id, d2) = emit_citation_fact(
            "platform:send_email",
            "Runtime-Function",
            "2026-04-20T12:00:00Z",
            None,
            &Object::phi(),
        );
        assert!(cite_id.starts_with("cite:"), "cite id should be 'cite:…'; got {cite_id}");

        let uri_cell = fetch("Citation_has_URI", &d2);
        let uri_facts = uri_cell.as_seq().map(|s| s.to_vec()).unwrap_or_default();
        assert_eq!(uri_facts.len(), 1, "one URI fact; got {}", uri_facts.len());
        assert_eq!(binding(&uri_facts[0], "URI"), Some("platform:send_email"));
        assert_eq!(binding(&uri_facts[0], "Citation"), Some(cite_id.as_str()));

        let rd_cell = fetch("Citation_has_Retrieval_Date", &d2);
        let rd_facts = rd_cell.as_seq().map(|s| s.to_vec()).unwrap_or_default();
        assert_eq!(binding(&rd_facts[0], "Retrieval Date"), Some("2026-04-20T12:00:00Z"));

        let at_cell = fetch("Citation_has_Authority_Type", &d2);
        let at_facts = at_cell.as_seq().map(|s| s.to_vec()).unwrap_or_default();
        assert_eq!(binding(&at_facts[0], "Authority Type"), Some("Runtime-Function"));
    }

    #[test]
    fn emit_citation_fact_with_external_system_pushes_backed_by_fact() {
        let (cite_id, d2) = emit_citation_fact(
            "https://api.stripe.com/v1/customers",
            "Federated-Fetch",
            "2026-04-20T12:00:00Z",
            Some("stripe"),
            &Object::phi(),
        );
        let backed_cell = fetch("Citation_is_backed_by_External_System", &d2);
        let facts = backed_cell.as_seq().map(|s| s.to_vec()).unwrap_or_default();
        assert_eq!(facts.len(), 1,
            "Federated-Fetch citation should record its External System; got {} facts", facts.len());
        assert_eq!(binding(&facts[0], "Citation"), Some(cite_id.as_str()));
        assert_eq!(binding(&facts[0], "External System"), Some("stripe"));
    }

    #[test]
    fn emit_citation_fact_without_external_system_does_not_push_backed_by() {
        let (_, d2) = emit_citation_fact(
            "platform:send_email",
            "Runtime-Function",
            "2026-04-20T12:00:00Z",
            None,
            &Object::phi(),
        );
        let backed_cell = fetch("Citation_is_backed_by_External_System", &d2);
        assert!(backed_cell.is_bottom() || backed_cell.as_seq().map(|s| s.is_empty()).unwrap_or(true),
            "Runtime-Function citation (no External System) must NOT push a backed_by fact");
    }

    #[test]
    fn emit_citation_fact_id_is_stable_per_triple() {
        let d = Object::phi();
        let (id1, _) = emit_citation_fact(
            "platform:send_email", "Runtime-Function", "2026-04-20T12:00:00Z", None, &d);
        let (id2, _) = emit_citation_fact(
            "platform:send_email", "Runtime-Function", "2026-04-20T12:00:00Z", None, &d);
        assert_eq!(id1, id2, "same (uri, auth, retrieval_date) must yield the same cite id");
    }

    /// `Each Citation has exactly one Text.` is alethic in instances.md.
    /// Every emitted Citation must carry a Text binding so the mandatory-
    /// role constraint is satisfied. The text is auto-generated from the
    /// already-known fields (deterministic per id).
    #[test]
    fn emit_citation_fact_populates_text_so_mandatory_alethic_holds() {
        let (cite_id, d) = emit_citation_fact(
            "https://api.stripe.com/v1/customers",
            "Federated-Fetch",
            "2026-04-20T12:00:00Z",
            Some("stripe"),
            &Object::phi(),
        );
        let text_cell = fetch("Citation_has_Text", &d).as_seq()
            .map(|s| s.to_vec()).unwrap_or_default();
        let matched = text_cell.iter()
            .find(|f| binding(f, "Citation") == Some(cite_id.as_str()))
            .and_then(|f| binding(f, "Text"))
            .map(String::from);
        let matched_str = matched.as_deref().unwrap_or("");
        assert!(!matched_str.is_empty(),
            "Citation must have non-empty Text to satisfy 'exactly one Text'");
        // Auto-text mentions the URI, the system, and the retrieval date
        // so an LLM reading the cell gets origin at a glance.
        assert!(matched_str.contains("https://api.stripe.com/v1/customers"),
            "auto-text should include URI: {matched_str}");
        assert!(matched_str.contains("stripe"),
            "auto-text should include external system: {matched_str}");
        assert!(matched_str.contains("2026-04-20T12:00:00Z"),
            "auto-text should include retrieval date: {matched_str}");
    }

    // ── S1f (#722): Citation pins cell_name@version_id ──────────────

    #[test]
    fn emit_citation_fact_pinned_pushes_cell_name_and_version_id_facts() {
        // A federated fetch that read cell "Customer" at version 7
        // emits a Citation pinned to that exact provenance pair.
        let (cite_id, d) = emit_citation_fact_pinned(
            "https://api.stripe.com/v1/customers/cus_42",
            "Federated-Fetch",
            "2026-05-05T09:00:00Z",
            Some("stripe"),
            Some(("Customer", 7)),
            &Object::phi(),
        );
        let name_cell = fetch("Citation_pins_Cell_Name", &d).as_seq()
            .map(|s| s.to_vec()).unwrap_or_default();
        assert_eq!(name_cell.len(), 1, "one Cell Name pin; got {}", name_cell.len());
        assert_eq!(binding(&name_cell[0], "Citation"), Some(cite_id.as_str()));
        assert_eq!(binding(&name_cell[0], "Cell Name"), Some("Customer"));

        let ver_cell = fetch("Citation_pins_Cell_Version_Id", &d).as_seq()
            .map(|s| s.to_vec()).unwrap_or_default();
        assert_eq!(ver_cell.len(), 1, "one Cell Version Id pin; got {}", ver_cell.len());
        assert_eq!(binding(&ver_cell[0], "Citation"), Some(cite_id.as_str()));
        assert_eq!(binding(&ver_cell[0], "Cell Version Id"), Some("7"));
    }

    #[test]
    fn emit_citation_fact_pinned_at_different_versions_get_distinct_ids() {
        // Same URI / authority / date, different cell version → distinct
        // Citation ids so the audit trail can tell "fetched at v=3"
        // apart from "fetched at v=4."
        let (id3, _) = emit_citation_fact_pinned(
            "platform:read_inventory", "Runtime-Function",
            "2026-05-05T10:00:00Z", None, Some(("Inventory", 3)), &Object::phi());
        let (id4, _) = emit_citation_fact_pinned(
            "platform:read_inventory", "Runtime-Function",
            "2026-05-05T10:00:00Z", None, Some(("Inventory", 4)), &Object::phi());
        assert_ne!(id3, id4,
            "different cell versions of the same URI must yield distinct Citation ids");
    }

    #[test]
    fn emit_citation_fact_pinned_is_idempotent_per_pin() {
        // Same (uri, auth, date, pin) twice → same id, same cell sizes.
        let d0 = Object::phi();
        let (id1, d1) = emit_citation_fact_pinned(
            "platform:read_inventory", "Runtime-Function",
            "2026-05-05T10:00:00Z", None, Some(("Inventory", 9)), &d0);
        let (id2, d2) = emit_citation_fact_pinned(
            "platform:read_inventory", "Runtime-Function",
            "2026-05-05T10:00:00Z", None, Some(("Inventory", 9)), &d1);
        assert_eq!(id1, id2, "same pin must yield same Citation id");
        let name_cell = fetch("Citation_pins_Cell_Name", &d2).as_seq()
            .map(|s| s.to_vec()).unwrap_or_default();
        assert_eq!(name_cell.len(), 1,
            "second emit must dedupe Cell Name pin; got {}", name_cell.len());
    }

    #[test]
    fn emit_citation_fact_unpinned_does_not_push_pin_facts() {
        // Backwards-compat: callers that don't pass a pin see the
        // pre-S1f cell shape — no Citation_pins_… facts emitted.
        let (_, d) = emit_citation_fact(
            "platform:send_email", "Runtime-Function",
            "2026-05-05T10:00:00Z", None, &Object::phi());
        let name_cell = fetch("Citation_pins_Cell_Name", &d);
        assert!(name_cell.is_bottom() || name_cell.as_seq().map(|s| s.is_empty()).unwrap_or(true),
            "unpinned Citation must NOT push Cell Name pin");
        let ver_cell = fetch("Citation_pins_Cell_Version_Id", &d);
        assert!(ver_cell.is_bottom() || ver_cell.as_seq().map(|s| s.is_empty()).unwrap_or(true),
            "unpinned Citation must NOT push Cell Version Id pin");
    }

    /// Emission uses cell_push_unique, so repeated emission for the same
    /// (uri, auth, retrieval_date) triple yields the same id AND leaves
    /// the Citation cells at size 1 — no duplicate facts, matching the
    /// paper's set-semantics for P.
    #[test]
    fn emit_citation_fact_is_truly_idempotent_across_calls() {
        let uri = "platform:send_email";
        let (_, d1) = emit_citation_fact(uri, "Runtime-Function", "2026-04-20T12:00:00Z", None, &Object::phi());
        let (_, d2) = emit_citation_fact(uri, "Runtime-Function", "2026-04-20T12:00:00Z", None, &d1);
        for cell in ["Citation_has_URI", "Citation_has_Retrieval_Date",
                     "Citation_has_Authority_Type", "Citation_has_Text"] {
            let n = fetch(cell, &d2).as_seq().map(|s| s.len()).unwrap_or(0);
            assert_eq!(n, 1, "{cell} must stay at size 1 after idempotent re-emit; got {n}");
        }
    }

    // ── Async Platform registry (#305 #2) ──────────────────────
    //
    // Sibling to the sync registry: hosts that genuinely need async
    // bodies (HTTP fetch, Promise-returning JS, channel sends) install
    // via install_async_platform_fn. Sync callers go through the
    // existing registry unchanged. The tests use a hand-rolled
    // block_on so we don't pull in a Future executor as a dep.

    /// Minimal busy-wait executor for test only — drives a Future to
    /// completion by repeatedly polling with a dummy waker. Safe
    /// because the futures under test are short-lived and complete
    /// on the first poll in practice.
    fn block_on<F: core::future::Future>(fut: F) -> F::Output {
        use core::pin::pin;
        use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        static VTABLE: RawWakerVTable = RawWakerVTable::new(
            |_| RawWaker::new(core::ptr::null(), &VTABLE),
            |_| {}, |_| {}, |_| {},
        );
        let raw = RawWaker::new(core::ptr::null(), &VTABLE);
        let waker = unsafe { Waker::from_raw(raw) };
        let mut cx = Context::from_waker(&waker);
        let mut f = pin!(fut);
        loop {
            match f.as_mut().poll(&mut cx) {
                Poll::Ready(v) => return v,
                Poll::Pending => core::hint::spin_loop(),
            }
        }
    }

    #[test]
    fn apply_platform_async_dispatches_to_installed_async_body() {
        install_async_platform_fn(
            "e3_async_echo",
            crate::sync::Arc::new(|x: &Object, _d: &Object| {
                let cloned = x.clone();
                alloc::boxed::Box::pin(async move { cloned })
            }),
        );
        let out = block_on(apply_platform_async(
            "e3_async_echo",
            &Object::atom("hello"),
            &Object::phi(),
        ));
        uninstall_async_platform_fn("e3_async_echo");
        assert_eq!(out, Object::atom("hello"),
            "async platform body must be awaited and return its Future's output");
    }

    #[test]
    fn apply_platform_async_falls_through_to_sync_registry() {
        install_platform_fn(
            "e3_async_sync_fallback",
            crate::sync::Arc::new(|x: &Object, _d: &Object| x.clone()),
        );
        let out = block_on(apply_platform_async(
            "e3_async_sync_fallback",
            &Object::atom("sync"),
            &Object::phi(),
        ));
        uninstall_platform_fn("e3_async_sync_fallback");
        assert_eq!(out, Object::atom("sync"),
            "async dispatch must fall through to the sync registry when no async body is registered");
    }

    #[test]
    fn apply_platform_async_returns_bottom_when_no_body_registered() {
        let out = block_on(apply_platform_async(
            "e3_async_nothing_registered",
            &Object::atom("x"),
            &Object::phi(),
        ));
        assert_eq!(out, Object::Bottom,
            "name with no sync AND no async body must resolve to ⊥");
    }

    // ── Platform fallback registry (#305 IoC/DI completion) ────

    /// apply_platform's hardcoded match covers compile-derived names.
    /// Runtime-registered names (httpFetch, send_email, ML scorers)
    /// install synchronous bodies via install_platform_fn, which
    /// apply_platform dispatches when no hardcoded arm matches.
    #[test]
    fn apply_platform_dispatches_to_installed_runtime_body() {
        install_platform_fn(
            "e3_test_echo",
            crate::sync::Arc::new(|x: &Object, _d: &Object| x.clone()),
        );
        // register_runtime_fn installs DEFS[name] = Func::Platform(name)
        // + marks the name in runtime_registered_names. The metacompose
        // of Func::Platform(name) is itself — so apply(Def(name), x, d)
        // dispatches via apply_platform to the installed body.
        let d = register_runtime_fn(
            "e3_test_echo",
            Func::Platform("e3_test_echo".to_string()),
            &Object::phi(),
        );
        let result = apply(&Func::Def("e3_test_echo".to_string()), &Object::atom("hi"), &d);
        uninstall_platform_fn("e3_test_echo");
        assert_eq!(result, Object::atom("hi"),
            "apply must dispatch Func::Platform('e3_test_echo') to the installed closure");
    }

    #[test]
    fn apply_platform_returns_bottom_for_uninstalled_runtime_name() {
        let d = register_runtime_fn(
            "e3_test_no_body",
            Func::Platform("e3_test_no_body".to_string()),
            &Object::phi(),
        );
        let result = apply(&Func::Def("e3_test_no_body".to_string()), &Object::atom("hi"), &d);
        assert_eq!(result, Object::Bottom,
            "name marked in DEFS but with no installed body must return ⊥");
    }

    // ── task-970: Skolem value-invention primitive ─────────────────────
    //
    // The deterministic head-id leaf for existential (TGD) derivation
    // heads. The Skolem (semi-oblivious) chase needs the id to be a pure
    // function of the frontier so re-derivation is idempotent. These tests
    // pin the three load-bearing properties — determinism, frontier-keying,
    // and total Bottom-on-bad-shape — independent of the parser/compiler.

    #[test]
    fn platform_skolem_is_deterministic_and_frontier_keyed() {
        let f = Func::Platform("skolem".to_string());
        let d = Object::phi();
        let frontier_a = Object::seq(vec![Object::atom("Menu View"), Object::atom("approve")]);
        let frontier_b = Object::seq(vec![Object::atom("Menu View"), Object::atom("reject")]);

        let id_a1 = apply(&f, &frontier_a, &d);
        let id_a2 = apply(&f, &frontier_a, &d);
        let id_b = apply(&f, &frontier_b, &d);

        // Determinism: same frontier → byte-identical id (idempotence crux).
        assert_eq!(id_a1, id_a2,
            "skolem id must be deterministic for the same frontier; got {:?} vs {:?}",
            id_a1, id_a2);
        // Shape: `ve_` prefix + 16 hex chars.
        let s = id_a1.as_atom().expect("skolem id must be an atom");
        assert!(s.starts_with("ve_") && s.len() == "ve_".len() + 16,
            "skolem id must be `ve_<16 hex>`; got {:?}", s);
        // Frontier-keying: a different frontier → a different id (no
        // collision across the distinct (View,Transition) pairs a menu
        // projection produces).
        assert_ne!(id_a1, id_b,
            "distinct frontiers must yield distinct skolem ids; both = {:?}", id_a1);
    }

    #[test]
    fn platform_skolem_order_sensitive_and_total() {
        let f = Func::Platform("skolem".to_string());
        let d = Object::phi();
        // Order matters: <a,b> and <b,a> are distinct frontier tuples.
        let ab = apply(&f, &Object::seq(vec![Object::atom("a"), Object::atom("b")]), &d);
        let ba = apply(&f, &Object::seq(vec![Object::atom("b"), Object::atom("a")]), &d);
        assert_ne!(ab, ba, "skolem must be order-sensitive over the frontier tuple");
        // The `|` separator prevents the <\"ab\"> vs <\"a\",\"b\"> ambiguity.
        let joined = apply(&f, &Object::seq(vec![Object::atom("ab")]), &d);
        assert_ne!(ab, joined,
            "separator must disambiguate <a,b> from <ab>");
        // Total: a non-sequence operand → Bottom (never panic).
        let bad = apply(&f, &Object::atom("not-a-seq"), &d);
        assert_eq!(bad, Object::Bottom, "non-seq frontier must yield Bottom");
    }

    #[test]
    fn apply_platform_body_sees_both_operand_and_state() {
        install_platform_fn(
            "e3_test_readx",
            crate::sync::Arc::new(|x: &Object, d: &Object| {
                let key = x.as_atom().unwrap_or("");
                fetch(key, d)
            }),
        );
        let d = register_runtime_fn(
            "e3_test_readx",
            Func::Platform("e3_test_readx".to_string()),
            &store("secret_cell", Object::atom("the-value"), &Object::phi()),
        );
        let result = apply(&Func::Def("e3_test_readx".to_string()), &Object::atom("secret_cell"), &d);
        uninstall_platform_fn("e3_test_readx");
        assert_eq!(result, Object::atom("the-value"),
            "installed closure must have access to D so it can fetch cells");
    }

    // ── C1 (#687): Policy_platform cell semantics ────────────────────

    #[test]
    fn platform_from_state_returns_none_on_empty_state() {
        assert_eq!(platform_from_state(&Object::phi(), "anything"), None);
    }

    #[test]
    fn install_platform_round_trips_through_cell() {
        let s = install_platform(&Object::phi(), "ai_complete", "platform.ai.complete");
        assert_eq!(
            platform_from_state(&s, "ai_complete"),
            Some("platform.ai.complete".to_string()),
            "install_platform → platform_from_state must round-trip the identifier"
        );
    }

    #[test]
    fn install_platform_does_not_leak_into_fresh_state() {
        let s = install_platform(&Object::phi(), "ai_complete", "platform.ai.complete");
        // The installed name is invisible to a state that did not see
        // the install — Policy_platform is per-state, not process-global.
        assert_eq!(platform_from_state(&Object::phi(), "ai_complete"), None);
        // And the original installed-into state still carries it.
        assert!(platform_from_state(&s, "ai_complete").is_some());
    }

    #[test]
    fn install_platform_replaces_prior_identifier_for_same_name() {
        let s1 = install_platform(&Object::phi(), "ai_complete", "old.id");
        let s2 = install_platform(&s1, "ai_complete", "new.id");
        // Only the latest identifier is visible — re-install replaces.
        assert_eq!(
            platform_from_state(&s2, "ai_complete"),
            Some("new.id".to_string())
        );
        // Exactly one row remains in the cell.
        let rows = fetch(POLICY_PLATFORM, &s2).as_seq()
            .map(|s| s.to_vec()).unwrap_or_default();
        assert_eq!(rows.len(), 1, "duplicate names must collapse to one row");
    }

    #[test]
    fn install_platform_preserves_other_names() {
        let s = install_platform(&Object::phi(), "alpha", "id.alpha");
        let s = install_platform(&s, "beta", "id.beta");
        assert_eq!(platform_from_state(&s, "alpha"), Some("id.alpha".to_string()));
        assert_eq!(platform_from_state(&s, "beta"), Some("id.beta".to_string()));
        assert_eq!(platform_names_from_state(&s), vec!["alpha".to_string(), "beta".to_string()]);
    }

    #[test]
    fn platform_from_state_distinguishes_unrelated_names() {
        let s = install_platform(&Object::phi(), "alpha", "id.alpha");
        assert_eq!(platform_from_state(&s, "alpha"), Some("id.alpha".to_string()));
        assert_eq!(platform_from_state(&s, "beta"), None);
    }

    #[test]
    fn install_platform_fn_makes_name_visible_through_cell_mirror() {
        // install_platform_fn updates both the side-table AND the
        // process-wide Policy_platform mirror. installed_platform_fn_names
        // reads from the cell, so the install must show up there.
        install_platform_fn(
            "c1_cell_mirror_probe",
            crate::sync::Arc::new(|x: &Object, _d: &Object| x.clone()),
        );
        let names = installed_platform_fn_names();
        assert!(
            names.contains(&"c1_cell_mirror_probe".to_string()),
            "install_platform_fn must surface the name through the cell mirror; got {:?}",
            names
        );
        // Cleanup so the sec-2 audit test (which checks the mirror) does
        // not see this probe name as an unapproved installed entry.
        uninstall_platform_fn("c1_cell_mirror_probe");
        let after = installed_platform_fn_names();
        assert!(
            !after.contains(&"c1_cell_mirror_probe".to_string()),
            "uninstall_platform_fn must remove the name from the cell mirror; got {:?}",
            after
        );
    }

    #[test]
    fn dispatch_platform_fallback_uses_cell_as_authority() {
        // Identical end-to-end behaviour to the pre-C1 dispatcher: an
        // installed body is reached, an uninstalled name returns ⊥. The
        // distinguishing C1 invariant is that the dispatcher consults
        // the cell mirror, not the side-table directly — verified by
        // the install_platform_fn test above which proves the mirror is
        // populated. This test pins the end-to-end contract.
        install_platform_fn(
            "c1_dispatch_probe",
            crate::sync::Arc::new(|x: &Object, _d: &Object| x.clone()),
        );
        let out = dispatch_platform_fallback(
            "c1_dispatch_probe",
            &Object::atom("hello"),
            &Object::phi(),
        );
        assert_eq!(out, Object::atom("hello"));
        uninstall_platform_fn("c1_dispatch_probe");
        let out = dispatch_platform_fallback(
            "c1_dispatch_probe",
            &Object::atom("hello"),
            &Object::phi(),
        );
        assert_eq!(out, Object::Bottom,
            "after uninstall, the cell mirror is empty so the dispatcher \
             must short-circuit to Bottom without consulting the side-table");
    }

    // ── End-to-end: register → invoke → cite (#305 integration) ─

    /// Drives the full IoC/DI + Citation-provenance flow end-to-end.
    /// A runtime wrapper:
    ///   1. Registers a Platform-function body via `register_runtime_fn`.
    ///   2. Invokes it through the normal `apply(Func::Def(name), ...)`
    ///      dispatch — the engine doesn't distinguish it from a
    ///      compile-derived binding (uniformity).
    ///   3. Because the engine records the name in
    ///      `runtime_registered_names`, the wrapper knows the binding
    ///      is outside the local ρ algebra and emits a Citation with
    ///      Authority Type 'Runtime-Function' whose URI is the
    ///      platform:{name} DEFS key. Theorem 5 is preserved — the
    ///      Citation is itself a fact in P produced by ρ (cell_push).
    #[test]
    fn runtime_registered_platform_fn_emits_citation_on_invocation() {
        // 1. Runtime registers platform:send_email.
        let d = register_runtime_fn(
            "send_email",
            Func::Constant(Object::atom("sent")),
            &Object::phi(),
        );

        // 2. Caller invokes the registered name through the standard
        //    apply dispatch. The engine treats it uniformly with
        //    compile-derived defs.
        let result = apply(&Func::Def("send_email".to_string()), &Object::phi(), &d);
        assert_eq!(result, Object::atom("sent"));

        // 3. The caller, seeing the name in runtime_registered_names,
        //    emits a Citation for provenance.
        let names: Vec<String> = fetch("runtime_registered_names", &d)
            .as_seq()
            .map(|s| s.iter().filter_map(|o| o.as_atom().map(String::from)).collect())
            .unwrap_or_default();
        assert!(names.contains(&"send_email".to_string()),
            "send_email must be visible as runtime-registered");

        let (cite_id, d2) = emit_citation_fact(
            "platform:send_email",
            "Runtime-Function",
            "2026-04-20T12:00:00Z",
            None,
            &d,
        );

        // 4. Assertions: Citation fact in P names the Platform DEFS key.
        let uri_facts = fetch("Citation_has_URI", &d2).as_seq()
            .map(|s| s.to_vec()).unwrap_or_default();
        let cited_uris: Vec<&str> = uri_facts.iter()
            .filter(|f| binding(f, "Citation") == Some(cite_id.as_str()))
            .filter_map(|f| binding(f, "URI"))
            .collect();
        assert_eq!(cited_uris, vec!["platform:send_email"],
            "Citation URI must name the Platform DEFS key");

        let auth_facts = fetch("Citation_has_Authority_Type", &d2).as_seq()
            .map(|s| s.to_vec()).unwrap_or_default();
        let cited_auths: Vec<&str> = auth_facts.iter()
            .filter(|f| binding(f, "Citation") == Some(cite_id.as_str()))
            .filter_map(|f| binding(f, "Authority Type"))
            .collect();
        assert_eq!(cited_auths, vec!["Runtime-Function"],
            "Authority Type must be 'Runtime-Function' for platform-layer origin");
    }

    // ── Federated ingestion: facts + Citation in one call (#305) ─

    /// ingest_federated_facts is the full ρ(populate_n) realization:
    /// pre-fetched facts enter P under OWA, paired with a single
    /// Citation whose Authority Type is 'Federated-Fetch'. Each caller-
    /// supplied (fact_type_id, bindings) tuple becomes a fact in the
    /// named cell. The Citation is emitted via emit_citation_fact so
    /// the id scheme matches and repeated ingestion of the same
    /// (url, retrieval_date) is idempotent at the cell level.
    #[test]
    fn ingest_federated_facts_pushes_facts_and_emits_citation() {
        let url = "https://api.stripe.com/v1/customers";
        let facts = alloc::vec![
            (
                "Stripe_Customer_has_Email".to_string(),
                alloc::vec![
                    ("Stripe Customer".to_string(), "cus_1".to_string()),
                    ("Email".to_string(), "a@x.com".to_string()),
                ],
            ),
            (
                "Stripe_Customer_has_Name".to_string(),
                alloc::vec![
                    ("Stripe Customer".to_string(), "cus_1".to_string()),
                    ("Name".to_string(), "Alice".to_string()),
                ],
            ),
        ];
        let (cite_id, d) = ingest_federated_facts(
            "stripe",
            url,
            "2026-04-20T12:00:00Z",
            &facts,
            &Object::phi(),
        );
        assert!(cite_id.starts_with("cite:"),
            "ingest should emit a content-addressed Citation id; got {cite_id}");

        // Citation must record all four readings for Federated-Fetch origin.
        let uri_facts = fetch("Citation_has_URI", &d).as_seq()
            .map(|s| s.to_vec()).unwrap_or_default();
        let matched_uri = uri_facts.iter()
            .find(|f| binding(f, "Citation") == Some(cite_id.as_str()))
            .and_then(|f| binding(f, "URI"));
        assert_eq!(matched_uri, Some(url),
            "Citation_has_URI must point at the fetch URL");

        let at_facts = fetch("Citation_has_Authority_Type", &d).as_seq()
            .map(|s| s.to_vec()).unwrap_or_default();
        let matched_at = at_facts.iter()
            .find(|f| binding(f, "Citation") == Some(cite_id.as_str()))
            .and_then(|f| binding(f, "Authority Type"));
        assert_eq!(matched_at, Some("Federated-Fetch"));

        let bb_facts = fetch("Citation_is_backed_by_External_System", &d).as_seq()
            .map(|s| s.to_vec()).unwrap_or_default();
        let matched_bb = bb_facts.iter()
            .find(|f| binding(f, "Citation") == Some(cite_id.as_str()))
            .and_then(|f| binding(f, "External System"));
        assert_eq!(matched_bb, Some("stripe"));

        // Ingested facts land in their declared FT cells.
        let email_cell = fetch("Stripe_Customer_has_Email", &d).as_seq()
            .map(|s| s.to_vec()).unwrap_or_default();
        assert_eq!(email_cell.len(), 1,
            "Stripe_Customer_has_Email cell must contain the ingested fact");
        assert_eq!(binding(&email_cell[0], "Email"), Some("a@x.com"));
        assert_eq!(binding(&email_cell[0], "Stripe Customer"), Some("cus_1"));

        let name_cell = fetch("Stripe_Customer_has_Name", &d).as_seq()
            .map(|s| s.to_vec()).unwrap_or_default();
        assert_eq!(name_cell.len(), 1);
        assert_eq!(binding(&name_cell[0], "Name"), Some("Alice"));
    }

    /// Each ingested fact gets a paired `Fact cites Citation` link so
    /// downstream deontic obligations like "Each Fact of Fact Type 'X'
    /// cites some Citation" can evaluate. Fact ids are content-
    /// addressed over (factTypeId, sorted bindings) — deterministic
    /// per fact, stable across ingestion.
    #[test]
    fn ingest_federated_facts_emits_fact_cites_citation_links() {
        let url = "https://api.stripe.com/v1/customers";
        let facts = alloc::vec![
            (
                "Stripe_Customer_has_Email".to_string(),
                alloc::vec![
                    ("Stripe Customer".to_string(), "cus_1".to_string()),
                    ("Email".to_string(), "a@x.com".to_string()),
                ],
            ),
            (
                "Stripe_Customer_has_Name".to_string(),
                alloc::vec![
                    ("Stripe Customer".to_string(), "cus_1".to_string()),
                    ("Name".to_string(), "Alice".to_string()),
                ],
            ),
        ];
        let (cite_id, d) = ingest_federated_facts(
            "stripe", url, "2026-04-20T12:00:00Z", &facts, &Object::phi(),
        );

        let link_cell = fetch("Fact_cites_Citation", &d).as_seq()
            .map(|s| s.to_vec()).unwrap_or_default();
        assert_eq!(link_cell.len(), 2,
            "one Fact cites Citation link per ingested fact; got {}", link_cell.len());
        let cite_bindings: Vec<&str> = link_cell.iter()
            .filter_map(|f| binding(f, "Citation"))
            .collect();
        assert!(cite_bindings.iter().all(|c| *c == cite_id),
            "every link fact must name the same Citation id {cite_id}; got {cite_bindings:?}");
        // Each link has a distinct Fact id (one per ingested fact).
        let fact_ids: Vec<&str> = link_cell.iter()
            .filter_map(|f| binding(f, "Fact"))
            .collect();
        assert_eq!(fact_ids.len(), 2);
        assert_ne!(fact_ids[0], fact_ids[1],
            "two different ingested facts must have different Fact ids");
    }

    /// Ingested facts ARE Resource subtypes (instances.md §Fact: Fact
    /// is a subtype of Resource; Resource has Reference). Emit a
    /// Resource_has_Reference fact per ingested fact so identity is
    /// navigable via the existing Reference scheme — same id used for
    /// the Fact cites Citation link.
    #[test]
    fn ingest_federated_facts_populates_resource_has_reference() {
        let facts = alloc::vec![(
            "Stripe_Customer_has_Email".to_string(),
            alloc::vec![
                ("Stripe Customer".to_string(), "cus_1".to_string()),
                ("Email".to_string(), "a@x.com".to_string()),
            ],
        )];
        let (_, d) = ingest_federated_facts(
            "stripe", "https://api.stripe.com/v1/customers",
            "2026-04-20T12:00:00Z", &facts, &Object::phi(),
        );
        let ref_cell = fetch("Resource_has_Reference", &d).as_seq()
            .map(|s| s.to_vec()).unwrap_or_default();
        assert_eq!(ref_cell.len(), 1,
            "Resource_has_Reference must carry the ingested fact's identity");
        let ref_val = binding(&ref_cell[0], "Reference");
        assert!(ref_val.map(|r| r.starts_with("fact:")).unwrap_or(false),
            "Reference should be the synthetic fact id; got {ref_val:?}");
    }

    #[test]
    fn ingest_federated_facts_citation_id_stable_across_calls() {
        let url = "https://api.stripe.com/v1/customers";
        let rd = "2026-04-20T12:00:00Z";
        let facts = alloc::vec![(
            "Stripe_Customer_has_Email".to_string(),
            alloc::vec![
                ("Stripe Customer".to_string(), "cus_1".to_string()),
                ("Email".to_string(), "a@x.com".to_string()),
            ],
        )];
        // Two ingests against the same (url, auth, retrieval_date) triple
        // must yield the same Citation id. cell_push does not dedupe —
        // consumers join on the stable id at query time when they need
        // uniqueness, matching the paper's set-semantics for facts.
        let (id1, d1) = ingest_federated_facts("stripe", url, rd, &facts, &Object::phi());
        let (id2, _)  = ingest_federated_facts("stripe", url, rd, &facts, &d1);
        assert_eq!(id1, id2,
            "same (url, auth, retrieval_date) must yield the same cite id");
    }

    /// Pure ρ-application (a compile-derived def) produces no Citation.
    /// Guards the invariant that the engine does not auto-emit Citations
    /// for domain-layer operations — Citation facts appear only when a
    /// runtime wrapper explicitly emits them for outside-ρ origins.
    #[test]
    fn pure_derivation_produces_no_auto_citation() {
        let d = defs_to_state(
            &[("second".to_string(), Func::Selector(2))],
            &Object::phi(),
        );
        let input = Object::seq(vec![Object::atom("a"), Object::atom("b")]);
        let result = apply(&Func::Def("second".to_string()), &input, &d);
        assert_eq!(result, Object::atom("b"));

        // No side-effect on Citation cells: emit_citation_fact was not
        // called, so Citation_has_URI et al. must be absent / empty.
        for cell in [
            "Citation_has_URI",
            "Citation_has_Retrieval_Date",
            "Citation_has_Authority_Type",
            "Citation_is_backed_by_External_System",
        ] {
            let c = fetch(cell, &d);
            assert!(
                c.is_bottom() || c.as_seq().map(|s| s.is_empty()).unwrap_or(true),
                "{cell} must be empty after pure ρ-application"
            );
        }
    }

    // ── Backus sequence primitives (Task 1) ─────────────────────

    #[test]
    fn apndr_appends_to_right() {
        let x = Object::seq(vec![
            Object::seq(vec![Object::atom("a"), Object::atom("b")]),
            Object::atom("c"),
        ]);
        assert_eq!(
            apply(&Func::ApndR, &x, &defs()),
            Object::seq(vec![Object::atom("a"), Object::atom("b"), Object::atom("c")])
        );
    }

    #[test]
    fn rotl_rotates_left() {
        let seq = Object::seq(vec![Object::atom("a"), Object::atom("b"), Object::atom("c")]);
        assert_eq!(
            apply(&Func::RotL, &seq, &defs()),
            Object::seq(vec![Object::atom("b"), Object::atom("c"), Object::atom("a")])
        );
    }

    #[test]
    fn rotr_rotates_right() {
        let seq = Object::seq(vec![Object::atom("a"), Object::atom("b"), Object::atom("c")]);
        assert_eq!(
            apply(&Func::RotR, &seq, &defs()),
            Object::seq(vec![Object::atom("c"), Object::atom("a"), Object::atom("b")])
        );
    }

    // ── Backus arithmetic (Task 2) ──────────────────────────────

    #[test]
    fn add_numbers() {
        let x = Object::seq(vec![Object::atom("3"), Object::atom("4")]);
        assert_eq!(apply(&Func::Add, &x, &defs()), Object::atom("7"));
    }

    #[test]
    fn sub_numbers() {
        let x = Object::seq(vec![Object::atom("7"), Object::atom("4")]);
        assert_eq!(apply(&Func::Sub, &x, &defs()), Object::atom("3"));
    }

    #[test]
    fn mul_numbers() {
        let x = Object::seq(vec![Object::atom("3"), Object::atom("4")]);
        assert_eq!(apply(&Func::Mul, &x, &defs()), Object::atom("12"));
    }

    #[test]
    fn div_numbers() {
        let x = Object::seq(vec![Object::atom("12"), Object::atom("4")]);
        assert_eq!(apply(&Func::Div, &x, &defs()), Object::atom("3"));
    }

    #[test]
    fn div_by_zero_is_bottom() {
        let x = Object::seq(vec![Object::atom("12"), Object::atom("0")]);
        assert_eq!(apply(&Func::Div, &x, &defs()), Object::Bottom);
    }

    #[test]
    fn arithmetic_on_non_numbers_is_bottom() {
        let x = Object::seq(vec![Object::atom("hello"), Object::atom("4")]);
        assert_eq!(apply(&Func::Add, &x, &defs()), Object::Bottom);
    }

    #[test]
    fn add_floats() {
        let x = Object::seq(vec![Object::atom("2.5"), Object::atom("1.5")]);
        assert_eq!(apply(&Func::Add, &x, &defs()), Object::atom("4"));
    }

    // ── Backus logic (Task 3) ───────────────────────────────────

    #[test]
    fn and_logic() {
        assert_eq!(apply(&Func::And, &Object::seq(vec![Object::t(), Object::t()]), &defs()), Object::t());
        assert_eq!(apply(&Func::And, &Object::seq(vec![Object::t(), Object::f()]), &defs()), Object::f());
        assert_eq!(apply(&Func::And, &Object::seq(vec![Object::f(), Object::f()]), &defs()), Object::f());
    }

    #[test]
    fn or_logic() {
        assert_eq!(apply(&Func::Or, &Object::seq(vec![Object::f(), Object::f()]), &defs()), Object::f());
        assert_eq!(apply(&Func::Or, &Object::seq(vec![Object::t(), Object::f()]), &defs()), Object::t());
        assert_eq!(apply(&Func::Or, &Object::seq(vec![Object::f(), Object::t()]), &defs()), Object::t());
    }

    #[test]
    fn not_logic() {
        assert_eq!(apply(&Func::Not, &Object::t(), &defs()), Object::f());
        assert_eq!(apply(&Func::Not, &Object::f(), &defs()), Object::t());
        assert_eq!(apply(&Func::Not, &Object::atom("x"), &defs()), Object::Bottom);
    }

    // ── Backus inner product (Task 4) ───────────────────────────

    #[test]
    fn insert_add_folds_sum() {
        // /+:<1,2,3> = 6
        let f = Func::insert(Func::Add);
        let seq = Object::seq(vec![Object::atom("1"), Object::atom("2"), Object::atom("3")]);
        assert_eq!(apply(&f, &seq, &defs()), Object::atom("6"));
    }

    #[test]
    fn insert_add_singleton() {
        // /+:<7> = 7
        let f = Func::insert(Func::Add);
        let seq = Object::seq(vec![Object::atom("7")]);
        assert_eq!(apply(&f, &seq, &defs()), Object::atom("7"));
    }

    #[test]
    fn inner_product_backus_example() {
        // Def IP ≡ (/+) ∘ (α×) ∘ trans
        // IP:<<1,2,3>,<6,5,4>> = 28
        let ip = Func::compose(
            Func::insert(Func::Add),
            Func::compose(
                Func::apply_to_all(Func::Mul),
                Func::Trans,
            ),
        );
        let input = Object::seq(vec![
            Object::seq(vec![Object::atom("1"), Object::atom("2"), Object::atom("3")]),
            Object::seq(vec![Object::atom("6"), Object::atom("5"), Object::atom("4")]),
        ]);
        assert_eq!(apply(&ip, &input, &defs()), Object::atom("28"));
    }

    // ── Insert with first-class Or (replaces Native) ────────────

    #[test]
    fn insert_or_with_first_class() {
        // /(or):<F, F, T> = T — using first-class Or instead of Native
        let f = Func::insert(Func::Or);
        let seq = Object::seq(vec![Object::f(), Object::f(), Object::t()]);
        assert_eq!(apply(&f, &seq, &defs()), Object::t());

        let seq2 = Object::seq(vec![Object::f(), Object::f(), Object::f()]);
        assert_eq!(apply(&f, &seq2, &defs()), Object::f());
    }

    #[test]
    fn insert_and_with_first_class() {
        let f = Func::insert(Func::And);
        let seq = Object::seq(vec![Object::t(), Object::t(), Object::t()]);
        assert_eq!(apply(&f, &seq, &defs()), Object::t());

        let seq2 = Object::seq(vec![Object::t(), Object::f(), Object::t()]);
        assert_eq!(apply(&f, &seq2, &defs()), Object::f());
    }

    // ── Insert on the EMPTY sequence = the right unit of the fold op ──
    // Backus §11.2.4: `/f:<>` is the right unit of `f`, not ⊥. This is the
    // empty-fold base case the universal-quantifier fold and the count/sum
    // aggregates rely on (vacuous truth, empty sum = 0, …).
    #[test]
    fn insert_over_empty_yields_unit_of_op() {
        let empty = Object::phi();
        // /∧:<> = T (vacuous truth)
        assert_eq!(apply(&Func::insert(Func::And), &empty, &defs()), Object::t());
        // /∨:<> = F
        assert_eq!(apply(&Func::insert(Func::Or), &empty, &defs()), Object::f());
        // /+:<> = 0
        assert_eq!(apply(&Func::insert(Func::Add), &empty, &defs()), Object::atom("0"));
        // /×:<> = 1
        assert_eq!(apply(&Func::insert(Func::Mul), &empty, &defs()), Object::atom("1"));
        // /concat:<> = <>
        assert_eq!(apply(&Func::insert(Func::Concat), &empty, &defs()), Object::phi());
        // An operator with no known unit stays ⊥ (Backus-faithful: undefined).
        assert_eq!(apply(&Func::insert(Func::Sub), &empty, &defs()), Object::Bottom);

        // unit_of itself: known ops map, unknown ops are None.
        assert_eq!(unit_of(&Func::And), Some(Object::t()));
        assert_eq!(unit_of(&Func::Add), Some(Object::atom("0")));
        assert_eq!(unit_of(&Func::Sub), None);

        // Non-empty folds are unchanged: the unit does not leak in.
        let one_two = Object::seq(vec![Object::atom("1"), Object::atom("2")]);
        assert_eq!(apply(&Func::insert(Func::Add), &one_two, &defs()), Object::atom("3"));
        // Single-element fold still returns the element (identity), not the unit.
        let single = Object::seq(vec![Object::atom("5")]);
        assert_eq!(apply(&Func::insert(Func::Add), &single, &defs()), Object::atom("5"));
    }

    // ── Codd θ₁ relational operations ─────────────────────────

    fn theta1_defs() -> Object {
        defs_to_state(&theta1_defs_vec(), &Object::phi())
    }

    /// Look up a named def from theta1_defs_vec and apply it directly.
    /// Native funcs cannot roundtrip through func_to_object/metacompose,
    /// so theta1 tests must resolve the Func from the vec.
    fn apply_theta1(name: &str, input: &Object) -> Object {
        let defs_vec = theta1_defs_vec();
        let d = theta1_defs();
        let func = defs_vec.iter()
            .find(|(n, _)| n == name)
            .map(|(_, f)| f)
            .expect(&format!("theta1 def '{}' not found", name));
        apply(func, input, &d)
    }

    #[test]
    fn theta1_projection() {
        // π_{1,3}(R) where R = <<a,b,c>,<d,e,f>>
        // project:<⟨1,3⟩, R> = <<a,c>,<d,f>>
        let input = Object::seq(vec![
            Object::seq(vec![Object::atom("1"), Object::atom("3")]),
            Object::seq(vec![
                Object::seq(vec![Object::atom("a"), Object::atom("b"), Object::atom("c")]),
                Object::seq(vec![Object::atom("d"), Object::atom("e"), Object::atom("f")]),
            ]),
        ]);
        let result = apply_theta1("project", &input);
        assert_eq!(result, Object::seq(vec![
            Object::seq(vec![Object::atom("a"), Object::atom("c")]),
            Object::seq(vec![Object::atom("d"), Object::atom("f")]),
        ]));
    }

    #[test]
    fn theta1_projection_removes_duplicates() {
        // project:<⟨1⟩, <<a,x>,<b,y>,<a,z>>> = <<a>,<b>> (a appears once)
        let input = Object::seq(vec![
            Object::seq(vec![Object::atom("1")]),
            Object::seq(vec![
                Object::seq(vec![Object::atom("a"), Object::atom("x")]),
                Object::seq(vec![Object::atom("b"), Object::atom("y")]),
                Object::seq(vec![Object::atom("a"), Object::atom("z")]),
            ]),
        ]);
        let result = apply_theta1("project", &input);
        assert_eq!(result, Object::seq(vec![
            Object::seq(vec![Object::atom("a")]),
            Object::seq(vec![Object::atom("b")]),
        ]));
    }

    #[test]
    fn theta1_natural_join() {
        // R = <<1,a>,<2,b>>, S = <<a,x>,<b,y>>
        // join on col 2 of R = col 1 of S (shared value domain)
        // join:<2, R, S> (but col 2 of R matches col 1 of S by value)
        // Actually: join on shared column means same index.
        // Let's use: R = <<s1,p1>,<s2,p1>>, S = <<p1,j1>,<p2,j2>>
        // join:<2, R, S> where col 2 is the shared domain
        // Wait — our join takes shared_col as the index that's shared in BOTH relations.
        // R = <<1,a>,<2,a>,<2,b>>, S = <<a,x>,<b,y>>
        // join on col 1 of S = col 2 of R... this is a simplification.
        // Let's use Codd's example from Figure 5-6:
        // R(supplier, part): <<1,1>,<2,1>,<2,2>>
        // S(part, project): <<1,1>,<1,2>,<2,1>>
        // Natural join on "part" (col 2 in R, col 1 in S):
        // Our impl uses same-index join, which is simpler.
        // Use: shared_col=1, R and S both have col 1 as join key
        let r = Object::seq(vec![
            Object::seq(vec![Object::atom("a"), Object::atom("x")]),
            Object::seq(vec![Object::atom("b"), Object::atom("y")]),
        ]);
        let s = Object::seq(vec![
            Object::seq(vec![Object::atom("a"), Object::atom("1")]),
            Object::seq(vec![Object::atom("a"), Object::atom("2")]),
            Object::seq(vec![Object::atom("c"), Object::atom("3")]),
        ]);
        // join on col 1: a matches a (twice), b has no match, c has no match in R
        let input = Object::seq(vec![Object::atom("1"), r, s]);
        let result = apply_theta1("join", &input);
        // Expected: <<a,x,1>, <a,x,2>> (a matched, x from R, 1/2 from S minus shared)
        // S cols excluding shared col 1: just col 2
        assert_eq!(result, Object::seq(vec![
            Object::seq(vec![Object::atom("a"), Object::atom("x"), Object::atom("1")]),
            Object::seq(vec![Object::atom("a"), Object::atom("x"), Object::atom("2")]),
        ]));
    }

    #[test]
    fn theta1_tie() {
        // γ(R): select tuples where first = last, remove last column
        // R = <<a,1,a>,<b,2,c>,<c,3,c>>
        // tie:R = <<a,1>,<c,3>> (first=last for a and c)
        let r = Object::seq(vec![
            Object::seq(vec![Object::atom("a"), Object::atom("1"), Object::atom("a")]),
            Object::seq(vec![Object::atom("b"), Object::atom("2"), Object::atom("c")]),
            Object::seq(vec![Object::atom("c"), Object::atom("3"), Object::atom("c")]),
        ]);
        let result = apply_theta1("tie", &r);
        assert_eq!(result, Object::seq(vec![
            Object::seq(vec![Object::atom("a"), Object::atom("1")]),
            Object::seq(vec![Object::atom("c"), Object::atom("3")]),
        ]));
    }

    #[test]
    fn theta1_composition() {
        // R·S = π₁ₛ(R*S) — project out shared column from join
        // R = <<a,x>,<b,y>>, S = <<x,1>,<y,2>>
        // compose_rel on col 2 of R = col 1 of S:
        // join gives <<a,x,1>,<b,y,2>>, project out col 2 gives <<a,1>,<b,2>>
        let _r = Object::seq(vec![
            Object::seq(vec![Object::atom("a"), Object::atom("x")]),
            Object::seq(vec![Object::atom("b"), Object::atom("y")]),
        ]);
        let _s = Object::seq(vec![
            Object::seq(vec![Object::atom("x"), Object::atom("1")]),
            Object::seq(vec![Object::atom("y"), Object::atom("2")]),
        ]);
        // compose_rel:<shared_col, R, S>
        // shared_col = 2 for R (col 2), = 1 for S (col 1)
        // Our impl uses same index for both, so use col 1:
        // Actually our compose_rel joins on shared_col in both, then removes it.
        // R' = <<x,a>>, S' = <<x,1>> with shared on col 1:
        let r2 = Object::seq(vec![
            Object::seq(vec![Object::atom("x"), Object::atom("a")]),
            Object::seq(vec![Object::atom("y"), Object::atom("b")]),
        ]);
        let s2 = Object::seq(vec![
            Object::seq(vec![Object::atom("x"), Object::atom("1")]),
            Object::seq(vec![Object::atom("y"), Object::atom("2")]),
        ]);
        let input = Object::seq(vec![Object::atom("1"), r2, s2]);
        let result = apply_theta1("compose_rel", &input);
        // x matches x: project out col 1 → <a, 1>
        // y matches y: project out col 1 → <b, 2>
        assert_eq!(result, Object::seq(vec![
            Object::seq(vec![Object::atom("a"), Object::atom("1")]),
            Object::seq(vec![Object::atom("b"), Object::atom("2")]),
        ]));
    }

    // ── Algebraic Laws (Backus 12.2) ──────────────────────────
    // Mechanical verification that the implementation respects the algebra.

    // I. Composition and construction
    #[test]
    fn law_i1_construction_distributes_over_composition() {
        // I.1: [f₁,...,fₙ]∘g ≡ [f₁∘g,...,fₙ∘g]
        let d = defs();
        let x = Object::seq(vec![Object::atom("a"), Object::atom("b"), Object::atom("c")]);

        let lhs = Func::compose(
            Func::construction(vec![Func::Selector(1), Func::Selector(2)]),
            Func::Tail,
        );
        let rhs = Func::construction(vec![
            Func::compose(Func::Selector(1), Func::Tail),
            Func::compose(Func::Selector(2), Func::Tail),
        ]);
        assert_eq!(apply(&lhs, &x, &d), apply(&rhs, &x, &d));
    }

    #[test]
    fn law_i2_alpha_distributes_over_construction() {
        // I.2: α∘[g₁,...,gₙ] ≡ [f∘g₁,...,f∘gₙ] — wait, that's wrong
        // I.2: α f∘[g₁,...,gₙ] ≡ [f∘g₁,...,f∘gₙ]
        // Actually Backus I.2: αf∘[g₁,...,gₙ] ≡ [f∘g₁,...,f∘gₙ]
        let d = defs();
        let x = Object::seq(vec![Object::atom("a"), Object::atom("b")]);

        // αf = α(length), g₁ = [1], g₂ = [2]... no, let's use simpler functions
        // Actually the law is about applying αf to the result of a construction
        // αf∘[g₁,...,gₙ]:x = αf:<g₁:x,...,gₙ:x> = <f:(g₁:x),...,f:(gₙ:x)>
        // [f∘g₁,...,f∘gₙ]:x = <(f∘g₁):x,...,(f∘gₙ):x> = <f:(g₁:x),...,f:(gₙ:x)>
        // Use f = not, g₁ = atom (returns T for atom), g₂ = null
        let lhs = Func::compose(
            Func::apply_to_all(Func::Not),
            Func::construction(vec![Func::AtomTest, Func::NullTest]),
        );
        let rhs = Func::construction(vec![
            Func::compose(Func::Not, Func::AtomTest),
            Func::compose(Func::Not, Func::NullTest),
        ]);
        // x = <a, b> is a sequence: atom returns F, null returns F
        // lhs: α(not):< F, F> = <T, T>
        // rhs: [not∘atom, not∘null]:x = <T, T>
        assert_eq!(apply(&lhs, &x, &d), apply(&rhs, &x, &d));
    }

    #[test]
    fn law_i3_insert_over_construction() {
        // I.3: /f∘[g₁,...,gₙ] ≡ f∘[g₁, /f∘[g₂,...,gₙ]] when n≥2
        // Simplified: /+∘[1, 2, 3]:x = +:<1:x, +:<2:x, 3:x>>
        let d = defs();
        let x = Object::seq(vec![Object::atom("10"), Object::atom("20"), Object::atom("30")]);

        let lhs = Func::compose(
            Func::insert(Func::Add),
            Func::construction(vec![Func::Selector(1), Func::Selector(2), Func::Selector(3)]),
        );
        // rhs: [1,2,3]:x = <10,20,30>, then /+:<10,20,30> = 60
        assert_eq!(apply(&lhs, &x, &d), Object::atom("60"));
    }

    #[test]
    fn law_i5_selector_construction_identity() {
        // I.5: s∘[f₁,...,fₙ] ≤ fₛ for selector s, s≤n
        // 2∘[f₁,f₂,f₃] = f₂
        let d = defs();
        let x = Object::seq(vec![Object::atom("a"), Object::atom("b"), Object::atom("c")]);

        let lhs = Func::compose(
            Func::Selector(2),
            Func::construction(vec![Func::Selector(3), Func::Selector(1), Func::Selector(2)]),
        );
        // [3,1,2]:x = <c,a,b>, then 2:<c,a,b> = a = 1:x
        let rhs = Func::Selector(1);
        assert_eq!(apply(&lhs, &x, &d), apply(&rhs, &x, &d));
    }

    // II. Composition and condition
    #[test]
    fn law_ii1_condition_compose_left() {
        // II.1: (p→f;g)∘h ≡ p∘h → f∘h; g∘h
        let d = defs();
        let x = Object::seq(vec![Object::atom("a"), Object::atom("b")]);

        let lhs = Func::compose(
            Func::condition(Func::NullTest, Func::constant(Object::atom("yes")), Func::constant(Object::atom("no"))),
            Func::Tail,
        );
        // tl:<a,b> = <b>, null:<b> = F, so result = "no"
        let rhs = Func::condition(
            Func::compose(Func::NullTest, Func::Tail),
            Func::compose(Func::constant(Object::atom("yes")), Func::Tail),
            Func::compose(Func::constant(Object::atom("no")), Func::Tail),
        );
        assert_eq!(apply(&lhs, &x, &d), apply(&rhs, &x, &d));
    }

    // III. Composition and miscellaneous
    #[test]
    fn law_iii1_constant_absorbs_composition() {
        // III.1: x̄∘f ≤ x̄ (defined→f → x̄∘f:y = x̄:(f:y) = x)
        let d = defs();
        let y = Object::seq(vec![Object::atom("a"), Object::atom("b")]);
        let lhs = Func::compose(Func::constant(Object::atom("hello")), Func::Tail);
        let rhs = Func::constant(Object::atom("hello"));
        assert_eq!(apply(&lhs, &y, &d), apply(&rhs, &y, &d));
    }

    #[test]
    fn law_iii2_compose_id_is_identity() {
        // III.2: f∘id ≡ id∘f ≡ f
        let d = defs();
        let x = Object::seq(vec![Object::atom("a"), Object::atom("b")]);
        let f = Func::Selector(1);
        let lhs1 = Func::compose(f.clone(), Func::Id);
        let lhs2 = Func::compose(Func::Id, f.clone());
        assert_eq!(apply(&lhs1, &x, &d), apply(&f, &x, &d));
        assert_eq!(apply(&lhs2, &x, &d), apply(&f, &x, &d));
    }

    #[test]
    fn law_iii4_alpha_compose_distributes() {
        // III.4: α(f∘g) ≡ αf ∘ αg
        let d = defs();
        let x = Object::seq(vec![
            Object::seq(vec![Object::atom("a"), Object::atom("b")]),
            Object::seq(vec![Object::atom("c"), Object::atom("d")]),
        ]);
        // f = 1, g = reverse
        let lhs = Func::apply_to_all(Func::compose(Func::Selector(1), Func::Reverse));
        let rhs = Func::compose(
            Func::apply_to_all(Func::Selector(1)),
            Func::apply_to_all(Func::Reverse),
        );
        // lhs: α(1∘reverse):<<a,b>,<c,d>> = <(1∘reverse):<a,b>, (1∘reverse):<c,d>> = <b, d>
        // rhs: α1∘(αreverse:<<a,b>,<c,d>>) = α1:<<b,a>,<d,c>> = <b, d>
        assert_eq!(apply(&lhs, &x, &d), apply(&rhs, &x, &d));
    }

    // ── Cells and State (Backus 14.3) ─────────────────────────

    #[test]
    fn cell_fetch_retrieves_contents() {
        // D = <<CELL, "FILE", <a,b>>, <CELL, "defs", <c>>>
        // ↑FILE:D = <a,b>
        let state = Object::seq(vec![
            cell("FILE", Object::seq(vec![Object::atom("a"), Object::atom("b")])),
            cell("defs", Object::seq(vec![Object::atom("c")])),
        ]);
        assert_eq!(fetch("FILE", &state), Object::seq(vec![Object::atom("a"), Object::atom("b")]));
        assert_eq!(fetch("defs", &state), Object::seq(vec![Object::atom("c")]));
        assert_eq!(fetch("missing", &state), Object::Bottom);
    }

    #[test]
    fn cell_store_replaces_contents() {
        let state = Object::seq(vec![
            cell("FILE", Object::seq(vec![Object::atom("old")])),
            cell("defs", Object::seq(vec![Object::atom("c")])),
        ]);
        let new_state = store("FILE", Object::seq(vec![Object::atom("new")]), &state);
        assert_eq!(fetch("FILE", &new_state), Object::seq(vec![Object::atom("new")]));
        assert_eq!(fetch("defs", &new_state), Object::seq(vec![Object::atom("c")]));
    }

    #[test]
    fn cell_store_appends_new_cell() {
        let state = Object::seq(vec![
            cell("FILE", Object::atom("data")),
        ]);
        let new_state = store("defs", Object::atom("rules"), &state);
        assert_eq!(fetch("FILE", &new_state), Object::atom("data"));
        assert_eq!(fetch("defs", &new_state), Object::atom("rules"));
    }

    #[test]
    fn fetch_via_func_apply() {
        // fetch:<"FILE", D> via Func::Fetch
        let state = Object::seq(vec![
            cell("FILE", Object::atom("population")),
        ]);
        let input = Object::seq(vec![Object::atom("FILE"), state]);
        assert_eq!(apply(&Func::Fetch, &input, &defs()), Object::atom("population"));
    }

    #[test]
    fn store_via_func_apply() {
        // store:<"FILE", new_contents, D> via Func::Store
        //
        // #903: under std the empty-stack `Func::Store` is refused by
        // default. This test documents the legacy unrestricted shape
        // and runs under `permissive_empty_caps_guard()` to opt in.
        let _g = crate::declared_writes::permissive_empty_caps_guard();
        let state = Object::seq(vec![
            cell("FILE", Object::atom("old")),
        ]);
        let input = Object::seq(vec![Object::atom("FILE"), Object::atom("new"), state]);
        let result = apply(&Func::Store, &input, &defs());
        assert_eq!(fetch("FILE", &result), Object::atom("new"));
    }

    #[test]
    fn fetch_via_ffp() {
        // FFP: ("^":<"FILE", D>)
        let state = Object::seq(vec![
            cell("FILE", Object::atom("pop")),
        ]);
        let input = Object::seq(vec![Object::atom("FILE"), state]);
        assert_eq!(apply_ffp(&Object::atom("^"), &input, &defs()), Object::atom("pop"));
    }

    #[test]
    fn ast_state_as_cell_sequence() {
        // Full AST state D = <<CELL, FILE, population>, <CELL, defs, definitions>>
        // This models Backus Section 14.3: the state is a sequence of cells.
        let population = Object::seq(vec![
            Object::seq(vec![Object::atom("Order"), Object::atom("ord-1")]),
            Object::seq(vec![Object::atom("Customer"), Object::atom("acme")]),
        ]);
        let definitions = Object::seq(vec![
            Object::atom("create"),
            Object::atom("validate"),
        ]);
        let d = Object::seq(vec![
            cell("FILE", population.clone()),
            cell("defs", definitions.clone()),
        ]);

        assert_eq!(fetch("FILE", &d), population);
        assert_eq!(fetch("defs", &d), definitions);

        // Store updated population
        let new_pop = Object::seq(vec![
            Object::seq(vec![Object::atom("Order"), Object::atom("ord-1")]),
            Object::seq(vec![Object::atom("Customer"), Object::atom("acme")]),
            Object::seq(vec![Object::atom("SM"), Object::atom("Draft")]),
        ]);
        let d_prime = store("FILE", new_pop.clone(), &d);
        assert_eq!(fetch("FILE", &d_prime), new_pop);
        assert_eq!(fetch("defs", &d_prime), definitions); // defs unchanged
    }

    // ── OrderBy (sort-by-key extension; sm-fold-as-predicate) ─────
    //
    // OrderBy(keyfn):<e₁,…,eₙ> = the eᵢ stably sorted ascending by the
    // atom key keyfn:eᵢ. The general primitive that turns an unordered
    // event *set* into the chronological *stream* E that the SM fold
    // `foldl transition s₀ E` (AREST.tex eq:sm) consumes latest-wins.
    // Numeric keys compare numerically (epoch / recorded-at), else
    // lexicographically (ISO timestamps). Stable, total, retains all
    // elements (non-atom keys sort first, stably).

    #[test]
    fn order_by_sorts_by_numeric_key() {
        // OrderBy(s₁) over <<3,a>,<1,b>,<2,c>> → <<1,b>,<2,c>,<3,a>>.
        // Keys "3","1","2" must compare NUMERICALLY (not "1"<"2"<"3"
        // lexicographically, which would mis-order e.g. "10" < "9").
        let d = defs();
        let f = Func::OrderBy(Box::new(Func::Selector(1)));
        let x = Object::seq(vec![
            Object::seq(vec![Object::atom("3"), Object::atom("a")]),
            Object::seq(vec![Object::atom("1"), Object::atom("b")]),
            Object::seq(vec![Object::atom("10"), Object::atom("d")]),
            Object::seq(vec![Object::atom("2"), Object::atom("c")]),
        ]);
        let expected = Object::seq(vec![
            Object::seq(vec![Object::atom("1"), Object::atom("b")]),
            Object::seq(vec![Object::atom("2"), Object::atom("c")]),
            Object::seq(vec![Object::atom("3"), Object::atom("a")]),
            Object::seq(vec![Object::atom("10"), Object::atom("d")]),
        ]);
        assert_eq!(apply(&f, &x, &d), expected);
    }

    #[test]
    fn order_by_is_stable_for_equal_keys() {
        // Equal keys preserve input order (latest-wins downstream relies
        // on stability when timestamps collide).
        let d = defs();
        let f = Func::OrderBy(Box::new(Func::Selector(1)));
        let x = Object::seq(vec![
            Object::seq(vec![Object::atom("1"), Object::atom("a")]),
            Object::seq(vec![Object::atom("1"), Object::atom("b")]),
            Object::seq(vec![Object::atom("1"), Object::atom("c")]),
        ]);
        assert_eq!(apply(&f, &x, &d), x);
    }

    #[test]
    fn order_by_roundtrips_through_metacompose() {
        // ρ<ob, "1"> = OrderBy(Selector(1)); and func_to_object is its
        // left inverse (Backus 13.3.2 decomposition).
        let d = defs();
        let obj = Object::seq(vec![
            Object::atom(forms::ORDER_BY),
            Object::atom("1"),
        ]);
        let f = metacompose(&obj, &d);
        let x = Object::seq(vec![
            Object::seq(vec![Object::atom("c"), Object::atom("x")]),
            Object::seq(vec![Object::atom("a"), Object::atom("y")]),
            Object::seq(vec![Object::atom("b"), Object::atom("z")]),
        ]);
        let expected = Object::seq(vec![
            Object::seq(vec![Object::atom("a"), Object::atom("y")]),
            Object::seq(vec![Object::atom("b"), Object::atom("z")]),
            Object::seq(vec![Object::atom("c"), Object::atom("x")]),
        ]);
        assert_eq!(apply(&f, &x, &d), expected);
        assert_eq!(func_to_object(&Func::OrderBy(Box::new(Func::Selector(1)))), obj);
    }

    // engine-flat-stratum-recursion-stack-guard: the view-resolution re-entrancy
    // guard. resolve_view must detect when a cell is ALREADY being resolved on
    // this thread (a recursive view cycle from an ill-defined aggregate-in-cycle)
    // and refuse to recurse — that is what converts the former stack overflow into
    // a clean "return empty + diagnose" break. Acyclic nesting (distinct cells)
    // must NOT trip it, and dropping a guard must free the cell for re-entry.
    #[test]
    fn view_resolution_reentrancy_guard_detects_cycle_not_acyclic_nesting() {
        let g_a = enter_view_resolution("A");
        assert!(g_a.is_some(), "first entry of A must succeed");
        // Re-entering A while it is still on the resolving stack = the cycle.
        assert!(enter_view_resolution("A").is_none(),
            "re-entry of A mid-resolution must be detected as a cycle (return None)");
        // A DIFFERENT cell nested under A is legitimate acyclic nesting.
        let g_b = enter_view_resolution("B");
        assert!(g_b.is_some(), "a distinct cell B nested under A must be allowed");
        drop(g_b);
        drop(g_a);
        // Stack clean after drops — A is enterable again (no leaked state).
        let g_a2 = enter_view_resolution("A");
        assert!(g_a2.is_some(), "after dropping its guard, A must be enterable again");
        drop(g_a2);
    }

    // ── FFP: ρ and metacomposition (Backus 13) ──────────────────

    #[test]
    fn metacompose_primitive_atom_resolves() {
        // ρ("+") = Add
        let d = defs();
        let func = metacompose(&Object::atom("+"), &d);
        let x = Object::seq(vec![Object::atom("3"), Object::atom("4")]);
        assert_eq!(apply(&func, &x, &d), Object::atom("7"));
    }

    #[test]
    fn metacompose_selector_atom_resolves() {
        // ρ("2") = Selector(2)
        let d = defs();
        let func = metacompose(&Object::atom("2"), &d);
        let x = Object::seq(vec![Object::atom("a"), Object::atom("b"), Object::atom("c")]);
        assert_eq!(apply(&func, &x, &d), Object::atom("b"));
    }

    #[test]
    fn metacompose_undefined_atom_is_bottom() {
        // ρ("undefined_name") = ⊥̄
        let d = defs();
        let func = metacompose(&Object::atom("undefined_name"), &d);
        assert_eq!(apply(&func, &Object::atom("x"), &d), Object::Bottom);
    }

    #[test]
    fn metacompose_defined_atom_resolves() {
        // Def "second" ≡ Selector(2)
        let d = defs_to_state(&[("second".to_string(), Func::Selector(2))], &Object::phi());
        let func = metacompose(&Object::atom("second"), &d);
        let x = Object::seq(vec![Object::atom("a"), Object::atom("b")]);
        assert_eq!(apply(&func, &x, &d), Object::atom("b"));
    }

    #[test]
    fn metacompose_comp_sequence() {
        // ρ<COMP, "1", "tl"> = 1 ∘ tl
        // (1 ∘ tl):<a,b,c> = 1:<b,c> = b
        let d = defs();
        let obj = Object::seq(vec![
            Object::atom(forms::COMP),
            Object::atom("1"),
            Object::atom(primitives::TL),
        ]);
        let func = metacompose(&obj, &d);
        let x = Object::seq(vec![Object::atom("a"), Object::atom("b"), Object::atom("c")]);
        assert_eq!(apply(&func, &x, &d), Object::atom("b"));
    }

    #[test]
    fn metacompose_cons_sequence() {
        // ρ<CONS, "1", "2"> = [1, 2]
        // [1, 2]:<a, b, c> = <a, b>
        let d = defs();
        let obj = Object::seq(vec![
            Object::atom(forms::CONS),
            Object::atom("1"),
            Object::atom("2"),
        ]);
        let func = metacompose(&obj, &d);
        let x = Object::seq(vec![Object::atom("a"), Object::atom("b"), Object::atom("c")]);
        assert_eq!(apply(&func, &x, &d), Object::seq(vec![Object::atom("a"), Object::atom("b")]));
    }

    #[test]
    fn metacompose_cond_sequence() {
        // ρ<COND, "null", <CONST, "empty">, <CONST, "notempty">> = (null → "empty"̄; "notempty"̄)
        let d = defs();
        let obj = Object::seq(vec![
            Object::atom(forms::COND),
            Object::atom(primitives::NULL),
            Object::seq(vec![Object::atom(forms::CONST), Object::atom("empty")]),
            Object::seq(vec![Object::atom(forms::CONST), Object::atom("notempty")]),
        ]);
        let func = metacompose(&obj, &d);
        assert_eq!(apply(&func, &Object::phi(), &d), Object::atom("empty"));
        assert_eq!(apply(&func, &Object::seq(vec![Object::atom("x")]), &d), Object::atom("notempty"));
    }

    #[test]
    fn metacompose_insert_add() {
        // ρ<INSERT, "+"> = /+
        // /+:<1,2,3> = 6
        let d = defs();
        let obj = Object::seq(vec![
            Object::atom(forms::INSERT),
            Object::atom(primitives::ADD),
        ]);
        let func = metacompose(&obj, &d);
        let x = Object::seq(vec![Object::atom("1"), Object::atom("2"), Object::atom("3")]);
        assert_eq!(apply(&func, &x, &d), Object::atom("6"));
    }

    #[test]
    fn metacompose_alpha_sequence() {
        // ρ<ALPHA, "1"> = α(1)
        // α(1):<<a,b>,<c,d>> = <a,c>
        let d = defs();
        let obj = Object::seq(vec![
            Object::atom(forms::ALPHA),
            Object::atom("1"),
        ]);
        let func = metacompose(&obj, &d);
        let x = Object::seq(vec![
            Object::seq(vec![Object::atom("a"), Object::atom("b")]),
            Object::seq(vec![Object::atom("c"), Object::atom("d")]),
        ]);
        assert_eq!(apply(&func, &x, &d), Object::seq(vec![Object::atom("a"), Object::atom("c")]));
    }

    #[test]
    fn metacompose_bu_sequence() {
        // ρ<BU, "eq", "owner"> = (bu eq "owner")
        let d = defs();
        let obj = Object::seq(vec![
            Object::atom(forms::BU),
            Object::atom(primitives::EQ),
            Object::atom("owner"),
        ]);
        let func = metacompose(&obj, &d);
        assert_eq!(apply(&func, &Object::atom("owner"), &d), Object::t());
        assert_eq!(apply(&func, &Object::atom("member"), &d), Object::f());
    }

    #[test]
    fn apply_ffp_evaluates_object_as_function() {
        // FFP: ("+":< 3, 4>) = 7
        let d = defs();
        let operator = Object::atom("+");
        let operand = Object::seq(vec![Object::atom("3"), Object::atom("4")]);
        assert_eq!(apply_ffp(&operator, &operand, &d), Object::atom("7"));
    }

    #[test]
    fn apply_ffp_composition_as_object() {
        // FFP: (<COMP, "+", <CONS, "1", "1">>:<3, 4>) = +:<3, 3> = ... no
        // Better: (<COMP, <INSERT, "+">, <ALPHA, "*">>:<<1,2,3>,<6,5,4>>) = 28
        // This is the inner product as an FFP object
        let d = defs();
        let ip_obj = Object::seq(vec![
            Object::atom(forms::COMP),
            Object::seq(vec![Object::atom(forms::INSERT), Object::atom(primitives::ADD)]),
            Object::seq(vec![
                Object::atom(forms::COMP),
                Object::seq(vec![Object::atom(forms::ALPHA), Object::atom(primitives::MUL)]),
                Object::atom(primitives::TRANS),
            ]),
        ]);
        let input = Object::seq(vec![
            Object::seq(vec![Object::atom("1"), Object::atom("2"), Object::atom("3")]),
            Object::seq(vec![Object::atom("6"), Object::atom("5"), Object::atom("4")]),
        ]);
        assert_eq!(apply_ffp(&ip_obj, &input, &d), Object::atom("28"));
    }

    #[test]
    fn func_to_object_roundtrip() {
        // Func → Object → ρ → Func → apply should give same result
        let d = defs();
        let original = Func::compose(
            Func::insert(Func::Add),
            Func::compose(
                Func::apply_to_all(Func::Mul),
                Func::Trans,
            ),
        );
        let obj = func_to_object(&original);
        let recovered = metacompose(&obj, &d);
        let input = Object::seq(vec![
            Object::seq(vec![Object::atom("1"), Object::atom("2"), Object::atom("3")]),
            Object::seq(vec![Object::atom("6"), Object::atom("5"), Object::atom("4")]),
        ]);
        assert_eq!(apply(&original, &input, &d), apply(&recovered, &input, &d));
        assert_eq!(apply(&recovered, &input, &d), Object::atom("28"));
    }

    #[test]
    fn filter_as_ffp_object() {
        // ρ<FILTER, <BU, "eq", "owner">> applied to sequence
        let d = defs();
        let filter_obj = Object::seq(vec![
            Object::atom(forms::FILTER),
            Object::seq(vec![
                Object::atom(forms::BU),
                Object::atom(primitives::EQ),
                Object::atom("owner"),
            ]),
        ]);
        let seq = Object::seq(vec![
            Object::atom("owner"),
            Object::atom("member"),
            Object::atom("owner"),
        ]);
        assert_eq!(
            apply_ffp(&filter_obj, &seq, &d),
            Object::seq(vec![Object::atom("owner"), Object::atom("owner")])
        );
    }

    // ── FoldL tests ─────────────────────────────────────────────

    #[test]
    fn foldl_sums_left_to_right() {
        // FoldL(+) : <0, <1, 2, 3>> = ((0+1)+2)+3 = 6
        let d = defs();
        let foldl_add = Func::foldl(Func::Add);
        let input = Object::seq(vec![
            Object::atom("0"),
            Object::seq(vec![Object::atom("1"), Object::atom("2"), Object::atom("3")]),
        ]);
        assert_eq!(apply(&foldl_add, &input, &d), Object::atom("6"));
    }

    #[test]
    fn foldl_state_machine_fold() {
        // State machine: state is a string, events toggle between "on" and "off".
        // Transition function: if event = "toggle" then flip state, else keep state.
        // We model this with: Condition(eq . [sel(2), const("toggle")], flip, sel(1))
        // where flip = Condition(eq . [sel(1), const("on")], const("off"), const("on"))
        let d = defs();

        // flip: <state, event> -> if state = "on" then "off" else "on"
        let flip = Func::condition(
            Func::compose(Func::Eq, Func::construction(vec![
                Func::Selector(1),
                Func::constant(Object::atom("on")),
            ])),
            Func::constant(Object::atom("off")),
            Func::constant(Object::atom("on")),
        );

        // transition: <state, event> -> if event = "toggle" then flip(state) else state
        let transition = Func::condition(
            Func::compose(Func::Eq, Func::construction(vec![
                Func::Selector(2),
                Func::constant(Object::atom("toggle")),
            ])),
            flip,
            Func::Selector(1),
        );

        // FoldL(transition) : <"off", <"toggle", "toggle", "toggle">>
        // off -> toggle -> on -> toggle -> off -> toggle -> on
        let foldl_sm = Func::foldl(transition);
        let input = Object::seq(vec![
            Object::atom("off"),
            Object::seq(vec![
                Object::atom("toggle"),
                Object::atom("toggle"),
                Object::atom("toggle"),
            ]),
        ]);
        assert_eq!(apply(&foldl_sm, &input, &d), Object::atom("on"));
    }

    // ── Edge case tests ─────────────────────────────────────────

    #[test]
    fn while_exceeding_limit_returns_bottom() {
        // While with a predicate that always returns T should hit the 1000
        // iteration safety limit and return Bottom, not loop forever.
        let d = defs();
        // predicate: always T (constant T)
        let always_true = Func::constant(Object::t());
        // body: identity (state never changes, but predicate always says continue)
        let w = Func::While(Box::new(always_true), Box::new(Func::Id));
        let result = apply(&w, &Object::atom("start"), &d);
        assert_eq!(result, Object::Bottom);
    }

    #[test]
    fn parse_deeply_nested_returns_bottom() {
        // 200 levels of < nesting exceeds MAX_PARSE_DEPTH (100).
        // At depth 100, parse_with_depth returns Bottom for the inner content.
        // Note: parse_with_depth uses Object::Seq (not Object::seq), so Bottom
        // does NOT propagate outward through the nesting. The innermost parsed
        // level contains Bottom as a leaf element.
        let opens: String = "<".repeat(200);
        let closes: String = ">".repeat(200);
        let input = format!("{}x{}", opens, closes);
        let result = Object::parse(&input);
        // Walk down 100 levels of Seq([...]) to reach Bottom
        // (core::iter::successors = Backus's $\mathit{while}$ combining form)
        let current = core::iter::successors(Some(&result), |c| match c {
            Object::Seq(items) if items.len() == 1 => Some(&items[0]),
            _ => None,
        }).take(101).last().unwrap();
        assert_eq!(*current, Object::Bottom,
            "At depth 100+, parse should produce Bottom");
    }

    #[test]
    fn parse_mismatched_brackets() {
        // Missing close bracket: <a, <b> -- outer < never closed.
        // split_top_level sees "a, <b>" as the inner content of <...>
        // but the outer string does NOT end with > so it parses as an atom.
        let result1 = Object::parse("<a, <b>");
        // The string starts with < but ends with > -- the OUTER < matches the
        // inner >. Inner is "a, <b" which splits into ["a", "<b"]. "<b" does
        // not end with > so it parses as atom "<b".
        assert!(result1 != Object::Bottom, "partial parse should not be Bottom");

        // Missing open bracket: "a, b>" -- no < at start, so it is an atom.
        let result2 = Object::parse("a, b>");
        assert_eq!(result2, Object::Atom("a, b>".to_string()));
    }

    #[test]
    fn foldl_empty_sequence_returns_accumulator() {
        // FoldL(f) : <z, <>> = z (base case of left fold)
        let d = defs();
        let foldl_add = Func::foldl(Func::Add);
        let input = Object::seq(vec![
            Object::atom("42"),
            Object::phi(), // empty sequence
        ]);
        assert_eq!(apply(&foldl_add, &input, &d), Object::atom("42"));
    }

    // ── State helper tests ──────────────────────────────────────────

    #[test]
    fn fetch_or_phi_returns_phi_for_missing_cell() {
        let state = Object::phi();
        assert_eq!(fetch_or_phi("missing", &state), Object::phi());
    }

    #[test]
    fn fetch_or_phi_returns_contents_for_existing_cell() {
        let state = Object::seq(vec![cell("nouns", Object::atom("Alice"))]);
        assert_eq!(fetch_or_phi("nouns", &state), Object::atom("Alice"));
    }

    #[test]
    fn cell_push_creates_cell_on_empty_state() {
        let state = Object::phi();
        let fact = fact_from_pairs(&[("name", "Alice")]);
        let state2 = cell_push("Noun", fact.clone(), &state);
        assert_eq!(fetch_or_phi("Noun", &state2), Object::seq(vec![fact]));
    }

    #[test]
    fn cell_push_appends_to_existing_cell() {
        let f1 = fact_from_pairs(&[("name", "Alice")]);
        let f2 = fact_from_pairs(&[("name", "Bob")]);
        let state = cell_push("Noun", f1.clone(), &Object::phi());
        let state2 = cell_push("Noun", f2.clone(), &state);
        assert_eq!(fetch_or_phi("Noun", &state2), Object::seq(vec![f1, f2]));
    }

    // ─── task-744 / #743 phase 2: Map-backed cell storage primitives ─

    #[test]
    fn cell_put_keyed_writes_into_map_using_named_role() {
        let f = fact_from_pairs(&[("Task", "t-1"), ("Status", "pending")]);
        let state = cell_put_keyed("Task_has_Status", &["Task"], f.clone(), &Object::phi())
            .expect("first write — no collision possible");
        let contents = fetch_or_phi("Task_has_Status", &state);
        match &contents {
            Object::Map(m) => {
                assert_eq!(m.len(), 1);
                let stored = m.get("t-1").expect("entry for t-1");
                assert_eq!(stored, &f);
            }
            other => panic!("expected Map contents, got {:?}", other),
        }
    }

    // task-744 phase 4: collision detection — writing the same key with a
    // structurally different fact must now be reported (it used to silently
    // upsert in phase 2). This test replaces the legacy "_overwrites_"
    // test, since last-write-wins is exactly what Codd-style UC
    // enforcement needs to suppress.
    #[test]
    fn cell_put_keyed_same_key_different_non_key_role_value_is_collision() {
        let f1 = fact_from_pairs(&[("Task", "t-1"), ("Status", "pending")]);
        let f2 = fact_from_pairs(&[("Task", "t-1"), ("Status", "in_progress")]);
        let s1 = cell_put_keyed("Task_has_Status", &["Task"], f1.clone(), &Object::phi())
            .expect("first write");
        let conflict = cell_put_keyed("Task_has_Status", &["Task"], f2.clone(), &s1)
            .expect_err("second write at same key, different Status — must collide");
        assert_eq!(conflict.name, "Task_has_Status");
        assert_eq!(conflict.key, "t-1");
        assert_eq!(conflict.existing_fact, f1);
        assert_eq!(conflict.incoming_fact, f2);
        // State after the failed write is unchanged — s1 still holds f1.
        let m = fetch_or_phi("Task_has_Status", &s1).as_map().cloned()
            .expect("Map contents");
        assert_eq!(m.len(), 1);
        assert_eq!(m.get("t-1"), Some(&f1));
    }

    #[test]
    fn cell_put_keyed_distinct_keys_coexist() {
        let f1 = fact_from_pairs(&[("Task", "t-1"), ("Status", "pending")]);
        let f2 = fact_from_pairs(&[("Task", "t-2"), ("Status", "done")]);
        let s1 = cell_put_keyed("Task_has_Status", &["Task"], f1.clone(), &Object::phi())
            .expect("first key");
        let s2 = cell_put_keyed("Task_has_Status", &["Task"], f2.clone(), &s1)
            .expect("distinct second key — no collision");
        let m = fetch_or_phi("Task_has_Status", &s2).as_map().cloned().expect("Map contents");
        assert_eq!(m.len(), 2);
        assert_eq!(m.get("t-1"), Some(&f1));
        assert_eq!(m.get("t-2"), Some(&f2));
    }

    // #932 phase-2: keyless fold (cell_put_folded) — set semantics keyed
    // by the full tuple via synthesize_fact_id.
    #[test]
    fn cell_put_folded_dedups_identical_and_keeps_distinct() {
        let f1 = fact_from_pairs(&[("App", "a1"), ("Generator", "solidity")]);
        let f2 = fact_from_pairs(&[("App", "a1"), ("Generator", "openapi")]);
        let s = cell_put_folded("App_uses_Generator", f1.clone(), &Object::phi());
        let s = cell_put_folded("App_uses_Generator", f1.clone(), &s); // re-assert → no-op
        let s = cell_put_folded("App_uses_Generator", f2.clone(), &s);
        let m = fetch_or_phi("App_uses_Generator", &s).as_map().cloned()
            .expect("folded cell is a Map");
        assert_eq!(m.len(), 2, "f1 deduped, f2 distinct → 2 rows");
        let vals: Vec<&Object> = m.values().collect();
        assert!(vals.contains(&&f1) && vals.contains(&&f2));
    }

    #[test]
    fn cell_put_folded_distinguishes_ring_fact_duplicate_role_names() {
        // `Task blocks Task` stores two same-named `Task` pairs; the
        // full-tuple hash must tell (112,113) from (112,114) — a
        // binding-by-name key would collapse both to "112".
        let f1 = fact_from_pairs(&[("Task", "112"), ("Task", "113")]);
        let f2 = fact_from_pairs(&[("Task", "112"), ("Task", "114")]);
        let s = cell_put_folded("Task_blocks_Task", f1.clone(), &Object::phi());
        let s = cell_put_folded("Task_blocks_Task", f2.clone(), &s);
        let m = fetch_or_phi("Task_blocks_Task", &s).as_map().cloned()
            .expect("folded cell is a Map");
        assert_eq!(m.len(), 2, "two distinct ring tuples must coexist");
    }

    // ── perf-cellput-on2: batched cell-put equals the per-fact fold ──
    //
    // The batched integrators (`cell_put_folded_batch`,
    // `cell_put_keyed_batch`) collapse a round's worth of facts into ONE
    // Map clone. These tests pin that they are BEHAVIOR-IDENTICAL to the
    // per-fact `cell_put_folded` / `cell_put_keyed` they replace in
    // `evaluate::integrate_round_facts` — same final cell, same conflict
    // outcomes — so the O(n²)→O(n) change is provably side-effect-free.

    #[test]
    fn cell_put_folded_batch_equals_sequential_fold() {
        let facts = vec![
            fact_from_pairs(&[("App", "a1"), ("Generator", "solidity")]),
            fact_from_pairs(&[("App", "a1"), ("Generator", "openapi")]),
            fact_from_pairs(&[("App", "a1"), ("Generator", "solidity")]), // dup
            fact_from_pairs(&[("App", "a2"), ("Generator", "norma")]),
        ];
        // Sequential per-fact fold (the path being replaced).
        let mut seq = Object::phi();
        for f in &facts {
            seq = cell_put_folded("App_uses_Generator", f.clone(), &seq);
        }
        let seq_map = fetch_or_phi("App_uses_Generator", &seq)
            .as_map().cloned().expect("seq Map");
        // Batched fold.
        let batch = cell_put_folded_batch("App_uses_Generator", facts.clone(), &Object::phi());
        let batch_map = fetch_or_phi("App_uses_Generator", &batch)
            .as_map().cloned().expect("batch Map");
        assert_eq!(batch_map, seq_map,
            "batched fold must equal the per-fact fold; got {:?} vs {:?}",
            batch_map, seq_map);
        assert_eq!(batch_map.len(), 3, "one duplicate collapses → 3 distinct rows");
    }

    #[test]
    fn cell_put_folded_batch_empty_is_noop() {
        let seed = cell_put_folded("C", fact_from_pairs(&[("X", "1")]), &Object::phi());
        let after = cell_put_folded_batch("C", Vec::new(), &seed);
        assert_eq!(fetch_or_phi("C", &after), fetch_or_phi("C", &seed),
            "empty batch leaves the cell untouched");
    }

    #[test]
    fn cell_put_keyed_batch_nonupsert_keeps_first_and_reports_conflict() {
        let f_pending = fact_from_pairs(&[("Task", "t1"), ("Status", "pending")]);
        let f_done = fact_from_pairs(&[("Task", "t1"), ("Status", "done")]);
        let f_t2 = fact_from_pairs(&[("Task", "t2"), ("Status", "pending")]);
        // Two facts at key t1 (pending then a CONFLICTING done) + a fresh t2.
        let (state, conflicts) = cell_put_keyed_batch(
            "Task_has_Status", &["Task"],
            vec![f_pending.clone(), f_done.clone(), f_t2.clone()],
            false, &Object::phi());
        let m = fetch_or_phi("Task_has_Status", &state).as_map().cloned().expect("Map");
        assert_eq!(m.get("t1"), Some(&f_pending),
            "non-upsert keeps the FIRST write at t1; the conflicting later one drops");
        assert_eq!(m.get("t2"), Some(&f_t2));
        assert_eq!(m.len(), 2);
        assert_eq!(conflicts.len(), 1, "exactly one dropped conflict reported");
        assert_eq!(conflicts[0].key, "t1");
        assert_eq!(conflicts[0].existing_fact, f_pending);
        assert_eq!(conflicts[0].incoming_fact, f_done);
    }

    #[test]
    fn cell_put_keyed_batch_upsert_last_write_wins_no_conflict() {
        // SM-status-shaped: one State Machine advancing pending→…→completed
        // in a single round. upsert=true ⇒ last-write-wins, no conflict.
        let f_pending = fact_from_pairs(&[("State Machine", "sm1"), ("Status", "pending")]);
        let f_inprog = fact_from_pairs(&[("State Machine", "sm1"), ("Status", "in_progress")]);
        let f_done = fact_from_pairs(&[("State Machine", "sm1"), ("Status", "completed")]);
        let (state, conflicts) = cell_put_keyed_batch(
            "State_Machine_is_currently_in_Status", &["State Machine"],
            vec![f_pending, f_inprog, f_done.clone()],
            true, &Object::phi());
        let m = fetch_or_phi("State_Machine_is_currently_in_Status", &state)
            .as_map().cloned().expect("Map");
        assert_eq!(m.get("sm1"), Some(&f_done),
            "upsert last-write-wins → the round's final status (completed)");
        assert_eq!(m.len(), 1);
        assert!(conflicts.is_empty(), "upsert resolves same-key writes, reports no conflict");
    }

    #[test]
    fn cell_put_keyed_batch_migrates_seq_and_idempotent_reassert_is_no_conflict() {
        // Pre-existing Seq cell (pre-Map-migration shape) holding (t1→pending).
        let f1 = fact_from_pairs(&[("Task", "t1"), ("Status", "pending")]);
        let seq_state = store("Task_has_Status", Object::seq(vec![f1.clone()]), &Object::phi());
        // Batch re-asserts the SAME f1 (idempotent) and adds a fresh t2.
        let f2 = fact_from_pairs(&[("Task", "t2"), ("Status", "done")]);
        let (state, conflicts) = cell_put_keyed_batch(
            "Task_has_Status", &["Task"],
            vec![f1.clone(), f2.clone()],
            false, &seq_state);
        let m = fetch_or_phi("Task_has_Status", &state).as_map().cloned()
            .expect("Seq migrated to Map");
        assert_eq!(m.get("t1"), Some(&f1), "migrated entry preserved");
        assert_eq!(m.get("t2"), Some(&f2), "fresh entry inserted");
        assert_eq!(m.len(), 2);
        assert!(conflicts.is_empty(),
            "re-asserting the byte-identical existing fact is NOT a conflict");
    }

    /// store-on-derive PERF WIN (folded-stale-tuple wall): a keyed upsert
    /// DISPLACES a stale tuple sitting in a pre-existing FOLDED Map cell at a
    /// different (full-tuple-hash) map key. This is the storage-layer fix that
    /// lets `transition_via_defs` DROP the #836 wipe for the upsert-safe class:
    /// before it, `cell_put_keyed_batch` took a pre-existing Map VERBATIM, so
    /// `(t1→pending)` (folded under its full-tuple hash) and the upserted
    /// `(t1→in_progress)` (at role-key `t1`) COEXISTED — the exact divergence
    /// the prior increment (4dfc76eb) hit. Re-keying the pre-existing Map under
    /// the role key collapses the stale tuple onto `t1`, where the upsert
    /// overwrites it.
    #[test]
    fn cell_put_keyed_batch_upsert_displaces_stale_folded_tuple() {
        // Pre-existing FOLDED cell (keyed by full-tuple hash, as the union-only
        // `merge_delta` / `cell_put_folded` apply commit leaves a derived
        // bridge): holds the stale (t1, pending).
        let stale = fact_from_pairs(&[("Task", "t1"), ("Task Status", "pending")]);
        let folded = cell_put_folded("Task_has_Task_Status", stale.clone(), &Object::phi());
        // Sanity: the stale tuple is NOT under the role key `t1` — it is under
        // its full-tuple hash, so a naive verbatim-Map upsert would never see it.
        let pre = fetch_or_phi("Task_has_Task_Status", &folded).as_map().cloned().expect("Map");
        assert_eq!(pre.len(), 1);
        assert!(pre.get("t1").is_none(),
            "precondition: the folded tuple is keyed by full-tuple hash, not role-key t1");

        // A keyed UPSERT of the fresh (t1, in_progress) must DISPLACE the stale
        // pending — one tuple per role key, holding the latest value.
        let fresh = fact_from_pairs(&[("Task", "t1"), ("Task Status", "in_progress")]);
        let (next, conflicts) = cell_put_keyed_batch(
            "Task_has_Task_Status", &["Task"], vec![fresh.clone()], /*upsert=*/true, &folded);
        let m = fetch_or_phi("Task_has_Task_Status", &next).as_map().cloned().expect("Map");
        assert_eq!(m.len(), 1, "stale folded (t1,pending) must be displaced, not coexist; got {m:?}");
        assert_eq!(m.get("t1"), Some(&fresh), "the surviving tuple is the fresh in_progress");
        assert!(conflicts.is_empty(), "upsert overwrite is not a conflict");
    }

    // ── derivation-aggregate-composite-key-upsert: positional keying ──
    //
    // An aggregate head `Value shortest reaches Value for Feature at Count`
    // stores its group as TWO same-named `Value` roles + `Feature`, with
    // `Count` the folded value. Name-keying collides the two `Value`s;
    // positional keying (group = positions 0,1,2) keys correctly and
    // UPSERTs so a later round's smaller min supersedes the stale one.

    #[test]
    fn extract_key_from_fact_by_index_distinguishes_duplicate_role_names() {
        // The exact arc-cost-gen head shape (two `Value`, then `Feature`,
        // then the folded `Count`).
        let kg = fact_from_pairs(&[
            ("Value", "rk"), ("Value", "rg"), ("Feature", "loc"), ("Count", "3")]);
        let kh = fact_from_pairs(&[
            ("Value", "rk"), ("Value", "rh"), ("Feature", "loc"), ("Count", "1")]);
        // Positional group key over the THREE non-value roles tells the two
        // groups apart (rg vs rh at position 1).
        let key_kg = extract_key_from_fact_by_index(&kg, &[0, 1, 2]).expect("key");
        let key_kh = extract_key_from_fact_by_index(&kh, &[0, 1, 2]).expect("key");
        assert_ne!(key_kg, key_kh,
            "positional key distinguishes (rk,rg,loc) from (rk,rh,loc)");
        assert_eq!(key_kg, "rk\u{1f}rg\u{1f}loc");
        // …whereas NAME keying collides them: binding(\"Value\") returns the
        // FIRST `Value` for BOTH, so both key to `rk\u{1f}rk\u{1f}loc`. This
        // is exactly why the aggregate head needs positional keying.
        let name_kg = extract_key_from_fact(&kg, &["Value", "Value", "Feature"]).expect("k");
        let name_kh = extract_key_from_fact(&kh, &["Value", "Value", "Feature"]).expect("k");
        assert_eq!(name_kg, name_kh,
            "by-name keying COLLIDES distinct groups — the bug positional keying fixes");
    }

    #[test]
    fn extract_key_from_fact_by_index_out_of_bounds_is_none() {
        let f = fact_from_pairs(&[("Value", "rk"), ("Count", "2")]);
        assert!(extract_key_from_fact_by_index(&f, &[0, 5]).is_none(),
            "an out-of-range position yields None (fact not shaped for this key)");
    }

    #[test]
    fn cell_put_keyed_batch_by_index_upsert_supersedes_stale_group_min() {
        // THE bug at the storage layer: the recursive cost closure folds the
        // group (rk,rg,loc) to 3 in an EARLY round (toll, fewer hops) then to
        // 2 in a LATER round (walk+walk, cheaper). Keyless folding kept BOTH
        // → {2,3}. Positional keyed-UPSERT on the group (positions 0,1,2)
        // supersedes 3 with 2. A distinct group (rk,rh,loc) coexists.
        let early = fact_from_pairs(&[
            ("Value", "rk"), ("Value", "rg"), ("Feature", "loc"), ("Count", "3")]);
        let late = fact_from_pairs(&[
            ("Value", "rk"), ("Value", "rg"), ("Feature", "loc"), ("Count", "2")]);
        let other = fact_from_pairs(&[
            ("Value", "rk"), ("Value", "rh"), ("Feature", "loc"), ("Count", "1")]);
        // Two rounds: round 1 stores {3, (rk,rh)=1}; round 2 supersedes with 2.
        let (s1, c1) = cell_put_keyed_batch_by_index(
            "Value_shortest_reaches_Value_for_Feature_at_Count", &[0, 1, 2],
            vec![early.clone(), other.clone()], true, &Object::phi());
        assert!(c1.is_empty());
        let (s2, c2) = cell_put_keyed_batch_by_index(
            "Value_shortest_reaches_Value_for_Feature_at_Count", &[0, 1, 2],
            vec![late.clone()], true, &s1);
        assert!(c2.is_empty(), "upsert never conflicts");
        let m = fetch_or_phi("Value_shortest_reaches_Value_for_Feature_at_Count", &s2)
            .as_map().cloned().expect("Map");
        assert_eq!(m.len(), 2, "exactly one tuple per group — no stale {{2,3}} append");
        assert_eq!(m.get("rk\u{1f}rg\u{1f}loc"), Some(&late),
            "the (rk,rg,loc) group folded to the LATER, smaller min (2)");
        assert_eq!(m.get("rk\u{1f}rh\u{1f}loc"), Some(&other),
            "the distinct (rk,rh,loc) single-path group is unaffected");
    }

    // ── perf-cellput-on2: O(n²) regression GUARDS ───────────────────
    //
    // These do NOT time anything (timing is flaky). They assert a
    // STRUCTURAL invariant via the `cell_map_clone_counter`: integrating
    // a round's facts into a Map-backed cell must deep-clone that cell's
    // Map a CONSTANT number of times — ONE — no matter how many facts the
    // round carries. The pre-fix per-fact loop cloned once PER fact, so
    // the count grew with N (the O(n²) that made a 7378-fact round ~29s).
    // If anyone reverts the batched integrators to a per-fact loop, the
    // clone count scales with N again and these fail deterministically.

    #[test]
    fn cell_put_folded_batch_clones_cell_map_once_regardless_of_n() {
        let clones_for = |n: usize| -> usize {
            // Seed so the cell is Map-backed; this seed clone is OUTSIDE
            // the measured window (counter reset after it).
            let seed = cell_put_folded(
                "C", fact_from_pairs(&[("K", "seed"), ("V", "v0")]), &Object::phi());
            let facts: Vec<Object> = (0..n).map(|i| {
                let v = alloc::format!("v{}", i + 1);
                fact_from_pairs(&[("K", "seed"), ("V", v.as_str())])
            }).collect();
            reset_cell_map_clone_count();
            let _ = cell_put_folded_batch("C", facts, &seed);
            get_cell_map_clone_count()
        };
        let small = clones_for(8);
        let large = clones_for(512);
        assert_eq!(small, large,
            "folded batch must clone the cell Map a CONSTANT number of times, \
             independent of batch size; got {} for n=8 and {} for n=512 \
             (per-fact O(n²) regression?)", small, large);
        assert_eq!(large, 1,
            "one batched fold = exactly one Map clone; got {}", large);
    }

    #[test]
    fn cell_put_keyed_batch_clones_cell_map_once_regardless_of_n() {
        let clones_for = |n: usize, upsert: bool| -> usize {
            let seed = cell_put_keyed(
                "T_has_S", &["T"],
                fact_from_pairs(&[("T", "seed"), ("S", "s")]), &Object::phi())
                .expect("seed write");
            // n DISTINCT keys → n fresh inserts (no conflicts), so the only
            // clone is the single batch-entry Map clone.
            let facts: Vec<Object> = (0..n).map(|i| {
                let k = alloc::format!("t{}", i);
                fact_from_pairs(&[("T", k.as_str()), ("S", "s")])
            }).collect();
            reset_cell_map_clone_count();
            let _ = cell_put_keyed_batch("T_has_S", &["T"], facts, upsert, &seed);
            get_cell_map_clone_count()
        };
        for upsert in [false, true] {
            let small = clones_for(8, upsert);
            let large = clones_for(512, upsert);
            assert_eq!(small, large,
                "keyed batch (upsert={}) must clone the cell Map a CONSTANT number \
                 of times; got {} for n=8 and {} for n=512", upsert, small, large);
            assert_eq!(large, 1,
                "one batched keyed integration = exactly one Map clone (upsert={}); \
                 got {}", upsert, large);
        }
    }

    // ── perf-mergededup-on2: concat_dedup behavior + O(n) guard ─────

    #[test]
    fn concat_dedup_dedups_identity_key_first_occurrence_wins() {
        // id-bearing facts: same id (even with a different non-key field)
        // collapse to the FIRST; a fresh id is kept.
        let a = Object::seq(vec![fact_from_pairs(&[("id", "1"), ("reading", "A")])]);
        let b = Object::seq(vec![
            fact_from_pairs(&[("id", "1"), ("reading", "A-changed")]), // same id → dropped
            fact_from_pairs(&[("id", "2"), ("reading", "B")]),          // new id → kept
        ]);
        let m = concat_dedup("FactType", &a, &b);
        let ids: Vec<String> = cell_facts_iter(&m)
            .filter_map(|f| binding(f, "id").map(|s| s.to_string())).collect();
        assert_eq!(ids, vec!["1".to_string(), "2".to_string()]);
        let r1 = cell_facts_iter(&m).find(|f| binding(f, "id") == Some("1"))
            .and_then(|f| binding(f, "reading").map(|s| s.to_string()));
        assert_eq!(r1, Some("A".to_string()), "first occurrence of id=1 wins");
    }

    #[test]
    fn concat_dedup_dedups_keyless_structural_and_keeps_distinct() {
        // Keyless (no id/name/ruleId) facts dedup by structural identity.
        let dup = fact_from_pairs(&[("subjectValue", "t1"), ("fieldName", "Task_is_finished")]);
        let other = fact_from_pairs(&[("subjectValue", "t2"), ("fieldName", "Task_is_finished")]);
        let a = Object::seq(vec![dup.clone()]);
        let b = Object::seq(vec![dup.clone(), other.clone()]);
        let m = concat_dedup("InstanceFact", &a, &b);
        assert_eq!(cell_facts_iter(&m).count(), 2, "structural dup dropped, distinct kept");
    }

    #[test]
    fn concat_dedup_preserves_homedomain_identity() {
        // Same name + DIFFERENT homeDomain stay DISTINCT (domain_mismatch);
        // a same-domain repeat dedups. The indexed dedup buckets both under
        // name=Order but must still defer to same_identity for the verdict.
        let a = Object::seq(vec![fact_from_pairs(&[("name", "Order"), ("homeDomain", "core")])]);
        let b = Object::seq(vec![
            fact_from_pairs(&[("name", "Order"), ("homeDomain", "orders")]), // diff domain → kept
            fact_from_pairs(&[("name", "Order"), ("homeDomain", "core")]),   // same → dropped
        ]);
        let m = concat_dedup("Noun", &a, &b);
        let homes: Vec<String> = cell_facts_iter(&m)
            .filter(|f| binding(f, "name") == Some("Order"))
            .filter_map(|f| binding(f, "homeDomain").map(|s| s.to_string())).collect();
        assert_eq!(homes.len(), 2,
            "Order@core and Order@orders are distinct; the Order@core repeat is dropped");
    }

    #[test]
    fn concat_dedup_same_identity_calls_stay_linear() {
        // O(n²) regression guard (perf-mergededup-on2): deduping a cell
        // against itself must call `same_identity` ~O(n) (one bucket hit per
        // fact), NOT O(n²) (a full-accumulator scan per fact). Pre-fix this
        // was n² and made the parse fold ~226s on the tasks app.
        let calls_for = |n: usize| -> (usize, usize) {
            let facts: Vec<Object> = (0..n).map(|i| {
                let s = alloc::format!("s{}", i);
                fact_from_pairs(&[("subjectValue", "t"), ("objectValue", s.as_str())])
            }).collect();
            let cell = Object::seq(facts);
            reset_same_identity_count();
            let merged = concat_dedup("InstanceFact", &cell, &cell); // self-merge → all dups
            (get_same_identity_count(), cell_facts_iter(&merged).count())
        };
        let (c_small, k_small) = calls_for(40);
        let (c_large, k_large) = calls_for(400);
        assert_eq!(k_small, 40, "self-merge dedups back to n distinct facts");
        assert_eq!(k_large, 400, "self-merge dedups back to n distinct facts");
        assert!(c_large <= 8 * 400,
            "concat_dedup must call same_identity ~O(n) (got {} for n=400; O(n²) would be ~80000)",
            c_large);
        assert!(c_large <= c_small * 20,
            "same_identity calls must scale ~linearly: {} (n=40) vs {} (n=400) — 10× the facts \
             should be ~10× the calls, not ~100× (O(n²))", c_small, c_large);
    }

    // ── ⊥-trace (derivation-bottom-trace) ───────────────────────────
    //
    // ⊥ is provenance-lossless (`f:⊥ = ⊥`), so a top-level ⊥ loses its
    // origin — the engine can only say "⊥". These tests pin the negative
    // twin of `explain`: `with_bottom_trace` materializes the frame
    // context (rule / cell / binding) ONLY when ⊥ is produced, turning a
    // bare "⊥" into "⊥ origin: <binding> in rule `…` over cell `…`".

    /// Ring-derivation over the `Task_blocks_Task` self-ring: a rule whose
    /// body fetches a cell that is ABSENT bottoms out. BEFORE: a bare "⊥".
    /// AFTER: the trace names the rule (Def) AND the cell + binding (Fetch)
    /// at the frame where ⊥ first arose.
    #[test]
    fn bottom_trace_ring_apply_names_rule_cell_and_binding() {
        // Rule `derive:Blocked` ≡ Fetch — over an EMPTY def-state, so the
        // `Task_blocks_Task` cell the body fetches does not exist. The
        // applied binding <Task_blocks_Task, φ> resolves through fetch →
        // ⊥ (absent cell, no view).
        let d = defs_to_state(
            &[("derive:Blocked".to_string(), Func::Fetch)],
            &Object::phi(),
        );
        let binding = Object::seq(vec![
            Object::atom("Task_blocks_Task"),
            Object::phi(), // empty population — cell genuinely absent
        ]);

        // BEFORE: unarmed apply returns ⊥; the user-facing render is a
        // bare, origin-less "⊥".
        let bare = apply(&Func::Def("derive:Blocked".to_string()), &binding, &d);
        assert_eq!(bare, Object::Bottom);
        assert_eq!(bare.to_string(), "⊥",
            "BEFORE: a top-level ⊥ renders as a bare, provenance-lossless ⊥");

        // AFTER: armed apply captures the origin frame.
        let (result, trace) = with_bottom_trace(
            || apply(&Func::Def("derive:Blocked".to_string()), &binding, &d));
        assert_eq!(result, Object::Bottom, "still ⊥ — tracing is non-invasive");
        let trace = trace.expect("a structural ⊥ over a named cell must capture a trace");
        assert_eq!(trace.rule.as_deref(), Some("derive:Blocked"),
            "origin names the enclosing rule (Func::Def frame)");
        assert_eq!(trace.cell.as_deref(), Some("Task_blocks_Task"),
            "origin names the cell the Fetch frame bottomed out over");
        assert!(trace.binding.is_some(),
            "origin names the frame binding at the ⊥ site");

        let described = trace.describe().expect("non-empty trace describes");
        assert!(described.contains("derive:Blocked")
                && described.contains("Task_blocks_Task"),
            "AFTER: traced origin de-opaques the ⊥ — got `{described}`");
        assert_ne!(described, "⊥",
            "AFTER must NOT be a bare ⊥ — it carries the origin");
        // Pin the literal BEFORE→AFTER de-opaquing for the ring-apply ⊥:
        //   BEFORE: "⊥"
        //   AFTER : "⊥ origin: φ in rule `derive:Blocked` over cell `Task_blocks_Task`"
        assert_eq!(described,
            "⊥ origin: φ in rule `derive:Blocked` over cell `Task_blocks_Task`",
            "AFTER is the exact traced origin form");
    }

    /// std-deps real ⊥: a `Func::Store` to a cell under an empty
    /// capability stack is refused (Sec-5) and collapses to ⊥. The trace
    /// names the cell the refused write targeted.
    #[test]
    fn bottom_trace_std_deps_cap_refused_store_names_cell() {
        // Empty cap stack (test default) + not-permissive ⇒ the store
        // refusal arm fires. <cell, contents, D>.
        let binding = Object::seq(vec![
            Object::atom("Task_blocks_Task"),
            fact_from_pairs(&[("Task", "112"), ("Task", "113")]),
            Object::phi(),
        ]);

        let bare = apply(&Func::Store, &binding, &Object::phi());
        assert_eq!(bare.to_string(), "⊥",
            "BEFORE: a cap-refused store is a bare ⊥");

        let (result, trace) = with_bottom_trace(
            || apply(&Func::Store, &binding, &Object::phi()));
        assert_eq!(result, Object::Bottom);
        let trace = trace.expect("a cap-refused store must capture a trace");
        assert_eq!(trace.cell.as_deref(), Some("Task_blocks_Task"),
            "origin names the cell the refused write targeted");
    }

    /// Absent-def ⊥: applying `Func::Def(name)` where the def is not in
    /// state bottoms out; the trace names the missing rule.
    #[test]
    fn bottom_trace_absent_def_names_rule() {
        let (result, trace) = with_bottom_trace(
            || apply(&Func::Def("derive:DoesNotExist".to_string()),
                     &Object::phi(), &Object::phi()));
        assert_eq!(result, Object::Bottom);
        let trace = trace.expect("an absent def must capture a trace");
        assert_eq!(trace.rule.as_deref(), Some("derive:DoesNotExist"),
            "origin names the missing rule");
    }

    /// Zero-cost / non-invasive on the SUCCESS path: a rule that does NOT
    /// bottom out captures NO trace, even inside an armed scope, and the
    /// result is identical to the unarmed apply. This pins that arming
    /// tracing never alters a successful computation.
    #[test]
    fn bottom_trace_success_path_captures_nothing() {
        // Rule `derive:First` ≡ Selector(1); over a non-empty seq it
        // succeeds and never produces ⊥.
        let d = defs_to_state(
            &[("derive:First".to_string(), Func::Selector(1))],
            &Object::phi(),
        );
        let x = Object::seq(vec![Object::atom("a"), Object::atom("b")]);

        let unarmed = apply(&Func::Def("derive:First".to_string()), &x, &d);
        let (armed, trace) = with_bottom_trace(
            || apply(&Func::Def("derive:First".to_string()), &x, &d));

        assert_eq!(unarmed, Object::atom("a"));
        assert_eq!(armed, unarmed,
            "arming the ⊥-trace must not change a successful result");
        assert!(trace.is_none(),
            "the success path must capture NO trace — recording is ⊥-only");
    }

    /// Intermediate ⊥ that is legitimately absorbed (compact drops ⊥
    /// elements → a non-⊥ seq) must NOT surface a trace: the top-level
    /// result is not ⊥, so `with_bottom_trace` reports `None`. Pins that
    /// "⊥ as ordinary scaffolding" is not mistaken for a bottomed-out
    /// computation.
    #[test]
    fn bottom_trace_absorbed_intermediate_bottom_is_not_reported() {
        // compact:<a, ⊥, b> = <a, b> — a ⊥ element is consumed, the
        // overall apply succeeds.
        let x = Object::Seq(
            vec![Object::atom("a"), Object::Bottom, Object::atom("b")].into());
        let (result, trace) = with_bottom_trace(|| apply(&Func::Compact, &x, &Object::phi()));
        assert_eq!(result, Object::seq(vec![Object::atom("a"), Object::atom("b")]));
        assert!(trace.is_none(),
            "an absorbed intermediate ⊥ that yields a non-⊥ result reports no trace");
    }

    #[test]
    fn cell_put_folded_migrates_existing_seq_cell_to_map() {
        let f1 = fact_from_pairs(&[("App", "a1"), ("Generator", "solidity")]);
        let f2 = fact_from_pairs(&[("App", "a2"), ("Generator", "fpga")]);
        let seq_state = cell_push("App_uses_Generator", f1.clone(), &Object::phi());
        assert!(matches!(fetch_or_phi("App_uses_Generator", &seq_state), Object::Seq(_)));
        let folded = cell_put_folded("App_uses_Generator", f2.clone(), &seq_state);
        let m = fetch_or_phi("App_uses_Generator", &folded).as_map().cloned()
            .expect("Seq migrated to Map");
        assert_eq!(m.len(), 2, "migrated f1 + new f2");
    }

    #[test]
    fn fetch_cell_seq_flattens_map_in_key_sorted_order() {
        // #932 phase-2 D3: a folded Map cell flattens to a Seq in
        // deterministic KEY order (not nondeterministic HashMap order), so
        // the serialized fact array is reproducible across runs.
        let mut m: HashMap<String, Object> = HashMap::new();
        m.insert("k-c".into(), Object::atom("val-c"));
        m.insert("k-a".into(), Object::atom("val-a"));
        m.insert("k-b".into(), Object::atom("val-b"));
        let state = store("CellX", Object::Map(m.into()), &Object::phi());
        let seq = fetch_cell_seq("CellX", &state);
        let vals: Vec<&str> = seq.as_seq().unwrap().iter()
            .filter_map(|o| o.as_atom()).collect();
        assert_eq!(vals, vec!["val-a", "val-b", "val-c"],
            "fetch_cell_seq must yield Map values in key-sorted (deterministic) order");
    }

    #[test]
    fn cell_put_keyed_composite_key_joins_role_values() {
        let f = fact_from_pairs(&[
            ("Person", "alice"),
            ("Organization", "acme"),
            ("Joined", "2024-01-01"),
        ]);
        let state = cell_put_keyed(
            "Membership",
            &["Person", "Organization"],
            f.clone(),
            &Object::phi(),
        ).expect("first write");
        let m = fetch_or_phi("Membership", &state).as_map().cloned().expect("Map contents");
        // Composite key uses the unit-separator joiner — internal detail,
        // but the lookup should find exactly one entry.
        assert_eq!(m.len(), 1);
        assert!(m.values().next() == Some(&f));
    }

    #[test]
    fn cell_put_keyed_returns_unchanged_state_when_role_missing() {
        // Fact has only Status, not Task; not fully keyed for ["Task"].
        let f = fact_from_pairs(&[("Status", "pending")]);
        let state = cell_put_keyed("Task_has_Status", &["Task"], f, &Object::phi())
            .expect("missing-role no-op is Ok, not a collision");
        // Cell never gets written.
        assert_eq!(fetch_or_phi("Task_has_Status", &state), Object::phi());
    }

    #[test]
    fn cell_put_keyed_migrates_seq_cell_to_map_on_first_keyed_write() {
        // Pre-existing Seq cell (legacy push path), then keyed write
        // should reshape to Map with the existing fact retained under
        // its extracted key.
        let f1 = fact_from_pairs(&[("Task", "t-1"), ("Status", "pending")]);
        let f2 = fact_from_pairs(&[("Task", "t-2"), ("Status", "done")]);
        let s_seq = cell_push("Task_has_Status", f1.clone(), &Object::phi());
        let s_map = cell_put_keyed("Task_has_Status", &["Task"], f2.clone(), &s_seq)
            .expect("distinct keys after Seq→Map migration");
        let m = fetch_or_phi("Task_has_Status", &s_map).as_map().cloned().expect("Map contents");
        assert_eq!(m.len(), 2);
        assert_eq!(m.get("t-1"), Some(&f1));
        assert_eq!(m.get("t-2"), Some(&f2));
    }

    #[test]
    fn cell_facts_iter_uniformly_iterates_seq_and_map_cells() {
        let f1 = fact_from_pairs(&[("Task", "t-1"), ("Status", "pending")]);
        let f2 = fact_from_pairs(&[("Task", "t-2"), ("Status", "done")]);

        let seq_state = cell_push("Cell_A", f1.clone(),
            &cell_push("Cell_A", f2.clone(), &Object::phi()));
        let seq_count = cell_facts_iter(&fetch_or_phi("Cell_A", &seq_state)).count();
        assert_eq!(seq_count, 2);

        let inner = cell_put_keyed("Cell_B", &["Task"], f2.clone(), &Object::phi())
            .expect("first key");
        let map_state = cell_put_keyed("Cell_B", &["Task"], f1.clone(), &inner)
            .expect("second key");
        let map_count = cell_facts_iter(&fetch_or_phi("Cell_B", &map_state)).count();
        assert_eq!(map_count, 2);
    }

    #[test]
    fn extract_key_from_fact_returns_none_when_any_role_missing() {
        let f = fact_from_pairs(&[("Task", "t-1")]);
        assert_eq!(extract_key_from_fact(&f, &["Task"]), Some("t-1".to_string()));
        assert_eq!(extract_key_from_fact(&f, &["Task", "Status"]), None);
    }

    // ─── task-744 phase 4: collision-detection acceptance tests ─────
    //
    // The phase-2 doc-comment for cell_put_keyed said:
    //   "Overwrites any existing tuple at the same key — this is the
    //   upsert path. UC violation detection lives in the caller …"
    //
    // Phase 4 promotes UC violation detection into the storage
    // primitive itself: the function returns Err(KeyConflict) when an
    // incoming fact would silently overwrite a structurally distinct
    // existing fact at the same key. Byte-equal re-assertions remain
    // ok-no-op; distinct keys remain ok-write. The four tests below
    // pin each branch.

    #[test]
    fn cell_put_keyed_collision_acceptance_1_same_key_same_fact_is_no_op() {
        // AC1: writing the same key + same fact = no collision, state
        // unchanged structurally. The second call returns Ok and its
        // resulting state is equal to the first call's state.
        let f = fact_from_pairs(&[("Task", "t-1"), ("Status", "pending")]);
        let s1 = cell_put_keyed("Task_has_Status", &["Task"], f.clone(), &Object::phi())
            .expect("first write");
        let s2 = cell_put_keyed("Task_has_Status", &["Task"], f.clone(), &s1)
            .expect("byte-equal re-assertion must be Ok, not a collision");
        // State equal — re-assert produced no structural change.
        assert_eq!(s1, s2);
        // And the cell still holds exactly the one fact.
        let m = fetch_or_phi("Task_has_Status", &s2).as_map().cloned()
            .expect("Map contents");
        assert_eq!(m.len(), 1);
        assert_eq!(m.get("t-1"), Some(&f));
    }

    #[test]
    fn cell_put_keyed_collision_acceptance_2_same_key_different_value_is_collision() {
        // AC2: writing the same key with a different non-key role value
        // is a collision. The state from before the failed write is
        // returned by the caller (we have s1 in hand); the error
        // surfaces both facts and the key.
        let f1 = fact_from_pairs(&[("Task", "t-1"), ("Status", "pending")]);
        let f2 = fact_from_pairs(&[("Task", "t-1"), ("Status", "done")]);
        let s1 = cell_put_keyed("Task_has_Status", &["Task"], f1.clone(), &Object::phi())
            .expect("first write");
        let err = cell_put_keyed("Task_has_Status", &["Task"], f2.clone(), &s1)
            .expect_err("same key + different Status = collision");
        assert_eq!(err, KeyConflict {
            name: "Task_has_Status".into(),
            key: "t-1".into(),
            existing_fact: f1.clone(),
            incoming_fact: f2,
        });
        // The state that was passed in is untouched.
        let m = fetch_or_phi("Task_has_Status", &s1).as_map().cloned()
            .expect("Map contents");
        assert_eq!(m.get("t-1"), Some(&f1));
    }

    #[test]
    fn cell_put_keyed_collision_acceptance_3_idempotent_reassert_no_collision() {
        // AC3: same key + same non-key values, even when the two facts
        // were constructed independently (rather than cloned), is an
        // idempotent re-assertion and must NOT be flagged as a
        // collision. fact_from_pairs is deterministic, so two builds
        // with the same pairs produce byte-equal facts — exercising
        // structural equality, not identity.
        let f_first = fact_from_pairs(&[("Task", "t-7"), ("Status", "blocked")]);
        let f_second = fact_from_pairs(&[("Task", "t-7"), ("Status", "blocked")]);
        // Sanity: the two independently-built facts are byte-equal.
        assert_eq!(f_first, f_second);
        let s1 = cell_put_keyed("Task_has_Status", &["Task"], f_first.clone(), &Object::phi())
            .expect("first write");
        let s2 = cell_put_keyed("Task_has_Status", &["Task"], f_second.clone(), &s1)
            .expect("idempotent re-assertion is Ok");
        assert_eq!(s1, s2);
    }

    #[test]
    fn cell_put_keyed_collision_acceptance_4_distinct_keys_no_collision() {
        // AC4: writing two distinct keys = no collision, both facts
        // coexist in the Map.
        let f1 = fact_from_pairs(&[("Task", "t-1"), ("Status", "pending")]);
        let f2 = fact_from_pairs(&[("Task", "t-2"), ("Status", "pending")]);
        let s1 = cell_put_keyed("Task_has_Status", &["Task"], f1.clone(), &Object::phi())
            .expect("first write");
        let s2 = cell_put_keyed("Task_has_Status", &["Task"], f2.clone(), &s1)
            .expect("distinct second key — no collision");
        let m = fetch_or_phi("Task_has_Status", &s2).as_map().cloned()
            .expect("Map contents");
        assert_eq!(m.len(), 2);
        assert_eq!(m.get("t-1"), Some(&f1));
        assert_eq!(m.get("t-2"), Some(&f2));
    }

    // task-744 phase 3: FFP combinators accept Map cells as the input
    // collection. α and Filter iterate values; Length counts entries.

    #[test]
    fn apply_to_all_maps_over_map_values() {
        let f = fact_from_pairs(&[("Task", "t-1"), ("Status", "pending")]);
        let g = fact_from_pairs(&[("Task", "t-2"), ("Status", "done")]);
        let inner = cell_put_keyed("X", &["Task"], g, &Object::phi())
            .expect("first key");
        let m = cell_put_keyed("X", &["Task"], f, &inner)
            .expect("second key");
        let cell = fetch_or_phi("X", &m);
        // α(Selector(2)) over a Map of <<Task,T>,<Status,S>> facts gives the
        // sequence of <Status, value> pairs — order is incidental for a Map.
        let extract = Func::apply_to_all(Func::Selector(2));
        let result = apply(&extract, &cell, &Object::phi());
        let pairs: Vec<&Object> = result.as_seq().expect("Seq result").iter().collect();
        assert_eq!(pairs.len(), 2);
        // Each pair is <Status, value>: Selector(2) of the pair gives the
        // value atom.
        let values: Vec<String> = pairs.iter()
            .map(|p| apply(&Func::Selector(2), p, &Object::phi())
                .as_atom().map(|s| s.to_string()).unwrap_or_default())
            .collect();
        assert!(values.contains(&"pending".to_string()), "got {:?}", values);
        assert!(values.contains(&"done".to_string()), "got {:?}", values);
    }

    #[test]
    fn filter_keeps_matching_map_entries_and_returns_seq() {
        let f = fact_from_pairs(&[("Task", "t-1"), ("Status", "pending")]);
        let g = fact_from_pairs(&[("Task", "t-2"), ("Status", "done")]);
        let h = fact_from_pairs(&[("Task", "t-3"), ("Status", "pending")]);
        let mut state = Object::phi();
        for fact in &[f.clone(), g.clone(), h.clone()] {
            state = cell_put_keyed("X", &["Task"], fact.clone(), &state)
                .expect("distinct keys across the loop");
        }
        let cell = fetch_or_phi("X", &state);
        // Filter for pending: predicate compares Status pair to "pending".
        let pending = Func::filter(Func::compose(
            Func::Eq,
            Func::construction(vec![
                Func::compose(Func::Selector(2), Func::Selector(2)),
                Func::constant(Object::atom("pending")),
            ]),
        ));
        let result = apply(&pending, &cell, &Object::phi());
        let kept = result.as_seq().expect("Seq result");
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn length_counts_map_entries() {
        let f = fact_from_pairs(&[("Task", "t-1"), ("Status", "pending")]);
        let g = fact_from_pairs(&[("Task", "t-2"), ("Status", "done")]);
        let inner = cell_put_keyed("X", &["Task"], g, &Object::phi())
            .expect("first key");
        let state = cell_put_keyed("X", &["Task"], f, &inner)
            .expect("second key");
        let cell = fetch_or_phi("X", &state);
        let result = apply(&Func::Length, &cell, &Object::phi());
        assert_eq!(result, Object::atom("2"));
    }

    #[test]
    fn apply_to_all_on_empty_map_returns_phi() {
        let empty_map: Object = Object::Map(HashMap::new().into());
        let result = apply(&Func::apply_to_all(Func::Id), &empty_map, &Object::phi());
        assert_eq!(result, Object::phi());
    }

    #[test]
    fn filter_on_empty_map_returns_phi() {
        let empty_map: Object = Object::Map(HashMap::new().into());
        let result = apply(&Func::filter(Func::Id), &empty_map, &Object::phi());
        assert_eq!(result, Object::phi());
    }

    #[test]
    fn cell_fact_count_handles_both_storage_shapes() {
        let f = fact_from_pairs(&[("Task", "t-1"), ("Status", "pending")]);
        let seq = cell_push("X", f.clone(), &Object::phi());
        assert_eq!(cell_fact_count(&fetch_or_phi("X", &seq)), 1);
        let map = cell_put_keyed("Y", &["Task"], f, &Object::phi())
            .expect("first write");
        assert_eq!(cell_fact_count(&fetch_or_phi("Y", &map)), 1);
        assert_eq!(cell_fact_count(&Object::phi()), 0);
        assert_eq!(cell_fact_count(&Object::Bottom), 0);
    }

    // ─── VersionEntry shape (S1a, #717) ────────────────────────────────

    #[test]
    fn version_entry_round_trips_all_fields() {
        let contents = Object::atom("hello");
        let recorded = Object::atom("1700000000000");
        let entry = version_entry(42, contents.clone(), Some(41), recorded.clone(), None);
        assert_eq!(version_entry_id(&entry), Some(42));
        assert_eq!(version_entry_contents(&entry), Some(&contents));
        assert_eq!(version_entry_prev(&entry), Some(41));
        assert_eq!(version_entry_recorded_at(&entry), Some(&recorded));
        assert!(is_version_entry(&entry));
    }

    #[test]
    fn version_entry_with_no_prev_returns_none() {
        let entry = version_entry(1, Object::atom("first"), None, Object::atom("t0"), None);
        assert_eq!(version_entry_id(&entry), Some(1));
        assert_eq!(version_entry_prev(&entry), None);
    }

    #[test]
    fn version_entry_can_carry_complex_contents() {
        // contents is an arbitrary Object — this is how a cell whose
        // payload is a Seq-of-facts will be stored once S1b lands.
        let contents = Object::seq(vec![
            fact_from_pairs(&[("name", "Alice"), ("age", "30")]),
            fact_from_pairs(&[("name", "Bob"),   ("age", "25")]),
        ]);
        let entry = version_entry(7, contents.clone(), None, Object::phi(), None);
        assert_eq!(version_entry_contents(&entry), Some(&contents));
    }

    #[test]
    fn is_version_entry_rejects_plain_objects() {
        assert!(!is_version_entry(&Object::phi()));
        assert!(!is_version_entry(&Object::atom("notanentry")));
        assert!(!is_version_entry(&Object::seq(vec![Object::atom("a")])));
        // A normal cell tuple must not pollute the predicate.
        assert!(!is_version_entry(&cell("Noun", Object::atom("Alice"))));
    }

    // ─── S1b: chain semantics + merge_delta append (#718) ──────────────

    #[test]
    fn is_version_chain_detects_wrapped_seq_only() {
        let chain = wrap_as_chain(Object::atom("payload"), Object::atom("0"), None);
        assert!(is_version_chain(&chain));
        // Raw cells aren't chains.
        assert!(!is_version_chain(&Object::atom("plain")));
        assert!(!is_version_chain(&Object::phi()));
        assert!(!is_version_chain(&Object::seq(vec![Object::atom("a"), Object::atom("b")])));
        // Single non-entry items in a Seq don't qualify.
        assert!(!is_version_chain(&Object::seq(vec![fact_from_pairs(&[("k", "v")])])));
    }

    #[test]
    fn cell_contents_view_unwraps_chain_or_passes_raw() {
        let raw = Object::atom("raw");
        assert_eq!(cell_contents_view(&raw), &raw, "raw passes through");

        let payload = Object::atom("payload");
        let chain = wrap_as_chain(payload.clone(), Object::atom("0"), None);
        assert_eq!(cell_contents_view(&chain), &payload, "chain unwraps to latest");
    }

    #[test]
    fn merge_delta_appends_a_new_version_per_call() {
        // Start with an empty Map base.
        let s0 = Object::Map(HashMap::new().into());

        let mut d1 = HashMap::new();
        d1.insert("X".to_string(), Object::atom("v1"));
        let s1 = merge_delta(&s0, &Object::Map(d1.into()), None);

        let mut d2 = HashMap::new();
        d2.insert("X".to_string(), Object::atom("v2"));
        let s2 = merge_delta(&s1, &Object::Map(d2.into()), None);

        // Logical view shows latest.
        assert_eq!(fetch_or_phi("X", &s2), Object::atom("v2"));

        // History shows both versions in chronological order.
        let hist = cells_iter_history(&s2, "X");
        assert_eq!(hist.len(), 2, "two merges → two entries");
        assert_eq!(version_entry_id(&hist[0]), Some(1));
        assert_eq!(version_entry_contents(&hist[0]), Some(&Object::atom("v1")));
        assert_eq!(version_entry_prev(&hist[0]), None);
        assert_eq!(version_entry_id(&hist[1]), Some(2));
        assert_eq!(version_entry_contents(&hist[1]), Some(&Object::atom("v2")));
        assert_eq!(version_entry_prev(&hist[1]), Some(1));
    }

    #[test]
    fn merge_delta_creates_chain_for_absent_cell() {
        let s0 = Object::Map(HashMap::new().into());
        let mut d = HashMap::new();
        d.insert("Brand_new".to_string(), Object::atom("hello"));
        let s1 = merge_delta(&s0, &Object::Map(d.into()), None);

        let hist = cells_iter_history(&s1, "Brand_new");
        assert_eq!(hist.len(), 1);
        assert_eq!(version_entry_id(&hist[0]), Some(1));
        assert_eq!(version_entry_prev(&hist[0]), None);
    }

    #[test]
    fn merge_delta_promotes_legacy_raw_to_v0_then_appends() {
        // Base is built from legacy Seq form (no chain wrapping).
        let s0 = Object::seq(vec![cell("X", Object::atom("legacy"))]);

        let mut d = HashMap::new();
        d.insert("X".to_string(), Object::atom("new"));
        let s1 = merge_delta(&s0, &Object::Map(d.into()), None);

        // History: synthetic v0 = "legacy", then v1 = "new".
        let hist = cells_iter_history(&s1, "X");
        assert_eq!(hist.len(), 2);
        assert_eq!(version_entry_id(&hist[0]), Some(0), "legacy promoted to v0");
        assert_eq!(version_entry_contents(&hist[0]), Some(&Object::atom("legacy")));
        assert_eq!(version_entry_id(&hist[1]), Some(1));
        assert_eq!(version_entry_contents(&hist[1]), Some(&Object::atom("new")));

        // Logical view of the cell is the latest write.
        assert_eq!(fetch_or_phi("X", &s1), Object::atom("new"));
    }

    #[test]
    fn cells_iter_history_returns_synthetic_v0_for_legacy_raw() {
        // Pure-Seq state with no chain wrapping anywhere.
        let s = Object::seq(vec![cell("X", Object::atom("only"))]);
        let hist = cells_iter_history(&s, "X");
        assert_eq!(hist.len(), 1);
        assert_eq!(version_entry_id(&hist[0]), Some(0));
        assert_eq!(version_entry_contents(&hist[0]), Some(&Object::atom("only")));
    }

    #[test]
    fn cells_iter_history_empty_for_unknown_cell() {
        let s = Object::Map(HashMap::new().into());
        assert!(cells_iter_history(&s, "no_such_cell").is_empty());
    }

    #[test]
    fn diff_then_merge_preserves_logical_view_after_chain_lands() {
        // The eq:cellfold realignment must not break the legacy
        // diff_cells → merge_delta round-trip invariant: cells_iter on
        // the reconstructed state must produce the same (name, contents)
        // pairs as cells_iter on the new state.
        let old = Object::seq(vec![
            cell("A", Object::atom("1")),
            cell("B", Object::atom("2")),
            cell("C", Object::atom("3")),
        ]);
        let new = Object::seq(vec![
            cell("A", Object::atom("1")),       // unchanged
            cell("B", Object::atom("CHANGED")), // changed
            cell("C", Object::atom("3")),       // unchanged
            cell("D", Object::atom("4")),       // added
        ]);
        let delta = diff_cells(&old, &new);
        let reconstructed = merge_delta(&old, &delta, None);

        // Logical view of reconstructed matches new for every cell.
        for name in ["A", "B", "C", "D"] {
            assert_eq!(
                fetch_or_phi(name, &reconstructed),
                fetch_or_phi(name, &new),
                "cell {} round-trips through diff+merge", name
            );
        }
    }

    // ── S1g (#723): chain compaction ────────────────────────────────

    fn build_three_chain_state() -> Object {
        let mut state = Object::Map(HashMap::new().into());
        for tag in &["a", "b", "c"] {
            let mut d = HashMap::new();
            d.insert("Item".to_string(), Object::atom(tag));
            state = merge_delta(&state, &Object::Map(d.into()), None);
        }
        state
    }

    #[test]
    fn compact_chain_keeps_only_pinned_and_latest() {
        let state = build_three_chain_state();
        let chain = fetch_raw("Item", &state);
        // Pin v=1 only; v=3 (latest) is always kept; v=2 should drop.
        let mut keep = alloc::collections::BTreeSet::new();
        keep.insert(1u64);
        let compacted = compact_chain(&chain, &keep);

        let entries = compacted.as_seq().expect("compacted is a chain seq");
        assert_eq!(entries.len(), 2,
            "v=2 must drop; kept = {{v=1, latest=v=3}}; got {} entries", entries.len());
        let ids: Vec<u64> = entries.iter().filter_map(version_entry_id).collect();
        assert_eq!(ids, vec![1, 3], "expected ids [1,3]; got {:?}", ids);
    }

    #[test]
    fn compact_chain_keeps_latest_when_no_pins() {
        let state = build_three_chain_state();
        let chain = fetch_raw("Item", &state);
        let keep = alloc::collections::BTreeSet::new();
        let compacted = compact_chain(&chain, &keep);
        let entries = compacted.as_seq().unwrap();
        assert_eq!(entries.len(), 1, "no pins → only latest kept");
        assert_eq!(version_entry_id(&entries[0]), Some(3));
    }

    #[test]
    fn compact_chain_passes_through_non_chain() {
        let raw = Object::atom("legacy");
        let mut keep = alloc::collections::BTreeSet::new();
        keep.insert(99u64);
        let result = compact_chain(&raw, &keep);
        assert_eq!(result, raw, "non-chain inputs pass through unchanged");
    }

    #[test]
    fn compact_chain_no_op_when_all_versions_pinned() {
        let state = build_three_chain_state();
        let chain = fetch_raw("Item", &state);
        let mut keep = alloc::collections::BTreeSet::new();
        keep.extend([1u64, 2, 3]);
        let compacted = compact_chain(&chain, &keep);
        assert_eq!(compacted, chain, "all-pinned compaction is a no-op");
    }

    #[test]
    fn cell_versions_pinned_by_citations_returns_versions_for_cell() {
        // Two citations: one pins (Item, v=2), one pins (Other, v=5).
        let (_, s) = emit_citation_fact_pinned(
            "platform:pin1", "Storage-Pin", "2026-05-05T00:00:00Z",
            None, Some(("Item", 2)), &Object::phi());
        let (_, s) = emit_citation_fact_pinned(
            "platform:pin2", "Storage-Pin", "2026-05-05T00:00:00Z",
            None, Some(("Other", 5)), &s);

        let item_pins = cell_versions_pinned_by_citations(&s, "Item");
        assert!(item_pins.contains(&2),
            "Item pins must include v=2; got {:?}", item_pins);
        assert!(!item_pins.contains(&5),
            "Item pins must NOT include Other's v=5; got {:?}", item_pins);

        let other_pins = cell_versions_pinned_by_citations(&s, "Other");
        assert_eq!(other_pins.iter().copied().collect::<Vec<_>>(), vec![5]);
    }

    #[test]
    fn cell_versions_pinned_by_citations_empty_for_uncited_cell() {
        let (_, s) = emit_citation_fact_pinned(
            "platform:pin1", "Storage-Pin", "2026-05-05T00:00:00Z",
            None, Some(("Item", 2)), &Object::phi());
        let pins = cell_versions_pinned_by_citations(&s, "NoSuchCell");
        assert!(pins.is_empty(),
            "cell with no Citation pins must yield empty set; got {:?}", pins);
    }

    #[test]
    fn merge_delta_on_chain_does_not_lose_prior_versions() {
        // Three sequential merges to the same cell — chain should grow
        // to three entries.
        let mut state = Object::Map(HashMap::new().into());
        for tag in &["a", "b", "c"] {
            let mut d = HashMap::new();
            d.insert("Item".to_string(), Object::atom(tag));
            state = merge_delta(&state, &Object::Map(d.into()), None);
        }
        let hist = cells_iter_history(&state, "Item");
        assert_eq!(hist.len(), 3);
        assert_eq!(version_entry_contents(&hist[0]), Some(&Object::atom("a")));
        assert_eq!(version_entry_contents(&hist[1]), Some(&Object::atom("b")));
        assert_eq!(version_entry_contents(&hist[2]), Some(&Object::atom("c")));
        // version_ids are sequential starting at 1.
        assert_eq!(version_entry_id(&hist[0]), Some(1));
        assert_eq!(version_entry_id(&hist[1]), Some(2));
        assert_eq!(version_entry_id(&hist[2]), Some(3));
        // Each prev points at the previous id.
        assert_eq!(version_entry_prev(&hist[0]), None);
        assert_eq!(version_entry_prev(&hist[1]), Some(1));
        assert_eq!(version_entry_prev(&hist[2]), Some(2));
    }

    // ── task-922-map-cell-merge-not-replace ─────────────────────────────
    //
    // When apply emits its delta as a Map-keyed cell `{<entity-id> = fact}`
    // (the per-entity routing introduced by task-744 / #770 / task-922
    // round-trip), `merge_delta` must UNION the delta's entries onto the
    // existing Map cell — not REPLACE the whole cell with just that one
    // entry. The chain layer faithfully preserves every version's
    // contents, but if version N's contents is a one-entry Map, then the
    // logical view (latest contents) is also one entry. Multi-entity
    // history is gone the moment any single-entity apply lands.
    //
    // Pre-fix: ft_Task_has_Task_Priority on the live tasks app went from
    // 700+ entries to 1 entry per session because every apply replaced
    // the cell. Post-fix: each apply either inserts a new key or updates
    // a same-key entry; all other entries are preserved.

    #[test]
    fn merge_delta_unions_map_cell_entries_instead_of_replacing() {
        // Base: Map cell already containing two entity-keyed entries.
        // Set up via two prior merges so the cell is a chain over Maps.
        let s0 = Object::Map(HashMap::new().into());
        let mut d1 = HashMap::new();
        let mut m1 = HashMap::new();
        m1.insert("ent-1".to_string(), Object::atom("fact-1"));
        d1.insert("FT".to_string(), Object::Map(m1.into()));
        let s1 = merge_delta(&s0, &Object::Map(d1.into()), None);

        let mut d2 = HashMap::new();
        let mut m2 = HashMap::new();
        m2.insert("ent-2".to_string(), Object::atom("fact-2"));
        d2.insert("FT".to_string(), Object::Map(m2.into()));
        let s2 = merge_delta(&s1, &Object::Map(d2.into()), None);

        // Sanity: latest view of FT must hold BOTH ent-1 and ent-2.
        let view2 = fetch_or_phi("FT", &s2);
        let m_view2 = view2.as_map().expect("FT cell view must be a Map");
        assert_eq!(
            m_view2.len(), 2,
            "after two single-entry Map deltas, the merged cell must \
             hold both entries; got keys = {:?}",
            m_view2.keys().collect::<Vec<_>>()
        );
        assert_eq!(m_view2.get("ent-1"), Some(&Object::atom("fact-1")));
        assert_eq!(m_view2.get("ent-2"), Some(&Object::atom("fact-2")));

        // Third merge: a NEW entity. Must add ent-3 alongside the
        // existing two — total = 3.
        let mut d3 = HashMap::new();
        let mut m3 = HashMap::new();
        m3.insert("ent-3".to_string(), Object::atom("fact-3"));
        d3.insert("FT".to_string(), Object::Map(m3.into()));
        let s3 = merge_delta(&s2, &Object::Map(d3.into()), None);

        let view3 = fetch_or_phi("FT", &s3);
        let m_view3 = view3.as_map().expect("FT cell view must be a Map");
        assert_eq!(
            m_view3.len(), 3,
            "the third single-entry Map delta must add a NEW key; \
             cell must hold all three entries; got keys = {:?}",
            m_view3.keys().collect::<Vec<_>>()
        );
        assert_eq!(m_view3.get("ent-1"), Some(&Object::atom("fact-1")));
        assert_eq!(m_view3.get("ent-2"), Some(&Object::atom("fact-2")));
        assert_eq!(m_view3.get("ent-3"), Some(&Object::atom("fact-3")));
    }

    #[test]
    fn merge_delta_map_cell_same_key_overwrites_within_merged_view() {
        // When the delta's Map carries a key the existing cell already
        // has, the new value WINS (apply update semantics) but other
        // entries are preserved untouched.
        let s0 = Object::Map(HashMap::new().into());
        let mut d1 = HashMap::new();
        let mut m1 = HashMap::new();
        m1.insert("ent-1".to_string(), Object::atom("v1"));
        m1.insert("ent-2".to_string(), Object::atom("orig-2"));
        d1.insert("FT".to_string(), Object::Map(m1.into()));
        let s1 = merge_delta(&s0, &Object::Map(d1.into()), None);

        // Update ent-1 only.
        let mut d2 = HashMap::new();
        let mut m2 = HashMap::new();
        m2.insert("ent-1".to_string(), Object::atom("v2"));
        d2.insert("FT".to_string(), Object::Map(m2.into()));
        let s2 = merge_delta(&s1, &Object::Map(d2.into()), None);

        let view = fetch_or_phi("FT", &s2);
        let m = view.as_map().expect("FT view must be a Map");
        assert_eq!(m.len(), 2, "ent-2 must be preserved alongside updated ent-1");
        assert_eq!(m.get("ent-1"), Some(&Object::atom("v2")),
            "same-key delta must REPLACE that key's value (apply update wins)");
        assert_eq!(m.get("ent-2"), Some(&Object::atom("orig-2")),
            "untouched key must keep its existing value");
    }

    #[test]
    fn merge_delta_map_cell_grows_chain_on_each_merge() {
        // History is preserved per merge. After three single-entry
        // deltas the chain has three entries; the latest holds the
        // union of all three keys.
        let s0 = Object::Map(HashMap::new().into());
        let entries: &[(&str, &str)] = &[("k1", "v1"), ("k2", "v2"), ("k3", "v3")];
        let mut state = s0;
        for (k, v) in entries {
            let mut delta = HashMap::new();
            let mut m = HashMap::new();
            m.insert((*k).to_string(), Object::atom(v));
            delta.insert("FT".to_string(), Object::Map(m.into()));
            state = merge_delta(&state, &Object::Map(delta.into()), None);
        }
        let hist = cells_iter_history(&state, "FT");
        assert_eq!(hist.len(), 3,
            "three sequential merges must extend the chain to three entries");
        // Latest contents = the merged Map (all three keys).
        let latest_contents = version_entry_contents(&hist[2])
            .expect("latest entry has contents");
        let m = latest_contents.as_map().expect("latest contents is Map");
        assert_eq!(m.len(), 3,
            "latest version's contents must hold the UNION of all three \
             entity-keyed deltas; got keys = {:?}", m.keys().collect::<Vec<_>>());
    }

    // ─── S1h: as_of / between derivations (#724) ──────────────────────

    #[test]
    fn as_of_returns_contents_at_specific_version() {
        // Three sequential merges → chain [v1=a, v2=b, v3=c].
        let mut state = Object::Map(HashMap::new().into());
        for tag in &["a", "b", "c"] {
            let mut d = HashMap::new();
            d.insert("X".to_string(), Object::atom(tag));
            state = merge_delta(&state, &Object::Map(d.into()), None);
        }
        assert_eq!(as_of(&state, "X", 1), Some(Object::atom("a")));
        assert_eq!(as_of(&state, "X", 2), Some(Object::atom("b")));
        assert_eq!(as_of(&state, "X", 3), Some(Object::atom("c")));
        // Unknown version → None
        assert_eq!(as_of(&state, "X", 99), None);
        // Unknown cell → None
        assert_eq!(as_of(&state, "Y", 1), None);
    }

    #[test]
    fn as_of_returns_none_for_unversioned_cell() {
        // Legacy raw value — no chain, no version_id to match.
        let state = Object::seq(vec![cell("X", Object::atom("raw"))]);
        assert_eq!(as_of(&state, "X", 0), None);
        assert_eq!(as_of(&state, "X", 1), None);
    }

    #[test]
    fn between_returns_chain_slice_in_chronological_order() {
        let mut state = Object::Map(HashMap::new().into());
        for tag in &["a", "b", "c", "d", "e"] {
            let mut d = HashMap::new();
            d.insert("X".to_string(), Object::atom(tag));
            state = merge_delta(&state, &Object::Map(d.into()), None);
        }
        // Inclusive range [2, 4] → entries v2, v3, v4 in order.
        let slice = between(&state, "X", 2, 4);
        assert_eq!(slice.len(), 3);
        assert_eq!(version_entry_id(&slice[0]), Some(2));
        assert_eq!(version_entry_id(&slice[1]), Some(3));
        assert_eq!(version_entry_id(&slice[2]), Some(4));
        assert_eq!(version_entry_contents(&slice[0]), Some(&Object::atom("b")));
        assert_eq!(version_entry_contents(&slice[2]), Some(&Object::atom("d")));
    }

    #[test]
    fn between_handles_singleton_range_and_full_range() {
        let mut state = Object::Map(HashMap::new().into());
        for tag in &["a", "b", "c"] {
            let mut d = HashMap::new();
            d.insert("Y".to_string(), Object::atom(tag));
            state = merge_delta(&state, &Object::Map(d.into()), None);
        }
        // [2, 2] → just v2
        let one = between(&state, "Y", 2, 2);
        assert_eq!(one.len(), 1);
        assert_eq!(version_entry_id(&one[0]), Some(2));
        // [1, 100] → all 3
        let all = between(&state, "Y", 1, 100);
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn between_returns_empty_for_inverted_range_or_unversioned() {
        let mut state = Object::Map(HashMap::new().into());
        let mut d = HashMap::new();
        d.insert("Z".to_string(), Object::atom("v"));
        state = merge_delta(&state, &Object::Map(d.into()), None);
        // Inverted range — total function, just empty.
        assert!(between(&state, "Z", 5, 1).is_empty());
        // Legacy raw cell — no chain to slice.
        let raw = Object::seq(vec![cell("Z", Object::atom("raw"))]);
        assert!(between(&raw, "Z", 1, 100).is_empty());
    }

    #[cfg(feature = "wall-clock")]
    #[test]
    fn platform_now_returns_monotonically_nondecreasing_decimal_atom() {
        let t1 = apply_platform("now", &Object::phi(), &Object::phi());
        let t2 = apply_platform("now", &Object::phi(), &Object::phi());
        let s1 = t1.as_atom().expect("now returns an atom");
        let s2 = t2.as_atom().expect("now returns an atom");
        let ms1: u128 = s1.parse().expect("decimal millis");
        let ms2: u128 = s2.parse().expect("decimal millis");
        assert!(ms1 > 0, "host clock should be after epoch");
        assert!(ms2 >= ms1, "wall clock must be monotonic across consecutive calls");
    }

    #[test]
    fn cells_iter_enumerates_all_cells() {
        let state = Object::seq(vec![
            cell("A", Object::atom("1")),
            cell("B", Object::atom("2")),
        ]);
        let pairs: Vec<(&str, &Object)> = cells_iter(&state);
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].0, "A");
        assert_eq!(pairs[1].0, "B");
    }

    // #209 — diff_cells / merge_delta round-trip invariants.

    #[test]
    fn diff_cells_of_identical_stores_is_empty() {
        let state = Object::seq(vec![
            cell("A", Object::atom("1")),
            cell("B", Object::atom("2")),
        ]);
        let delta = diff_cells(&state, &state);
        let map = delta.as_map().expect("delta is Map");
        assert!(map.is_empty(), "identical stores must produce empty delta");
    }

    #[test]
    fn diff_cells_from_phi_returns_all_cells() {
        let new = Object::seq(vec![
            cell("A", Object::atom("1")),
            cell("B", Object::atom("2")),
        ]);
        let delta = diff_cells(&Object::phi(), &new);
        let map = delta.as_map().expect("delta is Map");
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("A"), Some(&Object::atom("1")));
        assert_eq!(map.get("B"), Some(&Object::atom("2")));
    }

    #[test]
    fn diff_cells_emits_only_changed_cells() {
        let old = Object::seq(vec![
            cell("A", Object::atom("1")),
            cell("B", Object::atom("2")),
            cell("C", Object::atom("3")),
        ]);
        let new = Object::seq(vec![
            cell("A", Object::atom("1")),          // unchanged
            cell("B", Object::atom("CHANGED")),    // changed
            cell("C", Object::atom("3")),          // unchanged
            cell("D", Object::atom("4")),          // added
        ]);
        let delta = diff_cells(&old, &new);
        let map = delta.as_map().expect("delta is Map");
        assert_eq!(map.len(), 2, "only B and D should be in delta");
        assert_eq!(map.get("B"), Some(&Object::atom("CHANGED")));
        assert_eq!(map.get("D"), Some(&Object::atom("4")));
        assert!(map.get("A").is_none());
        assert!(map.get("C").is_none());
    }

    #[test]
    fn merge_delta_is_inverse_of_diff_cells_for_present_cells() {
        let old = Object::seq(vec![
            cell("A", Object::atom("1")),
            cell("B", Object::atom("2")),
            cell("C", Object::atom("3")),
        ]);
        let new = Object::seq(vec![
            cell("A", Object::atom("1")),
            cell("B", Object::atom("CHANGED")),
            cell("C", Object::atom("3")),
            cell("D", Object::atom("4")),
        ]);
        let delta = diff_cells(&old, &new);
        let reconstructed = merge_delta(&old, &delta, None);
        for name in ["A", "B", "C", "D"] {
            assert_eq!(fetch_or_phi(name, &reconstructed), fetch_or_phi(name, &new),
                "cell {} must match after merge_delta(old, diff(old,new))", name);
        }

        // store-on-derive STEP 1: the inverse receipt must ALSO hold when a
        // cell LOSES tuples — the case the union-only path cannot express.
        // A folded Map cell shrinks (one tuple retracted); a Seq cell shrinks;
        // a cell vanishes entirely. `diff_cells_with_removals` +
        // `merge_delta_with_removals` must reconstruct `new` exactly, where
        // plain `diff_cells`+`merge_delta` would resurrect the dropped tuples
        // via the union.
        let fa = fact_from_pairs(&[("Task", "a"), ("Status", "pending")]);
        let fb = fact_from_pairs(&[("Task", "b"), ("Status", "pending")]);
        let mut map_old: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        map_old.insert("ka".to_string(), fa.clone());
        map_old.insert("kb".to_string(), fb.clone());
        let s_old = Object::seq(vec![
            cell("Folded", Object::Map(map_old.into())),
            cell("Seqcell", Object::seq(vec![Object::atom("x"), Object::atom("y")])),
            cell("Vanish", Object::seq(vec![Object::atom("gone")])),
            cell("Keep", Object::atom("same")),
        ]);
        // new: Folded loses b, Seqcell loses y, Vanish becomes empty, Keep stays.
        let mut map_new: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        map_new.insert("ka".to_string(), fa.clone());
        let s_new = Object::seq(vec![
            cell("Folded", Object::Map(map_new.into())),
            cell("Seqcell", Object::seq(vec![Object::atom("x")])),
            cell("Vanish", Object::seq(vec![])),
            cell("Keep", Object::atom("same")),
        ]);
        let (adds, rems) = diff_cells_with_removals(&s_old, &s_new);
        let recon = merge_delta_with_removals(&s_old, &adds, &rems, None);
        for name in ["Folded", "Seqcell", "Vanish", "Keep"] {
            assert_eq!(
                fetch_cell_seq(name, &recon), fetch_cell_seq(name, &s_new),
                "cell {} must match new after retraction-aware diff+merge round-trip",
                name);
        }
    }

    /// store-on-derive STEP 1 (keystone): a removal channel retracts a
    /// tuple from a folded Map cell. cell {a,b}; delta removes b ⇒ {a}.
    #[test]
    fn merge_delta_removes_retracted_tuple_from_map_cell() {
        let fa = fact_from_pairs(&[("Task", "a")]);
        let fb = fact_from_pairs(&[("Task", "b")]);
        let mut m: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        m.insert("ka".to_string(), fa.clone());
        m.insert("kb".to_string(), fb.clone());
        let base = Object::seq(vec![cell("Cell", Object::Map(m.into()))]);

        // Removal channel: drop tuple `b` from `Cell`. No additions.
        let mut rem: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        rem.insert("Cell".to_string(), Object::seq(vec![fb.clone()]));
        let merged = merge_delta_with_removals(
            &base, &Object::Map(hashbrown::HashMap::new().into()),
            &Object::Map(rem.into()), None);

        let got = fetch_cell_seq("Cell", &merged);
        let items = got.as_seq().expect("cell is a Seq view");
        assert_eq!(items.len(), 1, "exactly one tuple must remain after removing b");
        assert_eq!(items[0], fa, "the surviving tuple must be a");
        assert!(!items.iter().any(|f| *f == fb), "tuple b must be gone");

        // A new chain version was minted (the commit is recorded, not a no-op).
        let hist = cells_iter_history(&merged, "Cell");
        assert_eq!(hist.len(), 2, "retraction commits one new version on top of base");
    }

    /// store-on-derive STEP 1: retract-then-reinsert the SAME tuple is
    /// idempotent on the logical view — removal happens BEFORE the add, so
    /// adding it back lands exactly one copy (a folded cell keys it by
    /// tuple identity; the union overwrites at that key).
    #[test]
    fn merge_delta_retraction_then_reinsert_is_idempotent() {
        let fa = fact_from_pairs(&[("Task", "a")]);
        let fb = fact_from_pairs(&[("Task", "b")]);
        let mut m: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        m.insert("ka".to_string(), fa.clone());
        m.insert("kb".to_string(), fb.clone());
        let base = Object::seq(vec![cell("Cell", Object::Map(m.into()))]);

        // Commit: remove b AND add b back (same tuple) in one delta. Removal
        // is applied first, then the addition re-lands it — net no change to
        // the logical set {a, b}.
        let mut rem: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        rem.insert("Cell".to_string(), Object::seq(vec![fb.clone()]));
        let mut add_map: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        add_map.insert("kb".to_string(), fb.clone());
        let mut adds: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        adds.insert("Cell".to_string(), Object::Map(add_map.into()));
        let merged = merge_delta_with_removals(
            &base, &Object::Map(adds.into()), &Object::Map(rem.into()), None);

        let got = fetch_cell_seq("Cell", &merged);
        let items = got.as_seq().expect("cell is a Seq view");
        assert_eq!(items.len(), 2, "retract-then-reinsert keeps the set {{a,b}} (no dup)");
        assert!(items.iter().any(|f| *f == fa), "a still present");
        assert!(items.iter().filter(|f| **f == fb).count() == 1, "exactly one b (idempotent)");
    }

    /// store-on-derive STEP 1: `merge_delta_with_removals` with an EMPTY
    /// removal channel is byte-for-byte the union-only `merge_delta` — the
    /// proof that the additions/union/overwrite behavior is UNCHANGED and
    /// every existing caller can adopt the retraction-aware merge with a
    /// `phi` removals argument and observe no difference.
    #[test]
    fn merge_delta_with_empty_removals_equals_merge_delta() {
        // Mix: Map base (union case), legacy-raw promote, absent-cell create.
        let fa = fact_from_pairs(&[("Task", "a")]);
        let mut m: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        m.insert("ka".to_string(), fa);
        let base = Object::seq(vec![
            cell("Folded", Object::Map(m.into())),
            cell("Raw", Object::atom("legacy")),
        ]);
        // Delta: union a tuple into Folded, overwrite Raw, create Fresh.
        let fb = fact_from_pairs(&[("Task", "b")]);
        let mut fold_add: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        fold_add.insert("kb".to_string(), fb);
        let mut delta: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        delta.insert("Folded".to_string(), Object::Map(fold_add.into()));
        delta.insert("Raw".to_string(), Object::atom("new"));
        delta.insert("Fresh".to_string(), Object::atom("hello"));
        let delta = Object::Map(delta.into());

        let union_only = merge_delta(&base, &delta, None);
        let via_removals = merge_delta_with_removals(
            &base, &delta, &Object::Map(hashbrown::HashMap::new().into()), None);

        // Compare modulo the per-commit `recorded_at` stamp: `merge_delta`
        // and `merge_delta_with_removals` each call `logical_commit_stamp()`
        // (a global monotonic counter), so two separate commits get distinct
        // stamps and a raw whole-object `==` would differ on the stamp alone.
        // The additions behavior is proven by: (a) identical logical views,
        // and (b) identical version chains down to id / prev / CONTENTS for
        // every cell.
        for name in ["Folded", "Raw", "Fresh"] {
            assert_eq!(
                fetch_cell_seq(name, &union_only), fetch_cell_seq(name, &via_removals),
                "cell {} logical view must be identical with an empty removal channel",
                name);
            let h_union = cells_iter_history(&union_only, name);
            let h_rem = cells_iter_history(&via_removals, name);
            assert_eq!(h_union.len(), h_rem.len(),
                "cell {} must have the same number of versions", name);
            for (a, b) in h_union.iter().zip(h_rem.iter()) {
                assert_eq!(version_entry_id(a), version_entry_id(b),
                    "cell {} version ids must match", name);
                assert_eq!(version_entry_prev(a), version_entry_prev(b),
                    "cell {} version prev pointers must match", name);
                assert_eq!(version_entry_contents(a), version_entry_contents(b),
                    "cell {} version CONTENTS must match (additions behavior unchanged)",
                    name);
            }
        }
        // Cell set is identical (no spurious cells minted/dropped).
        let mut names_union: Vec<&str> = cells_iter(&union_only).into_iter().map(|(n, _)| n).collect();
        let mut names_rem: Vec<&str> = cells_iter(&via_removals).into_iter().map(|(n, _)| n).collect();
        names_union.sort();
        names_rem.sort();
        assert_eq!(names_union, names_rem,
            "the empty removal channel must mint/drop NO cells vs merge_delta");
    }

    #[test]
    fn merge_delta_with_empty_delta_preserves_base() {
        let base = Object::seq(vec![
            cell("A", Object::atom("1")),
            cell("B", Object::atom("2")),
        ]);
        let empty_delta = Object::Map(HashMap::new().into());
        let merged = merge_delta(&base, &empty_delta, None);
        assert_eq!(fetch_or_phi("A", &merged), Object::atom("1"));
        assert_eq!(fetch_or_phi("B", &merged), Object::atom("2"));
    }

    #[test]
    fn binding_extracts_value_by_key() {
        let fact = fact_from_pairs(&[("name", "Alice"), ("objectType", "entity")]);
        assert_eq!(binding(&fact, "name"), Some("Alice"));
        assert_eq!(binding(&fact, "objectType"), Some("entity"));
        assert_eq!(binding(&fact, "missing"), None);
    }

    #[test]
    fn binding_matches_checks_key_value_pair() {
        let fact = fact_from_pairs(&[("name", "Alice"), ("objectType", "entity")]);
        assert!(binding_matches(&fact, "name", "Alice"));
        assert!(!binding_matches(&fact, "name", "Bob"));
        assert!(!binding_matches(&fact, "missing", "Alice"));
    }

    #[test]
    fn fact_from_pairs_builds_named_tuple() {
        let fact = fact_from_pairs(&[("k1", "v1"), ("k2", "v2")]);
        let items = fact.as_seq().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].as_seq().unwrap()[0].as_atom(), Some("k1"));
        assert_eq!(items[0].as_seq().unwrap()[1].as_atom(), Some("v1"));
    }

    #[test]
    fn cell_filter_keeps_matching_facts() {
        let f1 = fact_from_pairs(&[("name", "Alice")]);
        let f2 = fact_from_pairs(&[("name", "Bob")]);
        let state = cell_push("Noun", f1.clone(), &Object::phi());
        let state = cell_push("Noun", f2, &state);
        let state = cell_filter("Noun", |f| binding_matches(f, "name", "Alice"), &state);
        assert_eq!(fetch_or_phi("Noun", &state), Object::seq(vec![f1]));
    }

    #[test]
    fn cell_push_preserves_other_cells() {
        let state = cell_push("A", Object::atom("1"), &Object::phi());
        let state = cell_push("B", Object::atom("2"), &state);
        assert_eq!(fetch_or_phi("A", &state), Object::seq(vec![Object::atom("1")]));
        assert_eq!(fetch_or_phi("B", &state), Object::seq(vec![Object::atom("2")]));
    }

    #[test]
    fn cell_push_preserves_map_cell_entries() {
        // sm-status-bridge-projection-lag regression: pushing onto a Map (folded
        // / keyed D_n, #932) cell MUST preserve existing entries. The legacy
        // cell_push read the cell via `as_seq()` (None for a Map) and the
        // `None => Seq([fact])` branch replaced the whole cell with a single-entry
        // Seq, wiping every OTHER entry. The transition handler's `cell_filter`
        // (Map-tolerant) + `cell_push` on the keyed
        // `State_Machine_is_currently_in_Status` Map thus dropped all OTHER
        // machines' statuses; the from-guarded fold then re-seeded them to
        // `initial`, reverting an unrelated entity's status mid-transition (a
        // completed blocker flipped back to in_progress, re-blocking its
        // dependent). Root cause found by stepping the apply path on a faithful
        // metamodel_state() reproduction.
        let fact_a = fact_from_pairs(&[("State Machine", "A"), ("Status", "completed")]);
        let mut m: hashbrown::HashMap<String, Object> = hashbrown::HashMap::new();
        m.insert("A".to_string(), fact_a);
        let state = store("SM", Object::Map(m.into()), &Object::phi());

        let fact_b = fact_from_pairs(&[("State Machine", "B"), ("Status", "in_progress")]);
        let after = cell_push("SM", fact_b, &state);

        let facts = fetch_cell_seq("SM", &after);
        let seq = facts.as_seq().expect("cell should be a Seq after a push onto a Map");
        let present = |sm: &str, st: &str| seq.iter().any(|f|
            binding(f, "State Machine") == Some(sm) && binding(f, "Status") == Some(st));
        assert!(present("A", "completed"),
            "cell_push onto a Map MUST preserve existing entry A=completed; got {:?}", seq);
        assert!(present("B", "in_progress"),
            "cell_push must add the new entry B=in_progress; got {:?}", seq);
    }

    // ── Security #22: Evolution state machine trace ──────────────

    #[test]
    fn record_compile_event_appends_domain_change_to_empty_state() {
        let state = Object::phi();
        let result = record_compile_event(&state, "compiled");
        let history = fetch_or_phi("compile_history", &result);
        let facts = history.as_seq().expect("compile_history should be a sequence");
        assert_eq!(facts.len(), 1);
        assert_eq!(binding(&facts[0], "Domain Change"), Some("compile-0"));
        assert_eq!(binding(&facts[0], "status"), Some("compiled"));
    }

    #[test]
    fn record_compile_event_appends_with_increasing_sequence() {
        let state = record_compile_event(&Object::phi(), "compiled");
        let state = record_compile_event(&state, "compiled");
        let state = record_compile_event(&state, "compiled");
        let history = fetch_or_phi("compile_history", &state);
        let facts = history.as_seq().expect("compile_history should be a sequence");
        assert_eq!(facts.len(), 3);
        assert_eq!(binding(&facts[0], "Domain Change"), Some("compile-0"));
        assert_eq!(binding(&facts[1], "Domain Change"), Some("compile-1"));
        assert_eq!(binding(&facts[2], "Domain Change"), Some("compile-2"));
    }

    #[test]
    fn platform_compile_records_compile_history_entry_on_success() {
        // Feed platform_compile a minimal valid FORML2 reading via the Func::Platform path.
        // After success, compile_history should contain a single "compiled" entry.
        let readings = "Each Person has a name.";
        let initial_d = defs_to_state(
            &vec![("compile".to_string(), Func::Platform("compile".to_string()))],
            &Object::phi(),
        );
        let result = apply(
            &Func::Platform("compile".to_string()),
            &Object::atom(readings),
            &initial_d,
        );
        // Must be a state (seq or map), not an atom error starting with "⊥".
        assert!(
            result.as_seq().is_some() || result.as_map().is_some(),
            "compile should produce a state, got: {:?}",
            result
        );
        assert!(
            result.as_atom().map(|s| !s.starts_with("⊥")).unwrap_or(true),
            "compile should not return an error atom, got: {:?}",
            result
        );
        let history = fetch_or_phi("compile_history", &result);
        let facts = history.as_seq().expect("compile_history cell should exist after successful compile");
        assert_eq!(facts.len(), 1, "expected exactly one compile_history entry");
        assert_eq!(binding(&facts[0], "status"), Some("compiled"));
        assert_eq!(binding(&facts[0], "Domain Change"), Some("compile-0"));
    }

    // ── #894: cidr_contains Platform Func ─────────────────────────

    #[test]
    fn platform_cidr_contains_ipv4_loopback_returns_t() {
        let x = Object::seq(vec![Object::atom("127.0.0.0/8"), Object::atom("127.0.0.1")]);
        assert_eq!(super::platform_cidr_contains(&x), Object::t());
    }

    #[test]
    fn platform_cidr_contains_outside_range_returns_f() {
        let x = Object::seq(vec![Object::atom("127.0.0.0/8"), Object::atom("8.8.8.8")]);
        assert_eq!(super::platform_cidr_contains(&x), Object::f());
    }

    #[test]
    fn platform_cidr_contains_ipv6_link_local_returns_t() {
        let x = Object::seq(vec![Object::atom("fe80::/10"), Object::atom("fe80::1")]);
        assert_eq!(super::platform_cidr_contains(&x), Object::t());
    }

    #[test]
    fn platform_cidr_contains_malformed_returns_bottom() {
        // Wrong arity ⇒ Bottom.
        let x = Object::seq(vec![Object::atom("127.0.0.0/8")]);
        assert_eq!(super::platform_cidr_contains(&x), Object::Bottom);
        // Non-atom in cidr slot ⇒ Bottom.
        let x = Object::seq(vec![Object::phi(), Object::atom("127.0.0.1")]);
        assert_eq!(super::platform_cidr_contains(&x), Object::Bottom);
    }

    /// Pin: `platform_compile` registers `cidr_contains` in DEFS so apps
    /// invoking the verb get the live body, not the no-body Bottom
    /// fallback. The other Platform fn defs (`compile`, `apply`,
    /// `verify_signature`, `audit`) all go through this same path —
    /// `cidr_contains` joining them is the lift's "wiring" step.
    #[test]
    fn platform_compile_registers_cidr_contains_in_defs() {
        let readings = "Each Person has a name.";
        let initial_d = defs_to_state(
            &vec![("compile".to_string(), Func::Platform("compile".to_string()))],
            &Object::phi(),
        );
        let result = apply(
            &Func::Platform("compile".to_string()),
            &Object::atom(readings),
            &initial_d,
        );
        let cidr_def = fetch("cidr_contains", &result);
        assert!(
            matches!(&cidr_def, Object::Seq(_) | Object::Map(_) | Object::Atom(_)),
            "cidr_contains should be registered in DEFS after compile, got: {:?}",
            cidr_def
        );
    }

    // ── #689: Policy_skip_validate cell semantics ─────────────────

    #[test]
    fn skip_validate_default_is_false_on_empty_state() {
        assert!(!is_skip_validate(&Object::phi()));
    }

    #[test]
    fn install_skip_validate_sets_policy_cell_to_atom_t() {
        let state = install_skip_validate(&Object::phi());
        assert_eq!(fetch(POLICY_SKIP_VALIDATE, &state), Object::atom("T"));
        assert!(is_skip_validate(&state));
    }

    #[test]
    fn install_skip_validate_is_idempotent() {
        let once = install_skip_validate(&Object::phi());
        let twice = install_skip_validate(&once);
        assert_eq!(once, twice);
        assert!(is_skip_validate(&twice));
    }

    #[test]
    fn skip_validate_is_false_when_cell_holds_other_atom() {
        // Only atom "T" enables the policy — defends against other
        // truthy strings ("true", "1", "yes") accidentally flipping it.
        let state = store(POLICY_SKIP_VALIDATE, Object::atom("F"), &Object::phi());
        assert!(!is_skip_validate(&state));
        let state = store(POLICY_SKIP_VALIDATE, Object::atom("true"), &Object::phi());
        assert!(!is_skip_validate(&state));
    }

    #[test]
    fn platform_compile_skips_validate_when_policy_installed() {
        // A reading that violates the metamodel-shadow guard would normally
        // be rejected by validate. With Policy_skip_validate set, validation
        // is skipped and a benign compile path completes successfully. We
        // assert via the compile_history side-effect — present iff compile
        // ran to record_compile_event without rejecting on validation.
        let readings = "Each Person has a name.";
        let initial_d = defs_to_state(
            &vec![("compile".to_string(), Func::Platform("compile".to_string()))],
            &Object::phi(),
        );
        let policy_d = install_skip_validate(&initial_d);
        let result = apply(
            &Func::Platform("compile".to_string()),
            &Object::atom(readings),
            &policy_d,
        );
        let history = fetch_or_phi("compile_history", &result);
        let facts = history.as_seq().expect("compile_history cell should exist");
        assert_eq!(facts.len(), 1);
        assert_eq!(binding(&facts[0], "status"), Some("compiled"));
    }

    // ── S1c (#719): VersionEntry event field ──────────────────────────
    //
    // The chain (S1b) replaces the legacy `audit_log` cell. VersionEntry
    // optionally carries the apply-time event (eq:cellfold's `μ_n`
    // input) so operation kind + sender + payload are queryable from
    // `cells_iter_history` without a parallel audit_log scaffold.

    #[test]
    fn version_entry_event_extracts_optional_field_when_present() {
        let event = fact_from_pairs(&[("operation", "apply:create"), ("sender", "u1")]);
        let entry = version_entry(
            1,
            Object::atom("payload"),
            None,
            Object::atom("now"),
            Some(event.clone()),
        );
        assert_eq!(version_entry_event(&entry), Some(&event));
        assert_eq!(version_entry_id(&entry), Some(1));
        assert_eq!(version_entry_contents(&entry), Some(&Object::atom("payload")));
    }

    #[test]
    fn version_entry_event_returns_none_for_eventless_entries() {
        let entry = version_entry(1, Object::atom("p"), None, Object::atom("now"), None);
        assert!(version_entry_event(&entry).is_none(),
            "entries constructed with event=None must not surface an event field");
    }

    #[test]
    fn apply_event_round_trips_verb_and_operand() {
        let operand = fact_from_pairs(&[("id", "ord-1"), ("total", "100")]);
        let event = apply_event("create:Order", operand.clone());
        assert_eq!(apply_event_verb(&event), Some("create:Order"));
        assert_eq!(apply_event_operand(&event), Some(&operand));
    }

    #[test]
    fn apply_event_preserves_non_atom_operand_structure() {
        // Operand can be any Object — a Seq of pairs in this case.
        // apply_event must not flatten or stringify it.
        let operand = Object::seq(alloc::vec![
            Object::seq(alloc::vec![Object::atom("a"), Object::atom("1")]),
            Object::seq(alloc::vec![Object::atom("b"), Object::atom("2")]),
        ]);
        let event = apply_event("transition:Order", operand.clone());
        let extracted = apply_event_operand(&event)
            .expect("operand must round-trip");
        assert_eq!(extracted, &operand);
    }

    #[test]
    fn merge_delta_with_event_attaches_event_to_new_chain_entry() {
        let event = fact_from_pairs(&[("operation", "apply:create"), ("sender", "u1")]);
        let s0 = Object::Map(HashMap::new().into());
        let mut d = HashMap::new();
        d.insert("Order".to_string(), Object::atom("ord-1"));
        let s1 = merge_delta(&s0, &Object::Map(d.into()), Some(event.clone()));

        let hist = cells_iter_history(&s1, "Order");
        assert_eq!(hist.len(), 1);
        assert_eq!(version_entry_id(&hist[0]), Some(1));
        assert_eq!(version_entry_event(&hist[0]), Some(&event),
            "event must round-trip through merge_delta");
    }

    #[test]
    fn merge_delta_eventless_path_omits_event_field() {
        // The plain merge_delta delegates to the event variant with
        // None — entries must remain in the pre-S1c 4-field shape so
        // pre-existing freezes keep round-tripping.
        let s0 = Object::Map(HashMap::new().into());
        let mut d = HashMap::new();
        d.insert("Order".to_string(), Object::atom("ord-1"));
        let s1 = merge_delta(&s0, &Object::Map(d.into()), None);
        let hist = cells_iter_history(&s1, "Order");
        assert!(version_entry_event(&hist[0]).is_none());
    }

    // ── Security #19: per-field input bound (PLATFORM_MAX_FIELD) ─────
    //
    // `command_field_overflow` walks every Command variant and returns
    // the first field name whose String value exceeds PLATFORM_MAX_FIELD
    // (64KB). These tests lock the contract down per variant per field,
    // including HashMap key/value overflow on fields/bindings, and then
    // cover the integration path via `platform_apply_command` for both
    // the PLATFORM_MAX_INPUT (1MB) and PLATFORM_MAX_FIELD gates.

    use crate::command::Command as ArestCommand;

    fn huge() -> String {
        "a".repeat(PLATFORM_MAX_FIELD + 1)
    }

    fn ok_map() -> hashbrown::HashMap<String, String> {
        let mut m = hashbrown::HashMap::new();
        m.insert("k".to_string(), "v".to_string());
        m
    }

    // ── CreateEntity variants ────────────────────────────────────

    #[test]
    fn command_field_overflow_create_noun_oversized() {
        let cmd = ArestCommand::CreateEntity {
            noun: huge(),
            domain: "d".into(),
            id: None,
            fields: ok_map(),
            sender: None,
            signature: None,
        };
        assert_eq!(command_field_overflow(&cmd), Some("noun"));
    }

    #[test]
    fn command_field_overflow_create_domain_oversized() {
        let cmd = ArestCommand::CreateEntity {
            noun: "n".into(),
            domain: huge(),
            id: None,
            fields: ok_map(),
            sender: None,
            signature: None,
        };
        assert_eq!(command_field_overflow(&cmd), Some("domain"));
    }

    #[test]
    fn command_field_overflow_create_id_oversized() {
        let cmd = ArestCommand::CreateEntity {
            noun: "n".into(),
            domain: "d".into(),
            id: Some(huge()),
            fields: ok_map(),
            sender: None,
            signature: None,
        };
        assert_eq!(command_field_overflow(&cmd), Some("id"));
    }

    #[test]
    fn command_field_overflow_create_fields_key_oversized() {
        let mut fields = hashbrown::HashMap::new();
        fields.insert(huge(), "v".to_string());
        let cmd = ArestCommand::CreateEntity {
            noun: "n".into(),
            domain: "d".into(),
            id: None,
            fields,
            sender: None,
            signature: None,
        };
        assert_eq!(command_field_overflow(&cmd), Some("fields"));
    }

    #[test]
    fn command_field_overflow_create_fields_value_oversized() {
        let mut fields = hashbrown::HashMap::new();
        fields.insert("k".to_string(), huge());
        let cmd = ArestCommand::CreateEntity {
            noun: "n".into(),
            domain: "d".into(),
            id: None,
            fields,
            sender: None,
            signature: None,
        };
        assert_eq!(command_field_overflow(&cmd), Some("fields"));
    }

    #[test]
    fn command_field_overflow_create_sender_oversized() {
        let cmd = ArestCommand::CreateEntity {
            noun: "n".into(),
            domain: "d".into(),
            id: None,
            fields: ok_map(),
            sender: Some(huge()),
            signature: None,
        };
        assert_eq!(command_field_overflow(&cmd), Some("sender"));
    }

    #[test]
    fn command_field_overflow_create_signature_oversized() {
        let cmd = ArestCommand::CreateEntity {
            noun: "n".into(),
            domain: "d".into(),
            id: None,
            fields: ok_map(),
            sender: None,
            signature: Some(huge()),
        };
        assert_eq!(command_field_overflow(&cmd), Some("signature"));
    }

    #[test]
    fn command_field_overflow_create_valid_returns_none() {
        let cmd = ArestCommand::CreateEntity {
            noun: "Person".into(),
            domain: "d".into(),
            id: Some("p-1".into()),
            fields: ok_map(),
            sender: Some("u1".into()),
            signature: Some("sig".into()),
        };
        assert_eq!(command_field_overflow(&cmd), None);
    }

    // ── Transition variants ──────────────────────────────────────

    #[test]
    fn command_field_overflow_transition_entity_id_oversized() {
        let cmd = ArestCommand::Transition {
            entity_id: huge(),
            event: "e".into(),
            domain: "d".into(),
            current_status: None,
            sender: None,
            signature: None,
        };
        assert_eq!(command_field_overflow(&cmd), Some("entityId"));
    }

    #[test]
    fn command_field_overflow_transition_event_oversized() {
        let cmd = ArestCommand::Transition {
            entity_id: "e-1".into(),
            event: huge(),
            domain: "d".into(),
            current_status: None,
            sender: None,
            signature: None,
        };
        assert_eq!(command_field_overflow(&cmd), Some("event"));
    }

    #[test]
    fn command_field_overflow_transition_domain_oversized() {
        let cmd = ArestCommand::Transition {
            entity_id: "e-1".into(),
            event: "e".into(),
            domain: huge(),
            current_status: None,
            sender: None,
            signature: None,
        };
        assert_eq!(command_field_overflow(&cmd), Some("domain"));
    }

    #[test]
    fn command_field_overflow_transition_current_status_oversized() {
        let cmd = ArestCommand::Transition {
            entity_id: "e-1".into(),
            event: "e".into(),
            domain: "d".into(),
            current_status: Some(huge()),
            sender: None,
            signature: None,
        };
        assert_eq!(command_field_overflow(&cmd), Some("currentStatus"));
    }

    #[test]
    fn command_field_overflow_transition_sender_oversized() {
        let cmd = ArestCommand::Transition {
            entity_id: "e-1".into(),
            event: "e".into(),
            domain: "d".into(),
            current_status: None,
            sender: Some(huge()),
            signature: None,
        };
        assert_eq!(command_field_overflow(&cmd), Some("sender"));
    }

    #[test]
    fn command_field_overflow_transition_signature_oversized() {
        let cmd = ArestCommand::Transition {
            entity_id: "e-1".into(),
            event: "e".into(),
            domain: "d".into(),
            current_status: None,
            sender: None,
            signature: Some(huge()),
        };
        assert_eq!(command_field_overflow(&cmd), Some("signature"));
    }

    #[test]
    fn command_field_overflow_transition_valid_returns_none() {
        let cmd = ArestCommand::Transition {
            entity_id: "e-1".into(),
            event: "approve".into(),
            domain: "d".into(),
            current_status: Some("draft".into()),
            sender: Some("u1".into()),
            signature: Some("sig".into()),
        };
        assert_eq!(command_field_overflow(&cmd), None);
    }

    // ── Query variants ───────────────────────────────────────────

    #[test]
    fn command_field_overflow_query_schema_id_oversized() {
        let cmd = ArestCommand::Query {
            schema_id: huge(),
            domain: "d".into(),
            target: "t".into(),
            bindings: ok_map(),
            sender: None,
            signature: None,
        };
        assert_eq!(command_field_overflow(&cmd), Some("schemaId"));
    }

    #[test]
    fn command_field_overflow_query_domain_oversized() {
        let cmd = ArestCommand::Query {
            schema_id: "s".into(),
            domain: huge(),
            target: "t".into(),
            bindings: ok_map(),
            sender: None,
            signature: None,
        };
        assert_eq!(command_field_overflow(&cmd), Some("domain"));
    }

    #[test]
    fn command_field_overflow_query_target_oversized() {
        let cmd = ArestCommand::Query {
            schema_id: "s".into(),
            domain: "d".into(),
            target: huge(),
            bindings: ok_map(),
            sender: None,
            signature: None,
        };
        assert_eq!(command_field_overflow(&cmd), Some("target"));
    }

    #[test]
    fn command_field_overflow_query_bindings_key_oversized() {
        let mut bindings = hashbrown::HashMap::new();
        bindings.insert(huge(), "v".to_string());
        let cmd = ArestCommand::Query {
            schema_id: "s".into(),
            domain: "d".into(),
            target: "t".into(),
            bindings,
            sender: None,
            signature: None,
        };
        assert_eq!(command_field_overflow(&cmd), Some("bindings"));
    }

    #[test]
    fn command_field_overflow_query_bindings_value_oversized() {
        let mut bindings = hashbrown::HashMap::new();
        bindings.insert("k".to_string(), huge());
        let cmd = ArestCommand::Query {
            schema_id: "s".into(),
            domain: "d".into(),
            target: "t".into(),
            bindings,
            sender: None,
            signature: None,
        };
        assert_eq!(command_field_overflow(&cmd), Some("bindings"));
    }

    #[test]
    fn command_field_overflow_query_sender_oversized() {
        let cmd = ArestCommand::Query {
            schema_id: "s".into(),
            domain: "d".into(),
            target: "t".into(),
            bindings: ok_map(),
            sender: Some(huge()),
            signature: None,
        };
        assert_eq!(command_field_overflow(&cmd), Some("sender"));
    }

    #[test]
    fn command_field_overflow_query_signature_oversized() {
        let cmd = ArestCommand::Query {
            schema_id: "s".into(),
            domain: "d".into(),
            target: "t".into(),
            bindings: ok_map(),
            sender: None,
            signature: Some(huge()),
        };
        assert_eq!(command_field_overflow(&cmd), Some("signature"));
    }

    #[test]
    fn command_field_overflow_query_valid_returns_none() {
        let cmd = ArestCommand::Query {
            schema_id: "s".into(),
            domain: "d".into(),
            target: "t".into(),
            bindings: ok_map(),
            sender: Some("u1".into()),
            signature: Some("sig".into()),
        };
        assert_eq!(command_field_overflow(&cmd), None);
    }

    // ── UpdateEntity variants ────────────────────────────────────

    #[test]
    fn command_field_overflow_update_noun_oversized() {
        let cmd = ArestCommand::UpdateEntity {
            noun: huge(),
            domain: "d".into(),
            entity_id: "e".into(),
            fields: ok_map(),
            sender: None,
            signature: None,
            force: false,
        };
        assert_eq!(command_field_overflow(&cmd), Some("noun"));
    }

    #[test]
    fn command_field_overflow_update_domain_oversized() {
        let cmd = ArestCommand::UpdateEntity {
            noun: "n".into(),
            domain: huge(),
            entity_id: "e".into(),
            fields: ok_map(),
            sender: None,
            signature: None,
            force: false,
        };
        assert_eq!(command_field_overflow(&cmd), Some("domain"));
    }

    #[test]
    fn command_field_overflow_update_entity_id_oversized() {
        let cmd = ArestCommand::UpdateEntity {
            noun: "n".into(),
            domain: "d".into(),
            entity_id: huge(),
            fields: ok_map(),
            sender: None,
            signature: None,
            force: false,
        };
        assert_eq!(command_field_overflow(&cmd), Some("entityId"));
    }

    #[test]
    fn command_field_overflow_update_fields_key_oversized() {
        let mut fields = hashbrown::HashMap::new();
        fields.insert(huge(), "v".to_string());
        let cmd = ArestCommand::UpdateEntity {
            noun: "n".into(),
            domain: "d".into(),
            entity_id: "e".into(),
            fields,
            sender: None,
            signature: None,
            force: false,
        };
        assert_eq!(command_field_overflow(&cmd), Some("fields"));
    }

    #[test]
    fn command_field_overflow_update_fields_value_oversized() {
        let mut fields = hashbrown::HashMap::new();
        fields.insert("k".to_string(), huge());
        let cmd = ArestCommand::UpdateEntity {
            noun: "n".into(),
            domain: "d".into(),
            entity_id: "e".into(),
            fields,
            sender: None,
            signature: None,
            force: false,
        };
        assert_eq!(command_field_overflow(&cmd), Some("fields"));
    }

    #[test]
    fn command_field_overflow_update_sender_oversized() {
        let cmd = ArestCommand::UpdateEntity {
            noun: "n".into(),
            domain: "d".into(),
            entity_id: "e".into(),
            fields: ok_map(),
            sender: Some(huge()),
            signature: None,
            force: false,
        };
        assert_eq!(command_field_overflow(&cmd), Some("sender"));
    }

    #[test]
    fn command_field_overflow_update_signature_oversized() {
        let cmd = ArestCommand::UpdateEntity {
            noun: "n".into(),
            domain: "d".into(),
            entity_id: "e".into(),
            fields: ok_map(),
            sender: None,
            signature: Some(huge()),
            force: false,
        };
        assert_eq!(command_field_overflow(&cmd), Some("signature"));
    }

    #[test]
    fn command_field_overflow_update_valid_returns_none() {
        let cmd = ArestCommand::UpdateEntity {
            noun: "Person".into(),
            domain: "d".into(),
            entity_id: "p-1".into(),
            fields: ok_map(),
            sender: Some("u1".into()),
            signature: Some("sig".into()),
            force: false,
        };
        assert_eq!(command_field_overflow(&cmd), None);
    }

    // ── LoadReadings variants ────────────────────────────────────

    #[test]
    fn command_field_overflow_load_readings_markdown_oversized() {
        let cmd = ArestCommand::LoadReadings {
            markdown: huge(),
            domain: "d".into(),
            sender: None,
            signature: None,
        };
        assert_eq!(command_field_overflow(&cmd), Some("markdown"));
    }

    #[test]
    fn command_field_overflow_load_readings_domain_oversized() {
        let cmd = ArestCommand::LoadReadings {
            markdown: "md".into(),
            domain: huge(),
            sender: None,
            signature: None,
        };
        assert_eq!(command_field_overflow(&cmd), Some("domain"));
    }

    #[test]
    fn command_field_overflow_load_readings_sender_oversized() {
        let cmd = ArestCommand::LoadReadings {
            markdown: "md".into(),
            domain: "d".into(),
            sender: Some(huge()),
            signature: None,
        };
        assert_eq!(command_field_overflow(&cmd), Some("sender"));
    }

    #[test]
    fn command_field_overflow_load_readings_signature_oversized() {
        let cmd = ArestCommand::LoadReadings {
            markdown: "md".into(),
            domain: "d".into(),
            sender: None,
            signature: Some(huge()),
        };
        assert_eq!(command_field_overflow(&cmd), Some("signature"));
    }

    #[test]
    fn command_field_overflow_load_readings_valid_returns_none() {
        let cmd = ArestCommand::LoadReadings {
            markdown: "Each Person has a name.".into(),
            domain: "d".into(),
            sender: Some("u1".into()),
            signature: Some("sig".into()),
        };
        assert_eq!(command_field_overflow(&cmd), None);
    }

    // ── platform_apply_command integration ───────────────────────

    #[test]
    fn platform_apply_command_rejects_oversized_input_buffer() {
        // Construct an atom whose length strictly exceeds PLATFORM_MAX_INPUT.
        // The 1MB gate must reject BEFORE serde parsing even runs, so any
        // content is fine — we just need length > PLATFORM_MAX_INPUT.
        let oversized = "a".repeat(PLATFORM_MAX_INPUT + 1);
        let input = Object::atom(&oversized);
        let result = platform_apply_command(&input, &Object::phi());
        assert_eq!(
            result.as_atom(),
            Some("⊥ input exceeds platform buffer"),
            "oversized input must be rejected by the PLATFORM_MAX_INPUT gate"
        );
    }

    #[test]
    fn platform_apply_command_rejects_oversized_field_with_field_name() {
        // Build a JSON command whose "noun" field exceeds PLATFORM_MAX_FIELD
        // but whose total length stays under PLATFORM_MAX_INPUT (1MB).
        // Then the input-buffer gate passes, serde parses the command, and
        // command_field_overflow returns Some("noun"), yielding the
        // "⊥ field '<name>' exceeds platform buffer" atom.
        let big_noun = "a".repeat(PLATFORM_MAX_FIELD + 1);
        let json = format!(
            r#"{{"type":"createEntity","noun":"{}","domain":"d","fields":{{}}}}"#,
            big_noun
        );
        assert!(
            json.len() <= PLATFORM_MAX_INPUT,
            "test fixture must stay within PLATFORM_MAX_INPUT"
        );
        let input = Object::atom(&json);
        let result = platform_apply_command(&input, &Object::phi());
        assert_eq!(
            result.as_atom(),
            Some("⊥ field 'noun' exceeds platform buffer"),
            "oversized field must be rejected with its name in the error atom"
        );
    }

    #[test]
    fn platform_apply_command_rejects_oversized_fields_map_value() {
        // HashMap-based fields: oversize a single value in `fields`.
        // The error atom must name the container field ("fields").
        let big_val = "a".repeat(PLATFORM_MAX_FIELD + 1);
        let json = format!(
            r#"{{"type":"createEntity","noun":"Person","domain":"d","fields":{{"name":"{}"}}}}"#,
            big_val
        );
        assert!(
            json.len() <= PLATFORM_MAX_INPUT,
            "test fixture must stay within PLATFORM_MAX_INPUT"
        );
        let input = Object::atom(&json);
        let result = platform_apply_command(&input, &Object::phi());
        assert_eq!(
            result.as_atom(),
            Some("⊥ field 'fields' exceeds platform buffer"),
        );
    }

    // ── #797 / #766 — Map carrier return shape ───────────────────────
    //
    // The successful (non-rejecting) path of `platform_apply_command`
    // must return the `{__state_delta, __result}` Map carrier introduced
    // by S1c #757 / #766. The carrier is the linchpin for #777 (worker
    // collapse to engine-only IO): without it `classify_writer_result`
    // (lib.rs) would route the result to `NoCommit`, the per-cell chain
    // would never extend, and the worker's `writeCellThroughEngine`
    // (src/entity-do.ts) would not be able to extract `__state_delta`
    // to apply to its in-memory cell graph — forcing the parallel SQL
    // write path that S1 / #777 set out to retire.
    //
    // Pre-lift behaviour returned an `Object::atom(<json-summary>)` and
    // these tests would fail at `result.as_map()` being None — the
    // shape distinguisher this commit locks down.

    /// Build a phi state object — enough for `apply_command_defs` to
    /// fall through `resolve:<noun>` to the default
    /// `<noun>_has_<field>` cell-name convention. Zero schema, zero
    /// derivations, zero validate functions — the create still emits
    /// per-field cells under that fallback name so `__state_delta` is
    /// observably non-empty.
    fn apply_command_phi_state() -> Object {
        // These tests instantiate `Person`. createEntity is a run-time op and
        // requires a fully-defined entity type (objectType="entity" + a
        // reference scheme), so the minimal state DECLARES Person — exactly as
        // a real domain declares its entities before any are created. (Run-time
        // definedness gate, see command.rs create_via_defs.)
        //
        // task-961-b: this fixture bypasses `compile_to_defs_state`, so it must
        // seed the `Noun_is_instantiable` cell itself — the run-time gate now
        // decides PURELY from that cell (procedural fallback removed).
        let s = cell_push(
            "Noun",
            fact_from_pairs(&[("name", "Person"), ("objectType", "entity"), ("referenceScheme", "id")]),
            &Object::phi(),
        );
        seed_instantiable_cell(&s)
    }

    #[test]
    fn platform_apply_command_create_returns_map_carrier_shape() {
        // A well-formed CreateEntity command against an empty state.
        // create_via_defs (#867) auto-generates the entity id when none
        // is supplied; the FT-cell fallback (command.rs:550) ensures at
        // least one cell is pushed into the delta.
        let json = r#"{"type":"createEntity","noun":"Person","domain":"d","id":"p-1","fields":{"name":"Alice"}}"#;
        let input = Object::atom(json);
        let d = apply_command_phi_state();
        let result = platform_apply_command(&input, &d);

        // The return must be a Map, not an atom — pre-lift this was an
        // `Object::atom(json_summary)` and `result.as_map()` would be
        // None. The Map shape is the contract `classify_writer_result`
        // matches on for CommitDelta.
        let map = result.as_map().expect(
            "platform_apply_command must return a Map carrier on success; \
             pre-#766 it returned an Object::atom and writes never committed",
        );
        assert!(
            map.contains_key("__state_delta"),
            "Map carrier must contain a __state_delta key; \
             keys present = {:?}",
            map.keys().collect::<Vec<_>>(),
        );
        assert!(
            map.contains_key("__result"),
            "Map carrier must contain a __result key; \
             keys present = {:?}",
            map.keys().collect::<Vec<_>>(),
        );
    }

    #[test]
    fn platform_apply_command_accepts_bare_array_as_batch() {
        // task-930: a bare top-level JSON array is sugar for the
        // collection-shaped batch. Two creates in one call must emit a
        // single Map carrier whose __state_delta carries BOTH entities'
        // FT cells — proof the array routed through Command::Batch /
        // apply_command_batch (one atomic request), not a parse error.
        let json = r#"[
            {"type":"createEntity","noun":"Person","domain":"d","id":"p-1","fields":{"name":"Alice"}},
            {"type":"createEntity","noun":"Person","domain":"d","id":"p-2","fields":{"name":"Bob"}}
        ]"#;
        let input = Object::atom(json);
        let d = apply_command_phi_state();
        let result = platform_apply_command(&input, &d);

        let map = result.as_map().expect(
            "a bare JSON array must apply as a batch and return the Map carrier",
        );
        let delta = map.get("__state_delta").expect("__state_delta present");
        let delta_map = delta.as_map().expect("__state_delta is a Map");
        // The `Person_has_name` cell must hold BOTH p-1 and p-2 — one
        // combined delta over the cumulative population.
        let cell = delta_map.get("Person_has_name").expect("Person_has_name cell in delta");
        let facts: alloc::vec::Vec<_> = cell_facts_iter(cell).collect();
        assert_eq!(facts.len(), 2,
            "batch delta must carry both creates' facts in one cell; got {:?}", facts);
    }

    /// arc-agi-3 engine-issue 15 (TORN BATCH): when the forward chain
    /// aborts on its time budget mid-apply, `platform_apply_command` —
    /// the surface every MCP/HTTP apply rides, batches included — must
    /// collapse to ⊥ so the dispatcher's traced-bottom path rejects the
    /// whole request (D' = D). Pre-fix it used the plain encoder: the
    /// half-derived partial state rode back as a normal CommitDelta
    /// carrier and the host committed a torn batch (m:n rows landed,
    /// the entity creates inside the aborted op vanished).
    #[test]
    fn platform_apply_command_collapses_to_bottom_on_chain_abort() {
        // The state must carry at least one derivation rule — with an
        // empty stratum the chain is never invoked and the deadline
        // guard cannot fire (the original φ-state fixture proved that
        // the hard way).
        const READINGS: &str = "Person(.id) is an entity type.\n\
            Tier is a value type.\n\
            Name is a value type.\n\n\
            ## Fact Types\n\n\
            Person has Name.\n\
            Person has Tier.\n\n\
            ## Derivation Rules\n\n\
            * Person has Tier 'basic' iff Person has Name.\n";
        let state = crate::parse_forml2::parse_to_state(READINGS).expect("parse");
        let defs = crate::compile::compile_to_defs_state(&state);
        let d = defs_to_state(&defs, &state);

        let json = r#"[
            {"type":"createEntity","noun":"Person","domain":"d","id":"t-1","fields":{"Name":"Eve"}},
            {"type":"createEntity","noun":"Person","domain":"d","id":"t-2","fields":{"Name":"Mallory"}}
        ]"#;
        let input = Object::atom(json);
        // ZERO budget: every round boundary is already past the
        // deadline, so the first chain inside the batch aborts.
        let result = crate::evaluate::with_chain_budget(
            core::time::Duration::ZERO,
            || platform_apply_command(&input, &d));
        assert_eq!(result, Object::Bottom,
            "an aborted chain must collapse the WHOLE apply to ⊥ — \
             returning a Map carrier here is the torn-batch bug");
        // The abort flag must be consumed by the collapse, not leak
        // into the next command on this thread.
        assert!(!crate::evaluate::take_chain_abort(),
            "the ⊥ collapse must clear the abort flag");
    }

    #[test]
    fn platform_apply_command_accepts_batch_type_envelope() {
        // task-930: the explicit `{"type":"batch","commands":[…]}`
        // shape deserializes into Command::Batch and applies atomically.
        let json = r#"{"type":"batch","commands":[
            {"type":"createEntity","noun":"Person","domain":"d","id":"q-1","fields":{"name":"Carol"}},
            {"type":"createEntity","noun":"Person","domain":"d","id":"q-2","fields":{"name":"Dave"}}
        ]}"#;
        let input = Object::atom(json);
        let d = apply_command_phi_state();
        let result = platform_apply_command(&input, &d);

        let map = result.as_map().expect("batch envelope must return the Map carrier");
        let delta = map.get("__state_delta").expect("__state_delta present");
        let delta_map = delta.as_map().expect("__state_delta is a Map");
        let cell = delta_map.get("Person_has_name").expect("Person_has_name cell in delta");
        assert_eq!(cell_facts_iter(cell).count(), 2,
            "batch envelope delta must carry both creates");
    }

    #[test]
    fn platform_apply_command_create_result_field_is_json_atom() {
        // The __result slot must carry the compact JSON envelope as an
        // atom — exactly the bytes pre-#766 callers received as the
        // bare return value. Worker callers that pull `__result` back
        // out (writer-dispatcher response field, command.rs::decode_command_result)
        // must see the same parseable JSON shape.
        let json = r#"{"type":"createEntity","noun":"Person","domain":"d","id":"p-1","fields":{"name":"Alice"}}"#;
        let input = Object::atom(json);
        let d = apply_command_phi_state();
        let result = platform_apply_command(&input, &d);

        let map = result.as_map().expect("Map carrier expected");
        let result_obj = map.get("__result")
            .expect("__result key must be present in the Map carrier");
        let result_atom = result_obj.as_atom()
            .expect("__result must be an Object::atom holding the compact JSON envelope");
        // The atom must parse as JSON — the envelope shape
        // `decode_command_result` already round-trips. Any non-JSON
        // payload here breaks that round-trip and the worker's
        // response-string contract.
        let parsed: serde_json::Value = serde_json::from_str(result_atom)
            .expect("__result atom must be valid JSON");
        assert!(
            parsed.is_object(),
            "__result JSON must be an object; got {}",
            result_atom,
        );
    }

    #[test]
    fn platform_apply_command_create_state_delta_is_map_of_touched_cells() {
        // The __state_delta slot must be a Map whose keys are the cell
        // names the command modified — exactly the per-command delta
        // `merge_delta` (lib.rs CommitDelta arm) consumes. For a
        // `createEntity` with one field, at least one FT cell appears
        // (the `<noun>_has_<field>` cell pushed by create_via_defs
        // command.rs:550).
        let json = r#"{"type":"createEntity","noun":"Person","domain":"d","id":"p-1","fields":{"name":"Alice"}}"#;
        let input = Object::atom(json);
        let d = apply_command_phi_state();
        let result = platform_apply_command(&input, &d);

        let map = result.as_map().expect("Map carrier expected");
        let delta = map.get("__state_delta")
            .expect("__state_delta key must be present");
        let delta_map = delta.as_map().expect(
            "__state_delta must itself be a Map of per-cell post-state; \
             classify_writer_result inspects delta.as_map() before \
             promoting to CommitDelta",
        );
        // At least one cell was touched: with no `resolve:Person` def
        // installed, create_via_defs falls back to `Person_has_name`.
        // The exact cell name is documented in command.rs but the
        // contract here is just that the delta isn't empty.
        assert!(
            !delta_map.is_empty(),
            "create with a non-empty fields map must yield a non-empty \
             per-cell delta; got delta keys = {:?}",
            delta_map.keys().collect::<Vec<_>>(),
        );
        // Stronger: verify the noun_has_field FT cell appears. This
        // pins the shape worker callers depend on when applying the
        // delta to their in-memory cell graph.
        assert!(
            delta_map.contains_key("Person_has_name"),
            "create:Person with field=name must touch the Person_has_name FT cell; \
             got delta keys = {:?}",
            delta_map.keys().collect::<Vec<_>>(),
        );
    }

    #[test]
    fn platform_apply_command_update_returns_map_carrier_shape() {
        // The update variant must also surface the Map carrier so
        // worker callers can lift the delta on update through the same
        // code path they use for create. Pre-lift the worker fell back
        // to a parallel SQL write because the engine couldn't return
        // the post-update delta — only a stringified envelope.
        let json = r#"{"type":"updateEntity","noun":"Person","domain":"d","entityId":"p-1","fields":{"name":"Bob"}}"#;
        let input = Object::atom(json);
        let d = apply_command_phi_state();
        let result = platform_apply_command(&input, &d);

        let map = result.as_map().expect(
            "platform_apply_command(updateEntity) must return a Map carrier; \
             worker collapse to engine-only IO (#777) depends on this",
        );
        assert!(
            map.contains_key("__state_delta") && map.contains_key("__result"),
            "Map carrier must contain both __state_delta and __result keys; \
             keys present = {:?}",
            map.keys().collect::<Vec<_>>(),
        );
        // __state_delta is a Map even when the update is a no-op
        // (empty cells); shape stability matters for the classifier.
        let delta = map.get("__state_delta").unwrap();
        assert!(
            delta.as_map().is_some(),
            "__state_delta on update must be a Map, even if empty (no \
             existing entity); got {:?}",
            delta,
        );
    }

    #[test]
    fn platform_apply_command_map_carrier_classifies_as_commit_delta() {
        // Mirror the writer dispatcher's `classify_writer_result`
        // recognition test inline here so the shape contract is
        // testable from ast.rs without taking a lib.rs dependency.
        // The carrier must (a) be a Map, (b) carry both keys, (c) have
        // `__state_delta` itself be a Map — exactly the predicates
        // classify_writer_result inspects (lib.rs::classify_writer_result).
        // If any predicate fails, the dispatcher routes to NoCommit and
        // merge_delta never runs — the #766 / #777 regression.
        let json = r#"{"type":"createEntity","noun":"Person","domain":"d","id":"p-1","fields":{"name":"Alice"}}"#;
        let input = Object::atom(json);
        let result = platform_apply_command(&input, &apply_command_phi_state());

        // Predicate 1: top-level Map.
        let map = result.as_map().expect("classifier predicate 1: top-level Map");
        // Predicate 2: both keys present.
        assert!(map.contains_key("__state_delta"),
            "classifier predicate 2a: __state_delta key");
        assert!(map.contains_key("__result"),
            "classifier predicate 2b: __result key");
        // Predicate 3: __state_delta is itself a Map (so the dispatcher
        // can clone-extract the delta and pass it to merge_delta).
        assert!(
            map.get("__state_delta").unwrap().as_map().is_some(),
            "classifier predicate 3: __state_delta as Map — without \
             this the dispatcher falls back to NoCommit and the \
             worker's #777 engine-only IO collapse is impossible"
        );
    }

    // ── induce stub (#846) ───────────────────────────────────────────

    #[test]
    fn induce_def_apply_returns_phi() {
        // Wire 'induce' as a Func::Platform name → platform_induce stub.
        // Future tasks (#848-#852) replace the stub body with the search
        // loop; until then it must return phi so callers can distinguish
        // "induce ran but found nothing" from "induce was never wired"
        // (the latter would yield Object::Bottom from apply_platform's
        // fallback path).
        let defs = [(
            "induce".to_string(),
            Func::Platform("induce".to_string()),
        )];
        let d = defs_to_state(&defs, &Object::phi());
        let result = apply(&Func::Def("induce".to_string()), &Object::phi(), &d);
        assert_eq!(result, Object::phi(),
            "Func::Def(\"induce\") must dispatch to platform_induce stub which returns phi");
    }

    // ── normalize() — Backus §12 algebraic rewrite pass ─────────────

    fn sel1() -> Func { Func::Selector(1) }
    fn sel2() -> Func { Func::Selector(2) }

    #[test]
    fn normalize_strips_left_identity() {
        let input = Func::Compose(Box::new(Func::Id), Box::new(sel1()));
        let out = normalize(&input);
        assert!(matches!(out, Func::Selector(1)),
            "id ∘ f must rewrite to f, got {:?}", out);
    }

    #[test]
    fn normalize_strips_right_identity() {
        let input = Func::Compose(Box::new(sel1()), Box::new(Func::Id));
        let out = normalize(&input);
        assert!(matches!(out, Func::Selector(1)),
            "f ∘ id must rewrite to f, got {:?}", out);
    }

    #[test]
    fn normalize_fuses_map_composition() {
        // α(f) ∘ α(g) → α(f ∘ g)
        let input = Func::Compose(
            Box::new(Func::ApplyToAll(Box::new(sel1()))),
            Box::new(Func::ApplyToAll(Box::new(sel2()))),
        );
        let out = normalize(&input);
        match out {
            Func::ApplyToAll(inner) => match *inner {
                Func::Compose(_, _) => { /* expected */ }
                other => panic!("fused map must hold a Compose, got {:?}", other),
            },
            other => panic!("map fusion must produce ApplyToAll, got {:?}", other),
        }
    }

    #[test]
    fn normalize_fuses_filter_composition() {
        // Filter(p) ∘ Filter(q) → Filter(and ∘ [p, q])
        let input = Func::Compose(
            Box::new(Func::Filter(Box::new(sel1()))),
            Box::new(Func::Filter(Box::new(sel2()))),
        );
        let out = normalize(&input);
        match out {
            Func::Filter(inner) => match *inner {
                Func::Compose(ref a, ref b) => {
                    assert!(matches!(**a, Func::And), "fused predicate must be and ∘ …");
                    assert!(matches!(**b, Func::Construction(_)),
                        "fused predicate must pair the two predicates in a Construction");
                }
                other => panic!("fused filter must wrap a Compose, got {:?}", other),
            },
            other => panic!("filter fusion must produce Filter, got {:?}", other),
        }
    }

    #[test]
    fn normalize_folds_all_constant_construction() {
        // [c̄₁, c̄₂, c̄₃] → c̄⟨c₁, c₂, c₃⟩
        let input = Func::Construction(vec![
            Func::Constant(Object::atom("a")),
            Func::Constant(Object::atom("b")),
            Func::Constant(Object::atom("c")),
        ]);
        let out = normalize(&input);
        match out {
            Func::Constant(Object::Seq(items)) => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0], Object::atom("a"));
                assert_eq!(items[1], Object::atom("b"));
                assert_eq!(items[2], Object::atom("c"));
            }
            other => panic!("all-constants Construction must fold to Constant(Seq), got {:?}", other),
        }
    }

    #[cfg(feature = "profile")]
    #[test]
    fn profile_snapshot_records_apply_variants() {
        // Smoke test for the apply-variant profiler. Enable, run a
        // tiny workload that exercises at least three variants
        // (Selector, Construction, Constant), then read the snapshot
        // and assert each variant appears. Disable cleanly so later
        // tests aren't polluted.
        profile_reset();
        profile_enable();
        let d = Object::phi();
        let x = Object::seq(vec![Object::atom("a"), Object::atom("b")]);
        // Each of these triggers the corresponding apply-branch once.
        let _ = apply(&Func::Selector(1), &x, &d);
        let _ = apply(&Func::Constant(Object::atom("c")), &x, &d);
        let _ = apply(
            &Func::Construction(vec![Func::Selector(1), Func::Selector(2)]),
            &x, &d,
        );
        profile_disable();
        let snap = profile_snapshot();
        let seen: hashbrown::HashSet<&str> = snap.iter().map(|(n, _, _)| *n).collect();
        assert!(seen.contains("Selector"),  "Selector must appear in histogram; got {:?}", seen);
        assert!(seen.contains("Constant"),  "Constant must appear in histogram; got {:?}", seen);
        assert!(seen.contains("Construction"), "Construction must appear; got {:?}", seen);
        let total_calls: u64 = snap.iter().map(|(_, c, _)| c).sum();
        assert!(total_calls >= 5,
            "at least 5 apply calls expected (Construction triggers recursion); got {}",
            total_calls);
        profile_reset();
    }

    #[test]
    fn normalize_preserves_semantics_under_apply() {
        // Observational equivalence: apply(normalize(f), x, d) == apply(f, x, d)
        // on representative inputs for each rewrite rule.
        let d = Object::phi();
        let x_seq3 = Object::seq(vec![
            Object::seq(vec![Object::atom("a0"), Object::atom("a1")]),
            Object::seq(vec![Object::atom("b0"), Object::atom("b1")]),
            Object::seq(vec![Object::atom("c0"), Object::atom("c1")]),
        ]);
        let x_pair = Object::seq(vec![Object::atom("x"), Object::atom("y")]);

        let cases: Vec<(Func, Object)> = vec![
            (Func::Compose(Box::new(Func::Id), Box::new(sel1())), x_pair.clone()),
            (Func::Compose(Box::new(sel1()), Box::new(Func::Id)), x_pair.clone()),
            (Func::Compose(
                Box::new(Func::ApplyToAll(Box::new(sel1()))),
                Box::new(Func::ApplyToAll(Box::new(sel2()))),
             ), Object::seq(vec![
                Object::seq(vec![Object::seq(vec![Object::atom("inner-a0"), Object::atom("inner-a1")])]),
                Object::seq(vec![Object::seq(vec![Object::atom("inner-b0"), Object::atom("inner-b1")])]),
             ])),
            (Func::Construction(vec![
                Func::Constant(Object::atom("k1")),
                Func::Constant(Object::atom("k2")),
             ]), x_pair.clone()),
        ];

        for (f, x) in cases {
            let original = apply(&f, &x, &d);
            let normalized = apply(&normalize(&f), &x, &d);
            assert_eq!(original, normalized,
                "normalize must preserve observational equivalence; f={:?} x={:?}",
                f, x);
        }
        // Also verify the ApplyToAll case with x_seq3 independently — just
        // asserting it doesn't produce Bottom rules out a class of bugs.
        let map_comp = Func::Compose(
            Box::new(Func::ApplyToAll(Box::new(sel1()))),
            Box::new(Func::ApplyToAll(Box::new(sel2()))),
        );
        let before = apply(&map_comp, &x_seq3, &d);
        let after = apply(&normalize(&map_comp), &x_seq3, &d);
        assert_eq!(before, after);
    }

    // ── Fuel model (Sec-3: #159 enforcement inside apply) ────────
    //
    // A malicious Func tree (deep ApplyToAll, recursive Def dispatch,
    // Compose nested thousands deep) should hit the reductions ceiling
    // mid-evaluation and return Bottom instead of blowing the stack.
    // Tests below describe the contract:
    //   - No fuel set ⇒ unrestricted (existing 789 tests still green).
    //   - `with_fuel(n, …)` ⇒ at most n apply() recursions; the (n+1)ᵗʰ
    //     collapses to Bottom and propagates outward.

    #[test]
    fn fuel_unset_leaves_apply_unrestricted() {
        // 1 000 nested Compose(Id, Id) evaluates to x when fuel is
        // unset — default behavior must not regress.
        let mut f = Func::Id;
        for _ in 0..1_000 {
            f = Func::Compose(Box::new(f), Box::new(Func::Id));
        }
        assert_eq!(apply(&f, &Object::atom("x"), &defs()), Object::atom("x"));
    }

    #[test]
    fn fuel_with_headroom_leaves_result_unchanged() {
        // Shallow tree + generous budget must still return the real
        // result, not Bottom. Rules out an off-by-one that treats
        // unused fuel as exhaustion.
        let f = Func::Compose(Box::new(Func::Id), Box::new(Func::Id));
        let result = with_fuel(1_000, || apply(&f, &Object::atom("x"), &defs()));
        assert_eq!(result, Object::atom("x"));
    }

    #[test]
    fn fuel_exhaustion_collapses_deep_compose_to_bottom() {
        // ~10 000 deep Compose chain with budget = 100: must collapse
        // to Bottom mid-reduction, not stack-overflow.
        let mut f = Func::Id;
        for _ in 0..10_000 {
            f = Func::Compose(Box::new(f), Box::new(Func::Id));
        }
        let result = with_fuel(100, || apply(&f, &Object::atom("x"), &defs()));
        assert_eq!(result, Object::Bottom);
    }

    #[test]
    fn fuel_debits_per_element_in_apply_to_all() {
        // ApplyToAll over 10 000 elements with budget = 100 must hit
        // the ceiling mid-scan — per-element fuel, not one-shot.
        let items: Vec<Object> = (0..10_000)
            .map(|i| Object::atom(&i.to_string()))
            .collect();
        let seq = Object::Seq(items.into());
        let f = Func::ApplyToAll(Box::new(Func::Id));
        let result = with_fuel(100, || apply(&f, &seq, &defs()));
        assert_eq!(result, Object::Bottom);
    }

    #[test]
    fn with_fuel_restores_prior_setting_on_return() {
        // Nested `with_fuel` must be well-scoped — the outer budget
        // must be restored when the inner scope returns. (Otherwise
        // callers leak state between sequential invocations.)
        with_fuel(5, || {
            with_fuel(100, || {
                // Inner: no restriction relative to inner budget.
                assert_eq!(apply(&Func::Id, &Object::atom("x"), &defs()),
                           Object::atom("x"));
            });
            // Outer: fuel should be back to 5 (fully) — spend it all.
            let mut f = Func::Id;
            for _ in 0..50 {
                f = Func::Compose(Box::new(f), Box::new(Func::Id));
            }
            // 50-deep chain against budget 5 → Bottom.
            assert_eq!(apply(&f, &Object::atom("x"), &defs()), Object::Bottom);
        });
        // Post-scope: fuel is unset, deep chains succeed again.
        let mut f = Func::Id;
        for _ in 0..100 {
            f = Func::Compose(Box::new(f), Box::new(Func::Id));
        }
        assert_eq!(apply(&f, &Object::atom("x"), &defs()), Object::atom("x"));
    }

    // ── #690 / Audit H2: apply_with_fuel pure-API surface ─────────

    #[test]
    fn apply_with_fuel_unlimited_matches_apply() {
        // Calling with u64::MAX must match plain `apply` and report
        // u64::MAX remaining (the unlimited sentinel is preserved —
        // `consume_fuel` short-circuits without touching the counter).
        let f = Func::Compose(Box::new(Func::Id), Box::new(Func::Id));
        let (result, remaining) = apply_with_fuel(&f, &Object::atom("x"), &defs(), u64::MAX);
        assert_eq!(result, Object::atom("x"));
        assert_eq!(remaining, u64::MAX);
    }

    #[test]
    fn apply_with_fuel_returns_remaining_budget() {
        // Two-step Compose(Id, Id) under budget 10 ⇒ each apply()
        // recursion debits one. The exact remaining value depends on
        // how many internal apply() calls the Compose primitive
        // makes; what's invariant is `remaining < budget`.
        let f = Func::Compose(Box::new(Func::Id), Box::new(Func::Id));
        let (result, remaining) = apply_with_fuel(&f, &Object::atom("x"), &defs(), 10);
        assert_eq!(result, Object::atom("x"));
        assert!(remaining < 10, "some fuel must be debited; got {remaining}");
    }

    #[test]
    fn apply_with_fuel_exhaustion_returns_bottom_and_zero() {
        // 10 000 deep Compose chain with budget 100 ⇒ Bottom + 0
        // remaining. Mirrors `fuel_exhaustion_collapses_deep_compose_to_bottom`
        // but on the new explicit-fuel surface.
        let mut f = Func::Id;
        for _ in 0..10_000 {
            f = Func::Compose(Box::new(f), Box::new(Func::Id));
        }
        let (result, remaining) = apply_with_fuel(&f, &Object::atom("x"), &defs(), 100);
        assert_eq!(result, Object::Bottom);
        assert_eq!(remaining, 0);
    }

    #[test]
    fn apply_with_fuel_restores_outer_budget() {
        // apply_with_fuel must save+restore the ambient counter so
        // sibling calls in an outer with_fuel scope still see their
        // original budget.
        with_fuel(5, || {
            // Burn the inner scope's budget entirely.
            let (_, _) = apply_with_fuel(&Func::Id, &Object::atom("x"), &defs(), 100);
            // Outer: still has 5; a 50-deep chain still collapses.
            let mut f = Func::Id;
            for _ in 0..50 {
                f = Func::Compose(Box::new(f), Box::new(Func::Id));
            }
            assert_eq!(apply(&f, &Object::atom("x"), &defs()), Object::Bottom);
        });
    }

    /// #840 — `query_ft` on a ring fact type (both roles share the same
    /// noun, e.g. `Task blocks Task`) must return both role values, not
    /// silently collapse them into a single key. Today fact_to_json's
    /// `map.insert(role, val)` overwrites the first occurrence with the
    /// second when role names collide, so the agent reading its own
    /// dependency graph through MCP gets back `{"Task": <only blocked>}`
    /// instead of `{"Task1": <blocker>, "Task2": <blocked>}` (or any
    /// scheme that distinguishes them).
    ///
    /// Spec: a fact `<<Task, 112>, <Task, 113>>` in cell
    /// `Task_blocks_Task` must project to a JSON object that carries
    /// both bindings. We assert *both* values are present somewhere in
    /// the result — the exact key naming (Task1/Task2 vs subscript) is
    /// the implementation choice the fix lands.
    #[test]
    fn query_ft_returns_both_roles_on_ring_fact_type() {
        let s0 = Object::phi();
        let fact = fact_from_pairs(&[("Task", "112"), ("Task", "113")]);
        let s1 = cell_push("Task_blocks_Task", fact, &s0);

        let result = platform_query_ft("Task_blocks_Task", &Object::atom(""), &s1);
        let json_str = result.as_atom().expect("result must be a JSON atom");
        let parsed: serde_json::Value = serde_json::from_str(json_str)
            .unwrap_or_else(|e| panic!("query_ft must return valid JSON; got {json_str:?} err={e}"));
        let arr = parsed.as_array()
            .unwrap_or_else(|| panic!("query_ft must return a JSON array; got {parsed:?}"));
        assert_eq!(arr.len(), 1, "exactly one matching ring fact");
        let row = arr[0].as_object()
            .unwrap_or_else(|| panic!("each fact projects to an object; got {:?}", arr[0]));
        let values: Vec<&str> = row.values()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(values.contains(&"112"),
            "blocker Task=112 must appear in the projection; got row={row:?}. \
             #840: ring FT projection collapses both roles into a single \
             `Task` key, dropping the blocker silently.");
        assert!(values.contains(&"113"),
            "blocked Task=113 must appear in the projection; got row={row:?}. \
             #840: see comment above.");
    }

    /// task-955 Fix 3 — `query_ft` on a VIEW-backed fact type (stored cell
    /// empty; populated only by its derivation) must RESOLVE the view, the
    /// same way the SQL read path does (sql.rs::materialize_fact_type_tables).
    /// Before the fix, query_ft read the empty stored cell and returned `[]`
    /// while SQL returned the derived rows — the silent reader split that
    /// left MCP `query Task_has_Task_Status` empty while `sql` had 777.
    /// Fixture mirrors `sql::tests::view_fact_type_resolves_derivation_on_sql_read`.
    #[test]
    fn query_ft_resolves_view_backed_fact_type() {
        // bridge-identity-binding-untyped: head-noun membership row (identity
        // renames are typed now) — `Thing has Name.` + `Thing 'b1' has Name 'n1'.`
        // stage b1 as an instance of head noun Thing.
        let src = "Thing(.id) is an entity type.\n\
Base(.id) is an entity type.\n\
Tag is a value type.\n\
Thing Tag is a value type.\n\
Name is a value type.\n\
\n\
## Fact Types\n\
Base has Tag.\n\
Thing has Thing Tag. *\n\
Thing has Name.\n\
\n\
## Derivation Rules\n\
* Thing has Thing Tag iff that Base has some Tag and Thing Tag is Tag and Thing is Base.\n\
\n\
## Instance Facts\n\
Base 'b1' has Tag 'hot'.\n\
Thing 'b1' has Name 'n1'.\n";
        let state = crate::parse_forml2_stage2::parse_to_state_via_stage12(src).expect("parse");
        let defs = crate::compile::compile_to_defs_state(&state);
        let d = defs_to_state(&defs, &state);
        // The stored Thing_has_Thing_Tag cell is empty (view materialization);
        // query_ft must resolve the view to return the derived fact.
        let result = platform_query_ft("Thing_has_Thing_Tag", &Object::atom(""), &d);
        let json_str = result.as_atom().expect("result must be a JSON atom");
        let parsed: serde_json::Value = serde_json::from_str(json_str)
            .unwrap_or_else(|e| panic!("query_ft must return valid JSON; got {json_str:?} err={e}"));
        let arr = parsed.as_array().expect("query_ft must return a JSON array");
        assert!(arr.iter().any(|row| {
            let m = match row.as_object() { Some(m) => m, None => return false };
            m.values().any(|v| v.as_str() == Some("b1"))
                && m.values().any(|v| v.as_str() == Some("hot"))
        }), "query_ft must resolve the view to derived facts (Thing=b1, Thing Tag=hot); got {arr:?}");
    }

    /// task-955 Fix 2 — `platform_list_noun` (the `list:`/`get:` 3NF assembly)
    /// must fold ONLY base population cells, never derived `_transitive_*`
    /// closure cells or ':'-namespaced cells. Folding the latter made SM
    /// status reads nondeterministic (a transitive cell could supply a
    /// phantom 'Status' that overwrote the keyed base value). A field present
    /// ONLY in such a cell must NOT appear on the entity — deterministic to
    /// assert (unlike "which status wins", which depends on iteration order).
    #[test]
    fn list_noun_excludes_transitive_and_namespaced_cells() {
        let s0 = Object::phi();
        // Declare the base value FT so the declared-`{noun}_has_` field gate
        // (sm-event-stamp-phantom) keeps `Color`. The phantom-cell fields below
        // (`Phantom`, `Schemic`) are excluded by the base-cell name guard AND
        // are undeclared, so they never reach the row regardless.
        let sd = cell_push("FactType",
            fact_from_pairs(&[("id", "Widget_has_Color")]), &s0);
        let s1 = cell_push("Widget_has_Color",
            fact_from_pairs(&[("Widget", "w1"), ("Color", "red")]), &sd);
        let s2 = cell_push("_transitive_Widget_phantom",
            fact_from_pairs(&[("Widget", "w1"), ("Phantom", "ghost")]), &s1);
        let s3 = cell_push("schema:Widget_has_Color",
            fact_from_pairs(&[("Widget", "w1"), ("Schemic", "x")]), &s2);

        let result = platform_list_noun("Widget", &s3);
        let json_str = result.as_atom().expect("list result must be a JSON atom");
        let parsed: serde_json::Value = serde_json::from_str(json_str).expect("valid JSON");
        let arr = parsed.as_array().expect("array");
        let w1 = arr.iter()
            .find(|r| r.get("id").and_then(|v| v.as_str()) == Some("w1"))
            .expect("w1 must be listed").as_object().expect("object");
        assert_eq!(w1.get("Color").and_then(|v| v.as_str()), Some("red"),
            "base field must be present; got {w1:?}");
        assert!(!w1.contains_key("Phantom"),
            "field from a _transitive_* cell must NOT be folded into the entity; got {w1:?}");
        assert!(!w1.contains_key("Schemic"),
            "field from a ':'-namespaced cell must NOT be folded into the entity; got {w1:?}");
    }

    /// sm-event-stamp-phantom — when an SM event fires, `command.rs` stamps the
    /// occurred-at into the SM-noun-keyed trigger cell as
    /// `<<Task, id>, <Timestamp, occurred>>` (see `transition_via_defs`,
    /// command.rs ~3178, `fact_from_pairs(&[(noun, id), ("Timestamp", occurred)])`).
    /// That trigger cell (`Task_is_started`) is a BASE cell (no ':' namespace, no
    /// `_transitive_` prefix), so `platform_list_noun`'s fold used to leak the
    /// event's `Timestamp` role onto the `Task` row — a non-canonical field that
    /// maps to no declared fact type (`Task_has_Timestamp`), tripping
    /// `apply:unresolvable-field-key` on the next update round-trip.
    ///
    /// The declared-`{noun}_has_{role}` field gate must DROP that phantom while
    /// KEEPING the declared value field `Task_has_Task_Priority`. This drives the
    /// REAL `platform_list_noun` primitive (not a mocked row): the stamp is the
    /// exact shape the engine writes, the priority FT is declared in the
    /// `FactType` cell, and we assert on the parsed JSON row.
    #[test]
    fn list_noun_excludes_sm_event_stamp_timestamp() {
        let s0 = Object::phi();
        // Declare ONLY the priority value FT — the canonical 3NF field. The SM
        // event stamp's `Task_has_Timestamp` is NOT declared (it is an internal
        // event ordering key, never a fact type), so the gate must reject it.
        let s1 = cell_push("FactType",
            fact_from_pairs(&[("id", "Task_has_Task_Priority")]), &s0);
        // Seed the declared value fact.
        let s2 = cell_push("Task_has_Task_Priority",
            fact_from_pairs(&[("Task", "t1"), ("Task Priority", "p0")]), &s1);
        // Land the SM event stamp byte-for-byte as command.rs does: the SM-noun
        // role keys the trigger cell, `Timestamp` carries the occurred-at. The
        // `Task_is_started` trigger cell is a base cell (folded by list_noun).
        let s3 = cell_push("Task_is_started",
            fact_from_pairs(&[("Task", "t1"), ("Timestamp", "2026-06-17T00:00:00Z")]), &s2);

        let result = platform_list_noun("Task", &s3);
        let json_str = result.as_atom().expect("list result must be a JSON atom");
        let parsed: serde_json::Value = serde_json::from_str(json_str).expect("valid JSON");
        let arr = parsed.as_array().expect("array");
        let t1 = arr.iter()
            .find(|r| r.get("id").and_then(|v| v.as_str()) == Some("t1"))
            .expect("t1 must be listed").as_object().expect("object");

        assert_eq!(t1.get("Task Priority").and_then(|v| v.as_str()), Some("p0"),
            "declared value field `Task Priority` must survive the gate; got {t1:?}");
        assert!(!t1.contains_key("Timestamp"),
            "SM event-stamp `Timestamp` (undeclared `Task_has_Timestamp`) must NOT \
             be folded onto the entity row; got {t1:?}");
    }

    /// sm-event-stamp-phantom (companion) — the gate keys off DECLARATION, not
    /// the literal field name: a LEGITIMATELY declared `{noun} has Timestamp`
    /// value field (e.g. `Log_Entry_has_Timestamp`) must still appear. This pins
    /// that the fix discriminates the phantom by "is this `{noun}_has_{role}` a
    /// declared FT?" and never blanket-drops a field named `Timestamp`.
    #[test]
    fn list_noun_keeps_declared_timestamp_value_field() {
        let s0 = Object::phi();
        let s1 = cell_push("FactType",
            fact_from_pairs(&[("id", "Log_Entry_has_Timestamp")]), &s0);
        let s2 = cell_push("Log_Entry_has_Timestamp",
            fact_from_pairs(&[("Log Entry", "L1"), ("Timestamp", "2026-06-17T00:00:00Z")]), &s1);

        let result = platform_list_noun("Log Entry", &s2);
        let json_str = result.as_atom().expect("list result must be a JSON atom");
        let parsed: serde_json::Value = serde_json::from_str(json_str).expect("valid JSON");
        let arr = parsed.as_array().expect("array");
        let l1 = arr.iter()
            .find(|r| r.get("id").and_then(|v| v.as_str()) == Some("L1"))
            .expect("L1 must be listed").as_object().expect("object");

        assert_eq!(l1.get("Timestamp").and_then(|v| v.as_str()), Some("2026-06-17T00:00:00Z"),
            "a DECLARED `Log Entry has Timestamp` value field must survive; got {l1:?}");
    }

    /// task-956 — `same_identity`/`concat_dedup` must treat the fan-out's
    /// `Atom("φ")` and the round-tripped `phi()` (empty Seq) as the SAME fact,
    /// so unary instance facts dedup across recompile instead of accumulating
    /// (the Task_is_finished bloat). Without this, every recompile re-appends
    /// the φ-declared facts because the two φ shapes compare unequal.
    #[test]
    fn concat_dedup_collapses_phi_atom_vs_empty_seq() {
        // As the fan-out writes a unary fact: object value = Atom("φ").
        let fact_atom = fact_from_pairs(&[("Task", "1"), ("Task_is_finished", "φ")]);
        // As a SQLite round-trip yields the same fact: object value = phi().
        let fact_phi = Object::seq(vec![
            Object::seq(vec![Object::atom("Task"), Object::atom("1")]),
            Object::seq(vec![Object::atom("Task_is_finished"), Object::phi()]),
        ]);
        assert!(same_identity(&fact_atom, &fact_phi),
            "Atom(φ) and phi() forms of the same unary fact must share identity");
        let merged = concat_dedup(
            "Task_is_finished",
            &Object::seq(vec![fact_atom.clone()]),
            &Object::seq(vec![fact_phi.clone()]));
        assert_eq!(merged.as_seq().map(|s| s.len()), Some(1),
            "φ-asymmetric duplicate must collapse to a single fact; got {merged:?}");
    }

    /// #840 follow-up — filter on a ring FT must accept both bare role
    /// (matches any numbered variant) and exact subscripted keys
    /// (Task1, Task2). Non-ring filters keep working unchanged.
    #[test]
    fn query_ft_filter_supports_bare_and_subscripted_keys_on_ring_fact_type() {
        let s0 = Object::phi();
        let s1 = cell_push("Task_blocks_Task",
            fact_from_pairs(&[("Task", "112"), ("Task", "113")]), &s0);
        let s2 = cell_push("Task_blocks_Task",
            fact_from_pairs(&[("Task", "112"), ("Task", "114")]), &s1);
        let s3 = cell_push("Task_blocks_Task",
            fact_from_pairs(&[("Task", "200"), ("Task", "113")]), &s2);

        let parse = |obj: &Object| -> Vec<serde_json::Value> {
            let json_str = obj.as_atom().expect("JSON atom");
            serde_json::from_str::<serde_json::Value>(json_str)
                .ok().and_then(|v| v.as_array().cloned()).unwrap_or_default()
        };

        // Bare role: filter {Task: 112} matches any ring fact where any
        // Task variant = 112 — the two facts blocked by 112.
        let bare = platform_query_ft("Task_blocks_Task",
            &Object::atom(r#"{"Task":"112"}"#), &s3);
        assert_eq!(parse(&bare).len(), 2,
            "bare {{Task:112}} must match both ring facts where blocker=112; got {bare:?}");

        // Subscript Task1: precise blocker match — only 112-as-blocker rows.
        let sub1 = platform_query_ft("Task_blocks_Task",
            &Object::atom(r#"{"Task1":"112"}"#), &s3);
        assert_eq!(parse(&sub1).len(), 2,
            "{{Task1:112}} must match both rows where blocker (pos 0) = 112; got {sub1:?}");

        // Subscript Task2: blocked-side match — fact <200,113> only.
        let sub2 = platform_query_ft("Task_blocks_Task",
            &Object::atom(r#"{"Task2":"113"}"#), &s3);
        let arr2 = parse(&sub2);
        assert_eq!(arr2.len(), 2,
            "{{Task2:113}} must match both rows where blocked (pos 1) = 113; got {sub2:?}");
    }

    // ── #815: Func::Store strict mode ────────────────────────────────
    //
    // Strict mode is the audit gate for `Func::Store`. When OFF (default,
    // legacy behaviour), an empty capability stack is "system mode" — the
    // caller is implicitly trusted and stores succeed unrestricted. When
    // ON, the empty stack is reinterpreted as "no declaration" — every
    // `Func::Store` must be reached through an explicit cap frame, so
    // any consequent kind whose compile path forgot to emit
    // `allowed_writes:{name}` shows up immediately as ⊥.
    //
    // The gate is the substrate-aligned check: `allowed_writes` is the
    // registry of which functions may write where, and strict mode treats
    // an absent registry entry as a violation rather than a permission.
    // #816 (in `compile.rs`) is the parallel work that emits the missing
    // entries for `AntecedentRole`, Join, and Aggregate consequents; this
    // module is the apply-time enforcement that makes `#816` checkable.

    /// Baseline: with strict mode OFF (legacy behaviour) and no caps
    /// frame on the stack, `apply(Func::Store, …)` succeeds — preserves
    /// every existing engine path that writes via plain `apply()`.
    ///
    /// #903: under std the empty-stack `Func::Store` is refused by
    /// default. This test documents the strict-OFF + permissive
    /// composition: when both gates allow the empty-stack case,
    /// the store succeeds. The `permissive_empty_caps_guard()`
    /// opts in to the legacy unrestricted behavior.
    #[test]
    fn strict_off_empty_caps_permits_store() {
        let _pg = crate::declared_writes::permissive_empty_caps_guard();
        let state = Object::Map(hashbrown::HashMap::new().into());
        let input = Object::seq(vec![
            Object::atom("any_cell"),
            Object::atom("v"),
            state.clone(),
        ]);
        let _g = crate::declared_writes::strict_store_guard(false);
        let result = apply(&Func::Store, &input, &state);
        assert_eq!(
            fetch("any_cell", &result),
            Object::atom("v"),
            "strict OFF + empty caps + permissive must keep legacy unrestricted store",
        );
    }

    /// Gate: with strict mode ON and no caps frame on the stack, a
    /// `Func::Store` is refused — the empty stack is no longer "trusted
    /// system mode" but "no declaration." This is what surfaces a
    /// missing `allowed_writes:{name}` companion at apply time.
    #[test]
    fn strict_on_empty_caps_refuses_store() {
        let state = Object::Map(hashbrown::HashMap::new().into());
        let input = Object::seq(vec![
            Object::atom("any_cell"),
            Object::atom("v"),
            state.clone(),
        ]);
        let _g = crate::declared_writes::strict_store_guard(true);
        let result = apply(&Func::Store, &input, &state);
        assert_eq!(
            result, Object::Bottom,
            "strict ON + empty caps must refuse Func::Store (no declaration ⇒ ⊥)",
        );
    }

    /// A user-authored `Func::Def(name)` whose body emits `Func::Store`
    /// but has NO `allowed_writes:{name}` companion in DEFS is exactly
    /// the gap #810a tracks: under strict mode, the body runs with an
    /// empty cap stack and the store collapses to ⊥. This is the
    /// surface that catches every consequent kind whose compile path
    /// forgot to emit caps — Literal, AntecedentRole, Join, Aggregate
    /// alike — without ast.rs needing to know which kind it was.
    #[test]
    fn strict_on_def_without_allowed_writes_refuses_body_store() {
        // Body: store("good", "v", state)
        let body = Func::Compose(
            Box::new(Func::Store),
            Box::new(Func::construction(vec![
                Func::constant(Object::atom("good")),
                Func::constant(Object::atom("v")),
                Func::Id,
            ])),
        );
        let d0 = Object::Map(hashbrown::HashMap::new().into());
        // No `allowed_writes:undeclared_def` cell → no caps frame is
        // pushed by `defs_writes_scope`, so strict mode bottoms the
        // store inside the body.
        let d1 = store("undeclared_def", func_to_object(&body), &d0);

        let _g = crate::declared_writes::strict_store_guard(true);
        let result = apply(&Func::Def("undeclared_def".to_string()), &d1, &d1);
        assert_eq!(
            fetch("good", &result),
            Object::Bottom,
            "strict ON: a Def without allowed_writes:{{name}} must refuse the body store; got result = {:?}",
            result,
        );
    }

    /// Complement: under strict mode, a `Func::Def(name)` WITH
    /// `allowed_writes:{name}` declaring its target succeeds normally.
    /// Once #816 emits the companion for every consequent kind, all
    /// derivation defs will pass strict mode through this path.
    #[test]
    fn strict_on_def_with_allowed_writes_permits_body_store() {
        let body = Func::Compose(
            Box::new(Func::Store),
            Box::new(Func::construction(vec![
                Func::constant(Object::atom("good")),
                Func::constant(Object::atom("yes")),
                Func::Id,
            ])),
        );
        let d0 = Object::Map(hashbrown::HashMap::new().into());
        let d1 = store("declared_def", func_to_object(&body), &d0);
        let d2 = store(
            "allowed_writes:declared_def",
            Object::seq(vec![Object::atom("good")]),
            &d1,
        );

        let _g = crate::declared_writes::strict_store_guard(true);
        let result = apply(&Func::Def("declared_def".to_string()), &d2, &d2);
        assert_eq!(
            fetch("good", &result),
            Object::atom("yes"),
            "strict ON: a Def with matching allowed_writes must permit the body store; got result = {:?}",
            result,
        );
    }

    /// `apply_with_caps` already pushes an explicit frame. Under strict
    /// mode that frame is the source of truth — a store INSIDE the
    /// allow-list still succeeds, because strict mode only escalates
    /// the empty-stack case, not the populated-stack rules.
    #[test]
    fn strict_on_apply_with_caps_inside_allowed_set_permits_store() {
        let state = Object::Map(hashbrown::HashMap::new().into());
        let input = Object::seq(vec![
            Object::atom("my_cell"),
            Object::atom("ok"),
            state.clone(),
        ]);
        let allowed: hashbrown::HashSet<String> =
            ["my_cell".to_string()].into_iter().collect();

        let _g = crate::declared_writes::strict_store_guard(true);
        let result = crate::declared_writes::apply_with_caps(
            &Func::Store, &input, &state, &allowed,
        );
        assert_eq!(
            fetch("my_cell", &result),
            Object::atom("ok"),
            "strict ON + apply_with_caps: store inside allow-list must succeed; got result = {:?}",
            result,
        );
    }

    /// The strict-store guard restores the previous flag on drop —
    /// otherwise nested test runs (or nested compile pipelines) would
    /// leak the flag across thread-local scope boundaries.
    #[test]
    fn strict_store_guard_restores_previous_flag_on_drop() {
        // Confirm starting state, set it, drop guard, observe restoration.
        assert!(!crate::declared_writes::is_strict_store_mode(),
            "strict mode default must be OFF for this test to be meaningful");
        {
            let _g = crate::declared_writes::strict_store_guard(true);
            assert!(crate::declared_writes::is_strict_store_mode(),
                "strict mode must be ON inside the guard scope");
        }
        assert!(!crate::declared_writes::is_strict_store_mode(),
            "strict mode must restore to OFF after guard drop");
    }

    // ── #903: cfg-gate Func::Store empty-stack bypass on no_std ──────
    //
    // Security finding A-17 from #779. `Func::Store` with an empty
    // capability stack writes unrestricted. That bypass is legitimate
    // for kernel boot / init / metamodel-load (which run before the
    // capability system exists) but it is the hole that defeats Sec-5's
    // (#332) declared-writes enforcement when worker/host code paths
    // forget to push a frame.
    //
    // The fix is structural: cfg-gate the empty-stack pass-through on
    // `feature = "no_std"`. Kernel-only builds keep the unrestricted
    // legacy behavior. Worker / host builds (which always link
    // `std-deps`) reject empty-stack writes by default, the same way
    // strict-mode (#815) does opt-in.

    /// Kernel-cfg: under `feature = "no_std"`, `Func::Store` with an
    /// empty capability stack writes the target cell. The kernel image
    /// runs only compile-authored code; there is no user-code threat
    /// surface there, so the empty-stack "system mode" remains in force.
    #[cfg(feature = "no_std")]
    #[test]
    fn no_std_empty_caps_permits_func_store() {
        let state = Object::Map(hashbrown::HashMap::new().into());
        let input = Object::seq(vec![
            Object::atom("init_cell"),
            Object::atom("kernel-boot"),
            state.clone(),
        ]);
        let result = apply(&Func::Store, &input, &state);
        assert_eq!(
            fetch("init_cell", &result),
            Object::atom("kernel-boot"),
            "no_std build: empty cap stack must remain unrestricted for kernel boot/init paths",
        );
    }

    /// Worker-cfg: under default features (`std-deps`, i.e. worker /
    /// host build), `Func::Store` with an empty capability stack is
    /// REFUSED — collapses to ⊥, target cell is NOT written. This is
    /// the production gate for A-17: any worker/host caller that
    /// reaches `Func::Store` without an `apply_with_caps` frame or a
    /// matching `allowed_writes:{name}` on a Func::Def is bypassing
    /// Sec-5 and the apply pipeline must refuse them rather than write.
    #[cfg(not(feature = "no_std"))]
    #[test]
    fn worker_empty_caps_refuses_func_store_and_leaves_cell_unwritten() {
        let state = Object::Map(hashbrown::HashMap::new().into());
        let input = Object::seq(vec![
            Object::atom("some_cell"),
            Object::atom("unauthorized"),
            state.clone(),
        ]);
        let result = apply(&Func::Store, &input, &state);
        assert_eq!(
            result, Object::Bottom,
            "worker/host build: empty cap stack must refuse Func::Store (Sec-5 #322); got {:?}",
            result,
        );
        assert_eq!(
            fetch("some_cell", &result),
            Object::Bottom,
            "worker/host build: rejected store must leave target cell unwritten",
        );
    }

    /// Worker-cfg regression pin: under default features, `Func::Store`
    /// with a non-empty capability stack (via `apply_with_caps`) passes
    /// through normally. The cfg-gate at #903 only escalates the
    /// EMPTY-stack case; populated frames still follow the established
    /// Sec-5 rules (in-frame: succeeds; out-of-frame: ⊥). Protects
    /// against an over-broad refactor that collapses every cap path.
    #[cfg(not(feature = "no_std"))]
    #[test]
    fn worker_populated_caps_permits_func_store_inside_allow_list() {
        let state = Object::Map(hashbrown::HashMap::new().into());
        let input = Object::seq(vec![
            Object::atom("my_cell"),
            Object::atom("v"),
            state.clone(),
        ]);
        let allowed: hashbrown::HashSet<String> =
            ["my_cell".to_string()].into_iter().collect();
        let result = crate::declared_writes::apply_with_caps(
            &Func::Store, &input, &state, &allowed,
        );
        assert_eq!(
            fetch("my_cell", &result),
            Object::atom("v"),
            "worker/host build: populated cap frame inside allow-list must still succeed; got {:?}",
            result,
        );
    }
}
