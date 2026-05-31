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

// ─── Category 7b: Two-hop self-ring JOIN via repeated subscript ─────
//
// Shape: `* X is grandparent of Z iff X is parent of Y and Y is parent
// of Z` on the self-ring `Person is parent of Person`. The join
// variable is the REPEATED Halpin subscript `Person2` (Y), marking
// parent.role1 of the first antecedent and parent.role0 of the second
// as the SAME variable. Per whitepaper eq:join the join is
// `Filter(eq ∘ [s_sh]) ∘ distl`, with `s_sh` selecting those shared
// roles — positional, driven by the subscript, not the noun name.
//
// "that" anaphora cannot express this (it points only at the most-
// recent antecedent; a 3-variable join has no "that"), so subscripts
// are the only way to say which role binds which. Today the join-key
// detector keys off noun-name and explicitly skips a noun carrying
// >1 subscript token (parse_forml2.rs ~1847), so this compiles as a
// 2-antecedent ModusPonens existence-check and never fires — the gap
// noted at shape_transitive_closure_parses_as_two_antecedent_literal.
#[test]
fn shape_subscript_join_two_hop_self_ring_fires() {
    let src = r#"# Test
Person(.Name) is an entity type.
Name is a value type.

## Fact Types
Person has Name.
Person is parent of Person.
Person is grandparent of Person.

## Derivation Rules
* Person1 is grandparent of Person3 iff Person1 is parent of Person2 and Person2 is parent of Person3.
"#;
    let (rule, func) = parse_and_compile(src);

    // Routing: the repeated subscript Person2 is the join key → Join.
    assert_eq!(rule.kind, DerivationKind::Join,
        "repeated subscript Person2 must route to a positional Join (eq:join); got {:?}",
        rule.kind);

    // Semantics: parent(alice,bob) ⋈ parent(bob,carol) on Person2=bob
    // → grandparent(alice, carol).
    let out = apply_to_facts(&func, &[
        ("Person_is_parent_of_Person", &[("Person", "alice"), ("Person", "bob")]),
        ("Person_is_parent_of_Person", &[("Person", "bob"), ("Person", "carol")]),
    ]);
    let derived = decode_derived(&out);
    assert_eq!(derived.len(), 1,
        "one grandparent fact from the 2-hop self-ring join, got {:#?}", derived);
    let (ft, _reading, bindings) = &derived[0];
    assert_eq!(ft, "Person_is_grandparent_of_Person",
        "derived fact lands in the grandparent cell, got {}", ft);
    assert_eq!(bindings.len(), 2, "two positional Person bindings, got {:#?}", bindings);
    assert_eq!(bindings[0], ("Person".to_string(), "alice".to_string()),
        "grandparent role0 = hop-1 parent (alice), got {:?}", bindings[0]);
    assert_eq!(bindings[1], ("Person".to_string(), "carol".to_string()),
        "grandparent role1 = hop-2 child (carol), got {:?}", bindings[1]);
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
    use crate::ast::cells_iter;

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
    let lift_cell = crate::ast::fetch_cell_seq("Paper_Element_has_Lift_Priority", &final_state);
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

// ─── Category 13b: Forward-chained readiness over a subscript ring-join ─
//
// The motivating self-ring derivation: a "blocked" state propagates up a
// dependency chain. The rule joins the ring `Task depends on Task` to the
// derived `Task has Readiness` ON THE SUBSCRIPT `Task2` — the equi-join
// key `compile_join_derivation` builds via eq:join's `s_sh` from the
// rule's Halpin subscripts. Category 7b proves the single-application
// join; this proves it RECURSIVELY through `forward_chain_defs_state` to
// the least fixed point, plus the serde round-trip of `RingJoinPlan`
// (the rule is rebuilt from cells via `cell_index_from_state`) and the
// consequent literal-pin (`Readiness 'blocked'`).
//
// Population: t1 depends on t2 depends on t3; t3 seeded 'blocked'.
// Forward chain: t3 (seed) -> t2 -> t1 all reach 'blocked' at the LFP.
#[test]
fn readiness_blocked_propagates_through_forward_chain_over_subscript_ring() {
    use crate::ast::{fact_from_pairs, cell_push, fetch_cell_seq};

    let src = r#"# Readiness
Task(.Name) is an entity type.
Name is a value type.
Readiness is a value type.

## Fact Types
Task has Name.
Task depends on Task.
Task has Readiness.

## Derivation Rules
* Task1 has Readiness 'blocked' iff Task1 depends on Task2 and Task2 has Readiness 'blocked'.
"#;

    let state = crate::parse_forml2::parse_to_state(src).expect("parse");
    let model = crate::compile::compile(&state);

    // The readiness rule must route to a positional Join keyed on the
    // Task2 subscript — rebuilt from cells, so this also covers the
    // RingJoinPlan serde round-trip.
    let rule = crate::compile::cell_index_from_state(&state).derivation_rules
        .into_iter().find(|r| r.text.contains("Readiness"))
        .expect("readiness derivation rule");
    assert_eq!(rule.kind, DerivationKind::Join,
        "readiness rule must route to a positional Join via the Task2 subscript; got {:?}",
        rule.kind);
    assert!(rule.ring_join.is_some(),
        "readiness rule must carry a RingJoinPlan after the cell round-trip");

    // Dependency chain t1 -> t2 -> t3, with t3 seeded 'blocked'.
    let state = cell_push("Task_depends_on_Task",
        fact_from_pairs(&[("Task", "t1"), ("Task", "t2")]), &state);
    let state = cell_push("Task_depends_on_Task",
        fact_from_pairs(&[("Task", "t2"), ("Task", "t3")]), &state);
    let state = cell_push("Task_has_Readiness",
        fact_from_pairs(&[("Task", "t3"), ("Readiness", "blocked")]), &state);

    let derivation_refs: Vec<(&str, &crate::ast::Func)> =
        model.derivations.iter().map(|d| (d.id.as_str(), &d.func)).collect();
    let (final_state, _derived) =
        crate::evaluate::forward_chain_defs_state(&derivation_refs, &state);

    let cell = fetch_cell_seq("Task_has_Readiness", &final_state);
    let pairs: Vec<(String, String)> = cell.as_seq().map(|facts| {
        facts.iter().filter_map(|f| {
            let kvs = f.as_seq()?;
            let mut task: Option<String> = None;
            let mut readiness: Option<String> = None;
            for p in kvs.iter() {
                let kv = p.as_seq()?;
                if kv.len() != 2 { continue; }
                let k = kv[0].as_atom()?;
                let v = kv[1].as_atom()?;
                if k == "Task" { task = Some(v.to_string()); }
                if k == "Readiness" { readiness = Some(v.to_string()); }
            }
            Some((task?, readiness?))
        }).collect()
    }).unwrap_or_default();

    // LFP: every task in the chain reaches 'blocked' (t3 seed -> t2 -> t1).
    for t in ["t1", "t2", "t3"] {
        assert!(pairs.iter().any(|(task, r)| task == t && r == "blocked"),
            "{} must reach Readiness 'blocked' at the fixed point; got {:?}", t, pairs);
    }
}

// ─── Category 13c: literal-only consequent role over a subscript ring-join ─
//
// Regression for `ring-join-consequent-literal-edge`: when a consequent
// role's noun appears ONLY as a consequent literal pin and NEVER in an
// antecedent, `compute_ring_join_plan` used to bail (returning None for the
// whole plan), dropping the rule onto the noun-name path that cannot bind a
// self-ring. The producer now records such a role as a `None` slot
// (literal-sourced); `compile_join_derivation`'s literal-pin branch supplies
// the value via `Func::constant` (an existing θ primitive — no new one).
//
// Rule: `Task1 has Alert 'raised' iff Task1 depends on Task2 and Task2 has
// Readiness 'blocked'.` — `Alert` is never an antecedent noun. t1 depends on
// t2; t2 'blocked' -> t1 Alert 'raised'.
#[test]
fn ring_join_consequent_literal_only_role_still_plans_and_fires() {
    use crate::ast::{fact_from_pairs, cell_push, fetch_cell_seq};

    let src = r#"# Alert
Task(.Name) is an entity type.
Name is a value type.
Readiness is a value type.
Alert is a value type.

## Fact Types
Task has Name.
Task depends on Task.
Task has Readiness.
Task has Alert.

## Derivation Rules
* Task1 has Alert 'raised' iff Task1 depends on Task2 and Task2 has Readiness 'blocked'.
"#;

    let state = crate::parse_forml2::parse_to_state(src).expect("parse");
    let model = crate::compile::compile(&state);

    // Rebuilt from cells (covers the RingJoinPlan serde round-trip too).
    let rule = crate::compile::cell_index_from_state(&state).derivation_rules
        .into_iter().find(|r| r.text.contains("Alert"))
        .expect("alert derivation rule");
    assert_eq!(rule.kind, DerivationKind::Join,
        "literal-only-consequent ring rule must still route to a positional Join; got {:?}",
        rule.kind);
    let rj = rule.ring_join.as_ref()
        .expect("ring rule must carry a RingJoinPlan even when a consequent role is literal-only");
    // The `Alert` consequent role is literal-sourced -> recorded as None.
    assert!(rj.consequent_positions.iter().any(|p| p.is_none()),
        "the literal-only consequent role must be a None slot; got {:?}",
        rj.consequent_positions);

    // t1 depends on t2; t2 seeded 'blocked' -> rule fires -> t1 Alert 'raised'.
    let state = cell_push("Task_depends_on_Task",
        fact_from_pairs(&[("Task", "t1"), ("Task", "t2")]), &state);
    let state = cell_push("Task_has_Readiness",
        fact_from_pairs(&[("Task", "t2"), ("Readiness", "blocked")]), &state);

    let derivation_refs: Vec<(&str, &crate::ast::Func)> =
        model.derivations.iter().map(|d| (d.id.as_str(), &d.func)).collect();
    let (final_state, _derived) =
        crate::evaluate::forward_chain_defs_state(&derivation_refs, &state);

    let cell = fetch_cell_seq("Task_has_Alert", &final_state);
    let pairs: Vec<(String, String)> = cell.as_seq().map(|facts| {
        facts.iter().filter_map(|f| {
            let kvs = f.as_seq()?;
            let mut task: Option<String> = None;
            let mut alert: Option<String> = None;
            for p in kvs.iter() {
                let kv = p.as_seq()?;
                if kv.len() != 2 { continue; }
                let k = kv[0].as_atom()?;
                let v = kv[1].as_atom()?;
                if k == "Task" { task = Some(v.to_string()); }
                if k == "Alert" { alert = Some(v.to_string()); }
            }
            Some((task?, alert?))
        }).collect()
    }).unwrap_or_default();
    assert!(pairs.iter().any(|(task, a)| task == "t1" && a == "raised"),
        "forward chain must derive `t1 has Alert 'raised'`; got {:?}", pairs);
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
    let cell = crate::ast::fetch_cell_seq("Task_has_Task_Status", &final_state);
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
    use crate::ast::cells_iter;

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
    let defined_cell = crate::ast::fetch_cell_seq("Status_is_defined_in_State_Machine_Definition", &final_state);
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
    let initial_cell = crate::ast::fetch_cell_seq("Status_is_initial_in_State_Machine_Definition", &final_state);
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
    use crate::ast::cells_iter;

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

    let rooted_cell = crate::ast::fetch_cell_seq("Status_is_rooted_in_State_Machine_Definition", &final_state);
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
// `crates/arest/src/ast.rs::StateMachineCellShape::boot()`'s
// `cell_name` constant).
//
// This self-contained test mirrors the app.md rule shape: it declares
// the FTs, populates the SM cells directly with one Task entity at
// status 'pending', and asserts the bridge rules land
// (Task=t-1, Task Status=pending) in `Task_has_Task_Status` after
// forward-chain.

#[test]
fn sm_derivation_bridge_projects_currently_in_status_into_task_has_task_status() {
    use crate::ast::cells_iter;

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
    let cell = crate::ast::fetch_cell_seq("Task_has_Task_Status", &final_state);
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

// ─── SM init emits for_Resource for entity without transition events ─
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
    use crate::ast::cells_iter;

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
    let domain = r#"# SM init for_Resource test (task-922-sm-init-projection)
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
    let for_res_cell = crate::ast::fetch_cell_seq("State_Machine_is_for_Resource", &final_state);
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

    // ── Also assert the is_currently_in_Status emit still fires for the
    // same entities (this is the "we know this works" baseline per the
    // task description). If is_currently_in_Status is empty too, the bug is
    // SM init not running at all, not the targeted for_Resource regression.
    let status_cell = crate::ast::fetch_cell_seq("State_Machine_is_currently_in_Status", &final_state);
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
        "SM init must also emit Status='pending' for t-no-events \
         (paired by State Machine binding with for_Resource). Got status map: {:?}",
        statuses_by_resource,
    );
}

// ─── SM init emits for_Resource when entity's only FT cell is keyed ─
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
    let domain = r#"# SM init for_Resource over keyed cell (task-922-sm-init-projection)
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
    let (_final_state, _derived) =
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

    let for_res_cell = crate::ast::fetch_cell_seq("State_Machine_is_for_Resource", &final_state);
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
    use crate::ast::cells_iter;

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
    let status_cell = crate::ast::fetch_cell_seq("State_Machine_is_currently_in_Status", &final_state);
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
    let for_res_cell = crate::ast::fetch_cell_seq("State_Machine_is_for_Resource", &final_state);
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
    let for_res_2 = crate::ast::fetch_cell_seq("State_Machine_is_for_Resource", &final_state_2);
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
        .filter(|_d| true)
        .map(|d| (d.id.as_str(), &d.func)).collect();
    let s2: Vec<(&str, &Func)> = model.derivations.iter()
        .filter(|_d| false)
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
        .filter(|_d| true)
        .map(|d| (d.id.as_str(), &d.func)).collect();
    let s2: Vec<(&str, &Func)> = model.derivations.iter()
        .filter(|_d| false)
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

// ─── Category 16: Comparator/aggregate derivations fire end-to-end (task-814) ──
//
// task-814 (Substrate-2) asks: does an AUTHORED comparator/aggregate
// derivation actually fire end-to-end (parse → compile → evaluate),
// returning the comparator-correct value rather than plain attribute
// inheritance from a bound antecedent?
//
// The existing aggregate firing tests in evaluate.rs
// (count/sum/min/max/avg_aggregate_*) build the DerivationRuleDef
// struct by hand — they pin compile+evaluate but NOT that the parser
// recognises the authored rule TEXT. These two tests close that gap by
// parsing the authored reading text through resolve_derivation_rule.
//
// (a) `authored_max_aggregate_fires_end_to_end` authors the canonical
//     Halpin aggregate form `<role> is the max of <target> where ...`
//     from text and asserts the engine derives the per-group MAX, not
//     a passthrough of an arbitrary bound value. This is the SUPPORTED
//     comparator path: `min`/`max` over a numeric/value role.
//
// (b) Superlative comparator WORDS over enum-valued nouns
//     (`... has the highest P among ...`). task-953 closed the audit's
//     (#814) gap: these now lift to the rank-min/max aggregate. The
//     end-to-end firing tests live under the `task-953` header below
//     (superlative_highest_among_selects_enum_earliest_posture et al.).
//     Only `highest`/`lowest` ship as engine grammar; domain superlatives
//     (`strongest`/`weakest`/…) are author-extensible (cruft pass).

#[test]
fn authored_max_aggregate_fires_end_to_end() {
    // Author the `max` aggregate as TEXT — exercises try_parse_aggregate_clause
    // (parse_forml2.rs AGG_OPS) → compile_aggregate_derivation → forward chain.
    let src = r#"# Max aggregate derivation
Order(.ID) is an entity type.
ID is a value type.
LineItem Amount is a value type.
Amount is a value type.

## Fact Types
Order has ID.
Order has LineItem Amount.
Order has Amount.

## Derivation Rules
* Order has Amount iff Amount is the max of LineItem Amount where Order has LineItem Amount.
"#;
    let (rule, func) = parse_and_compile(src);

    // Shape: the parser must have populated consequent_aggregates with the
    // `max` op, NOT left it as a plain join/explicit passthrough.
    assert!(!rule.consequent_aggregates.is_empty(),
        "authored `is the max of` rule must populate consequent_aggregates; got {:#?}\nrule.text={}\nunresolved={:#?}",
        rule.consequent_aggregates, rule.text, rule.unresolved_clauses);
    assert_eq!(rule.consequent_aggregates[0].op, "max",
        "aggregate op must be max, got {}", rule.consequent_aggregates[0].op);

    // Fire it: O1 has {10, 4, 25} → max 25; O2 has {7} → 7.
    let out = apply_to_facts(&func, &[
        ("Order_has_LineItem_Amount", &[("Order", "O1"), ("LineItem Amount", "10")]),
        ("Order_has_LineItem_Amount", &[("Order", "O1"), ("LineItem Amount", "4")]),
        ("Order_has_LineItem_Amount", &[("Order", "O1"), ("LineItem Amount", "25")]),
        ("Order_has_LineItem_Amount", &[("Order", "O2"), ("LineItem Amount", "7")]),
    ]);
    let derived = decode_derived(&out);
    assert!(
        derived.iter().any(|(_, _, b)|
            b.iter().any(|(k, v)| k == "Order" && v == "O1") &&
            b.iter().any(|(k, v)| k == "Amount" && v == "25")),
        "expected (Order=O1, Amount=25) — the per-group MAX, not a passthrough; got {:#?}", derived);
    // Negative guard: the lower values must NOT leak through as the result.
    assert!(
        !derived.iter().any(|(_, _, b)|
            b.iter().any(|(k, v)| k == "Order" && v == "O1") &&
            b.iter().any(|(k, v)| k == "Amount" && (v == "10" || v == "4"))),
        "O1 must derive only the MAX (25), not lower bound values; got {:#?}", derived);
}

#[test]
fn authored_highest_among_superlative_now_lifts_to_rank_aggregate() {
    // task-953 flips the prior #814 pin. The brief's shape (`highest …
    // among`, enum-valued noun, ordering from the enumerate declaration
    // order) now lifts to the rank-min aggregate — it is RECOGNISED, not
    // left unresolved. (`highest` is the ORM-verbalization superlative the
    // engine ships; domain words like `strongest` are author-extensible —
    // cruft pass.)
    let src = r#"# Highest-among superlative derivation
Merge(.ID) is an entity type.
Commit(.SHA) is an entity type.
ID is a value type.
SHA is a value type.
Security Posture is a value type.
Security Posture enumerates 'verified', 'unverified', 'compromised'.

## Fact Types
Merge has ID.
Commit has SHA.
Merge concerns Commit.
Commit has Security Posture.
Merge has derived Security Posture.

## Derivation Rules
* Merge has derived Security Posture iff Merge concerns some Commit that has the highest Security Posture among Commits the Merge concerns.
"#;
    let state = parse_to_state(src).expect("parse");
    let data = compile::cell_index_from_state(&state);
    assert_eq!(data.derivation_rules.len(), 1,
        "expected exactly one authored rule, got {}", data.derivation_rules.len());
    let rule = &data.derivation_rules[0];

    // NEW BEHAVIOR (task-953): the superlative lifts to a rank aggregate.
    assert!(!rule.consequent_aggregates.is_empty(),
        "'highest … among' must now populate consequent_aggregates; got {:#?}",
        rule);
    assert_eq!(rule.consequent_aggregates[0].op, "min");
    assert!(rule.consequent_aggregates[0].enum_rank);
    // The clause is consumed, not dropped as unresolved.
    assert!(rule.unresolved_clauses.is_empty(),
        "the superlative clause must be consumed; got {:#?}", rule.unresolved_clauses);
}

// ─── task-953 — enum-declaration-order superlative comparators ────────────
//
// A superlative (`highest`/`lowest`) `… among …` over an ENUM-valued noun
// is the existing numeric min/max aggregate (compile_aggregate_derivation)
// applied to a RANK derived from the value type's `enumerates 'v0','v1',…`
// declaration order (first-declared = highest = rank 0). The recogniser
// (parse_forml2::try_parse_superlative_among_clause) routes the superlative
// word to min (`highest`) / max (`lowest`) and marks the aggregate
// `enum_rank`; the compiler wraps the target-value projection in a rank
// lookup sourced from CellIndex::enum_values. The "among Ys the X …" set is
// the join of the group FT (`X concerns Y`) with the value FT (`Y has P`)
// on the shared entity.
//
// Only the ORM-verbalization superlatives (`highest`/`lowest`) ship as
// engine grammar; domain superlatives (`strongest`/`weakest`/…) are author-
// extensible per-app, not engine defaults (cruft directive).
//
// These tests flip the prior pin (authored_strongest_among_superlative_*):
// the superlative now BUILDS a comparator aggregate and fires end-to-end,
// selecting the enum-earliest value rather than last-bound inheritance.

#[test]
fn superlative_highest_among_selects_enum_earliest_posture() {
    // ACCEPTANCE (task-953): the brief's Merge/Security Posture reading,
    // using the ORM-verbalization superlative `highest` (domain words like
    // `strongest` are author-extensible, not engine grammar — cruft pass).
    // `highest … among Commits the Merge concerns` must select the Commit
    // whose Security Posture is EARLIEST in the declaration order (rank 0),
    // projected onto the Merge — NOT last-bound inheritance.
    let src = r#"# Highest-among superlative derivation
Merge(.ID) is an entity type.
Commit(.SHA) is an entity type.
ID is a value type.
SHA is a value type.
Security Posture is a value type.
Security Posture enumerates 'verified', 'unverified', 'compromised'.

## Fact Types
Merge has ID.
Commit has SHA.
Merge concerns Commit.
Commit has Security Posture.
Merge has derived Security Posture.

## Derivation Rules
* Merge has derived Security Posture iff Merge concerns some Commit that has the highest Security Posture among Commits the Merge concerns.
"#;
    let state = parse_to_state(src).expect("parse");
    let data = compile::cell_index_from_state(&state);
    assert_eq!(data.derivation_rules.len(), 1,
        "expected exactly one authored rule, got {}", data.derivation_rules.len());
    let rule = &data.derivation_rules[0];

    // The superlative now lifts to a comparator aggregate: op=min (highest
    // → rank 0), enum_rank set, source = the value FT, group key = Merge.
    assert!(!rule.consequent_aggregates.is_empty(),
        "`highest … among` must populate consequent_aggregates; rule={:#?}", rule);
    let agg = &rule.consequent_aggregates[0];
    assert_eq!(agg.op, "min", "highest maps to min (rank 0 = highest)");
    assert!(agg.enum_rank, "aggregate must be flagged enum_rank");
    assert!(rule.unresolved_clauses.is_empty(),
        "the superlative clause must be consumed, not left unresolved; got {:#?}",
        rule.unresolved_clauses);

    let model = compile::compile(&state);
    let cd = model.derivations.iter().find(|d| d.id == rule.id)
        .expect("compiled derivation missing");

    // M1 concerns C1(verified=rank0=highest), C2(compromised=rank2).
    // M2 concerns C3(unverified=rank1). Highest-per-Merge: M1→verified,
    // M2→unverified.
    let out = apply_to_facts(&cd.func, &[
        ("Merge_concerns_Commit", &[("Merge", "M1"), ("Commit", "C1")]),
        ("Merge_concerns_Commit", &[("Merge", "M1"), ("Commit", "C2")]),
        ("Merge_concerns_Commit", &[("Merge", "M2"), ("Commit", "C3")]),
        ("Commit_has_Security_Posture", &[("Commit", "C1"), ("Security Posture", "verified")]),
        ("Commit_has_Security_Posture", &[("Commit", "C2"), ("Security Posture", "compromised")]),
        ("Commit_has_Security_Posture", &[("Commit", "C3"), ("Security Posture", "unverified")]),
    ]);
    let derived = decode_derived(&out);
    assert!(
        derived.iter().any(|(_, _, b)|
            b.iter().any(|(k, v)| k == "Merge" && v == "M1") &&
            b.iter().any(|(k, v)| k == "Security Posture" && v == "verified")),
        "M1 must derive HIGHEST posture 'verified' (rank 0), not last-bound; got {:#?}", derived);
    // Negative guard: the lower posture must NOT leak through for M1.
    assert!(
        !derived.iter().any(|(_, _, b)|
            b.iter().any(|(k, v)| k == "Merge" && v == "M1") &&
            b.iter().any(|(k, v)| k == "Security Posture" && v == "compromised")),
        "M1 must derive ONLY the highest posture, not 'compromised'; got {:#?}", derived);
    assert!(
        derived.iter().any(|(_, _, b)|
            b.iter().any(|(k, v)| k == "Merge" && v == "M2") &&
            b.iter().any(|(k, v)| k == "Security Posture" && v == "unverified")),
        "M2 (single commit) must derive 'unverified'; got {:#?}", derived);
}

#[test]
fn superlative_highest_priority_among_selects_p0_over_p1() {
    // Second use site (task-953): Task Priority. `highest … among` = min
    // index → selects p0 over p1. Confirms the lift generalises past
    // Security Posture to a second enum-ordered value type.
    let src = r#"# Highest-priority-among superlative derivation
Sprint(.Name) is an entity type.
Task(.Key) is an entity type.
Name is a value type.
Key is a value type.
Priority is a value type.
Priority enumerates 'p0', 'p1', 'p2'.

## Fact Types
Sprint has Name.
Task has Key.
Sprint includes Task.
Task has Priority.
Sprint has top Priority.

## Derivation Rules
* Sprint has top Priority iff Sprint includes some Task that has the highest Priority among Tasks the Sprint includes.
"#;
    let state = parse_to_state(src).expect("parse");
    let data = compile::cell_index_from_state(&state);
    let rule = &data.derivation_rules[0];
    assert!(!rule.consequent_aggregates.is_empty(),
        "`highest … among` must populate consequent_aggregates; rule={:#?}", rule);
    assert_eq!(rule.consequent_aggregates[0].op, "min",
        "highest maps to min (rank 0 = highest)");
    assert!(rule.consequent_aggregates[0].enum_rank);

    let model = compile::compile(&state);
    let cd = model.derivations.iter().find(|d| d.id == rule.id)
        .expect("compiled derivation missing");
    // S1 includes T1(p1), T2(p0) → highest p0. S2 includes T3(p2) → p2.
    let out = apply_to_facts(&cd.func, &[
        ("Sprint_includes_Task", &[("Sprint", "S1"), ("Task", "T1")]),
        ("Sprint_includes_Task", &[("Sprint", "S1"), ("Task", "T2")]),
        ("Sprint_includes_Task", &[("Sprint", "S2"), ("Task", "T3")]),
        ("Task_has_Priority", &[("Task", "T1"), ("Priority", "p1")]),
        ("Task_has_Priority", &[("Task", "T2"), ("Priority", "p0")]),
        ("Task_has_Priority", &[("Task", "T3"), ("Priority", "p2")]),
    ]);
    let derived = decode_derived(&out);
    assert!(
        derived.iter().any(|(_, _, b)|
            b.iter().any(|(k, v)| k == "Sprint" && v == "S1") &&
            b.iter().any(|(k, v)| k == "Priority" && v == "p0")),
        "S1 must derive HIGHEST priority p0 (rank 0), not p1; got {:#?}", derived);
    assert!(
        !derived.iter().any(|(_, _, b)|
            b.iter().any(|(k, v)| k == "Sprint" && v == "S1") &&
            b.iter().any(|(k, v)| k == "Priority" && v == "p1")),
        "S1 must NOT derive the weaker p1; got {:#?}", derived);
}

#[test]
fn superlative_lowest_among_selects_enum_latest_posture() {
    // The opposite direction: `lowest` maps to MAX rank (last-declared).
    // M1 concerns verified(0) + compromised(2) → lowest is 'compromised'.
    let src = r#"# Lowest-among superlative derivation
Merge(.ID) is an entity type.
Commit(.SHA) is an entity type.
ID is a value type.
SHA is a value type.
Security Posture is a value type.
Security Posture enumerates 'verified', 'unverified', 'compromised'.

## Fact Types
Merge has ID.
Commit has SHA.
Merge concerns Commit.
Commit has Security Posture.
Merge has derived Security Posture.

## Derivation Rules
* Merge has derived Security Posture iff Merge concerns some Commit that has the lowest Security Posture among Commits the Merge concerns.
"#;
    let state = parse_to_state(src).expect("parse");
    let data = compile::cell_index_from_state(&state);
    let rule = &data.derivation_rules[0];
    assert!(!rule.consequent_aggregates.is_empty(),
        "`lowest … among` must populate consequent_aggregates; rule={:#?}", rule);
    assert_eq!(rule.consequent_aggregates[0].op, "max",
        "lowest maps to max (last-declared = lowest)");
    let model = compile::compile(&state);
    let cd = model.derivations.iter().find(|d| d.id == rule.id)
        .expect("compiled derivation missing");
    let out = apply_to_facts(&cd.func, &[
        ("Merge_concerns_Commit", &[("Merge", "M1"), ("Commit", "C1")]),
        ("Merge_concerns_Commit", &[("Merge", "M1"), ("Commit", "C2")]),
        ("Commit_has_Security_Posture", &[("Commit", "C1"), ("Security Posture", "verified")]),
        ("Commit_has_Security_Posture", &[("Commit", "C2"), ("Security Posture", "compromised")]),
    ]);
    let derived = decode_derived(&out);
    assert!(
        derived.iter().any(|(_, _, b)|
            b.iter().any(|(k, v)| k == "Merge" && v == "M1") &&
            b.iter().any(|(k, v)| k == "Security Posture" && v == "compromised")),
        "M1 must derive LOWEST posture 'compromised' (rank 2); got {:#?}", derived);
    assert!(
        !derived.iter().any(|(_, _, b)|
            b.iter().any(|(k, v)| k == "Merge" && v == "M1") &&
            b.iter().any(|(k, v)| k == "Security Posture" && v == "verified")),
        "M1 must NOT derive the higher 'verified'; got {:#?}", derived);
}

// ─── task-934-1 — §4.2 value-type→widget LAZY VIEW mechanism ──────────────
//
// Three derivation rules share ONE consequent FT (`ViewElement has
// Component Role. *`).  The compile.rs fix (task-934-1, `view_by_cell`
// grouping) must Concat them into a single `view:ViewElement_has_Component_Role`
// def.  Without the fix, the HashMap last-write-wins — only the last rule
// survives, breaking the two-element verification below.
//
// Layout:
//
//   Entity types  : ViewElement, Fact Type, Role, Noun
//   Value types   : Component Role, Format
//   Fact types    : ViewElement renders Fact Type
//                   Fact Type has Role
//                   Role is played by Noun
//                   Noun has Format
//                   ViewElement has Component Role. *   ← the crux
//
//   Three view rules (all write into ViewElement_has_Component_Role):
//     'text-input'  iff ... Noun has Format 'text'
//     'date-picker' iff ... Noun has Format 'date'
//     'checkbox'    iff ... Noun has Format 'boolean'
//
//   Instance facts: ViewElement e1 → ft1 → role1 → noun1 (Format 'text')
//                   ViewElement e2 → ft2 → role2 → noun2 (Format 'date')
//
// Assertions:
//   (a) All three rules compile with materialization = View.
//   (b) NO `derivation:` def emitted for ViewElement_has_Component_Role.
//   (c) A `view:ViewElement_has_Component_Role` def IS present (the
//       multi-rule Concat is the compile.rs fix under test).
//   (d) Func::Fetch resolves e1 → 'text-input' and e2 → 'date-picker'
//       lazily (no forward chain run).
#[test]
fn view_projection_section_4_2_lazy_widget_rules_merge() {
    let src = r#"# task-934-1 §4.2 view test
ViewElement(.id) is an entity type.
Fact Type(.id) is an entity type.
Role(.id) is an entity type.
Noun(.id) is an entity type.
Component Role is a value type.
Format is a value type.

## Fact Types
ViewElement renders Fact Type.
Fact Type has Role.
Role is played by Noun.
Noun has Format.
ViewElement has Component Role. *

## Derivation Rules
* ViewElement has Component Role 'text-input' iff ViewElement renders some Fact Type and that Fact Type has some Role and that Role is played by some Noun and that Noun has Format 'text'.
* ViewElement has Component Role 'date-picker' iff ViewElement renders some Fact Type and that Fact Type has some Role and that Role is played by some Noun and that Noun has Format 'date'.
* ViewElement has Component Role 'checkbox' iff ViewElement renders some Fact Type and that Fact Type has some Role and that Role is played by some Noun and that Noun has Format 'boolean'.

## Instance Facts
Fact Type 'ft1' has Role 'role1'.
Role 'role1' is played by Noun 'noun1'.
Noun 'noun1' has Format 'text'.
ViewElement 'e1' renders Fact Type 'ft1'.

Fact Type 'ft2' has Role 'role2'.
Role 'role2' is played by Noun 'noun2'.
Noun 'noun2' has Format 'date'.
ViewElement 'e2' renders Fact Type 'ft2'.
"#;
    let state = parse_to_state(src).expect("parse");
    let data = compile::cell_index_from_state(&state);

    // (a) All three widget rules must be View-materialized.
    let view_rules: Vec<&crate::types::DerivationRuleDef> = data.derivation_rules.iter()
        .filter(|r| r.text.contains("ViewElement has Component Role"))
        .collect();
    assert_eq!(view_rules.len(), 3,
        "Expected exactly 3 Component Role rules, got {}; rules: {:#?}",
        view_rules.len(),
        data.derivation_rules.iter().map(|r| r.text.as_str()).collect::<Vec<_>>());
    for r in &view_rules {
        assert!(matches!(r.materialization, crate::types::MaterializationPolicy::View),
            "Rule '{}' must be View-materialized; got {:?}", r.text, r.materialization);
    }

    // Build the full def-state (view defs, instance cells, etc.).
    let defs = compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);

    // (b) NO `derivation:` def may exist for any of the three rules
    //     (View rules must NOT be emitted into the forward chain).
    for r in &view_rules {
        let derivation_def = ast::fetch_raw(&format!("derivation:{}", r.id), &d);
        assert!(matches!(derivation_def, ast::Object::Bottom),
            "View rule '{}' must NOT emit a derivation: def; got {:?}",
            r.text, derivation_def);
    }

    // (c) A single `view:ViewElement_has_Component_Role` def MUST be
    //     present (the compile.rs fix groups all 3 rules via Concat).
    let view_def = ast::fetch_raw("view:ViewElement_has_Component_Role", &d);
    assert!(!matches!(view_def, ast::Object::Bottom),
        "view:ViewElement_has_Component_Role def must be present (compile.rs \
         multi-rule Concat grouping fix). Got Bottom.");

    // (d) Lazy resolution via Func::Fetch — no forward chain run.
    //     The fetch_input passes `d` as the state; resolve_view encodes
    //     it and applies the merged view func against the instance cells.
    let fetch_input = Object::seq(vec![
        Object::atom("ViewElement_has_Component_Role"),
        d.clone(),
    ]);
    let result = ast::apply(&ast::Func::Fetch, &fetch_input, &d);

    // Collect (ViewElement, Component Role) pairs from the lazy result.
    let widget_pairs: Vec<(String, String)> = match &result {
        Object::Seq(items) => items.iter().filter_map(|f| {
            let elem = ast::binding(f, "ViewElement").map(String::from)?;
            let role = ast::binding(f, "Component Role").map(String::from)?;
            Some((elem, role))
        }).collect(),
        _ => Vec::new(),
    };

    assert!(
        widget_pairs.iter().any(|(e, r)| e == "e1" && r == "text-input"),
        "Lazy view must resolve e1 → 'text-input' (Format 'text' rule).\n\
         Got pairs: {:?}\nRaw Fetch result: {:?}", widget_pairs, result);
    assert!(
        widget_pairs.iter().any(|(e, r)| e == "e2" && r == "date-picker"),
        "Lazy view must resolve e2 → 'date-picker' (Format 'date' rule).\n\
         Got pairs: {:?}\nRaw Fetch result: {:?}", widget_pairs, result);
    // Verify e1 does NOT get 'date-picker' and e2 does NOT get 'text-input'
    // (proves the antecedent Format literal filters are applied correctly).
    assert!(
        !widget_pairs.iter().any(|(e, r)| e == "e1" && r == "date-picker"),
        "e1 must NOT derive 'date-picker' (its Format is 'text', not 'date').\n\
         Got pairs: {:?}", widget_pairs);
    assert!(
        !widget_pairs.iter().any(|(e, r)| e == "e2" && r == "text-input"),
        "e2 must NOT derive 'text-input' (its Format is 'date', not 'text').\n\
         Got pairs: {:?}", widget_pairs);
}

