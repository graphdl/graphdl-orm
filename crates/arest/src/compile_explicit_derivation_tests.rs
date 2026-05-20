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
    // Binding keys preserve noun spaces, matching the parser convention
    // (compile.rs::role_value_by_name looks up by the noun name verbatim
    // — cell names are FT-id-style with underscores, but inner bindings
    // are noun-name-style with spaces).
    let out = apply_to_facts(&func, &[
        ("Vehicle_has_Weight_Class", &[("Vehicle", "v-heavy"), ("Weight Class", "extra heavy")]),
        ("Vehicle_has_Weight_Class", &[("Vehicle", "v-light"), ("Weight Class", "light")]),
    ]);
    let derived = decode_derived(&out);
    assert_eq!(derived.len(), 1, "only the matching Vehicle should derive, got {:#?}", derived);
    let (_ft, _reading, bindings) = &derived[0];
    assert!(
        bindings.iter().any(|(k, v)| k == "Vehicle" && v == "v-heavy"),
        "expected Vehicle='v-heavy', got {:#?}", bindings,
    );
    assert!(
        bindings.iter().any(|(k, v)| k == "Transit Category" && v == "heavy"),
        "expected Transit Category='heavy', got {:#?}", bindings,
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

// ─── Category 5b: Bridge derivation (role-renaming) — task-922 ──────
//
// Shape: `* X has B iff Y has A and B is A and X is Y` — a single
// antecedent rule that RE-KEYS the antecedent fact's bindings into a
// consequent FT with DIFFERENT role names. The bridge use case is the
// SM-cell → Task_has_Task_Status carrier (apps/tasks Stage 2): the
// antecedent reads `Resource is currently in Status` (roles Resource,
// Status) and the consequent is `Task has Task Status` (roles Task,
// Task Status). The computed bindings `Task Status is Status` and
// `Task is Resource` are the renames.
//
// Pre-fix (task-922-sql-projection-rolemismatch): the bindings function
// was `Concat · [Id, computed_pairs]`, so the emitted fact carried 4
// bindings — `<<Resource, X>, <Status, X>, <Task Status, X>, <Task, X>>`
// — exceeding the consequent FT's declared arity (2 roles). SQL
// projection's position-based column mapping was reading values from
// the WRONG binding slots (`Resource` slot ⇒ `Task` column, `Status`
// slot ⇒ `Task_Status` column), producing incorrect projection rows
// and leaving downstream readiness derivations broken.
//
// Post-fix: when `consequent_role_literals` is empty but
// `consequent_computed_bindings` is non-empty AND the consequent FT
// resolves, build bindings in the consequent FT's declared role order
// — literal pin → computed binding (via `compile_arith_expr`) →
// name-based lookup fallback. Emits exactly `arity(cons)` bindings so
// SQL position-mapping lands every value in its right column.

#[test]
fn shape_bridge_derivation_emits_fact_with_consequent_ft_arity() {
    let src = r#"# Test
Resource(.Reference) is an entity type.
Reference is a value type.
Status is a value type.
Task(.id) is an entity type.
id is a value type.
Task Status is a value type.

## Fact Types
Resource has Reference.
Resource is currently in Status.
Task has Task Status.

## Derivation Rules
* Task has Task Status iff that Resource is currently in some Status and Task Status is Status and Task is Resource.
"#;
    let (rule, func) = parse_and_compile(src);

    // Shape: single antecedent (Resource_is_currently_in_Status);
    // two computed bindings for the role renames (Task Status is
    // Status, Task is Resource); no literal pins.
    match &rule.consequent_cell {
        ConsequentCellSource::Literal(id) => assert_eq!(id, "Task_has_Task_Status",
            "consequent must resolve to Task_has_Task_Status, got {}", id),
        other => panic!("expected Literal(..), got {:?}", other),
    }
    assert_eq!(rule.antecedent_sources.len(), 1,
        "single antecedent expected (the role-rename clauses are computed bindings, \
         not antecedents), got {:#?}", rule.antecedent_sources);
    assert!(rule.consequent_role_literals.is_empty(),
        "no literal pins on the bridge — both bindings are role-rename computed \
         bindings, got {:#?}", rule.consequent_role_literals);
    assert_eq!(rule.consequent_computed_bindings.len(), 2,
        "two computed-binding role renames expected, got {:#?}",
        rule.consequent_computed_bindings);

    // Evaluate against one antecedent fact and verify the emitted fact
    // has EXACTLY 2 bindings (the consequent FT's declared arity), not
    // 4. Pre-fix, this would emit 4 bindings — `<Resource, X>`,
    // `<Status, X>`, plus the two computed renames — which the SQL
    // shadow then mis-mapped positionally.
    let out = apply_to_facts(&func, &[
        ("Resource_is_currently_in_Status",
            &[("Resource", "task-1"), ("Status", "in_progress")]),
    ]);
    let derived = decode_derived(&out);
    assert_eq!(derived.len(), 1, "one derived fact expected, got {:#?}", derived);
    let (ft, _reading, bindings) = &derived[0];
    assert_eq!(ft, "Task_has_Task_Status",
        "fact must land in Task_has_Task_Status cell, got {}", ft);
    assert_eq!(bindings.len(), 2,
        "consequent FT declares 2 roles (Task, Task Status); the derived fact's \
         binding count MUST match — got {} bindings: {:#?}",
        bindings.len(), bindings);
    // Position 0 must be the Task role (matches consequent FT's declared
    // role order). Value comes from the antecedent's Resource role per
    // the `Task is Resource` computed binding.
    assert_eq!(bindings[0], ("Task".to_string(), "task-1".to_string()),
        "first binding must be <Task, task-1> in declared role order, got {:?}",
        bindings[0]);
    assert_eq!(bindings[1], ("Task Status".to_string(), "in_progress".to_string()),
        "second binding must be <Task Status, in_progress>, got {:?}",
        bindings[1]);
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
            &[("Paper Element", "pe-1"), ("Implementation Mode", "In Code Only")]),
        ("Paper_Element_has_Implementation_Mode",
            &[("Paper Element", "pe-2"), ("Implementation Mode", "In Readings")]),
        ("Paper_Element_has_Implementation_Mode",
            &[("Paper Element", "pe-3"), ("Implementation Mode", "Aspirational")]),
    ]);
    let derived = decode_derived(&out);

    let liftable_pes: Vec<String> = derived.iter()
        .flat_map(|(_, _, b)| b.iter())
        .filter(|(k, _)| k == "Paper Element")
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

// ─── Category 13: Forward-chain over populated state (#817 MCP path) ─
//
// The earlier #817 test at `shape_some_quantifier_with_multi_word_literal_filters_antecedent`
// exercises the compiled Func directly via apply_to_facts — that proves
// the parser+compile pipeline emits a correct Func. But the MCP/worker
// path doesn't apply the Func directly; it routes through
// `forward_chain_defs_state` over a state populated with instance
// facts. If forward_chain has a bug — wrong cell name resolution,
// fact normalization mismatch, derivation persistence skipped — the
// previous test wouldn't catch it.
//
// This test pre-populates Paper Element instance facts inside the
// reading itself (the same way apps/paper's instance file does), runs
// the engine compile + forward_chain pipeline, and asserts the
// derived Lift Priority cell contains Liftable for the In-Code-Only
// elements. If this fails, the gap is in forward_chain (engine), not
// in the compiled Func itself.

#[test]
fn paper_lift_priority_derivation_fires_through_forward_chain() {
    use crate::ast::{cells_iter, fetch_or_phi};

    let src = r#"# Paper Forward-Chain Test
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

## Instance Facts
Paper Element 'pe-code' has Implementation Mode 'In Code Only'.
Paper Element 'pe-readings' has Implementation Mode 'In Readings'.
Paper Element 'pe-aspirational' has Implementation Mode 'Aspirational'.
"#;

    let state = crate::parse_forml2::parse_to_state(src).expect("parse");
    let model = crate::compile::compile(&state);

    // Sanity: the rule is in the compiled derivation set.
    let lift_rule = model.derivations.iter()
        .find(|d| d.text.contains("Lift Priority") && d.text.contains("Liftable"));
    assert!(
        lift_rule.is_some(),
        "Lift Priority derivation must be in compiled model.derivations.\n\
         Got {} derivations: {:#?}",
        model.derivations.len(),
        model.derivations.iter().map(|d| d.text.as_str()).collect::<Vec<_>>(),
    );

    // Run forward_chain_defs_state over the populated state — this is
    // the path the MCP/worker takes during apply.
    let derivation_refs: Vec<(&str, &crate::ast::Func)> =
        model.derivations.iter().map(|d| (d.id.as_str(), &d.func)).collect();
    let (final_state, _derived) =
        crate::evaluate::forward_chain_defs_state(&derivation_refs, &state);

    // Read the derived cell.
    let lift_cell = fetch_or_phi("Paper_Element_has_Lift_Priority", &final_state);
    let lift_pairs: Vec<(String, String)> = lift_cell.as_seq().map(|facts| {
        facts.iter().filter_map(|f| {
            let pairs = f.as_seq()?;
            let mut pe: Option<String> = None;
            let mut lp: Option<String> = None;
            for p in pairs.iter() {
                let kv = p.as_seq()?;
                if kv.len() != 2 { continue; }
                let k = kv[0].as_atom()?;
                let v = kv[1].as_atom()?;
                if k == "Paper Element" || k == "Paper_Element" { pe = Some(v.to_string()); }
                if k == "Lift Priority" || k == "Lift_Priority" { lp = Some(v.to_string()); }
            }
            Some((pe?, lp?))
        }).collect()
    }).unwrap_or_default();

    // The In-Code-Only Paper Element must have Liftable in its Lift Priority cell.
    assert!(
        lift_pairs.iter().any(|(pe, lp)| pe == "pe-code" && lp == "Liftable"),
        "expected (pe-code, Liftable) in Paper_Element_has_Lift_Priority after forward-chain.\n\
         Got: {:?}\n\
         All cells: {:?}",
        lift_pairs,
        cells_iter(&final_state).iter().map(|(n, _)| *n).collect::<Vec<_>>(),
    );
    // Negative cases: pe-readings and pe-aspirational must NOT derive Liftable.
    assert!(
        !lift_pairs.iter().any(|(pe, _)| pe == "pe-readings"),
        "pe-readings (Mode=In Readings) must NOT derive Liftable, got {:?}", lift_pairs);
    assert!(
        !lift_pairs.iter().any(|(pe, _)| pe == "pe-aspirational"),
        "pe-aspirational (Mode=Aspirational) must NOT derive Liftable, got {:?}", lift_pairs);
}

