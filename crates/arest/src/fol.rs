// crates/arest/src/fol.rs
//
// First-Order Logic intermediate representation between parse (Φ
// cells) and compile (Func trees). The compile half of Theorem 2's
// `parse: R → Φ` / `compile: Φ → O` pipeline. Closes #357.
//
// Why FolTerm exists:
//
// `compile.rs` had ~17 functions (compile_uniqueness_ast,
// compile_ring_irreflexive_ast, compile_subset_ast, ...) each
// hand-translating a constraint kind directly to Func. That's a
// hand-rolled FOL→FFP compiler, distributed across 17 sites with
// no shared algebra. Adding a new constraint kind meant adding a
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
// What shipped (compile half — #357 closed):
//
//   * `enum FolTerm` — variants for boolean combinators, FOL
//     quantifiers, atomic predicates, terms, plus `FactRole` for
//     "role N of the fact reached by accessor F" (the commonest
//     shape in ring / UC / MC / FC / SS / EQ predicates) and
//     `Raw` as a fall-through for arbitrary Func.
//   * `FactSource::{Single, Union}` — what a quantifier ranges
//     over. `Union` flattens via `Concat` (fixed from an initial
//     `Compact` bug in 7d25dec).
//   * `to_func(self) -> Func` lowering. Quantifier lowering uses
//     Backus's Insert form (∀f.P(f) → Insert(And)∘α(P)) with an
//     ApndL unit prepend so empty-source quantifiers reduce to
//     the right boolean per FOL semantics.
//   * 15 of 17 constraint / derivation compile sites rewired:
//     IR, AS, SY, AT, IT, TR, RF, UC, MC, FC, SS, EQ, set_comparison
//     (XO/XC/OR), explicit-derivation dedup, join-derivation
//     join_pred. AC skipped (Platform("tc_cycles") primitive);
//     VC skipped (requires Var scope resolution — open for a
//     follow-up wave).
//   * Raw-wrap reduction: 35 → 8 residue (77%). Remaining Raws
//     are legitimate: 1 outer-instance accessor, 6 key/val tuple-
//     field accesses in MC/set_comparison binding_match (not the
//     `compose(role_value, accessor)` shape FactRole captures),
//     1 Contains wrap in join derivation.
//   * 18 unit tests covering truth values, boolean combinators,
//     quantifiers (empty + populated), Lt/Gt/Le/Ge, Implies
//     vacuous cases, Union multi-id flattening, FactRole across
//     selector shapes, nested-quantifier #[ignore] stub
//     documenting the Var scope-resolution follow-up, and an
//     And/Or fold-identity property test.
//
// What's deliberately NOT in #357's scope (separate future work):
//
//   * Parse half: `parse_forml2 → FolTerm` directly (today parse
//     still produces ConstraintDef/DerivationRuleDef structs,
//     which compile_*_ast then translates into FolTerm). The
//     compile half's success validates the IR shape; the parse-
//     side reshape would make Theorem 2's pipeline literal in
//     code (R → Φ → O with Φ as FolTerm throughout).
//   * `Var` scope resolution — today `Var(x) → Func::Id`, correct
//     only for the innermost binding. Narrowed docstring
//     documents the gap; nested-quantifier test is #[ignore]'d.
//     Fix unlocks the VC rewire + cleaner ring/join shapes.
//   * Optimisation passes over FolTerm (Backus §12 distributivity,
//     redundant quantifier elimination).
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
    /// Variable reference — shorthand for the innermost
    /// enclosing quantifier's binding. Lowers to `Func::Id`, so
    /// semantically *only* the innermost binding is addressable
    /// today; nested quantifiers with `Var("outer")` silently
    /// resolve to the inner binding and produce wrong Funcs.
    ///
    /// When a rewire needs true scope resolution (e.g. the ring
    /// shortcut-match predicate reaching across `<<f1, f2>, cand>`
    /// wants to name `f1`/`f2`/`cand` independently), build the
    /// accessor explicitly via `FactRole { fact: <selector>, … }`
    /// or `Raw(<selector>)` instead of relying on `Var`. Fixing
    /// this to thread a real scope stack is a follow-up tracked
    /// under the #357 review findings.
    Var(VarName),
    /// `var.role(n)` — the n'th role's value of the fact bound to
    /// `var`. Lowers to a `Selector(n)` against the bound fact.
    RoleVal(VarName, RoleIdx),
    /// Role value of a fact reached via an arbitrary `Func` accessor.
    ///
    /// The ring / UC / MC / FC / SS / EQ compilers in compile.rs all
    /// run over input shapes like `<fact, candidate>` or
    /// `<<f1, f2>, candidate>`, where the fact(s) are pulled out with
    /// a composition of `Func::Selector(_)`s rather than sitting at
    /// "self". Prior to this variant each site wrote
    /// `FolTerm::Raw(Func::compose(role_value(n), accessor))`, the
    /// biggest class of `Raw` wraps in the codebase. `FactRole`
    /// lifts that pattern into a first-class atom so the surrounding
    /// FolTerm shape stays non-Raw and readable.
    ///
    /// `fact` is a Func (not a FolTerm) because the accessors in
    /// question — `Selector(1)`, `Selector(2)`, or small compositions
    /// of them — already exist as Func values at every call site;
    /// re-wrapping in FolTerm::Raw is bureaucratic. The review
    /// (ac474845) flagged this variant as the biggest reach-per-
    /// effort Raw-debt follow-up.
    ///
    /// Role index is 1-indexed, matching `RoleVal`.
    FactRole { fact: Func, role: RoleIdx },
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
            // "role `role` of the fact `fact` reaches" — first run
            // `fact` to produce the fact (a seq of <key, val>
            // pairs), then extract pair `role`'s value. Shape:
            // `Func::compose(role_extractor, fact)` — read right to
            // left, the fact accessor runs first (innermost).
            FolTerm::FactRole { fact, role } => Func::compose(
                Func::compose(Func::Selector(2), Func::Selector(role)),
                fact,
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
            // Flatten via Concat ∘ Construction — each `parts[i]`
            // is a seq of facts for a single fact type, so Concat
            // turns `<seq_ft1, seq_ft2, ...>` into the flat seq
            // of all facts. Missing cells surface as phi (empty
            // seq) through FetchOrPhi, which Concat absorbs
            // cleanly. Compact would only drop ⊥ elements — it
            // does not flatten — which leaves a seq-of-seqs that
            // breaks any per-fact RoleVal access downstream.
            Func::compose(Func::Concat, Func::construction(parts))
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
    fn lt_gt_le_ge_lower_via_construction_and_primitive() {
        let phi = Object::phi();
        let two = FolTerm::Const(Object::atom("2"));
        let three = FolTerm::Const(Object::atom("3"));

        // Lt: 2 < 3 -> True, 3 < 3 -> False
        assert_eq!(
            apply(&FolTerm::Lt(Box::new(two.clone()), Box::new(three.clone())).to_func(), &phi, &phi),
            t(),
        );
        assert_eq!(
            apply(&FolTerm::Lt(Box::new(three.clone()), Box::new(three.clone())).to_func(), &phi, &phi),
            f(),
        );

        // Gt: 3 > 2 -> True, 2 > 2 -> False
        assert_eq!(
            apply(&FolTerm::Gt(Box::new(three.clone()), Box::new(two.clone())).to_func(), &phi, &phi),
            t(),
        );
        assert_eq!(
            apply(&FolTerm::Gt(Box::new(two.clone()), Box::new(two.clone())).to_func(), &phi, &phi),
            f(),
        );

        // Le: 2 <= 2 -> True, 3 <= 2 -> False
        assert_eq!(
            apply(&FolTerm::Le(Box::new(two.clone()), Box::new(two.clone())).to_func(), &phi, &phi),
            t(),
        );
        assert_eq!(
            apply(&FolTerm::Le(Box::new(three.clone()), Box::new(two.clone())).to_func(), &phi, &phi),
            f(),
        );

        // Ge: 3 >= 3 -> True, 2 >= 3 -> False
        assert_eq!(
            apply(&FolTerm::Ge(Box::new(three.clone()), Box::new(three)).to_func(), &phi, &phi),
            t(),
        );
        assert_eq!(
            apply(&FolTerm::Ge(Box::new(two.clone()), Box::new(FolTerm::Const(Object::atom("3")))).to_func(), &phi, &phi),
            f(),
        );
    }

    #[test]
    fn implies_false_to_true_is_vacuously_true() {
        // The review flagged that `implies_lowers_to_or_not` only
        // covered `True -> True`, `True -> False`, and `False -> False`
        // — missing the `False -> True = True` vacuous case that
        // distinguishes implication from conjunction.
        let phi = Object::phi();
        let term = FolTerm::Implies(
            Box::new(FolTerm::False),
            Box::new(FolTerm::True),
        );
        assert_eq!(apply(&term.to_func(), &phi, &phi), t());
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
    fn forall_union_multi_id_walks_every_fact() {
        // ∀ f ∈ ft1 ∪ ft2. role_1(f) = "x"
        // Facts in both fact types have role 1 = "x"; the union
        // must reach every individual fact, not the per-ft seqs.
        // A Compact-based lowering would leave a seq-of-seqs and
        // RoleVal would hit the wrong shape. The correct lowering
        // uses Concat to flatten.
        let mut state = Object::phi();
        state = ast::cell_push("ft1", ast::fact_from_pairs(&[("a", "x")]), &state);
        state = ast::cell_push("ft2", ast::fact_from_pairs(&[("b", "x")]), &state);
        let term = FolTerm::ForAll(
            "f".into(),
            FactSource::Union(vec!["ft1".into(), "ft2".into()]),
            Box::new(FolTerm::Eq(
                Box::new(FolTerm::RoleVal("f".into(), 1)),
                Box::new(FolTerm::Const(Object::atom("x"))),
            )),
        );
        assert_eq!(apply(&term.to_func(), &state, &state), t());
    }

    #[test]
    fn exists_union_multi_id_flattens_before_predicate() {
        // ∃ f ∈ ft1 ∪ ft2. role_1(f) = "y"
        // Only ft2's fact has role 1 = "y"; ft1's fact has "x".
        // After flattening, Or over the per-fact predicate results
        // must pick up the single match.
        let mut state = Object::phi();
        state = ast::cell_push("ft1", ast::fact_from_pairs(&[("a", "x")]), &state);
        state = ast::cell_push("ft2", ast::fact_from_pairs(&[("b", "y")]), &state);
        let term = FolTerm::Exists(
            "f".into(),
            FactSource::Union(vec!["ft1".into(), "ft2".into()]),
            Box::new(FolTerm::Eq(
                Box::new(FolTerm::RoleVal("f".into(), 1)),
                Box::new(FolTerm::Const(Object::atom("y"))),
            )),
        );
        assert_eq!(apply(&term.to_func(), &state, &state), t());
    }

    #[test]
    fn fact_role_picks_role_from_chosen_fact() {
        // Input shape is `<fact1, fact2>`. FactRole's `fact`
        // accessor picks which one; its `role` picks which role
        // value to extract. This mirrors the ring / UC / SS / EQ
        // compile sites that used to wrap
        // `Func::compose(role_value(i), Func::Selector(n))` in
        // `FolTerm::Raw`.
        let fact1 = ast::fact_from_pairs(&[("a", "x"), ("b", "y")]);
        let fact2 = ast::fact_from_pairs(&[("c", "u"), ("d", "v")]);
        let input = Object::seq(vec![fact1, fact2]);
        let phi = Object::phi();

        // role 1 of fact1 = "x"
        let term = FolTerm::FactRole { fact: Func::Selector(1), role: 1 };
        assert_eq!(apply(&term.to_func(), &input, &phi), Object::atom("x"));

        // role 2 of fact1 = "y"
        let term = FolTerm::FactRole { fact: Func::Selector(1), role: 2 };
        assert_eq!(apply(&term.to_func(), &input, &phi), Object::atom("y"));

        // role 1 of fact2 = "u"
        let term = FolTerm::FactRole { fact: Func::Selector(2), role: 1 };
        assert_eq!(apply(&term.to_func(), &input, &phi), Object::atom("u"));

        // role 2 of fact2 = "v"
        let term = FolTerm::FactRole { fact: Func::Selector(2), role: 2 };
        assert_eq!(apply(&term.to_func(), &input, &phi), Object::atom("v"));
    }

    #[test]
    fn fact_role_with_nested_accessor_reaches_through_construction() {
        // Shape used by the ring transitivity shortcut-match
        // compiler: input is `<<f1, f2>, candidate>`, reaching f1
        // needs `Selector(1).Selector(1)` and reaching f2 needs
        // `Selector(2).Selector(1)`.
        let f1 = ast::fact_from_pairs(&[("r1", "a"), ("r2", "b")]);
        let f2 = ast::fact_from_pairs(&[("r1", "b"), ("r2", "c")]);
        let cand = ast::fact_from_pairs(&[("r1", "a"), ("r2", "c")]);
        let chain = Object::seq(vec![f1, f2]);
        let input = Object::seq(vec![chain, cand]);
        let phi = Object::phi();

        // role 1 of f1 (via <<f1,f2>, cand>) = "a"
        let term = FolTerm::FactRole {
            fact: Func::compose(Func::Selector(1), Func::Selector(1)),
            role: 1,
        };
        assert_eq!(apply(&term.to_func(), &input, &phi), Object::atom("a"));

        // role 2 of f2 = "c"
        let term = FolTerm::FactRole {
            fact: Func::compose(Func::Selector(2), Func::Selector(1)),
            role: 2,
        };
        assert_eq!(apply(&term.to_func(), &input, &phi), Object::atom("c"));
    }

    #[test]
    fn raw_escape_hatch_returns_inner() {
        let phi = Object::phi();
        let inner = Func::constant(Object::atom("answer"));
        let term = FolTerm::Raw(inner);
        assert_eq!(apply(&term.to_func(), &phi, &phi), Object::atom("answer"));
    }

    /// Review finding (ac474845): even a `#[ignore]` stub would
    /// document the gap. The foundation docstring (`fol.rs:118-130`
    /// on `FolTerm::Var`) explicitly flags that nested quantifiers
    /// with `Var("outer")` silently resolve to the *innermost*
    /// binding because lowering today just emits `Func::Id`.
    ///
    /// This test sets up `ForAll(x, ..., ForAll(y, ..., Eq(Var("x"),
    /// Var("y"))))` with a state where the outer and inner fact
    /// types have distinct role-1 atoms. Under correct scope
    /// resolution the inner body should compare the outer-bound
    /// variable to the inner-bound variable (so True iff outer == inner
    /// for every pair), which with disjoint role values is False for
    /// the cross terms and True only on the diagonal.
    ///
    /// Under the current `Var` lowering (`Func::Id`) both `Var("x")`
    /// and `Var("y")` resolve to the innermost bound fact, so the
    /// comparison degrades to `fact_y = fact_y` and always returns
    /// True — masking the bug.
    ///
    /// Until the `Var` → scope-stack rewire lands, this test stays
    /// `#[ignore]` so it documents the open work without failing
    /// the suite.
    #[test]
    #[ignore]
    fn nested_quantifier_resolves_outer_var() {
        // State: ft_outer has one fact with role 1 = "x";
        //        ft_inner has two facts with role 1 = "x" and "y".
        let mut state = Object::phi();
        state = ast::cell_push("ft_outer", ast::fact_from_pairs(&[("o", "x")]), &state);
        state = ast::cell_push("ft_inner", ast::fact_from_pairs(&[("i", "x")]), &state);
        state = ast::cell_push("ft_inner", ast::fact_from_pairs(&[("i", "y")]), &state);

        // ∀ x ∈ ft_outer. ∀ y ∈ ft_inner. role_1(x) = role_1(y)
        // Correct semantics: outer role_1 is "x"; inner role_1 is
        // "x" for one fact and "y" for the other. ∀-over-inner
        // requires every pairing to hold, and "x" ≠ "y" for one of
        // the pairs, so the whole expression is False.
        let term = FolTerm::ForAll(
            "x".into(),
            FactSource::Single("ft_outer".into()),
            Box::new(FolTerm::ForAll(
                "y".into(),
                FactSource::Single("ft_inner".into()),
                Box::new(FolTerm::Eq(
                    Box::new(FolTerm::RoleVal("x".into(), 1)),
                    Box::new(FolTerm::RoleVal("y".into(), 1)),
                )),
            )),
        );
        assert_eq!(apply(&term.to_func(), &state, &state), f());
    }

    /// Review finding (ac474845): "one quickcheck closure catches
    /// `Insert`-over-empty edge cases better than adding 6 one-off
    /// variant tests."
    ///
    /// This is a hand-rolled property check (no quickcheck crate)
    /// over a small set of FolTerm shapes. It verifies the semantic
    /// identity:
    ///
    ///   And(xs).to_func().evaluate(state)
    ///     == T iff every x in xs lowers to T
    ///
    /// and likewise for Or. The shape set covers:
    ///
    ///   * empty   — Backus's Insert is undefined on `<>`, so
    ///     lowering must emit the unit element directly
    ///     (`And` → T, `Or` → F).
    ///   * singleton — lowering passes the element through; no
    ///     Insert wrapper.
    ///   * N-ary (2+ elements) — Insert-over-Construction path,
    ///     mixing T/F inputs to exercise both early- and late-
    ///     terminating fold positions.
    #[test]
    fn and_or_lowerings_match_fold_identity() {
        let phi = Object::phi();
        let t_obj = t();

        // Hand-curated shape set. Each element reduces to T or F
        // via its `to_func()`, so we can compute the expected
        // conjunction / disjunction directly with `.all` / `.any`.
        let shape_sets: Vec<Vec<FolTerm>> = vec![
            // empty
            vec![],
            // singletons
            vec![FolTerm::True],
            vec![FolTerm::False],
            // 2-ary, both truth polarities
            vec![FolTerm::True, FolTerm::True],
            vec![FolTerm::True, FolTerm::False],
            vec![FolTerm::False, FolTerm::True],
            vec![FolTerm::False, FolTerm::False],
            // 3-ary mix — exercises the Construction path
            vec![FolTerm::True, FolTerm::True, FolTerm::True],
            vec![FolTerm::True, FolTerm::False, FolTerm::True],
            vec![FolTerm::False, FolTerm::False, FolTerm::True],
            // 5-ary — beyond the 2/3-arg cases the unit tests above
            // lock in, confirming the fold extends uniformly.
            vec![
                FolTerm::True,
                FolTerm::True,
                FolTerm::True,
                FolTerm::True,
                FolTerm::True,
            ],
            vec![
                FolTerm::True,
                FolTerm::True,
                FolTerm::False,
                FolTerm::True,
                FolTerm::True,
            ],
        ];

        for xs in shape_sets {
            // Expected values via Rust's own iterator fold over each
            // element's lowered evaluation.
            let per_elem: Vec<Object> = xs
                .iter()
                .map(|x| apply(&x.clone().to_func(), &phi, &phi))
                .collect();
            let expected_and = if per_elem.iter().all(|v| *v == t_obj) { t() } else { f() };
            let expected_or = if per_elem.iter().any(|v| *v == t_obj) { t() } else { f() };

            let and_result = apply(&FolTerm::And(xs.clone()).to_func(), &phi, &phi);
            let or_result = apply(&FolTerm::Or(xs.clone()).to_func(), &phi, &phi);

            assert_eq!(
                and_result, expected_and,
                "And fold identity violated for shape {:?}",
                xs,
            );
            assert_eq!(
                or_result, expected_or,
                "Or fold identity violated for shape {:?}",
                xs,
            );
        }
    }
}
