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
// All framework objects compile to these types:
//   Role        → Selector
//   Graph Schema → Construction (CONS of roles)
//   Query       → partial application (some roles bound)
//   Fact        → fully applied Construction (all roles bound)
//   Derivation  → Composition chain
//   Constraint  → Condition
//   Aggregation → Insert (fold)
//   Population traversal → ApplyToAll (map)

use std::sync::Arc;
use std::fmt;

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
}

/// Split a string on commas, respecting nested <> brackets.
fn split_top_level(s: &str) -> Vec<&str> {
    let mut result = vec![];
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => {
                result.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    result.push(&s[start..]);
    result
}

/// Maximum nesting depth for `Object::parse` to prevent stack overflow on
/// maliciously crafted inputs (e.g. deeply nested `< < < ... > > >`).
const MAX_PARSE_DEPTH: usize = 100;

fn parse_with_depth(input: &str, depth: usize) -> Object {
    let s = input.trim();
    if s.is_empty() || s == "\u{03C6}" { return Object::phi(); }
    if s == "\u{22A5}" { return Object::Bottom; }
    if s.starts_with('<') && s.ends_with('>') {
        if depth >= MAX_PARSE_DEPTH {
            return Object::Bottom;
        }
        let inner = &s[1..s.len()-1];
        if inner.trim().is_empty() { return Object::phi(); }
        let items = split_top_level(inner);
        return Object::Seq(items.into_iter().map(|i| parse_with_depth(i.trim(), depth + 1)).collect());
    }
    Object::Atom(s.to_string())
}

impl fmt::Display for Object {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Object::Atom(s) => write!(f, "{}", s),
            Object::Seq(items) if items.is_empty() => write!(f, "φ"),
            Object::Seq(items) => {
                write!(f, "<")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{}", item)?;
                }
                write!(f, ">")
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
pub fn encode_state(state: &Object) -> Object {
    let fact_types: Vec<Object> = cells_iter(state).into_iter().map(|(ft_id, contents)| {
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
    let items = obj.as_seq()?;
    if items.len() != 3 { return None; }
    let detail = match &items[2] {
        Object::Atom(s) => s.clone(),
        Object::Seq(parts) => parts.iter()
            .filter_map(|p| p.as_atom())
            .collect::<Vec<_>>()
            .join(" "),
        _ => return None,
    };
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
pub fn defs_to_state(defs: &[(String, Func)], state: &Object) -> Object {
    defs.iter().fold(state.clone(), |acc, (name, func)| {
        store(name, func_to_object(func), &acc)
    })
}

pub fn apply(func: &Func, x: &Object, d: &Object) -> Object {
    // All functions are bottom-preserving
    if x.is_bottom() {
        return Object::Bottom;
    }

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
                Some(items) => {
                    let mut result = Vec::new();
                    for item in items {
                        match item.as_seq() {
                            Some(sub) => result.extend_from_slice(sub),
                            None => result.push(item.clone()),
                        }
                    }
                    Object::Seq(result)
                }
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

        Func::Trans => {
            match x.as_seq() {
                Some(rows) if rows.is_empty() => Object::phi(),
                Some(rows) => {
                    // All rows must be sequences of the same length
                    let inner: Vec<&[Object]> = rows.iter()
                        .filter_map(|r| r.as_seq())
                        .collect();
                    if inner.len() != rows.len() { return Object::Bottom; }
                    if inner.is_empty() { return Object::phi(); }
                    let cols = inner[0].len();
                    if inner.iter().any(|r| r.len() != cols) { return Object::Bottom; }
                    Object::Seq(
                        (0..cols).map(|c|
                            Object::Seq(inner.iter().map(|r| r[c].clone()).collect())
                        ).collect()
                    )
                }
                _ => Object::Bottom,
            }
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
            let mut current = x.clone();
            let max_iterations = 1000; // safety limit
            for _ in 0..max_iterations {
                match apply(p, &current, d) {
                    Object::Atom(ref s) if s == "T" => {
                        current = apply(f, &current, d);
                        if current.is_bottom() { return Object::Bottom; }
                    }
                    Object::Atom(ref s) if s == "F" => return current,
                    _ => return Object::Bottom,
                }
            }
            Object::Bottom // exceeded iteration limit
        }

        Func::FoldL(f) => {
            match x.as_seq() {
                Some(items) if items.len() == 2 => {
                    let mut acc = items[0].clone();
                    let seq = match items[1].as_seq() {
                        Some(s) => s,
                        None => return Object::Bottom,
                    };
                    for element in seq {
                        let pair = Object::seq(vec![acc, element.clone()]);
                        acc = apply(f, &pair, d);
                        if acc.is_bottom() { return Object::Bottom; }
                    }
                    acc
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
        _ => Object::Bottom,
    }
}

/// compile ∘ parse: readings text → new defs merged into D.
/// Returns the new state D' (caller stores it).
/// Max input buffer size — platform hardware limit.
const PLATFORM_MAX_INPUT: usize = 1_024 * 1_024;

fn platform_compile(x: &Object, d: &Object) -> Object {
    let input = match x.as_atom() {
        Some(s) if s.len() <= PLATFORM_MAX_INPUT => s,
        Some(_) => return Object::atom("⊥ input exceeds platform buffer"),
        None => return Object::Bottom,
    };

    let existing_domain = crate::compile::state_to_domain(d);

    let ir = if existing_domain.nouns.is_empty() {
        crate::parse_forml2::parse_markdown(input)
    } else {
        crate::parse_forml2::parse_markdown_with_context(input, &existing_domain.nouns, &existing_domain.fact_types)
    };
    let domain = match ir {
        Ok(d) => d,
        Err(e) => return Object::atom(&format!("⊥ {}", e)),
    };

    // UC guard on Noun has Object Type: reject if a noun is redeclared with a different type.
    // This is what validate would catch if HashMap didn't erase the conflict.
    let mut merged = existing_domain;
    let noun_conflicts: Vec<String> = domain.nouns.iter()
        .filter_map(|(name, new_def)| {
            merged.nouns.get(name)
                .filter(|existing| existing.object_type != new_def.object_type)
                .map(|existing| format!("Each Noun has exactly one Object Type. '{}' is {} but redeclared as {}",
                    name, existing.object_type, new_def.object_type))
        })
        .collect();
    if !noun_conflicts.is_empty() {
        return Object::atom(&format!("⊥ constraint violation: {}", noun_conflicts.join("; ")));
    }
    merged.nouns.extend(domain.nouns);
    merged.fact_types.extend(domain.fact_types);
    merged.constraints.extend(domain.constraints);
    merged.state_machines.extend(domain.state_machines);
    merged.derivation_rules.extend(domain.derivation_rules);
    merged.general_instance_facts.extend(domain.general_instance_facts);
    merged.subtypes.extend(domain.subtypes);
    merged.enum_values.extend(domain.enum_values);
    merged.ref_schemes.extend(domain.ref_schemes);
    merged.objectifications.extend(domain.objectifications);
    merged.named_spans.extend(domain.named_spans);
    merged.autofill_spans.extend(domain.autofill_spans);

    let state = crate::parse_forml2::domain_to_state(&merged);
    let mut defs = crate::compile::compile_to_defs_state(&state);
    // Re-register platform primitives in D' (they must survive state transitions)
    defs.push(("compile".to_string(), Func::Platform("compile".to_string())));
    defs.push(("apply".to_string(), Func::Platform("apply_command".to_string())));
    let new_d = defs_to_state(&defs, &state);

    // validate: apply the compiled constraint set to the merged state.
    // If alethic violations exist, reject the compile (return violations, not D').
    let ctx = crate::ast::encode_eval_context_state("", None, &state);
    let violations = apply(&Func::Def("validate".to_string()), &ctx, &new_d);
    let decoded = crate::ast::decode_violations(&violations);
    let has_alethic = decoded.iter().any(|v| v.alethic);
    // Alethic violation → reject. Return the violation text.
    match has_alethic {
        true => Object::atom(&format!("⊥ constraint violation: {}",
            decoded.iter().filter(|v| v.alethic).map(|v| v.constraint_text.as_str()).collect::<Vec<_>>().join("; "))),
        false => new_d,
    }
}

/// apply command: create = emit ∘ validate ∘ derive ∘ resolve (Eq. 10).
/// Input x is the command JSON. Returns the CommandResult as a serialized Object.
fn platform_apply_command(x: &Object, d: &Object) -> Object {
    let input = match x.as_atom() {
        Some(s) => s,
        None => return Object::Bottom,
    };
    let command: crate::arest::Command = match serde_json::from_str(input) {
        Ok(c) => c,
        Err(e) => return Object::atom(&format!("⊥ {}", e)),
    };
    let state = crate::compile::state_to_domain(d);
    let pop_state = crate::parse_forml2::domain_to_state(&state);
    let result = crate::arest::apply_command_defs(d, &command, &pop_state);
    match serde_json::to_string(&result) {
        Ok(json) => Object::atom(&json),
        Err(e) => Object::atom(&format!("⊥ {}", e)),
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

/// Fetch (↑n): retrieve contents of the first cell named n from a sequence of cells.
/// ↑n:D → c where D contains <CELL, n, c>
/// Returns bottom if no cell named n exists.
pub fn fetch(name: &str, state: &Object) -> Object {
    match state.as_seq() {
        Some(cells) => {
            for cell_obj in cells {
                if let Some(items) = cell_obj.as_seq() {
                    if items.len() == 3
                        && items[0].as_atom() == Some(CELL_TAG)
                        && items[1].as_atom() == Some(name)
                    {
                        return items[2].clone();
                    }
                }
            }
            Object::Bottom
        }
        None => Object::Bottom,
    }
}

/// Store (↓n): replace or append cell named n with new contents.
/// ↓n:<x, D> → D' where D' has cell n with contents x.
/// If cell n exists, its contents are replaced. Otherwise a new cell is appended.
pub fn store(name: &str, contents: Object, state: &Object) -> Object {
    let cells = match state.as_seq() {
        Some(cells) => cells,
        None => return Object::Bottom,
    };

    let mut result: Vec<Object> = Vec::new();
    let mut found = false;
    for cell_obj in cells {
        if let Some(items) = cell_obj.as_seq() {
            if items.len() == 3
                && items[0].as_atom() == Some(CELL_TAG)
                && items[1].as_atom() == Some(name)
            {
                // Replace contents
                result.push(cell(name, contents.clone()));
                found = true;
                continue;
            }
        }
        result.push(cell_obj.clone());
    }
    if !found {
        result.push(cell(name, contents));
    }
    Object::Seq(result)
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

/// Iterate all cells in state as (name, contents) pairs.
/// Replaces: population.facts.iter()
pub fn cells_iter(state: &Object) -> Vec<(&str, &Object)> {
    state.as_seq().map(|cells| {
        cells.iter().filter_map(|c| {
            let items = c.as_seq()?;
            if items.len() == 3 && items[0].as_atom() == Some(CELL_TAG) {
                Some((items[1].as_atom()?, &items[2]))
            } else {
                None
            }
        }).collect()
    }).unwrap_or_default()
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
    if items.is_empty() { return Func::Constant(Object::Bottom); }

    // The controlling operator is the first element
    let controller = match items[0].as_atom() {
        Some(name) => name,
        None => return Func::Constant(Object::Bottom),
    };

    match controller {
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
    }
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
        let items = match x.as_seq() {
            Some(items) if items.len() == 2 => items,
            _ => return Object::Bottom,
        };
        let indices = match items[0].as_seq() {
            Some(idx) => idx,
            None => return Object::Bottom,
        };
        let relation = match items[1].as_seq() {
            Some(r) => r,
            None => return Object::Bottom,
        };
        let selectors: Vec<usize> = indices.iter()
            .filter_map(|i| i.as_atom().and_then(|s| s.parse().ok()))
            .collect();
        if selectors.is_empty() { return Object::Bottom; }

        let mut rows: Vec<Object> = Vec::new();
        for tuple in relation {
            if let Some(cols) = tuple.as_seq() {
                let projected: Vec<Object> = selectors.iter()
                    .filter_map(|&s| if s >= 1 && s <= cols.len() { Some(cols[s-1].clone()) } else { None })
                    .collect();
                let row = Object::Seq(projected);
                if !rows.contains(&row) {
                    rows.push(row);
                }
            }
        }
        Object::Seq(rows)
    }))));

    // join: join:<shared_col, R, S> = natural join on shared column index.
    // NATIVE because: shared_col is a runtime value that determines which
    // Selector to use for comparison and which columns to include in the
    // merged tuple. Pure Func cannot parameterize Selector indices from data.
    defs.push(("join".to_string(), Func::Native(Arc::new(|x: &Object| {
        let items = match x.as_seq() {
            Some(items) if items.len() == 3 => items,
            _ => return Object::Bottom,
        };
        let shared_col: usize = match items[0].as_atom().and_then(|s| s.parse().ok()) {
            Some(c) => c,
            None => return Object::Bottom,
        };
        let r = match items[1].as_seq() { Some(r) => r, None => return Object::Bottom };
        let s = match items[2].as_seq() { Some(s) => s, None => return Object::Bottom };

        let mut result: Vec<Object> = Vec::new();
        for r_tuple in r {
            let r_cols = match r_tuple.as_seq() { Some(c) => c, None => continue };
            let r_val = if shared_col >= 1 && shared_col <= r_cols.len() {
                &r_cols[shared_col - 1]
            } else { continue };

            for s_tuple in s {
                let s_cols = match s_tuple.as_seq() { Some(c) => c, None => continue };
                let s_val = if shared_col >= 1 && shared_col <= s_cols.len() {
                    &s_cols[shared_col - 1]
                } else { continue };

                if r_val == s_val {
                    let mut merged: Vec<Object> = r_cols.to_vec();
                    for (i, col) in s_cols.iter().enumerate() {
                        if i + 1 != shared_col {
                            merged.push(col.clone());
                        }
                    }
                    result.push(Object::Seq(merged));
                }
            }
        }
        Object::Seq(result)
    }))));

    // tie: gamma(R) = Filter(eq . [sel(1), sel(n)]) : R
    // Selects tuples where first column = last column, then removes the last column.
    // NATIVE because: "last column" requires knowing the tuple arity n at runtime.
    // Backus's Selector(n) requires a fixed n at compile time. There is no
    // "select last element" primitive in FP. The Reverse+Selector(1) trick
    // works for comparison but the "remove last column" step still needs
    // dynamic-arity Construction to rebuild the tuple without its last element.
    defs.push(("tie".to_string(), Func::Native(Arc::new(|x: &Object| {
        let relation = match x.as_seq() {
            Some(r) => r,
            None => return Object::Bottom,
        };
        let mut result: Vec<Object> = Vec::new();
        for tuple in relation {
            if let Some(cols) = tuple.as_seq() {
                if cols.len() >= 2 && cols[0] == cols[cols.len() - 1] {
                    result.push(Object::Seq(cols[..cols.len()-1].to_vec()));
                }
            }
        }
        Object::Seq(result)
    }))));

    // compose_rel: R . S = pi_1s(R*S) -- relational composition.
    // Join R and S on shared column, then project out the shared column.
    // NATIVE because: inherits both join's dynamic column selection and
    // project's dynamic Construction building. The shared_col parameter
    // determines runtime behavior that cannot be fixed at compile time.
    defs.push(("compose_rel".to_string(), Func::Native(Arc::new(|x: &Object| {
        let items = match x.as_seq() {
            Some(items) if items.len() == 3 => items,
            _ => return Object::Bottom,
        };
        let shared_col: usize = match items[0].as_atom().and_then(|s| s.parse().ok()) {
            Some(c) => c,
            None => return Object::Bottom,
        };
        let r = match items[1].as_seq() { Some(r) => r, None => return Object::Bottom };
        let s = match items[2].as_seq() { Some(s) => s, None => return Object::Bottom };

        let mut result: Vec<Object> = Vec::new();
        for r_tuple in r {
            let r_cols = match r_tuple.as_seq() { Some(c) => c, None => continue };
            let r_val = if shared_col >= 1 && shared_col <= r_cols.len() {
                &r_cols[shared_col - 1]
            } else { continue };

            for s_tuple in s {
                let s_cols = match s_tuple.as_seq() { Some(c) => c, None => continue };
                let s_val = if shared_col >= 1 && shared_col <= s_cols.len() {
                    &s_cols[shared_col - 1]
                } else { continue };

                if r_val == s_val {
                    let mut projected: Vec<Object> = Vec::new();
                    for (i, col) in r_cols.iter().enumerate() {
                        if i + 1 != shared_col { projected.push(col.clone()); }
                    }
                    for (i, col) in s_cols.iter().enumerate() {
                        if i + 1 != shared_col { projected.push(col.clone()); }
                    }
                    let row = Object::Seq(projected);
                    if !result.contains(&row) {
                        result.push(row);
                    }
                }
            }
        }
        Object::Seq(result)
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
            Func::Native(_) | Func::Platform(_) => true,
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
                write!(f, "[")?;
                for (i, func) in funcs.iter().enumerate() {
                    if i > 0 { write!(f, ", ")?; }
                    write!(f, "{:?}", func)?;
                }
                write!(f, "]")
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
    use std::collections::HashMap;

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
        let mut current = &result;
        for _ in 0..100 {
            match current {
                Object::Seq(items) if items.len() == 1 => current = &items[0],
                other => { current = other; break; }
            }
        }
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
}