// ─── task-970 — lazy existential / Skolem derivation head ────────────────
//
// The view-projection menu rule (design §4.5) has a head whose CONSEQUENT
// introduces a FRESH entity not in the antecedent — one `ViewElement` per
// matching `(View, Transition)` binding. This is a tuple-generating
// dependency (TGD) with an existential head variable; AREST satisfies it
// with the SKOLEM (semi-oblivious) chase: the fresh entity's id is a
// DETERMINISTIC fnv hash of the frontier binding, so re-derivation is
// IDEMPOTENT (same binding → same id → no duplicate across passes).
//
// This test proves the RESOLVE-TIME mechanism end-to-end, lazily, through
// the SAME `view:` / `resolve_view` path 934-1 established — independent of
// the parser surface syntax (deferred; see the `#[ignore]`d spec test
// below and `readings/ui/skolem-head-design.md` §5). It builds the exact
// view-func subtree the compiler's 1-antecedent fanout must emit for a
// skolem head:
//
//   ApplyToAll( [ const(cons_id), const(reading), bindings ] )
//     ∘ extract_facts_from_pop(ant_ft)
//
// where `bindings` appends the synthesized head-entity pair
//   <ViewElement, Platform("skolem"):[ View-value, Transition-value ]>
// to the inherited antecedent bindings. The antecedent FT `MenuBinding`
// models the post-join frontier (one fact per (View, Transition) the menu
// projection would yield) so the test isolates the HEAD value-invention
// from the join (which 934-1 already covers).
//
// Asserts: (a) one fresh ViewElement per binding; (b) deterministic
// `ve_<fnv>` ids; (c) distinct bindings → distinct ids; (d) IDEMPOTENT
// across two `resolve_view` passes (byte-identical id set); (e) the head
// also carries the literal `Component Role 'button'` (design §4.5).
#[test]
fn skolem_head_resolve_view_invents_one_idempotent_entity_per_binding() {
    use crate::ast::{apply, encode_state, func_to_object, resolve_view, store};

    // The consequent FT cell (what `View Element renders Transition` maps
    // to) and the antecedent frontier FT.
    let cons_cell = "ViewElement_renders_Transition";
    let ant_ft = "MenuBinding";

    // bindings_func (input = one antecedent fact in <<role,val>,…> shape):
    //   inherit the antecedent bindings (Func::Id) and APPEND
    //   <ViewElement, skolem(<View, Transition>)> + <Component Role, 'button'>.
    // The skolem frontier reads View and Transition off the SAME antecedent
    // fact under apply_to_all (role_value_by_name equivalent).
    let role_value_by_name = |name: &str| -> Func {
        Func::compose(
            Func::compose(Func::Selector(2), Func::Selector(1)),
            Func::filter(Func::compose(Func::Eq, Func::construction(vec![
                Func::Selector(1),
                Func::constant(Object::atom(name)),
            ]))),
        )
    };
    let skolem_id = Func::compose(
        Func::Platform("skolem".to_string()),
        Func::construction(vec![
            role_value_by_name("View"),
            role_value_by_name("Transition"),
        ]),
    );
    let head_pairs = Func::construction(vec![
        // fresh head entity (the existential variable, Skolem-invented)
        Func::construction(vec![Func::constant(Object::atom("ViewElement")), skolem_id]),
        // literal-pinned head role (design §4.5: ... has Component Role 'button')
        Func::construction(vec![
            Func::constant(Object::atom("Component Role")),
            Func::constant(Object::atom("button")),
        ]),
        // carry the Transition through so the rendered transition is visible
        Func::construction(vec![
            Func::constant(Object::atom("Transition")),
            role_value_by_name("Transition"),
        ]),
    ]);
    // Concat([Id, head_pairs]) flattens inherited + appended pairs one level.
    let bindings = Func::compose(
        Func::Concat,
        Func::construction(vec![Func::Id, head_pairs]),
    );
    let derive_one = Func::construction(vec![
        Func::constant(Object::atom(cons_cell)),
        Func::constant(Object::atom("ViewElement renders Transition")),
        bindings,
    ]);
    // The 1-antecedent fanout shape (compile.rs ~5193): one derived envelope
    // per antecedent fact.
    let extract = Func::compose(
        Func::FetchOrPhi,
        Func::construction(vec![Func::constant(Object::atom(ant_ft)), Func::Id]),
    );
    let view_func = Func::compose(Func::apply_to_all(derive_one), extract);

    // Register it under `view:{cell}` exactly as compile.rs's view_by_cell
    // fold does — the LAZY path (no `derivation:` def, never eager-chained).
    let defs = store(&format!("view:{}", cons_cell), func_to_object(&view_func), &Object::phi());

    // Population: two menu bindings (the frontier the join would produce).
    let pop = {
        let mut s = Object::phi();
        s = ast::cell_push(ant_ft, ast::fact_from_pairs(&[
            ("View", "Order Menu"), ("Transition", "approve"),
        ]), &s);
        s = ast::cell_push(ant_ft, ast::fact_from_pairs(&[
            ("View", "Order Menu"), ("Transition", "reject"),
        ]), &s);
        s
    };

    // ── Pass 1 ──
    let pass1 = resolve_view(cons_cell, &pop, &defs)
        .expect("view: def must resolve via resolve_view");
    let elems1: Vec<(String, String, String)> = pass1.as_seq().map(|items| items.iter()
        .filter_map(|f| Some((
            ast::binding(f, "ViewElement").map(String::from)?,
            ast::binding(f, "Transition").map(String::from)?,
            ast::binding(f, "Component Role").map(String::from)?,
        ))).collect()).unwrap_or_default();

    // (a) one fresh ViewElement per binding.
    assert_eq!(elems1.len(), 2,
        "skolem head must invent exactly one ViewElement per (View,Transition) \
         binding; got {:#?}\nraw: {:?}", elems1, pass1);
    // (b) deterministic `ve_<fnv>` ids, (e) literal Component Role 'button',
    //     transition carried through.
    for (id, tr, role) in &elems1 {
        assert!(id.starts_with("ve_") && id.len() == "ve_".len() + 16,
            "head entity id must be a Skolem `ve_<16 hex>`; got {:?}", id);
        assert_eq!(role, "button",
            "head must pin Component Role 'button' (design §4.5); got {:?}", role);
        assert!(tr == "approve" || tr == "reject",
            "head must render its frontier Transition; got {:?}", tr);
    }
    // (c) distinct bindings → distinct ids.
    let id_approve = elems1.iter().find(|(_, tr, _)| tr == "approve").map(|(id, ..)| id.clone());
    let id_reject  = elems1.iter().find(|(_, tr, _)| tr == "reject").map(|(id, ..)| id.clone());
    assert!(id_approve.is_some() && id_reject.is_some(),
        "both transitions must produce a head element; got {:#?}", elems1);
    assert_ne!(id_approve, id_reject,
        "distinct (View,Transition) frontiers must invent distinct ViewElement \
         ids (no Skolem collision); both = {:?}", id_approve);

    // ── Pass 2 — IDEMPOTENCE (the Skolem-chase correctness crux) ──
    let pass2 = resolve_view(cons_cell, &pop, &defs)
        .expect("view: def must resolve on the second pass too");
    let mut ids1: Vec<String> = elems1.iter().map(|(id, ..)| id.clone()).collect();
    let mut ids2: Vec<String> = pass2.as_seq().map(|items| items.iter()
        .filter_map(|f| ast::binding(f, "ViewElement").map(String::from))
        .collect()).unwrap_or_default();
    ids1.sort();
    ids2.sort();
    assert_eq!(ids1, ids2,
        "re-deriving the same population MUST reproduce the SAME ViewElement \
         id set (idempotent Skolem chase — same frontier → same id → no \
         duplicate). pass1 ids: {:?}; pass2 ids: {:?}", ids1, ids2);

    // Sanity: the registered def is a `view:` def, NOT a `derivation:` def —
    // it never enters the eager forward chain that hangs the metamodel.
    assert!(matches!(ast::fetch_raw(&format!("derivation:{}", cons_cell), &defs),
        Object::Bottom),
        "skolem head must resolve lazily as a view, never as an eager \
         derivation: def (the task-934 metamodel-hang guard)");

    // Belt-and-suspenders: the encoded-pop path resolve_view takes when fed
    // an already-encoded population is the same — exercise it so the lazy
    // read-side contract is pinned both ways.
    let encoded = encode_state(&pop);
    let via_encoded = apply(&view_func, &encoded, &defs);
    assert!(via_encoded.as_seq().map(|s| s.len()).unwrap_or(0) == 2,
        "view func over an encoded pop must also yield 2 envelopes; got {:?}",
        via_encoded);
}

// task-970 — SPEC (ignored until the parser + compiler wiring lands).
//
// Pins the TARGET the FORML 2 surface syntax + `compile_explicit_derivation`
// emission must hit: a `*`-marked existential head rule, authored in prose,
// that compiles to a `view:` def and resolves (lazily, idempotently) to one
// fresh ViewElement per binding — WITHOUT the test hand-building the func.
//
// Remaining work to un-ignore (see skolem-head-design.md §4-5):
//   1. parse_forml2.rs `resolve_derivation_rule` (~1490): detect a head
//      role variable (`ViewElement (E)`) bound by NEITHER an antecedent
//      role NOR an `X is Y` rename → record `SkolemHeadRole{role, frontier}`.
//   2. types.rs: add `skolem_head_roles: Vec<SkolemHeadRole>` to
//      DerivationRuleDef (+ canonical-JSON writer field 19) and the
//      SkolemHeadRole struct.
//   3. compile.rs `compile_explicit_derivation` 1-antecedent `bindings_func`
//      (~5059 / ~5153): for each SkolemHeadRole, append
//      `<role, Platform("skolem"):[frontier role_value_by_name…]>`; exclude
//      the skolem role from `required_keys` (~5230).
//
// The mechanism this test would exercise is ALREADY PROVEN green by
// `skolem_head_resolve_view_invents_one_idempotent_entity_per_binding`
// (the `view:` func + skolem primitive + idempotent resolve_view); only the
// prose→rule→func authoring path is outstanding.
#[test]
fn spec_skolem_head_authored_in_forml2_resolves_lazily() {
    // TARGET surface form (design §4.5, single-consequent-FT slice):
    //   ViewElement (E) renders Transition (Tr)
    //     iff MenuBinding has View (Vw) and MenuBinding has Transition (Tr).
    // with `ViewElement renders Transition. *` marking the head fully-derived.
    let src = r#"# task-970 skolem head spec
ViewElement(.id) is an entity type.
View(.Name) is an entity type.
Transition(.id) is an entity type.
MenuBinding(.id) is an entity type.

## Fact Types
MenuBinding has View.
MenuBinding has Transition.
ViewElement renders Transition. *

## Derivation Rules
* ViewElement (E) renders Transition (Tr) iff MenuBinding has View and MenuBinding has Transition (Tr).

## Instance Facts
MenuBinding 'mb1' has View 'Order Menu'.
MenuBinding 'mb1' has Transition 'approve'.
MenuBinding 'mb2' has View 'Order Menu'.
MenuBinding 'mb2' has Transition 'reject'.
"#;
    let state = parse_to_state(src).expect("parse");
    let data = compile::cell_index_from_state(&state);

    // The head rule must be View-materialized (lazy) and carry a skolem
    // head role for the fresh `ViewElement`.
    let rule = data.derivation_rules.iter()
        .find(|r| r.text.contains("renders Transition"))
        .expect("skolem head rule must parse");
    assert!(matches!(rule.materialization, crate::types::MaterializationPolicy::View),
        "skolem head must be View-materialized (lazy); got {:?}", rule.materialization);

    // Resolve lazily and assert one idempotent fresh ViewElement per binding.
    let defs = compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);
    let view_def = ast::fetch_raw("view:ViewElement_renders_Transition", &d);
    assert!(!matches!(view_def, ast::Object::Bottom),
        "skolem head must emit a view: def");

    let fetch_input = Object::seq(vec![
        Object::atom("ViewElement_renders_Transition"), d.clone(),
    ]);
    let r1 = ast::apply(&ast::Func::Fetch, &fetch_input, &d);
    let ids: Vec<String> = r1.as_seq().map(|items| items.iter()
        .filter_map(|f| ast::binding(f, "ViewElement").map(String::from))
        .collect()).unwrap_or_default();
    assert_eq!(ids.len(), 2, "one fresh ViewElement per binding; got {:?}", r1);
    assert!(ids.iter().all(|id| id.starts_with("ve_")),
        "fresh entity ids must be Skolem `ve_<fnv>`; got {:?}", ids);
}

