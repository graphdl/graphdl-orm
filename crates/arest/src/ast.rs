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
// Skip-validate flag: set by CLI --no-validate to bypass constraint
// evaluation during bulk compile. Validation is O(constraints × population);
// per-fact-type indexing is available via the `validate:{fact_type_id}`
// defs produced by compile_to_defs_state. Bulk loads may still skip
// validation entirely when the readings are known-good.
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

#[cfg(not(feature = "no_std"))]
thread_local! {
    static SKIP_VALIDATE: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}
#[cfg(not(feature = "no_std"))]
pub fn set_skip_validate(on: bool) { SKIP_VALIDATE.with(|b| b.set(on)); }
#[cfg(not(feature = "no_std"))]
fn is_skip_validate() -> bool { SKIP_VALIDATE.with(|b| b.get()) }

#[cfg(feature = "no_std")]
static SKIP_VALIDATE_ATOMIC: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "no_std")]
pub fn set_skip_validate(on: bool) { SKIP_VALIDATE_ATOMIC.store(on, core::sync::atomic::Ordering::Relaxed); }
#[cfg(feature = "no_std")]
fn is_skip_validate() -> bool { SKIP_VALIDATE_ATOMIC.load(core::sync::atomic::Ordering::Relaxed) }

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
    Map(HashMap<String, Object>),

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
            Object::Map(m) => Some(m),
            _ => None,
        }
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
                Object::Map(map)
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

/// Split a string on commas, respecting nested <> brackets.
/// foldl over chars, accumulating (depth, start, splits).
fn split_top_level(s: &str) -> Vec<&str> {
    let (_, start, mut splits) = s.char_indices().fold((0i32, 0usize, vec![]), |(depth, start, mut acc), (i, c)| match c {
        '<' => (depth + 1, start, acc),
        '>' => (depth - 1, start, acc),
        ',' if depth == 0 => { acc.push(&s[start..i]); (depth, i + 1, acc) }
        _ => (depth, start, acc),
    });
    splits.push(&s[start..]);
    splits
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
        atom => Object::Atom(atom.to_string()),
    }
}

impl fmt::Display for Object {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Object::Atom(s) => write!(f, "{}", s),
            Object::Seq(items) if items.is_empty() => write!(f, "φ"),
            Object::Seq(items) => {
                write!(f, "<{}>", items.iter().map(|item| item.to_string())
                    .collect::<Vec<_>>().join(", "))
            }
            Object::Map(map) => {
                write!(f, "{{{}}}",
                    map.iter().map(|(k, v)| format!("{}={}", k, v))
                        .collect::<Vec<_>>().join(", "))
            }
            Object::Bottom => write!(f, "⊥"),
        }
    }
}

// ── State encoding for evaluation ────────────────────────────────────
// State = Object (sequence of cells). No Population struct.

// `types::Violation` is a serde-derived struct — gated out of no_std
// along with the `types` module itself. The three helpers below
// (encode/decode_violation, decode_violations) are only called by
// check / compile pipelines that are themselves std-only.
#[cfg(not(feature = "no_std"))]
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
            let fact_objs: Vec<Object> = contents.as_seq().map(|facts| {
                facts.iter().map(|fact| {
                    let bindings: Vec<Object> = fact.as_seq().map(|pairs| {
                        pairs.iter().cloned().collect::<Vec<Object>>()
                    }).unwrap_or_default();
                    Object::Seq(Arc::from(bindings))
                }).collect::<Vec<Object>>()
            }).unwrap_or_default();
            (ft_id.to_string(), Object::Seq(Arc::from(fact_objs)))
        }).collect();
    Object::Map(map)
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
            let fact_objs: Vec<Object> = contents.as_seq().map(|facts| {
                facts.iter().map(|fact| {
                    let bindings: Vec<Object> = fact.as_seq().map(|pairs| {
                        pairs.iter().map(|pair: &Object| pair.clone()).collect::<Vec<Object>>()
                    }).unwrap_or_default();
                    Object::Seq(Arc::from(bindings))
                }).collect::<Vec<Object>>()
            }).unwrap_or_default();
            Object::seq(vec![Object::atom(ft_id), Object::Seq(Arc::from(fact_objs))])
        }).collect();
    Object::Seq(fact_types.into())
}

/// Decode a violation Object back to a Violation struct.
/// Expected: <constraint_id, constraint_text, detail>
/// Decode a violation Object back to a Violation struct.
/// Expected: <constraint_id, constraint_text, detail>
/// Detail can be an atom (string) or a sequence of atoms (joined with spaces).
#[cfg(not(feature = "no_std"))]
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

