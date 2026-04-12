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
// evaluation during bulk compile. Validation is O(constraints × population)
// and should be indexed by affected fact type (TODO). Until then, bulk
// loads can skip it when the readings are known-good.
thread_local! {
    static SKIP_VALIDATE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
pub fn set_skip_validate(on: bool) { SKIP_VALIDATE.with(|b| b.set(on)); }
fn is_skip_validate() -> bool { SKIP_VALIDATE.with(|b| b.get()) }
//
// All framework objects compile to these types:
//   Role        → Selector
//   Graph Schema → Construction (CONS of roles)
//   Query       → partial application (some roles bound)
//   Fact        → fully applied Construction (all roles bound)
//   Derivation  → Composition chain
//   Constraint  → Condition
//   Aggregation → Insert (fold)
//   Population traversal → ApplyToAll (map)

use std::collections::HashMap;
use std::sync::Arc;
use std::fmt;

#[cfg(feature = "parallel")]
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
    Seq(Vec<Object>),

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
    pub fn phi() -> Self { Object::Seq(vec![]) }

    pub fn seq(items: Vec<Object>) -> Self {
        // Bottom-preserving: if any element is Bottom, whole sequence is Bottom.
        if items.iter().any(|x| matches!(x, Object::Bottom)) {
            Object::Bottom
        } else {
            Object::Seq(items)
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
                for cell_obj in cells {
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
                        .collect()
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
    Object::seq(vec![response_obj, sender_obj, pop_obj])
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
                    Object::Seq(bindings)
                }).collect::<Vec<Object>>()
            }).unwrap_or_default();
            Object::seq(vec![Object::atom(ft_id), Object::Seq(fact_objs)])
        }).collect();
    Object::Seq(fact_types)
}

/// Decode a violation Object back to a Violation struct.
/// Expected: <constraint_id, constraint_text, detail>
/// Decode a violation Object back to a Violation struct.
/// Expected: <constraint_id, constraint_text, detail>
/// Detail can be an atom (string) or a sequence of atoms (joined with spaces).
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
pub fn decode_violations(obj: &Object) -> Vec<Violation> {
    match obj.as_seq() {
        Some(items) => items.iter().flat_map(|item|
            decode_violation(item).map_or_else(|| decode_violations(item), |v| vec![v])
        ).collect(),
        None => vec![],
    }
}

/// Encode a Violation as an Object.
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

    /// Equals: eq:<x, y> = T if x = y, F otherwise
    Eq,

    /// Contains: contains:<x,y> = T if atom x contains atom y (case-insensitive), else F
    Contains,

    /// Lower: lower:x = lowercase of atom x
    Lower,

    /// Length: length:<x₁, ..., xₙ> = n
    Length,

    /// Concat: concat:<<x1,...>, <y1,...>, ...> = <x1,...,y1,...,...>
    /// Flattens one level of nesting. Each element must be a sequence.
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
    Fetch,
    /// Store: ↓n:<name, contents, D> → D' with cell name updated
    Store,

    // ── Combining Forms ──────────────────────────────────────────

    /// Constant: x̄:y = x (for all y ≠ ⊥). A literal value in a reading.
    Constant(Object),

    /// Composition: (f ∘ g):x = f:(g:x). Derivation rule chains.
    Compose(Box<Func>, Box<Func>),

    /// Construction: [f₁,...,fₙ]:x = <f₁:x,...,fₙ:x>. Graph Schema = CONS of Roles.
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

    /// Filter: Filter(p):<x₁,...,xₙ> = <xᵢ | p:xᵢ = T>.
    /// The missing primitive for queries as partial application.
    /// Partial apply a graph schema (bind some roles) → predicate falls out.
    /// Filter(predicate) applied to population → matching facts.
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
    /// that don't fit the AST (arithmetic, string ops, external calls).
    /// TODO: replace all uses with Platform for FPGA synthesis.
    Native(Fn1),
}