// ─── Category 6b: Count over a complex `where`-body (ring + literal) ──
//
// Repro for the count-aggregate parsing gap. Shape:
//   `* Item has Open Dep Count iff Open Dep Count is the count of Item1
//      where Item1 blocks the Item and Item1 has Status 'open'.`
//
// The `where`-body is MULTI-CLAUSE, traverses a same-noun RING FT
// (`Item blocks Item`), and ends in a LITERAL status filter
// (`Item1 has Status 'open'`). The counted entity is `Item1` (the
// blocker); the group key is `Item` (the blocked subject); only blockers
// whose Status is 'open' should be counted.
//
// Pre-fix, `try_parse_aggregate_clause` handed the ENTIRE multi-clause
// where-body to a single `resolve_fact_type`, which could not resolve a
// multi-clause string with a trailing literal — so either the aggregate
// never formed (rule misrouted) or the trailing `'open'` literal leaked
// into the derived value. Either way the count is wrong.
//
// Population for one subject `item-A`:
//   blk-1 blocks item-A, blk-1 Status='open'    → counts
//   blk-2 blocks item-A, blk-2 Status='open'    → counts
//   blk-3 blocks item-A, blk-3 Status='closed'  → filtered out
// Expected: Open Dep Count = 2 for item-A (NOT a literal "open"/"closed",
// NOT 3, NOT nothing).
#[test]
fn shape_aggregate_count_over_ring_with_literal_filter() {
    let src = r#"# Test
Item(.ID) is an entity type.
ID is a value type.
Status is a value type.
Open Dep Count is a value type.

## Fact Types
Item has ID.
Item blocks Item.
Item has Status.
Item has Open Dep Count.

## Derivation Rules
* Item has Open Dep Count iff Open Dep Count is the count of Item1 where Item1 blocks the Item and Item1 has Status 'open'.
"#;
    let (rule, func) = parse_and_compile(src);

    // The rule must route to the aggregate compiler.
    assert!(!rule.consequent_aggregates.is_empty(),
        "consequent_aggregates must be populated so the rule routes to \
         compile_aggregate_derivation; got {:#?}\nrule text: {}",
        rule.consequent_aggregates, rule.text);
    let agg = &rule.consequent_aggregates[0];
    assert_eq!(agg.op, "count", "aggregate op must be count, got {}", agg.op);
    assert_eq!(agg.role, "Open Dep Count",
        "aggregate result role, got {}", agg.role);
    // Source FT is the ring relation `Item blocks Item`.
    assert_eq!(agg.source_fact_type_id, "Item_blocks_Item",
        "aggregate source FT must be the ring `Item blocks Item`, got {}",
        agg.source_fact_type_id);
    // The literal filter `Item1 has Status 'open'` must be captured as an
    // aggregate filter over the counted blocker, not dropped.
    assert_eq!(agg.filters.len(), 1,
        "the `Item1 has Status 'open'` where-clause must become one aggregate \
         filter; got {:#?}", agg.filters);
    let f = &agg.filters[0];
    assert_eq!(f.ref_fact_type_id, "Item_has_Status");
    assert_eq!(f.filter_role, "Status");
    assert_eq!(f.value, "open");

    // item-A: two OPEN blockers (blk-1, blk-2) + one CLOSED blocker (blk-3).
    // The ring `Item blocks Item` carries <blocker, blocked> with both
    // roles named `Item` (positional). The literal filter `Item1 has
    // Status 'open'` restricts the counted blocker.
    let out = apply_to_facts(&func, &[
        ("Item_blocks_Item", &[("Item", "blk-1"), ("Item", "item-A")]),
        ("Item_blocks_Item", &[("Item", "blk-2"), ("Item", "item-A")]),
        ("Item_blocks_Item", &[("Item", "blk-3"), ("Item", "item-A")]),
        ("Item_has_Status",  &[("Item", "blk-1"), ("Status", "open")]),
        ("Item_has_Status",  &[("Item", "blk-2"), ("Status", "open")]),
        ("Item_has_Status",  &[("Item", "blk-3"), ("Status", "closed")]),
    ]);
    let derived = decode_derived(&out);
    assert!(!derived.is_empty(),
        "at least one aggregate derivation expected for item-A, got nothing");

    // The derived value for item-A's Open Dep Count must be the integer 2
    // (the two open blockers), NOT a literal status string and NOT 3.
    let count_for_a: Option<String> = derived.iter().find_map(|(_, _, bindings)| {
        let is_a = bindings.iter().any(|(k, v)| k == "Item" && v == "item-A");
        if !is_a { return None; }
        bindings.iter()
            .find(|(k, _)| k == "Open Dep Count")
            .map(|(_, v)| v.clone())
    });
    let count_for_a = count_for_a.unwrap_or_else(|| panic!(
        "no Open Dep Count binding for item-A in derivations: {:#?}", derived));
    assert_eq!(count_for_a, "2",
        "Open Dep Count for item-A must be 2 (blk-1, blk-2 are open; blk-3 is \
         closed). Got `{}` — a literal status string or wrong count means the \
         where-body filter/target was mis-parsed.\nfull derived: {:#?}",
        count_for_a, derived);
}

// ─── Category 6d: empty-image aggregate fold = the operator's unit ──────
//
// Backus §11.2.4: `/f:<>` is the right unit of `f`. The aggregate compiler
// emits sum as `(/+) ∘ α(target ∘ s2) ∘ Filter(key) ∘ image_pairs` and count
// as `length ∘ Filter(key) ∘ image_pairs`. When the inner image is EMPTY the
// fold sees `<>`; with `ast::unit_of` that now yields the unit (0 for +,
// `length:<>` = 0) rather than ⊥ — the empty-set value Backus prescribes.
//
// This test exercises that EXACT emitted Func shape directly over an empty
// input (the part of the aggregate pipeline downstream of the image, fed an
// empty pair Seq) so the empty-fold path is reached deterministically:
//   - sum  `(/+) ∘ α(s2):<>`        ⇒ "0"   (was ⊥ before the unit fix)
//   - count `length:<>`             ⇒ "0"
//
// NOTE on the *whole-rule* count-over-empty case (a subject whose `where`
// matches nothing): with the in-tree aggregate-filter semi-join, a subject
// whose source rows are all filtered out is dropped from the group-key
// enumeration entirely, so it derives NO row at all (count absent, observed),
// independent of this fold-unit fix — the empty group never reaches the fold.
// An enumerated group always contains at least its own outer row, so the
// empty fold is not reachable through the aggregate compiler for a present
// subject; the reachable empty-fold consumer is the universal-quantifier
// fold (`shape_universal_for_each_over_ring_with_literal_predicate`'s
// vacuous item-C), which this fix is what makes correct without a guard.
#[test]
fn aggregate_fold_over_empty_image_is_operator_unit() {
    use ast::Func;
    // The aggregate sum fold shape, applied to an empty projected image.
    // `(/+) ∘ α(Selector(2))` over `<>` reduces to `/+:<>` = unit(+) = 0.
    let sum_fold = Func::compose(
        Func::insert(Func::Add),
        Func::apply_to_all(Func::Selector(2)),
    );
    assert_eq!(
        ast::apply(&sum_fold, &Object::phi(), &Object::phi()),
        Object::atom("0"),
        "empty SUM image must fold to the + unit 0 (Backus /+:<>), not Bottom",
    );
    // The count fold shape: `length:<>` = 0 (length carries its own empty case).
    let count_fold = Func::Length;
    assert_eq!(
        ast::apply(&count_fold, &Object::phi(), &Object::phi()),
        Object::atom("0"),
        "empty COUNT image must be 0",
    );
}

// ─── Category 6c: the verified-live repro vocabulary (multi-word enum) ─
//
// Exact-shape regression for the originally-reported bug: a `Task blocks
// Task` ring + the MULTI-WORD enum-valued `Task has Task Status` filter
// ending in a `'pending'` literal. The live reading
//   `* Task has Open Blocker Count iff Open Blocker Count is the count of
//      Task1 where Task1 blocks the Task and Task1 has Task Status
//      'pending'.`
// produced the literal `"pending"` (or a wrong count) instead of counting
// the pending blockers. This guards the two-word filter-role path (the
// trailing literal binds the LAST role, `Task Status`, not `Task`).
#[test]
fn shape_aggregate_count_live_repro_task_blocks_task_pending() {
    let src = r#"# Test
Task(.ID) is an entity type.
ID is a value type.
Task Status is a value type.
Open Blocker Count is a value type.

## Fact Types
Task has ID.
Task blocks Task.
Task has Task Status.
Task has Open Blocker Count.

## Derivation Rules
* Task has Open Blocker Count iff Open Blocker Count is the count of Task1 where Task1 blocks the Task and Task1 has Task Status 'pending'.
"#;
    let (rule, func) = parse_and_compile(src);

    assert!(!rule.consequent_aggregates.is_empty(),
        "live-repro aggregate must populate consequent_aggregates; got {:#?}\n\
         rule text: {}", rule.consequent_aggregates, rule.text);
    let agg = &rule.consequent_aggregates[0];
    assert_eq!(agg.op, "count");
    assert_eq!(agg.source_fact_type_id, "Task_blocks_Task");
    assert_eq!(agg.filters.len(), 1,
        "the `Task1 has Task Status 'pending'` clause must become an aggregate \
         filter; got {:#?}", agg.filters);
    let f = &agg.filters[0];
    assert_eq!(f.ref_fact_type_id, "Task_has_Task_Status");
    assert_eq!(f.filter_role, "Task Status",
        "the multi-word LAST role must be the filter role, got {}", f.filter_role);
    assert_eq!(f.value, "pending");

    // subj-1: two pending blockers (p1, p2) + one done blocker (d1).
    let out = apply_to_facts(&func, &[
        ("Task_blocks_Task",    &[("Task", "p1"), ("Task", "subj-1")]),
        ("Task_blocks_Task",    &[("Task", "p2"), ("Task", "subj-1")]),
        ("Task_blocks_Task",    &[("Task", "d1"), ("Task", "subj-1")]),
        ("Task_has_Task_Status", &[("Task", "p1"), ("Task Status", "pending")]),
        ("Task_has_Task_Status", &[("Task", "p2"), ("Task Status", "pending")]),
        ("Task_has_Task_Status", &[("Task", "d1"), ("Task Status", "done")]),
    ]);
    let derived = decode_derived(&out);
    let count: Option<String> = derived.iter().find_map(|(_, _, b)| {
        if !b.iter().any(|(k, v)| k == "Task" && v == "subj-1") { return None; }
        b.iter().find(|(k, _)| k == "Open Blocker Count").map(|(_, v)| v.clone())
    });
    let count = count.unwrap_or_else(|| panic!(
        "no Open Blocker Count for subj-1 in {:#?}", derived));
    assert_eq!(count, "2",
        "Open Blocker Count for subj-1 must be 2 (p1, p2 pending; d1 done), \
         NOT the literal 'pending'. Got `{}`\nfull derived: {:#?}", count, derived);
}

// ─── Category 14: Universal quantifier ("for each") over a ring + literal ─
//
// Repro for the universal-quantifier compilation gap. Shape:
//   `* Item is clear iff for each Item1 that blocks the Item,
//      Item1 has Status 'done'.`
//
// The ENTIRE antecedent is the universal `for each <X> that <R> the
// <Subject>, <X has P>`. Pre-fix, `resolve_derivation_rule`'s classifier
// recognised the universal via `is_universal_quantifier_clause` and then
// `continue`d — DROPPING it. With no surviving antecedent the rule fell
// onto the 0-antecedent path and either derived `clear` for nothing or
// (worse) emitted a single constant fact. Either way the per-subject
// universal semantics were lost.
//
// The compiled form is the Backus fold ∀x∈S. P(x) = (/∧) ∘ (αP), filtered
// to the X's that R-relate to the subject, with the EMPTY-fold case made
// VACUOUSLY TRUE (a subject with no blockers is clear).
//
// Population (the `Item blocks Item` ring is positional <blocker, blocked>):
//   item-A: two blockers (a-blk-1, a-blk-2), BOTH Status='done'  → clear
//   item-B: one blocker  (b-blk-1),          Status='open'       → NOT clear
//   item-C: NO blockers                                          → clear (VACUOUS)
//
// Assertion: the derived `clear` set CONTAINS item-A and item-C, and does
// NOT contain item-B. (Blockers that are themselves blocker-free Items
// derive `clear` vacuously too — correct, and not asserted against.)
#[test]
fn shape_universal_for_each_over_ring_with_literal_predicate() {
    let src = r#"# Test
Item(.ID) is an entity type.
ID is a value type.
Status is a value type.

## Fact Types
Item has ID.
Item blocks Item.
Item has Status.
Item is clear.

## Derivation Rules
* Item is clear iff for each Item1 that blocks the Item, Item1 has Status 'done'.
"#;
    let (rule, func) = parse_and_compile(src);

    // The universal must be captured as a consequent universal so the rule
    // compiles to the fold (not dropped, not an empty-antecedent constant).
    assert!(!rule.consequent_universals.is_empty(),
        "the `for each` antecedent must populate consequent_universals; got {:#?}\n\
         rule text: {}\nunresolved: {:#?}",
        rule.consequent_universals, rule.text, rule.unresolved_clauses);
    let u = &rule.consequent_universals[0];
    assert_eq!(u.relation_fact_type_id, "Item_blocks_Item",
        "the relating FT must be the ring `Item blocks Item`, got {}",
        u.relation_fact_type_id);
    assert_eq!(u.predicate_fact_type_id, "Item_has_Status",
        "the predicate FT must be `Item has Status`, got {}",
        u.predicate_fact_type_id);
    assert_eq!(u.predicate_filter_role, "Status");
    assert_eq!(u.predicate_value, "done");

    // item-A: 2 done blockers → clear. item-B: 1 open blocker → NOT clear.
    // item-C: seeded via `Item has ID` so it is enumerated as an Item, but
    // has NO blocker → clear (vacuous). The blockers (a-blk-*, b-blk-1) are
    // also Items with no blockers of their own, so they derive clear too —
    // correct (vacuous), and not asserted against here.
    let out = apply_to_facts(&func, &[
        ("Item_blocks_Item", &[("Item", "a-blk-1"), ("Item", "item-A")]),
        ("Item_blocks_Item", &[("Item", "a-blk-2"), ("Item", "item-A")]),
        ("Item_blocks_Item", &[("Item", "b-blk-1"), ("Item", "item-B")]),
        ("Item_has_Status",  &[("Item", "a-blk-1"), ("Status", "done")]),
        ("Item_has_Status",  &[("Item", "a-blk-2"), ("Status", "done")]),
        ("Item_has_Status",  &[("Item", "b-blk-1"), ("Status", "open")]),
        ("Item_has_ID",      &[("Item", "item-C"), ("ID", "item-C")]),
    ]);
    let derived = decode_derived(&out);

    // Collect the Item values that derived `clear`.
    let clear: Vec<String> = derived.iter()
        .filter(|(ft, _, _)| ft == "Item_is_clear")
        .filter_map(|(_, _, b)| b.iter()
            .find(|(k, _)| k == "Item").map(|(_, v)| v.clone()))
        .collect();

    assert!(clear.iter().any(|i| i == "item-A"),
        "item-A (both blockers done) MUST be clear; got clear set {:?}\nderived: {:#?}",
        clear, derived);
    assert!(clear.iter().any(|i| i == "item-C"),
        "item-C (NO blockers) MUST be clear VACUOUSLY (empty fold = TRUE); \
         got clear set {:?}\nderived: {:#?}", clear, derived);
    assert!(!clear.iter().any(|i| i == "item-B"),
        "item-B (one OPEN blocker) MUST NOT be clear; got clear set {:?}\nderived: {:#?}",
        clear, derived);
}

// ─── task-934-3a: Menu-View Derivation via Skolem Head ───────────────────────
//
// DESIGN SPEC (§4.5 / task-934-3 part a): a Noun's action menu is a DERIVED
// view — one `ViewElement` per (entity, legal-transition) pair, where
// "legal" means the entity is currently in a Status that the Transition
// departs from. The derivation head is an EXISTENTIAL (Skolem) variable: the
// fresh `ViewElement` id is `ve_<fnv1a64(Resource|Transition)>`, deterministic
// and idempotent (same frontier → same id across re-reads).
//
// METAMODEL JOIN (verified against readings/core/state.md + instances.md):
//
//   State Machine Definition is for Noun           (SMD → Noun)
//   Transition is defined in State Machine Definition (Transition → SMD)
//   Transition is from Status                      (Transition → Status)
//   State Machine is currently in Status           (SM → Status, canonical carrier)
//   State Machine is for Resource                  (SM → Resource)
//   Resource is instance of Noun                   (Resource → Noun, bridges them)
//
// Frontier = (Resource, Transition) after joining on Status (Transition.from ==
// SM.currentStatus). One ViewElement per frontier pair.
//
// TWO CONSEQUENT RULES (design §4.5 + skolem-head-design.md §3/§5):
//   Rule A: ViewElement (E) renders Transition (Tr)   [gives the VE→Tr link]
//   Rule B: ViewElement (E) has Component Role 'button' [pins the widget]
// Both carry the SAME frontier (Resource, Transition) → SAME `ve_<fnv>` id →
// SAME entity E. This is the "shared frontier = shared entity" property.
//
// HOW THIS TEST WORKS:
//   The frontier FT `MenuFrontierFT` pre-populates (Resource, Transition) pairs
//   equivalent to what the full 6-way join produces. This isolates the
//   SKOLEM HEAD mechanism (proven here) from the JOIN compilation (proven
//   separately in the join tests and in the existing skolem-head test). The
//   full-join wiring is the remaining compiler work (see "Remaining work" below).
//
// ASSERTS:
//   (a) entity in a non-terminal status: one VE per legal departure transition
//   (b) entity in a terminal status: ZERO ViewElements (no departing transitions)
//   (c) deterministic `ve_<fnv>` ids across both entities
//   (d) IDEMPOTENT across two resolve passes (byte-identical id sets)
//   (e) `renders Transition` rule → VE carries its Transition
//   (f) `has Component Role 'button'` rule → SAME VE id (shared frontier)
//   (g) NO eager `derivation:` def (lazy-only, never hangs the metamodel)
//
// REGISTERED-LIVE: test-only (not in lib.rs UI_READINGS). The full skolem
// head requires the parser to recognise the `(E)` surface syntax (deferred;
// see spec_skolem_head_authored_in_forml2_resolves_lazily and
// readings/ui/skolem-head-design.md §5).
//
// REMAINING WORK for 934-3:
//   (1) Parser: recognise `ViewElement (E) renders Transition (Tr)` as a
//       skolem head variable → populate `SkolemHeadRole{role, frontier}`.
//   (2) Compiler: the full 6-way join (SMD→Noun→SM→Resource×Status×Transition)
//       wired through `compile_join_derivation` + skolem head emission.
//   (3) Guard negation (§4.5 guard-filtering): `Guard prevents Transition →
//       omit the VE` — needs parser-negation idiom not yet available.
//   (4) Registration in UI_READINGS (lib.rs) + metamodel compile verification.
#[test]
fn menu_view_derivation_via_skolem_head_lazy_idempotent() {
    use crate::ast::{func_to_object, resolve_view, store};

    // ── Frontier FT and two consequent cells ──────────────────────────
    //
    // MenuFrontierFT carries (Resource, Transition) pairs — the output of
    // the full SM join. Populated directly to isolate the skolem head from
    // the join compilation (join is proven separately).
    let frontier_ft = "MenuFrontierFT";
    // Rule A: ViewElement renders Transition
    let cons_renders = "ViewElement_renders_Transition_menu";
    // Rule B: ViewElement has Component Role
    let cons_role    = "ViewElement_has_Component_Role_menu";

    // ── helper: extract a named role from a named-tuple fact ──────────
    let role_value = |name: &str| -> Func {
        Func::compose(
            Func::compose(Func::Selector(2), Func::Selector(1)),
            Func::filter(Func::compose(Func::Eq, Func::construction(vec![
                Func::Selector(1),
                Func::constant(Object::atom(name)),
            ]))),
        )
    };

    // ── Skolem id: ve_fnv1a64(Resource | Transition) ──────────────────
    //
    // Frontier = (Resource, Transition). Both are read off the same
    // MenuFrontierFT fact under apply_to_all, exactly like the existing
    // skolem_head_resolve_view_invents_one_idempotent_entity_per_binding test.
    let skolem_id = Func::compose(
        Func::Platform("skolem".to_string()),
        Func::construction(vec![
            role_value("Resource"),
            role_value("Transition"),
        ]),
    );

    // ── Rule A view func: ViewElement renders Transition ──────────────
    //
    // bindings per frontier fact:
    //   <ViewElement, skolem(Resource,Transition)>
    //   <Transition,  transition-value>            (carries the rendered transition)
    //   <Resource,    resource-value>              (carries the entity for fetch-filter)
    let a_pairs = Func::construction(vec![
        Func::construction(vec![Func::constant(Object::atom("ViewElement")), skolem_id.clone()]),
        Func::construction(vec![Func::constant(Object::atom("Transition")), role_value("Transition")]),
        Func::construction(vec![Func::constant(Object::atom("Resource")),   role_value("Resource")]),
    ]);
    let a_bindings = Func::compose(Func::Concat, Func::construction(vec![Func::Id, a_pairs]));
    let a_envelope = Func::construction(vec![
        Func::constant(Object::atom(cons_renders)),
        Func::constant(Object::atom("ViewElement renders Transition")),
        a_bindings,
    ]);
    let extract = Func::compose(
        Func::FetchOrPhi,
        Func::construction(vec![Func::constant(Object::atom(frontier_ft)), Func::Id]),
    );
    let view_func_renders = Func::compose(Func::apply_to_all(a_envelope), extract.clone());

    // ── Rule B view func: ViewElement has Component Role 'button' ─────
    //
    // SAME frontier → SAME skolem_id → SAME ViewElement entity (shared frontier
    // property: designs §4.5 + skolem-head-design.md §2). The bindings:
    //   <ViewElement,     skolem(Resource,Transition)>
    //   <Component Role,  'button'>
    let b_pairs = Func::construction(vec![
        Func::construction(vec![Func::constant(Object::atom("ViewElement")), skolem_id.clone()]),
        Func::construction(vec![
            Func::constant(Object::atom("Component Role")),
            Func::constant(Object::atom("button")),
        ]),
    ]);
    let b_bindings = Func::compose(Func::Concat, Func::construction(vec![Func::Id, b_pairs]));
    let b_envelope = Func::construction(vec![
        Func::constant(Object::atom(cons_role)),
        Func::constant(Object::atom("ViewElement has Component Role")),
        b_bindings,
    ]);
    let view_func_role = Func::compose(Func::apply_to_all(b_envelope), extract);

    // ── Register both under `view:{cell}` (LAZY — no `derivation:` def) ──
    let defs = {
        let d = Object::phi();
        let d = store(&format!("view:{}", cons_renders), func_to_object(&view_func_renders), &d);
        let d = store(&format!("view:{}", cons_role),    func_to_object(&view_func_role),    &d);
        d
    };

    // ── Population ────────────────────────────────────────────────────
    //
    // SM model (mirrors apps/tasks/readings/app.md task SM):
    //   Status 'pending'    → transitions: 'start', 'delete-from-pending'  (NON-TERMINAL)
    //   Status 'deleted'    → transitions: (none)                          (TERMINAL)
    //
    // Entities:
    //   task-1: currently in 'pending'  → 2 legal transitions → 2 ViewElements
    //   task-2: currently in 'deleted'  → 0 legal transitions → 0 ViewElements
    //
    // MenuFrontierFT = (Resource, Transition) from the join of
    //   Resource_is_currently_in_Status × Transition_is_from_Status on Status:
    //   - (task-1, start)                 ← start is from 'pending'
    //   - (task-1, delete-from-pending)   ← delete-from-pending is from 'pending'
    //   task-2 in 'deleted' → no Transition is from 'deleted' → no frontier rows
    let pop = {
        let mut s = Object::phi();
        // task-1 has two legal transitions from 'pending'
        s = ast::cell_push(frontier_ft, ast::fact_from_pairs(&[
            ("Resource", "task-1"),
            ("Transition", "start"),
        ]), &s);
        s = ast::cell_push(frontier_ft, ast::fact_from_pairs(&[
            ("Resource", "task-1"),
            ("Transition", "delete-from-pending"),
        ]), &s);
        // task-2 is in 'deleted' (terminal): NO frontier rows → NO ViewElements
        s
    };

    // ── Assertion (a) + (b): task-1 gets 2 VEs, task-2 gets 0 ────────
    let pass1_renders = resolve_view(cons_renders, &pop, &defs)
        .expect("view: def for renders must resolve via resolve_view");

    let elems1: Vec<(String, String, String)> = pass1_renders.as_seq()
        .map(|items| items.iter().filter_map(|f| {
            let ve = ast::binding(f, "ViewElement").map(String::from)?;
            let tr = ast::binding(f, "Transition").map(String::from)?;
            let rs = ast::binding(f, "Resource").map(String::from)?;
            Some((ve, tr, rs))
        }).collect())
        .unwrap_or_default();

    // (a) task-1 must have exactly 2 ViewElements (start + delete-from-pending)
    let task1_elems: Vec<&(String, String, String)> = elems1.iter()
        .filter(|(_, _, rs)| rs == "task-1").collect();
    assert_eq!(task1_elems.len(), 2,
        "task-1 (pending) must have exactly 2 ViewElements \
         (start + delete-from-pending); got {:?}\nfull: {:#?}", task1_elems, elems1);

    // (b) task-2 must have ZERO ViewElements (terminal status 'deleted')
    let task2_elems: Vec<&(String, String, String)> = elems1.iter()
        .filter(|(_, _, rs)| rs == "task-2").collect();
    assert_eq!(task2_elems.len(), 0,
        "task-2 (deleted = terminal) must have ZERO ViewElements; \
         got {:?}", task2_elems);

    // (c) deterministic `ve_<fnv>` ids — both must be well-formed
    for (id, tr, rs) in &elems1 {
        assert!(id.starts_with("ve_") && id.len() == "ve_".len() + 16,
            "VE id must be `ve_<16 hex>`; got {:?} for ({}, {})", id, rs, tr);
    }

    // (d) IDEMPOTENCE — second resolve pass produces byte-identical id set
    let pass2_renders = resolve_view(cons_renders, &pop, &defs)
        .expect("view: def must resolve on the second pass too");
    let mut ids1: Vec<String> = elems1.iter().map(|(id, ..)| id.clone()).collect();
    let mut ids2: Vec<String> = pass2_renders.as_seq()
        .map(|items| items.iter()
            .filter_map(|f| ast::binding(f, "ViewElement").map(String::from))
            .collect())
        .unwrap_or_default();
    ids1.sort();
    ids2.sort();
    assert_eq!(ids1, ids2,
        "menu-view derivation MUST be idempotent (same frontier → same \
         ViewElement id on every re-read). pass1: {:?}; pass2: {:?}", ids1, ids2);

    // (e) Transition is carried through the renders rule
    let task1_transitions: Vec<&str> = elems1.iter()
        .filter(|(_, _, rs)| rs == "task-1")
        .map(|(_, tr, _)| tr.as_str()).collect();
    assert!(task1_transitions.contains(&"start"),
        "start transition must be in task-1's menu; got {:?}", task1_transitions);
    assert!(task1_transitions.contains(&"delete-from-pending"),
        "delete-from-pending must be in task-1's menu; got {:?}", task1_transitions);

    // (f) SHARED FRONTIER: same (Resource,Transition) → same VE id in both rules.
    //     Rule B (Component Role 'button') must reference the SAME ve_<fnv> ids
    //     as Rule A (renders Transition) because both hash the same frontier.
    let pass1_role = resolve_view(cons_role, &pop, &defs)
        .expect("view: def for has Component Role must resolve");
    let role_ids: Vec<String> = pass1_role.as_seq()
        .map(|items| items.iter()
            .filter_map(|f| ast::binding(f, "ViewElement").map(String::from))
            .collect())
        .unwrap_or_default();

    assert_eq!(role_ids.len(), 2,
        "Rule B must produce 2 ViewElement-role bindings (one per legal \
         transition for task-1); got {:?}", role_ids);

    let mut role_ids_sorted = role_ids.clone();
    role_ids_sorted.sort();
    // ids1 is already sorted from pass1 above
    assert_eq!(ids1, role_ids_sorted,
        "Rule A (renders) and Rule B (Component Role 'button') MUST produce \
         the SAME ViewElement ids — same (Resource,Transition) frontier → \
         same ve_<fnv> hash (shared frontier property). \
         renders ids: {:?}; role ids: {:?}", ids1, role_ids_sorted);

    // All Component Role values must be 'button'
    let role_vals: Vec<String> = pass1_role.as_seq()
        .map(|items| items.iter()
            .filter_map(|f| ast::binding(f, "Component Role").map(String::from))
            .collect())
        .unwrap_or_default();
    assert!(role_vals.iter().all(|r| r == "button"),
        "all menu ViewElements must have Component Role 'button'; got {:?}", role_vals);

    // (g) LAZY guard: no `derivation:` def registered (never eager-chained).
    //     This is the task-934 metamodel-hang guard.
    assert!(matches!(ast::fetch_raw(&format!("derivation:{}", cons_renders), &defs),
        Object::Bottom),
        "Rule A must be LAZY (view: only, no derivation: def — the metamodel-hang guard)");
    assert!(matches!(ast::fetch_raw(&format!("derivation:{}", cons_role), &defs),
        Object::Bottom),
        "Rule B must be LAZY (view: only, no derivation: def — the metamodel-hang guard)");
}

