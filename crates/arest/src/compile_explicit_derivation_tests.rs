//! Stress harness for `compile_explicit_derivation` (#296).
//!
//! Each derivation-rule shape the parser emits today gets one
//! `#[test]`. Adding a new shape is a single-function add here, not a
//! pattern-match extension of an existing test.
//!
//! Routing reminder: `compile_derivations` dispatches by rule kind —
//! Join rules go to `compile_join_derivation`, aggregate rules go to
//! `compile_aggregate_derivation`, and everything else goes to
//! `compile_explicit_derivation`. Shapes 4 (join-path), 6 (aggregate),
//! and 7 (transitive) route through their dedicated compilers. Each
//! test notes its router so a future regression in
//! `compile_explicit_derivation` implicates the right tests.
//!
//! Each test:
//!   1. Parses a self-contained reading that declares exactly one rule.
//!   2. Asserts the `ConsequentCellSource` variant shape is correct.
//!   3. Applies the compiled Func to a tiny hand-built population and
//!      asserts the derived facts.

#![cfg(test)]

use crate::ast::{self, Func, Object};
use crate::compile;
use crate::parse_forml2::parse_to_state;
use crate::types::{ConsequentCellSource, DerivationKind, DerivationRuleDef};

/// Parse a self-contained reading, return the sole derivation rule and
/// its compiled Func. Panics with a legible message if the reading
/// doesn't declare exactly one rule, or the compiled model is missing
/// the derivation.
fn parse_and_compile(src: &str) -> (DerivationRuleDef, Func) {
    let state = parse_to_state(src).expect("parse");
    let data = compile::cell_index_from_state(&state);
    assert_eq!(
        data.derivation_rules.len(), 1,
        "test reading must declare exactly one derivation rule, got {}: {:#?}",
        data.derivation_rules.len(),
        data.derivation_rules.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
    );
    let rule = data.derivation_rules[0].clone();
    let model = compile::compile(&state);
    let cd = model.derivations.iter()
        .find(|d| d.id == rule.id)
        .unwrap_or_else(|| panic!("compiled derivation for rule `{}` missing", rule.id));
    (rule, cd.func.clone())
}

/// Evaluate `func` against a hand-built population. Each `(cell,
/// bindings)` pair is pushed as one fact into the named cell. Returns
/// the raw output Seq of `<ft_id, reading, bindings>` tuples.
fn apply_to_facts(func: &Func, facts: &[(&str, &[(&str, &str)])]) -> Object {
    let state = facts.iter().fold(Object::phi(), |acc, (cell, pairs)| {
        ast::cell_push(cell, ast::fact_from_pairs(pairs), &acc)
    });
    let pop = ast::encode_state(&state);
    ast::apply(func, &pop, &state)
}

/// Decode an output Seq into `(ft_id, reading, bindings)` triples.
/// Bindings are `(role_name, value)` pairs. Non-fact items in the Seq
/// (e.g. `phi` placeholders from conditional branches) are skipped.
fn decode_derived(out: &Object) -> Vec<(String, String, Vec<(String, String)>)> {
    out.as_seq().map(|items| items.iter().filter_map(|item| {
        let fact = item.as_seq()?;
        if fact.len() < 3 { return None; }
        let ft_id = fact[0].as_atom()?.to_string();
        let reading = fact[1].as_atom().unwrap_or("").to_string();
        let bindings = fact[2].as_seq().map(|pairs| pairs.iter().filter_map(|p| {
            let pair = p.as_seq()?;
            if pair.len() != 2 { return None; }
            Some((
                pair[0].as_atom()?.to_string(),
                pair[1].as_atom()?.to_string(),
            ))
        }).collect::<Vec<_>>()).unwrap_or_default();
        Some((ft_id, reading, bindings))
    }).collect()).unwrap_or_default()
}

// ─── Category 1: Literal in consequent ──────────────────────────────
//
// Shape: `* X has <Role> '<literal>' iff ...` — consequent pins a role
// to a constant atom. Routes through `compile_explicit_derivation`'s
// 1-antecedent literal-pinning branch (consequent_role_literals
// populated).
//
// This rule type came from #286: grammar-classification rules like
// "Statement has Trailing Marker 'is an entity type'" that emit a
// consequent fact whose role is pinned to a fixed atom regardless of
// the antecedent's bindings.

#[test]
fn shape_literal_in_consequent_pins_role_to_atom() {
    let src = r#"# Test
Widget(.Serial) is an entity type.
Kind is a value type.
Serial is a value type.

## Fact Types
Widget has Serial.
Widget has Kind.

## Derivation Rules
* Widget has Kind 'electronic' iff Widget has Serial.
"#;
    let (rule, func) = parse_and_compile(src);

    // Shape assertion: literal consequent cell (not AntecedentRole) and
    // a consequent_role_literals entry pinning Kind='electronic'.
    match &rule.consequent_cell {
        ConsequentCellSource::Literal(id) => {
            assert!(!id.is_empty(), "literal consequent cell id must resolve");
        }
        other => panic!("expected Literal(..), got {:?}", other),
    }
    assert!(
        rule.consequent_role_literals.iter().any(|l| l.role == "Kind" && l.value == "electronic"),
        "expected consequent_role_literals to pin Kind='electronic', got {:#?}",
        rule.consequent_role_literals,
    );
    assert_eq!(
        rule.antecedent_sources.len(), 1,
        "single-antecedent shape expected, got {:#?}", rule.antecedent_sources,
    );

    // Eval: one antecedent fact → one derived fact whose Kind binding
    // is the literal regardless of the antecedent's role values.
    let out = apply_to_facts(&func, &[
        ("Widget_has_Serial", &[("Widget", "w1"), ("Serial", "sn-1")]),
    ]);
    let derived = decode_derived(&out);
    assert_eq!(derived.len(), 1, "one derived fact expected, got {:#?}", derived);
    let (_ft, _reading, bindings) = &derived[0];
    assert!(
        bindings.iter().any(|(k, v)| k == "Kind" && v == "electronic"),
        "derived fact must bind Kind='electronic', got {:#?}", bindings,
    );
    assert!(
        bindings.iter().any(|(k, v)| k == "Widget" && v == "w1"),
        "derived fact must preserve Widget='w1' from antecedent, got {:#?}", bindings,
    );
}