/// task-924: SM-status → normalized-property bridge hop 2. The tasks
/// app re-keys `Resource is currently in Status` into `Task has Task
/// Status` via a 1-antecedent rule with identity-binding clauses
/// (`Task is Resource`, `Task Status is Status`). On the live DB this
/// fires ZERO (Task_has_Task_Status empty though the source cell has
/// 751 rows). Pin: a populated Resource_is_currently_in_Status must
/// re-key into Task_has_Task_Status.
#[test]
fn sm_status_bridge_rekeys_into_task_has_task_status() {
    use crate::ast::fetch_or_phi;
    let src = r#"# bridge repro
Task(.id) is an entity type.
Resource(.id) is an entity type.
Status is a value type.
Task Status is a value type.

## Fact Types
Resource is currently in Status.
Task has Task Status.

## Constraints
Each Resource is currently in at most one Status.

## Derivation Rules
* Task has Task Status iff that Resource is currently in some Status and Task Status is Status and Task is Resource.

## Instance Facts
Resource 't1' is currently in Status 'Active'.
"#;
    let state = crate::parse_forml2_stage2::parse_to_state_via_stage12(src).expect("parse");
    let defs = crate::compile::compile_to_defs_state(&state);
    let d = crate::ast::defs_to_state(&defs, &state);
    let stratum1: Vec<(String, crate::ast::Func)> = crate::ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation:rule_"))
        .map(|(n, contents)| (n.to_string(), crate::ast::metacompose(contents, &d)))
        .collect();
    let refs: Vec<(&str, &crate::ast::Func)> =
        stratum1.iter().map(|(n, f)| (n.as_str(), f)).collect();
    let (final_state, _) = crate::evaluate::forward_chain_defs_state(&refs, &state);
    let cell = fetch_or_phi("Task_has_Task_Status", &final_state);
    let pairs: Vec<(String, String)> = cell.as_seq().map(|facts| {
        facts.iter().filter_map(|f| {
            let ps = f.as_seq()?;
            let mut task = None; let mut st = None;
            for p in ps.iter() {
                let kv = p.as_seq()?; if kv.len() != 2 { continue; }
                let k = kv[0].as_atom()?; let v = kv[1].as_atom()?;
                if k == "Task" { task = Some(v.to_string()); }
                if k == "Task Status" || k == "Task_Status" { st = Some(v.to_string()); }
            }
            Some((task?, st?))
        }).collect()
    }).unwrap_or_default();
    assert!(pairs.iter().any(|(t, s)| t == "t1" && s == "Active"),
        "bridge hop 2 must re-key <Resource=t1, Status=Active> into \
         Task_has_Task_Status <Task=t1, Task Status=Active>; got: {:?}", pairs);
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

// ─── SM cell → Task_has_Task_Status bridge (task-860) ───────────────
//
// After task-742 renamed the SM cells to FORML2-verbalized form
// (`State_Machine_is_currently_in_Status` with roles `State Machine`
// and `Status`; `State_Machine_is_for_Resource` with roles `State
// Machine` and `Resource`), apps/tasks/readings/app.md migrated the
// canonical Task status onto the SM cell. Legacy readers — the
// readiness derivation and queries against `Task_has_Task_Status` —
// still expect that FT cell to be populated. task-860 adds derivation
// rules in app.md that bridge the two: they re-project the SM cell's
// (Resource, Status) tuples into Task_has_Task_Status.
//
// The bridge is two stages because FORML2 join derivations don't carry
// computed bindings through the consequent (the join compiler only
// projects nouns that appear as roles in the antecedent FTs — see
// `compile.rs::compile_join_derivation::binding_parts`). Stage 1 joins
// the two SM cells on State Machine into the metamodel-declared
// `Resource is currently in Status` cell (roles match exactly).
// Stage 2 is a 1-antecedent ModusPonens rule that uses computed
// bindings (`Task is Resource`, `Task Status is Status`) to re-key
// the projected fact into Task_has_Task_Status's role schema —
// `compile_explicit_derivation`'s 1-antecedent path DOES honor
// `consequent_computed_bindings`.
//
//   * Resource is currently in Status iff some State Machine is for
//     that Resource and that State Machine is currently in that Status.
//   * Task has Task Status iff that Resource is currently in some
//     Status and Task Status is Status and Task is Resource.
//
// `State Machine is for Resource` and `Resource is currently in Status`
// are both declared in readings/core/instances.md. `State Machine is
// currently in Status` matches the post-task-742 cell name and is
// declared in app.md so the catalog resolves the rule's antecedent
// (its FT id `State_Machine_is_currently_in_Status` lines up with
// `crates/arest/src/command.rs::StateMachineCellShape::boot()`'s
// `cell_name` constant).
//
// This self-contained test mirrors the app.md rule shape: it declares
// the FTs, populates the SM cells directly with one Task entity at
// status 'pending', and asserts the bridge rules land
// (Task=t-1, Task Status=pending) in `Task_has_Task_Status` after
// forward-chain.

#[test]
fn sm_derivation_bridge_projects_currently_in_status_into_task_has_task_status() {
    use crate::ast::{cells_iter, fetch_or_phi};

    // Self-contained reading: declare the Task entity (subtype of
    // Resource so role-based lookups treat Task ids as Resource
    // values), the SM-cell-shape FTs matching the post-task-742 cell
    // names, and both bridge derivation rules. The instance facts
    // populate the SM cells directly via the SM cell's natural FT
    // readings so the test doesn't depend on the SM-init derivation
    // firing — we want to test the bridge in isolation.
    let src = r#"# Bridge test (task-860)
Task(.id) is an entity type.
State Machine(.id) is an entity type.
Resource(.Reference) is an entity type.

Task is a subtype of Resource.

Task Status is a value type.
Status is a value type.

## Fact Types
Task has Task Status.
Resource is currently in Status.
State Machine is for Resource.
State Machine is currently in Status.

## Derivation Rules
* Resource is currently in Status iff some State Machine is for that Resource and that State Machine is currently in that Status.
* Task has Task Status iff that Resource is currently in some Status and Task Status is Status and Task is Resource.

## Instance Facts
State Machine 'sm-1' is for Resource 't-1'.
State Machine 'sm-1' is currently in Status 'pending'.
"#;
    let state = crate::parse_forml2::parse_to_state(src).expect("parse");
    let model = crate::compile::compile(&state);
    let derivation_refs: Vec<(&str, &crate::ast::Func)> =
        model.derivations.iter().map(|d| (d.id.as_str(), &d.func)).collect();
    let (final_state, _derived) =
        crate::evaluate::forward_chain_defs_state(&derivation_refs, &state);

    // Collect (Task, Task Status) pairs from the cell.
    let cell = fetch_or_phi("Task_has_Task_Status", &final_state);
    let pairs: Vec<(String, String)> = cell.as_seq().map(|facts| {
        facts.iter().filter_map(|f| {
            let pairs = f.as_seq()?;
            let mut task: Option<String> = None;
            let mut status: Option<String> = None;
            for p in pairs.iter() {
                let kv = p.as_seq()?;
                if kv.len() != 2 { continue; }
                let k = kv[0].as_atom()?;
                let v = kv[1].as_atom()?;
                if k == "Task" { task = Some(v.to_string()); }
                if k == "Task Status" { status = Some(v.to_string()); }
            }
            Some((task?, status?))
        }).collect()
    }).unwrap_or_default();

    assert!(
        pairs.iter().any(|(t, s)| t == "t-1" && s == "pending"),
        "task-860 bridge: Task_has_Task_Status must contain (Task=t-1, Task Status=pending)\n\
         derived from State_Machine_is_for_Resource('sm-1','t-1') × \
         State_Machine_is_currently_in_Status('sm-1','pending') via the \
         Resource_is_currently_in_Status intermediate.\n\
         Got pairs: {:?}\nfinal cells: {:?}",
        pairs,
        cells_iter(&final_state).iter().map(|(n, _)| *n).collect::<Vec<_>>(),
    );
}

// task-860 acceptance criterion 2: the bridge rule's projection
// into `Task_has_Task_Status` must continue to satisfy the existing
// readiness derivation (`Task has Task Readiness 'ready' iff Task
// has Task Status 'pending' and Task has no Task Readiness
// 'blocked'`) — readiness lands `ready` for pending+unblocked Tasks
// whose status is sourced from the SM cell.

// ─── Two-priority cascade comparator workaround (task-814) ──────────
//
// The task-814 audit documents that comparator words ('strongest',
// 'highest', 'max' over a value type) are NOT recognised by
// `resolve_derivation_rule`'s cascade. The author-side workaround
// (see `readings/core/derivation.md` "Workaround: 2-level priority
// cascade with `has no` negation") is to write ONE rule per priority
// level, each guarded by an `AbsenceOf` clause that suppresses lower
// values when a higher value was already derived.
//
// TWO-LEVEL CASCADE LIMIT: the workaround composes cleanly for TWO
// priority levels because the forward-chain stratifier
// (`forward_chain_stratified` in `crates/arest/src/evaluate.rs`) runs
// only TWO strata — positive then negation-guarded. With a 3+-level
// cascade, the second + third level both land in stratum 2 and fire
// in the SAME inner round, so the third level can't observe the
// second level's emit and the third's `AbsenceOf` guard fails to
// suppress it. The substrate-user must either accept multi-emit on
// 3+-level cascades or pre-compute the priority levels into distinct
// cells the chainer can stratify by reading dependency. (Audit's
// "Engine gap: multi-level priority cascade is non-stratifiable"
// section in `readings/core/derivation.md` cites this.)
//
// This test pins the TWO-level cascade that DOES work: a Task whose
// `Readiness` should be 'ready' when its precomputed `Candidate
// Readiness` cell carries 'ready', otherwise 'blocked'. Each stage-2
// rule is a 1-positive-antecedent + AbsenceOf shape, handled correctly
// by `compile_explicit_derivation`'s multi-AbsenceOf branch (#918).
//
// Three Tasks exercise the 2-level cascade:
//   task-1 has candidate readiness {'ready', 'blocked'}
//     → expected Task Readiness = 'ready' (higher priority candidate)
//   task-2 has candidate readiness {'blocked'}
//     → expected Task Readiness = 'blocked' (only candidate)
//   task-3 has candidate readiness {'ready'}
//     → expected Task Readiness = 'ready' (only candidate)
//
// The forward chain is stratified — positive rules fire first to
// fixpoint, then `has no` negation-guarded rules suppress weaker
// values once a stronger one was emitted. `forward_chain_stratified`
// alternates strata until both pass with zero novel facts so the
// cascade reaches a unique least fixed point.
//
// If this test fails after a parser or compile refactor, the
// priority-cascade workaround in `readings/core/derivation.md` no
// longer holds and that doc needs to be revised alongside the
// engine change.

// ─── SM init emits forResource for entity without transition events ─
//
// task-922-sm-init-projection — surfaced by task-922 verification.
// `ft_State_Machine_is_for_Resource` showed 0 rows for Tasks that have
// no recorded SM events (e.g. fresh tasks without any
// `Task_is_started` / `Task_is_finished` etc.). The downstream
// `Task_has_Task_Status` bridge then can't materialize because its
// antecedent `Resource is currently in Status` joins on
// `State_Machine_is_for_Resource` × `State_Machine_is_currently_in_Status`,
// and the for_Resource side is empty for the fresh-entity case.
//
// Acceptance: for every entity of an SM-bound noun that exists in the
// population (via any FT cell binding) AND has no event facts, SM init
// must emit BOTH `State_Machine_is_currently_in_Status` (the existing
// behavior — gives the initial status) AND `State_Machine_is_for_Resource`
// (the regressed behavior — gives the entity-to-SM binding the bridge
// rule needs).
//
// This test:
//   1. Declares a Task SM with one transition (start: pending →
//      in_progress, triggered by `Task is started`).
//   2. Pushes two Tasks into the population via primary-field facts —
//      `Task t-no-events has Owner 'Alice'` etc. — but NO event facts.
//   3. Asserts after forward chain that BOTH SM cells contain a row
//      per Task.

#[test]
fn sm_init_emits_for_resource_for_entity_without_any_event_facts() {
    use crate::ast::{cells_iter, fetch_or_phi};

    // Metamodel — same surface as `readings/core/state.md` plus the
    // `Fact Type` noun so `Transition 'start' is triggered by Fact
    // Type '...'` parses as an Instance Fact rather than a Fact Type
    // declaration. (Mirrors the tasks-app load path: bootstrap +
    // state.md provide the metamodel scaffolding.)
    let meta = r#"# State metamodel
## Entity Types
Status(.Name) is an entity type.
State Machine Definition is a subtype of Status.
Transition(.id) is an entity type.
Fact Type(.id) is an entity type.
Noun is an entity type.
Name is a value type.

## Fact Types
State Machine Definition is for Noun.
Status is initial in State Machine Definition.
Transition is defined in State Machine Definition.
Transition is from Status.
Transition is to Status.
Transition is triggered by Fact Type.
"#;
    let domain = r#"# SM init forResource test (task-922-sm-init-projection)
## Entity Types
Task(.id) is an entity type.

## Value Types
Owner is a value type.

## Fact Types
Task has Owner.
Task is started.

## Instance Facts
State Machine Definition 'Task SM' is for Noun 'Task'.
Status 'pending' is initial in State Machine Definition 'Task SM'.

Transition 'start' is defined in State Machine Definition 'Task SM'.
Transition 'start' is from Status 'pending'.
Transition 'start' is to Status 'in_progress'.
Transition 'start' is triggered by Fact Type 'Task is started'.

Task 't-no-events' has Owner 'Alice'.
Task 't-also-no-events' has Owner 'Bob'.
"#;
    let meta_state = crate::parse_forml2::parse_to_state(meta).expect("parse metamodel");
    let domain_state = crate::parse_forml2::parse_to_state_with_nouns(domain, &meta_state)
        .expect("parse domain");
    let state = crate::ast::merge_states(&meta_state, &domain_state);
    let defs = crate::compile::compile_to_defs_state(&state);
    let d = crate::ast::defs_to_state(&defs, &state);

    // Collect SM init + event-fold derivations the way cli/entry.rs does
    // for the load path.
    let collect = |prefix: &str| -> Vec<(String, crate::ast::Func)> {
        let cell_prefix = format!("{}:", prefix);
        crate::ast::cells_iter(&d).into_iter()
            .filter(|(n, _)| n.starts_with(cell_prefix.as_str()))
            .map(|(n, contents)| (n.to_string(), crate::ast::metacompose(contents, &d)))
            .collect()
    };
    let mut stratum1 = collect("derivation:rule_");
    let init_and_fold: Vec<_> = crate::ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation:_sm_init_")
            || n.starts_with("derivation:_sm_event_fold_")
            || n.starts_with("derivation:_sm_for_resource_backfill_"))
        .map(|(n, contents)| (n.to_string(), crate::ast::metacompose(contents, &d)))
        .collect();
    stratum1.extend(init_and_fold);
    let s1_refs: Vec<(&str, &crate::ast::Func)> = stratum1.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let (final_state, _derived) =
        crate::evaluate::forward_chain_defs_state(&s1_refs, &state);

    // Collect Resource values from State_Machine_is_for_Resource cell.
    let for_res_cell = fetch_or_phi("State_Machine_is_for_Resource", &final_state);
    let resources: Vec<String> = for_res_cell.as_seq().map(|facts| {
        facts.iter().filter_map(|f| {
            let pairs = f.as_seq()?;
            for p in pairs.iter() {
                let kv = p.as_seq()?;
                if kv.len() != 2 { continue; }
                let k = kv[0].as_atom()?;
                let v = kv[1].as_atom()?;
                if k == "Resource" { return Some(v.to_string()); }
            }
            None
        }).collect()
    }).unwrap_or_default();

    let cells: Vec<&str> = cells_iter(&final_state).iter()
        .map(|(n, _)| *n).collect();

    assert!(
        resources.contains(&"t-no-events".to_string()),
        "SM init must emit State_Machine_is_for_Resource <_, t-no-events> for the \
         Task entity that has no event facts. Got Resources: {:?}\n\
         For_Resource cell contents: {:?}\n\
         All cells: {:?}",
        resources, for_res_cell, cells,
    );
    assert!(
        resources.contains(&"t-also-no-events".to_string()),
        "SM init must emit State_Machine_is_for_Resource <_, t-also-no-events> for \
         the second Task entity that has no event facts. Got Resources: {:?}\n\
         For_Resource cell contents: {:?}\n\
         All cells: {:?}",
        resources, for_res_cell, cells,
    );

    // ── Also assert the currentlyInStatus emit still fires for the
    // same entities (this is the "we know this works" baseline per the
    // task description). If currentlyInStatus is empty too, the bug is
    // SM init not running at all, not the targeted for_Resource regression.
    let status_cell = fetch_or_phi("State_Machine_is_currently_in_Status", &final_state);
    let statuses_by_resource: hashbrown::HashMap<String, String> = {
        let for_res_pairs: Vec<(String, String)> = for_res_cell.as_seq().map(|facts| {
            facts.iter().filter_map(|f| {
                let pairs = f.as_seq()?;
                let mut sm: Option<String> = None;
                let mut res: Option<String> = None;
                for p in pairs.iter() {
                    let kv = p.as_seq()?;
                    if kv.len() != 2 { continue; }
                    let k = kv[0].as_atom()?;
                    let v = kv[1].as_atom()?;
                    if k == "State Machine" { sm = Some(v.to_string()); }
                    if k == "Resource"      { res = Some(v.to_string()); }
                }
                Some((sm?, res?))
            }).collect()
        }).unwrap_or_default();
        let sm_to_status: hashbrown::HashMap<String, String> = status_cell.as_seq().map(|facts| {
            facts.iter().filter_map(|f| {
                let pairs = f.as_seq()?;
                let mut sm: Option<String> = None;
                let mut st: Option<String> = None;
                for p in pairs.iter() {
                    let kv = p.as_seq()?;
                    if kv.len() != 2 { continue; }
                    let k = kv[0].as_atom()?;
                    let v = kv[1].as_atom()?;
                    if k == "State Machine" { sm = Some(v.to_string()); }
                    if k == "Status"        { st = Some(v.to_string()); }
                }
                Some((sm?, st?))
            }).collect()
        }).unwrap_or_default();
        for_res_pairs.into_iter()
            .filter_map(|(sm, res)| sm_to_status.get(&sm).map(|st| (res, st.clone())))
            .collect()
    };
    assert_eq!(
        statuses_by_resource.get("t-no-events").map(|s| s.as_str()),
        Some("pending"),
        "SM init must also emit currentlyInStatus='pending' for t-no-events \
         (paired by State Machine binding with for_Resource). Got status map: {:?}",
        statuses_by_resource,
    );
}

