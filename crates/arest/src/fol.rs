// crates/arest/src/fol.rs
//
// First-Order Logic intermediate representation between parse (Φ
// cells) and compile (Func trees). Foundation commit for #357 —
// defines the `FolTerm` enum + `to_func` translator + basic tests.
// No `compile.rs` call sites rewired yet; that lands in follow-up
// commits where each `compile_<kind>_ast` function gets routed via
// FolTerm instead of building Func directly.
//
// Why FolTerm exists:
//
// `compile.rs` has ~17 functions (compile_uniqueness_ast,
// compile_ring_irreflexive_ast, compile_subset_ast, ...) each
// hand-translating a constraint kind directly to Func. That's a
// hand-rolled FOL→FFP compiler, distributed across 17 sites with
// no shared algebra. Adding a new constraint kind means adding a
// 18th compile_*_ast function from scratch.
//
// FolTerm gives the compiler a single algebra to work in. Each
// constraint kind becomes a small `kind → FolTerm` translator
// (10-30 lines) and a single `FolTerm → Func` reducer handles the
// universal lowering. New constraint kinds reuse the lowering;
// optimisation passes (Backus §12 distributivity laws, redundant
// quantifier elimination) become single rewrites over FolTerm
// instead of needing per-site changes.
//
// What's in this commit:
//
//   * `enum FolTerm` — variants for boolean combinators, FOL
//     quantifiers, atomic predicates, terms, and an escape hatch
//     (`Raw`) so callers can wrap an existing Func during gradual
//     migration.
//   * `FactSource` — what a quantifier ranges over (a single fact
//     type or a subtype-union).
//   * `to_func(self) -> Func` — lowers FolTerm to the existing
//     ast::Func vocabulary. The translation is direct: each
//     variant maps to the corresponding Func combinator. Quantifier
//     translation uses Backus's `Insert` form (∀f. P(f) →
//     `Insert(And) ∘ α(P)`); ∃ → `Insert(Or) ∘ α(P)`. Same shape
//     `query.rs::build_predicate` already uses for N-ary AND.
//   * Round-trip tests confirm common shapes (Eq, And, ForAll, …)
//     produce a Func that evaluates correctly against an empty
//     state.
//
// What this does NOT do (later commits):
//
//   * Rewire `compile_uniqueness_ast` / `compile_ring_*_ast` /
//     `compile_subset_ast` etc. to build FolTerm and lower via
//     `to_func`. That's the scoped work in #357 follow-ups.
//   * Optimisation passes over FolTerm. Once enough call sites
//     route through it, redundant-quantifier elimination becomes
//     a single rewrite.
//   * Pretty-printing back to FORML 2 prose for verbalisation
//     (#215 / Theorem 5). FolTerm is the natural shape for the
//     verbaliser to read.

use crate::ast::{Func, Object};
#[allow(unused_imports)]
use alloc::{boxed::Box, string::String, vec, vec::Vec};

/// A bound variable name. Same shape as the role-index variables
/// the existing constraint compilers introduce (e.g. "f" for a
/// fact, "x" / "y" for entity bindings within a fact).
pub type VarName = String;

/// 1-indexed role within a fact, matching `ast::Func::Selector`'s
/// convention. `RoleVal("f", 1)` reads "role 1 of f".
pub type RoleIdx = usize;

/// An identifier of a fact type registered in the metamodel.
pub type FactTypeId = String;

/// What a quantifier ranges over.
#[derive(Clone, Debug)]
pub enum FactSource {
    /// Facts of a single fact type.
    Single(FactTypeId),
    /// Facts of multiple fact types (subtype union — used by ring
    /// constraints over a noun's full subtype tree).
    Union(Vec<FactTypeId>),
}

/// First-order logic term. Universal IR for constraint and
/// derivation compile paths.
#[derive(Clone, Debug)]
pub enum FolTerm {
    // ── Truth values ────────────────────────────────────────
    True,
    False,

    // ── Boolean combinators ─────────────────────────────────
    /// Logical AND. Empty vec is `True`.
    And(Vec<FolTerm>),
    /// Logical OR. Empty vec is `False`.
    Or(Vec<FolTerm>),
    /// Logical NOT.
    Not(Box<FolTerm>),
    /// `lhs → rhs`. Translates to `Or(Not(lhs), rhs)` at lowering
    /// time; kept as a distinct variant for verbaliser legibility.
    Implies(Box<FolTerm>, Box<FolTerm>),