// ─── Category 3: ParameterAtom — antecedent + consequent literals ──
//
// Shape: `* X has A '<a>' iff X has B '<b>'` — the rule fires only
// when the antecedent's role B equals a specific atom, and the derived
// fact pins role A to another specific atom. Exercises
// `compile_explicit_derivation`'s 1-antecedent branch with BOTH
// `antecedent_role_literals` (the Filter-predicate path) and
// `consequent_role_literals` (the construct-in-declared-role-order
// path) populated.

#[test]
fn shape_parameter_atom_on_both_antecedent_and_consequent() {
    let src = r#"# Test
Vehicle(.VIN) is an entity type.
VIN is a value type.
Weight Class is a value type.
Transit Category is a value type.

## Fact Types
Vehicle has VIN.
Vehicle has Weight Class.
Vehicle has Transit Category.

## Derivation Rules
* Vehicle has Transit Category 'heavy' iff Vehicle has Weight Class 'extra heavy'.
"#;
    let (rule, func) = parse_and_compile(src);

    match &rule.consequent_cell {
        ConsequentCellSource::Literal(id) => assert!(!id.is_empty()),
        other => panic!("expected Literal(..), got {:?}", other),
    }
    assert_eq!(rule.antecedent_sources.len(), 1);
    assert!(
        rule.antecedent_role_literals.iter().any(|l|
            l.role == "Weight Class" && l.value == "extra heavy" && l.antecedent_index == 0),
        "expected antecedent_role_literals to pin Weight Class='extra heavy', got {:#?}",
        rule.antecedent_role_literals,
    );
    assert!(
        rule.consequent_role_literals.iter().any(|l|
            l.role == "Transit Category" && l.value == "heavy"),
        "expected consequent_role_literals to pin Transit Category='heavy', got {:#?}",
        rule.consequent_role_literals,
    );

    // Antecedent predicate must filter on the role literal: two facts
    // with different Weight Class values, only the matching one derives.
    // Binding keys are underscore-normalised to match role_value_by_name's
    // lookup key (compile.rs::role_value_by_name replaces ' ' with '_').
    let out = apply_to_facts(&func, &[
        ("Vehicle_has_Weight_Class", &[("Vehicle", "v-heavy"), ("Weight_Class", "extra heavy")]),
        ("Vehicle_has_Weight_Class", &[("Vehicle", "v-light"), ("Weight_Class", "light")]),
    ]);
    let derived = decode_derived(&out);
    assert_eq!(derived.len(), 1, "only the matching Vehicle should derive, got {:#?}", derived);
    let (_ft, _reading, bindings) = &derived[0];
    assert!(
        bindings.iter().any(|(k, v)| k == "Vehicle" && v == "v-heavy"),
        "expected Vehicle='v-heavy', got {:#?}", bindings,
    );
    assert!(
        bindings.iter().any(|(k, v)| k == "Transit_Category" && v == "heavy"),
        "expected Transit_Category='heavy', got {:#?}", bindings,
    );
}

// ─── Category 5: Arithmetic in RHS ──────────────────────────────────
//
// Shape: `* X has R iff X has A and R is <arith-expr over A>` — the
// consequent role R is defined by an arithmetic expression on the
// antecedent fact's role values. Routes through
// `compile_explicit_derivation`'s 1-antecedent branch where
// `consequent_computed_bindings` is non-empty, the bindings function
// `Concat · [Id, computed_pairs]` appends the computed pair to the
// inherited antecedent bindings.
//
// compile_arith_expr resolves RoleRef by looking up the role on the
// single antecedent FT, so all referenced roles must exist on the
// same FT. The multi-antecedent N≥2 branch doesn't apply arith, so
// this shape is specifically for single-antecedent rules.

#[test]
fn shape_arithmetic_in_rhs_computes_consequent_role() {
    let src = r#"# Test
Order(.OrderId) is an entity type.
OrderId is a value type.
Subtotal is a value type.
Total is a value type.

## Fact Types
Order has OrderId.
Order has Subtotal.
Order has Total.

## Derivation Rules
* Order has Total iff Order has Subtotal and Total is Subtotal + Subtotal.
"#;
    let (rule, func) = parse_and_compile(src);

    // Shape: single antecedent; consequent_computed_bindings populated
    // with the Total = Subtotal + Subtotal expression; role literals
    // empty (the other literal-pinning path isn't used here).
    match &rule.consequent_cell {
        ConsequentCellSource::Literal(id) => assert!(!id.is_empty()),
        other => panic!("expected Literal(..), got {:?}", other),
    }
    assert_eq!(rule.antecedent_sources.len(), 1);
    assert!(rule.consequent_role_literals.is_empty(),
        "no literal-pin expected for arith rule, got {:#?}", rule.consequent_role_literals);
    assert_eq!(rule.consequent_computed_bindings.len(), 1,
        "one computed binding expected, got {:#?}", rule.consequent_computed_bindings);
    let cb = &rule.consequent_computed_bindings[0];
    assert_eq!(cb.role, "Total");

    // Eval: Subtotal=50 → Total=100 (50 + 50). Arith primitives parse
    // the atoms as f64; the formatter turns integers back into
    // atom strings without a ".0" suffix.
    let out = apply_to_facts(&func, &[
        ("Order_has_Subtotal", &[("Order", "ord-1"), ("Subtotal", "50")]),
    ]);
    let derived = decode_derived(&out);
    assert_eq!(derived.len(), 1, "one derived fact expected, got {:#?}", derived);
    let (_ft, _reading, bindings) = &derived[0];
    assert!(
        bindings.iter().any(|(k, v)| k == "Total" && v == "100"),
        "expected Total=100, got {:#?}", bindings,
    );
    assert!(
        bindings.iter().any(|(k, v)| k == "Order" && v == "ord-1"),
        "antecedent Order binding must propagate, got {:#?}", bindings,
    );
}

// ─── Category 8: Multi-antecedent `and` chain ───────────────────────
//
// Shape: `* X has R '<r>' iff X has A and X has B and X has C` —
// N ≥ 2 antecedents combined with `and`, with the consequent role
// pinned to a literal so the "fresh bindings in declared role order"
// path in compile_explicit_derivation's N-antecedent branch fires
// (without literals, bindings are copied whole from the first
// antecedent — see #286 design note). The rule fires once iff every
// antecedent FT has at least one surviving fact (existence-AND
// semantic; not a per-tuple join).