// ─── SM init emits forResource when entity's only FT cell is keyed ─
//
// task-922-sm-init-projection — the actual production failure mode.
// The tasks-app reading declares `Task has Task Description` with an
// "at most one" alethic UC. The compiled `_CellKeyRoles` map registers
// `Task` as the key role, so `cell_put_keyed` writes the cell as a
// Map<task_id, fact>. The Seq-walking shape of `instances_of_noun_func`
// (`apply_to_all . Selector(2)` over the Seq of `<ft_id, facts>` pairs,
// then walking each fact's bindings) reads `encode_state(pop)` output —
// which already flattens Map<key, fact> to a Seq of values via
// `m.values()`. The instances_of_noun walk SHOULD find the same Tasks
// either way.
//
// If this test fails post-fix, the SM init derivation isn't picking
// up Tasks whose only FT cell is keyed — exactly the production
// symptom on tasks 919, 922 (and the 78 others surfaced by the task
// description). Conversely, if it passes alongside the non-keyed test,
// the SM-init derivation is fine and the production gap is somewhere
// else (likely the SQL-projection role-mismatch — separate task).

#[test]
fn sm_init_emits_for_resource_when_entitys_only_ft_cell_is_keyed_by_alethic_uc() {
    use crate::ast::{cells_iter, fetch_or_phi};

    let meta = r#"# State metamodel
## Entity Types
Status(.Name) is an entity type.
State Machine Definition is a subtype of Status.
Transition(.id) is an entity type.
Fact Type(.id) is an entity type.
Noun is an entity type.
Name is a value type.

## Fact Types
State Machine Definition is for Noun.
Status is initial in State Machine Definition.
Transition is defined in State Machine Definition.
Transition is from Status.
Transition is to Status.
Transition is triggered by Fact Type.
"#;
    // `Task has Task Description` carries `Each Task has at most one
    // Task Description.` — the parser registers a UC keyed on Task,
    // and `cell_put_keyed` will write the cell as a Map<task_id, fact>.
    // This is exactly the shape of `Task_has_Task_Description` and
    // `Task_has_Task_Priority` in the production tasks-app DB.
    let domain = r#"# SM init forResource over keyed cell (task-922-sm-init-projection)
## Entity Types
Task(.id) is an entity type.

## Value Types
Task Description is a value type.

## Fact Types
Task has Task Description.
  Each Task has at most one Task Description.
Task is started.

## Instance Facts
State Machine Definition 'Task SM' is for Noun 'Task'.
Status 'pending' is initial in State Machine Definition 'Task SM'.

Transition 'start' is defined in State Machine Definition 'Task SM'.
Transition 'start' is from Status 'pending'.
Transition 'start' is to Status 'in_progress'.
Transition 'start' is triggered by Fact Type 'Task is started'.

Task 't-keyed-1' has Task Description 'first task no events'.
Task 't-keyed-2' has Task Description 'second task no events'.
"#;
    let meta_state = crate::parse_forml2::parse_to_state(meta).expect("parse metamodel");
    let domain_state = crate::parse_forml2::parse_to_state_with_nouns(domain, &meta_state)
        .expect("parse domain");
    let state = crate::ast::merge_states(&meta_state, &domain_state);
    let defs = crate::compile::compile_to_defs_state(&state);
    let d = crate::ast::defs_to_state(&defs, &state);

    let collect = |prefix: &str| -> Vec<(String, crate::ast::Func)> {
        let cell_prefix = format!("{}:", prefix);
        crate::ast::cells_iter(&d).into_iter()
            .filter(|(n, _)| n.starts_with(cell_prefix.as_str()))
            .map(|(n, contents)| (n.to_string(), crate::ast::metacompose(contents, &d)))
            .collect()
    };
    let mut stratum1 = collect("derivation:rule_");
    let init_and_fold: Vec<_> = crate::ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation:_sm_init_")
            || n.starts_with("derivation:_sm_event_fold_")
            || n.starts_with("derivation:_sm_for_resource_backfill_"))
        .map(|(n, contents)| (n.to_string(), crate::ast::metacompose(contents, &d)))
        .collect();
    stratum1.extend(init_and_fold);
    let s1_refs: Vec<(&str, &crate::ast::Func)> = stratum1.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let (final_state, _derived) =
        crate::evaluate::forward_chain_defs_state(&s1_refs, &state);

    // Force the Map-storage shape: the parser writes instance-fact
    // payloads as Seq cells, but the production load path (the
    // alethic-UC-aware apply pipeline at `command.rs::create_via_defs`
    // via `push_with_uc_check` → `cell_put_keyed`) rewrites them into
    // Map<task_id, fact> shape. We simulate that here by walking the
    // Seq cell and re-keying it via `cell_put_keyed` BEFORE the SM
    // init derivation runs, so we exercise the same `encode_state` →
    // `instances_of_noun_func` path the production fixed-point runs
    // against.
    let mut keyed_state = state.clone();
    {
        let cell = fetch_or_phi("Task_has_Task_Description", &keyed_state);
        if let Some(facts) = cell.as_seq() {
            let facts_vec: Vec<crate::ast::Object> = facts.iter().cloned().collect();
            // Drop the Seq version first by writing a fresh Map.
            keyed_state = crate::ast::store(
                "Task_has_Task_Description",
                crate::ast::Object::Map(Default::default()),
                &keyed_state,
            );
            for fact in facts_vec {
                keyed_state = crate::ast::cell_put_keyed(
                    "Task_has_Task_Description",
                    &["Task"],
                    fact,
                    &keyed_state,
                ).expect("cell_put_keyed first write");
            }
        }
    }
    // Sanity-check the rewrite landed in Map storage — guards the test's
    // structural premise that we are exercising the Map-cell path.
    let desc_cell = fetch_or_phi("Task_has_Task_Description", &keyed_state);
    assert!(
        matches!(desc_cell, crate::ast::Object::Map(_)),
        "test setup: Task_has_Task_Description should now be Map-keyed \
         after the explicit `cell_put_keyed` rewrite. Got: {:?}",
        desc_cell,
    );

    // Re-run forward chain over the keyed state.
    let (final_state, _derived) =
        crate::evaluate::forward_chain_defs_state(&s1_refs, &keyed_state);

    let for_res_cell = fetch_or_phi("State_Machine_is_for_Resource", &final_state);
    let resources: Vec<String> = for_res_cell.as_seq().map(|facts| {
        facts.iter().filter_map(|f| {
            let pairs = f.as_seq()?;
            for p in pairs.iter() {
                let kv = p.as_seq()?;
                if kv.len() != 2 { continue; }
                let k = kv[0].as_atom()?;
                let v = kv[1].as_atom()?;
                if k == "Resource" { return Some(v.to_string()); }
            }
            None
        }).collect()
    }).unwrap_or_default();

    let cells: Vec<&str> = cells_iter(&final_state).iter()
        .map(|(n, _)| *n).collect();

    assert!(
        resources.contains(&"t-keyed-1".to_string()),
        "SM init must emit State_Machine_is_for_Resource <_, t-keyed-1> when \
         the Task's only FT cell is Map-keyed by alethic UC. Got Resources: \
         {:?}\nFor_Resource cell: {:?}\nAll cells: {:?}",
        resources, for_res_cell, cells,
    );
    assert!(
        resources.contains(&"t-keyed-2".to_string()),
        "SM init must emit State_Machine_is_for_Resource <_, t-keyed-2> when \
         the Task's only FT cell is Map-keyed by alethic UC. Got Resources: \
         {:?}\nFor_Resource cell: {:?}\nAll cells: {:?}",
        resources, for_res_cell, cells,
    );
}