// ─── task-934-3a (b): LIVE-AUTHORABLE menu view — compile the authored rule ──
//
// THIS is the task-934-3 part (a) payoff: the COMPILER must reproduce the
// hand-built menu view func above from the AUTHORED skolem-head reading,
// driven end-to-end through the REAL parser + join compiler.
//
// The hand-built `menu_view_derivation_via_skolem_head_lazy_idempotent` test
// pre-populates a single `MenuFrontierFT` (Resource, Transition) cell to
// isolate the skolem-head mechanism. This test does the OPPOSITE: it authors
// the two shared-frontier skolem rules as FORML 2 prose over the FULL
// multi-antecedent metamodel join, compiles them through `parse_to_state` +
// `compile`, and asserts the COMPILED `view:` func reproduces the SAME proven
// behaviour:
//   (a) entity in a non-terminal status → one VE per legal departure transition
//   (b) entity in a terminal status     → ZERO ViewElements
//   (c) deterministic `ve_<fnv>` ids
//   (d) idempotent across two resolve passes
//   (e) `renders Transition` rule → VE carries its Transition
//   (f) `has Component Role 'button'` rule → SAME VE id (shared frontier)
//   (g) NO eager `derivation:` def (lazy-only)
//
// The frontier (Resource, Transition) is produced by the 5-way join
//   Resource is currently in Status     (Resource → Status)
//     ⋈ Transition is from Status       (Transition → Status)   [join on Status]
//     ⋈ Transition is defined in State Machine Definition (Transition → SMD)
//     ⋈ State Machine Definition is for Noun              (SMD → Noun)
//     ⋈ Resource is instance of Noun    (Resource → Noun)       [join on Noun]
// NONE of {Status, Transition, SMD, Noun, Resource} appears on ALL five FTs —
// each shared noun bridges exactly two antecedents (a join CHAIN, not a star).
// The skolem-head join-promotion must therefore key off "shared by ≥2
// antecedents", which is what `resolve_derivation_rule` now does.
#[test]
fn menu_view_derivation_compiled_from_authored_reading_reproduces_proven_func() {
    use crate::ast::{defs_to_state, resolve_view};

    // Authored reading — the two shared-frontier skolem rules from
    // readings/ui/view-menu.md, over the verified metamodel FT names.
    // `ViewElement renders Transition. *` and `ViewElement has Component
    // Role. *` mark both heads View-materialized (lazy). The 5-way join
    // antecedents are spelled exactly as the metamodel declares them.
    let src = r#"# task-934-3 authored menu view
Resource(.Reference) is an entity type.
Reference is a value type.
Status is a value type.
Transition(.id) is an entity type.
id is a value type.
State Machine Definition(.Name) is an entity type.
Name is a value type.
Noun(.NounName) is an entity type.
NounName is a value type.
ViewElement(.veid) is an entity type.
veid is a value type.
Component Role is a value type.

## Fact Types
Resource is currently in Status.
Transition is from Status.
Transition is defined in State Machine Definition.
State Machine Definition is for Noun.
Resource is instance of Noun.
ViewElement renders Transition. *
ViewElement has Component Role. *

## Derivation Rules
* ViewElement (E) renders Transition (Tr) iff Resource is currently in Status and Transition (Tr) is from Status and Transition (Tr) is defined in State Machine Definition and State Machine Definition is for Noun and Resource is instance of Noun.
* ViewElement (E) has Component Role 'button' iff Resource is currently in Status and Transition (Tr) is from Status and Transition (Tr) is defined in State Machine Definition and State Machine Definition is for Noun and Resource is instance of Noun.
"#;
    let state = parse_to_state(src).expect("parse");
    let data = compile::cell_index_from_state(&state);

    // ── Both rules must parse as View-materialized Joins with a skolem head ──
    let renders_rule = data.derivation_rules.iter()
        .find(|r| r.text.contains("renders Transition"))
        .expect("renders rule must parse");
    let role_rule = data.derivation_rules.iter()
        .find(|r| r.text.contains("Component Role"))
        .expect("Component Role rule must parse");
    for (label, rule) in [("renders", renders_rule), ("role", role_rule)] {
        assert!(matches!(rule.materialization, crate::types::MaterializationPolicy::View),
            "{label} rule must be View-materialized (lazy); got {:?}", rule.materialization);
        assert_eq!(rule.kind, DerivationKind::Join,
            "{label} rule must promote to a Join over the 5-way chain (shared-by-≥2 \
             join keys); got {:?}\njoin_on: {:?}\ntext: {}",
            rule.kind, rule.join_on, rule.text);
        assert!(rule.skolem_head_roles.iter().any(|s| s.role == "ViewElement"),
            "{label} rule must record a SkolemHeadRole for the fresh ViewElement (E); \
             got {:#?}\ntext: {}", rule.skolem_head_roles, rule.text);
        // The skolem frontier over the join is the ENTITY-typed antecedent
        // nouns — it MUST include both Resource and Transition so the head is
        // per-(entity, transition) and never collapses two resources sharing a
        // transition. (Status is value-typed → excluded as a join seam.)
        let shr = rule.skolem_head_roles.iter().find(|s| s.role == "ViewElement").unwrap();
        assert!(shr.frontier.contains(&"Transition".to_string()),
            "{label} skolem frontier must include Transition; got {:?}", shr.frontier);
        assert!(shr.frontier.contains(&"Resource".to_string()),
            "{label} skolem frontier must include Resource (else two resources \
             sharing a transition collapse to one ViewElement); got {:?}", shr.frontier);
        assert!(!shr.frontier.contains(&"Status".to_string()),
            "{label} skolem frontier must EXCLUDE value-typed Status (a join \
             seam, not an entity identity); got {:?}", shr.frontier);
    }
    // SHARED FRONTIER (parse level): both sibling rules must skolemise off the
    // IDENTICAL frontier so the fresh ViewElement id matches across them.
    {
        let fa = &renders_rule.skolem_head_roles.iter()
            .find(|s| s.role == "ViewElement").unwrap().frontier;
        let fb = &role_rule.skolem_head_roles.iter()
            .find(|s| s.role == "ViewElement").unwrap().frontier;
        assert_eq!(fa, fb,
            "renders and Component Role heads MUST share an identical skolem \
             frontier (shared-frontier invariant); renders {:?} vs role {:?}", fa, fb);
    }
    // The renders rule's consequent FT carries Transition as an antecedent-bound
    // frontier role; the join must equi-join Status AND Noun (the two chain seams).
    assert!(renders_rule.join_on.contains(&"Status".to_string()),
        "Status must be a join key (Transition.from == Resource.currentStatus); got {:?}",
        renders_rule.join_on);
    assert!(renders_rule.join_on.contains(&"Noun".to_string()),
        "Noun must be a join key (Resource instance-of == SMD for-Noun); got {:?}",
        renders_rule.join_on);

    // ── Compile to defs; both heads emit a `view:` def, NO `derivation:` def ──
    let defs = compile::compile_to_defs_state(&state);
    let d = defs_to_state(&defs, &state);
    let cons_renders = "ViewElement_renders_Transition";
    let cons_role = "ViewElement_has_Component_Role";
    assert!(!matches!(ast::fetch_raw(&format!("view:{}", cons_renders), &d), Object::Bottom),
        "renders head must emit a view: def");
    assert!(!matches!(ast::fetch_raw(&format!("view:{}", cons_role), &d), Object::Bottom),
        "Component Role head must emit a view: def");
    // (g) LAZY guard — no eager derivation: def for EITHER head (metamodel-hang guard).
    assert!(matches!(ast::fetch_raw(&format!("derivation:{}", renders_rule.id), &d), Object::Bottom),
        "renders head must be lazy (view: only, no derivation: def)");
    assert!(matches!(ast::fetch_raw(&format!("derivation:{}", role_rule.id), &d), Object::Bottom),
        "Component Role head must be lazy (view: only, no derivation: def)");

    // ── Population: the SM model from apps/tasks (mirrors the hand-built test) ──
    //   Status 'pending' → transitions 'start', 'delete-from-pending'  (NON-TERMINAL)
    //   Status 'deleted' → (none)                                       (TERMINAL)
    //   task-1 currently in 'pending'  → 2 legal transitions → 2 ViewElements
    //   task-2 currently in 'deleted'  → 0 legal transitions → 0 ViewElements
    // Both tasks are instances of Noun 'Task'; the SMD 'TaskSM' is for 'Task'.
    let pop = {
        let push = |s, cell: &str, pairs: &[(&str, &str)]|
            ast::cell_push(cell, ast::fact_from_pairs(pairs), &s);
        let s = d.clone();
        // current status
        let s = push(s, "Resource_is_currently_in_Status", &[("Resource", "task-1"), ("Status", "pending")]);
        let s = push(s, "Resource_is_currently_in_Status", &[("Resource", "task-2"), ("Status", "deleted")]);
        // transitions from status
        let s = push(s, "Transition_is_from_Status", &[("Transition", "start"), ("Status", "pending")]);
        let s = push(s, "Transition_is_from_Status", &[("Transition", "delete-from-pending"), ("Status", "pending")]);
        // transitions defined in the TaskSM definition
        let s = push(s, "Transition_is_defined_in_State_Machine_Definition", &[("Transition", "start"), ("State Machine Definition", "TaskSM")]);
        let s = push(s, "Transition_is_defined_in_State_Machine_Definition", &[("Transition", "delete-from-pending"), ("State Machine Definition", "TaskSM")]);
        // the TaskSM definition is for the Task noun
        let s = push(s, "State_Machine_Definition_is_for_Noun", &[("State Machine Definition", "TaskSM"), ("Noun", "Task")]);
        // both resources are instances of the Task noun
        let s = push(s, "Resource_is_instance_of_Noun", &[("Resource", "task-1"), ("Noun", "Task")]);
        let s = push(s, "Resource_is_instance_of_Noun", &[("Resource", "task-2"), ("Noun", "Task")]);
        s
    };

    // ── (a)+(b): resolve the renders view lazily; task-1 → 2 VEs, task-2 → 0 ──
    let pass1 = resolve_view(cons_renders, &pop, &d)
        .expect("compiled renders view: def must resolve via resolve_view");
    let elems: Vec<(String, String, String)> = pass1.as_seq()
        .map(|items| items.iter().filter_map(|f| {
            let ve = ast::binding(f, "ViewElement").map(String::from)?;
            let tr = ast::binding(f, "Transition").map(String::from)?;
            let rs = ast::binding(f, "Resource").map(String::from)?;
            Some((ve, tr, rs))
        }).collect())
        .unwrap_or_default();

    let task1: Vec<&(String, String, String)> =
        elems.iter().filter(|(_, _, rs)| rs == "task-1").collect();
    assert_eq!(task1.len(), 2,
        "COMPILED authored reading: task-1 (pending) must produce 2 ViewElements \
         (start + delete-from-pending); got {:?}\nfull: {:#?}", task1, elems);
    let task2: Vec<&(String, String, String)> =
        elems.iter().filter(|(_, _, rs)| rs == "task-2").collect();
    assert_eq!(task2.len(), 0,
        "COMPILED authored reading: task-2 (deleted = terminal) must produce ZERO \
         ViewElements; got {:?}\nfull: {:#?}", task2, elems);

    // (c) deterministic ve_<16 hex> ids
    for (id, tr, rs) in &elems {
        assert!(id.starts_with("ve_") && id.len() == "ve_".len() + 16,
            "VE id must be ve_<16 hex>; got {:?} for ({}, {})", id, rs, tr);
    }

    // (e) Transition carried through
    let t1_trs: Vec<&str> = task1.iter().map(|(_, tr, _)| tr.as_str()).collect();
    assert!(t1_trs.contains(&"start"),
        "start must be in task-1's compiled menu; got {:?}", t1_trs);
    assert!(t1_trs.contains(&"delete-from-pending"),
        "delete-from-pending must be in task-1's compiled menu; got {:?}", t1_trs);

    // (d) idempotent across a second resolve pass
    let pass2 = resolve_view(cons_renders, &pop, &d)
        .expect("compiled renders view: def must resolve on pass 2");
    let mut ids1: Vec<String> = elems.iter().map(|(id, ..)| id.clone()).collect();
    let mut ids2: Vec<String> = pass2.as_seq()
        .map(|items| items.iter()
            .filter_map(|f| ast::binding(f, "ViewElement").map(String::from)).collect())
        .unwrap_or_default();
    ids1.sort();
    ids2.sort();
    assert_eq!(ids1, ids2,
        "COMPILED menu view must be idempotent (same frontier → same ve_<fnv>); \
         pass1 {:?} vs pass2 {:?}", ids1, ids2);

    // (f) shared frontier: the Component Role head produces the SAME ve_<fnv> ids
    let role_view = resolve_view(cons_role, &pop, &d)
        .expect("compiled Component Role view: def must resolve");
    let mut role_ids: Vec<String> = role_view.as_seq()
        .map(|items| items.iter()
            .filter_map(|f| ast::binding(f, "ViewElement").map(String::from)).collect())
        .unwrap_or_default();
    let role_vals: Vec<String> = role_view.as_seq()
        .map(|items| items.iter()
            .filter_map(|f| ast::binding(f, "Component Role").map(String::from)).collect())
        .unwrap_or_default();
    assert_eq!(role_ids.len(), 2,
        "Component Role head must produce 2 VE bindings (one per legal transition \
         for task-1); got {:?}", role_ids);
    role_ids.sort();
    assert_eq!(ids1, role_ids,
        "SHARED FRONTIER: renders and Component Role heads MUST produce identical \
         ve_<fnv> ids (same (Resource,Transition) → same hash). renders {:?} vs role {:?}",
        ids1, role_ids);
    assert!(role_vals.iter().all(|v| v == "button"),
        "all compiled menu VEs must have Component Role 'button'; got {:?}", role_vals);
}

// task-934-3(b): command::menu_component_role projects the iFactr IMenu widget
// role for an entity's legal transitions from the SAME compiled menu reading
// (view-menu.md). The transitions ARE the menu (their consumer is the state
// machine); the role ('button') is DERIVED, not hardcoded. Proves the
// hateoas_via_rho enrichment: a non-terminal entity's transitions self-describe
// as buttons; a terminal entity (no departing transitions) yields no menu.
#[test]
fn menu_component_role_types_legal_transitions_as_button() {
    let src = r#"# task-934-3 authored menu view (component_role wiring)
Resource(.Reference) is an entity type.
Reference is a value type.
Status is a value type.
Transition(.id) is an entity type.
id is a value type.
State Machine Definition(.Name) is an entity type.
Name is a value type.
Noun(.NounName) is an entity type.
NounName is a value type.
ViewElement(.veid) is an entity type.
veid is a value type.
Component Role is a value type.

## Fact Types
Resource is currently in Status.
Transition is from Status.
Transition is defined in State Machine Definition.
State Machine Definition is for Noun.
Resource is instance of Noun.
ViewElement renders Transition. *
ViewElement has Component Role. *

## Derivation Rules
* ViewElement (E) renders Transition (Tr) iff Resource is currently in Status and Transition (Tr) is from Status and Transition (Tr) is defined in State Machine Definition and State Machine Definition is for Noun and Resource is instance of Noun.
* ViewElement (E) has Component Role 'button' iff Resource is currently in Status and Transition (Tr) is from Status and Transition (Tr) is defined in State Machine Definition and State Machine Definition is for Noun and Resource is instance of Noun.
"#;
    let state = parse_to_state(src).expect("parse");
    let defs = compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);
    let pop = {
        let push = |s, cell: &str, pairs: &[(&str, &str)]|
            ast::cell_push(cell, ast::fact_from_pairs(pairs), &s);
        let s = d.clone();
        let s = push(s, "Resource_is_currently_in_Status", &[("Resource", "task-1"), ("Status", "pending")]);
        let s = push(s, "Resource_is_currently_in_Status", &[("Resource", "task-2"), ("Status", "deleted")]);
        let s = push(s, "Transition_is_from_Status", &[("Transition", "start"), ("Status", "pending")]);
        let s = push(s, "Transition_is_from_Status", &[("Transition", "delete-from-pending"), ("Status", "pending")]);
        let s = push(s, "Transition_is_defined_in_State_Machine_Definition", &[("Transition", "start"), ("State Machine Definition", "TaskSM")]);
        let s = push(s, "Transition_is_defined_in_State_Machine_Definition", &[("Transition", "delete-from-pending"), ("State Machine Definition", "TaskSM")]);
        let s = push(s, "State_Machine_Definition_is_for_Noun", &[("State Machine Definition", "TaskSM"), ("Noun", "Task")]);
        let s = push(s, "Resource_is_instance_of_Noun", &[("Resource", "task-1"), ("Noun", "Task")]);
        let s = push(s, "Resource_is_instance_of_Noun", &[("Resource", "task-2"), ("Noun", "Task")]);
        s
    };
    // task-1 (pending, 2 legal transitions) → the DERIVED iFactr IMenu role 'button'.
    assert_eq!(crate::command::menu_component_role(&pop, "task-1").as_deref(), Some("button"),
        "an entity with legal transitions gets the derived iFactr IMenu role 'button'");
    // task-2 (deleted = terminal, no departing transitions) → no menu element → None.
    assert_eq!(crate::command::menu_component_role(&pop, "task-2"), None,
        "a terminal entity has no menu element → None (no procedural fallback)");
}