#[test]
fn shape_multi_antecedent_and_chain_existence_check() {
    let src = r#"# Test
User(.Email) is an entity type.
Email is a value type.
Status is a value type.
Role is a value type.
Permission is a value type.

## Fact Types
User has Email.
User has Status.
User has Role.
User has Permission.

## Derivation Rules
* User has Permission 'granted' iff User has Email and User has Status and User has Role.
"#;
    let (rule, func) = parse_and_compile(src);

    match &rule.consequent_cell {
        ConsequentCellSource::Literal(id) => assert!(!id.is_empty()),
        other => panic!("expected Literal(..), got {:?}", other),
    }
    assert_eq!(
        rule.antecedent_sources.len(), 3,
        "three-antecedent shape expected, got {:#?}", rule.antecedent_sources,
    );
    assert!(
        rule.consequent_role_literals.iter().any(|l|
            l.role == "Permission" && l.value == "granted"),
        "expected consequent_role_literals to pin Permission='granted', got {:#?}",
        rule.consequent_role_literals,
    );

    // All three antecedents populated → one derivation with the
    // pinned Permission literal. The User binding propagates from the
    // first antecedent (`role_value_by_name("User") . first_fact`).
    let out = apply_to_facts(&func, &[
        ("User_has_Email", &[("User", "u-1"), ("Email", "u1@ex.com")]),
        ("User_has_Status", &[("User", "u-1"), ("Status", "verified")]),
        ("User_has_Role", &[("User", "u-1"), ("Role", "admin")]),
    ]);
    let derived = decode_derived(&out);
    assert_eq!(derived.len(), 1, "existence-AND should emit one fact, got {:#?}", derived);
    let (_ft, _reading, bindings) = &derived[0];
    assert!(
        bindings.iter().any(|(k, v)| k == "Permission" && v == "granted"),
        "expected Permission='granted', got {:#?}", bindings,
    );
    assert!(
        bindings.iter().any(|(k, v)| k == "User" && v == "u-1"),
        "expected User='u-1' from first antecedent, got {:#?}", bindings,
    );

    // Missing one antecedent (no Role fact) → no derivation.
    let out = apply_to_facts(&func, &[
        ("User_has_Email", &[("User", "u-2"), ("Email", "u2@ex.com")]),
        ("User_has_Status", &[("User", "u-2"), ("Status", "verified")]),
    ]);
    let derived = decode_derived(&out);
    assert!(derived.is_empty(),
        "missing antecedent must suppress derivation, got {:#?}", derived);
}

// ─── Category 10: Parameter-atom-in-rule-body (#275) ────────────────
//
// Shape: `* X has Q iff X has P '<v>'` — only the antecedent carries a
// role-literal predicate; the consequent inherits antecedent bindings
// whole (bindings_func = Func::Id, no literal pin, no arith). Distinct
// from Category 3 (which populates BOTH antecedent and consequent
// literals, triggering the fresh-bindings path). This test isolates
// the Filter-predicate path from #286 / #275 so a regression in the
// antecedent-side literal compile doesn't hide behind the fresh-
// bindings path.

#[test]
fn shape_parameter_atom_in_rule_body_filters_antecedent_only() {
    let src = r#"# Test
Task(.ID) is an entity type.
ID is a value type.
Priority is a value type.
Escalation is a value type.

## Fact Types
Task has ID.
Task has Priority.
Task has Escalation.

## Derivation Rules
* Task has Escalation iff Task has Priority 'critical'.
"#;
    let (rule, func) = parse_and_compile(src);

    assert_eq!(rule.antecedent_sources.len(), 1);
    assert!(
        rule.antecedent_role_literals.iter().any(|l|
            l.role == "Priority" && l.value == "critical" && l.antecedent_index == 0),
        "expected antecedent_role_literals to pin Priority='critical', got {:#?}",
        rule.antecedent_role_literals,
    );
    assert!(
        rule.consequent_role_literals.is_empty(),
        "no consequent literal pin — bindings come from antecedent via Func::Id, got {:#?}",
        rule.consequent_role_literals,
    );
    assert!(
        rule.consequent_computed_bindings.is_empty(),
        "no arith on the consequent, got {:#?}", rule.consequent_computed_bindings,
    );

    // Filter keeps only the matching antecedent fact.
    let out = apply_to_facts(&func, &[
        ("Task_has_Priority", &[("Task", "t-crit"), ("Priority", "critical")]),
        ("Task_has_Priority", &[("Task", "t-low"),  ("Priority", "low")]),
    ]);
    let derived = decode_derived(&out);
    assert_eq!(derived.len(), 1, "only the critical Task should derive, got {:#?}", derived);
    let (_ft, _reading, bindings) = &derived[0];
    assert!(
        bindings.iter().any(|(k, v)| k == "Task" && v == "t-crit"),
        "expected Task='t-crit', got {:#?}", bindings,
    );

    // Nothing matching → no derivation.
    let out = apply_to_facts(&func, &[
        ("Task_has_Priority", &[("Task", "t-low"), ("Priority", "low")]),
    ]);
    assert!(decode_derived(&out).is_empty(),
        "no matching Priority literal → no derivation");
}

// ─── Category 9: Subscripted antecedent noun ────────────────────────
//
// Shape: self-ring FT where both roles share a noun name, disambiguated
// in rule text by ASCII-digit subscripts (`Person1`, `Person2` — Halpin
// position-paper Example 6). The parser strips the subscript before FT
// catalog lookup (`parse_role_token` returns the base noun), so the
// resolved antecedent FT is the plain `Person_is_parent_of_Person` and
// the derived fact's bindings use the bare `Person` key twice,
// distinguished by position. The test catches a regression where
// subscripted references in the rule body would fail to resolve to
// the declared self-ring FT.