// ── Application (the single operation) ───────────────────────────────
// f:x → Object. This is beta reduction.

/// Parse a pair of number atoms, apply an arithmetic operation (Backus +,-,×,÷).
fn apply_arithmetic(x: &Object, op: fn(f64, f64) -> Option<f64>) -> Object {
    match x.as_seq() {
        Some(items) if items.len() == 2 => {
            let a = items[0].as_atom().and_then(|s| s.parse::<f64>().ok());
            let b = items[1].as_atom().and_then(|s| s.parse::<f64>().ok());
            match (a, b) {
                (Some(a), Some(b)) => match op(a, b) {
                    Some(r) => {
                        if r.fract() == 0.0 && r.abs() < i64::MAX as f64 {
                            Object::Atom((r as i64).to_string())
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

pub fn apply(func: &Func, x: &Object, d: &Object) -> Object {
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
                Some(items) => Object::Seq(items[1..].to_vec()),
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

        Func::Eq => {
            match x.as_seq() {
                Some(items) if items.len() == 2 => {
                    if items[0] == items[1] { Object::t() } else { Object::f() }
                }
                _ => Object::Bottom,
            }
        }

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
                Some(items) => Object::Seq(items.iter().flat_map(|item|
                    item.as_seq().map(|sub| sub.to_vec())
                        .unwrap_or_else(|| vec![item.clone()])
                ).collect()),
                _ => Object::Bottom,
            }
        }

        Func::DistL => {
            match x.as_seq() {
                Some(items) if items.len() == 2 => {
                    let y = &items[0];
                    match items[1].as_seq() {
                        Some(zs) if zs.is_empty() => Object::phi(),
                        Some(zs) => Object::Seq(
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
                        Some(ys) => Object::Seq(
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
                            Object::Seq(result)
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
                            Object::Seq(result)
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
                    Object::Seq(result)
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
                    Object::Seq(result)
                }
                Some(_) => x.clone(),
                _ => Object::Bottom,
            }
        }

        Func::Add => apply_arithmetic(x, |a, b| Some(a + b)),
        Func::Sub => apply_arithmetic(x, |a, b| Some(a - b)),
        Func::Mul => apply_arithmetic(x, |a, b| Some(a * b)),
        Func::Div => apply_arithmetic(x, |a, b| if b == 0.0 { None } else { Some(a / b) }),

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
            // store:<name, contents, D> → D' with cell updated
            match x.as_seq() {
                Some(items) if items.len() == 3 => {
                    match items[0].as_atom() {
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
            #[cfg(feature = "parallel")]
            if funcs.len() >= 16 {
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
                    #[cfg(feature = "parallel")]
                    if items.len() >= 64 {
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
                    let rest = Object::Seq(items[1..].to_vec());
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
                    #[cfg(feature = "parallel")]
                    if items.len() >= 64 {
                        let kept: Vec<Object> = items.par_iter()
                            .filter(|xi| apply(p, xi, d) == Object::t())
                            .cloned()
                            .collect();
                        return Object::Seq(kept);
                    }
                    let kept: Vec<Object> = items.iter()
                        .filter(|xi| apply(p, xi, d) == Object::t())
                        .cloned()
                        .collect();
                    Object::Seq(kept)
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
                obj => apply(&metacompose(&obj, d), x, d),
            }
        }

        Func::Platform(name) => apply_platform(name, x, d),

        Func::Native(f) => f(x),
    }
}

/// Platform primitives — known operations resolved by name.
/// Each is a fixed function (x, D) → Object. Synthesizable to hardware.
fn apply_platform(name: &str, x: &Object, d: &Object) -> Object {
    match name {
        "compile" => platform_compile(x, d),
        "apply_command" => platform_apply_command(x, d),
        "verify_signature" => platform_verify_signature(x),
        s if s.starts_with("create:") => platform_create(&s[7..], x, d),
        s if s.starts_with("update:") => platform_update(&s[7..], x, d),
        s if s.starts_with("transition:") => platform_transition(&s[11..], x, d),
        _ => Object::Bottom,
    }
}

/// Platform primitive: signature verification (AREST §5.5).
/// Input: seq<atom, atom, atom> — (sender, payload, signature).
/// Output: atom("true"|"false"), or Object::Bottom on malformed input.
/// Wired through crate::crypto::verify_signature — currently a
/// DefaultHasher MAC placeholder; swap to HMAC-SHA256 when upgrading.
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
    "Graph Schema",
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
    let merged_domain = crate::compile::state_to_domain(&merged_state);
    let model_errors = crate::compile::validate_model(&merged_domain);
    model_errors.iter().for_each(|e| eprintln!("[model warning] {}", e));

    // Compile defs from merged state + re-register platform primitives
    let mut defs = crate::compile::compile_to_defs_state(&merged_state);
    defs.push(("compile".to_string(), Func::Platform("compile".to_string())));
    defs.push(("apply".to_string(), Func::Platform("apply_command".to_string())));
    defs.push(("verify_signature".to_string(), Func::Platform("verify_signature".to_string())));
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
    record_audit(&with_history, "compile", status, None)
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
) -> Object {
    let seq = fetch_or_phi("audit_log", state)
        .as_seq()
        .map(|items| items.len())
        .unwrap_or(0);
    let seq_str = seq.to_string();
    let sender_val = sender.unwrap_or("");
    let fact = fact_from_pairs(&[
        ("operation", operation),
        ("outcome", outcome),
        ("sequence", seq_str.as_str()),
        ("sender", sender_val),
    ]);
    cell_push("audit_log", fact, state)
}

/// apply command: create = emit ∘ validate ∘ derive ∘ resolve (Eq. 10).
/// Identity is a fact in the input — "Resource is created by User" (instances.md).
/// Authorization is enforced by the constraint pipeline, not by this function.
fn platform_apply_command(x: &Object, d: &Object) -> Object {
    let input = match x.as_atom() {
        Some(s) if s.len() <= PLATFORM_MAX_INPUT => s,
        Some(_) => return Object::atom("⊥ input exceeds platform buffer"),
        None => return Object::Bottom,
    };
    let command: crate::arest::Command = match serde_json::from_str(input) {
        Ok(c) => c,
        Err(e) => return Object::atom(&format!("⊥ {}", e)),
    };
    // Per-field bound: reject commands whose field values exceed the platform limit.
    match command_field_overflow(&command) {
        Some(field) => return Object::atom(&format!("⊥ field '{}' exceeds platform buffer", field)),
        None => {}
    }
    // D contains both population cells and def cells.
    // apply_command_defs uses d for ρ-dispatch and state for population.
    let result = crate::arest::apply_command_defs(d, &command, d);
    match serde_json::to_string(&result) {
        Ok(s) => Object::atom(&s),
        Err(e) => Object::atom(&format!("⊥ {}", e)),
    }
}

/// Platform primitive: create entity from fact pairs (AREST Eq. 6).
/// Key: "create:{noun}". Input: <<field, value>, ...> or <<id, val>, <field, val>, ...>.
/// Returns the result as an Object containing the new state.
fn platform_create(noun: &str, x: &Object, d: &Object) -> Object {
    let (id, fields) = extract_fact_pairs(x);
    let command = crate::arest::Command::CreateEntity {
        noun: noun.to_string(),
        domain: String::new(),
        id,
        fields,
        sender: None,
        signature: None,
    };
    let result = crate::arest::apply_command_defs(d, &command, d);
    crate::arest::encode_command_result(&result)
}

/// Platform primitive: update entity from fact pairs.
/// Key: "update:{noun}". Input: <<id, val>, <field, val>, ...>.
fn platform_update(noun: &str, x: &Object, d: &Object) -> Object {
    let (id, fields) = extract_fact_pairs(x);
    let entity_id = id.unwrap_or_default();
    let command = crate::arest::Command::UpdateEntity {
        noun: noun.to_string(),
        domain: String::new(),
        entity_id,
        fields,
        sender: None,
        signature: None,
    };
    let result = crate::arest::apply_command_defs(d, &command, d);
    crate::arest::encode_command_result(&result)
}

/// Platform primitive: transition entity state machine.
/// Key: "transition:{noun}". Input: <entity_id, event>.
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
    let command = crate::arest::Command::Transition {
        entity_id,
        event,
        domain: String::new(),
        current_status,
        sender: None,
        signature: None,
    };
    let result = crate::arest::apply_command_defs(d, &command, d);
    crate::arest::encode_command_result(&result)
}

/// Extract (optional id, field map) from an Object of fact pairs.
/// Input: <<id, val>, <field1, val1>, ...> or <<field1, val1>, ...>
fn extract_fact_pairs(x: &Object) -> (Option<String>, std::collections::HashMap<String, String>) {
    let mut fields = std::collections::HashMap::new();
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

/// Walk a Command's string fields and return the name of the first field whose
/// value exceeds PLATFORM_MAX_FIELD bytes, or None if all values are within bound.
fn command_field_overflow(command: &crate::arest::Command) -> Option<&'static str> {
    use crate::arest::Command;
    let over = |s: &str| s.len() > PLATFORM_MAX_FIELD;
    let map_over = |m: &std::collections::HashMap<String, String>| -> bool {
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
    pub const NULL: &str = "0?";
    pub const REVERSE: &str = "<>";
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
    pub const STORE: &str = "v";
    pub const CONTAINS: &str = "in";
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
    Object::Seq(vec![Object::atom(CELL_TAG), Object::atom(name), contents])
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
                true => Object::Seq(replaced),
                false => Object::Seq([replaced, vec![cell(name, contents)]].concat()),
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
            Object::Seq(v)
        }
        None => Object::Seq(vec![fact]),
    };
    store(name, new_contents, state)
}

/// Merge two states in O(n): collect all cells into a HashMap,
/// concatenate overlapping cells, return as Map store.
pub fn merge_states(target: &Object, source: &Object) -> Object {
    let mut map: HashMap<String, Object> = cells_iter(target).into_iter()
        .map(|(name, contents)| (name.to_string(), contents.clone()))
        .collect();
    cells_iter(source).into_iter().for_each(|(name, contents)| {
        let entry = map.entry(name.to_string()).or_insert_with(Object::phi);
        *entry = concat_seq(entry, contents);
    });
    Object::Map(map)
}

/// Concatenate two sequences: <a₁,...,aₙ> ++ <b₁,...,bₘ> = <a₁,...,aₙ,b₁,...,bₘ>
fn concat_seq(a: &Object, b: &Object) -> Object {
    let mut items = a.as_seq().map(|s| s.to_vec()).unwrap_or_default();
    items.extend(b.as_seq().map(|s| s.to_vec()).unwrap_or_else(|| vec![b.clone()]));
    Object::Seq(items)
}

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
        Object::Seq(vec![Object::atom(k), Object::atom(v)])
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
        primitives::CONTAINS => Func::Contains,
        primitives::CONCAT => Func::Concat,
        primitives::LOWER => Func::Lower,
        primitives::NULL => Func::NullTest,
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
        Func::Eq => Object::atom(primitives::EQ),
        Func::Contains => Object::atom(primitives::CONTAINS),
        Func::Concat => Object::atom(primitives::CONCAT),
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
        Func::Store => Object::atom(primitives::STORE),
        Func::Constant(x) => Object::seq(vec![Object::atom(forms::CONST), x.clone()]),
        Func::Compose(f, g) => Object::seq(vec![
            Object::atom(forms::COMP), func_to_object(f), func_to_object(g),
        ]),
        Func::Construction(funcs) => {
            let mut items = vec![Object::atom(forms::CONS)];
            items.extend(funcs.iter().map(func_to_object));
            Object::Seq(items) // not bottom-preserving — these are form objects
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
/// All four remain Native with clear documentation of why.
pub fn theta1_defs_vec() -> Vec<(String, Func)> {
    let mut defs = Vec::new();
    register_theta1_into(&mut defs);
    defs
}
fn register_theta1_into(defs: &mut Vec<(String, Func)>) {
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
                        Some(Object::Seq(projected))
                    })
                    .fold(Vec::new(), |mut acc, row| {
                        (!acc.contains(&row)).then(|| acc.push(row));
                        acc
                    });
                Some(Object::Seq(rows))
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
                                Object::Seq(merged)
                            })
                        })
                    })
                    .collect();
                Some(Object::Seq(result))
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
                            .then(|| Object::Seq(cols[..cols.len()-1].to_vec()))
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
                                Object::Seq(projected)
                            })
                        })
                    })
                    .fold(Vec::new(), |mut acc, row| {
                        (!acc.contains(&row)).then(|| acc.push(row));
                        acc
                    });
                Some(Object::Seq(result))
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
            Func::Eq => write!(f, "eq"),
            Func::Contains => write!(f, "contains"),
            Func::Concat => write!(f, "concat"),
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
        assert_eq!(Object::phi(), Object::Seq(vec![]));
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
    fn construction_is_graph_schema() {
        // Graph schema "User has Org Role in Organization" = [Role₁, Role₂, Role₃]
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
            Object::Seq(vec![
                Object::seq(vec![Object::atom("a"), Object::atom("b"), Object::atom("c")]),
                Object::seq(vec![Object::atom("d"), Object::atom("e"), Object::atom("f")]),
            ]),
        ]);
        let result = apply_theta1("project", &input);
        assert_eq!(result, Object::Seq(vec![
            Object::Seq(vec![Object::atom("a"), Object::atom("c")]),
            Object::Seq(vec![Object::atom("d"), Object::atom("f")]),
        ]));
    }

    #[test]
    fn theta1_projection_removes_duplicates() {
        // project:<⟨1⟩, <<a,x>,<b,y>,<a,z>>> = <<a>,<b>> (a appears once)
        let input = Object::seq(vec![
            Object::seq(vec![Object::atom("1")]),
            Object::Seq(vec![
                Object::seq(vec![Object::atom("a"), Object::atom("x")]),
                Object::seq(vec![Object::atom("b"), Object::atom("y")]),
                Object::seq(vec![Object::atom("a"), Object::atom("z")]),
            ]),
        ]);
        let result = apply_theta1("project", &input);
        assert_eq!(result, Object::Seq(vec![
            Object::Seq(vec![Object::atom("a")]),
            Object::Seq(vec![Object::atom("b")]),
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
        let r = Object::Seq(vec![
            Object::seq(vec![Object::atom("a"), Object::atom("x")]),
            Object::seq(vec![Object::atom("b"), Object::atom("y")]),
        ]);
        let s = Object::Seq(vec![
            Object::seq(vec![Object::atom("a"), Object::atom("1")]),
            Object::seq(vec![Object::atom("a"), Object::atom("2")]),
            Object::seq(vec![Object::atom("c"), Object::atom("3")]),
        ]);
        // join on col 1: a matches a (twice), b has no match, c has no match in R
        let input = Object::seq(vec![Object::atom("1"), r, s]);
        let result = apply_theta1("join", &input);
        // Expected: <<a,x,1>, <a,x,2>> (a matched, x from R, 1/2 from S minus shared)
        // S cols excluding shared col 1: just col 2
        assert_eq!(result, Object::Seq(vec![
            Object::Seq(vec![Object::atom("a"), Object::atom("x"), Object::atom("1")]),
            Object::Seq(vec![Object::atom("a"), Object::atom("x"), Object::atom("2")]),
        ]));
    }

    #[test]
    fn theta1_tie() {
        // γ(R): select tuples where first = last, remove last column
        // R = <<a,1,a>,<b,2,c>,<c,3,c>>
        // tie:R = <<a,1>,<c,3>> (first=last for a and c)
        let r = Object::Seq(vec![
            Object::seq(vec![Object::atom("a"), Object::atom("1"), Object::atom("a")]),
            Object::seq(vec![Object::atom("b"), Object::atom("2"), Object::atom("c")]),
            Object::seq(vec![Object::atom("c"), Object::atom("3"), Object::atom("c")]),
        ]);
        let result = apply_theta1("tie", &r);
        assert_eq!(result, Object::Seq(vec![
            Object::Seq(vec![Object::atom("a"), Object::atom("1")]),
            Object::Seq(vec![Object::atom("c"), Object::atom("3")]),
        ]));
    }

    #[test]
    fn theta1_composition() {
        // R·S = π₁ₛ(R*S) — project out shared column from join
        // R = <<a,x>,<b,y>>, S = <<x,1>,<y,2>>
        // compose_rel on col 2 of R = col 1 of S:
        // join gives <<a,x,1>,<b,y,2>>, project out col 2 gives <<a,1>,<b,2>>
        let _r = Object::Seq(vec![
            Object::seq(vec![Object::atom("a"), Object::atom("x")]),
            Object::seq(vec![Object::atom("b"), Object::atom("y")]),
        ]);
        let _s = Object::Seq(vec![
            Object::seq(vec![Object::atom("x"), Object::atom("1")]),
            Object::seq(vec![Object::atom("y"), Object::atom("2")]),
        ]);
        // compose_rel:<shared_col, R, S>
        // shared_col = 2 for R (col 2), = 1 for S (col 1)
        // Our impl uses same index for both, so use col 1:
        // Actually our compose_rel joins on shared_col in both, then removes it.
        // R' = <<x,a>>, S' = <<x,1>> with shared on col 1:
        let r2 = Object::Seq(vec![
            Object::seq(vec![Object::atom("x"), Object::atom("a")]),
            Object::seq(vec![Object::atom("y"), Object::atom("b")]),
        ]);
        let s2 = Object::Seq(vec![
            Object::seq(vec![Object::atom("x"), Object::atom("1")]),
            Object::seq(vec![Object::atom("y"), Object::atom("2")]),
        ]);
        let input = Object::seq(vec![Object::atom("1"), r2, s2]);
        let result = apply_theta1("compose_rel", &input);
        // x matches x: project out col 1 → <a, 1>
        // y matches y: project out col 1 → <b, 2>
        assert_eq!(result, Object::Seq(vec![
            Object::Seq(vec![Object::atom("a"), Object::atom("1")]),
            Object::Seq(vec![Object::atom("b"), Object::atom("2")]),
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
        let state = Object::Seq(vec![
            cell("FILE", Object::seq(vec![Object::atom("a"), Object::atom("b")])),
            cell("defs", Object::seq(vec![Object::atom("c")])),
        ]);
        assert_eq!(fetch("FILE", &state), Object::seq(vec![Object::atom("a"), Object::atom("b")]));
        assert_eq!(fetch("defs", &state), Object::seq(vec![Object::atom("c")]));
        assert_eq!(fetch("missing", &state), Object::Bottom);
    }

    #[test]
    fn cell_store_replaces_contents() {
        let state = Object::Seq(vec![
            cell("FILE", Object::seq(vec![Object::atom("old")])),
            cell("defs", Object::seq(vec![Object::atom("c")])),
        ]);
        let new_state = store("FILE", Object::seq(vec![Object::atom("new")]), &state);
        assert_eq!(fetch("FILE", &new_state), Object::seq(vec![Object::atom("new")]));
        assert_eq!(fetch("defs", &new_state), Object::seq(vec![Object::atom("c")]));
    }

    #[test]
    fn cell_store_appends_new_cell() {
        let state = Object::Seq(vec![
            cell("FILE", Object::atom("data")),
        ]);
        let new_state = store("defs", Object::atom("rules"), &state);
        assert_eq!(fetch("FILE", &new_state), Object::atom("data"));
        assert_eq!(fetch("defs", &new_state), Object::atom("rules"));
    }

    #[test]
    fn fetch_via_func_apply() {
        // fetch:<"FILE", D> via Func::Fetch
        let state = Object::Seq(vec![
            cell("FILE", Object::atom("population")),
        ]);
        let input = Object::seq(vec![Object::atom("FILE"), state]);
        assert_eq!(apply(&Func::Fetch, &input, &defs()), Object::atom("population"));
    }

    #[test]
    fn store_via_func_apply() {
        // store:<"FILE", new_contents, D> via Func::Store
        let state = Object::Seq(vec![
            cell("FILE", Object::atom("old")),
        ]);
        let input = Object::seq(vec![Object::atom("FILE"), Object::atom("new"), state]);
        let result = apply(&Func::Store, &input, &defs());
        assert_eq!(fetch("FILE", &result), Object::atom("new"));
    }

    #[test]
    fn fetch_via_ffp() {
        // FFP: ("^":<"FILE", D>)
        let state = Object::Seq(vec![
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
        let d = Object::Seq(vec![
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
        let obj = Object::Seq(vec![
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
        let obj = Object::Seq(vec![
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
        let obj = Object::Seq(vec![
            Object::atom(forms::COND),
            Object::atom(primitives::NULL),
            Object::Seq(vec![Object::atom(forms::CONST), Object::atom("empty")]),
            Object::Seq(vec![Object::atom(forms::CONST), Object::atom("notempty")]),
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
        let obj = Object::Seq(vec![
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
        let obj = Object::Seq(vec![
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
        let obj = Object::Seq(vec![
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
        let ip_obj = Object::Seq(vec![
            Object::atom(forms::COMP),
            Object::Seq(vec![Object::atom(forms::INSERT), Object::atom(primitives::ADD)]),
            Object::Seq(vec![
                Object::atom(forms::COMP),
                Object::Seq(vec![Object::atom(forms::ALPHA), Object::atom(primitives::MUL)]),
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
        let filter_obj = Object::Seq(vec![
            Object::atom(forms::FILTER),
            Object::Seq(vec![
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
        // (std::iter::successors = Backus's $\mathit{while}$ combining form)
        let current = std::iter::successors(Some(&result), |c| match c {
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
        let state = Object::Seq(vec![cell("nouns", Object::atom("Alice"))]);
        assert_eq!(fetch_or_phi("nouns", &state), Object::atom("Alice"));
    }

    #[test]
    fn cell_push_creates_cell_on_empty_state() {
        let state = Object::phi();
        let fact = fact_from_pairs(&[("name", "Alice")]);
        let state2 = cell_push("Noun", fact.clone(), &state);
        assert_eq!(fetch_or_phi("Noun", &state2), Object::Seq(vec![fact]));
    }

    #[test]
    fn cell_push_appends_to_existing_cell() {
        let f1 = fact_from_pairs(&[("name", "Alice")]);
        let f2 = fact_from_pairs(&[("name", "Bob")]);
        let state = cell_push("Noun", f1.clone(), &Object::phi());
        let state2 = cell_push("Noun", f2.clone(), &state);
        assert_eq!(fetch_or_phi("Noun", &state2), Object::Seq(vec![f1, f2]));
    }

    #[test]
    fn cells_iter_enumerates_all_cells() {
        let state = Object::Seq(vec![
            cell("A", Object::atom("1")),
            cell("B", Object::atom("2")),
        ]);
        let pairs: Vec<(&str, &Object)> = cells_iter(&state);
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].0, "A");
        assert_eq!(pairs[1].0, "B");
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
        assert_eq!(fetch_or_phi("Noun", &state), Object::Seq(vec![f1]));
    }

    #[test]
    fn cell_push_preserves_other_cells() {
        let state = cell_push("A", Object::atom("1"), &Object::phi());
        let state = cell_push("B", Object::atom("2"), &state);
        assert_eq!(fetch_or_phi("A", &state), Object::Seq(vec![Object::atom("1")]));
        assert_eq!(fetch_or_phi("B", &state), Object::Seq(vec![Object::atom("2")]));
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
        let result = record_audit(&state, "compile", "compiled", Some("root@example"));
        let log = fetch_or_phi("audit_log", &result);
        let facts = log.as_seq().expect("audit_log should be a sequence");
        assert_eq!(facts.len(), 1, "empty state should yield exactly one audit entry");
        assert_eq!(binding(&facts[0], "operation"), Some("compile"));
        assert_eq!(binding(&facts[0], "outcome"), Some("compiled"));
        assert_eq!(binding(&facts[0], "sequence"), Some("0"));
        assert_eq!(binding(&facts[0], "sender"), Some("root@example"));
    }

    #[test]
    fn record_audit_next_entry_uses_cell_length_as_sequence() {
        // Pre-populate the audit_log with two arbitrary prior entries.
        let state = record_audit(&Object::phi(), "compile", "compiled", None);
        let state = record_audit(&state, "apply:create", "ok", Some("u1"));
        // The third push must observe sequence = 2 (current cell length).
        let state = record_audit(&state, "apply:create", "rejected", Some("u2"));
        let log = fetch_or_phi("audit_log", &state);
        let facts = log.as_seq().expect("audit_log should be a sequence");
        assert_eq!(facts.len(), 3, "three pushes should yield three entries");
        assert_eq!(binding(&facts[2], "operation"), Some("apply:create"));
        assert_eq!(binding(&facts[2], "outcome"), Some("rejected"));
        assert_eq!(binding(&facts[2], "sequence"), Some("2"));
        assert_eq!(binding(&facts[2], "sender"), Some("u2"));
    }

    #[test]
    fn record_audit_omitted_sender_renders_as_empty_string() {
        let result = record_audit(&Object::phi(), "compile", "compiled", None);
        let log = fetch_or_phi("audit_log", &result);
        let facts = log.as_seq().expect("audit_log should be a sequence");
        assert_eq!(facts.len(), 1);
        // `None` sender must materialize as "" so downstream binding
        // lookups never hit a missing key (totality of the fact schema).
        assert_eq!(binding(&facts[0], "sender"), Some(""));
        assert_eq!(binding(&facts[0], "sequence"), Some("0"));
    }

    // ── Security #19: per-field input bound (PLATFORM_MAX_FIELD) ─────
    //
    // `command_field_overflow` walks every Command variant and returns
    // the first field name whose String value exceeds PLATFORM_MAX_FIELD
    // (64KB). These tests lock the contract down per variant per field,
    // including HashMap key/value overflow on fields/bindings, and then
    // cover the integration path via `platform_apply_command` for both
    // the PLATFORM_MAX_INPUT (1MB) and PLATFORM_MAX_FIELD gates.

    use crate::arest::Command as ArestCommand;

    fn huge() -> String {
        "a".repeat(PLATFORM_MAX_FIELD + 1)
    }

    fn ok_map() -> std::collections::HashMap<String, String> {
        let mut m = std::collections::HashMap::new();
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
        let mut fields = std::collections::HashMap::new();
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
        let mut fields = std::collections::HashMap::new();
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
        let mut bindings = std::collections::HashMap::new();
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
        let mut bindings = std::collections::HashMap::new();
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
        let mut fields = std::collections::HashMap::new();
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
        let mut fields = std::collections::HashMap::new();
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
}