/// Decode a sequence of violation Objects.
#[cfg(not(feature = "no_std"))]
pub fn decode_violations(obj: &Object) -> Vec<Violation> {
    match obj.as_seq() {
        Some(items) => items.iter().flat_map(|item|
            decode_violation(item).map_or_else(|| decode_violations(item), |v| vec![v])
        ).collect(),
        None => vec![],
    }
}

/// Encode a Violation as an Object.
#[cfg(not(feature = "no_std"))]
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
    /// For an empty sequence, /f:phi = Bottom (undefined).
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
    use core::hash::{BuildHasher, Hash, Hasher};
    let mut h = hashbrown::hash_map::DefaultHashBuilder::default().build_hasher();
    uri.hash(&mut h);
    authority_type.hash(&mut h);
    retrieval_date.hash(&mut h);
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
    let final_state = external_system
        .map(|ext| {
            cell_push_unique(
                "Citation_is_backed_by_External_System",
                fact_from_pairs(&[("Citation", &cite_id), ("External System", ext)]),
                &with_at,
            )
        })
        .unwrap_or(with_at);
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

#[cfg(not(feature = "no_std"))]
pub type PlatformFn = crate::sync::Arc<
    dyn Fn(&Object, &Object) -> Object + Send + Sync
>;

#[cfg(not(feature = "no_std"))]
static PLATFORM_FALLBACK: crate::sync::OnceLock<
    crate::sync::RwLock<HashMap<String, PlatformFn>>
> = crate::sync::OnceLock::new();

/// Install a synchronous Platform body. apply_platform falls through
/// here for names not covered by the hardcoded match. The body is an
/// `Arc<dyn Fn(&Object, &Object) -> Object>` — takes the operand and
/// the current `D`, returns an Object. Thread-safe; callers may
/// re-install to replace the body.
#[cfg(not(feature = "no_std"))]
pub fn install_platform_fn(name: &str, f: PlatformFn) {
    let reg = PLATFORM_FALLBACK.get_or_init(|| crate::sync::RwLock::new(HashMap::new()));
    reg.write().insert(name.to_string(), f);
}

/// Remove a previously-installed Platform body. Used by tests to
/// avoid leakage between test cases sharing process state.
#[cfg(not(feature = "no_std"))]
pub fn uninstall_platform_fn(name: &str) {
    if let Some(reg) = PLATFORM_FALLBACK.get() {
        reg.write().remove(name);
    }
}

/// Names the crate's production paths are permitted to install into
/// `PLATFORM_FALLBACK`. Empty by construction: no production path in
/// `arest` writes this registry today — see
/// `_reports/sec-2-platform-audit-2026-04-21.md`. A future writer
/// MUST add its name here and revise the audit; the integration test
/// `tests/sec_2_platform_fallback_audit.rs` fails otherwise.
#[cfg(not(feature = "no_std"))]
pub const APPROVED_PLATFORM_FN_NAMES: &[&str] = &[];

/// Sorted names currently installed in `PLATFORM_FALLBACK`. Empty
/// when the `OnceLock` has never been initialized. Used by the sec-2
/// guard test; also exposable for host-side introspection.
#[cfg(not(feature = "no_std"))]
pub fn installed_platform_fn_names() -> alloc::vec::Vec<alloc::string::String> {
    match PLATFORM_FALLBACK.get() {
        Some(reg) => {
            let mut v: alloc::vec::Vec<alloc::string::String> =
                reg.read().keys().cloned().collect();
            v.sort();
            v
        }
        None => alloc::vec::Vec::new(),
    }
}