// ─── for_Resource backfill picks up entities with only a status row ─
//
// task-922-sm-init-projection production failure mode: an entity
// exists in `State_Machine_is_currently_in_Status` (because
// `command.rs::transition_via_defs` or `create_via_defs` pushed a
// status row directly) without a matching `State_Machine_is_for_Resource`
// row. The legacy SM init scan via `instances_of_noun_func` reads
// the noun's primary FT cells — it MISSES entities whose only
// population trace is in the SM cells themselves (no Task_has_*,
// no Task_is_*, etc.). This shape was task 922 in production:
// SM cell carried `State_Machine=922, Status=completed` but every
// `ft_Task_*` table reported zero rows for task 922.
//
// After the backfill derivation lands, an SM entity with a status
// row but no for_Resource row must materialize a for_Resource
// `<SM=e, Resource=e>` row in the same forward chain. Without that
// the bridge derivation (`Resource is currently in Status iff some
// State Machine is for that Resource and that State Machine is
// currently in that Status`) can't join, and the downstream
// Task_has_Task_Status projection drops the entity.

#[test]
fn sm_for_resource_backfill_emits_for_entity_with_only_currently_in_status_row() {
    use crate::ast::{cells_iter, fetch_or_phi};

    let meta = r#"# State metamodel
## Entity Types
Status(.Name) is an entity type.
State Machine Definition is a subtype of Status.
Transition(.id) is an entity type.
Fact Type(.id) is an entity type.
Noun is an entity type.
Name is a value type.

## Fact Types
State Machine Definition is for Noun.
Status is initial in State Machine Definition.
Transition is defined in State Machine Definition.
Transition is from Status.
Transition is to Status.
Transition is triggered by Fact Type.
"#;
    let domain = r#"# SM init for_Resource backfill (task-922-sm-init-projection)
## Entity Types
Task(.id) is an entity type.
State Machine(.id) is an entity type.
Resource(.Reference) is an entity type.

Task is a subtype of Resource.

## Value Types
Status is a value type.

## Fact Types
Task is started.
State Machine is currently in Status.
State Machine is for Resource.

## Instance Facts
State Machine Definition 'Task SM' is for Noun 'Task'.
Status 'pending' is initial in State Machine Definition 'Task SM'.

Transition 'start' is defined in State Machine Definition 'Task SM'.
Transition 'start' is from Status 'pending'.
Transition 'start' is to Status 'in_progress'.
Transition 'start' is triggered by Fact Type 'Task is started'.

State Machine 't-orphan' is currently in Status 'completed'.
"#;
    let meta_state = crate::parse_forml2::parse_to_state(meta).expect("parse metamodel");
    let domain_state = crate::parse_forml2::parse_to_state_with_nouns(domain, &meta_state)
        .expect("parse domain");
    let state = crate::ast::merge_states(&meta_state, &domain_state);
    let defs = crate::compile::compile_to_defs_state(&state);
    let d = crate::ast::defs_to_state(&defs, &state);

    let init_and_fold_and_backfill: Vec<_> = crate::ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation:_sm_init_")
            || n.starts_with("derivation:_sm_event_fold_")
            || n.starts_with("derivation:_sm_for_resource_backfill_"))
        .map(|(n, contents)| (n.to_string(), crate::ast::metacompose(contents, &d)))
        .collect();
    assert!(
        init_and_fold_and_backfill.iter().any(|(n, _)| n.contains("for_resource_backfill")),
        "compile_to_defs_state must emit a `_sm_for_resource_backfill_<Noun>` \
         derivation alongside _sm_init_ and _sm_event_fold_ for every SM. \
         Got defs: {:?}",
        init_and_fold_and_backfill.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
    );
    let s1_refs: Vec<(&str, &crate::ast::Func)> = init_and_fold_and_backfill.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let (final_state, _derived) =
        crate::evaluate::forward_chain_defs_state(&s1_refs, &state);

    // Pre-condition sanity check: the orphan currently_in_Status row
    // is still in the population. If it's not, the test setup
    // regressed and we'd see an empty backfill for the wrong reason.
    let status_cell = fetch_or_phi("State_Machine_is_currently_in_Status", &final_state);
    let has_orphan_status = status_cell.as_seq().map(|facts| {
        facts.iter().any(|f| {
            let pairs = f.as_seq().unwrap_or(&[]);
            let mut sm: Option<&str> = None;
            let mut st: Option<&str> = None;
            for p in pairs.iter() {
                let kv = match p.as_seq() { Some(kv) => kv, None => continue };
                if kv.len() != 2 { continue; }
                let k = match kv[0].as_atom() { Some(k) => k, None => continue };
                let v = match kv[1].as_atom() { Some(v) => v, None => continue };
                if k == "State Machine" { sm = Some(v); }
                if k == "Status"        { st = Some(v); }
            }
            sm == Some("t-orphan") && st == Some("completed")
        })
    }).unwrap_or(false);
    assert!(
        has_orphan_status,
        "Pre-condition: the orphan State_Machine_is_currently_in_Status row \
         must still be in the population after forward chain. Got: {:?}",
        status_cell,
    );

    // The actual assertion: after backfill, the orphan SM has a
    // for_Resource row keyed by the same id (State Machine id == Resource id).
    let for_res_cell = fetch_or_phi("State_Machine_is_for_Resource", &final_state);
    let backfilled = for_res_cell.as_seq().map(|facts| {
        facts.iter().any(|f| {
            let pairs = f.as_seq().unwrap_or(&[]);
            let mut sm: Option<&str> = None;
            let mut res: Option<&str> = None;
            for p in pairs.iter() {
                let kv = match p.as_seq() { Some(kv) => kv, None => continue };
                if kv.len() != 2 { continue; }
                let k = match kv[0].as_atom() { Some(k) => k, None => continue };
                let v = match kv[1].as_atom() { Some(v) => v, None => continue };
                if k == "State Machine" { sm = Some(v); }
                if k == "Resource"      { res = Some(v); }
            }
            sm == Some("t-orphan") && res == Some("t-orphan")
        })
    }).unwrap_or(false);
    assert!(
        backfilled,
        "task-922-sm-init-projection: the for_Resource backfill derivation \
         must emit <SM=t-orphan, Resource=t-orphan> when the SM is in \
         currently_in_Status but missing from for_Resource. Got for_Resource: \
         {:?}\nAll cells: {:?}",
        for_res_cell,
        cells_iter(&final_state).iter().map(|(n, _)| *n).collect::<Vec<_>>(),
    );

    // Idempotency: a second forward-chain pass must not introduce a
    // duplicate row for the same Resource. for_Resource has a
    // "Each Resource has at most one State Machine" UC; the
    // existing-set subtraction in `compile_sm_for_resource_backfill_for`
    // is what guards against re-emit. If this test fires duplicate
    // rows the UC fixpoint would surface a violation in the
    // production load path.
    let (final_state_2, _) =
        crate::evaluate::forward_chain_defs_state(&s1_refs, &final_state);
    let for_res_2 = fetch_or_phi("State_Machine_is_for_Resource", &final_state_2);
    let row_count = |cell: &crate::ast::Object| -> usize {
        cell.as_seq().map(|s| s.iter().filter(|f| {
            f.as_seq().map(|pairs| pairs.iter().any(|p| {
                p.as_seq().map(|kv| {
                    kv.len() == 2
                        && kv[0].as_atom() == Some("Resource")
                        && kv[1].as_atom() == Some("t-orphan")
                }).unwrap_or(false)
            })).unwrap_or(false)
        }).count()).unwrap_or(0)
    };
    assert_eq!(
        row_count(&for_res_cell), 1,
        "task-922-sm-init-projection: exactly one for_Resource row for \
         t-orphan after the first forward-chain pass — got {}: {:?}",
        row_count(&for_res_cell), for_res_cell,
    );
    assert_eq!(
        row_count(&for_res_2), 1,
        "task-922-sm-init-projection: forward chain must be idempotent — \
         a second pass must not duplicate the backfilled for_Resource row. \
         Got {} rows for t-orphan: {:?}",
        row_count(&for_res_2), for_res_2,
    );
}