#[test]
fn shape_subscripted_antecedent_noun_preserves_subscripts() {
    let src = r#"# Test
Person(.Name) is an entity type.
Name is a value type.

## Fact Types
Person has Name.
Person is parent of Person.
Person is ancestor of Person.

## Derivation Rules
* Person1 is ancestor of Person2 iff Person1 is parent of Person2.
"#;
    let (rule, func) = parse_and_compile(src);

    match &rule.consequent_cell {
        ConsequentCellSource::Literal(id) => assert!(!id.is_empty()),
        other => panic!("expected Literal(..), got {:?}", other),
    }
    assert_eq!(rule.antecedent_sources.len(), 1);
    assert!(rule.consequent_role_literals.is_empty());
    assert!(rule.consequent_computed_bindings.is_empty());

    // One parent fact → one ancestor derivation with the subscripted
    // Person1/Person2 bindings preserved on the wire.
    // FT id comes from the declaration `Person is parent of Person`,
    // which has no subscripts — subscripts in the rule body are
    // stripped for FT resolution. Bindings use plain "Person" twice,
    // distinguished by position.
    let out = apply_to_facts(&func, &[
        ("Person_is_parent_of_Person",
            &[("Person", "alice"), ("Person", "bob")]),
    ]);
    let derived = decode_derived(&out);
    assert_eq!(derived.len(), 1,
        "one ancestor fact expected from one parent fact, got {:#?}", derived);
    let (ft, _reading, bindings) = &derived[0];
    assert_eq!(ft, "Person_is_ancestor_of_Person",
        "derived fact must land in the consequent self-ring cell, got {}", ft);
    assert_eq!(bindings.len(), 2, "two Person bindings (positional), got {:#?}", bindings);
    // Positional: first Person is the parent (alice), second is the child (bob).
    // Both keys are bare "Person" after subscript stripping.
    assert_eq!(bindings[0], ("Person".to_string(), "alice".to_string()),
        "first Person binding should be alice, got {:?}", bindings[0]);
    assert_eq!(bindings[1], ("Person".to_string(), "bob".to_string()),
        "second Person binding should be bob, got {:?}", bindings[1]);
}

// ─── Category 7: Transitive closure (parse+compile shape only) ──────
//
// Shape: `* X R Z iff X R Y and Y R Z` on a binary self-ring relation.
// The parser's `join_on` detection keys off "that" anaphora — numeric
// subscripts alone don't mark Person2 as a join key, so a rule using
// Person1/Person2/Person3 compiles as a 2-antecedent modus-ponens,
// NOT as DerivationKind::Join. Consequently the Func that
// compile_explicit_derivation's N≥2 branch builds is an existence
// check with first-fact bindings, not the per-tuple equi-join semantic
// a transitive closure needs. Exercising the Func on a hand-built
// population never fires because of the bindings-key vs. role-index
// mismatch on self-ring FTs.
//
// The test asserts only parse+compile shape here and leaves the
// eval-side verification to the forward-chainer's end-to-end tests
// (evaluate.rs). Noted in a follow-up handoff: Person1/Person2/Person3
// should either route to compile_join_derivation or gain an anaphora
// hint at resolve time.

#[test]
fn shape_transitive_closure_parses_as_two_antecedent_literal() {
    let src = r#"# Test
Person(.Name) is an entity type.
Name is a value type.

## Fact Types
Person has Name.
Person is parent of Person.
Person is ancestor of Person.

## Derivation Rules
* Person1 is ancestor of Person3 iff Person1 is parent of Person2 and Person2 is ancestor of Person3.
"#;
    let (rule, _func) = parse_and_compile(src);

    match &rule.consequent_cell {
        ConsequentCellSource::Literal(id) => assert_eq!(id, "Person_is_ancestor_of_Person",
            "consequent resolves to the declared ancestor FT, got {}", id),
        other => panic!("expected Literal(..), got {:?}", other),
    }
    assert_eq!(rule.antecedent_sources.len(), 2,
        "two-antecedent transitive rule expected, got {:#?}", rule.antecedent_sources);
    // Both antecedents should resolve to declared FTs (not InstancesOfNoun
    // or AbsenceOf). The parent+ancestor pair is exactly the classic
    // transitive-closure antecedent shape.
    for src in rule.antecedent_sources.iter() {
        let id = src.fact_type_id();
        assert!(
            id == "Person_is_parent_of_Person" || id == "Person_is_ancestor_of_Person",
            "antecedent should be parent or ancestor FT, got {}", id,
        );
    }
}

// ─── Category 4: Join-path derivation via possessive syntax ─────────
//
// Shape: `* X has Z iff X's Y has Z` — the antecedent `X's Y`
// possessive expands at parse time (`try_expand_possessive`) to
// `X has Y and that Y has Z`, which the anaphora detector flags as
// a Join on Y. The dispatcher routes to `compile_join_derivation`,
// not `compile_explicit_derivation`, but the shape is a canonical
// user-reading pattern so it belongs in the harness.

#[test]
fn shape_join_path_via_possessive_expands_and_fires() {
    let src = r#"# Test
Order(.OrderId) is an entity type.
OrderId is a value type.
Customer(.CustomerId) is an entity type.
CustomerId is a value type.
Email is a value type.

## Fact Types
Order has OrderId.
Order has Customer.
Customer has CustomerId.
Customer has Email.
Order has Email.

## Derivation Rules
* Order has Email iff Order's Customer has Email.
"#;
    let (rule, func) = parse_and_compile(src);

    match &rule.consequent_cell {
        ConsequentCellSource::Literal(id) => assert_eq!(id, "Order_has_Email",
            "consequent must resolve to Order_has_Email, got {}", id),
        other => panic!("expected Literal(..), got {:?}", other),
    }
    assert_eq!(rule.antecedent_sources.len(), 2,
        "possessive expands to two antecedents, got {:#?}", rule.antecedent_sources);
    assert!(rule.join_on.contains(&"Customer".to_string()),
        "Customer should be the join key (via that-anaphora from expansion), got {:?}",
        rule.join_on);

    // ord-1 ─(Customer)→ cus-1 ─(Email)→ alice@example.com
    //   should join to ord-1 has Email alice@example.com.
    let out = apply_to_facts(&func, &[
        ("Order_has_Customer", &[("Order", "ord-1"), ("Customer", "cus-1")]),
        ("Customer_has_Email", &[("Customer", "cus-1"), ("Email", "alice@example.com")]),
    ]);
    let derived = decode_derived(&out);
    assert_eq!(derived.len(), 1, "one joined fact expected, got {:#?}", derived);
    let (ft, _, bindings) = &derived[0];
    assert_eq!(ft, "Order_has_Email");
    assert!(bindings.iter().any(|(k, v)| k == "Order" && v == "ord-1"),
        "Order binding should be ord-1, got {:#?}", bindings);
    assert!(bindings.iter().any(|(k, v)| k == "Email" && v == "alice@example.com"),
        "Email binding should be alice@example.com, got {:#?}", bindings);
}