#[cfg(not(feature = "no_std"))]
fn dispatch_platform_fallback(name: &str, x: &Object, d: &Object) -> Object {
    let reg = match PLATFORM_FALLBACK.get() {
        Some(r) => r,
        None => return Object::Bottom,
    };
    let maybe_f = reg.read().get(name).cloned();
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
    Object::Map(map)
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
            match x.as_seq() {
                Some(items) => Object::Atom(items.len().to_string()),
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
            match x.as_seq() {
                Some(items) if items.len() == 2 => {
                    let y = &items[0];
                    match items[1].as_seq() {
                        Some(zs) if zs.is_empty() => Object::phi(),
                        Some(zs) => Object::seq(
                            zs.iter().map(|z| Object::seq(vec![y.clone(), z.clone()])).collect()
                        ),
                        _ => Object::Bottom,
                    }
                }
                _ => Object::Bottom,
            }
        }

        Func::DistR => {
            match x.as_seq() {
                Some(items) if items.len() == 2 => {
                    let z = &items[1];
                    match items[0].as_seq() {
                        Some(ys) if ys.is_empty() => Object::phi(),
                        Some(ys) => Object::seq(
                            ys.iter().map(|y| Object::seq(vec![y.clone(), z.clone()])).collect()
                        ),
                        _ => Object::Bottom,
                    }
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
            match x.as_seq() {
                Some(items) if items.len() == 2 => match items[0].as_atom() {
                    Some(name) => fetch_or_phi(name, &items[1]),
                    None => Object::Bottom,
                },
                _ => Object::Bottom,
            }
        }

        Func::Fetch => {
            // fetch:<name, D> → contents of cell named name in D
            match x.as_seq() {
                Some(items) if items.len() == 2 => {
                    match items[0].as_atom() {
                        Some(name) => fetch(name, &items[1]),
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
            // protected metamodel set collapse to ⊥. Empty stack =
            // system mode = unrestricted (preserves legacy apply()).
            match x.as_seq() {
                Some(items) if items.len() == 3 => {
                    match items[0].as_atom() {
                        // declared_writes is now no_std-reachable (#565);
                        // its no_std shim returns true unconditionally
                        // (kernel runs only compile-authored code, no
                        // user-code threat surface), so this arm is a
                        // no-op there but enforces caps under std.
                        Some(name) if !crate::declared_writes::is_store_allowed(name) =>
                            Object::Bottom,
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
            match x.as_seq() {
                Some(items) if items.is_empty() => Object::phi(),
                Some(items) => {
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
                _ => Object::Bottom,
            }
        }

        Func::Insert(f) => {
            match x.as_seq() {
                Some(items) if items.len() == 1 => items[0].clone(),
                Some(items) if items.len() >= 2 => {
                    let rest = Object::Seq(items[1..].into());
                    let reduced = apply(&Func::Insert(f.clone()), &rest, d);
                    apply(f, &Object::seq(vec![items[0].clone(), reduced]), d)
                }
                _ => Object::Bottom,
            }
        }

        Func::Filter(p) => {
            match x.as_seq() {
                Some(items) if items.is_empty() => Object::phi(),
                Some(items) => {
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

        Func::Def(name) => {
            let def_obj = fetch(name, d);
            match def_obj {
                Object::Bottom => Object::Bottom,
                obj => {
                    // Sec-5: if DEFS contains `allowed_writes:{name}`
                    // as a Seq of atom cell names, scope the body under
                    // those caps. Absent cell = unrestricted (legacy
                    // behavior, preserves the established baseline).
                    // Now no_std-reachable (#565) — kernel build's
                    // shimmed `push_caps` is a no-op so this is safe.
                    let _caps_guard = defs_writes_scope(name, d);
                    apply(&metacompose(&obj, d), x, d)
                }
            }
        }

        // Platform primitives require serde_json + std modules; not available
        // in the no_std kernel build. Return Bottom so apply() stays total.
        #[cfg(not(feature = "no_std"))]
        Func::Platform(name) => apply_platform(name, x, d),
        #[cfg(feature = "no_std")]
        Func::Platform(_) => Object::Bottom,

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
        s if s.starts_with("create:") => platform_create(&s[7..], x, d),
        s if s.starts_with("update:") => platform_update(&s[7..], x, d),
        s if s.starts_with("transition:") => platform_transition(&s[11..], x, d),
        s if s.starts_with("list_noun:") => platform_list_noun(&s[10..], d),
        s if s.starts_with("get_noun:") => platform_get_noun(&s[9..], x, d),
        s if s.starts_with("query_ft:") => platform_query_ft(&s[9..], x, d),
        "audit" => platform_audit_log(d),
        // Fall through to the runtime-installed callback registry for
        // names outside the compile-derived range. See
        // `install_platform_fn` — hosts (ML scorer, local projector,
        // tests) install sync bodies here. Returns Bottom when no body
        // is installed, preserving total-function semantics.
        _ => dispatch_platform_fallback(name, x, d),
    }
}

/// Platform primitive: return the audit_log cell as a JSON array.
/// Key: "audit_log". Input: ignored. Each entry renders as
/// `{operation, outcome, sequence, sender, entity}`. Empty cell or
/// missing cell yields `[]` — never Bottom.
#[cfg(not(feature = "no_std"))]
fn platform_audit_log(d: &Object) -> Object {
    let log = fetch_or_phi("audit_log", d);
    let items: Vec<serde_json::Value> = log.as_seq()
        .map(|facts| facts.iter().filter_map(|fact| {
            let pairs = fact.as_seq()?;
            let mut map = serde_json::Map::new();
            pairs.iter().for_each(|pair| {
                if let Some(kv) = pair.as_seq() {
                    if let (Some(role), Some(val)) = (
                        kv.first().and_then(|k| k.as_atom()),
                        kv.get(1).and_then(|v| v.as_atom()),
                    ) {
                        map.insert(role.to_string(), serde_json::Value::String(val.to_string()));
                    }
                }
            });
            Some(serde_json::Value::Object(map))
        }).collect())
        .unwrap_or_default();
    let json = serde_json::to_string(&serde_json::Value::Array(items))
        .unwrap_or_else(|_| "[]".to_string());
    Object::atom(&json)
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

/// Platform primitive: signature verification (AREST §5.5).
/// Input: seq<atom, atom, atom> — (sender, payload, signature).
/// Output: atom("true"|"false"), or Object::Bottom on malformed input.
/// Wired through crate::crypto::verify_signature — currently a
/// DefaultHasher MAC placeholder; swap to HMAC-SHA256 when upgrading.
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
    fetch_or_phi("Noun", state)
        .as_seq()
        .map(|facts| facts.iter().any(|f| binding(f, "name") == Some(name)))
        .unwrap_or(false)
}

/// Find the first reserved metamodel noun that `parsed` declares AND that is
/// already present in `existing`. Returns None when the check passes (either
/// because the parsed state does not touch the metamodel namespace, or because
/// this is the bootstrap compile that legitimately owns the first declaration).
fn find_metamodel_shadow(parsed: &Object, existing: &Object) -> Option<String> {
    let parsed_nouns = fetch_or_phi("Noun", parsed);
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

    // SSRF defense (#25): External System federation must not reach
    // internal/loopback/link-local hosts, file:// URLs, or internal DNS.
    // Walk the parsed InstanceFact cell and reject any forbidden URL.
    match crate::parse_forml2::find_forbidden_instance_url(&parsed) {
        Some(url) => return Object::atom(&format!("⊥ forbidden URL in External System: {}", url)),
        None => {}
    }

    // Merge: foldl(concat_cell, D, cells(parsed))
    let merged_state = merge_states(d, &parsed);

    // Structural model validation (#48) — catch FORML2 violations.
    // Warnings only for now — pre-existing metamodel issues need cleanup first.
    let model_errors = crate::compile::validate_model_from_state(&merged_state);
    model_errors.iter().for_each(|e| { diag!("[model warning] {}", e); });

    // Compile defs from merged state + re-register platform primitives
    let mut defs = crate::compile::compile_to_defs_state(&merged_state);
    defs.push(("compile".to_string(), Func::Platform("compile".to_string())));
    defs.push(("apply".to_string(), Func::Platform("apply_command".to_string())));
    defs.push(("verify_signature".to_string(), Func::Platform("verify_signature".to_string())));
    defs.push(("audit".to_string(), Func::Platform("audit".to_string())));
    let new_d = defs_to_state(&defs, &merged_state);

    // Validate: ρ(validate) applied to merged state. Alethic violations reject.
    // Skipped when SKIP_VALIDATE is set (--no-validate flag for bulk compile).
    let decoded = match is_skip_validate() {
        true => vec![],
        false => {
            let ctx = encode_eval_context_state("", None, &merged_state);
            let violations = apply(&Func::Def("validate".to_string()), &ctx, &new_d);
            decode_violations(&violations)
        }
    };
    match decoded.iter().any(|v| v.alethic) {
        true => Object::atom(&format!("⊥ constraint violation: {}",
            decoded.iter().filter(|v| v.alethic).map(|v| v.constraint_text.as_str()).collect::<Vec<_>>().join("; "))),
        false => record_compile_event(&new_d, "compiled"),
    }
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
    let seq = fetch_or_phi("compile_history", state)
        .as_seq()
        .map(|items| items.len())
        .unwrap_or(0);
    let id = format!("compile-{}", seq);
    let fact = fact_from_pairs(&[
        ("Domain Change", id.as_str()),
        ("status", status),
    ]);
    let with_history = cell_push("compile_history", fact, state);
    record_audit(&with_history, "compile", status, None, None)
}

/// Security #26 — Audit trail for compile and apply operations.
///
/// Every `platform_compile` and `platform_apply_command` invocation appends
/// a fact to an `audit_log` cell: <operation, outcome, sequence, sender?>.
/// Sequence number is the current length of the cell, so the trace is
/// totally ordered and WASM-safe (no wall clock). Rejected operations
/// whose state is discarded by the host harness cannot persist their
/// audit entries; this is a known limitation tracked alongside the
/// reject-persistence semantics of platform_compile / platform_apply.
pub(crate) fn record_audit(
    state: &Object,
    operation: &str,
    outcome: &str,
    sender: Option<&str>,
    entity: Option<&str>,
) -> Object {
    let seq = fetch_or_phi("audit_log", state)
        .as_seq()
        .map(|items| items.len())
        .unwrap_or(0);
    let seq_str = seq.to_string();
    let sender_val = sender.unwrap_or("");
    let entity_val = entity.unwrap_or("");
    let fact = fact_from_pairs(&[
        ("operation", operation),
        ("outcome", outcome),
        ("sequence", seq_str.as_str()),
        ("sender", sender_val),
        ("entity", entity_val),
    ]);
    cell_push("audit_log", fact, state)
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
    let (command_json, population_str): (serde_json::Value, Option<String>) = if parsed.get("command").is_some() {
        // (b) — extract the inner command + population fields.
        let cmd = parsed.get("command").cloned().unwrap_or(serde_json::Value::Null);
        let pop = parsed.get("population")
            .and_then(|v| if v.is_string() { v.as_str().map(String::from) } else { Some(v.to_string()) });
        (cmd, pop)
    } else {
        (parsed, None)
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
    match serde_json::to_string(&result) {
        Ok(s) => Object::atom(&s),
        Err(e) => Object::atom(&format!("⊥ {}", e)),
    }
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
    let ft_cell = fetch_or_phi("FactType", d);
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
    crate::command::encode_command_result(&result)
}

/// Platform primitive: update entity from fact pairs.
/// Key: "update:{noun}". Input: <<id, val>, <field, val>, ...>.
#[cfg(not(feature = "no_std"))]
fn platform_update(noun: &str, x: &Object, d: &Object) -> Object {
    let (id, fields) = extract_fact_pairs(x);
    let entity_id = id.unwrap_or_default();
    let command = crate::command::Command::UpdateEntity {
        noun: noun.to_string(),
        domain: String::new(),
        entity_id,
        fields,
        sender: None,
        signature: None,
    };
    let result = crate::command::apply_command_defs(d, &command, d);
    crate::command::encode_command_result(&result)
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
    let status_key = "StateMachine_has_currentlyInStatus";
    let current_status = fetch_or_phi(status_key, d).as_seq()
        .and_then(|facts| facts.iter()
            .find(|f| binding_matches(f, "State Machine", &entity_id))
            .and_then(|f| binding(f, "currentlyInStatus").map(|s| s.to_string())));
    let command = crate::command::Command::Transition {
        entity_id,
        event,
        domain: String::new(),
        current_status,
        sender: None,
        signature: None,
    };
    let result = crate::command::apply_command_defs(d, &command, d);
    crate::command::encode_command_result(&result)
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

/// Platform primitive: list entities of a noun by reading D at apply-time.
/// Key: "list_noun:{noun}". Input: operand is ignored (may be empty).
///
/// Walks every fact cell in D. A fact contributes to an entity summary if
/// one of its role bindings has a role name equal to the target noun — the
/// role's value is the entity id. All other bindings on that fact become
/// field/value entries on the entity summary. Multiple facts about the same
/// entity merge; later facts overwrite earlier ones for the same field.
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
    let mut entities: HashMap<String, HashMap<String, String>> = HashMap::new();

    cells_iter(&d).iter().for_each(|(_, contents)| {
        let facts = contents.as_seq().map(|s| s.to_vec()).unwrap_or_default();
        facts.iter().for_each(|fact| {
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
                    (role != noun).then(|| entry.insert(role.to_string(), val.to_string()));
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
    let facts = fetch_or_phi(ft_id, &d);
    let facts_seq = facts.as_seq().map(|s| s.to_vec()).unwrap_or_default();

    let filter: hashbrown::HashMap<String, String> = x.as_atom()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.as_object().cloned())
        .map(|obj| obj.iter().filter_map(|(k, v)|
            v.as_str().map(|s| (k.clone(), s.to_string()))
        ).collect())
        .unwrap_or_default();

    let fact_to_json = |fact: &Object| -> Option<serde_json::Value> {
        let pairs = fact.as_seq()?;
        let mut map = serde_json::Map::new();
        pairs.iter().for_each(|pair| {
            if let Some(kv) = pair.as_seq() {
                if let (Some(role), Some(val)) = (
                    kv.first().and_then(|k| k.as_atom()),
                    kv.get(1).and_then(|v| v.as_atom()),
                ) {
                    map.insert(role.to_string(), serde_json::Value::String(val.to_string()));
                }
            }
        });
        Some(serde_json::Value::Object(map))
    };

    let matched: Vec<serde_json::Value> = facts_seq.iter()
        .filter_map(fact_to_json)
        .filter(|obj| {
            let m = match obj.as_object() { Some(m) => m, None => return false };
            filter.iter().all(|(k, v)|
                m.get(k).and_then(|val| val.as_str()) == Some(v.as_str())
            )
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
        Command::UpdateEntity { noun, domain, entity_id, fields, sender, signature } => {
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

/// Fetch (↑n): retrieve contents of the first cell named n from a store.
/// ↑n:D → c where D contains <CELL, n, c>
/// Returns bottom if no cell named n exists.
/// O(1) for Map stores, O(n) fallback for Seq stores.
pub fn fetch(name: &str, state: &Object) -> Object {
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

/// Store (↓n): replace or append cell named n with new contents.
/// ↓n:<x, D> → D' where D' has cell n with contents x.
/// If cell n exists, its contents are replaced. Otherwise a new cell is appended.
/// O(1) for Map stores, O(n) fallback for Seq stores.
pub fn store(name: &str, contents: Object, state: &Object) -> Object {
    match state {
        Object::Map(map) => {
            let mut new_map = map.clone();
            new_map.insert(name.to_string(), contents);
            Object::Map(new_map)
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
    let new_contents = match existing.as_seq() {
        Some(items) => {
            let mut v = items.to_vec();
            v.push(fact);
            Object::Seq(v.into())
        }
        None => Object::seq(vec![fact]),
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
        *entry = concat_dedup(entry, contents);
    });
    Object::Map(map)
}

/// Concatenate two sequences and drop duplicates, identity-aware.
/// Preserves first-occurrence order. When two facts share an identity
/// key (`id`, `name`, or `ruleId`), the first is kept and the second
/// dropped — this handles the case where one file declares a noun fully
/// and another references it, producing two Noun facts that differ in
/// bindings but represent the same entity.
fn concat_dedup(a: &Object, b: &Object) -> Object {
    let a_items: Vec<Object> = a.as_seq().map(|s| s.to_vec()).unwrap_or_default();
    let b_items: Vec<Object> = b.as_seq()
        .map(|s| s.to_vec())
        .unwrap_or_else(|| vec![b.clone()]);
    let mut out = a_items;
    for item in b_items {
        if out.iter().any(|existing| same_identity(existing, &item)) { continue; }
        out.push(item);
    }
    Object::Seq(out.into())
}

/// Two facts share identity when they have the same value at a canonical
/// identity binding (`id`, `name`, or `ruleId`), or — falling back —
/// when they are structurally equal.
fn same_identity(a: &Object, b: &Object) -> bool {
    if a == b { return true; }
    const IDENTITY_KEYS: &[&str] = &["id", "name", "ruleId", "Change Id", "Signal Id"];
    for key in IDENTITY_KEYS {
        let av = binding(a, key);
        let bv = binding(b, key);
        if let (Some(av), Some(bv)) = (av, bv) {
            return av == bv;
        }
    }
    false
}

/// Concatenate two sequences: <a₁,...,aₙ> ++ <b₁,...,bₘ> = <a₁,...,aₙ,b₁,...,bₘ>
/// Iterate all cells in state as (name, contents) pairs.
/// Replaces: population.facts.iter()
pub fn cells_iter(state: &Object) -> Vec<(&str, &Object)> {
    match state {
        Object::Map(map) => map.iter()
            .map(|(k, v)| (k.as_str(), v))
            .collect(),
        Object::Seq(cells) => cells.iter().filter_map(|c| {
            let items = c.as_seq()?;
            if items.len() == 3 && items[0].as_atom() == Some(CELL_TAG) {
                Some((items[1].as_atom()?, &items[2]))
            } else {
                None
            }
        }).collect(),
        _ => Vec::new(),
    }
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
    Object::Map(delta)
}

/// Merge a cell delta onto a base store. For each cell in `delta`,
/// overwrite the corresponding cell in `base`; other cells pass
/// through unchanged. Complement of `diff_cells`: for any (old, new),
/// `merge_delta(old, diff_cells(old, new)) == new` for the cells
/// present in new.
pub fn merge_delta(base: &Object, delta: &Object) -> Object {
    let base_map: HashMap<String, Object> = cells_iter(base).into_iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    let delta_map: HashMap<String, Object> = cells_iter(delta).into_iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    let mut merged = base_map;
    for (k, v) in delta_map { merged.insert(k, v); }
    Object::Map(merged)
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

/// Check if a named-tuple fact has a binding matching key=val.
/// Replaces: fact.bindings.iter().any(|(k, v)| k == key && v == val)
pub fn binding_matches(fact: &Object, key: &str, val: &str) -> bool {
    binding(fact, key) == Some(val)
}

/// Retain only facts in a cell that satisfy a predicate. Pure functional filter.
/// Replaces: instances.retain(|inst| predicate(inst))
pub fn cell_filter(name: &str, predicate: impl Fn(&Object) -> bool, state: &Object) -> Object {
    let existing = fetch_or_phi(name, state);
    let filtered = match existing.as_seq() {
        Some(items) => Object::Seq(items.iter().filter(|f| predicate(f)).cloned().collect()),
        None => Object::phi(),
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
    pairs.sort();
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
    let migrations = fetch_or_phi("Migration", state);
    let active_migrations: HashSet<String> = migrations.as_seq()
        .map(|s| s.iter().filter_map(|m| binding(m, "id").map(String::from)).collect())
        .unwrap_or_default();
    if active_migrations.is_empty() { return state.clone(); }
    let mas = fetch_or_phi("MigrationApplication", state);
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
        let Some(facts) = contents.as_seq() else { continue; };
        let filtered: Vec<Object> = facts.iter().filter(|f| {
            let fid = synthesize_fact_id(&cell_name, f);
            !hidden_ids.contains(&fid)
        }).cloned().collect();
        if filtered.len() != facts.len() {
            out = store(&cell_name, Object::Seq(filtered.into()), &out);
        }
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

#[allow(dead_code)] // reference implementations kept for docs; dispatch goes via Platform
fn _register_theta1_native_legacy(defs: &mut Vec<(String, Func)>) {
    // project: pi_L(R) = alpha([s_i1,...,s_ik]) : R
    // Takes <indices, R> and projects R onto those columns.
    // NATIVE because: indices are data that determine which Selectors to build.
    // A pure Func would require alpha(Construction(selectors)) but Construction
    // is a compile-time combinator -- the selector list is determined by the
    // index sequence at runtime.
    defs.push(("project".to_string(), Func::Native(Arc::new(|x: &Object| {
        // Monadic bind via ? on Option — Backus cond lifted into Option.
        // Any shape mismatch folds to None, then unwraps to Object::Bottom.
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
    }))));

    // join: join:<shared_col, R, S> = natural join on shared column index.
    // NATIVE because: shared_col is a runtime value that determines which
    // Selector to use for comparison and which columns to include in the
    // merged tuple. Pure Func cannot parameterize Selector indices from data.
    defs.push(("join".to_string(), Func::Native(Arc::new(|x: &Object| {
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
    }))));

    // tie: gamma(R) = Filter(eq . [sel(1), sel(n)]) : R
    // Selects tuples where first column = last column, then removes the last column.
    // NATIVE because: "last column" requires knowing the tuple arity n at runtime.
    // Backus's Selector(n) requires a fixed n at compile time. There is no
    // "select last element" primitive in FP. The Reverse+Selector(1) trick
    // works for comparison but the "remove last column" step still needs
    // dynamic-arity Construction to rebuild the tuple without its last element.
    defs.push(("tie".to_string(), Func::Native(Arc::new(|x: &Object| {
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
    }))));

    // compose_rel: R . S = pi_1s(R*S) -- relational composition.
    // Join R and S on shared column, then project out the shared column.
    // NATIVE because: inherits both join's dynamic column selection and
    // project's dynamic Construction building. The shared_col parameter
    // determines runtime behavior that cannot be fixed at compile time.
    defs.push(("compose_rel".to_string(), Func::Native(Arc::new(|x: &Object| {
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
                    .fold(Vec::new(), |mut acc, row| {
                        (!acc.contains(&row)).then(|| acc.push(row));
                        acc
                    });
                Some(Object::Seq(result.into()))
            })
            .unwrap_or(Object::Bottom)
    }))));
}

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
            Func::ApplyToAll(f) | Func::Insert(f) | Func::Filter(f) | Func::FoldL(f) => f.has_native(),
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

    fn defs() -> Object { Object::phi() }

    // ── Object construction ──────────────────────────────────────

    #[test]
    fn bottom_propagates_through_sequence() {
        let seq = Object::seq(vec![Object::atom("a"), Object::Bottom, Object::atom("c")]);
        assert_eq!(seq, Object::Bottom);
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
        let reconstructed = merge_delta(&old, &delta);
        for name in ["A", "B", "C", "D"] {
            assert_eq!(fetch_or_phi(name, &reconstructed), fetch_or_phi(name, &new),
                "cell {} must match after merge_delta(old, diff(old,new))", name);
        }
    }

    #[test]
    fn merge_delta_with_empty_delta_preserves_base() {
        let base = Object::seq(vec![
            cell("A", Object::atom("1")),
            cell("B", Object::atom("2")),
        ]);
        let empty_delta = Object::Map(HashMap::new());
        let merged = merge_delta(&base, &empty_delta);
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

    // ── Security #26: audit trail unit tests ─────────────────────
    //
    // Direct coverage of the `record_audit` helper that backs every
    // compile/apply audit push. Test the three shape invariants:
    //   1. on empty state, first entry gets sequence 0;
    //   2. on an N-entry state, the next entry gets sequence N;
    //   3. all four bindings (operation, outcome, sequence, sender)
    //      are present, with an omitted sender rendering as "".

    #[test]
    fn record_audit_appends_entry_with_sequence_zero_on_empty_state() {
        let state = Object::phi();
        let result = record_audit(&state, "compile", "compiled", Some("root@example"), None);
        let log = fetch_or_phi("audit_log", &result);
        let facts = log.as_seq().expect("audit_log should be a sequence");
        assert_eq!(facts.len(), 1, "empty state should yield exactly one audit entry");
        assert_eq!(binding(&facts[0], "operation"), Some("compile"));
        assert_eq!(binding(&facts[0], "outcome"), Some("compiled"));
        assert_eq!(binding(&facts[0], "sequence"), Some("0"));
        assert_eq!(binding(&facts[0], "sender"), Some("root@example"));
        assert_eq!(binding(&facts[0], "entity"), Some(""));
    }

    #[test]
    fn record_audit_next_entry_uses_cell_length_as_sequence() {
        // Pre-populate the audit_log with two arbitrary prior entries.
        let state = record_audit(&Object::phi(), "compile", "compiled", None, None);
        let state = record_audit(&state, "apply:create", "ok", Some("u1"), Some("ord-1"));
        // The third push must observe sequence = 2 (current cell length).
        let state = record_audit(&state, "apply:create", "rejected", Some("u2"), Some("ord-2"));
        let log = fetch_or_phi("audit_log", &state);
        let facts = log.as_seq().expect("audit_log should be a sequence");
        assert_eq!(facts.len(), 3, "three pushes should yield three entries");
        assert_eq!(binding(&facts[2], "operation"), Some("apply:create"));
        assert_eq!(binding(&facts[2], "outcome"), Some("rejected"));
        assert_eq!(binding(&facts[2], "sequence"), Some("2"));
        assert_eq!(binding(&facts[2], "sender"), Some("u2"));
        assert_eq!(binding(&facts[2], "entity"), Some("ord-2"));
    }

    #[test]
    fn record_audit_omitted_sender_renders_as_empty_string() {
        let result = record_audit(&Object::phi(), "compile", "compiled", None, None);
        let log = fetch_or_phi("audit_log", &result);
        let facts = log.as_seq().expect("audit_log should be a sequence");
        assert_eq!(facts.len(), 1);
        // `None` sender must materialize as "" so downstream binding
        // lookups never hit a missing key (totality of the fact type).
        assert_eq!(binding(&facts[0], "sender"), Some(""));
        assert_eq!(binding(&facts[0], "sequence"), Some("0"));
        assert_eq!(binding(&facts[0], "entity"), Some(""));
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
}