// ─── Category 18: Existential-over-join fans out per X (#814) ───────
//
// Shape: `* X has Y 'lit' iff X concerns some Z that has Y 'lit'.`
//
// After parsing, `that` is consumed by `expand_that_relatives` into a
// 2-antecedent rule shape:
//
//   antecedent 0: X concerns Z
//   antecedent 1: Z has Y
//
// The join key is Z (the existential variable bound by `some`), NOT a
// role of the consequent. The consequent's roles are {X, Y}; Z appears
// in both antecedents but is invisible to the consequent.
//
// Before the fix, `compile_explicit_derivation`'s multi-antecedent
// path computed `join_roles` as `cons_roles ∩ ant0_roles ∩ ant1_roles`
// — which for this shape is `{X, Y} ∩ {X, Z} ∩ {Z, Y}` = ∅. So it fell
// through to the existence-check fallback at compile.rs:4026, which
// fires ONCE GLOBALLY on existence of facts in both antecedents and
// emits ONE consequent fact using bindings from antecedent 0's first
// fact — silently dropping every other qualifying X.
//
// The fix: when join detection finds no role-of-consequent join key,
// look for an EXISTENTIAL join key — a role shared between two or more
// antecedents that doesn't appear in the consequent (here, Z). When
// found, fanout per binding of antecedent 0 just like the consequent-
// role-joined branch does, with the existential equality gate carried
// across to the other antecedent(s).
//
// Acceptance: 3 X's each with their own qualifying Z → 3 derived
// consequents (one per X), not 1.
#[test]
fn shape_existential_over_join_fans_out_per_x() {
    let src = r#"# Existential-over-join test (#814)
Feature(.Key) is an entity type.
Product(.Key) is an entity type.
Key is a value type.
Status is a value type.

## Fact Types
Feature has Key.
Product has Key.
Feature has Status.
Product has Status.
Feature concerns Product.

## Derivation Rules
* Feature has Status 'critical' iff Feature concerns some Product that has Status 'critical'.
"#;
    let (rule, func) = parse_and_compile(src);

    // Parse shape: 2 antecedents (X concerns Z, Z has Y 'lit'). The
    // `that` is consumed by expand_that_relatives — so `join_on` is
    // empty and the rule routes through compile_explicit_derivation's
    // multi-antecedent path, not compile_join_derivation.
    assert_eq!(rule.antecedent_sources.len(), 2,
        "expected 2 antecedents (Feature concerns Product; Product has Status), got {:#?}",
        rule.antecedent_sources);

    // Consequent: Feature has Status 'critical', pinned via
    // consequent_role_literals.
    match &rule.consequent_cell {
        ConsequentCellSource::Literal(_) => {}
        other => panic!("expected Literal consequent, got {:?}", other),
    }
    assert!(
        rule.consequent_role_literals.iter()
            .any(|l| l.role == "Status" && l.value == "critical"),
        "expected consequent literal Status='critical', got {:#?}",
        rule.consequent_role_literals);

    // Population: 3 Features, each concerning a different Product
    // that has Status='critical'. Plus one Feature concerning a
    // non-critical Product (must NOT derive).
    let out = apply_to_facts(&func, &[
        ("Feature_concerns_Product", &[("Feature", "f-a"), ("Product", "p-a")]),
        ("Feature_concerns_Product", &[("Feature", "f-b"), ("Product", "p-b")]),
        ("Feature_concerns_Product", &[("Feature", "f-c"), ("Product", "p-c")]),
        ("Feature_concerns_Product", &[("Feature", "f-d"), ("Product", "p-d")]),
        ("Product_has_Status", &[("Product", "p-a"), ("Status", "critical")]),
        ("Product_has_Status", &[("Product", "p-b"), ("Status", "critical")]),
        ("Product_has_Status", &[("Product", "p-c"), ("Status", "critical")]),
        ("Product_has_Status", &[("Product", "p-d"), ("Status", "minor")]),
    ]);
    let derived = decode_derived(&out);

    // Collect the Feature bindings of every derived fact.
    let critical_features: Vec<String> = derived.iter()
        .flat_map(|(_, _, b)| b.iter())
        .filter(|(k, _)| k == "Feature")
        .map(|(_, v)| v.clone())
        .collect();

    // Per-X fanout: f-a, f-b, f-c must EACH derive a consequent.
    // Pre-fix behavior emits ONE fact globally using antecedent 0's
    // first fact's bindings (so this would be 1, not 3).
    assert!(critical_features.iter().any(|f| f == "f-a"),
        "f-a must derive (concerns p-a/critical); got Feature-bindings {:?}\n\
         full derived: {:#?}", critical_features, derived);
    assert!(critical_features.iter().any(|f| f == "f-b"),
        "f-b must derive (concerns p-b/critical); got Feature-bindings {:?}\n\
         full derived: {:#?}", critical_features, derived);
    assert!(critical_features.iter().any(|f| f == "f-c"),
        "f-c must derive (concerns p-c/critical); got Feature-bindings {:?}\n\
         full derived: {:#?}", critical_features, derived);

    // f-d (concerns p-d/minor) must NOT derive — its Product's Status
    // doesn't match the existential's literal filter.
    assert!(!critical_features.iter().any(|f| f == "f-d"),
        "f-d must NOT derive (concerns p-d/minor, fails Status='critical' filter);\n\
         got Feature-bindings {:?}\nfull derived: {:#?}", critical_features, derived);

    // The strict per-X count: exactly 3 qualifying Features.
    assert_eq!(critical_features.len(), 3,
        "expected exactly 3 derived consequents (one per qualifying Feature),\n\
         got {} with Feature-bindings {:?}\nfull derived: {:#?}",
        critical_features.len(), critical_features, derived);
}

// ─── #927 — coexist with parallelizable's own derivation
//
// Production has BOTH `parallelizable iff ...` AND `recommended iff
// ... parallelizable ...` in the same readings. The implicit-equi-
// join branch test above only declares recommended. Adding the
// parallelizable rule alongside reproduces the resolve-context
// hypothesis: the parser sees `Task is parallelizable` as both a
// rule consequent AND as a positive antecedent of recommended, and
// may resolve them inconsistently.
#[test]
fn implicit_equi_join_materializes_when_parallelizable_is_itself_derived() {
    let src = r#"# task-927
Task(.id) is an entity type.
Task Readiness is a value type.
Task Priority is a value type.

## Fact Types
Task has Task Readiness.
Task has Task Priority.
Task is parallelizable.
Task is epic.
Task is recommended.

## Derivation Rules
* Task is parallelizable iff Task has Task Readiness 'ready' and Task is not epic.
* Task is recommended iff Task has Task Readiness 'ready' and Task has Task Priority 'p0' and Task is parallelizable and Task is not epic.
"#;
    let state = parse_to_state(src).expect("parse");
    let data = compile::cell_index_from_state(&state);
    assert_eq!(data.derivation_rules.len(), 2);
    let model = compile::compile(&state);

    // Apply rule funcs to a hand-built population using
    // forward_chain_defs_state so parallelizable's emit feeds
    // recommended (mirrors what the apply path does).
    let mut state = Object::phi();
    state = ast::cell_push("Task_has_Task_Readiness",
        ast::fact_from_pairs(&[("Task", "a"), ("Task Readiness", "ready")]), &state);
    state = ast::cell_push("Task_has_Task_Priority",
        ast::fact_from_pairs(&[("Task", "a"), ("Task Priority", "p0")]), &state);

    // Partition rules into stratum1 (no negation) and stratum2 (with).
    let s1: Vec<(&str, &Func)> = model.derivations.iter()
        .filter(|d| !d.uses_negation)
        .map(|d| (d.id.as_str(), &d.func)).collect();
    let s2: Vec<(&str, &Func)> = model.derivations.iter()
        .filter(|d| d.uses_negation)
        .map(|d| (d.id.as_str(), &d.func)).collect();

    let (post_s1, _) = if s1.is_empty() {
        (state.clone(), Vec::new())
    } else { crate::evaluate::forward_chain_defs_state(&s1, &state) };
    let (post, _) = if s2.is_empty() {
        (post_s1, Vec::new())
    } else { crate::evaluate::forward_chain_defs_state(&s2, &post_s1) };

    let rec_cell = ast::fetch_or_phi("Task_is_recommended", &post);
    let recs: Vec<String> = match &rec_cell {
        Object::Seq(items) => items.iter()
            .filter_map(|f| ast::binding(f, "Task").map(String::from)).collect(),
        Object::Map(m) => m.values()
            .filter_map(|f| ast::binding(f, "Task").map(String::from)).collect(),
        _ => Vec::new(),
    };
    assert!(recs.contains(&"a".to_string()),
        "Task 'a' (ready+p0+derived-parallelizable+not-epic) must be \
         recommended. recs={:?}, parallelizable cell: {:?}",
        recs, ast::fetch_or_phi("Task_is_parallelizable", &post));
}

// ─── #927 — Map-backed Readiness/Priority + derived parallelizable
//
// The Seq-backed test above passes. Adds UCs on Readiness/Priority
// to force Map storage, mirroring apps/tasks. parallelizable still
// derived. If THIS fails while the Seq variant passes, the issue
// is implicit-equi-join's handling of Map antecedents WHEN those
// antecedents are themselves derived by stratum2 rules.
#[test]
fn implicit_equi_join_map_backed_with_derived_parallelizable() {
    let src = r#"# task-927 map + derived parallelizable
Task(.id) is an entity type.
Task Readiness is a value type.
Task Priority is a value type.

## Fact Types
Task has Task Readiness.
  Each Task has at most one Task Readiness.
Task has Task Priority.
  Each Task has at most one Task Priority.
Task is parallelizable.
Task is epic.
Task is recommended.

## Derivation Rules
* Task is parallelizable iff Task has Task Readiness 'ready' and Task is not epic.
* Task is recommended iff Task has Task Readiness 'ready' and Task has Task Priority 'p0' and Task is parallelizable and Task is not epic.
"#;
    let state_initial = parse_to_state(src).expect("parse");
    let model = compile::compile(&state_initial);

    let mut state = state_initial.clone();
    let map_put = |state: Object, cell: &str, pairs: &[(&str, &str)]| -> Object {
        ast::cell_put_keyed(cell, &["Task"], ast::fact_from_pairs(pairs), &state).unwrap()
    };
    state = map_put(state, "Task_has_Task_Readiness", &[("Task", "a"), ("Task Readiness", "ready")]);
    state = map_put(state, "Task_has_Task_Priority",  &[("Task", "a"), ("Task Priority", "p0")]);

    let s1: Vec<(&str, &Func)> = model.derivations.iter()
        .filter(|d| !d.uses_negation)
        .map(|d| (d.id.as_str(), &d.func)).collect();
    let s2: Vec<(&str, &Func)> = model.derivations.iter()
        .filter(|d| d.uses_negation)
        .map(|d| (d.id.as_str(), &d.func)).collect();

    let (post_s1, _) = if s1.is_empty() {
        (state.clone(), Vec::new())
    } else { crate::evaluate::forward_chain_defs_state(&s1, &state) };
    let (post, _) = if s2.is_empty() {
        (post_s1, Vec::new())
    } else { crate::evaluate::forward_chain_defs_state(&s2, &post_s1) };

    let rec_cell = ast::fetch_or_phi("Task_is_recommended", &post);
    let recs: Vec<String> = match &rec_cell {
        Object::Seq(items) => items.iter()
            .filter_map(|f| ast::binding(f, "Task").map(String::from)).collect(),
        Object::Map(m) => m.values()
            .filter_map(|f| ast::binding(f, "Task").map(String::from)).collect(),
        _ => Vec::new(),
    };
    let para_cell = ast::fetch_or_phi("Task_is_parallelizable", &post);
    assert!(recs.contains(&"a".to_string()),
        "Task 'a' must be recommended. recs={:?}, parallelizable={:?}, \
         readiness={:?}",
        recs, para_cell,
        ast::fetch_or_phi("Task_has_Task_Readiness", &post));
}

// ─── #927 — round-trip via compile_to_defs_state + metacompose
//
// Production goes: compile_to_defs_state -> defs_to_state -> at
// runtime fetch via cells_iter + metacompose -> apply. My earlier
// tests used model.derivations directly. If func_to_object /
// metacompose loses fidelity for the recommended rule's compiled
// Func shape, the recovered Func may differ from the original.
#[test]
fn implicit_equi_join_round_trip_through_defs_state() {
    let src = r#"# task-927 defs round-trip
Task(.id) is an entity type.
Task Readiness is a value type.
Task Priority is a value type.

## Fact Types
Task has Task Readiness.
  Each Task has at most one Task Readiness.
Task has Task Priority.
  Each Task has at most one Task Priority.
Task is parallelizable.
Task is epic.
Task is recommended.

## Derivation Rules
* Task is parallelizable iff Task has Task Readiness 'ready' and Task is not epic.
* Task is recommended iff Task has Task Readiness 'ready' and Task has Task Priority 'p0' and Task is parallelizable and Task is not epic.
"#;
    let state_initial = parse_to_state(src).expect("parse");
    let defs = compile::compile_to_defs_state(&state_initial);
    let d = ast::defs_to_state(&defs, &state_initial);

    // Build population the way apply does.
    let mut pop_state = state_initial.clone();
    let map_put = |state: Object, cell: &str, pairs: &[(&str, &str)]| -> Object {
        ast::cell_put_keyed(cell, &["Task"], ast::fact_from_pairs(pairs), &state).unwrap()
    };
    pop_state = map_put(pop_state, "Task_has_Task_Readiness", &[("Task", "a"), ("Task Readiness", "ready")]);
    pop_state = map_put(pop_state, "Task_has_Task_Priority",  &[("Task", "a"), ("Task Priority", "p0")]);

    // Mirror command.rs's collect_stratum: walk d's derivation cells,
    // metacompose contents back to Func.
    let collect_stratum = |prefix: &str| -> Vec<(String, ast::Func)> {
        let cell_prefix = alloc::format!("{}:", prefix);
        ast::cells_iter(&d).into_iter()
            .filter(|(n, _)| n.starts_with(cell_prefix.as_str()))
            .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, &d)))
            .collect()
    };
    let stratum1 = collect_stratum("derivation");
    let stratum2 = collect_stratum("derivation_strat2");
    let s1: Vec<(&str, &ast::Func)> = stratum1.iter().map(|(n, f)| (n.as_str(), f)).collect();
    let s2: Vec<(&str, &ast::Func)> = stratum2.iter().map(|(n, f)| (n.as_str(), f)).collect();

    let (post_s1, _) = if s1.is_empty() {
        (pop_state.clone(), Vec::new())
    } else { crate::evaluate::forward_chain_defs_state(&s1, &pop_state) };
    let (post, _) = if s2.is_empty() {
        (post_s1, Vec::new())
    } else { crate::evaluate::forward_chain_defs_state(&s2, &post_s1) };

    let rec_cell = ast::fetch_or_phi("Task_is_recommended", &post);
    let recs: Vec<String> = match &rec_cell {
        Object::Seq(items) => items.iter()
            .filter_map(|f| ast::binding(f, "Task").map(String::from)).collect(),
        Object::Map(m) => m.values()
            .filter_map(|f| ast::binding(f, "Task").map(String::from)).collect(),
        _ => Vec::new(),
    };
    let para_cell = ast::fetch_or_phi("Task_is_parallelizable", &post);
    assert!(recs.contains(&"a".to_string()),
        "After defs_to_state + metacompose round-trip: Task 'a' must \
         be recommended. recs={:?}, parallelizable={:?}, s1_count={}, \
         s2_count={}",
        recs, para_cell, s1.len(), s2.len());
}

