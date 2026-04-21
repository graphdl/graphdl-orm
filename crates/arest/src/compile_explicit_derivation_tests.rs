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
use crate::types::{ConsequentCellSource, DerivationRuleDef};

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