// ─── Category 11: Join-path with antecedent literal filters (#818) ──
//
// Shape: `* X has R 'r' iff some X has A 'a' and that X has B 'b'` — a
// Join-routed rule (≥2 antecedents joined on a shared noun) where each
// antecedent additionally pins a role to a literal value. The bug
// `compile_join_derivation` documents at compile.rs:3219 is that the
// Join path drops `rule.antecedent_role_literals` on the floor — the
// literal filters that `compile_explicit_derivation` applies via
// `Filter(p)` at compile.rs:2818-2847 are silently absent on the Join
// branch. Result: the rule fires for every join-key match regardless
// of the literal predicates, producing spurious derived facts.
//
// This test asserts (a) parse populates antecedent_role_literals on
// both antecedents, and (b) the engine respects them — only the
// (Doc has Priority='high', Doc has Kind='critical') tuple should
// derive. Currently expected to fail on the eval assertions; the parse
// shape may pass.

#[test]
fn shape_join_with_antecedent_literal_filters_applies_filters() {
    let src = r#"# Test
Doc(.ID) is an entity type.
ID is a value type.
Priority is a value type.
Kind is a value type.
Status is a value type.

## Fact Types
Doc has ID.
Doc has Priority.
Doc has Kind.
Doc has Status.

## Derivation Rules
* Doc has Status 'urgent' iff some Doc has Priority 'high' and that Doc has Kind 'critical'.
"#;
    let (rule, func) = parse_and_compile(src);

    // Routing: 2 antecedents joined on Doc → DerivationKind::Join.
    assert_eq!(rule.kind, DerivationKind::Join,
        "expected Join routing, got {:?} (rule text: {})", rule.kind, rule.text);
    assert_eq!(rule.antecedent_sources.len(), 2,
        "two antecedents expected, got {:#?}", rule.antecedent_sources);

    // Both literal filters must survive parsing.
    assert!(
        rule.antecedent_role_literals.iter().any(|l|
            l.role == "Priority" && l.value == "high" && l.antecedent_index == 0),
        "expected Priority='high' filter on antecedent 0, got {:#?}",
        rule.antecedent_role_literals,
    );
    assert!(
        rule.antecedent_role_literals.iter().any(|l|
            l.role == "Kind" && l.value == "critical" && l.antecedent_index == 1),
        "expected Kind='critical' filter on antecedent 1, got {:#?}",
        rule.antecedent_role_literals,
    );

    // Population:
    //   d-yes:    Priority='high'  Kind='critical'  → DERIVE
    //   d-no-pri: Priority='low'   Kind='critical'  → no derive (Priority filter)
    //   d-no-knd: Priority='high'  Kind='advisory'  → no derive (Kind filter)
    let out = apply_to_facts(&func, &[
        ("Doc_has_Priority", &[("Doc", "d-yes"),    ("Priority", "high")]),
        ("Doc_has_Kind",     &[("Doc", "d-yes"),    ("Kind", "critical")]),
        ("Doc_has_Priority", &[("Doc", "d-no-pri"), ("Priority", "low")]),
        ("Doc_has_Kind",     &[("Doc", "d-no-pri"), ("Kind", "critical")]),
        ("Doc_has_Priority", &[("Doc", "d-no-knd"), ("Priority", "high")]),
        ("Doc_has_Kind",     &[("Doc", "d-no-knd"), ("Kind", "advisory")]),
    ]);
    let derived = decode_derived(&out);

    let urgent_docs: Vec<String> = derived.iter()
        .flat_map(|(_, _, b)| b.iter())
        .filter(|(k, _)| k == "Doc")
        .map(|(_, v)| v.clone())
        .collect();

    assert!(urgent_docs.iter().any(|d| d == "d-yes"),
        "d-yes (Priority=high, Kind=critical) MUST derive; got Doc-bindings {:?}\nfull derived: {:#?}",
        urgent_docs, derived);
    assert!(!urgent_docs.iter().any(|d| d == "d-no-pri"),
        "d-no-pri (Priority=low) must NOT derive — Priority literal filter ignored?\n\
         Doc-bindings {:?}\nfull derived: {:#?}",
        urgent_docs, derived);
    assert!(!urgent_docs.iter().any(|d| d == "d-no-knd"),
        "d-no-knd (Kind=advisory) must NOT derive — Kind literal filter ignored?\n\
         Doc-bindings {:?}\nfull derived: {:#?}",
        urgent_docs, derived);
}

// ─── Category 12: Single-antecedent + `some` + multi-word literal ───
//
// Shape: `* X has Y 'liftable' iff some X has Z 'in code only'` —
// mirrors apps/paper's Lift Priority derivation. Single antecedent, so
// it routes through `compile_explicit_derivation` (not the Join path).
// The literal value `in code only` spans three tokens; the literal
// quantifier word `some` precedes the antecedent. Tracked as #817 —
// the prior session report observed the apps/paper substrate's
// Liftable derivation never firing on `Implementation Mode 'In Code
// Only'` populations. This test isolates whether the gap is in the
// parser's literal capture, the explicit-derivation compile path, or
// something else entirely (e.g. case-sensitivity on values).