// ─── #927 — tight repro: cross-noun variable unification in
// derivation rule body. The bridge in apps/tasks reads
// `Task has Task Status iff Resource is currently in Status and Task
// Status is Status and Task is Resource` — three antecedents where
// the second and third are value-equality clauses binding consequent
// variables (Task, Task Status) to upstream antecedent variables
// (Resource, Status). This minimal repro tests the same shape:
// derive `Bar has Color iff Source has Hue and Bar is Source and
// Color is Hue` — Bar binds to Source, Color binds to Hue.
#[test]
fn cross_noun_variable_unification_in_derivation_body() {
    let src = r#"# task-927 tight repro
Source(.id) is an entity type.
Bar(.id) is an entity type.
Hue is a value type.
Color is a value type.

## Fact Types
Source has Hue.
Bar has Color.

## Derivation Rules
* Bar has Color iff Source has Hue and Bar is Source and Color is Hue.

## Instance Facts
Source 'src1' has Hue 'red'.
"#;
    let state = parse_to_state(src).expect("parse");
    let model = compile::compile(&state);
    let bar_color_func = model.derivations.iter()
        .find(|d| d.text.starts_with("Bar has Color iff"))
        .map(|d| d.func.clone())
        .expect("bridge rule must compile");

    let pop = ast::encode_state(&state);
    let out = ast::apply(&bar_color_func, &pop, &state);
    let derived = decode_derived(&out);
    let bar_colors: Vec<(String, String)> = derived.iter()
        .filter(|(ft, _, _)| ft == "Bar_has_Color")
        .filter_map(|(_, _, bs)| {
            let bar = bs.iter().find(|(k, _)| k == "Bar").map(|(_, v)| v.clone())?;
            let color = bs.iter().find(|(k, _)| k == "Color").map(|(_, v)| v.clone())?;
            Some((bar, color))
        })
        .collect();

    // Diagnostic: dump the rule shape so we can compare with the live
    // bridge that fails. The cross-noun clauses (`Bar is Source`,
    // `Color is Hue`) should appear somewhere in the rule structure.
    let data = compile::cell_index_from_state(&state);
    for rule in &data.derivation_rules {
        if rule.text.starts_with("Bar has Color iff") {
            eprintln!("[probe-synth] rule {} kind={:?} join_on={:?} match_on={:?} consequent_bindings={:?}",
                rule.id, rule.kind, rule.join_on, rule.match_on, rule.consequent_bindings);
            eprintln!("[probe-synth] antecedent_sources: {:?}", rule.antecedent_sources);
            eprintln!("[probe-synth] antecedent_role_literals: {:?}", rule.antecedent_role_literals);
        }
    }

    assert_eq!(bar_colors, vec![("src1".to_string(), "red".to_string())],
        "Bridge rule with cross-noun unification must derive Bar='src1' \
         Color='red' from Source 'src1' has Hue 'red'. Got: {:?}", derived);
}

// ─── #927 — extension of tight repro: upstream antecedent is itself
// DERIVED rather than stored. The previous test (stored upstream)
// passes. The live apps/tasks failure has a chain shape where
// `Task has Task Status` derives from `Resource is currently in
// Status`, which is itself derived from SM cells. This isolates
// whether the derived-antecedent interaction is what breaks
// downstream cross-noun unification.
//
// Setup:
//   Origin has Hue   (stored)
//   Source has Hue   (derived: iff Origin has Hue and Source is Origin)
//   Bar has Color    (derived: iff Source has Hue and Bar is Source and Color is Hue)
//
// Expectation: both derivations fire to fixpoint and the chain
// produces Bar='o1', Color='red'. If the bug reproduces here,
// fingerprint should look like Source_has_Hue or Bar_has_Color
// carrying empty-bindings Seq([Seq([])]) — same shape as the live
// apps/tasks symptom.
#[test]
fn cross_noun_unification_with_derived_upstream_antecedent() {
    let src = r#"# task-927 chain repro
Origin(.id) is an entity type.
Source(.id) is an entity type.
Bar(.id) is an entity type.
Hue is a value type.
Color is a value type.

## Fact Types
Origin has Hue.
Source has Hue.
Bar has Color.

## Derivation Rules
* Source has Hue iff Origin has Hue and Source is Origin.
* Bar has Color iff Source has Hue and Bar is Source and Color is Hue.

## Instance Facts
Origin 'o1' has Hue 'red'.
"#;
    let state = parse_to_state(src).expect("parse");
    let model = compile::compile(&state);

    let derivations: Vec<(&str, &Func)> = model.derivations.iter()
        .map(|d| (d.id.as_str(), &d.func))
        .collect();

    let (post, _) = crate::evaluate::forward_chain_defs_state(&derivations, &state);

    let extract = |cell: &Object, role_a: &str, role_b: &str| -> Vec<(String, String)> {
        let unpack = |f: &Object| -> Option<(String, String)> {
            let a = ast::binding(f, role_a).map(String::from)?;
            let b = ast::binding(f, role_b).map(String::from)?;
            Some((a, b))
        };
        match cell {
            Object::Seq(items) => items.iter().filter_map(unpack).collect(),
            Object::Map(m) => m.values().filter_map(unpack).collect(),
            _ => Vec::new(),
        }
    };

    let source_hue_cell = ast::fetch_or_phi("Source_has_Hue", &post);
    let bar_color_cell = ast::fetch_or_phi("Bar_has_Color", &post);
    let source_hues = extract(&source_hue_cell, "Source", "Hue");
    let bar_colors = extract(&bar_color_cell, "Bar", "Color");

    assert_eq!(source_hues, vec![("o1".to_string(), "red".to_string())],
        "Stratum-1: Source has Hue must derive 'o1','red' from Origin \
         'o1' has Hue 'red' (via `Source is Origin` unification).\n\
         Source_has_Hue cell raw: {:?}", source_hue_cell);

    assert_eq!(bar_colors, vec![("o1".to_string(), "red".to_string())],
        "Stratum-2: Bar has Color must derive 'o1','red' from the \
         derived upstream Source has Hue.\n\
         Source_has_Hue intermediate: {:?}\nBar_has_Color cell raw: {:?}",
        source_hue_cell, bar_color_cell);
}

// ─── task-930 — view rule materializes lazily on read.
// Marks `Bar has Color` FT as fully derived (`*` suffix per Halpin
// ORM2). The chain doesn't run the rule (compile_to_defs_state
// filters Stored-only for derivation cells); reading the consequent
// cell via Func::Fetch triggers resolve_view, which evaluates the
// view's func against the current state and returns the derived
// facts.
#[test]
fn view_materialization_computes_lazily_on_read() {
    let src = r#"# task-930 view repro
Source(.id) is an entity type.
Bar(.id) is an entity type.
Hue is a value type.
Color is a value type.

## Fact Types
Source has Hue.
Bar has Color. *

## Derivation Rules
* Bar has Color iff Source has Hue and Bar is Source and Color is Hue.

## Instance Facts
Source 'src1' has Hue 'red'.
"#;
    let state = parse_to_state(src).expect("parse");
    let data = compile::cell_index_from_state(&state);

    // The rule itself must be flagged as View (consequent FT marked `*`).
    let bar_color_rule = data.derivation_rules.iter()
        .find(|r| r.text.starts_with("Bar has Color iff"))
        .expect("rule must compile");
    assert!(matches!(bar_color_rule.materialization,
        crate::types::MaterializationPolicy::View),
        "View marker not detected; got materialization={:?}",
        bar_color_rule.materialization);

    // Build the full def-state — view def gets emitted under
    // `view:Bar_has_Color` and Func::Fetch should pick it up.
    let defs = compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);

    // task-930 v2: views emit ONLY under `view:` (chain skips them);
    // extract_facts_from_pop now consults views via Func::FetchOrPhi
    // → resolve_view, so downstream rules see view-derived bindings
    // without the v1 eager-emit workaround.
    let derivation_def = ast::fetch_raw(
        &format!("derivation:{}", bar_color_rule.id), &d);
    assert!(matches!(derivation_def, ast::Object::Bottom),
        "View rule must NOT emit derivation: def; got {:?}", derivation_def);

    // Sanity: view def IS present.
    let view_def = ast::fetch_raw("view:Bar_has_Color", &d);
    assert!(!matches!(view_def, ast::Object::Bottom),
        "View def missing for Bar_has_Color");

    // Read the cell through Func::Fetch — should trigger lazy view eval
    // and return derived facts even though nothing materialized them.
    let fetch_input = Object::seq(vec![
        Object::atom("Bar_has_Color"),
        d.clone(),
    ]);
    let result = ast::apply(&ast::Func::Fetch, &fetch_input, &d);
    let bar_colors: Vec<(String, String)> = match &result {
        Object::Seq(items) => items.iter().filter_map(|f| {
            let bar = ast::binding(f, "Bar").map(String::from)?;
            let color = ast::binding(f, "Color").map(String::from)?;
            Some((bar, color))
        }).collect(),
        _ => Vec::new(),
    };
    assert_eq!(bar_colors, vec![("src1".to_string(), "red".to_string())],
        "View must compute on Func::Fetch read. Got: {:?}", result);
}