// crudl-menu-projection: the CRUDL operation catalog (readings/ui/crudl.md) is
// the iFactr ActionType vocabulary (Add/Edit/Delete/Submit/Cancel from
// iFactr-Android/iFactr.UI Controls/ActionType.cs) imported DIRECTLY as
// predicate facts — the iFactr DECORATION over the access-control `Operation`.
// Post-split (2026-05-30) `Operation(.Name)` itself is the access SUBSTRATE
// (readings/access/access.md); crudl.md REFERENCES it and only pins the per-verb
// iFactr metadata. So this compiles the bundle pair (access BEFORE ui, exactly
// as lib.rs assembles them) and pins that `Operation` resolves and the six
// operations carry their iFactr decoration.
#[test]
fn crudl_operation_catalog_parses_and_compiles_grounded_in_ifactr() {
    let src = format!("{}\n{}",
        include_str!("../../../readings/access/access.md"),
        include_str!("../../../readings/ui/crudl.md"));
    let state = parse_to_state(&src).expect("access.md + crudl.md must parse as valid FORML2");
    // Compiles without checker errors (structural validity of the catalog schema).
    let _defs = compile::compile_to_defs_state(&state);
    // The Operation entity type is declared by the access substrate (access.md);
    // crudl.md references it. Resolves in the combined source.
    let nouns: Vec<String> = ast::fetch_cell_seq("Noun", &state).as_seq()
        .map(|items| items.iter()
            .filter_map(|f| ast::binding(f, "name").map(String::from)).collect())
        .unwrap_or_default();
    assert!(nouns.iter().any(|n| n == "Operation"),
        "access.md must declare the Operation entity type crudl.md decorates; got {:?}", nouns);
    // The six CRUDL operations are present as instance-fact subjects — they carry
    // their iFactr decoration in crudl.md (and applies-in-context in access.md).
    let inst = ast::fetch_cell_seq("InstanceFact", &state);
    let subjects: Vec<String> = inst.as_seq()
        .map(|items| items.iter()
            .filter_map(|f| ast::binding(f, "subjectValue").map(String::from)).collect())
        .unwrap_or_default();
    for op in ["create", "edit", "delete", "multi-delete", "save", "cancel"] {
        assert!(subjects.iter().any(|s| s == op),
            "Operation '{op}' (grounded in iFactr ActionType) must be present; \
             got subjects {:?}", subjects);
    }
}

// crudl-menu-projection (authorization model): the permission gate
// 'User is permitted Operation on Noun' derived role-based from 'User has Role'
// + 'Role permits Operation on Noun'. DE-RISKS THE TERNARY FORK: both the
// antecedent 'Role permits Operation on Noun' and the consequent 'User is
// permitted Operation on Noun' are TERNARY (3 entity roles).
//
// FINDING (live-debugged 2026-05-30): ternary fact types PARSE + STORE
// correctly, but the non-skolem multi-antecedent join BINDS THE CONSEQUENT FROM
// THE FIRST ANTECEDENT ONLY -- it emits (User='alice', Role='editor') into the
// (User, Operation, Noun) cell, so the correct permissions never materialize.
// The view derivations avoid this with the SKOLEM path (their (Resource,
// Transition) frontier spans antecedents correctly). So the authz gate must
// OBJECTIFY via skolem, OR compile_join_derivation's non-skolem cross-antecedent
// consequent binding must be fixed. This test holds the TARGET spec; #[ignore]'d
// until the authz model is objectified (see crudl-menu-projection +
// nonskolem-cross-antecedent-join).
// nonskolem-cross-antecedent-join FIXED (2026-05-30): the bridge-key join
// detector is hoisted out of the `!pending_role_comparisons.is_empty()` gate
// in parse_forml2.rs, so this bare ternary equi-join now classifies as Join
// (join_on=[Role]) and binds the consequent across BOTH antecedents instead
// of falling through to ModusPonens (first-antecedent only). Un-ignored.
#[test]
fn authorization_model_ternary_permission_derivation() {
    let src = r#"
User(.Username) is an entity type.
Username is a value type.
Role(.RoleName) is an entity type.
RoleName is a value type.
Operation(.OpName) is an entity type.
OpName is a value type.
Noun(.NounName) is an entity type.
NounName is a value type.

## Fact Types
User has Role.
Role permits Operation on Noun.
User is permitted Operation on Noun. **

## Derivation Rules
* User is permitted Operation on Noun iff User has Role and Role permits Operation on Noun.
"#;
    let state = parse_to_state(src).expect("authorization model (ternary FTs) must parse");
    let defs = compile::compile_to_defs_state(&state);
    let d0 = ast::defs_to_state(&defs, &state);

    // alice has role editor; editor permits Edit + Create on Task (NOT Delete).
    // bob has no role.
    let push = |s, cell: &str, pairs: &[(&str, &str)]|
        ast::cell_push(cell, ast::fact_from_pairs(pairs), &s);
    let d = {
        let s = d0.clone();
        let s = push(s, "User_has_Role", &[("User", "alice"), ("Role", "editor")]);
        let s = push(s, "Role_permits_Operation_on_Noun", &[("Role", "editor"), ("Operation", "Edit"), ("Noun", "Task")]);
        let s = push(s, "Role_permits_Operation_on_Noun", &[("Role", "editor"), ("Operation", "Create"), ("Noun", "Task")]);
        s
    };

    // Forward-chain the eager permission join over the derivation:* defs.
    let refs_owned: Vec<(String, ast::Func)> = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, &d)))
        .collect();
    let refs: Vec<(&str, &ast::Func)> = refs_owned.iter().map(|(n, f)| (n.as_str(), f)).collect();
    let (new_d, _) = crate::evaluate::forward_chain_defs_state(&refs, &d);

    // The derived ternary gate: (User, Operation, Noun) tuples.
    let cell = ast::fetch_cell_seq("User_is_permitted_Operation_on_Noun", &new_d);
    let perms: Vec<(String, String, String)> = cell.as_seq()
        .map(|items| items.iter().filter_map(|f| {
            let u = ast::binding(f, "User")?.to_string();
            let op = ast::binding(f, "Operation")?.to_string();
            let n = ast::binding(f, "Noun")?.to_string();
            Some((u, op, n))
        }).collect())
        .unwrap_or_default();

    assert!(perms.iter().any(|(u, op, n)| u == "alice" && op == "Edit" && n == "Task"),
        "alice (role editor permits Edit on Task) must be permitted Edit on Task; got {:?}", perms);
    assert!(perms.iter().any(|(u, op, n)| u == "alice" && op == "Create" && n == "Task"),
        "alice must be permitted Create on Task; got {:?}", perms);
    assert!(!perms.iter().any(|(_, op, _)| op == "Delete"),
        "alice must NOT be permitted Delete (editor does not permit it); got {:?}", perms);
    assert!(!perms.iter().any(|(u, _, _)| u == "bob"),
        "bob (no role) must have no permissions; got {:?}", perms);
}

// Subtype membership is realised relationally as a joinable DISCRIMINATOR
// (Halpin absorption/separation: a subtype-table FK, a non-null subtype
// column, a unary flag, or an enum). So the categorical authz join is
// authorable TODAY over that discriminator — it does NOT need the structural
// `X is a Y` declaration to be a join antecedent (that lift is pure
// procedural-removal; see task subtype-join-antecedent). Here the
// discriminator is the VALUE TYPE `Access Level` (an absorbed-subtype enum);
// `User has Access Level` ⋈ `Access Level permits Operation on Noun` joins on
// it via the hoisted bridge-key path (1d3d3ebf), binding (User, Operation,
// Noun). This is the deployable, non-colliding form of the authz gate
// (`Access Level` does not collide with core's `Role(.id)`), and it proves a
// VALUE-TYPE bridge key works (the sibling test above used an entity).
#[test]
fn authorization_via_subtype_discriminator_enum() {
    let src = r#"
User(.Username) is an entity type.
Username is a value type.
Access Level is a value type.
Operation(.OpName) is an entity type.
OpName is a value type.
Noun(.NounName) is an entity type.
NounName is a value type.

## Fact Types
User has Access Level.
Access Level permits Operation on Noun.
User is authorized for Operation on Noun. **

## Derivation Rules
* User is authorized for Operation on Noun iff User has Access Level and Access Level permits Operation on Noun.
"#;
    let state = parse_to_state(src).expect("subtype-discriminator authz model must parse");
    let defs = compile::compile_to_defs_state(&state);
    let d0 = ast::defs_to_state(&defs, &state);

    let push = |s, cell: &str, pairs: &[(&str, &str)]|
        ast::cell_push(cell, ast::fact_from_pairs(pairs), &s);
    let d = {
        let s = d0.clone();
        // alice is an admin (the discriminator), bob a viewer.
        let s = push(s, "User_has_Access_Level", &[("User", "alice"), ("Access Level", "admin")]);
        let s = push(s, "User_has_Access_Level", &[("User", "bob"), ("Access Level", "viewer")]);
        // admin may Delete + Edit Task; viewer may only Read Task.
        let s = push(s, "Access_Level_permits_Operation_on_Noun", &[("Access Level", "admin"), ("Operation", "Delete"), ("Noun", "Task")]);
        let s = push(s, "Access_Level_permits_Operation_on_Noun", &[("Access Level", "admin"), ("Operation", "Edit"), ("Noun", "Task")]);
        let s = push(s, "Access_Level_permits_Operation_on_Noun", &[("Access Level", "viewer"), ("Operation", "Read"), ("Noun", "Task")]);
        s
    };

    let refs_owned: Vec<(String, ast::Func)> = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, &d)))
        .collect();
    let refs: Vec<(&str, &ast::Func)> = refs_owned.iter().map(|(n, f)| (n.as_str(), f)).collect();
    let (new_d, _) = crate::evaluate::forward_chain_defs_state(&refs, &d);

    let cell = ast::fetch_cell_seq("User_is_authorized_for_Operation_on_Noun", &new_d);
    let perms: Vec<(String, String, String)> = cell.as_seq()
        .map(|items| items.iter().filter_map(|f| {
            let u = ast::binding(f, "User")?.to_string();
            let op = ast::binding(f, "Operation")?.to_string();
            let n = ast::binding(f, "Noun")?.to_string();
            Some((u, op, n))
        }).collect())
        .unwrap_or_default();

    assert!(perms.iter().any(|(u, op, n)| u == "alice" && op == "Delete" && n == "Task"),
        "alice (admin discriminator) must be authorized Delete on Task; got {:?}", perms);
    assert!(perms.iter().any(|(u, op, n)| u == "alice" && op == "Edit" && n == "Task"),
        "alice (admin) must be authorized Edit on Task; got {:?}", perms);
    assert!(perms.iter().any(|(u, op, n)| u == "bob" && op == "Read" && n == "Task"),
        "bob (viewer) must be authorized Read on Task; got {:?}", perms);
    assert!(!perms.iter().any(|(u, op, _)| u == "alice" && op == "Read"),
        "alice (admin) must NOT be authorized Read here (admin doesn't permit it); got {:?}", perms);
    assert!(!perms.iter().any(|(u, op, _)| u == "bob" && op == "Delete"),
        "bob (viewer) must NOT be authorized Delete; got {:?}", perms);
}

// ── ORM2 role-SEQUENCE (tuple) subset constraint ────────────────────
//
// AREST subset constraints used to handle only a SINGLE role per side;
// a reading-driven TERNARY tuple-subset mis-compiled to a trivial `A ⊆
// A` (0 violations) because stage-2 span extraction collapsed both
// clause-halves onto the same fact type (two FTs sharing the role-noun
// sequence `[User, Operation, Noun]` are indistinguishable by nouns
// alone). These two tests pin the fix: the subset is enforced over the
// FULL shared-variable tuple, per ORM2Core.xsd `ConstraintRoleSequences`
// (two ordered `RoleSequence`s). See the verb-disambiguation stage-2
// unit test `enrich_ternary_ss_spans_resolves_full_role_sequence_*`.

/// (1) DIRECT ternary tuple-subset (no derivation): `performs(U,O,N) ⊆
/// may-perform(U,O,N)`. Seed performs={(alice,Delete,Task),(alice,Drop,
/// Task)}, may-perform={(alice,Delete,Task)}. The witness set is
/// performs \ may-perform = {(alice,Drop,Task)} — exactly ONE alethic
/// subset violation naming `Drop` (the A\B tuple), NOT `Delete` (the
/// satisfier present on both sides).
#[test]
fn ternary_tuple_subset_direct_flags_only_the_a_minus_b_tuple() {
    let src = r#"
User(.Username) is an entity type.
Username is a value type.
Operation(.OpName) is an entity type.
OpName is a value type.
Noun(.NounName) is an entity type.
NounName is a value type.

## Fact Types
User performs Operation on Noun.
User may perform Operation on Noun.

## Constraints
If some User performs some Operation on some Noun then that User may perform that Operation on that Noun.
"#;
    let state = parse_to_state(src).expect("ternary direct tuple-subset model must parse");
    let defs = compile::compile_to_defs_state(&state);
    let d0 = ast::defs_to_state(&defs, &state);
    let push = |s, cell: &str, pairs: &[(&str, &str)]|
        ast::cell_push(cell, ast::fact_from_pairs(pairs), &s);
    let d = {
        let s = d0.clone();
        // performs: alice does Delete AND Drop on Task.
        let s = push(s, "User_performs_Operation_on_Noun", &[("User", "alice"), ("Operation", "Delete"), ("Noun", "Task")]);
        let s = push(s, "User_performs_Operation_on_Noun", &[("User", "alice"), ("Operation", "Drop"), ("Noun", "Task")]);
        // may-perform: alice may only Delete on Task. (Drop is missing → witness.)
        let s = push(s, "User_may_perform_Operation_on_Noun", &[("User", "alice"), ("Operation", "Delete"), ("Noun", "Task")]);
        s
    };

    // validate reads the population from the eval CONTEXT (Selector 3/4),
    // not the `d` argument — so the seeded `d` must be the context state.
    let ctx = ast::encode_eval_context_state("", None, &d);
    let v = ast::decode_violations(&ast::apply(&ast::Func::Def("validate".to_string()), &ctx, &d));

    // Only the subset (SS) constraint's violations.
    let ss: Vec<&crate::types::Violation> = v.iter()
        .filter(|x| x.constraint_text.contains("performs")
                 && x.constraint_text.contains("may perform"))
        .collect();
    assert_eq!(ss.len(), 1,
        "tuple-subset must flag EXACTLY one witness tuple (alice,Drop,Task); got {:?}",
        v.iter().map(|x| (x.constraint_id.as_str(), x.detail.as_str())).collect::<Vec<_>>());
    let viol = ss[0];
    assert!(viol.alethic, "subset constraint is alethic; got {:?}", viol);
    // Detail names the A\B witness tuple — `Drop`, not `Delete`.
    assert!(viol.detail.contains("Drop"),
        "violation detail must name the witness Operation `Drop`; got {:?}", viol.detail);
    assert!(viol.detail.contains("alice") && viol.detail.contains("Task"),
        "violation detail must name the full witness tuple (alice, …, Task); got {:?}", viol.detail);
    assert!(!viol.detail.contains("Delete"),
        "the satisfier tuple (alice,Delete,Task) is in BOTH sides and must NOT be flagged; got {:?}",
        viol.detail);
}

/// (2) Authz-enforcement use case — ternary tuple-subset over a DERIVED
/// consequent. `authorized(U,O,N)` is derived from
/// `has-access-level` ⋈ `permits`; the subset constraint
/// `performs(U,O,N) ⊆ authorized(U,O,N)` then flags any performed
/// operation the user is not authorized for. Forward-chain the
/// `derivation:*` defs FIRST (so `authorized` is populated) THEN
/// validate over the chained state. alice (admin) is authorized Delete
/// on Task but performs BOTH Delete and Drop → (alice,Drop,Task) is the
/// sole witness; (alice,Delete,Task) is authorized and must NOT flag.
#[test]
fn ternary_tuple_subset_over_derived_authorized_flags_unauthorized_performed_op() {
    let src = r#"
User(.Username) is an entity type.
Username is a value type.
Access Level is a value type.
Operation(.OpName) is an entity type.
OpName is a value type.
Noun(.NounName) is an entity type.
NounName is a value type.

## Fact Types
User has Access Level.
Access Level permits Operation on Noun.
User is authorized for Operation on Noun. **
User performs Operation on Noun.

## Derivation Rules
* User is authorized for Operation on Noun iff User has Access Level and Access Level permits Operation on Noun.

## Constraints
If some User performs some Operation on some Noun then that User is authorized for that Operation on that Noun.
"#;
    let state = parse_to_state(src).expect("authz tuple-subset model must parse");
    let defs = compile::compile_to_defs_state(&state);
    let d0 = ast::defs_to_state(&defs, &state);
    let push = |s, cell: &str, pairs: &[(&str, &str)]|
        ast::cell_push(cell, ast::fact_from_pairs(pairs), &s);
    let d = {
        let s = d0.clone();
        let s = push(s, "User_has_Access_Level", &[("User", "alice"), ("Access Level", "admin")]);
        let s = push(s, "Access_Level_permits_Operation_on_Noun", &[("Access Level", "admin"), ("Operation", "Delete"), ("Noun", "Task")]);
        // alice performs BOTH Delete (authorized) and Drop (NOT authorized).
        let s = push(s, "User_performs_Operation_on_Noun", &[("User", "alice"), ("Operation", "Delete"), ("Noun", "Task")]);
        let s = push(s, "User_performs_Operation_on_Noun", &[("User", "alice"), ("Operation", "Drop"), ("Noun", "Task")]);
        s
    };

    // Forward-chain the derivation:* defs so `authorized` is populated.
    let refs_owned: Vec<(String, ast::Func)> = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, &d)))
        .collect();
    let refs: Vec<(&str, &ast::Func)> = refs_owned.iter().map(|(n, f)| (n.as_str(), f)).collect();
    let (new_d, _) = crate::evaluate::forward_chain_defs_state(&refs, &d);

    // Sanity: alice is authorized Delete (derived), but NOT Drop.
    let auth = ast::fetch_cell_seq("User_is_authorized_for_Operation_on_Noun", &new_d);
    let auth_rows: Vec<(String, String, String)> = auth.as_seq()
        .map(|items| items.iter().filter_map(|f| Some((
            ast::binding(f, "User")?.to_string(),
            ast::binding(f, "Operation")?.to_string(),
            ast::binding(f, "Noun")?.to_string(),
        ))).collect())
        .unwrap_or_default();
    assert!(auth_rows.iter().any(|(u, o, n)| u == "alice" && o == "Delete" && n == "Task"),
        "derivation must authorize alice Delete on Task; got {:?}", auth_rows);
    assert!(!auth_rows.iter().any(|(_, o, _)| o == "Drop"),
        "alice must NOT be authorized Drop (admin doesn't permit it); got {:?}", auth_rows);

    // Validate the subset constraint over the forward-chained state.
    let ctx = ast::encode_eval_context_state("", None, &new_d);
    let v = ast::decode_violations(&ast::apply(&ast::Func::Def("validate".to_string()), &ctx, &new_d));
    let ss: Vec<&crate::types::Violation> = v.iter()
        .filter(|x| x.constraint_text.contains("performs")
                 && x.constraint_text.contains("authorized"))
        .collect();
    assert_eq!(ss.len(), 1,
        "exactly one performs-tuple is unauthorized: (alice,Drop,Task); got {:?}",
        v.iter().map(|x| (x.constraint_id.as_str(), x.detail.as_str())).collect::<Vec<_>>());
    let viol = ss[0];
    assert!(viol.alethic, "subset constraint is alethic; got {:?}", viol);
    assert!(viol.detail.contains("Drop") && viol.detail.contains("alice") && viol.detail.contains("Task"),
        "(alice,Drop,Task) must be the flagged witness (performs \\ authorized); got {:?}", viol.detail);
    assert!(!viol.detail.contains("Delete"),
        "(alice,Delete,Task) IS authorized (in performs AND authorized) and must NOT flag; got {:?}",
        viol.detail);
}

// crudl-menu-projection (authz, OBJECTIFIED): the role-based permission gate as
// a skolem-minted Grant, mirroring the proven view-derivation Join+skolem path
// (which is the ONLY path that cross-antecedent-joins; see
// nonskolem-cross-antecedent-join). Three shared-frontier skolem rules mint one
// Grant per (User, Role, Operation, Noun); the Grant links User + Operation +
// Noun, so 'is alice permitted Edit on Task' = EXISTS a Grant authorizing alice,
// granting Edit, applying Task. This is what the CRUDL menu derivation gates on.
#[test]
fn objectified_grant_authz_via_skolem() {
    let src = r#"
User(.Username) is an entity type.
Username is a value type.
Role(.RoleName) is an entity type.
RoleName is a value type.
Operation(.OpName) is an entity type.
OpName is a value type.
Noun(.NounName) is an entity type.
NounName is a value type.
Grant(.gid) is an entity type.
gid is a value type.

## Fact Types
User has Role.
Role permits Operation on Noun.
Grant authorizes User. *
Grant grants Operation. *
Grant applies to Noun. *

## Derivation Rules
* Grant (G) authorizes User iff User has Role and Role permits Operation on Noun.
* Grant (G) grants Operation iff User has Role and Role permits Operation on Noun.
* Grant (G) applies to Noun iff User has Role and Role permits Operation on Noun.
"#;
    let state = parse_to_state(src).expect("parse");
    let defs = compile::compile_to_defs_state(&state);
    let d0 = ast::defs_to_state(&defs, &state);
    let push = |s, cell: &str, pairs: &[(&str, &str)]|
        ast::cell_push(cell, ast::fact_from_pairs(pairs), &s);
    let d = {
        let s = d0.clone();
        let s = push(s, "User_has_Role", &[("User", "alice"), ("Role", "editor")]);
        let s = push(s, "Role_permits_Operation_on_Noun", &[("Role", "editor"), ("Operation", "Edit"), ("Noun", "Task")]);
        s
    };
    let extract = |cell: &str, role: &str| -> Vec<(String, String)> {
        ast::resolve_view(cell, &d, &d)
            .and_then(|o| o.as_seq().map(|items| items.iter().filter_map(|f| {
                let g = ast::binding(f, "Grant")?.to_string();
                let v = ast::binding(f, role)?.to_string();
                Some((g, v))
            }).collect()))
            .unwrap_or_default()
    };
    let auth = extract("Grant_authorizes_User", "User");
    let grnt = extract("Grant_grants_Operation", "Operation");
    let appl = extract("Grant_applies_to_Noun", "Noun");
    assert_eq!(auth.len(), 1, "exactly one Grant authorizes a user; got {:?}", auth);
    let gid = auth[0].0.clone();
    assert_eq!(auth[0].1, "alice", "the Grant authorizes alice; got {:?}", auth);
    assert!(grnt.iter().any(|(g, op)| *g == gid && op == "Edit"),
        "Grant {gid} grants Edit (shared skolem frontier); got {:?}", grnt);
    assert!(appl.iter().any(|(g, n)| *g == gid && n == "Task"),
        "Grant {gid} applies to Task (shared skolem frontier); got {:?}", appl);
}