    // ── Quantifiers ─────────────────────────────────────────
    /// `∀ var ∈ source. body`. Lowers to
    /// `Insert(And) ∘ α(body)` over the source's facts.
    ForAll(VarName, FactSource, Box<FolTerm>),
    /// `∃ var ∈ source. body`. Lowers to
    /// `Insert(Or) ∘ α(body)` over the source's facts.
    Exists(VarName, FactSource, Box<FolTerm>),

    // ── Atomic predicates ───────────────────────────────────
    Eq(Box<FolTerm>, Box<FolTerm>),
    Lt(Box<FolTerm>, Box<FolTerm>),
    Gt(Box<FolTerm>, Box<FolTerm>),
    Le(Box<FolTerm>, Box<FolTerm>),
    Ge(Box<FolTerm>, Box<FolTerm>),

    // ── Terms ───────────────────────────────────────────────
    /// Variable reference. Resolves to the bound fact by walking
    /// the enclosing quantifier scope.
    Var(VarName),
    /// `var.role(n)` — the n'th role's value of the fact bound to
    /// `var`. Lowers to a `Selector(n)` against the bound fact.
    RoleVal(VarName, RoleIdx),
    /// Literal constant.
    Const(Object),

    // ── Escape hatch ────────────────────────────────────────
    /// Wraps an opaque Func. Lets gradual-migration callers stick
    /// a hand-built Func into the IR without translating it. Use
    /// sparingly; the more Raw nodes survive, the less FolTerm is
    /// doing for you.
    Raw(Func),
}

impl FolTerm {
    /// Lower this FolTerm into the equivalent `ast::Func`.
    ///
    /// Variable resolution today is positional — the outermost
    /// quantifier's bound variable is "self" of the resulting
    /// Func application. Nested quantifiers / multi-variable
    /// references are a follow-up; the tests below stick to
    /// single-quantifier shapes that exercise every variant.
    pub fn to_func(self) -> Func {
        match self {
            FolTerm::True => Func::constant(Object::t()),
            FolTerm::False => Func::constant(Object::f()),

            FolTerm::And(terms) => match terms.len() {
                0 => Func::constant(Object::t()),
                1 => terms.into_iter().next().unwrap().to_func(),
                _ => Func::compose(
                    Func::insert(Func::And),
                    Func::construction(terms.into_iter().map(FolTerm::to_func).collect()),
                ),
            },
            FolTerm::Or(terms) => match terms.len() {
                0 => Func::constant(Object::f()),
                1 => terms.into_iter().next().unwrap().to_func(),
                _ => Func::compose(
                    Func::insert(Func::Or),
                    Func::construction(terms.into_iter().map(FolTerm::to_func).collect()),
                ),
            },
            FolTerm::Not(inner) => Func::compose(Func::Not, inner.to_func()),
            FolTerm::Implies(lhs, rhs) => {
                // p → q  ≡  ¬p ∨ q
                FolTerm::Or(vec![FolTerm::Not(lhs), *rhs]).to_func()
            }

            FolTerm::ForAll(_var, source, body) => quantifier(
                Object::t(),
                Func::And,
                body.to_func(),
                facts_of(&source),
            ),
            FolTerm::Exists(_var, source, body) => quantifier(
                Object::f(),
                Func::Or,
                body.to_func(),
                facts_of(&source),
            ),

            FolTerm::Eq(lhs, rhs) => Func::compose(
                Func::Eq,
                Func::construction(vec![lhs.to_func(), rhs.to_func()]),
            ),
            FolTerm::Lt(lhs, rhs) => Func::compose(
                Func::Lt,
                Func::construction(vec![lhs.to_func(), rhs.to_func()]),
            ),
            FolTerm::Gt(lhs, rhs) => Func::compose(
                Func::Gt,
                Func::construction(vec![lhs.to_func(), rhs.to_func()]),
            ),
            FolTerm::Le(lhs, rhs) => Func::compose(
                Func::Le,
                Func::construction(vec![lhs.to_func(), rhs.to_func()]),
            ),
            FolTerm::Ge(lhs, rhs) => Func::compose(
                Func::Ge,
                Func::construction(vec![lhs.to_func(), rhs.to_func()]),
            ),

            FolTerm::Var(_) => Func::Id,
            // 1-indexed role n on the fact bound to var (matches
            // FORML 2 "role n" verbal convention). Engine fact
            // shape: `<<key1, val1>, <key2, val2>, ...>`.
            // Pick pair n, then its value (Selector 2).
            FolTerm::RoleVal(_, n) => Func::compose(
                Func::Selector(2),
                Func::Selector(n),
            ),
            FolTerm::Const(obj) => Func::constant(obj),

            FolTerm::Raw(f) => f,
        }
    }
}