// ─── task-930 v2 — downstream rule whose antecedent is a `*`-marked
// FT sees view-derived bindings WITHOUT the v1 emit-to-derivation:
// workaround. Verifies the chain's `extract_facts_from_pop` (which
// now composes Func::FetchOrPhi) resolves views lazily on read.
//
// Shape:
//   Bar has Color iff Source has Hue and Bar is Source and Color is Hue.   // view
//   Bar is colorful iff Bar has Color 'red'.                               // stored
//
// `Bar has Color` is View-marked (no `derivation:` def emitted). The
// downstream `Bar is colorful` rule's antecedent reader hits the
// encoded pop for `Bar_has_Color`, finds nothing (chain skipped the
// view), falls through to resolve_view via Func::FetchOrPhi, and
// gets the view-computed bindings. The colorful consequent then
// fires for src1 (Hue=red).
#[test]
fn downstream_rule_sees_view_derived_facts_via_lazy_eval() {
    let src = r#"# task-930 v2 downstream-view-read repro
Source(.id) is an entity type.
Bar(.id) is an entity type.
Hue is a value type.
Color is a value type.

## Fact Types
Source has Hue.
Bar has Color. *
Bar is colorful.

## Derivation Rules
* Bar has Color iff Source has Hue and Bar is Source and Color is Hue.
Bar is colorful iff Bar has Color 'red'.

## Instance Facts
Source 'src1' has Hue 'red'.
"#;
    let state = parse_to_state(src).expect("parse");
    let data = compile::cell_index_from_state(&state);

    // The Bar-has-Color rule must be flagged View.
    let bar_color_rule = data.derivation_rules.iter()
        .find(|r| r.text.starts_with("Bar has Color iff"))
        .expect("Bar has Color rule must compile");
    assert!(matches!(bar_color_rule.materialization,
        crate::types::MaterializationPolicy::View),
        "Bar has Color must be View-marked; got materialization={:?}",
        bar_color_rule.materialization);

    // The downstream colorful rule must NOT be View (the `*` lives
    // on the FT, not the rule; the downstream rule has no `*`).
    let colorful_rule = data.derivation_rules.iter()
        .find(|r| r.text.starts_with("Bar is colorful iff"))
        .expect("Bar is colorful rule must compile");
    assert!(matches!(colorful_rule.materialization,
        crate::types::MaterializationPolicy::Stored),
        "Bar is colorful must be Stored (downstream of view); got {:?}",
        colorful_rule.materialization);

    // Build the def-state and prove the v2 layout: NO derivation:
    // def for the view, only a view: def.
    let defs = compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);
    let view_derivation_def = ast::fetch_raw(
        &format!("derivation:{}", bar_color_rule.id), &d);
    assert!(matches!(view_derivation_def, ast::Object::Bottom),
        "View rule must NOT emit a derivation: def under v2; got {:?}",
        view_derivation_def);
    let view_def = ast::fetch_raw("view:Bar_has_Color", &d);
    assert!(!matches!(view_def, ast::Object::Bottom),
        "View def missing for Bar_has_Color");

    // The downstream Stored rule DOES emit a derivation: def — it's
    // what the chain runs against the encoded pop, and its
    // antecedent extractor will lazy-resolve the view at read time.
    let colorful_derivation_def = ast::fetch_raw(
        &format!("derivation:{}", colorful_rule.id), &d);
    assert!(!matches!(colorful_derivation_def, ast::Object::Bottom),
        "Stored downstream rule must emit a derivation: def");

    // Now apply the colorful rule's compiled func against an encoded
    // pop containing only the seed Source_has_Hue cell. The view
    // is NOT populated in the encoded pop. The colorful rule's
    // extract_facts_from_pop("Bar_has_Color") should hit the lazy
    // view resolution and find one Bar/Color binding to filter on.
    let model = compile::compile(&state);
    let colorful_func = model.derivations.iter()
        .find(|d| d.id == colorful_rule.id)
        .expect("colorful derivation func must compile")
        .func.clone();
    let pop = ast::encode_state(&state);
    let out = ast::apply(&colorful_func, &pop, &d);
    let bars: Vec<String> = out.as_seq().map(|items| items.iter()
        .filter_map(|f| {
            let env = f.as_seq()?;
            if env.len() < 3 { return None; }
            let bindings = env[2].as_seq()?;
            bindings.iter().find_map(|p| {
                let kv = p.as_seq()?;
                if kv.len() != 2 { return None; }
                if kv[0].as_atom() == Some("Bar") {
                    kv[1].as_atom().map(String::from)
                } else { None }
            })
        }).collect()).unwrap_or_default();
    assert!(bars.contains(&"src1".to_string()),
        "Downstream `Bar is colorful` must fire for src1 via lazy view \
         eval. Got bars={:?}; raw colorful output: {:?}", bars, out);
}

// ─── task-930 v2 follow-up: even when the view-marked FT cell is
// PRESENT-BUT-EMPTY in the encoded population (the typical post
// `drop derived cells before forward-chain` shape — apps/tasks
// recompile path), `extract_facts_from_pop` must STILL fall through
// to lazy view resolution. Without this, encoded_pop_lookup returns
// the empty entry and Func::FetchOrPhi short-circuits to phi, the
// downstream rule sees nothing, and the cascade collapses despite
// the view producing valid derivations from upstream antecedents.
#[test]
fn downstream_rule_sees_view_facts_even_when_view_cell_is_empty_in_pop() {
    let src = r#"# task-930 v2 empty-cell short-circuit repro
Source(.id) is an entity type.
Bar(.id) is an entity type.
Hue is a value type.
Color is a value type.

## Fact Types
Source has Hue.
Bar has Color. *
Bar is colorful.

## Derivation Rules
* Bar has Color iff Source has Hue and Bar is Source and Color is Hue.
Bar is colorful iff Bar has Color 'red'.

## Instance Facts
Source 'src1' has Hue 'red'.
"#;
    let state = parse_to_state(src).expect("parse");
    let model = compile::compile(&state);
    let defs = compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);

    // Mirror the chain's pre-chain "drop derived cells" pass: store
    // an explicit empty Seq under Bar_has_Color in the state before
    // encoding. This is what the live compile path does for every
    // derived cell so the chain re-derives from primary facts each
    // round (LFP per request, #836).
    let dropped_state = ast::store(
        "Bar_has_Color",
        ast::Object::phi(),  // empty Seq, matching pre-chain drop
        &state,
    );
    let pop = ast::encode_state(&dropped_state);

    // Confirm the encoded pop DOES contain Bar_has_Color with an
    // empty entry. That's the live shape; the test is meaningless
    // without this precondition.
    let bar_color_entry = pop.as_seq().and_then(|cells| cells.iter().find(|c| {
        c.as_seq().and_then(|items| items.first().and_then(|n| n.as_atom())) == Some("Bar_has_Color")
    }).cloned()).expect("encoded pop must include Bar_has_Color cell (even if empty)");
    let entry_facts = bar_color_entry.as_seq().and_then(|items| items.get(1).cloned())
        .expect("Bar_has_Color entry must have a facts slot");
    let entry_is_empty = matches!(&entry_facts,
        ast::Object::Seq(items) if items.is_empty());
    assert!(entry_is_empty,
        "Bar_has_Color must be present-but-empty in encoded pop \
         (was: {:?})", entry_facts);

    // Now apply the downstream colorful rule. Its antecedent
    // extract_facts_from_pop("Bar_has_Color") must NOT short-circuit
    // on the empty entry — it must fall through to resolve_view.
    let colorful_func = model.derivations.iter()
        .find(|der| der.text.starts_with("Bar is colorful iff"))
        .expect("colorful rule must compile").func.clone();
    let out = ast::apply(&colorful_func, &pop, &d);
    let bars: Vec<String> = out.as_seq().map(|items| items.iter()
        .filter_map(|f| {
            let env = f.as_seq()?;
            if env.len() < 3 { return None; }
            let bindings = env[2].as_seq()?;
            bindings.iter().find_map(|p| {
                let kv = p.as_seq()?;
                if kv.len() == 2 && kv[0].as_atom() == Some("Bar") {
                    kv[1].as_atom().map(String::from)
                } else { None }
            })
        }).collect()).unwrap_or_default();
    assert!(bars.contains(&"src1".to_string()),
        "Downstream colorful must fire for src1 via lazy view eval even \
         when Bar_has_Color is empty in encoded pop. Got bars={:?}; \
         raw output: {:?}", bars, out);
}

// ─── #924 — cross-noun unification with `that .../some...` quantifier.
// Hypothesis: the live bridge `Task has Task Status iff that Resource
// is currently in some Status and Task Status is Status and Task is
// Resource` fails because the `that .../some...` form changes how the
// compiler captures the cross-noun clauses. Test with the quantifier
// to isolate.
#[test]
fn cross_noun_unification_with_that_some_quantifier() {
    let src = r#"# task-924 quantifier repro
Source(.id) is an entity type.
Bar(.id) is an entity type.
Hue is a value type.
Color is a value type.

## Fact Types
Source has Hue.
Bar has Color.

## Derivation Rules
* Bar has Color iff that Source has some Hue and Bar is Source and Color is Hue.

## Instance Facts
Source 'src1' has Hue 'red'.
"#;
    let state = parse_to_state(src).expect("parse");
    let model = compile::compile(&state);
    let bar_color_func = model.derivations.iter()
        .find(|d| d.text.starts_with("Bar has Color iff"))
        .map(|d| d.func.clone())
        .expect("bridge rule must compile");

    let pop = ast::encode_state(&state);
    let out = ast::apply(&bar_color_func, &pop, &state);
    let derived = decode_derived(&out);
    let bar_colors: Vec<(String, String)> = derived.iter()
        .filter(|(ft, _, _)| ft == "Bar_has_Color")
        .filter_map(|(_, _, bs)| {
            let bar = bs.iter().find(|(k, _)| k == "Bar").map(|(_, v)| v.clone())?;
            let color = bs.iter().find(|(k, _)| k == "Color").map(|(_, v)| v.clone())?;
            Some((bar, color))
        })
        .collect();

    let data = compile::cell_index_from_state(&state);
    for rule in &data.derivation_rules {
        if rule.text.starts_with("Bar has Color iff") {
            eprintln!("[probe-quant] rule {} kind={:?} join_on={:?} match_on={:?} consequent_bindings={:?}",
                rule.id, rule.kind, rule.join_on, rule.match_on, rule.consequent_bindings);
            eprintln!("[probe-quant] antecedent_sources: {:?}", rule.antecedent_sources);
            eprintln!("[probe-quant] antecedent_role_literals: {:?}", rule.antecedent_role_literals);
        }
    }

    assert_eq!(bar_colors, vec![("src1".to_string(), "red".to_string())],
        "Bridge with `that .../some...` quantifier must derive Bar='src1' Color='red'. \
         Got: {:?}", derived);
}

// ─── #924 — cross-noun unification + MAP-backed upstream. Mirrors the
// live apps/tasks bridge `Task has Task Status iff Resource is
// currently in Status and Task Status is Status and Task is Resource`
// shape: cross-noun unification (Task<->Resource, Task Status<->Status)
// AND the upstream cell is Map-backed (per cell_put_keyed emit path).
//
// The prior cross-noun unification tests (stored + derived antecedent)
// use Seq cells throughout. This test pins what live apps/tasks
// actually fails on: bridge func reads Map cell, emits nothing.
#[test]
fn cross_noun_unification_with_map_backed_upstream() {
    let src = r#"# task-924 map+cross-noun repro
Source(.id) is an entity type.
Bar(.id) is an entity type.
Hue is a value type.
Color is a value type.

## Fact Types
Source has Hue.
  Each Source has at most one Hue.
Bar has Color.
  Each Bar has at most one Color.

## Derivation Rules
* Bar has Color iff Source has Hue and Bar is Source and Color is Hue.
"#;
    let state_initial = parse_to_state(src).expect("parse");
    let model = compile::compile(&state_initial);
    let bar_color_func = model.derivations.iter()
        .find(|d| d.text.starts_with("Bar has Color iff"))
        .map(|d| d.func.clone())
        .expect("bridge rule must compile");

    // Build upstream via cell_put_keyed so Source_has_Hue lands as Map
    // (same shape live forward-chain produces when the FT has a UC).
    let mut state = state_initial.clone();
    state = ast::cell_put_keyed(
        "Source_has_Hue", &["Source"],
        ast::fact_from_pairs(&[("Source", "src1"), ("Hue", "red")]),
        &state,
    ).unwrap();
    // Sanity: cell is Map.
    let sh_cell = ast::fetch("Source_has_Hue", &state);
    assert!(matches!(sh_cell, Object::Map(_)),
        "test fixture: Source_has_Hue must be Map, got {:?}", sh_cell);

    let pop = ast::encode_state(&state);
    let out = ast::apply(&bar_color_func, &pop, &state);
    let derived = decode_derived(&out);
    let bar_colors: Vec<(String, String)> = derived.iter()
        .filter(|(ft, _, _)| ft == "Bar_has_Color")
        .filter_map(|(_, _, bs)| {
            let bar = bs.iter().find(|(k, _)| k == "Bar").map(|(_, v)| v.clone())?;
            let color = bs.iter().find(|(k, _)| k == "Color").map(|(_, v)| v.clone())?;
            Some((bar, color))
        })
        .collect();

    assert_eq!(bar_colors, vec![("src1".to_string(), "red".to_string())],
        "Bridge rule with cross-noun unification + Map-backed upstream must \
         derive Bar='src1' Color='red'. Got derived: {:?}\n\
         Source_has_Hue cell: {:?}", derived, sh_cell);
}