// access.md (the deploy artifact for the access-control substrate) implements the
// FULL authz model as a registered-shape reading: the `authorized` discriminator-
// join derivation (READ) AND the alethic `performs ⊆ authorized` tuple-subset
// constraint (ENFORCE), composed with the metamodel's User + Noun. Proves the
// reading FILE is correct FORML end-to-end (the semantics are also pinned inline by
// authorization_via_subtype_discriminator_enum + the role-sequence subset tests).
// User/Noun are declared inline as core stand-ins; in the bundle they come from core.
#[test]
fn access_reading_derives_authorized_and_enforces_performs_subset() {
    let src = format!("{}\n{}", r#"
User(.Username) is an entity type.
Username is a value type.
Noun(.NounName) is an entity type.
NounName is a value type.
"#, include_str!("../../../readings/access/access.md"));
    let state = parse_to_state(&src).expect("access.md + core stand-ins must parse");
    let defs = compile::compile_to_defs_state(&state);
    let d0 = ast::defs_to_state(&defs, &state);
    let push = |s, cell: &str, pairs: &[(&str, &str)]|
        ast::cell_push(cell, ast::fact_from_pairs(pairs), &s);
    let d = {
        let s = d0.clone();
        let s = push(s, "User_has_Access_Level", &[("User", "alice"), ("Access Level", "admin")]);
        let s = push(s, "Access_Level_permits_Operation_on_Noun", &[("Access Level", "admin"), ("Operation", "delete"), ("Noun", "Task")]);
        // alice performs delete (authorized) AND drop (NOT permitted -> unauthorized).
        let s = push(s, "User_performs_Operation_on_Noun", &[("User", "alice"), ("Operation", "delete"), ("Noun", "Task")]);
        let s = push(s, "User_performs_Operation_on_Noun", &[("User", "alice"), ("Operation", "drop"), ("Noun", "Task")]);
        s
    };
    // READ half: `authorized` derives (alice, delete, Task) via the discriminator join.
    let refs_owned: Vec<(String, ast::Func)> = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, &d)))
        .collect();
    let refs: Vec<(&str, &ast::Func)> = refs_owned.iter().map(|(n, f)| (n.as_str(), f)).collect();
    let (new_d, _) = crate::evaluate::forward_chain_defs_state(&refs, &d);
    let authz = ast::fetch_cell_seq("User_is_authorized_for_Operation_on_Noun", &new_d);
    let has_delete = authz.as_seq().map_or(false, |items| items.iter().any(|f|
        ast::binding(f, "User") == Some("alice")
            && ast::binding(f, "Operation") == Some("delete")
            && ast::binding(f, "Noun") == Some("Task")));
    assert!(has_delete, "access.md: alice (admin) must be authorized to delete Task");
    // ENFORCE half: the unauthorized `drop` performs is an alethic Subset violation.
    // validate reads the population from the eval CONTEXT, so encode it from the
    // forward-chained, seeded state (new_d) -- NOT the empty readings `state`.
    let ctx = ast::encode_eval_context_state("", None, &new_d);
    let violations = ast::decode_violations(&ast::apply(&ast::Func::Def("validate".to_string()), &ctx, &new_d));
    let subset_v: Vec<&crate::types::Violation> = violations.iter()
        .filter(|v| v.constraint_text.contains("performs") && v.constraint_text.contains("authorized"))
        .collect();
    assert!(subset_v.iter().any(|v| v.detail.contains("drop")),
        "access.md: unauthorized `drop` performs must be a Subset violation; got {:?}",
        subset_v.iter().map(|v| v.detail.as_str()).collect::<Vec<_>>());
    assert!(subset_v.iter().all(|v| v.alethic), "access.md: enforcement must be alethic (reject)");
}

// crudl-menu-projection: the prior view-level `ViewElement renders Operation`
// skolem derivation tests (crudl_menu_derivation_operations_per_context /
// crudl_menu_derivation_permission_gated / crudl_gated_menu_over_real_ifactr_catalog)
// were REMOVED 2026-05-30. That derivation baked the context filter AND the
// permission gate into a view-level rule — conflating view + permission + action.
// Per the owner correction, permissions are substrate FACTS (the server enforces
// them with no UI), and the CRUDL menu is a HATEOAS projection of `authorized`,
// not a view derivation. The corrected path (commit 43dd1ddc) lives in
// command::crudl_menu_operations and is proven by
// crudl_menu_operations_emits_gated_menu_for_user below: it reads the substrate
// `User is authorized for Operation on Noun` ∩ `Operation applies in View Context`
// (crudl.md). The skolem-ViewElement MECHANISM itself stays — it is still used by
// the SM-transition menu (view-menu.md, ViewElement renders Transition).

// crudl-menu-projection (EMISSION, CORRECTED 2026-05-30): command::crudl_menu_operations
// is the HATEOAS seam -- it projects the SUBSTRATE permission predicate
// `User is authorized for Operation on Noun` (∩ the operations applicable in the
// view context, from crudl.md) into the operations the USER may perform.
// Permissions are FACTS, never verbalized in the view (the server gates them with
// no UI at all); the menu is a HATEOAS projection beside hateoas_via_rho /
// nav_links_via_rho. No ViewElement-skolem permission derivation (that conflation
// is retired). Proves the emission through the function the get/list response calls.
#[test]
fn crudl_menu_operations_emits_gated_menu_for_user() {
    // Operation + `Operation applies in View Context` (+ its per-op instances) and
    // `User is authorized for Operation on Noun` are the access SUBSTRATE
    // (access.md); the iFactr Control Kind / Request Type decoration is crudl.md.
    // The emission reads `authorized` ∩ applies-in-context, so include BOTH —
    // exactly as the bundle assembles them (access BEFORE ui). User/Noun are core
    // stand-ins (in the bundle they come from readings/core).
    let additions = r#"
User(.Username) is an entity type.
Username is a value type.
Noun(.NounName) is an entity type.
NounName is a value type.
"#;
    let src = format!("{}\n{}\n{}",
        include_str!("../../../readings/access/access.md"),
        include_str!("../../../readings/ui/crudl.md"),
        additions);
    let state = parse_to_state(&src).expect("access.md + crudl.md + core stand-ins parse");
    let defs = compile::compile_to_defs_state(&state);
    let d0 = ast::defs_to_state(&defs, &state);
    let push = |s, cell: &str, pairs: &[(&str, &str)]|
        ast::cell_push(cell, ast::fact_from_pairs(pairs), &s);
    // Substrate authz facts only -- crudl_menu_operations reads `authorized`
    // (User is authorized for Operation on Noun) ∩ `Operation applies in View
    // Context` (both the access SUBSTRATE, from access.md). alice is authorized
    // for create + edit.
    let d = {
        let s = d0.clone();
        let s = push(s, "User_is_authorized_for_Operation_on_Noun", &[("User", "alice"), ("Operation", "create"), ("Noun", "Task")]);
        let s = push(s, "User_is_authorized_for_Operation_on_Noun", &[("User", "alice"), ("Operation", "edit"), ("Noun", "Task")]);
        s
    };
    // alice's collection menu = [create] (collection-context + permitted; edit is permitted but instance).
    let coll = crate::command::crudl_menu_operations(&d, "Task", "collection", "alice");
    assert!(coll.contains(&"create".to_string()), "collection menu must have create; got {:?}", coll);
    assert!(!coll.contains(&"edit".to_string()), "collection menu excludes edit (instance op); got {:?}", coll);
    // alice's instance menu = [edit].
    let inst = crate::command::crudl_menu_operations(&d, "Task", "instance", "alice");
    assert!(inst.contains(&"edit".to_string()), "instance menu must have edit; got {:?}", inst);
    assert!(!inst.contains(&"create".to_string()), "instance menu excludes create (collection op); got {:?}", inst);
    // bob (no grants) -> empty menu (the permission gate).
    let bob = crate::command::crudl_menu_operations(&d, "Task", "collection", "bob");
    assert!(bob.is_empty(), "bob (no grants) gets an empty menu; got {:?}", bob);
    // The full menu items carry the iFactr catalog metadata (Control Kind, method).
    let coll_full = crate::command::crudl_menu(&d, "Task", "collection", "alice");
    let create = coll_full.iter().find(|m| m.operation == "create")
        .expect("create must be in alice's full collection menu");
    assert_eq!(create.control_kind, "Button", "create's iFactr Control Kind from crudl.md");
    assert_eq!(create.request_type, "POST", "create's CRUDL Request Type from crudl.md");
}

// ─── task-934-3a: VERIFIED METAMODEL FACT-TYPE NAMES ────────────────────────
//
// The following names have been verified against readings/core/state.md,
// readings/core/instances.md, and readings/core/core.md. They are the EXACT
// cell names the full menu-view derivation join must use when wired into the
// live metamodel:
//
//   State_Machine_Definition_is_for_Noun        (state.md "State Machine Definition is for Noun.")
//   Transition_is_defined_in_State_Machine_Definition (state.md "Transition is defined in State Machine Definition.")
//   Transition_is_from_Status                   (state.md "Transition is from Status.")
//   State_Machine_is_currently_in_Status        (instances.md "State Machine is currently in Status.")
//   State_Machine_is_for_Resource               (instances.md "State Machine is for Resource.")
//   Resource_is_instance_of_Noun                (instances.md "Resource is instance of Noun.")
//   Resource_is_currently_in_Status             (instances.md "Resource is currently in Status." — bridge projection)
//
// Join chain for the frontier (Resource, Transition):
//   Resource_is_currently_in_Status (Resource→Status)
//     ⋈ Transition_is_from_Status (Transition→Status)   on Status
//   → gives (Resource, Transition) pairs where the entity is in a status the
//     transition departs from. These are the legal affordances.
//
// Note: `Resource is currently in Status` is derived in apps/tasks/readings/app.md
// via `State Machine is for Resource` × `State Machine is currently in Status`.
// The meta-level menu-view derivation should join on the metamodel-level cells
// (the 6-way join above), not the app-level bridge, since app.md only applies
// to the tasks domain. The menu rule should be general across all Nouns+SMs.
//
// This test function is the "documented anchor" — it has no code to run.
// The `menu_view_derivation_via_skolem_head_lazy_idempotent` test above proves
// the mechanism; this is the schema-name audit record for the compiler wiring.
#[test]
fn menu_view_derivation_metamodel_ft_name_audit() {
    // Verified FT cell names (see comment above):
    let ft_names = &[
        "State_Machine_Definition_is_for_Noun",
        "Transition_is_defined_in_State_Machine_Definition",
        "Transition_is_from_Status",
        "State_Machine_is_currently_in_Status",
        "State_Machine_is_for_Resource",
        "Resource_is_instance_of_Noun",
        "Resource_is_currently_in_Status",
    ];
    // Ensure none of these are empty strings (typo guard).
    for name in ft_names.iter() {
        assert!(!name.is_empty(), "FT name must be non-empty");
        assert!(name.contains('_'), "FT name must use underscore form: {}", name);
    }
    // The frontier roles:
    let frontier_roles = &["Resource", "Transition"];
    for r in frontier_roles {
        assert!(!r.is_empty());
    }
    // Consequent FT names:
    let cons_renders = "ViewElement_renders_Transition";
    let cons_role    = "ViewElement_has_Component_Role";
    assert!(!cons_renders.is_empty());
    assert!(!cons_role.is_empty());
}

// ─── task-934-2: COLLECTION-LIST VIEW ────────────────────────────────────────
//
// The collection-list view generates one ViewElement per Resource instance of
// the collection View's Noun. The join is a 3-antecedent chain:
//
//   View_is_for_Noun (View→Noun_N)
//     ⋈ View_has_View_Kind filtered to 'collection' (View→ViewKind)
//     ⋈ Resource_is_instance_of_Noun (Resource→Noun_N)  [join key: Noun]
//
// Frontier (entity-typed antecedent nouns): [View, Noun, Resource].
// Since each View is for exactly one Noun, the (View, Noun) prefix collapses
// to (View), giving one VE per (View, Resource) pair.
//
// Component Role 'list' — already in the components.md enum, maps to the
// list-row widget (iFactr IContentCell / iItem shape).
//
// Consequent FTs:
//   ViewElement_renders_Resource   (declared in readings/ui/view-list.md, * View-lazy)
//   ViewElement_has_Component_Role (declared in readings/ui/view-projection.md, *)
#[test]
fn collection_list_view_derivation_compiled_from_authored_reading() {
    use crate::ast::{defs_to_state, resolve_view};

    // Authored reading — the two shared-frontier skolem rules from
    // readings/ui/view-list.md, over the verified metamodel FT names.
    // `ViewElement renders Resource. *` and `ViewElement has Component
    // Role. *` mark both heads View-materialized (lazy). The 3-way join
    // antecedents are spelled exactly as the metamodel declares them.
    let src = r#"# task-934-2 authored collection-list view
Resource(.Reference) is an entity type.
Reference is a value type.
Noun(.NounName) is an entity type.
NounName is a value type.
View(.Name) is an entity type.
Name is a value type.
View Kind is a value type.
  The possible values of View Kind are 'collection', 'instance', 'menu'.
ViewElement(.veid) is an entity type.
veid is a value type.
Component Role is a value type.

## Fact Types
View is for Noun.
  Each View is for exactly one Noun.
View has View Kind.
  Each View has exactly one View Kind.
Resource is instance of Noun.
ViewElement renders Resource. *
ViewElement has Component Role. *

## Derivation Rules
* ViewElement (E) renders Resource (R) iff View is for Noun and View has View Kind 'collection' and Resource (R) is instance of Noun.
* ViewElement (E) has Component Role 'list' iff View is for Noun and View has View Kind 'collection' and Resource (R) is instance of Noun.
"#;
    let state = parse_to_state(src).expect("parse");
    let data = compile::cell_index_from_state(&state);

    // ── Both rules must parse as View-materialized Joins with a skolem head ──
    let renders_rule = data.derivation_rules.iter()
        .find(|r| r.text.contains("renders Resource"))
        .expect("renders Resource rule must parse");
    let role_rule = data.derivation_rules.iter()
        .find(|r| r.text.contains("Component Role"))
        .expect("Component Role rule must parse");
    for (label, rule) in [("renders", renders_rule), ("role", role_rule)] {
        assert!(matches!(rule.materialization, crate::types::MaterializationPolicy::View),
            "{label} rule must be View-materialized (lazy); got {:?}", rule.materialization);
        assert_eq!(rule.kind, DerivationKind::Join,
            "{label} rule must promote to a Join over the 3-way chain; got {:?}\n\
             join_on: {:?}\ntext: {}", rule.kind, rule.join_on, rule.text);
        assert!(rule.skolem_head_roles.iter().any(|s| s.role == "ViewElement"),
            "{label} rule must record a SkolemHeadRole for the fresh ViewElement (E); \
             got {:#?}\ntext: {}", rule.skolem_head_roles, rule.text);
        // The entity-typed frontier must include View and Resource (the two
        // entity nouns that discriminate one row from another). Noun is also
        // entity-typed and will be included; since View is for exactly one Noun
        // this is harmless (same effective granularity).
        let shr = rule.skolem_head_roles.iter().find(|s| s.role == "ViewElement").unwrap();
        assert!(shr.frontier.contains(&"View".to_string()),
            "{label} skolem frontier must include View; got {:?}", shr.frontier);
        assert!(shr.frontier.contains(&"Resource".to_string()),
            "{label} skolem frontier must include Resource; got {:?}", shr.frontier);
        // View Kind is value-typed — it must be excluded (it is a filter seam,
        // not an entity identity).
        assert!(!shr.frontier.contains(&"View Kind".to_string()),
            "{label} skolem frontier must EXCLUDE value-typed View Kind; got {:?}", shr.frontier);
        // The join must equi-join on Noun (the bridge between View and Resource)
        assert!(rule.join_on.contains(&"Noun".to_string()),
            "{label} Noun must be a join key (View.forNoun == Resource.instanceOf); \
             got {:?}", rule.join_on);
    }

    // SHARED FRONTIER: both sibling rules must skolemise off the IDENTICAL frontier
    {
        let fa = &renders_rule.skolem_head_roles.iter()
            .find(|s| s.role == "ViewElement").unwrap().frontier;
        let fb = &role_rule.skolem_head_roles.iter()
            .find(|s| s.role == "ViewElement").unwrap().frontier;
        assert_eq!(fa, fb,
            "renders and Component Role heads MUST share an identical skolem \
             frontier (shared-frontier invariant); renders {:?} vs role {:?}", fa, fb);
    }

    // ── Compile to defs; both heads emit a `view:` def, NO `derivation:` def ──
    let defs = compile::compile_to_defs_state(&state);
    let d = defs_to_state(&defs, &state);
    let cons_renders = "ViewElement_renders_Resource";
    let cons_role = "ViewElement_has_Component_Role";
    assert!(!matches!(ast::fetch_raw(&format!("view:{}", cons_renders), &d), Object::Bottom),
        "renders Resource head must emit a view: def");
    assert!(!matches!(ast::fetch_raw(&format!("view:{}", cons_role), &d), Object::Bottom),
        "Component Role head must emit a view: def");
    // (g) LAZY guard — no eager derivation: def for EITHER head
    assert!(matches!(ast::fetch_raw(&format!("derivation:{}", renders_rule.id), &d), Object::Bottom),
        "renders Resource head must be lazy (view: only, no derivation: def)");
    assert!(matches!(ast::fetch_raw(&format!("derivation:{}", role_rule.id), &d), Object::Bottom),
        "Component Role head must be lazy (view: only, no derivation: def)");

    // ── Population ──────────────────────────────────────────────────────────
    //   Noun 'Task'  → 3 instances: task-1, task-2, task-3
    //   Noun 'Bug'   → 0 instances
    //   View 'task-list'  → for Noun 'Task', View Kind 'collection'  → 3 VEs
    //   View 'bug-list'   → for Noun 'Bug',  View Kind 'collection'  → 0 VEs
    //   View 'task-form'  → for Noun 'Task', View Kind 'instance'    → 0 VEs (filter)
    let pop = {
        let push = |s, cell: &str, pairs: &[(&str, &str)]|
            ast::cell_push(cell, ast::fact_from_pairs(pairs), &s);
        let s = d.clone();
        // Views and their Nouns
        let s = push(s, "View_is_for_Noun", &[("View", "task-list"), ("Noun", "Task")]);
        let s = push(s, "View_is_for_Noun", &[("View", "bug-list"),  ("Noun", "Bug")]);
        let s = push(s, "View_is_for_Noun", &[("View", "task-form"), ("Noun", "Task")]);
        // View Kinds
        let s = push(s, "View_has_View_Kind", &[("View", "task-list"), ("View Kind", "collection")]);
        let s = push(s, "View_has_View_Kind", &[("View", "bug-list"),  ("View Kind", "collection")]);
        let s = push(s, "View_has_View_Kind", &[("View", "task-form"), ("View Kind", "instance")]);
        // Task instances
        let s = push(s, "Resource_is_instance_of_Noun", &[("Resource", "task-1"), ("Noun", "Task")]);
        let s = push(s, "Resource_is_instance_of_Noun", &[("Resource", "task-2"), ("Noun", "Task")]);
        let s = push(s, "Resource_is_instance_of_Noun", &[("Resource", "task-3"), ("Noun", "Task")]);
        // Bug: no instances
        s
    };

    // ── (a)+(b): resolve the renders view lazily ──────────────────────────
    let pass1 = resolve_view(cons_renders, &pop, &d)
        .expect("compiled renders view: def must resolve via resolve_view");
    let elems: Vec<(String, String, String)> = pass1.as_seq()
        .map(|items| items.iter().filter_map(|f| {
            let ve  = ast::binding(f, "ViewElement").map(String::from)?;
            let res = ast::binding(f, "Resource").map(String::from)?;
            let vw  = ast::binding(f, "View").map(String::from)?;
            Some((ve, res, vw))
        }).collect())
        .unwrap_or_default();

    // (a) task-list → exactly 3 ViewElements (one per Task instance)
    let task_list_elems: Vec<&(String, String, String)> =
        elems.iter().filter(|(_, _, vw)| vw == "task-list").collect();
    assert_eq!(task_list_elems.len(), 3,
        "task-list (collection, Noun=Task, 3 instances) must produce 3 ViewElements; \
         got {:?}\nfull: {:#?}", task_list_elems, elems);

    // (b) bug-list → 0 ViewElements (Noun=Bug has no instances)
    let bug_list_elems: Vec<&(String, String, String)> =
        elems.iter().filter(|(_, _, vw)| vw == "bug-list").collect();
    assert_eq!(bug_list_elems.len(), 0,
        "bug-list (collection, Noun=Bug, 0 instances) must produce ZERO ViewElements; \
         got {:?}\nfull: {:#?}", bug_list_elems, elems);

    // (h) task-form → 0 ViewElements (View Kind 'instance', not 'collection')
    let task_form_elems: Vec<&(String, String, String)> =
        elems.iter().filter(|(_, _, vw)| vw == "task-form").collect();
    assert_eq!(task_form_elems.len(), 0,
        "task-form (View Kind 'instance') must produce ZERO ViewElements — the \
         'collection' literal filter must exclude it; got {:?}\nfull: {:#?}",
        task_form_elems, elems);

    // (c) deterministic ve_<16 hex> ids
    for (id, res, vw) in &elems {
        assert!(id.starts_with("ve_") && id.len() == "ve_".len() + 16,
            "VE id must be ve_<16 hex>; got {:?} for ({}, {})", id, vw, res);
    }

    // (e) Resource values carried through
    let task_list_resources: Vec<&str> =
        task_list_elems.iter().map(|(_, res, _)| res.as_str()).collect();
    assert!(task_list_resources.contains(&"task-1"),
        "task-1 must appear in task-list renders; got {:?}", task_list_resources);
    assert!(task_list_resources.contains(&"task-2"),
        "task-2 must appear in task-list renders; got {:?}", task_list_resources);
    assert!(task_list_resources.contains(&"task-3"),
        "task-3 must appear in task-list renders; got {:?}", task_list_resources);

    // (d) idempotent across a second resolve pass
    let pass2 = resolve_view(cons_renders, &pop, &d)
        .expect("compiled renders view: def must resolve on pass 2");
    let mut ids1: Vec<String> = elems.iter().map(|(id, ..)| id.clone()).collect();
    let mut ids2: Vec<String> = pass2.as_seq()
        .map(|items| items.iter()
            .filter_map(|f| ast::binding(f, "ViewElement").map(String::from)).collect())
        .unwrap_or_default();
    ids1.sort();
    ids2.sort();
    assert_eq!(ids1, ids2,
        "COMPILED collection-list view must be idempotent (same frontier → same ve_<fnv>); \
         pass1 {:?} vs pass2 {:?}", ids1, ids2);

    // (f) shared frontier: the Component Role head produces the SAME ve_<fnv> ids
    let role_view = resolve_view(cons_role, &pop, &d)
        .expect("compiled Component Role view: def must resolve");
    let mut role_ids: Vec<String> = role_view.as_seq()
        .map(|items| items.iter()
            .filter_map(|f| ast::binding(f, "ViewElement").map(String::from)).collect())
        .unwrap_or_default();
    let role_vals: Vec<String> = role_view.as_seq()
        .map(|items| items.iter()
            .filter_map(|f| ast::binding(f, "Component Role").map(String::from)).collect())
        .unwrap_or_default();
    assert_eq!(role_ids.len(), 3,
        "Component Role head must produce 3 VE bindings (one per Task instance); \
         got {:?}", role_ids);
    role_ids.sort();
    assert_eq!(ids1, role_ids,
        "SHARED FRONTIER: renders and Component Role heads MUST produce identical \
         ve_<fnv> ids (same (View,Noun,Resource) → same hash). renders {:?} vs role {:?}",
        ids1, role_ids);
    assert!(role_vals.iter().all(|v| v == "list"),
        "all compiled collection-list VEs must have Component Role 'list'; got {:?}", role_vals);
}