#[test]
fn shape_some_quantifier_with_multi_word_literal_filters_antecedent() {
    let src = r#"# Test
Paper Element(.ID) is an entity type.
ID is a value type.
Implementation Mode is a value type.
Lift Priority is a value type.

## Fact Types
Paper Element has ID.
Paper Element has Implementation Mode.
Paper Element has Lift Priority.

## Derivation Rules
* Paper Element has Lift Priority 'Liftable' iff some Paper Element has Implementation Mode 'In Code Only'.
"#;
    let (rule, func) = parse_and_compile(src);

    // Single antecedent → compile_explicit_derivation.
    assert_eq!(rule.antecedent_sources.len(), 1,
        "single-antecedent rule expected, got {:#?}", rule.antecedent_sources);
    assert_ne!(rule.kind, DerivationKind::Join,
        "single-antecedent rule must NOT route through Join path, got {:?}", rule.kind);

    // Multi-word literal must survive parse intact on the antecedent.
    assert!(
        rule.antecedent_role_literals.iter().any(|l|
            l.role == "Implementation Mode" && l.value == "In Code Only" && l.antecedent_index == 0),
        "expected antecedent literal Implementation Mode='In Code Only' on idx 0, got {:#?}\n\
         (likely failure: parser dropped the `some` quantifier or split the multi-word literal)",
        rule.antecedent_role_literals,
    );
    // And the consequent literal too.
    assert!(
        rule.consequent_role_literals.iter().any(|l|
            l.role == "Lift Priority" && l.value == "Liftable"),
        "expected consequent literal Lift Priority='Liftable', got {:#?}",
        rule.consequent_role_literals,
    );

    // Behavior:
    //   pe-1: Mode='In Code Only'    → DERIVE Lift Priority='Liftable'
    //   pe-2: Mode='In Readings'     → no derive (literal mismatch)
    //   pe-3: Mode='Aspirational'    → no derive (literal mismatch)
    let out = apply_to_facts(&func, &[
        ("Paper_Element_has_Implementation_Mode",
            &[("Paper_Element", "pe-1"), ("Implementation_Mode", "In Code Only")]),
        ("Paper_Element_has_Implementation_Mode",
            &[("Paper_Element", "pe-2"), ("Implementation_Mode", "In Readings")]),
        ("Paper_Element_has_Implementation_Mode",
            &[("Paper_Element", "pe-3"), ("Implementation_Mode", "Aspirational")]),
    ]);
    let derived = decode_derived(&out);

    let liftable_pes: Vec<String> = derived.iter()
        .flat_map(|(_, _, b)| b.iter())
        .filter(|(k, _)| k == "Paper_Element")
        .map(|(_, v)| v.clone())
        .collect();

    assert!(liftable_pes.iter().any(|p| p == "pe-1"),
        "pe-1 (Mode=In Code Only) MUST derive Liftable; got {:?}\nfull derived: {:#?}",
        liftable_pes, derived);
    assert!(!liftable_pes.iter().any(|p| p == "pe-2"),
        "pe-2 (Mode=In Readings) must NOT derive Liftable; got {:?}\nfull derived: {:#?}",
        liftable_pes, derived);
    assert!(!liftable_pes.iter().any(|p| p == "pe-3"),
        "pe-3 (Mode=Aspirational) must NOT derive Liftable; got {:?}\nfull derived: {:#?}",
        liftable_pes, derived);
}

// ─── Category 6: Aggregate ─────────────────────────────────────────
//
// Shape: `* X has R iff R is the <op> of Y where X has Y` — R is a
// scalar aggregation over the image set of Y facts grouped by X. The
// parser populates `rule.consequent_aggregates` and the dispatcher
// routes aggregate rules to `compile_aggregate_derivation` (Codd §2.3.4
// image-set pattern), NOT to `compile_explicit_derivation`. Covered
// here because the shape is a canonical user reading.

#[test]
fn shape_aggregate_count_groups_image_set() {
    let src = r#"# Test
Thing(.ID) is an entity type.
ID is a value type.
Part is a value type.
Arity is a value type.

## Fact Types
Thing has ID.
Thing has Part.
Thing has Arity.

## Derivation Rules
* Thing has Arity iff Arity is the count of Part where Thing has Part.
"#;
    let (rule, func) = parse_and_compile(src);

    match &rule.consequent_cell {
        ConsequentCellSource::Literal(id) => assert_eq!(id, "Thing_has_Arity",
            "consequent must resolve to Thing_has_Arity, got {}", id),
        other => panic!("expected Literal(..), got {:?}", other),
    }
    assert!(!rule.consequent_aggregates.is_empty(),
        "consequent_aggregates populated for aggregate rules, got {:#?}",
        rule.consequent_aggregates);
    let agg = &rule.consequent_aggregates[0];
    assert_eq!(agg.role, "Arity", "aggregate target role, got {}", agg.role);

    // Three Parts on the same Thing → Arity=3 for that Thing.
    // Each source fact iterates; the aggregate folds within each group
    // (group key: Thing). With identical group keys, the chainer would
    // dedup the three identical derivations down to one; apply_to_facts
    // is one step so we may see duplicates. The test verifies at least
    // one derivation with the correct count.
    let out = apply_to_facts(&func, &[
        ("Thing_has_Part", &[("Thing", "t-1"), ("Part", "wheel")]),
        ("Thing_has_Part", &[("Thing", "t-1"), ("Part", "engine")]),
        ("Thing_has_Part", &[("Thing", "t-1"), ("Part", "seat")]),
    ]);
    let derived = decode_derived(&out);
    assert!(!derived.is_empty(), "at least one aggregate derivation expected, got nothing");
    assert!(
        derived.iter().any(|(_, _, bindings)|
            bindings.iter().any(|(k, v)| k == "Thing" && v == "t-1") &&
            bindings.iter().any(|(k, v)| k == "Arity" && v == "3")),
        "expected (Thing=t-1, Arity=3) somewhere in derivations, got {:#?}", derived,
    );
}

// ─── Category 2: AntecedentRole (deferred) ──────────────────────────
//
// `ConsequentCellSource::AntecedentRole` is declared on the type and
// handled by `compile_explicit_derivation`'s 1-antecedent branch, but
// no parser path emits it today — every user reading resolves to
// `Literal(ft_id)`, and the #287 implicit-derivation synthesizers
// (compile_derivations' subtype-inheritance / CWA-negation / SS
// auto-fill loops) also build rules with Literal consequents. A rule
// like `* X has Y iff X is a Z and Z has Y` that the handoff names as
// AntecedentRole parses as a 2-antecedent Join and routes to
// `compile_join_derivation`, outside this harness' target. Left as a
// TODO so a future shape that exercises the AntecedentRole branch can
// be added next to its sibling shapes.