/// Quantifier-via-Insert lowering with a unit-element prepend so
/// that empty-source quantifiers reduce to the right boolean.
///
/// Without the prepend, `Insert(op)` over an empty seq returns ⊥
/// (Backus's Insert is undefined on `<>`), but FOL semantics
/// require ∀ over an empty domain to be `True` and ∃ to be `False`.
/// `apndl:[unit, α(body):facts]` ensures the input to Insert is
/// never empty — at minimum it's `<unit>`, and Insert on a
/// single-element seq returns the element. For populated sources
/// the prepended unit is absorbed by the operator (T ∧ x = x,
/// F ∨ x = x), so semantics stay correct.
fn quantifier(unit: Object, op: Func, body: Func, source: Func) -> Func {
    Func::compose(
        Func::insert(op),
        Func::compose(
            Func::ApndL,
            Func::construction(vec![
                Func::constant(unit),
                Func::compose(Func::apply_to_all(body), source),
            ]),
        ),
    )
}

/// Build a Func that fetches the facts of a given source from D.
///
/// `Single(ft)`  →  `FetchOrPhi ∘ <ft, D>`
/// `Union(ids)` →  flattened concat of each ft's facts.
fn facts_of(source: &FactSource) -> Func {
    match source {
        FactSource::Single(ft_id) => Func::compose(
            Func::FetchOrPhi,
            Func::construction(vec![
                Func::constant(Object::atom(ft_id)),
                Func::Id,
            ]),
        ),
        FactSource::Union(ids) => {
            // Build [facts(id_0), facts(id_1), ...] then flatten.
            // For now Union with a single id degrades to Single.
            if ids.len() == 1 {
                return facts_of(&FactSource::Single(ids[0].clone()));
            }
            let parts: Vec<Func> = ids
                .iter()
                .map(|id| facts_of(&FactSource::Single(id.clone())))
                .collect();
            // Flatten via Compact ∘ Construction — Compact removes
            // ⊥ entries the per-id facts may produce when a fact
            // type has no population.
            Func::compose(Func::Compact, Func::construction(parts))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{self, apply, Object};

    fn t() -> Object { Object::t() }
    fn f() -> Object { Object::f() }

    #[test]
    fn truth_values_lower_directly() {
        let phi = Object::phi();
        assert_eq!(apply(&FolTerm::True.to_func(), &phi, &phi), t());
        assert_eq!(apply(&FolTerm::False.to_func(), &phi, &phi), f());
    }

    #[test]
    fn and_empty_is_true_and_single_passes_through() {
        let phi = Object::phi();
        assert_eq!(apply(&FolTerm::And(vec![]).to_func(), &phi, &phi), t());
        assert_eq!(
            apply(&FolTerm::And(vec![FolTerm::True]).to_func(), &phi, &phi),
            t(),
        );
    }

    #[test]
    fn or_empty_is_false_and_single_passes_through() {
        let phi = Object::phi();
        assert_eq!(apply(&FolTerm::Or(vec![]).to_func(), &phi, &phi), f());
        assert_eq!(
            apply(&FolTerm::Or(vec![FolTerm::False]).to_func(), &phi, &phi),
            f(),
        );
    }

    #[test]
    fn and_combines_via_insert() {
        let phi = Object::phi();
        // True ∧ True  =  True
        let term = FolTerm::And(vec![FolTerm::True, FolTerm::True]);
        assert_eq!(apply(&term.to_func(), &phi, &phi), t());
        // True ∧ False =  False
        let term = FolTerm::And(vec![FolTerm::True, FolTerm::False]);
        assert_eq!(apply(&term.to_func(), &phi, &phi), f());
    }

    #[test]
    fn or_combines_via_insert() {
        let phi = Object::phi();
        // False ∨ True  =  True
        let term = FolTerm::Or(vec![FolTerm::False, FolTerm::True]);
        assert_eq!(apply(&term.to_func(), &phi, &phi), t());
        // False ∨ False =  False
        let term = FolTerm::Or(vec![FolTerm::False, FolTerm::False]);
        assert_eq!(apply(&term.to_func(), &phi, &phi), f());
    }

    #[test]
    fn not_negates() {
        let phi = Object::phi();
        assert_eq!(
            apply(&FolTerm::Not(Box::new(FolTerm::True)).to_func(), &phi, &phi),
            f(),
        );
        assert_eq!(
            apply(&FolTerm::Not(Box::new(FolTerm::False)).to_func(), &phi, &phi),
            t(),
        );
    }

    #[test]
    fn implies_lowers_to_or_not() {
        let phi = Object::phi();
        // True → True   =  True
        let term = FolTerm::Implies(
            Box::new(FolTerm::True),
            Box::new(FolTerm::True),
        );
        assert_eq!(apply(&term.to_func(), &phi, &phi), t());
        // True → False  =  False
        let term = FolTerm::Implies(
            Box::new(FolTerm::True),
            Box::new(FolTerm::False),
        );
        assert_eq!(apply(&term.to_func(), &phi, &phi), f());
        // False → False =  True (vacuous)
        let term = FolTerm::Implies(
            Box::new(FolTerm::False),
            Box::new(FolTerm::False),
        );
        assert_eq!(apply(&term.to_func(), &phi, &phi), t());
    }

    #[test]
    fn eq_compares_constants() {
        let phi = Object::phi();
        let three = Object::atom("3");
        // 3 = 3   →  True
        let term = FolTerm::Eq(
            Box::new(FolTerm::Const(three.clone())),
            Box::new(FolTerm::Const(three.clone())),
        );
        assert_eq!(apply(&term.to_func(), &phi, &phi), t());
        // 3 = 4   →  False
        let term = FolTerm::Eq(
            Box::new(FolTerm::Const(three)),
            Box::new(FolTerm::Const(Object::atom("4"))),
        );
        assert_eq!(apply(&term.to_func(), &phi, &phi), f());
    }

    #[test]
    fn forall_over_empty_source_is_true() {
        // ∀ f ∈ ft1. True   with no facts under "ft1"  →  True
        let phi = Object::phi();
        let term = FolTerm::ForAll(
            "f".into(),
            FactSource::Single("ft1".into()),
            Box::new(FolTerm::True),
        );
        assert_eq!(apply(&term.to_func(), &phi, &phi), t());
    }

    #[test]
    fn exists_over_empty_source_is_false() {
        // ∃ f ∈ ft1. True   with no facts under "ft1"  →  False
        let phi = Object::phi();
        let term = FolTerm::Exists(
            "f".into(),
            FactSource::Single("ft1".into()),
            Box::new(FolTerm::True),
        );
        assert_eq!(apply(&term.to_func(), &phi, &phi), f());
    }

    #[test]
    fn forall_with_populated_source_evaluates_body() {
        // Build a state with two facts under ft1, each with role 1
        // = "x". Body: role 1 = "x".  ∀ f. role1(f) = "x"  →  True
        let mut state = Object::phi();
        state = ast::cell_push("ft1", ast::fact_from_pairs(&[("a", "x")]), &state);
        state = ast::cell_push("ft1", ast::fact_from_pairs(&[("b", "x")]), &state);
        let term = FolTerm::ForAll(
            "f".into(),
            FactSource::Single("ft1".into()),
            Box::new(FolTerm::Eq(
                Box::new(FolTerm::RoleVal("f".into(), 1)),
                Box::new(FolTerm::Const(Object::atom("x"))),
            )),
        );
        assert_eq!(apply(&term.to_func(), &state, &state), t());
    }

    #[test]
    fn raw_escape_hatch_returns_inner() {
        let phi = Object::phi();
        let inner = Func::constant(Object::atom("answer"));
        let term = FolTerm::Raw(inner);
        assert_eq!(apply(&term.to_func(), &phi, &phi), Object::atom("answer"));
    }
}