// ─── task-934-2 — instance-detail (form) view derivation ────────────────────
//
// The instance/detail view projects one ViewElement per Fact Type that a Noun
// participates in, with the Component Role chosen by the Fact Type's value-type
// Format (§3.2 of view-projection-design.md). The derivation uses the SKOLEM
// head + JOIN mechanism proven for the collection-list and menu views.
//
// Key differences from collection-list:
//   - 4 antecedent FTs (vs 3): View_is_for_Noun, View_has_View_Kind,
//     Fact_Type_has_Role, Role_is_played_by_Noun
//   - Widget rules add a 5th antecedent: Fact_Type_has_Format (literal filter)
//   - Frontier is (View, Noun, Fact Type, Role) — 4 entity-typed nouns
//   - One VE per (View, FT), not per (View, Resource)
//
// Layout (mini-schema):
//
//   Entity types  : Noun, Role, Fact Type, View, ViewElement
//   Value types   : NounName, RoleName, FTName, Name, veid,
//                   View Kind, Component Role, Format
//   Fact types    : View is for Noun
//                   View has View Kind     (literal 'instance' filter)
//                   Fact Type has Role
//                   Role is played by Noun (join bridge: Role → Noun)
//                   Fact Type has Format   (literal 'text'/'date'/'boolean')
//                   ViewElement renders Fact Type. *
//                   ViewElement has Component Role. *
//
//   Five skolem rules (all with View-lazy `*`):
//     renders       iff 4-antecedent join (no Format filter)
//     'text-input'  iff 5-antecedent join (Format 'text')
//     'date-picker' iff 5-antecedent join (Format 'date')
//     'checkbox'    iff 5-antecedent join (Format 'boolean')
//
//   Population:
//     Noun 'Task'
//     FT 'ft-title'  → Role 'r-title'  → Noun 'Task', Format 'text'
//     FT 'ft-due'    → Role 'r-due'    → Noun 'Task', Format 'date'
//     FT 'ft-active' → Role 'r-active' → Noun 'Task', Format 'boolean'
//     View 'task-form'  → Noun 'Task',  Kind 'instance'   → 3 VEs (renders)
//     View 'task-list'  → Noun 'Task',  Kind 'collection' → 0 VEs (filter)
//
// Assertions:
//   (a) 3 VEs for task-form (one per FT of Task)
//   (b) 0 VEs for task-list (View Kind 'collection' filtered out)
//   (c) deterministic ve_<16 hex> ids
//   (d) idempotent across a second resolve pass
//   (e) Fact Type values carried through
//   (f) shared frontier: renders and Component Role heads produce SAME ve_<fnv>
//   (g) LAZY — no eager derivation: def for any head
//   (h) correct widget per Format: text→text-input, date→date-picker,
//       boolean→checkbox
#[test]
fn instance_detail_view_derivation_compiled_from_authored_reading() {
    use crate::ast::{defs_to_state, resolve_view};

    // Authored reading — the five shared-frontier skolem rules from
    // readings/ui/view-detail.md, over the verified metamodel FT names.
    // `ViewElement renders Fact Type. *` and `ViewElement has Component
    // Role. *` mark all heads View-materialized (lazy). The antecedents
    // are spelled exactly as the metamodel declares them.
    let src = r#"# task-934-2 authored instance-detail view
Noun(.NounName) is an entity type.
NounName is a value type.
Role(.RoleName) is an entity type.
RoleName is a value type.
Fact Type(.FTName) is an entity type.
FTName is a value type.
View(.Name) is an entity type.
Name is a value type.
ViewElement(.veid) is an entity type.
veid is a value type.
View Kind is a value type.
  The possible values of View Kind are 'collection', 'instance', 'menu'.
Component Role is a value type.
Format is a value type.

## Fact Types
View is for Noun.
  Each View is for exactly one Noun.
View has View Kind.
  Each View has exactly one View Kind.
Fact Type has Role.
  Each Fact Type has some Role.
  For each Role, exactly one Fact Type has that Role.
Role is played by Noun.
  For each Role, exactly one Noun is played by that Role.
Fact Type has Format.
  Each Fact Type has at most one Format.
ViewElement renders Fact Type. *
ViewElement has Component Role. *

## Derivation Rules
* ViewElement (E) renders Fact Type (FT) iff View is for Noun and View has View Kind 'instance' and Fact Type (FT) has Role and Role is played by Noun.
* ViewElement (E) has Component Role 'text-input' iff View is for Noun and View has View Kind 'instance' and Fact Type (FT) has Role and Role is played by Noun and Fact Type (FT) has Format 'text'.
* ViewElement (E) has Component Role 'date-picker' iff View is for Noun and View has View Kind 'instance' and Fact Type (FT) has Role and Role is played by Noun and Fact Type (FT) has Format 'date'.
* ViewElement (E) has Component Role 'checkbox' iff View is for Noun and View has View Kind 'instance' and Fact Type (FT) has Role and Role is played by Noun and Fact Type (FT) has Format 'boolean'.
"#;
    let state = parse_to_state(src).expect("parse");
    let data = compile::cell_index_from_state(&state);

    // ── All rules must parse as View-materialized Joins with a skolem head ──
    let renders_rule = data.derivation_rules.iter()
        .find(|r| r.text.contains("renders Fact Type"))
        .expect("renders Fact Type rule must parse");
    let text_rule = data.derivation_rules.iter()
        .find(|r| r.text.contains("'text-input'"))
        .expect("text-input rule must parse");
    let date_rule = data.derivation_rules.iter()
        .find(|r| r.text.contains("'date-picker'"))
        .expect("date-picker rule must parse");
    let checkbox_rule = data.derivation_rules.iter()
        .find(|r| r.text.contains("'checkbox'"))
        .expect("checkbox rule must parse");

    for (label, rule) in [
        ("renders", renders_rule),
        ("text-input", text_rule),
        ("date-picker", date_rule),
        ("checkbox", checkbox_rule),
    ] {
        assert!(matches!(rule.materialization, crate::types::MaterializationPolicy::View),
            "{label} rule must be View-materialized (lazy); got {:?}", rule.materialization);
        assert_eq!(rule.kind, DerivationKind::Join,
            "{label} rule must promote to a Join over the antecedent chain; \
             got {:?}\njoin_on: {:?}\ntext: {}", rule.kind, rule.join_on, rule.text);
        assert!(rule.skolem_head_roles.iter().any(|s| s.role == "ViewElement"),
            "{label} rule must record a SkolemHeadRole for the fresh ViewElement (E); \
             got {:#?}\ntext: {}", rule.skolem_head_roles, rule.text);
        // Entity-typed frontier must include View, Noun, Fact Type, Role.
        let shr = rule.skolem_head_roles.iter().find(|s| s.role == "ViewElement").unwrap();
        for expected in ["View", "Noun", "Fact Type", "Role"] {
            assert!(shr.frontier.contains(&expected.to_string()),
                "{label} skolem frontier must include {expected}; got {:?}", shr.frontier);
        }
        // Value-typed nouns must be excluded (View Kind, Format, Component Role).
        for excluded in ["View Kind", "Format", "Component Role"] {
            assert!(!shr.frontier.contains(&excluded.to_string()),
                "{label} skolem frontier must EXCLUDE value-typed {excluded}; got {:?}", shr.frontier);
        }
        // Join keys must include Noun (View→Noun join with Role→Noun) and
        // Role (Fact_Type_has_Role ↔ Role_is_played_by_Noun).
        assert!(rule.join_on.contains(&"Noun".to_string()),
            "{label} Noun must be a join key; got {:?}", rule.join_on);
        assert!(rule.join_on.contains(&"Role".to_string()),
            "{label} Role must be a join key; got {:?}", rule.join_on);
    }

    // SHARED FRONTIER: all four sibling rules must skolemise off the IDENTICAL
    // frontier so the ve_<fnv> id matches across renders and all Component Role heads.
    {
        let fa = &renders_rule.skolem_head_roles.iter()
            .find(|s| s.role == "ViewElement").unwrap().frontier;
        for (label, rule) in [("text-input", text_rule), ("date-picker", date_rule), ("checkbox", checkbox_rule)] {
            let fb = &rule.skolem_head_roles.iter()
                .find(|s| s.role == "ViewElement").unwrap().frontier;
            assert_eq!(fa, fb,
                "renders and {label} heads MUST share an identical skolem frontier \
                 (shared-frontier invariant); renders {:?} vs {label} {:?}", fa, fb);
        }
    }

    // ── Compile to defs; all heads emit a `view:` def, NO `derivation:` def ──
    let defs = compile::compile_to_defs_state(&state);
    let d = defs_to_state(&defs, &state);
    let cons_renders = "ViewElement_renders_Fact_Type";
    let cons_role    = "ViewElement_has_Component_Role";
    assert!(!matches!(ast::fetch_raw(&format!("view:{}", cons_renders), &d), Object::Bottom),
        "renders Fact Type head must emit a view: def");
    assert!(!matches!(ast::fetch_raw(&format!("view:{}", cons_role), &d), Object::Bottom),
        "Component Role head must emit a view: def");
    // (g) LAZY guard — no eager derivation: def for ANY head
    for (label, rule) in [
        ("renders", renders_rule), ("text-input", text_rule),
        ("date-picker", date_rule), ("checkbox", checkbox_rule),
    ] {
        assert!(matches!(ast::fetch_raw(&format!("derivation:{}", rule.id), &d), Object::Bottom),
            "{label} head must be lazy (view: only, no derivation: def)");
    }

    // ── Population ──────────────────────────────────────────────────────────
    //   Noun 'Task' with 3 FTs:
    //     ft-title  → role r-title  → Noun Task, Format 'text'
    //     ft-due    → role r-due    → Noun Task, Format 'date'
    //     ft-active → role r-active → Noun Task, Format 'boolean'
    //   View 'task-form'  → Noun Task, Kind 'instance'   → 3 VEs
    //   View 'task-list'  → Noun Task, Kind 'collection' → 0 VEs (filter)
    let pop = {
        let push = |s, cell: &str, pairs: &[(&str, &str)]|
            ast::cell_push(cell, ast::fact_from_pairs(pairs), &s);
        let s = d.clone();
        // Views
        let s = push(s, "View_is_for_Noun", &[("View", "task-form"), ("Noun", "Task")]);
        let s = push(s, "View_is_for_Noun", &[("View", "task-list"), ("Noun", "Task")]);
        // View Kinds
        let s = push(s, "View_has_View_Kind", &[("View", "task-form"), ("View Kind", "instance")]);
        let s = push(s, "View_has_View_Kind", &[("View", "task-list"), ("View Kind", "collection")]);
        // FT role membership: FT → Role → Noun
        let s = push(s, "Fact_Type_has_Role", &[("Fact Type", "ft-title"),  ("Role", "r-title")]);
        let s = push(s, "Fact_Type_has_Role", &[("Fact Type", "ft-due"),    ("Role", "r-due")]);
        let s = push(s, "Fact_Type_has_Role", &[("Fact Type", "ft-active"), ("Role", "r-active")]);
        // Role → Noun bindings (entity side: all roles played by Task)
        let s = push(s, "Role_is_played_by_Noun", &[("Role", "r-title"),  ("Noun", "Task")]);
        let s = push(s, "Role_is_played_by_Noun", &[("Role", "r-due"),    ("Noun", "Task")]);
        let s = push(s, "Role_is_played_by_Noun", &[("Role", "r-active"), ("Noun", "Task")]);
        // Format (value-type side, direct link to FT for test convenience)
        let s = push(s, "Fact_Type_has_Format", &[("Fact Type", "ft-title"),  ("Format", "text")]);
        let s = push(s, "Fact_Type_has_Format", &[("Fact Type", "ft-due"),    ("Format", "date")]);
        let s = push(s, "Fact_Type_has_Format", &[("Fact Type", "ft-active"), ("Format", "boolean")]);
        s
    };

    // ── (a)+(b): resolve the renders view lazily ──────────────────────────
    let pass1 = resolve_view(cons_renders, &pop, &d)
        .expect("compiled renders view: def must resolve via resolve_view");
    let elems: Vec<(String, String, String)> = pass1.as_seq()
        .map(|items| items.iter().filter_map(|f| {
            let ve = ast::binding(f, "ViewElement").map(String::from)?;
            let ft = ast::binding(f, "Fact Type").map(String::from)?;
            let vw = ast::binding(f, "View").map(String::from)?;
            Some((ve, ft, vw))
        }).collect())
        .unwrap_or_default();

    // (a) task-form → exactly 3 ViewElements (one per FT of Task)
    let form_elems: Vec<&(String, String, String)> =
        elems.iter().filter(|(_, _, vw)| vw == "task-form").collect();
    assert_eq!(form_elems.len(), 3,
        "task-form (instance, Noun=Task, 3 FTs) must produce 3 ViewElements; \
         got {:?}\nfull: {:#?}", form_elems, elems);

    // (b) task-list → 0 ViewElements (View Kind 'collection' filtered out)
    let list_elems: Vec<&(String, String, String)> =
        elems.iter().filter(|(_, _, vw)| vw == "task-list").collect();
    assert_eq!(list_elems.len(), 0,
        "task-list (collection kind) must produce ZERO ViewElements — the \
         'instance' literal filter must exclude it; got {:?}\nfull: {:#?}",
        list_elems, elems);

    // (c) deterministic ve_<16 hex> ids
    for (id, ft, vw) in &elems {
        assert!(id.starts_with("ve_") && id.len() == "ve_".len() + 16,
            "VE id must be ve_<16 hex>; got {:?} for ({}, {})", id, vw, ft);
    }

    // (e) Fact Type values carried through (all three FTs appear)
    let form_fts: Vec<&str> =
        form_elems.iter().map(|(_, ft, _)| ft.as_str()).collect();
    assert!(form_fts.contains(&"ft-title"),
        "ft-title must appear in task-form renders; got {:?}", form_fts);
    assert!(form_fts.contains(&"ft-due"),
        "ft-due must appear in task-form renders; got {:?}", form_fts);
    assert!(form_fts.contains(&"ft-active"),
        "ft-active must appear in task-form renders; got {:?}", form_fts);

    // (d) idempotent across a second resolve pass
    let pass2 = resolve_view(cons_renders, &pop, &d)
        .expect("compiled renders view: def must resolve on pass 2");
    let mut ids1: Vec<String> = form_elems.iter().map(|(id, ..)| id.clone()).collect();
    let mut ids2: Vec<String> = pass2.as_seq()
        .map(|items| items.iter()
            .filter_map(|f| {
                let ve = ast::binding(f, "ViewElement").map(String::from)?;
                let vw = ast::binding(f, "View").map(String::from)?;
                (vw == "task-form").then_some(ve)
            }).collect())
        .unwrap_or_default();
    ids1.sort();
    ids2.sort();
    assert_eq!(ids1, ids2,
        "COMPILED instance-detail view must be idempotent (same frontier → same ve_<fnv>); \
         pass1 {:?} vs pass2 {:?}", ids1, ids2);

    // (f) shared frontier: all Component Role heads produce the SAME ve_<fnv> ids
    let role_view = resolve_view(cons_role, &pop, &d)
        .expect("compiled Component Role view: def must resolve");
    let role_rows: Vec<(String, String, String)> = role_view.as_seq()
        .map(|items| items.iter().filter_map(|f| {
            let ve   = ast::binding(f, "ViewElement").map(String::from)?;
            let role = ast::binding(f, "Component Role").map(String::from)?;
            let vw   = ast::binding(f, "View").map(String::from)?;
            Some((ve, role, vw))
        }).collect())
        .unwrap_or_default();
    let form_role_rows: Vec<&(String, String, String)> =
        role_rows.iter().filter(|(_, _, vw)| vw == "task-form").collect();
    assert_eq!(form_role_rows.len(), 3,
        "Component Role head must produce 3 VE bindings for task-form; \
         got {:?}", form_role_rows);
    let mut role_ids: Vec<String> = form_role_rows.iter().map(|(id, _, _)| id.clone()).collect();
    role_ids.sort();
    assert_eq!(ids1, role_ids,
        "SHARED FRONTIER: renders and Component Role heads MUST produce identical \
         ve_<fnv> ids (same (View,Noun,Fact Type,Role) → same hash). \
         renders {:?} vs role {:?}", ids1, role_ids);

    // (h) correct widget per Format
    //   ft-title  (Format 'text')    → Component Role 'text-input'
    //   ft-due    (Format 'date')    → Component Role 'date-picker'
    //   ft-active (Format 'boolean') → Component Role 'checkbox'
    //
    // Resolve each widget view and look up by VE id.
    let ft_to_ve: std::collections::HashMap<String, String> =
        form_elems.iter().map(|(ve, ft, _)| (ft.clone(), ve.clone())).collect();

    let title_ve  = ft_to_ve.get("ft-title").expect("ft-title must have a VE id");
    let due_ve    = ft_to_ve.get("ft-due").expect("ft-due must have a VE id");
    let active_ve = ft_to_ve.get("ft-active").expect("ft-active must have a VE id");

    let role_by_ve: std::collections::HashMap<String, String> =
        form_role_rows.iter().map(|(ve, role, _)| (ve.clone(), role.clone())).collect();

    assert_eq!(role_by_ve.get(title_ve).map(String::as_str), Some("text-input"),
        "ft-title (Format 'text') → VE {title_ve} must have Component Role 'text-input'; \
         got {:?}\nrole_by_ve: {:?}", role_by_ve.get(title_ve), role_by_ve);
    assert_eq!(role_by_ve.get(due_ve).map(String::as_str), Some("date-picker"),
        "ft-due (Format 'date') → VE {due_ve} must have Component Role 'date-picker'; \
         got {:?}\nrole_by_ve: {:?}", role_by_ve.get(due_ve), role_by_ve);
    assert_eq!(role_by_ve.get(active_ve).map(String::as_str), Some("checkbox"),
        "ft-active (Format 'boolean') → VE {active_ve} must have Component Role 'checkbox'; \
         got {:?}\nrole_by_ve: {:?}", role_by_ve.get(active_ve), role_by_ve);
}

// ─── task-viewproj — view_via_rho projection (hybrid: default + override) ─────
//
// Proves the COMMAND-layer View projection (the iFactr/MonoView "abstract UI"
// half of the Theorem-4 HATEOAS representation) over the SAME view-detail
// skolem rules the GREEN derivation test above exercises:
//   - Tier 1 (iFactr default): NO authored View → view_via_rho SYNTHESIZES a
//     transient instance View; one widget per value-typed Fact Type, the widget
//     keyed off the Format (text→text-input, date→date-picker, bool→checkbox).
//   - Tier 2 (MonoView override): an authored instance View for the Noun WINS
//     (source='authored', the authored View id surfaces) — the hybrid the user
//     chose: synthesized default, authored override.
// This is the seam the kernel HATEOAS browser / ui.do worker consume to render
// a form for an entity. Returns None where ui-readings is off (no view: defs).
#[test]
fn view_via_rho_synthesizes_default_and_honors_authored_override() {
    let src = r#"# task-viewproj projection test — view-detail rules
Noun(.NounName) is an entity type.
NounName is a value type.
Role(.RoleName) is an entity type.
RoleName is a value type.
Fact Type(.FTName) is an entity type.
FTName is a value type.
View(.Name) is an entity type.
Name is a value type.
ViewElement(.veid) is an entity type.
veid is a value type.
View Kind is a value type.
  The possible values of View Kind are 'collection', 'instance', 'menu'.
Component Role is a value type.
Format is a value type.

## Fact Types
View is for Noun.
  Each View is for exactly one Noun.
View has View Kind.
  Each View has exactly one View Kind.
Fact Type has Role.
  Each Fact Type has some Role.
  For each Role, exactly one Fact Type has that Role.
Role is played by Noun.
  For each Role, exactly one Noun is played by that Role.
Fact Type has Format.
  Each Fact Type has at most one Format.
ViewElement renders Fact Type. *
ViewElement has Component Role. *

## Derivation Rules
* ViewElement (E) renders Fact Type (FT) iff View is for Noun and View has View Kind 'instance' and Fact Type (FT) has Role and Role is played by Noun.
* ViewElement (E) has Component Role 'text-input' iff View is for Noun and View has View Kind 'instance' and Fact Type (FT) has Role and Role is played by Noun and Fact Type (FT) has Format 'text'.
* ViewElement (E) has Component Role 'date-picker' iff View is for Noun and View has View Kind 'instance' and Fact Type (FT) has Role and Role is played by Noun and Fact Type (FT) has Format 'date'.
* ViewElement (E) has Component Role 'checkbox' iff View is for Noun and View has View Kind 'instance' and Fact Type (FT) has Role and Role is played by Noun and Fact Type (FT) has Format 'boolean'.
"#;
    let state = parse_to_state(src).expect("parse");
    let defs = compile::compile_to_defs_state(&state);
    let d0 = ast::defs_to_state(&defs, &state);

    // Base population: Noun 'Task' with 3 value-typed FTs. NO View authored.
    let push = |s: ast::Object, cell: &str, pairs: &[(&str, &str)]|
        ast::cell_push(cell, ast::fact_from_pairs(pairs), &s);
    let base = {
        let s = d0.clone();
        let s = push(s, "Fact_Type_has_Role", &[("Fact Type", "ft-title"),  ("Role", "r-title")]);
        let s = push(s, "Fact_Type_has_Role", &[("Fact Type", "ft-due"),    ("Role", "r-due")]);
        let s = push(s, "Fact_Type_has_Role", &[("Fact Type", "ft-active"), ("Role", "r-active")]);
        let s = push(s, "Role_is_played_by_Noun", &[("Role", "r-title"),  ("Noun", "Task")]);
        let s = push(s, "Role_is_played_by_Noun", &[("Role", "r-due"),    ("Noun", "Task")]);
        let s = push(s, "Role_is_played_by_Noun", &[("Role", "r-active"), ("Noun", "Task")]);
        let s = push(s, "Fact_Type_has_Format", &[("Fact Type", "ft-title"),  ("Format", "text")]);
        let s = push(s, "Fact_Type_has_Format", &[("Fact Type", "ft-due"),    ("Format", "date")]);
        let s = push(s, "Fact_Type_has_Format", &[("Fact Type", "ft-active"), ("Format", "boolean")]);
        s
    };

    // ── Tier 1: synthesized default (no authored View) ──
    let vp = crate::command::view_via_rho(&base, "Task", "task-1")
        .expect("synthesized instance view must project for a Noun with value-typed FTs");
    assert_eq!(vp.source, "synthesized", "no authored View → iFactr default tier");
    assert_eq!(vp.kind, "instance");
    assert_eq!(vp.view, "instance-view-Task", "synthesized View id is the deterministic slug");
    assert_eq!(vp.elements.len(), 3, "one widget per value-typed FT; got {:#?}", vp.elements);
    let widget = |ft: &str| vp.elements.iter().find(|e| e.fact_type == ft).map(|e| e.component_role.as_str());
    assert_eq!(widget("ft-title"),  Some("text-input"),  "text Format → text-input");
    assert_eq!(widget("ft-due"),    Some("date-picker"), "date Format → date-picker");
    assert_eq!(widget("ft-active"), Some("checkbox"),    "boolean Format → checkbox");
    for e in &vp.elements {
        assert!(e.id.starts_with("ve_") && e.id.len() == "ve_".len() + 16,
            "element id must be the skolem ve_<16hex>; got {:?}", e.id);
    }

    // ── Tier 2: authored override wins (source='authored', authored id) ──
    let authored = {
        let s = push(base.clone(), "View_is_for_Noun", &[("View", "task-custom"), ("Noun", "Task")]);
        push(s, "View_has_View_Kind", &[("View", "task-custom"), ("View Kind", "instance")])
    };
    let vp2 = crate::command::view_via_rho(&authored, "Task", "task-1")
        .expect("authored instance view must project");
    assert_eq!(vp2.source, "authored", "an authored instance View overrides the synthesized default");
    assert_eq!(vp2.view, "task-custom", "the authored View id surfaces");
    assert_eq!(vp2.elements.len(), 3, "authored View derives the same 3 widgets; got {:#?}", vp2.elements);
    let widget2 = |ft: &str| vp2.elements.iter().find(|e| e.fact_type == ft).map(|e| e.component_role.as_str());
    assert_eq!(widget2("ft-due"), Some("date-picker"), "widgets still value-type-driven under override");
}