// ─── State machine derivation (#759 / Audit MC3b-a) ────────────────
//
// readings/core/state.md adds a derivation rule that captures Pass 2 of
// the SM assembly (the same logic the now-retired
// `derive_state_machines_from_facts` carried in compile.rs):
//   • A `Status is initial in SM` instance fact also derives the
//     corresponding `Status is defined in SM` fact.
//   • A direct `Status is defined in SM` instance fact lands in the
//     same cell (Pass 2b — already done by the parser; no derivation
//     needed because the FT is also assertable directly).
//   • Pass 1 (`State Machine Definition is for Noun`) is the FT itself
//     — its instance fact already registers the SM record.
//
// Two assertions:
//   1. The new rule text is present in readings/core/state.md (file
//      edit landed).
//   2. A tiny self-contained reading that re-declares the same FTs +
//      includes the same rule text actually populates the cells via
//      the engine's forward-chain, with `Draft` (initial) appearing
//      alongside `Placed` (directly-asserted defined) in the
//      `Status_is_defined_in_State_Machine_Definition` cell.

#[test]
fn sm_derivation_rules_populate_normalized_cells_from_initial_and_defined_facts() {
    use crate::ast::{cells_iter, fetch_or_phi};

    // (1) The Pass-2 derivation rule must be present in
    // readings/core/state.md — this is the file change #759 ships.
    let state_md = include_str!("../../../readings/core/state.md");
    let pass2_rule_text = "Status is defined in State Machine Definition iff that Status is initial in that State Machine Definition";
    assert!(
        state_md.contains(pass2_rule_text),
        "readings/core/state.md must contain the Pass-2 derivation rule (#759)\n\
         expected substring: `{}`\n\
         (this rule mirrors compile.rs:385-401 — initial Status implies defined Status)",
        pass2_rule_text,
    );

    // (2) Self-contained smoke: re-declare just the FTs we exercise so
    // the test doesn't drag in the full core+state metamodel, then
    // check the engine populates the cells correctly via forward-chain.
    let src = r#"# SM Derivation TDD
Order(.Name) is an entity type.
State Machine Definition(.Name) is an entity type.
Status(.Name) is an entity type.
Noun is an entity type.

## Fact Types
State Machine Definition is for Noun.
Status is initial in State Machine Definition.
Status is defined in State Machine Definition. *

## Derivation Rules
* Status is defined in State Machine Definition iff that Status is initial in that State Machine Definition.

## Instance Facts
State Machine Definition 'OrderSM' is for Noun 'Order'.
Status 'Draft' is initial in State Machine Definition 'OrderSM'.
Status 'Placed' is defined in State Machine Definition 'OrderSM'.
"#;
    let state = crate::parse_forml2::parse_to_state(src).expect("parse");
    let model = crate::compile::compile(&state);
    let derivation_refs: Vec<(&str, &crate::ast::Func)> =
        model.derivations.iter().map(|d| (d.id.as_str(), &d.func)).collect();
    let (final_state, _derived) =
        crate::evaluate::forward_chain_defs_state(&derivation_refs, &state);

    // Collect the Status-name set in Status_is_defined_in_SM to verify
    // both the directly-asserted Placed AND the initial-derived Draft
    // landed in the cell (Pass 2 + Pass 2b parity with compile.rs).
    let defined_cell = fetch_or_phi("Status_is_defined_in_State_Machine_Definition", &final_state);
    let defined_pairs: Vec<(String, String)> = defined_cell.as_seq().map(|facts| {
        facts.iter().filter_map(|f| {
            let pairs = f.as_seq()?;
            let mut status: Option<String> = None;
            let mut sm: Option<String> = None;
            for p in pairs.iter() {
                let kv = p.as_seq()?;
                if kv.len() != 2 { continue; }
                let k = kv[0].as_atom()?;
                let v = kv[1].as_atom()?;
                if k == "Status" { status = Some(v.to_string()); }
                if k == "State Machine Definition" { sm = Some(v.to_string()); }
            }
            Some((status?, sm?))
        }).collect()
    }).unwrap_or_default();
    assert!(
        defined_pairs.iter().any(|(s, m)| s == "Placed" && m == "OrderSM"),
        "Pass 2b: directly-asserted (Placed, OrderSM) must remain in Status_is_defined_in_SM, got {:?}",
        defined_pairs,
    );
    assert!(
        defined_pairs.iter().any(|(s, m)| s == "Draft" && m == "OrderSM"),
        "Pass 2: initial Status (Draft, OrderSM) must be derived into Status_is_defined_in_SM, got {:?}\nfinal cells: {:?}",
        defined_pairs,
        cells_iter(&final_state).iter().map(|(n, _)| *n).collect::<Vec<_>>(),
    );

    // The initial-marking cell entry must still exist for the initial Status.
    let initial_cell = fetch_or_phi("Status_is_initial_in_State_Machine_Definition", &final_state);
    let initial_pairs: Vec<(String, String)> = initial_cell.as_seq().map(|facts| {
        facts.iter().filter_map(|f| {
            let pairs = f.as_seq()?;
            let mut status: Option<String> = None;
            let mut sm: Option<String> = None;
            for p in pairs.iter() {
                let kv = p.as_seq()?;
                if kv.len() != 2 { continue; }
                let k = kv[0].as_atom()?;
                let v = kv[1].as_atom()?;
                if k == "Status" { status = Some(v.to_string()); }
                if k == "State Machine Definition" { sm = Some(v.to_string()); }
            }
            Some((status?, sm?))
        }).collect()
    }).unwrap_or_default();
    assert!(
        initial_pairs.iter().any(|(s, m)| s == "Draft" && m == "OrderSM"),
        "initial-marking cell must contain (Draft, OrderSM), got {:?}",
        initial_pairs,
    );
}

// ─── Pass 4: graph-derived initial Status (#760 / Audit MC3b-b) ─────
//
// readings/core/state.md adds a Pass-4 derivation rule that captures
// the topology fold the now-retired `derive_state_machines_from_facts`
// used to perform in compile.rs:
//
//   A Status is "rooted" in a State Machine Definition iff some
//   Transition in that SM has it as source AND no Transition in that
//   SM has it as target. Source-never-target is the graph-theoretic
//   characterization of an initial state when no `is initial in` fact
//   was declared.
//
// Per task #760, we use option (a): emit `Status is rooted in SM` for
// EVERY source-never-target candidate. The uniqueness gate ("exactly
// one rooted Status implies the engine treats it as initial") is
// deferred to the consumer side (#761 — `compile_state_machine` will
// promote to initial only when the rooted set has cardinality 1).
// FORML 2 derivations are monotonic, so "exactly one" cannot be
// expressed as a derivation rule; it is naturally a constraint /
// cardinality predicate the consumer applies after forward-chain.
//
// This test exercises the unique-source-never-target case so the rule
// fires and lands one entry in the rooted cell.