// ─── #922 — apply-path: stratum-2 rule whose POSITIVE antecedent reads
// another stratum-2 rule's consequent.
//
// Setup mirrors apps/tasks: file-conflicting + preceded fire from
// stratum 1 (purely positive); parallelizable + recommended both
// land in stratum 2 because each carries at least one AbsenceOf.
// `recommended` reads `Task is parallelizable` as a POSITIVE
// antecedent — same stratum, just emitted by another rule in the
// same bucket.
//
// command.rs:1020-1033 (create_via_defs) and command.rs:2100-2113
// (update_via_defs) call `forward_chain_defs_state(stratum1)` then
// `forward_chain_defs_state(stratum2)` sequentially. The inner
// fixpoint of the stratum-2 call SHOULD let `recommended` re-fire
// in round 2 against state where `parallelizable` has integrated.
// If it doesn't, the cell stays empty even though every antecedent
// is satisfied — the apps/tasks symptom.
//
// This test pins the apply path end-to-end so a regression in
// either the chain orchestrator or the integrate-derived path
// surfaces here.
#[test]
fn apply_path_chain_fixpoints_stratum2_positive_dep_on_stratum2_emit() {
    use crate::ast;
    use crate::evaluate;

    let src = r#"# task-922 apply-path repro
Task(.id) is an entity type.
Task Readiness is a value type.
Task Priority is a value type.

## Fact Types
Task has Task Readiness.
Task has Task Priority.
Task is parallelizable.
Task is epic.
Task is recommended.

## Derivation Rules
* Task is parallelizable iff Task has Task Readiness 'ready' and Task is not epic.
* Task is recommended iff Task has Task Readiness 'ready' and Task has Task Priority 'p0' and Task is parallelizable and Task is not epic.

## Instance Facts
Task 'a' has Task Readiness 'ready'.
Task 'a' has Task Priority 'p0'.
Task 'b' has Task Readiness 'ready'.
Task 'b' has Task Priority 'p0'.
"#;

    let state = crate::parse_forml2::parse_to_state(src).expect("parse");
    let defs = crate::compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);
    let model = crate::compile::compile(&state);

    // Partition compiled derivations by uses_negation flag — the
    // same partitioning compile_to_defs_state uses when emitting
    // `derivation:rule_*` vs `derivation_strat2:rule_*` cells.
    let s1: Vec<(&str, &ast::Func)> = model.derivations.iter()
        .filter(|cd| !cd.uses_negation)
        .map(|cd| (cd.id.as_str(), &cd.func))
        .collect();
    let s2: Vec<(&str, &ast::Func)> = model.derivations.iter()
        .filter(|cd| cd.uses_negation)
        .map(|cd| (cd.id.as_str(), &cd.func))
        .collect();
    let s1_refs = s1.as_slice();
    let s2_refs = s2.as_slice();

    // Sequential pattern — what command.rs does today.
    let (post_s1, _) = if s1_refs.is_empty() {
        (d.clone(), Vec::new())
    } else {
        evaluate::forward_chain_defs_state(s1_refs, &d)
    };
    let (post_seq, _) = if s2_refs.is_empty() {
        (post_s1.clone(), Vec::new())
    } else {
        evaluate::forward_chain_defs_state(s2_refs, &post_s1)
    };

    // Alternating pattern — forward_chain_stratified does outer
    // alternation until both passes produce nothing novel.
    let (post_alt, _) = evaluate::forward_chain_stratified(
        s1_refs, s2_refs, &d, 10);

    let count = |state: &ast::Object, cell: &str| -> usize {
        match ast::fetch_or_phi(cell, state) {
            ast::Object::Seq(items) => items.len(),
            ast::Object::Map(m) => m.len(),
            _ => 0,
        }
    };

    let seq_para = count(&post_seq, "Task_is_parallelizable");
    let seq_rec = count(&post_seq, "Task_is_recommended");
    let alt_para = count(&post_alt, "Task_is_parallelizable");
    let alt_rec = count(&post_alt, "Task_is_recommended");

    // The alternating shape MUST land all 2 tasks in both cells.
    // The sequential shape (the bug) should leave recommended
    // empty (or under-firing) when stratum-2 rules emit-depend on
    // each other in the same bucket. If sequential also produces
    // 2 in recommended, then the inner fixpoint of
    // forward_chain_defs_state on stratum2 is doing the work and
    // the apps/tasks symptom has a different root cause.
    assert_eq!(alt_para, 2,
        "alternating chain: both tasks must be parallelizable");
    assert_eq!(alt_rec, 2,
        "alternating chain: both tasks must be recommended");

    // The critical assertion: sequential MUST match alternating
    // (because the inner fixpoint should saturate stratum 2).
    // If this assertion fails, the bug IS the sequential pattern
    // and the fix is to switch command.rs to forward_chain_stratified.
    assert_eq!(seq_rec, alt_rec,
        "sequential chain produced {} recommended, alternating produced {}. \
         If sequential under-fires, command.rs:1020/2100 must switch to \
         forward_chain_stratified for the alternating-fixpoint guarantee. \
         seq_para={}, alt_para={}.",
        seq_rec, alt_rec, seq_para, alt_para);
}

// ─── Category 14: Join-path with consequent role literal pin (#814) ─
//
// Shape: `* X has Y 'y' iff some X has A 'a' and that X has B 'b'` — a
// Join-routed rule (≥2 antecedents joined on a shared noun) whose
// consequent FT has a role (`Y`) pinned to a literal value. The role
// being pinned (`Y`) is NOT shared with any antecedent FT — it's only
// on the consequent FT.
//
// Pre-#814 bug: `compile_join_derivation`'s N≥2 path built
// `binding_parts` by walking `binding_nouns` (the consequent FT's
// declared role nouns) and dropping any noun not found on any
// antecedent — so a target role pinned to a literal in the consequent
// itself was silently lost. The derived fact was missing that role
// entirely, breaking downstream queries like `select X where
// Y = 'y'`.
//
// `compile_explicit_derivation` already handles this correctly via
// its M1b path (compile.rs:3322-3360, the "literal pin wins" branch).
// This test mirrors the join + consequent-literal-pin shape from the
// #814 audit's Merge example, in the same `some X ... and that X ...`
// surface form the existing #818 test exercises (the Merge audit's
// `concerns some C that has S` surface form is parsed today as a
// 1-antecedent ModusPonens rule with an embedded `that-clause`, not
// as a Join routing).

#[test]
fn shape_join_with_consequent_role_literal_pin_emits_literal() {
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

    // Consequent literal-pin must survive parsing — `Status` only
    // appears on the consequent FT, never on either antecedent. This
    // is exactly the shape the join compiler used to drop on the
    // floor.
    assert!(
        rule.consequent_role_literals.iter().any(|l|
            l.role == "Status" && l.value == "urgent"),
        "expected consequent_role_literals to pin Status='urgent', got {:#?}",
        rule.consequent_role_literals,
    );

    // Eval: only the d-yes Doc satisfies both antecedent literals
    // (Priority='high' AND Kind='critical'), so it must derive a
    // single Doc-has-Status fact carrying Status='urgent'.
    let out = apply_to_facts(&func, &[
        ("Doc_has_Priority", &[("Doc", "d-yes"),    ("Priority", "high")]),
        ("Doc_has_Kind",     &[("Doc", "d-yes"),    ("Kind", "critical")]),
        ("Doc_has_Priority", &[("Doc", "d-no-pri"), ("Priority", "low")]),
        ("Doc_has_Kind",     &[("Doc", "d-no-pri"), ("Kind", "critical")]),
    ]);
    let derived = decode_derived(&out);

    // The load-bearing assertion: the derived fact MUST carry the
    // consequent-pinned literal as a binding. Pre-#814 this binding
    // is silently absent — the derived fact has only the Doc binding
    // (or empty bindings) without Status.
    let urgent_facts: Vec<_> = derived.iter()
        .filter(|(_, _, b)| b.iter().any(|(k, v)|
            k == "Status" && v == "urgent"))
        .collect();
    assert!(!urgent_facts.is_empty(),
        "derived fact MUST carry Status='urgent' binding\n\
         (#814: compile_join_derivation dropped consequent_role_literals)\n\
         got derived: {:#?}", derived);

    // And it must be the d-yes Doc, not d-no-pri (antecedent literal
    // filter still applies — Priority='high' only matches d-yes).
    let doc_ids_with_urgent: Vec<String> = urgent_facts.iter()
        .flat_map(|(_, _, b)| b.iter())
        .filter(|(k, _)| k == "Doc")
        .map(|(_, v)| v.clone())
        .collect();
    assert!(doc_ids_with_urgent.iter().any(|d| d == "d-yes"),
        "d-yes (Priority=high, Kind=critical) MUST derive Status='urgent'; got Doc bindings {:?}\nfull derived: {:#?}",
        doc_ids_with_urgent, derived);
    assert!(!doc_ids_with_urgent.iter().any(|d| d == "d-no-pri"),
        "d-no-pri (Priority=low) must NOT derive Status='urgent' — antecedent literal filter\n\
         got Doc bindings {:?}\nfull derived: {:#?}",
        doc_ids_with_urgent, derived);
}

// ─── Category 15: Subscript-driven join + AbsenceOf (task-918-subscript-and-fallback-absence) ──
//
// task-918 follow-up: compile_explicit_derivation has THREE more
// branches that hardcoded `uses_negation: false` AND don't compose
// absent_guard predicates at all when the rule has AbsenceOf
// antecedents:
//
//   (1) subscript-driven join branch, success return (compile.rs site
//       was line ~4428 at task ticket time). Pre-fix: the iterative
//       join loop walked all `antecedent_ids[k]` including AbsenceOf
//       positions where antecedent_ids[k] = "" → data.fact_types
//       lookup failed → fast-path return phi. The whole rule emitted
//       no facts even when the AbsenceOf guard was satisfied.
//
//   (2) subscript-driven join branch, fast-path failure return (compile.rs
//       site was line ~4258). Same trigger, same emission of phi.
//
//   (3) function-end existence-check fallback (compile.rs site was line
//       ~5031). Pre-fix the fallback's `ant_checks` extracted facts for
//       all antecedent positions; AbsenceOf positions yielded extract(i,
//       "") = phi → Not(NullTest)(phi) = false → all_hold conjunction
//       collapsed → rule never fired.
//
// Fix mirrors task-814-join-absence's port to compile_join_derivation:
// partition antecedent_sources into positive_idx / absence_idx, build
// the join/extract product over positives only, gate with absent_guard
// predicates per AbsenceOf antecedent, set uses_negation true when any
// AbsenceOf is present so the chain orchestrator routes the rule into
// stratum 2.
//
// Three tests, one per branch, each exercising both the positive case
// (rule fires when AbsenceOf guard is satisfied) and the counterfactual
// (rule does NOT fire when negation matches).