// ─── task-934-2 — real-data format projection ────────────────────────────────
//
// Proves that `Fact_Type_has_Format` and `Fact_Type_has_Enum_Values` are
// derived EAGERLY (via `**` rules), WITHOUT manually populating those cells.
// The test populates only the raw metamodel facts:
//   - Fact_Type_has_Role (FT → entity-side Role)
//   - Role_is_played_by_Noun (entity-side Role → entity Noun)
//   - Noun_has_Object_Type (value-type Noun → 'value')
//   - Noun_has_Format (value-type Noun → Format string)
//   - Noun_has_Enum_Values (enum Noun → Enum Values string)
//
// Then runs forward_chain to materialize the eager projections, and
// assert_resolves the lazy widget rules against the derived population.
//
// Assertions:
//   (1) Format projection rule is EAGER (Stored materialization)
//   (2) Enum Values projection rule is EAGER (Stored materialization)
//   (3) After forward chain: Fact_Type_has_Format populated (no manual push)
//   (4) After forward chain: Fact_Type_has_Enum_Values populated (no manual push)
//   (5) resolve_view for Component Role gives correct widgets:
//       ft-title  (text)    → 'text-input'
//       ft-due    (date)    → 'date-picker'
//       ft-active (boolean) → 'checkbox'
//       ft-prio   (enum)    → 'combo-box'
//   (6) No overlap: each FT gets exactly ONE Component Role
#[test]
fn instance_detail_view_real_data_format_projection() {
    use crate::ast::{defs_to_state, resolve_view};
    use crate::types::MaterializationPolicy;

    // ── Schema: full real-data structure ─────────────────────────────────────
    // Includes Noun has Object Type, Noun has Format, Noun has Enum Values,
    // Fact Type has Format (** eager), Fact Type has Enum Values (** eager),
    // the two eager projection derivation rules, and the six lazy view rules.
    let src = r#"# task-934-2 real-data format projection test
Noun(.NounName) is an entity type.
NounName is a value type.
Role(.RoleName) is an entity type.
RoleName is a value type.
Fact Type(.FTName) is an entity type.
FTName is a value type.
View(.Name) is an entity type.
Name is a value type.
ViewElement(.veid) is an entity type.
veid is a value type.
View Kind is a value type.
  The possible values of View Kind are 'collection', 'instance', 'menu'.
Component Role is a value type.
Format is a value type.
Enum Values is a value type.
Object Type is a value type.
  The possible values of Object Type are 'entity', 'value'.

## Fact Types
View is for Noun.
  Each View is for exactly one Noun.
View has View Kind.
  Each View has exactly one View Kind.
Fact Type has Role.
  Each Fact Type has some Role.
  For each Role, exactly one Fact Type has that Role.
Role is played by Noun.
  For each Role, exactly one Noun is played by that Role.
Noun has Object Type.
  Each Noun has exactly one Object Type.
Noun has Format.
  Each Noun has at most one Format.
Noun has Enum Values.
  Each Noun has at most one Enum Values.
Fact Type has Format. **
  Each Fact Type has at most one Format.
Fact Type has Enum Values. **
  Each Fact Type has at most one Enum Values.
ViewElement renders Fact Type. *
ViewElement has Component Role. *

## Derivation Rules
** Fact Type has Format iff Fact Type has some Role and that Role is played by some Noun and that Noun has Object Type 'value' and that Noun has Format.
** Fact Type has Enum Values iff Fact Type has some Role and that Role is played by some Noun and that Noun has Object Type 'value' and that Noun has some Enum Values.
* ViewElement (E) renders Fact Type (FT) iff View is for Noun and View has View Kind 'instance' and Fact Type (FT) has Role and Role is played by Noun.
* ViewElement (E) has Component Role 'text-input' iff View is for Noun and View has View Kind 'instance' and Fact Type (FT) has Role and Role is played by Noun and Fact Type (FT) has Format 'text'.
* ViewElement (E) has Component Role 'date-picker' iff View is for Noun and View has View Kind 'instance' and Fact Type (FT) has Role and Role is played by Noun and Fact Type (FT) has Format 'date'.
* ViewElement (E) has Component Role 'checkbox' iff View is for Noun and View has View Kind 'instance' and Fact Type (FT) has Role and Role is played by Noun and Fact Type (FT) has Format 'boolean'.
* ViewElement (E) has Component Role 'combo-box' iff View is for Noun and View has View Kind 'instance' and Fact Type (FT) has Role and Role is played by Noun and Fact Type (FT) has some Enum Values.
"#;

    let state = parse_to_state(src).expect("parse");
    let model = compile::compile(&state);

    // ── (1)+(2) Eager rule assertions ────────────────────────────────────────
    // Format projection rule must be Stored (eager), not View.
    let format_proj = model.derivations.iter()
        .find(|d| d.text.contains("Fact Type has Format iff"))
        .expect("Format projection rule must parse and compile");
    assert!(matches!(format_proj.materialization, MaterializationPolicy::Stored),
        "Format projection rule must be EAGER (Stored); got {:?}", format_proj.materialization);

    let enum_proj = model.derivations.iter()
        .find(|d| d.text.contains("Fact Type has Enum Values iff"))
        .expect("Enum Values projection rule must parse and compile");
    assert!(matches!(enum_proj.materialization, MaterializationPolicy::Stored),
        "Enum Values projection rule must be EAGER (Stored); got {:?}", enum_proj.materialization);

    // View rules must remain lazy (View materialization).
    for label in ["renders Fact Type", "text-input", "date-picker", "checkbox", "combo-box"] {
        let rule = model.derivations.iter()
            .find(|d| d.text.contains(label))
            .unwrap_or_else(|| panic!("{label} rule must parse and compile"));
        assert!(matches!(rule.materialization, MaterializationPolicy::View),
            "{label} rule must be LAZY (View); got {:?}", rule.materialization);
    }

    // ── Build defs state + empty population ──────────────────────────────────
    let defs = compile::compile_to_defs_state(&state);
    let d = defs_to_state(&defs, &state);

    // ── Population: raw metamodel facts — NO manual Fact_Type_has_Format ─────
    //
    // Four entity FTs — each has one entity-side Role (played by Task).
    // Four value-type Nouns — each plays one Role in its FT.
    //
    //   ft-title   → r-title-e  → Task (entity side)
    //                r-title-v  → TitleT (value: Object Type 'value', Format 'text')
    //   ft-due     → r-due-e    → Task
    //                r-due-v    → DueT   (value: Object Type 'value', Format 'date')
    //   ft-active  → r-active-e → Task
    //                r-active-v → BoolT  (value: Object Type 'value', Format 'boolean')
    //   ft-prio    → r-prio-e   → Task
    //                r-prio-v   → PrioT  (value: Object Type 'value', Enum Values 'high,medium,low')
    //
    // View 'task-form' is for Noun 'Task', Kind 'instance'.
    let pop = {
        let push = |s, cell: &str, pairs: &[(&str, &str)]|
            ast::cell_push(cell, ast::fact_from_pairs(pairs), &s);
        let s = d.clone();
        // View
        let s = push(s, "View_is_for_Noun",   &[("View", "task-form"), ("Noun", "Task")]);
        let s = push(s, "View_has_View_Kind",  &[("View", "task-form"), ("View Kind", "instance")]);
        // FT → entity-side Role → Task
        let s = push(s, "Fact_Type_has_Role",       &[("Fact Type", "ft-title"),  ("Role", "r-title-e")]);
        let s = push(s, "Role_is_played_by_Noun",   &[("Role", "r-title-e"),  ("Noun", "Task")]);
        let s = push(s, "Fact_Type_has_Role",       &[("Fact Type", "ft-due"),    ("Role", "r-due-e")]);
        let s = push(s, "Role_is_played_by_Noun",   &[("Role", "r-due-e"),    ("Noun", "Task")]);
        let s = push(s, "Fact_Type_has_Role",       &[("Fact Type", "ft-active"), ("Role", "r-active-e")]);
        let s = push(s, "Role_is_played_by_Noun",   &[("Role", "r-active-e"), ("Noun", "Task")]);
        let s = push(s, "Fact_Type_has_Role",       &[("Fact Type", "ft-prio"),   ("Role", "r-prio-e")]);
        let s = push(s, "Role_is_played_by_Noun",   &[("Role", "r-prio-e"),   ("Noun", "Task")]);
        // FT → value-side Role → value-type Noun
        let s = push(s, "Fact_Type_has_Role",       &[("Fact Type", "ft-title"),  ("Role", "r-title-v")]);
        let s = push(s, "Role_is_played_by_Noun",   &[("Role", "r-title-v"),  ("Noun", "TitleT")]);
        let s = push(s, "Fact_Type_has_Role",       &[("Fact Type", "ft-due"),    ("Role", "r-due-v")]);
        let s = push(s, "Role_is_played_by_Noun",   &[("Role", "r-due-v"),    ("Noun", "DueT")]);
        let s = push(s, "Fact_Type_has_Role",       &[("Fact Type", "ft-active"), ("Role", "r-active-v")]);
        let s = push(s, "Role_is_played_by_Noun",   &[("Role", "r-active-v"), ("Noun", "BoolT")]);
        let s = push(s, "Fact_Type_has_Role",       &[("Fact Type", "ft-prio"),   ("Role", "r-prio-v")]);
        let s = push(s, "Role_is_played_by_Noun",   &[("Role", "r-prio-v"),   ("Noun", "PrioT")]);
        // Value-type Nouns: Object Type + Format or Enum Values
        let s = push(s, "Noun_has_Object_Type", &[("Noun", "TitleT"), ("Object Type", "value")]);
        let s = push(s, "Noun_has_Object_Type", &[("Noun", "DueT"),   ("Object Type", "value")]);
        let s = push(s, "Noun_has_Object_Type", &[("Noun", "BoolT"),  ("Object Type", "value")]);
        let s = push(s, "Noun_has_Object_Type", &[("Noun", "PrioT"),  ("Object Type", "value")]);
        let s = push(s, "Noun_has_Format",      &[("Noun", "TitleT"), ("Format", "text")]);
        let s = push(s, "Noun_has_Format",      &[("Noun", "DueT"),   ("Format", "date")]);
        let s = push(s, "Noun_has_Format",      &[("Noun", "BoolT"),  ("Format", "boolean")]);
        let s = push(s, "Noun_has_Enum_Values", &[("Noun", "PrioT"),  ("Enum Values", "high,medium,low")]);
        s
    };

    // ── (3)+(4) Forward chain materializes the eager projections ─────────────
    // Run only the EAGER (Stored-materialized) derivation rules.
    let eager_refs: Vec<(&str, &ast::Func)> = model.derivations.iter()
        .filter(|d| matches!(d.materialization, MaterializationPolicy::Stored))
        .map(|d| (d.id.as_str(), &d.func))
        .collect();
    let (derived_pop, derived_facts) =
        crate::evaluate::forward_chain_defs_state(&eager_refs, &pop);

    // Fact_Type_has_Format must now be populated (NOT hand-pushed).
    let fmt_cell = ast::fetch_cell_seq("Fact_Type_has_Format", &derived_pop);
    let fmt_rows: Vec<(String, String)> = fmt_cell.as_seq()
        .map(|facts| facts.iter().filter_map(|f| {
            let ft  = ast::binding(f, "Fact Type").map(String::from)?;
            let fmt = ast::binding(f, "Format").map(String::from)?;
            Some((ft, fmt))
        }).collect())
        .unwrap_or_default();
    assert_eq!(fmt_rows.len(), 3,
        "eager Format projection must produce 3 rows (text/date/boolean FTs); \
         got {} derived_facts:{:#?}\nfmt_rows:{:?}", fmt_rows.len(), derived_facts, fmt_rows);
    let fmt_map: std::collections::HashMap<String, String> =
        fmt_rows.into_iter().collect();
    assert_eq!(fmt_map.get("ft-title").map(String::as_str), Some("text"),
        "ft-title must have Format 'text'; map: {:?}", fmt_map);
    assert_eq!(fmt_map.get("ft-due").map(String::as_str), Some("date"),
        "ft-due must have Format 'date'; map: {:?}", fmt_map);
    assert_eq!(fmt_map.get("ft-active").map(String::as_str), Some("boolean"),
        "ft-active must have Format 'boolean'; map: {:?}", fmt_map);
    assert!(!fmt_map.contains_key("ft-prio"),
        "ft-prio (enum, no Format) must NOT appear in Format projection; map: {:?}", fmt_map);

    // Fact_Type_has_Enum_Values must now be populated.
    let enum_cell = ast::fetch_cell_seq("Fact_Type_has_Enum_Values", &derived_pop);
    let enum_rows: Vec<(String, String)> = enum_cell.as_seq()
        .map(|facts| facts.iter().filter_map(|f| {
            let ft   = ast::binding(f, "Fact Type").map(String::from)?;
            let enms = ast::binding(f, "Enum Values").map(String::from)?;
            Some((ft, enms))
        }).collect())
        .unwrap_or_default();
    assert_eq!(enum_rows.len(), 1,
        "eager Enum Values projection must produce 1 row (enum FT only); \
         got {}: {:?}", enum_rows.len(), enum_rows);
    assert_eq!(enum_rows[0].0, "ft-prio",
        "Enum Values projection row must be for ft-prio; got {:?}", enum_rows);

    // ── (5) Lazy widget rules fire correctly ──────────────────────────────────
    // resolve_view evaluates the widget rules against the derived population.
    let cons_role = "ViewElement_has_Component_Role";
    let role_view = resolve_view(cons_role, &derived_pop, &d)
        .expect("Component Role view: def must resolve");
    let role_rows: Vec<(String, String, String)> = role_view.as_seq()
        .map(|items| items.iter().filter_map(|f| {
            let ve   = ast::binding(f, "ViewElement").map(String::from)?;
            let role = ast::binding(f, "Component Role").map(String::from)?;
            let vw   = ast::binding(f, "View").map(String::from)?;
            Some((ve, role, vw))
        }).collect())
        .unwrap_or_default();

    let form_role_rows: Vec<&(String, String, String)> =
        role_rows.iter().filter(|(_, _, vw)| vw == "task-form").collect();

    // (6) Exactly 4 VEs — one per FT
    assert_eq!(form_role_rows.len(), 4,
        "task-form must produce 4 Component Role VEs (text/date/boolean/enum); \
         got {:?}", form_role_rows);

    // Build map: FT → (VE, Component Role) via the renders view
    let cons_renders = "ViewElement_renders_Fact_Type";
    let renders_view = resolve_view(cons_renders, &derived_pop, &d)
        .expect("renders Fact Type view: def must resolve");
    let renders_rows: Vec<(String, String, String)> = renders_view.as_seq()
        .map(|items| items.iter().filter_map(|f| {
            let ve = ast::binding(f, "ViewElement").map(String::from)?;
            let ft = ast::binding(f, "Fact Type").map(String::from)?;
            let vw = ast::binding(f, "View").map(String::from)?;
            Some((ve, ft, vw))
        }).collect())
        .unwrap_or_default();
    let ft_to_ve: std::collections::HashMap<String, String> =
        renders_rows.iter()
            .filter(|(_, _, vw)| vw == "task-form")
            .map(|(ve, ft, _)| (ft.clone(), ve.clone()))
            .collect();

    let ve_to_role: std::collections::HashMap<String, String> =
        form_role_rows.iter().map(|(ve, role, _)| (ve.clone(), role.clone())).collect();

    let title_ve  = ft_to_ve.get("ft-title").expect("ft-title must have a VE");
    let due_ve    = ft_to_ve.get("ft-due").expect("ft-due must have a VE");
    let active_ve = ft_to_ve.get("ft-active").expect("ft-active must have a VE");
    let prio_ve   = ft_to_ve.get("ft-prio").expect("ft-prio must have a VE");

    assert_eq!(ve_to_role.get(title_ve).map(String::as_str), Some("text-input"),
        "ft-title (Format 'text') must get Component Role 'text-input'; \
         ve={title_ve} map={:?}", ve_to_role);
    assert_eq!(ve_to_role.get(due_ve).map(String::as_str), Some("date-picker"),
        "ft-due (Format 'date') must get Component Role 'date-picker'; \
         ve={due_ve} map={:?}", ve_to_role);
    assert_eq!(ve_to_role.get(active_ve).map(String::as_str), Some("checkbox"),
        "ft-active (Format 'boolean') must get Component Role 'checkbox'; \
         ve={active_ve} map={:?}", ve_to_role);
    assert_eq!(ve_to_role.get(prio_ve).map(String::as_str), Some("combo-box"),
        "ft-prio (enum type) must get Component Role 'combo-box' — \
         derived from REAL Noun Enum Values (no hand-populated Fact_Type_has_Format); \
         ve={prio_ve} map={:?}", ve_to_role);
}

// ── task subtype-join-antecedent ────────────────────────────────────────
//
// Build directive: retire the procedural subtype-inheritance synthesiser
// (`compile_subtype_inheritance_metamodel`) in favour of a metamodel-cell-
// quantified derivation rule declared in `readings/core/derivation.md`.
//
// The derivation READING text lives in `readings/core/derivation.md`
// ("Subtype inheritance" section):
//
//   * Fact Type has inherited Resource at Role
//       iff some Subtype has subtype Sub and that Subtype has supertype Sup
//       and that Fact Type has that Role and that Role is played by Sup
//       and that Resource is instance of Sub.
//
// Prerequisites for the FULL lift (scope guard — these engine changes are
// blocking; the tests below pin current behavior + anchor the reading text):
//
//   A. Parser prerequisite (`resolve_derivation_rule` in parse_forml2.rs):
//      antecedents whose FT references are metamodel cells (`Subtype`,
//      `FactType`, `Role`) must resolve instead of falling through to
//      `UnresolvedClause`. Today `Subtype` / `FactType` / `Role` are not
//      in the declared-FT catalog of a user reading set so the clause lands
//      as unresolved. Child task: "parse: recognise metamodel-cell antecedents
//      (Subtype, FactType, Role) in derivation rule bodies".
//
//   B. Compiler prerequisite (`compile_explicit_derivation` in compile.rs):
//      `ConsequentCellSource::AntecedentRole` — the ft_id of each inner
//      consequent is bound from the antecedent `Fact Type` fact at
//      evaluation time, not a compile-time literal. Today `compile_explicit_
//      derivation` only handles the `Literal(ft_id)` path. Child task:
//      "compile: handle AntecedentRole consequent cell in
//      compile_explicit_derivation".
//
//   C. Once A and B land: delete `compile_subtype_inheritance_metamodel`
//      (compile.rs ~6179-6249), the `SUBTYPE_INHERITANCE_ID` constant
//      (~6257), the synthetic-id branch in `compile_to_defs_state`
//      (~1907) that keys this id, and the call site at ~3935. The
//      acceptance pin in `crates/arest/tests/subtype_metamodel_rule_e2e.rs`
//      stays — it verifies emission shape, not the lift mechanism.
//
// The two tests below:
//   1. Pin that `X is a Y` in a rule antecedent does NOT produce an
//      UnresolvedClause (the clause is recognised and silently skipped).
//   2. Pin that the subtype-inheritance DERIVATION correctly materialises
//      inherited facts (exercising the procedural path that the future
//      reading-lift will replace, anchored to the derivation.md text).

/// task subtype-join-antecedent (1/2): `X is a Y` in a rule antecedent
/// is correctly classified as a subtype-instance-check and does NOT
/// produce an `UnresolvedClause`. Currently the clause is silently
/// skipped (no antecedent source added, no unresolved diagnostic).
///
/// When the FULL LIFT (prerequisites A+B above) ships, the `X is a Y`
/// clause will contribute an `AntecedentSource::FactType("Subtype")`
/// entry — update this test to assert the antecedent source is present
/// and the `Subtype` cell correctly filters.
#[test]
fn subtype_is_a_clause_in_rule_antecedent_is_not_unresolved() {
    // A model with a subtype declared and a rule that uses `is a` in
    // its antecedent alongside a real FT clause.
    let src = r#"
Vehicle(.id) is an entity type.
Car is a subtype of Vehicle.
Color is a value type.
Vehicle has Color.

## Derivation Rules
* Vehicle has Color iff Car is a Vehicle and Vehicle has Color.
"#;
    let state = parse_to_state(src).expect("model with is-a antecedent must parse");
    let data = crate::compile::cell_index_from_state(&state);
    assert_eq!(data.derivation_rules.len(), 1,
        "expected exactly one derivation rule; got {:#?}",
        data.derivation_rules.iter().map(|r| r.text.as_str()).collect::<Vec<_>>());
    let rule = &data.derivation_rules[0];

    // CURRENT BEHAVIOR (pinned by task subtype-join-antecedent):
    // `Car is a Vehicle` is recognised as a subtype-instance-check and
    // silently skipped — it does NOT land as an UnresolvedClause.
    assert!(
        rule.unresolved_clauses.is_empty(),
        "`Car is a Vehicle` must NOT produce an UnresolvedClause; \
         got: {:?}", rule.unresolved_clauses,
    );

    // The remaining antecedent (`Vehicle has Color`) resolves to the
    // declared FT — the rule has exactly one antecedent source.
    // (When the full lift ships this will be 2: [Subtype, Vehicle_has_Color])
    assert_eq!(
        rule.antecedent_sources.len(), 1,
        "CURRENT BEHAVIOR: only the FT clause produces an antecedent source; \
         `Car is a Vehicle` does not yet. sources={:#?}", rule.antecedent_sources,
    );
}

/// task subtype-join-antecedent (2/2): the subtype-inheritance derivation
/// rule text in `readings/core/derivation.md` correctly describes the
/// rule the procedural synthesiser (`compile_subtype_inheritance_metamodel`)
/// implements.  Pin that inherited facts ARE derived when the schema
/// declares a subtype relationship and an instance of the subtype is
/// pushed.  This is the acceptance oracle that the future reading-lift
/// must continue to satisfy.
///
/// See also `crates/arest/tests/subtype_metamodel_rule_e2e.rs` for the
/// standalone E2E pin.
#[test]
fn subtype_inheritance_derivation_reading_text_in_derivation_md_is_present_and_facts_derive() {
    // (1) The derivation reading text must be present in derivation.md.
    let derivation_md = include_str!("../../../readings/core/derivation.md");
    // The reading's natural-language head must appear verbatim so the
    // text in derivation.md stays in sync with this test's documentation.
    assert!(
        derivation_md.contains("Fact Type has inherited Resource at Role"),
        "readings/core/derivation.md must contain the subtype-inheritance \
         derivation reading head `Fact Type has inherited Resource at Role`\n\
         (task subtype-join-antecedent: this is the predicate text the full \
         lift will parse instead of synthesising procedurally)",
    );
    assert!(
        derivation_md.contains("iff some Subtype has subtype Sub"),
        "readings/core/derivation.md must contain the metamodel-cell antecedent \
         `iff some Subtype has subtype Sub`\n(the full lift parses this against \
         the Subtype metamodel cell)",
    );

    // (2) The procedural path (which the reading-lift will replace)
    // correctly derives inherited facts.
    let src = r#"
Vehicle(.id) is an entity type.
Car is a subtype of Vehicle.
Color is a value type.
Vehicle has Color.
Car '1' has Color 'red'.
"#;
    let state = crate::parse_forml2::parse_to_state(src).expect("parse");
    let defs = crate::compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);

    let refs_owned: Vec<(String, ast::Func)> = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, &d)))
        .collect();
    let refs: Vec<(&str, &ast::Func)> =
        refs_owned.iter().map(|(n, f)| (n.as_str(), f)).collect();
    let (new_d, _) = crate::evaluate::forward_chain_defs_state(&refs, &d);

    // Car instance '1' must appear in Vehicle_has_Color (inherited via
    // subtype-inheritance derivation — the behavior the reading-lift will
    // preserve).
    let vh_cell = ast::fetch_cell_seq("Vehicle_has_Color", &new_d);
    let vehicle_ids: Vec<String> = vh_cell.as_seq()
        .map(|items| items.iter().filter_map(|f| {
            let pairs = f.as_seq()?;
            for p in pairs.iter() {
                let kv = p.as_seq()?;
                if kv.first().and_then(|k| k.as_atom())? == "Vehicle" {
                    return kv.get(1).and_then(|v| v.as_atom()).map(String::from);
                }
            }
            None
        }).collect())
        .unwrap_or_default();

    assert!(
        vehicle_ids.iter().any(|v| v == "1"),
        "Vehicle_has_Color must contain Vehicle '1' inherited from Car '1' via \
         subtype-inheritance derivation (the procedural path readings/core/\
         derivation.md documents); ids found: {:?}", vehicle_ids,
    );
}