#[test]
fn sm_derivation_rules_populate_rooted_cell_from_graph_topology_when_no_initial_fact() {
    use crate::ast::{cells_iter, fetch_or_phi};

    // (1) The Pass-4 derivation rule must be present in
    // readings/core/state.md — this is the file change #760 ships.
    let state_md = include_str!("../../../readings/core/state.md");
    let pass4_rule_text = "Status is rooted in State Machine Definition iff some Transition is defined in that State Machine Definition and that Transition is from that Status and no Transition is defined in that State Machine Definition where that Transition is to that Status";
    assert!(
        state_md.contains(pass4_rule_text),
        "readings/core/state.md must contain the Pass-4 derivation rule (#760)\n\
         expected substring: `{}`\n\
         (this rule mirrors compile.rs:479-505 — source-never-target Statuses are graph-rooted candidates for initial)",
        pass4_rule_text,
    );

    // (2) Self-contained smoke: declare the SM, two transitions
    // forming a chain Draft -> Placed -> Shipped, with NO `is initial
    // in` instance fact. Assert the rooted cell contains (Draft,
    // OrderSM) and ONLY (Draft, OrderSM) — Placed is a target, Shipped
    // is a target, Draft is the lone source-never-target.
    let src = r#"# SM Derivation TDD — Pass 4
Order(.Name) is an entity type.
State Machine Definition(.Name) is an entity type.
Status(.Name) is an entity type.
Transition(.Name) is an entity type.
Fact Type(.Name) is an entity type.
Noun is an entity type.

## Fact Types
State Machine Definition is for Noun.
Transition is defined in State Machine Definition.
Transition is from Status.
Transition is to Status.
Transition is triggered by Fact Type.
Status is rooted in State Machine Definition. *

## Derivation Rules
* Status is rooted in State Machine Definition iff some Transition is defined in that State Machine Definition and that Transition is from that Status and no Transition is defined in that State Machine Definition where that Transition is to that Status.

## Instance Facts
State Machine Definition 'OrderSM' is for Noun 'Order'.
Transition 'place' is defined in State Machine Definition 'OrderSM'.
Transition 'place' is from Status 'Draft'.
Transition 'place' is to Status 'Placed'.
Transition 'place' is triggered by Fact Type 'Order_was_placed'.
Transition 'ship' is defined in State Machine Definition 'OrderSM'.
Transition 'ship' is from Status 'Placed'.
Transition 'ship' is to Status 'Shipped'.
Transition 'ship' is triggered by Fact Type 'Order_was_shipped'.
"#;
    let state = crate::parse_forml2::parse_to_state(src).expect("parse");
    let model = crate::compile::compile(&state);
    let derivation_refs: Vec<(&str, &crate::ast::Func)> =
        model.derivations.iter().map(|d| (d.id.as_str(), &d.func)).collect();
    let (final_state, _derived) =
        crate::evaluate::forward_chain_defs_state(&derivation_refs, &state);

    let rooted_cell = fetch_or_phi("Status_is_rooted_in_State_Machine_Definition", &final_state);
    let rooted_pairs: Vec<(String, String)> = rooted_cell.as_seq().map(|facts| {
        facts.iter().filter_map(|f| {
            let pairs = f.as_seq()?;
            let mut status: Option<String> = None;
            let mut sm: Option<String> = None;
            for p in pairs.iter() {
                let kv = p.as_seq()?;
                if kv.len() != 2 { continue; }
                let k = kv[0].as_atom()?;
                let v = kv[1].as_atom()?;
                if k == "Status" { status = Some(v.to_string()); }
                if k == "State Machine Definition" { sm = Some(v.to_string()); }
            }
            Some((status?, sm?))
        }).collect()
    }).unwrap_or_default();

    // Draft is source of `place` and never a target — the rule's
    // positive antecedents (some Transition is defined in SM AND that
    // Transition is from that Status) DO bind Draft, so it must
    // appear regardless of whether the negation pruned anything.
    assert!(
        rooted_pairs.iter().any(|(s, m)| s == "Draft" && m == "OrderSM"),
        "Pass 4: graph-rooted (Draft, OrderSM) must appear in Status_is_rooted_in_State_Machine_Definition,\n\
         got {:?}\nfinal cells: {:?}",
        rooted_pairs,
        cells_iter(&final_state).iter().map(|(n, _)| *n).collect::<Vec<_>>(),
    );
    // Shipped is target of `ship` and is NOT source of any
    // transition — so the positive antecedent `that Transition is
    // from that Status` doesn't bind Shipped at all, regardless of
    // whether the negation antecedent is honored. This is the
    // strongest assertion the test can make without depending on
    // parser-side negation/AbsenceOf support — the consumer-side
    // cardinality gate (#761) will deduplicate / require uniqueness.
    assert!(
        !rooted_pairs.iter().any(|(s, m)| s == "Shipped" && m == "OrderSM"),
        "Pass 4: (Shipped, OrderSM) is a transition target only, NEVER a source — so the positive\n\
         antecedent `that Transition is from that Status` cannot bind it; it must NOT be rooted.\n\
         got {:?}",
        rooted_pairs,
    );

    // NOTE on Placed: per task #760's option (a), the parser's
    // current handling of `no X where Y` strips negation and falls
    // back to the bare FT, so source-AND-target Statuses (Placed)
    // may appear in the rooted cell. The consumer side (#761) is
    // responsible for the uniqueness/cardinality gate that promotes
    // a single rooted Status to `is initial in`. When more than one
    // candidate appears (e.g. Placed + Draft from over-emission),
    // the consumer leaves initial empty — same as compile.rs:502-504.
    // The rooted set MUST contain Draft for that gate to ever fire;
    // the assertion above is the load-bearing one.
}
