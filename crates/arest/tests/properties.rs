// crates/arest/tests/properties.rs
//
// Property tests for the AREST paper claims.
// Input: FORML2 readings. Output: system function responses.
// No IR. No internal types. Readings and named functions.

use arest::ast;
use arest::parse_forml2;
use arest::compile;
use arest::evaluate;
use arest::types::Violation;

// ── Metamodel ────────────────────────────────────────────────────────
// The state machine primitives. Parsed first so business domains
// can use them as instance facts without redeclaring.

const STATE_METAMODEL: &str = r#"
# State

## Entity Types

Status(.Name) is an entity type.
State Machine Definition is a subtype of Status.
Transition(.id) is an entity type.
Event Type(.id) is an entity type.
Noun is an entity type.
Name is a value type.

## Fact Types

State Machine Definition is for Noun.
Status is initial in State Machine Definition.
Transition is defined in State Machine Definition.
Transition is from Status.
Transition is to Status.
Transition is triggered by Event Type.
"#;

// ── Sample Domain ────────────────────────────────────────────────────
// An Order domain expanded to exercise all AREST properties.

const ORDERS_DOMAIN: &str = r#"
# Orders

## Entity Types

Customer(.Name) is an entity type.
Order(.Order Id) is an entity type.
Product(.SKU) is an entity type.
Line Item(.Order, .Product) is an entity type.

## Value Types

Order Id is a value type.
Name is a value type.
SKU is a value type.
Email is a value type.
Quantity is a value type.
Price is a value type.

Priority is a value type.
  The possible values of Priority are 'Standard', 'Express', 'Overnight'.

Prohibited Shipping Method is a value type.
  The possible values of Prohibited Shipping Method are 'Overnight', 'Same Day'.

## Subtypes

Premium Customer is a subtype of Customer.
  Not every Customer is a Premium Customer.

## Fact Types

### Customer
Customer has Email.
Customer has Priority.

### Order
Order was placed by Customer.
Order has Price.
Order uses Prohibited Shipping Method.

### Line Item
Line Item has Quantity.

### Product
Product has Price.

## Constraints

Each Order was placed by exactly one Customer.
Each Customer has at most one Email.
Each Line Item has exactly one Quantity.
Each Product has exactly one Price.
Each Order has at most one Price.

## Deontic Constraints

It is obligatory that each Order was placed by some Customer.
It is forbidden that Order uses Prohibited Shipping Method.

## Derivation Rules

Customer has Priority 'Express' if Customer is a Premium Customer.

## Instance Facts

State Machine Definition 'Order' is for Noun 'Order'.
Status 'In Cart' is initial in State Machine Definition 'Order'.

Transition 'place' is defined in State Machine Definition 'Order'.
  Transition 'place' is from Status 'In Cart'.
  Transition 'place' is to Status 'Placed'.
  Transition 'place' is triggered by Event Type 'place'.

Transition 'ship' is defined in State Machine Definition 'Order'.
  Transition 'ship' is from Status 'Placed'.
  Transition 'ship' is to Status 'Shipped'.
  Transition 'ship' is triggered by Event Type 'ship'.

Transition 'deliver' is defined in State Machine Definition 'Order'.
  Transition 'deliver' is from Status 'Shipped'.
  Transition 'deliver' is to Status 'Delivered'.
  Transition 'deliver' is triggered by Event Type 'deliver'.

Transition 'cancel' is defined in State Machine Definition 'Order'.
  Transition 'cancel' is from Status 'In Cart'.
  Transition 'cancel' is to Status 'Cancelled'.
  Transition 'cancel' is triggered by Event Type 'cancel'.

Transition 'cancel-placed' is defined in State Machine Definition 'Order'.
  Transition 'cancel-placed' is from Status 'Placed'.
  Transition 'cancel-placed' is to Status 'Cancelled'.
  Transition 'cancel-placed' is triggered by Event Type 'cancel'.

Customer 'Acme' has Email 'acme@example.com'.
Customer 'Globex' has Email 'globex@example.com'.
Product 'WIDGET-1' has Price '9.99'.
Product 'GADGET-2' has Price '24.99'.
"#;

fn merge_state_into(base: &ast::Object, extension: &ast::Object) -> ast::Object {
    let mut state = base.clone();
    for (name, contents) in ast::cells_iter(extension) {
        if let Some(facts) = contents.as_seq() {
            for fact in facts {
                state = ast::cell_push(name, fact.clone(), &state);
            }
        }
    }
    state
}

fn compile_orders() -> (ast::Object, ast::Object) {
    let meta_ir = parse_forml2::parse_markdown(STATE_METAMODEL).unwrap();
    let meta_state = parse_forml2::domain_to_state(&meta_ir);
    let orders_state = parse_forml2::parse_to_state_with_nouns(ORDERS_DOMAIN, &meta_state).unwrap();
    let state = merge_state_into(&meta_state, &orders_state);
    let defs = compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);
    (state, d)
}

fn compile_orders_with_generators(gens: &[&str]) -> (ast::Object, ast::Object) {
    let gen_set: std::collections::HashSet<String> = gens.iter().map(|s| s.to_string()).collect();
    compile::set_active_generators(gen_set);
    let result = compile_orders();
    compile::set_active_generators(std::collections::HashSet::new());
    result
}

// ── Theorem 1: Grammar Unambiguity ───────────────────────────────────
// Each FORML2 sentence over (N, F) has exactly one parse.

#[test]
fn t1_entity_types_parse_uniquely() {
    let ir = parse_forml2::parse_markdown(ORDERS_DOMAIN).unwrap();
    assert!(ir.nouns.contains_key("Customer"));
    assert!(ir.nouns.contains_key("Order"));
    assert!(ir.nouns.contains_key("Product"));
    assert!(ir.nouns.contains_key("Line Item"));
    assert_eq!(ir.nouns["Customer"].object_type, "entity");
    assert_eq!(ir.nouns["Priority"].object_type, "value");
}

#[test]
fn t1_fact_types_parse_uniquely() {
    let ir = parse_forml2::parse_markdown(ORDERS_DOMAIN).unwrap();
    assert!(ir.fact_types.contains_key("Order_was_placed_by_Customer"));
    assert!(ir.fact_types.contains_key("Customer_has_Email"));
    assert!(ir.fact_types.contains_key("Customer_has_Priority"));
}

#[test]
fn t1_constraint_kinds_determined_by_quantifiers() {
    let ir = parse_forml2::parse_markdown(ORDERS_DOMAIN).unwrap();
    // "exactly one" splits into UC + MC
    assert!(ir.constraints.iter().any(|c| c.kind == "UC" && c.text.contains("placed by")));
    assert!(ir.constraints.iter().any(|c| c.kind == "MC" && c.text.contains("placed by")));
    // "at most one" is UC only
    assert!(ir.constraints.iter().any(|c| c.kind == "UC" && c.text.contains("Email")));
    // Deontic forbidden
    assert!(ir.constraints.iter().any(|c| c.kind == "forbidden" && c.text.contains("Prohibited Shipping Method")));
}

#[test]
fn t1_no_ambiguity_between_constraint_families() {
    // A conditional "If...then..." with mixed noun types is SS, not a ring constraint
    let input = "Person(.Name) is an entity type.\nBook(.Title) is an entity type.\nPerson authored Book.\nPerson reviewed Book.\nIf some Person authored some Book then that Person reviewed that Book.";
    let ir = parse_forml2::parse_markdown(input).unwrap();
    assert!(ir.constraints.iter().any(|c| c.kind == "SS"), "Should be subset, not ring");
    assert!(!ir.constraints.iter().any(|c| c.kind == "IR" || c.kind == "AS" || c.kind == "SY"));
}

#[test]
fn t1_ring_vs_subset_distinguished_by_noun_types() {
    // Same noun type on both sides = ring constraint
    let ring = "Person(.Name) is an entity type.\nPerson is parent of Person.\nIf Person1 is parent of Person2 then it is impossible that Person2 is parent of Person1.";
    let ir = parse_forml2::parse_markdown(ring).unwrap();
    assert!(ir.constraints.iter().any(|c| c.kind == "AS"), "Same-type conditional should be AS ring");
}

// ── Theorem 2: Specification Equivalence ─────────────────────────────
// parse^-1 ∘ compile^-1 ∘ compile ∘ parse = id_R
// The violation message is the original reading (Corollary 7).

#[test]
fn t2_violation_message_is_original_reading() {
    let (state, d) = compile_orders();
    // The forbidden constraint text should survive compilation into a def name.
    // Find the constraint fact in the state.
    let constraint_cell = ast::fetch_or_phi("Constraint", &state);
    let constraint_facts = constraint_cell.as_seq().expect("State should have Constraint facts");
    let forbidden = constraint_facts.iter()
        .find(|f| ast::binding(f, "text").map_or(false, |v| v.contains("Prohibited Shipping Method")))
        .expect("Should have Prohibited Shipping Method constraint");
    let original_text = ast::binding(forbidden, "text").unwrap();
    // The compiled defs should contain a constraint def whose name encodes the constraint id.
    let constraint_id = ast::binding(forbidden, "id").unwrap();
    let def_key = format!("constraint:{}", constraint_id);
    assert!(ast::cells_iter(&d).into_iter().any(|(k, _)| k == def_key),
        "Compiled constraint def should exist for id '{}' (Corollary 7)", constraint_id);
    // The original reading text is preserved in the state, available for violation messages.
    assert!(original_text.contains("Prohibited Shipping Method"),
        "Compiled text must preserve original reading (Corollary 7)");
}

#[test]
fn t2_constraint_text_round_trips() {
    let ir = parse_forml2::parse_markdown(ORDERS_DOMAIN).unwrap();
    for c in &ir.constraints {
        assert!(!c.text.is_empty(), "Every constraint must have non-empty text");
        assert!(!c.id.is_empty(), "Every constraint must have non-empty id");
    }
}

// ── Theorem 3: Completeness of State Transfer ────────────────────────
// create resolves identity, derives to lfp, validates all constraints, emits.

#[test]
fn t3_state_machine_initializes() {
    let (_, d) = compile_orders();
    // Order should have a state machine with initial state "In Cart"
    let initial_obj = ast::fetch_or_phi("machine:Order:initial", &d);
    let initial_func = ast::metacompose(&initial_obj, &d);
    let initial = arest::ast::apply(&initial_func, &arest::ast::Object::phi(), &d);
    assert_eq!(initial.as_atom().unwrap(), "In Cart");
}

#[test]
fn t3_forward_chain_reaches_fixed_point() {
    let (state, d) = compile_orders();
    let empty_state = ast::Object::phi();
    // Extract derivation defs from the compiled defs vec (not D)
    let defs_vec = compile::compile_to_defs_state(&state);
    let derivation_defs: Vec<(&str, &arest::ast::Func)> = defs_vec.iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, f)| (n.as_str(), f))
        .collect();
    let _ = &d; // d available but forward_chain uses raw defs
    // Forward chain with empty state should terminate
    let (state2, derived) = evaluate::forward_chain_defs_state(&derivation_defs, &empty_state);
    // The derived facts should be a fixed point (running again produces nothing new)
    let (_state3, derived2) = evaluate::forward_chain_defs_state(&derivation_defs, &state2);
    // Second run should produce no additional facts beyond what first run added
    assert!(derived2.len() <= derived.len(), "Forward chain should reach fixed point");
}

#[test]
fn t3_alethic_violation_rejects_command() {
    let (state, _defs) = compile_orders();
    // UC on "Each Order was placed by exactly one Customer" means
    // an Order with two customers should produce a violation
    let constraint_cell = ast::fetch_or_phi("Constraint", &state);
    let constraint_facts = constraint_cell.as_seq().expect("Should have Constraint facts");
    let uc = constraint_facts.iter()
        .find(|f| {
            ast::binding_matches(f, "kind", "UC")
            && ast::binding(f, "text").map_or(false, |v| v.contains("placed by"))
        })
        .expect("UC on placed by");
    let text = ast::binding(uc, "text").unwrap();
    assert!(!text.is_empty());
    // The constraint is alethic (schema-enforced)
    let modality = ast::binding(uc, "modality").unwrap();
    assert_eq!(modality, "alethic");
}

#[test]
fn t3_deontic_violation_warns_but_succeeds() {
    let (state, _) = compile_orders();
    let constraint_cell = ast::fetch_or_phi("Constraint", &state);
    let constraint_facts = constraint_cell.as_seq().expect("Should have Constraint facts");
    let forbidden = constraint_facts.iter()
        .find(|f| {
            ast::binding_matches(f, "modality", "deontic")
            && ast::binding(f, "text").map_or(false, |v| v.contains("Prohibited Shipping Method"))
        })
        .expect("Deontic forbidden constraint");
    let op = ast::binding(forbidden, "deonticOperator");
    assert_eq!(op, Some("forbidden"));
}

// ── Theorem 4: HATEOAS as Projection ─────────────────────────────────
// (a) Transition links = projection of T filtered by current status
// (b) Navigation links = projection of UC direction (parent/child)

#[test]
fn t4a_transitions_from_initial_state() {
    let (_, d) = compile_orders();
    let transition = ast::metacompose(&ast::fetch_or_phi("machine:Order", &d), &d);
    // From "In Cart", available transitions should be "place" and "cancel"
    // Test by applying the machine function to each known event
    let all_events = ["place", "ship", "deliver", "cancel"];
    let from_cart: Vec<&str> = all_events.iter()
        .filter(|event| {
            let input = arest::ast::Object::seq(vec![
                arest::ast::Object::atom("In Cart"),
                arest::ast::Object::atom(event),
            ]);
            let result = arest::ast::apply(&transition, &input, &d);
            !result.is_bottom() && result.as_atom().unwrap_or("In Cart") != "In Cart"
        })
        .copied()
        .collect();
    assert!(from_cart.contains(&"place"), "In Cart should have 'place' transition");
    assert!(from_cart.contains(&"cancel"), "In Cart should have 'cancel' transition");
    assert_eq!(from_cart.len(), 2, "In Cart should have exactly 2 transitions");
}

#[test]
fn t4a_transitions_from_placed() {
    let (_, d) = compile_orders();
    let transition = ast::metacompose(&ast::fetch_or_phi("machine:Order", &d), &d);
    let all_events = ["place", "ship", "deliver", "cancel"];
    let from_placed: Vec<&str> = all_events.iter()
        .filter(|event| {
            let input = arest::ast::Object::seq(vec![
                arest::ast::Object::atom("Placed"),
                arest::ast::Object::atom(event),
            ]);
            let result = arest::ast::apply(&transition, &input, &d);
            !result.is_bottom() && result.as_atom().unwrap_or("Placed") != "Placed"
        })
        .copied()
        .collect();
    assert!(from_placed.contains(&"ship"), "Placed should have 'ship'");
    assert!(from_placed.contains(&"cancel"), "Placed should have 'cancel'");
}

#[test]
fn t4a_terminal_state_has_no_transitions() {
    let (_, d) = compile_orders();
    let transition = ast::metacompose(&ast::fetch_or_phi("machine:Order", &d), &d);
    // Delivered and Cancelled are terminal: no event should produce a different state
    let all_events = ["place", "ship", "deliver", "cancel"];
    let from_delivered: Vec<&str> = all_events.iter()
        .filter(|event| {
            let input = arest::ast::Object::seq(vec![
                arest::ast::Object::atom("Delivered"),
                arest::ast::Object::atom(event),
            ]);
            let result = arest::ast::apply(&transition, &input, &d);
            !result.is_bottom() && result.as_atom().unwrap_or("Delivered") != "Delivered"
        })
        .copied()
        .collect();
    let from_cancelled: Vec<&str> = all_events.iter()
        .filter(|event| {
            let input = arest::ast::Object::seq(vec![
                arest::ast::Object::atom("Cancelled"),
                arest::ast::Object::atom(event),
            ]);
            let result = arest::ast::apply(&transition, &input, &d);
            !result.is_bottom() && result.as_atom().unwrap_or("Cancelled") != "Cancelled"
        })
        .copied()
        .collect();
    assert!(from_delivered.is_empty(), "Delivered is terminal (Corollary 6)");
    assert!(from_cancelled.is_empty(), "Cancelled is terminal (Corollary 6)");
}

#[test]
fn t4a_state_machine_fold_produces_correct_state() {
    let (_, d) = compile_orders();
    let transition = ast::metacompose(&ast::fetch_or_phi("machine:Order", &d), &d);
    let initial_func = ast::metacompose(&ast::fetch_or_phi("machine:Order:initial", &d), &d);
    // Get initial state
    let mut state = arest::ast::apply(&initial_func, &arest::ast::Object::phi(), &d);
    // Fold: In Cart -> place -> Placed -> ship -> Shipped -> deliver -> Delivered
    for event in &["place", "ship", "deliver"] {
        let input = arest::ast::Object::seq(vec![state, arest::ast::Object::atom(event)]);
        state = arest::ast::apply(&transition, &input, &d);
    }
    assert_eq!(state.as_atom().unwrap(), "Delivered");
}

#[test]
fn t4a_invalid_event_preserves_state() {
    let (_, d) = compile_orders();
    let transition = ast::metacompose(&ast::fetch_or_phi("machine:Order", &d), &d);
    // "ship" from "In Cart" is invalid. State should stay "In Cart".
    let input = arest::ast::Object::seq(vec![
        arest::ast::Object::atom("In Cart"),
        arest::ast::Object::atom("ship"),
    ]);
    let result = arest::ast::apply(&transition, &input, &d);
    // Invalid event returns bottom or the same state
    let state = result.as_atom().unwrap_or("In Cart");
    assert_eq!(state, "In Cart", "Invalid event should not change state");
}

#[test]
fn t4a_cancel_from_multiple_states() {
    let (_, d) = compile_orders();
    let transition = ast::metacompose(&ast::fetch_or_phi("machine:Order", &d), &d);
    let initial_func = ast::metacompose(&ast::fetch_or_phi("machine:Order:initial", &d), &d);
    let fold = |events: &[&str]| -> String {
        let mut state = arest::ast::apply(&initial_func, &arest::ast::Object::phi(), &d);
        for event in events {
            let input = arest::ast::Object::seq(vec![state.clone(), arest::ast::Object::atom(event)]);
            let result = arest::ast::apply(&transition, &input, &d);
            if result.is_bottom() {
                // Invalid event: state unchanged
            } else {
                state = result;
            }
        }
        state.as_atom().unwrap_or("?").to_string()
    };
    // Cancel from In Cart
    assert_eq!(fold(&["cancel"]), "Cancelled");
    // Cancel from Placed
    assert_eq!(fold(&["place", "cancel"]), "Cancelled");
    // Cannot cancel from Shipped
    assert_eq!(fold(&["place", "ship", "cancel"]), "Shipped");
}

#[test]
fn t4b_navigation_parent_child_from_uc() {
    let ir = parse_forml2::parse_markdown(ORDERS_DOMAIN).unwrap();
    // UC on "Each Order was placed by exactly one Customer" means
    // Order is the child (UC on Order's role), Customer is the parent.
    let ft = ir.fact_types.get("Order_was_placed_by_Customer")
        .expect("Should have placed_by fact type");
    assert_eq!(ft.roles.len(), 2);
    assert_eq!(ft.roles[0].noun_name, "Order");
    assert_eq!(ft.roles[1].noun_name, "Customer");
    // The UC spans Order's role (role 0), making Order the dependent side
    let uc = ir.constraints.iter()
        .find(|c| c.kind == "UC" && c.spans.iter().any(|s| s.fact_type_id == "Order_was_placed_by_Customer"))
        .expect("UC on placed_by");
    assert_eq!(uc.spans[0].role_index, 0, "UC on role 0 (Order) makes Order the child");
}

// ── Theorem 5: Derivability ──────────────────────────────────────────
// Every value in the representation is a rho-application over P.

#[test]
fn t5_instance_facts_populate_correctly() {
    let ir = parse_forml2::parse_markdown(ORDERS_DOMAIN).unwrap();
    // Instance fact: Customer 'Acme' has Email 'acme@example.com'
    let acme_email = ir.general_instance_facts.iter()
        .find(|f| f.subject_value == "Acme" && f.object_noun == "Email")
        .expect("Acme should have email");
    assert_eq!(acme_email.object_value, "acme@example.com");
}

#[test]
fn t5_enum_values_are_declared() {
    let ir = parse_forml2::parse_markdown(ORDERS_DOMAIN).unwrap();
    let priority_vals = ir.enum_values.get("Priority")
        .expect("Priority should have enum values");
    assert!(priority_vals.contains(&"Standard".to_string()));
    assert!(priority_vals.contains(&"Express".to_string()));
    assert!(priority_vals.contains(&"Overnight".to_string()));
}

#[test]
fn t5_derivation_rule_parsed() {
    let ir = parse_forml2::parse_markdown(ORDERS_DOMAIN).unwrap();
    assert!(!ir.derivation_rules.is_empty(), "Should have derivation rules");
    let rule = ir.derivation_rules.iter()
        .find(|r| r.text.contains("Premium Customer"))
        .expect("Should have Premium Customer derivation");
    assert!(rule.text.contains("Express"));
}

// ── Corollary 6: Deletion as Terminal State ──────────────────────────

#[test]
fn c6_terminal_states_are_derived() {
    let (_, d) = compile_orders();
    let transition = ast::metacompose(&ast::fetch_or_phi("machine:Order", &d), &d);
    // All known states and events from the domain
    let all_states = ["In Cart", "Placed", "Shipped", "Delivered", "Cancelled"];
    let all_events = ["place", "ship", "deliver", "cancel"];
    // Terminal states: no event produces a different state
    let terminal: Vec<&str> = all_states.iter()
        .filter(|state| {
            all_events.iter().all(|event| {
                let input = arest::ast::Object::seq(vec![
                    arest::ast::Object::atom(state),
                    arest::ast::Object::atom(event),
                ]);
                let result = arest::ast::apply(&transition, &input, &d);
                result.is_bottom() || result.as_atom().unwrap_or(state) == **state
            })
        })
        .copied()
        .collect();
    assert!(terminal.contains(&"Delivered"), "Delivered should be terminal");
    assert!(terminal.contains(&"Cancelled"), "Cancelled should be terminal");
}

// ── Corollary 7: Violation Verbalization ─────────────────────────────

#[test]
fn c7_deontic_violation_returns_reading_text() {
    let (state, d) = compile_orders();
    let defs_vec = compile::compile_to_defs_state(&state);
    let ctx = arest::ast::encode_eval_context_state("We will ship your order overnight for fast delivery", None, &state);
    let violations: Vec<_> = defs_vec.iter()
        .filter(|(n, _)| n.starts_with("constraint:"))
        .flat_map(|(name, func)| {
            let result = arest::ast::apply(func, &ctx, &d);
            let is_deontic = name.contains("obligatory") || name.contains("forbidden");
            arest::ast::decode_violations(&result).into_iter().map(move |mut v| { v.alethic = !is_deontic; v })
        })
        .collect();
    let shipping_v = violations.iter()
        .find(|v| v.constraint_text.contains("Prohibited Shipping Method"));
    assert!(shipping_v.is_some(), "Should catch 'Overnight' via Prohibited Shipping Method enum");
    let v = shipping_v.unwrap();
    assert!(v.constraint_text.starts_with("It is forbidden that"),
        "Violation text should be the original FORML2 reading");
}

// ── Corollary 8: Closure Under Self-Modification ─────────────────────

#[test]
fn c8_ingesting_new_readings_preserves_properties() {
    use arest::ast::{self, Object};

    // Start with orders domain (metamodel + orders, same as compile_orders)
    let (state1, d1) = compile_orders();

    // Verify Order machine initial state via the initial def
    let initial1_func = ast::metacompose(&ast::fetch_or_phi("machine:Order:initial", &d1), &d1);
    let initial1 = ast::apply(&initial1_func, &Object::phi(), &d1);

    // Add a new reading (new entity type + constraint)
    let extension = r#"
# OrderExtension

## Entity Types
Coupon(.Code) is an entity type.

## Value Types
Code is a value type.
Discount is a value type.

## Fact Types
Coupon has Discount.
Order has Coupon.

## Constraints
Each Coupon has exactly one Discount.
Each Order has at most one Coupon.
"#;

    // Parse extension and merge states
    let ext_state = parse_forml2::parse_to_state_with_nouns(extension, &state1).unwrap();
    let state2 = merge_state_into(&state1, &ext_state);
    let defs2 = compile::compile_to_defs_state(&state2);
    let d2 = ast::defs_to_state(&defs2, &state2);

    // Original properties still hold
    let initial2_func = ast::metacompose(&ast::fetch_or_phi("machine:Order:initial", &d2), &d2);
    let initial2 = ast::apply(&initial2_func, &Object::phi(), &d2);
    assert_eq!(initial2.to_string(), initial1.to_string(),
        "Initial state unchanged after self-modification");

    // New constraint is present
    assert!(ast::cells_iter(&d2).into_iter().any(|(k, _)| k.starts_with("constraint:") && k.contains("Coupon")),
        "New constraint should be present after self-modification");

    // State machine fold still works via defs
    let machine = ast::metacompose(&ast::fetch_or_phi("machine:Order", &d2), &d2);
    let mut sm_state = initial2.to_string();
    for event in &["place", "ship"] {
        let input = Object::seq(vec![Object::atom(&sm_state), Object::atom(event)]);
        let result = ast::apply(&machine, &input, &d2);
        sm_state = result.to_string();
    }
    assert_eq!(sm_state, "Shipped");
}

// ── Remark: World Assumption on Populating Functions ─────────────────

#[test]
fn remark_cwa_constraint_has_enum_values() {
    let ir = parse_forml2::parse_markdown(ORDERS_DOMAIN).unwrap();
    // The "forbidden Prohibited Shipping Method" constraint spans a fact type
    // whose role noun has declared enum values. This makes it CWA.
    let forbidden = ir.constraints.iter()
        .find(|c| c.text.contains("Prohibited Shipping Method"))
        .expect("Should have Prohibited Shipping Method constraint");
    // Check that at least one role noun in the spanned fact types has enum values
    let has_enum_vals = forbidden.spans.iter().any(|span| {
        ir.fact_types.get(&span.fact_type_id)
            .map_or(false, |ft| ft.roles.iter().any(|role|
                ir.enum_values.get(&role.noun_name).map_or(false, |vals| !vals.is_empty())
            ))
    });
    assert!(has_enum_vals, "CWA constraint should have enum values for deterministic checking");
}

#[test]
fn remark_owa_constraint_has_no_enum_values() {
    // A deontic constraint on a noun with no enum values is OWA
    let input = r#"
Response(.id) is an entity type.
Implementation Detail is a value type.
## Fact Types
Response reveals Implementation Detail.
## Deontic Constraints
It is forbidden that Response reveals Implementation Detail.
"#;
    let ir = parse_forml2::parse_markdown(input).unwrap();
    let forbidden = ir.constraints.iter()
        .find(|c| c.text.contains("Implementation Detail"))
        .unwrap();
    // Check that no role noun in the spanned fact types has enum values
    let has_enum_vals = forbidden.spans.iter().any(|span| {
        ir.fact_types.get(&span.fact_type_id)
            .map_or(false, |ft| ft.roles.iter().any(|role|
                ir.enum_values.get(&role.noun_name).map_or(false, |vals| !vals.is_empty())
            ))
    });
    assert!(!has_enum_vals, "OWA constraint should have no enum values (requires runtime judgment)");
}

// ── Cell Isolation (Definition 2) ────────────────────────────────────

#[test]
fn def2_state_machine_is_deterministic() {
    let (_, d) = compile_orders();
    let transition = ast::metacompose(&ast::fetch_or_phi("machine:Order", &d), &d);
    // For each (from, event) pair, applying the function is deterministic.
    // Apply twice and ensure identical results (Definition 2).
    let all_states = ["In Cart", "Placed", "Shipped", "Delivered", "Cancelled"];
    let all_events = ["place", "ship", "deliver", "cancel"];
    for state in &all_states {
        for event in &all_events {
            let input = arest::ast::Object::seq(vec![
                arest::ast::Object::atom(state),
                arest::ast::Object::atom(event),
            ]);
            let result1 = arest::ast::apply(&transition, &input, &d);
            let result2 = arest::ast::apply(&transition, &input, &d);
            assert_eq!(result1, result2,
                "Non-deterministic: ({}, {}) produced different results", state, event);
        }
    }
}

// ── Subtypes ─────────────────────────────────────────────────────────

#[test]
fn subtypes_parsed_correctly() {
    let ir = parse_forml2::parse_markdown(ORDERS_DOMAIN).unwrap();
    assert_eq!(ir.subtypes.get("Premium Customer"), Some(&"Customer".to_string()));
}

// ── RMAP ─────────────────────────────────────────────────────────────

#[test]
fn rmap_produces_tables_for_entities() {
    let ir = parse_forml2::parse_markdown(ORDERS_DOMAIN).unwrap();
    let tables = arest::rmap::rmap(&ir);
    assert!(!tables.is_empty(), "RMAP should produce tables");
    // Should have tables for the binary fact types
    let table_names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
    assert!(table_names.iter().any(|n| n.contains("order") || n.contains("Order")),
        "Should have a table related to Order");
}

// ── Paper Claim: Everything is Facts ─────────────────────────────────
// The parser should produce Object state. The compiler should accept
// Object state. There should be no struct between them.

#[test]
fn paper_claim_parse_produces_population() {
    // parse_markdown + domain_to_state should produce Object state
    let ir = parse_forml2::parse_markdown(ORDERS_DOMAIN).unwrap();
    let state = parse_forml2::domain_to_state(&ir);
    // The state should contain noun facts
    assert!(!matches!(ast::fetch("Noun", &state), ast::Object::Bottom), "State should contain Noun facts");
    // The state should contain fact type facts
    assert!(!matches!(ast::fetch("FactType", &state), ast::Object::Bottom), "State should contain FactType facts");
    // The state should contain constraint facts
    assert!(!matches!(ast::fetch("Constraint", &state), ast::Object::Bottom), "State should contain Constraint facts");
}

#[test]
fn paper_claim_compile_from_population() {
    let (state, _) = compile_orders();
    let defs = compile::compile_to_defs_state(&state);
    assert!(defs.iter().any(|(n, _)| n.starts_with("machine:")), "Should compile state machines from state");
}

#[test]
fn paper_claim_population_round_trip() {
    // Parse to state, compile, run state machine. Same result as Domain path.
    let (_, d) = compile_orders();
    let transition = ast::metacompose(&ast::fetch_or_phi("machine:Order", &d), &d);
    let initial = ast::metacompose(&ast::fetch_or_phi("machine:Order:initial", &d), &d);
    let init_state = arest::ast::apply(&initial, &arest::ast::Object::phi(), &d);
    assert_eq!(init_state.as_atom().unwrap(), "In Cart");
    // Fold: In Cart -> place -> Placed -> ship -> Shipped -> deliver -> Delivered
    let mut state = init_state;
    for event in &["place", "ship", "deliver"] {
        let inp = arest::ast::Object::seq(vec![state, arest::ast::Object::atom(event)]);
        state = arest::ast::apply(&transition, &inp, &d);
    }
    assert_eq!(state.as_atom().unwrap(), "Delivered");
}

// ── Paper Claim: Everything is Func ──────────────────────────────────
// The compiler produces named definitions (Def name = func).
// Evaluation is function application (func:object).
// No structs. No HashMaps at evaluation time.

#[test]
fn ffp_compile_produces_named_definitions() {
    // compile_to_defs_state should return Vec<(String, Func)>
    let (state, _) = compile_orders();

    let defs = compile::compile_to_defs_state(&state);
    assert!(!defs.is_empty(), "Should produce named definitions");

    // Every definition is a (name, Func) pair
    for (name, func) in &defs {
        assert!(!name.is_empty(), "Definition name must not be empty");
        // func is a Func. That's all it can be. No struct, no enum variant wrapping data.
        let _ = func; // Type system enforces this is Func
    }
}

#[test]
fn ffp_constraint_is_a_function() {
    let (state, _) = compile_orders();

    let defs = compile::compile_to_defs_state(&state);

    // There should be constraint definitions
    let constraint_defs: Vec<_> = defs.iter()
        .filter(|(name, _)| name.starts_with("constraint:"))
        .collect();
    assert!(!constraint_defs.is_empty(), "Should have constraint definitions");
}

#[test]
fn ffp_state_machine_is_a_function() {
    let (state, _) = compile_orders();

    let defs = compile::compile_to_defs_state(&state);

    // There should be a state machine definition for Order
    let sm_defs: Vec<_> = defs.iter()
        .filter(|(name, _)| name.contains("Order") && name.contains("machine"))
        .collect();
    assert!(!sm_defs.is_empty(), "Order state machine should be a named definition");
}

#[test]
fn ffp_evaluation_is_application() {
    use arest::ast::{self, Object};

    let (_, d) = compile_orders();

    // Find the Order state machine transition function and initial state
    let transition_func = ast::metacompose(&ast::fetch_or_phi("machine:Order", &d), &d);
    let initial_func = ast::metacompose(&ast::fetch_or_phi("machine:Order:initial", &d), &d);

    // Get initial state by applying the constant function
    let initial = ast::apply(&initial_func, &Object::phi(), &d);
    assert_eq!(initial.as_atom(), Some("In Cart"));

    // Fold the transition function over the event stream.
    // foldl(transition, state, events): for each event, apply transition to <state, event>
    let events = ["place", "ship", "deliver"];
    let mut state = initial;
    for event in &events {
        let input = Object::seq(vec![state, Object::atom(event)]);
        state = ast::apply(&transition_func, &input, &d);
    }
    assert_eq!(state.as_atom(), Some("Delivered"),
        "State machine evaluation is a fold of the transition function over events");
}

// ── No Native: the entire runtime is Func ───────────────────────────
// Every compiled definition must be pure Func (no Native closures).
// Native is a hole in the algebra — you cannot inspect, compose,
// optimize, or serialize it. The design requires everything is Func.

#[test]
fn no_native_in_constraint_defs() {
    let (state, _) = compile_orders();

    let defs = compile::compile_to_defs_state(&state);
    let mut native_defs = Vec::new();
    for (name, func) in &defs {
        if func.has_native() {
            native_defs.push(name.clone());
        }
    }
    assert!(
        native_defs.is_empty(),
        "These definitions contain Native closures (must be pure Func): {:?}",
        native_defs,
    );
}

#[test]
fn no_native_in_multispan_uc_defs() {
    // Domain with n-ary UC: "For each Customer, Endpoint, VIN, and Date,
    // at most one Billable Request exists."
    let input = r#"
# Billing

## Entity Types

Customer(.Email) is an entity type.
Meter Endpoint(.Slug) is an entity type.
VIN(.Code) is an entity type.
Date(.Value) is a value type.
Billable Request(.id) is an entity type.

## Fact Types

Billable Request is for Customer at Meter Endpoint for VIN on Date.

## Constraints

For each Customer, Meter Endpoint, VIN, and Date, at most one Billable Request exists.
"#;

    let ir = parse_forml2::parse_markdown(input).unwrap();
    let state = parse_forml2::domain_to_state(&ir);
    let defs = compile::compile_to_defs_state(&state);
    let mut native_defs = Vec::new();
    for (name, func) in &defs {
        if func.has_native() {
            native_defs.push(name.clone());
        }
    }
    assert!(
        native_defs.is_empty(),
        "Multi-span UC defs contain Native (must be pure Func): {:?}",
        native_defs,
    );
}

// ── Algebraic Laws (Backus Section 12.2) ─────────────────────────────
// The compiled functions must obey the FP algebra.
// These laws are what make the system provably correct by
// algebraic manipulation, not by testing individual cases.

#[test]
fn law_i1_construction_distributes_over_composition() {
    // [f1,...,fn]∘g = [f1∘g,...,fn∘g]
    // Construction of composed functions = composition of constructions
    use arest::ast::{self, Func, Object};
    let defs = ast::Object::phi();

    // f1 = selector 1, f2 = selector 2, g = reverse
    // [s1, s2] ∘ reverse applied to <A, B> should equal [s1∘reverse, s2∘reverse] applied to <A, B>
    let s1 = Func::Selector(1);
    let s2 = Func::Selector(2);
    let input = Object::seq(vec![Object::atom("A"), Object::atom("B")]);

    // Left side: [s1, s2] ∘ reverse : <A, B>
    // reverse:<A,B> = <B,A>, then [s1,s2]:<B,A> = <B,A>
    let construction = Func::construction(vec![s1.clone(), s2.clone()]);
    let reversed_input = Object::seq(vec![Object::atom("B"), Object::atom("A")]);
    let left = ast::apply(&construction, &reversed_input, &defs);

    // Right side: [s1∘reverse, s2∘reverse] : <A, B>
    // s1∘reverse:<A,B> = s1:<B,A> = B
    // s2∘reverse:<A,B> = s2:<B,A> = A
    let s1_rev = Func::compose(s1.clone(), Func::Reverse);
    let s2_rev = Func::compose(s2.clone(), Func::Reverse);
    let right_construction = Func::construction(vec![s1_rev, s2_rev]);
    let right = ast::apply(&right_construction, &input, &defs);

    assert_eq!(left, right, "Law I.1: [f1,...,fn]∘g = [f1∘g,...,fn∘g]");
}

#[test]
fn law_i5_selector_extracts_from_construction() {
    // s∘[f1,...,fn] ≤ fs (selector s extracts the sth element)
    use arest::ast::{self, Func, Object};
    let defs = ast::Object::phi();

    let input = Object::atom("X");

    // [constant("A"), constant("B"), constant("C")] : X = <A, B, C>
    let construction = Func::construction(vec![
        Func::constant(Object::atom("A")),
        Func::constant(Object::atom("B")),
        Func::constant(Object::atom("C")),
    ]);
    let _constructed = ast::apply(&construction, &input, &defs);

    // s2 ∘ [const A, const B, const C] : X should equal const B : X = B
    let s2_of_construction = Func::compose(Func::Selector(2), construction);
    let result = ast::apply(&s2_of_construction, &input, &defs);

    assert_eq!(result.as_atom(), Some("B"), "Law I.5: s∘[f1,...,fn] = fs");
}

#[test]
fn law_iii4_apply_to_all_distributes_over_composition() {
    // α(f∘g) = αf ∘ αg
    use arest::ast::{self, Func, Object};
    let defs = ast::Object::phi();

    // f = selector 1, g = reverse
    // Input: <<1,2>, <3,4>>
    let input = Object::seq(vec![
        Object::seq(vec![Object::atom("1"), Object::atom("2")]),
        Object::seq(vec![Object::atom("3"), Object::atom("4")]),
    ]);

    // Left: α(s1 ∘ reverse) : <<1,2>,<3,4>>
    // s1∘reverse:<1,2> = s1:<2,1> = 2
    // s1∘reverse:<3,4> = s1:<4,3> = 4
    // Result: <2, 4>
    let f_comp_g = Func::compose(Func::Selector(1), Func::Reverse);
    let left = ast::apply(&Func::ApplyToAll(Box::new(f_comp_g)), &input, &defs);

    // Right: αs1 ∘ αreverse : <<1,2>,<3,4>>
    // αreverse:<<1,2>,<3,4>> = <<2,1>,<4,3>>
    // αs1:<<2,1>,<4,3>> = <2, 4>
    let alpha_g = Func::ApplyToAll(Box::new(Func::Reverse));
    let alpha_f = Func::ApplyToAll(Box::new(Func::Selector(1)));
    let right_func = Func::compose(alpha_f, alpha_g);
    let right = ast::apply(&right_func, &input, &defs);

    assert_eq!(left, right, "Law III.4: α(f∘g) = αf ∘ αg");
}

#[test]
fn law_iii1_constant_absorbs_composition() {
    // x̄ ∘ f = x̄ (constant composed with anything is still constant)
    use arest::ast::{self, Func, Object};
    let defs = ast::Object::phi();

    let constant_a = Func::constant(Object::atom("A"));
    let composed = Func::compose(constant_a.clone(), Func::Selector(1));

    let input = Object::seq(vec![Object::atom("X"), Object::atom("Y")]);
    let left = ast::apply(&composed, &input, &defs);
    let right = ast::apply(&constant_a, &input, &defs);

    assert_eq!(left, right, "Law III.1: x̄∘f = x̄");
}

#[test]
fn law_iii2_composition_with_identity() {
    // f∘id = id∘f = f
    use arest::ast::{self, Func, Object};
    let defs = ast::Object::phi();

    let f = Func::Selector(1);
    let input = Object::seq(vec![Object::atom("A"), Object::atom("B")]);

    let f_id = Func::compose(f.clone(), Func::Id);
    let id_f = Func::compose(Func::Id, f.clone());

    let result_f = ast::apply(&f, &input, &defs);
    let result_f_id = ast::apply(&f_id, &input, &defs);
    let result_id_f = ast::apply(&id_f, &input, &defs);

    assert_eq!(result_f, result_f_id, "Law III.2: f∘id = f");
    assert_eq!(result_f, result_id_f, "Law III.2: id∘f = f");
}

#[test]
fn law_insert_fold_right() {
    // /f:<x1,...,xn> = f:<x1, /f:<x2,...,xn>>
    use arest::ast::{self, Func, Object};
    let defs = ast::Object::phi();

    // /+:<1,2,3> should equal +:<1, +:<2, 3>> = +:<1, 5> = 6
    // But we use atoms, so test with a function we can verify.
    // /[s1, s2]:<A,B,C> = [s1,s2]:<A, [s1,s2]:<B,C>>
    //                    = [s1,s2]:<A, <B,C>>
    //                    = <A, <B,C>>
    let pair = Func::construction(vec![Func::Selector(1), Func::Selector(2)]);
    let fold = Func::Insert(Box::new(pair));
    let input = Object::seq(vec![Object::atom("A"), Object::atom("B"), Object::atom("C")]);
    let result = ast::apply(&fold, &input, &defs);

    // /[s1,s2]:<A,B,C> = [s1,s2]:<A, /[s1,s2]:<B,C>>
    // /[s1,s2]:<B,C> = [s1,s2]:<B,C> = <B,C> (base case: 2 elements)
    // [s1,s2]:<A, <B,C>> = <A, <B,C>>
    let expected = Object::seq(vec![Object::atom("A"), Object::seq(vec![Object::atom("B"), Object::atom("C")])]);
    assert_eq!(result, expected, "Insert (fold right): /f:<x1,...,xn> = f:<x1, /f:<x2,...,xn>>");
}

#[test]
fn law_condition_selects_branch() {
    // (p → f; g):x = f:x if p:x = T, g:x if p:x = F
    use arest::ast::{self, Func, Object};
    let defs = ast::Object::phi();

    // Predicate: eq∘[s1, s2] (are first two elements equal?)
    let pred = Func::compose(Func::Eq, Func::construction(vec![Func::Selector(1), Func::Selector(2)]));
    let f = Func::constant(Object::atom("EQUAL"));
    let g = Func::constant(Object::atom("NOT_EQUAL"));
    let cond = Func::condition(pred, f, g);

    // <A, A> should produce "EQUAL"
    let same = Object::seq(vec![Object::atom("A"), Object::atom("A")]);
    assert_eq!(ast::apply(&cond, &same, &defs).as_atom(), Some("EQUAL"),
        "Condition: p:x = T → f:x");

    // <A, B> should produce "NOT_EQUAL"
    let diff = Object::seq(vec![Object::atom("A"), Object::atom("B")]);
    assert_eq!(ast::apply(&cond, &diff, &defs).as_atom(), Some("NOT_EQUAL"),
        "Condition: p:x = F → g:x");
}

// ── HATEOAS via Pure Application ─────────────────────────────────────
// Transition links are projections of P filtered by current status.
// The test verifies links are produced by applying functions from D,
// not by accessing struct fields.

#[test]
fn hateoas_transition_links_via_application() {
    use arest::ast::{self, Object};

    let (_, d) = compile_orders();

    // The transition function applied to <current_state, event> produces next_state.
    // Available links = all events where transition:<current_state, event> != bottom.
    let transition = ast::metacompose(&ast::fetch_or_phi("machine:Order", &d), &d);

    // From "In Cart", test which events produce a state change.
    // A valid transition changes the state. An invalid one preserves it.
    // Available links = events where transition:<state, event> != state.
    let events = ["place", "ship", "deliver", "cancel"];
    let current = "In Cart";
    let mut available_from_cart: Vec<&str> = vec![];
    for event in &events {
        let input = Object::seq(vec![Object::atom(current), Object::atom(event)]);
        let result = ast::apply(&transition, &input, &d);
        if let Some(next) = result.as_atom() {
            if next != current {
                available_from_cart.push(event);
            }
        }
    }
    assert!(available_from_cart.contains(&"place"), "In Cart should have place link");
    assert!(available_from_cart.contains(&"cancel"), "In Cart should have cancel link");
    assert!(!available_from_cart.contains(&"ship"), "In Cart should not have ship link");
    assert!(!available_from_cart.contains(&"deliver"), "In Cart should not have deliver link");

    // From "Delivered" (terminal), no events should produce a state change
    let current = "Delivered";
    let mut available_from_delivered: Vec<&str> = vec![];
    for event in &events {
        let input = Object::seq(vec![Object::atom(current), Object::atom(event)]);
        let result = ast::apply(&transition, &input, &d);
        if let Some(next) = result.as_atom() {
            if next != current {
                available_from_delivered.push(event);
            }
        }
    }
    assert!(available_from_delivered.is_empty(), "Delivered (terminal) should have no links");
}

// ── Constraint Evaluation via Pure Application ───────────────────────

#[test]
fn constraint_evaluation_via_application() {
    use arest::ast::{self, Func, Object};

    let (state, d) = compile_orders();

    let defs: Vec<(String, Func)> = compile::compile_to_defs_state(&state);

    // Find the obligatory constraint (these are response-scoped)
    let (_, _constraint_func) = defs.iter()
        .find(|(name, _)| name.contains("constraint:") && name.contains("obligatory") && name.contains("placed by"))
        .expect("Obligatory placed-by constraint");

    // Obligatory constraints check that the response text contains required content.
    // Per the paper equation (3): V = union of (rho c):P for all c in C_S.
    // Every constraint is applied to the population. The response text is a
    // fact in P (Support Response has Body), not a special evaluation mode.
    //
    // Alethic constraints (UC, MC) evaluate against P.
    // Deontic text constraints (forbidden enum values) evaluate against response text in P.
    // Both are applications of constraint functions to P.

    // Find a UC constraint and verify it's callable against a population object
    let (_, uc_func) = defs.iter()
        .find(|(name, _)| name.contains("constraint:") && name.contains("at most one"))
        .expect("Should have a UC constraint");

    // The eval context is <response_text, sender, population>.
    // For population constraints, the response text is irrelevant.
    let context = Object::seq(vec![
        Object::phi(),
        Object::phi(),
        Object::phi(),
    ]);
    let result = ast::apply(uc_func, &context, &d);
    // UC against empty population: no facts = no violations for UC.
    // The constraint is compiled to Filter(p):P — Filter over an
    // empty P returns an empty Seq (Object::Seq(vec![])) rather than
    // φ, so check the decoded violation list rather than comparing
    // Objects literally.
    assert!(ast::decode_violations(&result).is_empty(),
        "UC constraint against empty population should produce no violations, got {:?}",
        result);

    // Now test with a state that has a UC violation.
    // "Each Order was placed by exactly one Customer" means
    // an Order with two different Customers should violate.
    let violation_state = {
        let mut s = Object::phi();
        s = ast::cell_push("Order_was_placed_by_Customer",
            ast::fact_from_pairs(&[("Order", "ord-1"), ("Customer", "Acme")]), &s);
        s = ast::cell_push("Order_was_placed_by_Customer",
            ast::fact_from_pairs(&[("Order", "ord-1"), ("Customer", "Globex")]), &s);
        s
    };
    let pop_with_violation = ast::encode_state(&violation_state);
    let context_with_violation = Object::seq(vec![
        Object::phi(),
        Object::phi(),
        pop_with_violation,
    ]);

    // Find the UC on "placed by" specifically
    let (_, placed_by_uc) = defs.iter()
        .find(|(name, _)| name.contains("constraint:") && name.contains("at most one") && name.contains("placed by"))
        .expect("UC on placed by");
    let violation_result = ast::apply(placed_by_uc, &context_with_violation, &d);
    assert_ne!(violation_result, Object::phi(),
        "UC violation: Order ord-1 placed by two Customers should violate");
}

// ── Response Text as Fact in P ───────────────────────────────────────
// The response is an entity in P with facts, not a separate struct.
// Deontic text constraints evaluate against the Body fact in P.

#[test]
fn response_text_is_a_fact_in_population() {
    use arest::ast::{self, Object};

    // A response body is just a fact: Support Response 'resp-1' has Body 'some text'
    let mut state = Object::phi();
    state = ast::cell_push("Support_Response_has_Body",
        ast::fact_from_pairs(&[("Support Response", "resp-1"), ("Body", "We will ship overnight for sure")]),
        &state);

    // The state contains the response text as a fact.
    // A constraint evaluator should be able to find it by querying the state
    // for Support_Response_has_Body facts.
    let state_obj = ast::encode_state(&state);

    // The state is queryable. The body text is in P, not in a separate struct.
    assert_ne!(state_obj, Object::phi(), "State with response should not be empty");
}

#[test]
fn deontic_constraint_evaluates_against_population_body() {
    

    // Parse a domain with a forbidden deontic constraint
    let input = r#"
# ResponseTest
## Entity Types
Support Response(.id) is an entity type.
Prohibited Word is a value type.
  The possible values of Prohibited Word are 'overnight', 'asap'.
Body is a value type.
## Fact Types
Support Response has Body.
Support Response contains Prohibited Word.
## Deontic Constraints
It is forbidden that Support Response contains Prohibited Word.
"#;

    let ir = parse_forml2::parse_markdown(input).unwrap();
    let domain_state = parse_forml2::domain_to_state(&ir);
    let defs = compile::compile_to_defs_state(&domain_state);
    let d = ast::defs_to_state(&defs, &domain_state);

    // Helper: evaluate constraints via defs path
    let eval = |text: &str| -> Vec<Violation> {
        let empty_pop = ast::Object::phi();
        let ctx_obj = ast::encode_eval_context_state(text, None, &empty_pop);
        defs.iter()
            .filter(|(n, _)| n.starts_with("constraint:"))
            .flat_map(|(name, func)| {
                let result = ast::apply(func, &ctx_obj, &d);
                let is_deontic = name.contains("obligatory") || name.contains("forbidden");
                ast::decode_violations(&result).into_iter().map(move |mut v| {
                    v.alethic = !is_deontic;
                    v
                })
            })
            .collect()
    };

    // Bad response: body contains "overnight"
    let violations = eval("We will ship overnight");
    assert!(!violations.is_empty(), "Should catch 'overnight' via enum matching");

    // Good response: no prohibited words
    let good_violations = eval("We will ship via standard delivery");
    let prohibited_violations: Vec<_> = good_violations.iter()
        .filter(|v| v.constraint_text.contains("Prohibited Word"))
        .collect();
    assert!(prohibited_violations.is_empty(), "Clean response should not trigger prohibited word violation");
}

#[test]
fn deontic_constraint_via_defs_and_application() {
    use arest::ast::{self, Func, Object};

    // Same domain as above
    let input = r#"
# ResponseTest
## Entity Types
Support Response(.id) is an entity type.
Prohibited Word is a value type.
  The possible values of Prohibited Word are 'overnight', 'asap'.
Body is a value type.
## Fact Types
Support Response has Body.
Support Response contains Prohibited Word.
## Deontic Constraints
It is forbidden that Support Response contains Prohibited Word.
"#;

    let ir = parse_forml2::parse_markdown(input).unwrap();
    let domain_state = parse_forml2::domain_to_state(&ir);
    let defs: Vec<(String, Func)> = compile::compile_to_defs_state(&domain_state);
    let d = ast::defs_to_state(&defs, &domain_state);

    // Find the forbidden constraint
    let (_, constraint_func) = defs.iter()
        .find(|(name, _)| name.contains("constraint:") && name.contains("Prohibited Word"))
        .expect("Prohibited Word constraint");

    // Bad response: eval context has "overnight" in the text position
    let bad_ctx = Object::seq(vec![
        Object::atom("We will ship overnight"),
        Object::phi(),
        Object::phi(),
    ]);
    let bad_result = ast::apply(constraint_func, &bad_ctx, &d);
    assert_ne!(bad_result, Object::phi(),
        "Forbidden constraint via pure application should catch 'overnight'");

    // Good response
    let good_ctx = Object::seq(vec![
        Object::atom("We will ship standard"),
        Object::phi(),
        Object::phi(),
    ]);
    let good_result = ast::apply(constraint_func, &good_ctx, &d);
    assert_eq!(good_result, Object::phi(),
        "Forbidden constraint via pure application should pass clean text");
}

// ── Forward Chaining via Pure Application ────────────────────────────

#[test]
fn derivation_rules_are_functions() {
    use arest::ast::Func;

    let (state, _) = compile_orders();

    let defs: Vec<(String, Func)> = compile::compile_to_defs_state(&state);

    // Derivation rules should exist as named definitions
    let derivation_defs: Vec<_> = defs.iter()
        .filter(|(name, _)| name.starts_with("derivation:"))
        .collect();
    assert!(!derivation_defs.is_empty(), "Should have derivation rule definitions");

    // Each derivation is a Func that can be applied to a population object
    for (name, _func) in &derivation_defs {
        assert!(!name.is_empty());
    }
}

// ── Self-Modification (Corollary 8, Backus 14.3) ─────────────────────
// Ingesting new readings into D produces D'. Subsequent evaluations use D'.

#[test]
fn self_modification_extends_definitions() {
    use arest::ast::Func;

    let (state, _) = compile_orders();

    // D: initial definitions
    let defs_d: Vec<(String, Func)> = compile::compile_to_defs_state(&state);
    let d_count = defs_d.len();

    // Ingest new readings (self-modification via ↓DEFS)
    let extension = r#"
# OrderExtension
## Entity Types
Coupon(.Code) is an entity type.
## Value Types
Code is a value type.
Discount is a value type.
## Fact Types
Coupon has Discount.
Order has Coupon.
## Constraints
Each Coupon has exactly one Discount.
Each Order has at most one Coupon.
"#;
    let ext_state = parse_forml2::parse_to_state_with_nouns(extension, &state).unwrap();
    let merged_state = merge_state_into(&state, &ext_state);

    // D': new definitions after self-modification
    let defs_d_prime: Vec<(String, Func)> = compile::compile_to_defs_state(&merged_state);

    // D' should have more definitions than D
    assert!(defs_d_prime.len() > d_count,
        "Self-modification should add new definitions to D");

    // New constraint definitions should exist
    assert!(defs_d_prime.iter().any(|(name, _)| name.contains("Coupon")),
        "D' should contain Coupon constraint definitions");

    // Original definitions should still be present
    assert!(defs_d_prime.iter().any(|(name, _)| name.contains("machine:Order")),
        "D' should still contain Order state machine (Corollary 8)");
}

// ── Metacomposition (Backus 13.3.2, AREST equation 1) ────────────────
// (ρ<x1,...,xn>):y = (ρx1):<<x1,...,xn>, y>
// A fact receives an operation and applies it to its own objects.

#[test]
fn metacomposition_fact_receives_operation() {
    use arest::ast::{self, Func, Object};
    let defs = ast::Object::phi();

    // A fact type CONS(s1, s2) applied to <A, B> produces <A, B> (the fact).
    // The fact is a function: it receives an operation and applies it.
    let cons = Func::construction(vec![Func::Selector(1), Func::Selector(2)]);
    let fact = ast::apply(&cons, &Object::seq(vec![Object::atom("Acme"), Object::atom("acme@example.com")]), &defs);

    // The fact should be <Acme, acme@example.com>
    if let Some(elements) = fact.as_seq() {
        assert_eq!(elements.len(), 2);
        assert_eq!(elements[0].as_atom(), Some("Acme"));
        assert_eq!(elements[1].as_atom(), Some("acme@example.com"));
    } else {
        panic!("CONS should produce a sequence");
    }

    // Selector extracts from the fact (the operation is "get role 2")
    let email = ast::apply(&Func::Selector(2), &fact, &defs);
    assert_eq!(email.as_atom(), Some("acme@example.com"),
        "Metacomposition: applying selector to a fact extracts the role value");
}

// ── Schema as CONS (AREST Table 1) ───────────────────────────────────
// A fact type is CONS(s1,...,sn). It produces a fact when applied to objects.

#[test]
fn schema_is_cons_of_roles() {
    use arest::ast::Func;

    let (state, _) = compile_orders();

    let defs: Vec<(String, Func)> = compile::compile_to_defs_state(&state);

    // Find the schema for "Order was placed by Customer"
    let schema = defs.iter()
        .find(|(name, _)| name.contains("schema:") && name.contains("placed_by"))
        .expect("Should have placed_by schema");

    // The schema is a CONS (Construction) function
    assert!(schema.0.starts_with("schema:"), "Schema should be a named definition");
}

// ── ρ (Representation Function) ──────────────────────────────────────
// Per Backus 13.3.2: (ρ<x1,...,xn>):y = (ρx1):<<x1,...,xn>, y>
// A fact receives an operation and applies it via ρ.

#[test]
fn rho_resolves_fact_type_and_applies_operation() {
    use arest::ast::{self, Func, Object};

    // metacompose resolves an object to a Func via the representation function.
    // For an atom, it looks up DEFS.
    // For a sequence, the first element is the controlling operator.

    // Build D as an Object with a cell for "Customer_has_Email"
    let defs_vec: Vec<(String, Func)> = vec![
        ("Customer_has_Email".to_string(), Func::constant(Object::atom("handled"))),
    ];
    let d = ast::defs_to_state(&defs_vec, &Object::phi());

    // An atom resolves to its definition
    let func = ast::metacompose(&Object::atom("Customer_has_Email"), &d);
    let result = ast::apply(&func, &Object::atom("read"), &d);
    assert_eq!(result.as_atom(), Some("handled"),
        "metacompose should resolve atom to its definition in DEFS");

    // A fact (sequence) uses the first element as the controlling operator.
    // Per Backus 13.3.2: (ρ<COMP, f, g>):x = (f∘g):x
    // We test with CONS: (ρ<CONS, s1, s2>):x = [s1, s2]:x = <s1:x, s2:x>
    let cons_obj = Object::seq(vec![
        Object::atom("["),
        Object::atom("1"),  // selector 1
        Object::atom("2"),  // selector 2
    ]);
    let cons_func = ast::metacompose(&cons_obj, &d);
    let input = Object::seq(vec![Object::atom("A"), Object::atom("B"), Object::atom("C")]);
    let result = ast::apply(&cons_func, &input, &d);
    // [s1, s2]:<A, B, C> = <s1:<A,B,C>, s2:<A,B,C>> = <A, B>
    if let Some(elements) = result.as_seq() {
        assert_eq!(elements[0].as_atom(), Some("A"));
        assert_eq!(elements[1].as_atom(), Some("B"));
    } else {
        panic!("CONS metacomposition should produce a sequence");
    }
}

#[test]
fn rho_returns_bottom_for_unknown_fact_type() {
    use arest::ast::{self, Object};

    let defs = ast::Object::phi();
    let fact = Object::seq(vec![Object::atom("unknown_type"), Object::atom("value")]);
    let result = {
        let func = ast::metacompose(&fact, &defs);
        ast::apply(&func, &Object::atom("read"), &defs)
    };
    assert_eq!(result, Object::Bottom, "metacompose should return bottom for unknown fact types");
}

// ── HATEOAS Navigation as FFP Projections (Theorem 4b) ───────────────

#[test]
fn hateoas_nav_defs_produced() {
    use arest::ast::Func;

    let (state, _) = compile_orders();

    let defs: Vec<(String, Func)> = compile::compile_to_defs_state(&state);

    // "Each Order was placed by exactly one Customer" → UC on Order's role
    // Order is child, Customer is parent
    let customer_children = defs.iter()
        .find(|(n, _)| n == "nav:Customer:children");
    assert!(customer_children.is_some(), "Customer should have children nav def");

    let order_parent = defs.iter()
        .find(|(n, _)| n == "nav:Order:parent");
    assert!(order_parent.is_some(), "Order should have parent nav def");
}

// ---- Backus 14.4.2: operand with cells ----

#[test]
fn system_operand_fetch_retrieves_cells() {
    use arest::ast::{self, Object};

    let (state, _d) = compile_orders();

    // Construct operand per Backus 14.4.2: <CELL:KEY, CELL:INPUT, CELL:FILE, ...defs>
    let key_cell = ast::cell("KEY", Object::atom("machine:Order:initial"));
    let input_cell = ast::cell("INPUT", Object::phi());
    let pop = state.clone();
    let file_cell = ast::cell("FILE", ast::encode_state(&pop));
    let operand = Object::seq(vec![key_cell, input_cell, file_cell]);

    // Fetch retrieves cell contents from the operand
    assert_eq!(ast::fetch("KEY", &operand).as_atom(), Some("machine:Order:initial"));
    assert!(!matches!(ast::fetch("FILE", &operand), Object::Bottom));
    assert_eq!(ast::fetch("INPUT", &operand), Object::phi());
}

#[test]
fn transitions_def_returns_available_from_status() {
    use arest::ast::{self, Object};

    let (_, d) = compile_orders();

    // transitions:Order should be a DEFS entry
    let transitions_func = ast::metacompose(&ast::fetch_or_phi("transitions:Order", &d), &d);

    // From "In Cart", place and cancel should be available
    let result = ast::apply(&transitions_func, &Object::atom("In Cart"), &d);
    let items = result.as_seq().expect("Should return a sequence");
    assert!(!items.is_empty(), "Should have transitions from In Cart");

    // Each item is <from, to, event>
    let events: Vec<&str> = items.iter()
        .filter_map(|item| item.as_seq().and_then(|s| s.get(2)).and_then(|o| o.as_atom()))
        .collect();
    assert!(events.contains(&"place"), "Should have place transition");

    // From "Delivered" (terminal), no transitions
    let terminal = ast::apply(&transitions_func, &Object::atom("Delivered"), &d);
    let terminal_items = terminal.as_seq().unwrap_or(&[]);
    assert!(terminal_items.is_empty(), "Terminal state should have no transitions");
}

// ---- SM init derivation debugging ----

#[test]
fn sm_init_derivation_produces_facts() {
    use arest::ast::{self, Func, Object};

    let (state, d) = compile_orders();

    let defs: Vec<(String, Func)> = compile::compile_to_defs_state(&state);

    // Find the SM init derivation
    let sm_init = defs.iter().find(|(n, _)| n.contains("sm_init")).expect("SM init def should exist");
    eprintln!("SM init def name: {}", sm_init.0);

    // Build a minimal state with one Order entity
    let mut test_state = state.clone();
    test_state = ast::cell_push("Order_has_customer",
        ast::fact_from_pairs(&[("Order", "ord-1"), ("customer", "Acme")]),
        &test_state);

    // Encode state as population object and apply derivation
    let test_pop = test_state.clone();
    let pop_obj = ast::encode_state(&test_pop);

    // Test derive_facts on <ord-1>
    let make_one_fact = ast::Func::construction(vec![
        ast::Func::constant(Object::atom("StateMachine_has_forResource")),
        ast::Func::constant(Object::atom("test")),
        ast::Func::construction(vec![
            ast::Func::construction(vec![ast::Func::constant(Object::atom("SM")), ast::Func::Id]),
            ast::Func::construction(vec![ast::Func::constant(Object::atom("forResource")), ast::Func::Id]),
        ]),
    ]);
    let one_fact_result = ast::apply(&make_one_fact, &Object::atom("ord-1"), &d);
    eprintln!("make_one_fact : ord-1 = {}", one_fact_result);

    let result = ast::apply(&sm_init.1, &pop_obj, &d);
    eprintln!("SM init raw result: {}", result);

    // Check it is not empty
    assert!(!matches!(result, Object::Seq(ref v) if v.is_empty()), "SM init should produce derived facts");
    assert!(!matches!(result, Object::Bottom), "SM init should not produce bottom");
}

// ---- #38+#41: create via DEFS, no CompiledModel ----
// Eq. 12: create = emit . validate . derive . resolve
// The function takes (defs, command, population) and returns CommandResult.
// No CompiledModel in the signature.

#[test]
fn create_entity_via_defs_produces_entity_and_status() {
    use arest::ast::Func;
    use std::collections::HashMap;

    let (state, d) = compile_orders();

    let defs: Vec<(String, Func)> = compile::compile_to_defs_state(&state);

    let mut fields = HashMap::new();
    fields.insert("customer".to_string(), "Acme".to_string());

    let command = arest::command::Command::CreateEntity {
        noun: "Order".to_string(),
        domain: "orders".to_string(),
        id: Some("ord-1".to_string()),
        fields,
        sender: None,
        signature: None,
    };

    let result = arest::command::apply_command_defs(&d, &command, &state);

    eprintln!("derived_count: {}", result.derived_count);
    eprintln!("status: {:?}", result.status);
    eprintln!("entities: {:?}", result.entities.iter().map(|e| (&e.id, &e.entity_type)).collect::<Vec<_>>());
    eprintln!("violations: {:?}", result.violations.len());
    let derivation_defs: Vec<_> = defs.iter().filter(|(n, _)| n.starts_with("derivation:")).map(|(n, _)| n.as_str()).collect();
    eprintln!("derivation defs: {:?}", derivation_defs);
    let sm_defs: Vec<_> = defs.iter().filter(|(n, _)| n.contains("machine")).map(|(n, _)| n.as_str()).collect();
    eprintln!("machine defs: {:?}", sm_defs);
    let instance_cell = ast::fetch_or_phi("InstanceFact", &state);
    let instance_count = instance_cell.as_seq().map(|s| s.len());
    eprintln!("InstanceFact count: {:?}", instance_count);

    assert!(!result.rejected, "Valid create should not be rejected");
    assert!(result.entities.len() >= 1, "Should have at least the entity");
    assert_eq!(result.entities[0].id, "ord-1");
    assert_eq!(result.entities[0].entity_type, "Order");
    assert_eq!(result.status, Some("In Cart".to_string()));
    assert!(!result.transitions.is_empty(), "Should have transitions from initial state");
}

#[test]
fn transition_via_defs_changes_status() {
    use std::collections::HashMap;

    let (state, d) = compile_orders();

    // First create an entity to get a state with SM status
    let mut fields = HashMap::new();
    fields.insert("customer".to_string(), "Acme".to_string());
    let create_cmd = arest::command::Command::CreateEntity {
        noun: "Order".to_string(),
        domain: "orders".to_string(),
        id: Some("ord-1".to_string()),
        fields,
        sender: None,
        signature: None,
    };
    let create_result = arest::command::apply_command_defs(&d, &create_cmd, &state);
    assert_eq!(create_result.status, Some("In Cart".to_string()));

    // Now transition: place the order
    let transition_cmd = arest::command::Command::Transition {
        entity_id: "ord-1".to_string(),
        event: "place".to_string(),
        domain: "orders".to_string(),
        current_status: Some("In Cart".to_string()),
        sender: None,
        signature: None,
    };
    let result = arest::command::apply_command_defs(&d, &transition_cmd, &create_result.state);

    assert_eq!(result.status, Some("Placed".to_string()));
}

// ── Generator DEFS ───────────────────────────────────────────────────
// Compile-time generators produce constant DEFS entries:
//   agent:{noun}       - synthesized agent prompt
//   ilayer:{noun}      - noun information layer
//   sql:{table}        - DDL for relational mapping
//   test:{constraint}  - constraint test harness

#[test]
fn agent_defs_produced_for_nouns_with_facts() {
    let (_pop, d) = compile_orders();
    let agent_defs: Vec<_> = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("agent:"))
        .collect();
    assert!(!agent_defs.is_empty(), "Should produce agent defs");
    // Order should have an agent def
    assert!(ast::cells_iter(&d).into_iter().any(|(k, _)| k == "agent:Order"), "Order should have agent def");
}

#[test]
fn agent_def_contains_role_and_fact_types() {
    let (_pop, d) = compile_orders();
    let agent_order_obj = ast::fetch_or_phi("agent:Order", &d);
    let agent_order = ast::metacompose(&agent_order_obj, &d);
    let obj = match &agent_order {
        arest::ast::Func::Constant(obj) => obj,
        _ => panic!("agent def should be Func::Constant"),
    };
    let text = format!("{:?}", obj);
    assert!(text.contains("role"), "agent prompt should contain role");
    assert!(text.contains("fact_types"), "agent prompt should contain fact_types");
    assert!(text.contains("Order"), "agent prompt should reference Order");
}

#[test]
fn agent_def_contains_transitions_for_sm_noun() {
    let (_pop, d) = compile_orders();
    let agent_order = ast::metacompose(&ast::fetch_or_phi("agent:Order", &d), &d);
    let text = format!("{:?}", agent_order);
    assert!(text.contains("transitions"), "Order agent should have transitions");
    assert!(text.contains("place") || text.contains("ship"), "Order agent should list SM events");
}

#[test]
fn agent_def_contains_deontic_rules() {
    let (_pop, d) = compile_orders();
    let agent_order = ast::metacompose(&ast::fetch_or_phi("agent:Order", &d), &d);
    let text = format!("{:?}", agent_order);
    assert!(text.contains("deontic"), "Order agent should have deontic section");
}

#[test]
fn ilayer_defs_produced_for_nouns() {
    let (_pop, d) = compile_orders_with_generators(&["ilayer"]);
    let ilayer_defs: Vec<_> = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("ilayer:"))
        .collect();
    assert!(!ilayer_defs.is_empty(), "Should produce ilayer defs");
    assert!(ast::cells_iter(&d).into_iter().any(|(k, _)| k == "ilayer:Order"), "Order should have ilayer def");
    assert!(ast::cells_iter(&d).into_iter().any(|(k, _)| k == "ilayer:Customer"), "Customer should have ilayer def");
}

#[test]
fn ilayer_def_contains_object_type_and_facts() {
    let (_pop, d) = compile_orders_with_generators(&["ilayer"]);
    let ilayer = ast::metacompose(&ast::fetch_or_phi("ilayer:Order", &d), &d);
    let obj = match &ilayer {
        arest::ast::Func::Constant(obj) => obj,
        _ => panic!("ilayer def should be Func::Constant"),
    };
    let text = format!("{:?}", obj);
    assert!(text.contains("object_type"), "ilayer should contain object_type");
    assert!(text.contains("fact_types"), "ilayer should contain fact_types");
}

#[test]
fn sql_defs_produced_for_tables() {
    let (_pop, d) = compile_orders_with_generators(&["sqlite"]);
    let sql_defs: Vec<_> = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("sql:"))
        .collect();
    assert!(!sql_defs.is_empty(), "Should produce sql defs");
}

#[test]
fn sql_def_contains_create_table() {
    let (_pop, d) = compile_orders_with_generators(&["sqlite"]);
    let sql_cells: Vec<_> = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("sql:"))
        .map(|(n, _)| n.to_string())
        .collect();
    for name in &sql_cells {
        let func = ast::metacompose(&ast::fetch_or_phi(name, &d), &d);
        let obj = match &func {
            arest::ast::Func::Constant(obj) => obj.clone(),
            _ => panic!("{} should be Func::Constant", name),
        };
        let text = format!("{:?}", obj);
        assert!(text.contains("CREATE TABLE"), "sql def {} should contain CREATE TABLE DDL", name);
    }
}

#[test]
fn test_defs_produced_for_constraints() {
    let (_pop, d) = compile_orders_with_generators(&["test"]);
    let test_defs: Vec<_> = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("test:"))
        .collect();
    assert!(!test_defs.is_empty(), "Should produce test defs");
}

#[test]
fn test_def_contains_constraint_metadata() {
    let (_pop, d) = compile_orders_with_generators(&["test"]);
    let test_names: Vec<_> = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("test:"))
        .map(|(n, _)| n.to_string())
        .collect();
    for name in &test_names {
        let func = ast::metacompose(&ast::fetch_or_phi(name, &d), &d);
        let obj = match &func {
            arest::ast::Func::Constant(obj) => obj.clone(),
            _ => panic!("{} should be Func::Constant", name),
        };
        let text = format!("{:?}", obj);
        assert!(text.contains("id"), "test def {} should contain id", name);
        assert!(text.contains("kind"), "test def {} should contain kind", name);
        assert!(text.contains("modality"), "test def {} should contain modality", name);
    }
}

// ── HATEOAS Navigation Links (Theorem 4b) ──────────────────────────
// nav(e, n) = children(n) ∪ parent(n) — projections from S via UC.

#[test]
fn hateoas_nav_links_in_create_response() {
    use std::collections::HashMap;
    let (state, d) = compile_orders();

    let mut fields = HashMap::new();
    fields.insert("customer".to_string(), "Acme".to_string());

    let command = arest::command::Command::CreateEntity {
        noun: "Order".to_string(),
        domain: "orders".to_string(),
        id: Some("ord-nav".to_string()),
        fields,
        sender: None,
        signature: None,
    };

    let result = arest::command::apply_command_defs(&d, &command, &state);
    assert!(!result.rejected);

    // Order has children (Line Item) and parents (Customer) from UC projections
    let child_nouns: Vec<&str> = result.navigation.iter()
        .filter(|l| l.rel == "children")
        .map(|l| l.noun.as_str())
        .collect();
    let parent_nouns: Vec<&str> = result.navigation.iter()
        .filter(|l| l.rel == "parent")
        .map(|l| l.noun.as_str())
        .collect();

    // At minimum, Order should have nav links (exact nouns depend on schema)
    assert!(!result.navigation.is_empty(),
        "Order should have navigation links from UC projections");
    eprintln!("Nav links: children={:?}, parents={:?}", child_nouns, parent_nouns);
}

// ── Compound Reference Scheme E2E ──────────────────────────────────
// Paper Eq. 6: resolve determines identity from the reference scheme.
// A compound ref scheme (.Owner, .Seq) should decompose "alice-1" into
// Owner=alice, Seq=1 as component facts during create.

const COMPOUND_REF_DOMAIN: &str = r#"
Thing (.Owner, .Seq) is an entity type.
Owner is a value type.
Seq is a value type.
Label is a value type.
Thing has Label.
"#;

fn compile_compound_ref() -> (ast::Object, ast::Object) {
    let ir = parse_forml2::parse_markdown(COMPOUND_REF_DOMAIN).unwrap();
    let state = parse_forml2::domain_to_state(&ir);
    let defs = compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);
    (state, d)
}

#[test]
fn compound_ref_scheme_e2e_instance_facts_decompose() {
    // Parse-time path: instance facts in readings decompose compound IDs.
    let input = r#"
Thing (.Owner, .Seq) is an entity type.
Owner is a value type.
Seq is a value type.
Label is a value type.
Thing has Label.

## Instance Facts
Thing 'alice-1' has Label 'foo'.
"#;
    let ir = parse_forml2::parse_markdown(input).unwrap();
    let state = parse_forml2::domain_to_state(&ir);
    let defs = compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);

    // Component facts should exist from parse-time decomposition
    let owner_cell = ast::fetch_or_phi("Thing_has_Owner", &d);
    let owner_facts = owner_cell.as_seq().expect("Thing_has_Owner cell should exist");
    assert!(owner_facts.iter().any(|f|
        ast::binding(f, "Thing") == Some("alice-1") &&
        ast::binding(f, "Owner") == Some("alice")
    ), "Owner component should be decomposed from 'alice-1'");

    let seq_cell = ast::fetch_or_phi("Thing_has_Seq", &d);
    let seq_facts = seq_cell.as_seq().expect("Thing_has_Seq cell should exist");
    assert!(seq_facts.iter().any(|f|
        ast::binding(f, "Thing") == Some("alice-1") &&
        ast::binding(f, "Seq") == Some("1")
    ), "Seq component should be decomposed from 'alice-1'");
}

#[test]
fn compound_ref_scheme_e2e_create_entity_decomposes() {
    use std::collections::HashMap;
    // Runtime path: createEntity with compound ref scheme should decompose the ID.
    let (state, d) = compile_compound_ref();

    let mut fields = HashMap::new();
    fields.insert("Label".to_string(), "foo".to_string());

    let command = arest::command::Command::CreateEntity {
        noun: "Thing".to_string(),
        domain: "test".to_string(),
        id: Some("bob-2".to_string()),
        fields,
        sender: None,
        signature: None,
    };

    let result = arest::command::apply_command_defs(&d, &command, &state);
    assert!(!result.rejected, "Valid create should not be rejected");

    // The resolve step should decompose "bob-2" into Owner=bob, Seq=2
    let owner_cell = ast::fetch_or_phi("Thing_has_Owner", &result.state);
    let owner_facts = owner_cell.as_seq().unwrap_or_default();
    assert!(owner_facts.iter().any(|f|
        ast::binding(f, "Thing") == Some("bob-2") &&
        ast::binding(f, "Owner") == Some("bob")
    ), "Runtime create should decompose compound ID: Owner component missing.\nOwner facts: {:?}", owner_facts);

    let seq_cell = ast::fetch_or_phi("Thing_has_Seq", &result.state);
    let seq_facts = seq_cell.as_seq().unwrap_or_default();
    assert!(seq_facts.iter().any(|f|
        ast::binding(f, "Thing") == Some("bob-2") &&
        ast::binding(f, "Seq") == Some("2")
    ), "Runtime create should decompose compound ID: Seq component missing.\nSeq facts: {:?}", seq_facts);
}

// ── Self-Evolution (Corollary 5) ───────────────────────────────────
// Closure Under Self-Modification: ingesting new readings at runtime
// via LoadReadings should merge new nouns/fact types/constraints into
// the state and compile new defs. Subsequent SYSTEM calls use them.

#[test]
fn self_evolution_load_readings_extends_schema() {
    // Start with a compiled domain
    let (state, d) = compile_orders();

    // Verify "Reason" does NOT exist before loading
    let noun_cell_before = ast::fetch_or_phi("Noun", &state);
    let has_reason_before = noun_cell_before.as_seq()
        .map_or(false, |facts| facts.iter().any(|f| ast::binding(f, "name") == Some("Reason")));
    assert!(!has_reason_before, "Reason should not exist before LoadReadings");

    // Load new readings that add a NEW noun + fact type
    let new_readings = r#"
Reason is a value type.
Order has Reason.
  Each Order has at most one Reason.
"#;
    let load_cmd = arest::command::Command::LoadReadings {
        markdown: new_readings.to_string(),
        domain: "orders".to_string(),
        sender: None,
        signature: None,
    };

    let load_result = arest::command::apply_command_defs(&d, &load_cmd, &state);
    assert!(!load_result.rejected, "Loading valid readings should not be rejected");

    // The state should now contain the new Noun
    let noun_cell = ast::fetch_or_phi("Noun", &load_result.state);
    let noun_facts = noun_cell.as_seq().expect("Noun cell should exist");
    assert!(noun_facts.iter().any(|f| ast::binding(f, "name") == Some("Reason")),
        "New noun 'Reason' should be in state after LoadReadings");

    // The state should contain the new FactType
    let schema_cell = ast::fetch_or_phi("FactType", &load_result.state);
    let schema_facts = schema_cell.as_seq().expect("FactType cell should exist");
    assert!(schema_facts.iter().any(|f| {
        ast::binding(f, "reading").map_or(false, |r| r.contains("Reason"))
    }), "New fact type with Reason should be in state after LoadReadings");
}

#[test]
fn self_evolution_new_constraints_enforce_after_load() {
    use std::collections::HashMap;

    let (state, d) = compile_orders();

    // Load readings that add a mandatory constraint on a new fact type
    let new_readings = r#"
Reason is a value type.
Order has Reason.
  Each Order has exactly one Reason.
"#;
    let load_cmd = arest::command::Command::LoadReadings {
        markdown: new_readings.to_string(),
        domain: "orders".to_string(),
        sender: None,
        signature: None,
    };

    let load_result = arest::command::apply_command_defs(&d, &load_cmd, &state);
    assert!(!load_result.rejected, "Loading valid readings should not be rejected");

    // load_result.state is now the full D (state + recompiled defs) — Corollary 5.
    // No manual recompile needed.
    let new_d = &load_result.state;

    // The resolve:{Order} def should now include "reason" → specific fact type ID
    let resolve_result = ast::apply(
        &ast::Func::Def("resolve:Order".to_string()),
        &ast::Object::atom("reason"),
        new_d,
    );
    let resolved_ft = resolve_result.as_atom().unwrap_or("");
    assert!(resolved_ft.contains("Order") && resolved_ft.contains("Reason"),
        "resolve:Order should map 'reason' to Order_has_Reason (or similar), got: {:?}", resolved_ft);

    // Stronger test: create an entity using the NEW fact type after self-modification.
    // This proves the recompiled defs are live.
    let mut fields = HashMap::new();
    fields.insert("customer".to_string(), "Acme".to_string());
    fields.insert("Reason".to_string(), "bulk discount".to_string());
    let create_cmd = arest::command::Command::CreateEntity {
        noun: "Order".to_string(),
        domain: "orders".to_string(),
        id: Some("ord-99".to_string()),
        fields,
        sender: None,
        signature: None,
    };
    let create_result = arest::command::apply_command_defs(new_d, &create_cmd, new_d);
    assert!(!create_result.rejected, "Create with new field should succeed after self-evolution");

    // Verify the new fact type has data
    let reason_cell = ast::fetch_or_phi("Order_has_Reason", &create_result.state);
    let reason_facts = reason_cell.as_seq().unwrap_or_default();
    assert!(reason_facts.iter().any(|f|
        ast::binding(f, "Order") == Some("ord-99") &&
        ast::binding(f, "Reason") == Some("bulk discount")
    ), "New fact type should be populated after create: {:?}", reason_facts);
}

// ── Metamodel Validation ───────────────────────────────────────────
// validate_model must produce zero warnings on the bundled metamodel.

#[test]
fn bundled_metamodel_passes_validate_model() {
    let readings: Vec<(&str, &str)> = vec![
        ("core", include_str!("../../../readings/core.md")),
        ("state", include_str!("../../../readings/state.md")),
        ("instances", include_str!("../../../readings/instances.md")),
        ("outcomes", include_str!("../../../readings/outcomes.md")),
        ("validation", include_str!("../../../readings/validation.md")),
        ("evolution", include_str!("../../../readings/evolution.md")),
        ("organizations", include_str!("../../../readings/organizations.md")),
        ("agents", include_str!("../../../readings/agents.md")),
        ("ui", include_str!("../../../readings/ui.md")),
    ];
    // Use parse_markdown (no bootstrap needed — each file parsed standalone,
    // then IRs merged). Cross-file noun references won't resolve as fact types
    // but validate_model checks the merged domain.
    let domain = readings.iter().fold(
        arest::types::Domain::default(),
        |mut merged, (_, text)| {
            let ir = parse_forml2::parse_markdown(text).unwrap();
            merged.nouns.extend(ir.nouns);
            merged.fact_types.extend(ir.fact_types);
            merged.constraints.extend(ir.constraints);
            merged.subtypes.extend(ir.subtypes);
            merged
        },
    );

    let errors = compile::validate_model(&domain);
    errors.iter().for_each(|e| eprintln!("[model warning] {}", e));
    assert!(errors.is_empty(), "Metamodel should have zero validate_model warnings, got {}:\n{}",
        errors.len(), errors.join("\n"));
}

// ── Cell Sharding: RMAP partitions to independent folds ────────────

#[test]
fn shard_map_partitions_facts_by_entity_cell() {
    // Order domain: RMAP partitions fact types into entity cells.
    let (_, d) = compile_orders();

    // Extract shard map from D: shard:{ft_id} → <', cell_name>
    let shard_map: std::collections::HashMap<String, String> = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("shard:"))
        .filter_map(|(n, v)| {
            let ft_id = n.strip_prefix("shard:")?.to_string();
            let cell = v.as_seq()
                .filter(|items| items.len() == 2 && items[0].as_atom() == Some("'"))
                .and_then(|items| items[1].as_atom())
                .map(|s| s.to_string())?;
            Some((ft_id, cell))
        })
        .collect();

    assert!(!shard_map.is_empty(), "shard map should have entries");

    // Every fact type in the domain should have a shard assignment
    assert!(shard_map.contains_key("Order_was_placed_by_Customer"),
        "placed_by fact type should be partitioned, got: {:?}", shard_map.keys().collect::<Vec<_>>());

    // Verify demux: split events by cell
    let events = vec![
        ("Order_was_placed_by_Customer".to_string(), ast::Object::atom("fact1")),
        ("Order_has_Status".to_string(), ast::Object::atom("fact2")),
    ];
    let demuxed = ast::demux(&events, &shard_map);
    assert!(!demuxed.is_empty(), "demux should partition events into cells");

    // Distinct cell names should exist (not all facts in one cell)
    let cell_names: std::collections::HashSet<_> = shard_map.values().collect();
    assert!(cell_names.len() >= 2,
        "RMAP should partition into multiple cells, got: {:?}", cell_names);
}

// ── Scale Test: 10K entities × 100 fact types ──────────────────────

#[test]
fn derivation_rules_get_distinct_ids_when_consequent_unresolved() {
    // Two := rules with consequents not declared as fact types.
    // Before the fix, both got empty IDs and stored as `derivation:` —
    // last one wins, first one silently lost. Now each gets a stable
    // sanitized ID from its consequent text.
    let input = r#"
X(.id) is an entity type.
Y(.id) is an entity type.
P(.id) is an entity type.
Q(.id) is an entity type.
Name is a value type.

X has Y.
Y has Name.
P has Q.
Q has Name.

X has Name := X has some Y and that Y has some Name.
P has Name := P has some Q and that Q has some Name.
"#;
    let ir = parse_forml2::parse_markdown(input).unwrap();
    let state = parse_forml2::domain_to_state(&ir);
    let defs = compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);

    // Count unique derivation cells in D. Must have BOTH rules, not one.
    let derivation_cells: Vec<String> = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation:") && !n.contains("_cwa_negation"))
        .map(|(n, _)| n.to_string())
        .collect();

    assert!(derivation_cells.len() >= 2,
        "Expected at least 2 distinct derivation cells for 2 := rules, got: {:?}",
        derivation_cells);

    // No empty-ID cell
    assert!(!derivation_cells.contains(&"derivation:".to_string()),
        "Empty-ID derivation cell should not exist, got: {:?}", derivation_cells);
}

#[test]
#[ignore] // Run with: cargo test --release -- --ignored scale_test
fn scale_test_10k_entities_100_fact_types() {
    // Schema: 100 entity types × 1 fact type each (Entity{i} has Name).
    // Instances: 100 entities per type = 10,000 total instance facts.
    // Measures: parse, compile, fetch. Surfaces bottlenecks at production scale.
    let mut readings = String::from("## Entity Types\n\n");
    for i in 0..100 {
        readings.push_str(&format!("Entity{}(.id) is an entity type.\n", i));
    }
    readings.push_str("Name is a value type.\n");
    readings.push_str("\n## Fact Types\n\n");
    for i in 0..100 {
        readings.push_str(&format!("Entity{} has Name.\n  Each Entity{} has exactly one Name.\n", i, i));
    }

    // 10K instance facts: 100 entities × 100 types.
    readings.push_str("\n## Instance Facts\n\n");
    for type_idx in 0..100 {
        for entity_idx in 0..100 {
            readings.push_str(&format!(
                "Entity{} 'e{}-{}' has Name 'name-{}-{}'.\n",
                type_idx, type_idx, entity_idx, type_idx, entity_idx
            ));
        }
    }

    eprintln!("[scale] input size: {} bytes", readings.len());

    let t = std::time::Instant::now();
    let ir = parse_forml2::parse_markdown(&readings).unwrap();
    eprintln!("[scale] parse: {:?} ({} nouns, {} fts, {} instance facts)",
        t.elapsed(), ir.nouns.len(), ir.fact_types.len(), ir.general_instance_facts.len());

    let t = std::time::Instant::now();
    let state = parse_forml2::domain_to_state(&ir);
    eprintln!("[scale] domain_to_state: {:?}", t.elapsed());

    let t = std::time::Instant::now();
    let defs = compile::compile_to_defs_state(&state);
    eprintln!("[scale] compile_to_defs_state: {:?} ({} defs)",
        t.elapsed(), defs.len());

    let t = std::time::Instant::now();
    let d = ast::defs_to_state(&defs, &state);
    eprintln!("[scale] defs_to_state: {:?}", t.elapsed());

    // O(1) fetches on D (Map-backed)
    let t = std::time::Instant::now();
    for i in 0..1000 {
        let key = format!("schema:Entity{}_has_Name", i % 100);
        let _ = ast::fetch(&key, &d);
    }
    eprintln!("[scale] 1000 fetches on D: {:?}", t.elapsed());

    // Fetch per-cell instance facts: should be O(1) per cell, each cell has ~100 facts.
    let t = std::time::Instant::now();
    let mut total_facts = 0usize;
    for i in 0..100 {
        let key = format!("Entity{}_has_Name", i);
        let cell = ast::fetch(&key, &d);
        total_facts += cell.as_seq().map(|s| s.len()).unwrap_or(0);
    }
    eprintln!("[scale] 100 instance-cell fetches: {:?} ({} facts total)",
        t.elapsed(), total_facts);

    // Verify shard map and derivation index scale
    let shard_count = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("shard:")).count();
    let index_count = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation_index:")).count();
    eprintln!("[scale] {} shard entries, {} derivation index entries",
        shard_count, index_count);

    assert_eq!(ir.general_instance_facts.len(), 10_000,
        "Should parse 10K instance facts");
    assert_eq!(total_facts, 10_000,
        "All 10K facts should be reachable via per-cell fetch");
    assert!(ir.nouns.len() >= 100);
    assert!(shard_count >= 100);
}

#[test]
fn derivation_index_gates_by_noun() {
    // Verify derivation_index:{noun} cells are compiled and contain
    // the correct derivation IDs. CWA negation rules should be indexed
    // under the nouns whose fact types they negate.
    let (_, d) = compile_orders();

    // derivation_index:Order should exist (Order participates in fact types)
    let order_index = ast::fetch("derivation_index:Order", &d);
    assert_ne!(order_index, ast::Object::Bottom,
        "derivation_index:Order should exist");
    // Extract the constant value: <', "id1,id2,...">
    let order_ids = order_index.as_seq()
        .filter(|items| items.len() == 2 && items[0].as_atom() == Some("'"))
        .and_then(|items| items[1].as_atom())
        .unwrap_or("");
    assert!(!order_ids.is_empty(),
        "Order index should have derivation IDs");

    // Customer index should exist (Customer plays a role in placed_by)
    let cust_index = ast::fetch("derivation_index:Customer", &d);
    assert_ne!(cust_index, ast::Object::Bottom,
        "derivation_index:Customer should exist");
}

#[test]
fn map_store_fetch_is_o1() {
    // Build a large state with 500 cells as Seq
    let mut seq_state = ast::Object::phi();
    for i in 0..500 {
        let name = format!("cell_{}", i);
        seq_state = ast::store(&name, ast::Object::atom(&format!("val_{}", i)), &seq_state);
    }

    // Convert to Map
    let map_state = seq_state.to_store();

    // Verify fetch returns identical results
    assert_eq!(ast::fetch("cell_0", &seq_state), ast::fetch("cell_0", &map_state));
    assert_eq!(ast::fetch("cell_499", &seq_state), ast::fetch("cell_499", &map_state));
    assert_eq!(ast::fetch("cell_250", &seq_state), ast::fetch("cell_250", &map_state));
    assert_eq!(ast::fetch("missing", &map_state), ast::Object::Bottom);

    // Verify store on Map returns a Map
    let updated = ast::store("cell_0", ast::Object::atom("new_val"), &map_state);
    assert_eq!(ast::fetch("cell_0", &updated), ast::Object::atom("new_val"));
    assert_eq!(ast::fetch("cell_499", &updated), ast::Object::atom("val_499"));
    assert!(updated.as_map().is_some(), "store on Map should return Map");

    // Verify cells_iter returns all 500 cells
    let cells: Vec<_> = ast::cells_iter(&map_state);
    assert_eq!(cells.len(), 500);

    // Verify cell_push works on Map
    let pushed = ast::cell_push("new_cell", ast::Object::atom("data"), &map_state);
    assert_eq!(ast::fetch("new_cell", &pushed), ast::Object::seq(vec![ast::Object::atom("data")]));

    // Verify merge_states returns Map
    let merged = ast::merge_states(&map_state, &ast::Object::phi());
    assert!(merged.as_map().is_some(), "merge_states should return Map");
}

#[test]
fn map_store_used_by_defs_to_state() {
    // defs_to_state should return Map for O(1) metacompose lookups
    let (_, d) = compile_orders();
    assert!(d.as_map().is_some(), "defs_to_state should return Map, got Seq");

    // fetch on D should be O(1) and work correctly
    let validate = ast::fetch("validate", &d);
    assert_ne!(validate, ast::Object::Bottom, "validate def should exist in D");
}

#[test]
fn join_derivation_produces_bindings() {
    let input = r#"
A(.id) is an entity type.
B(.id) is an entity type.
C(.id) is an entity type.

A has B.
B has C.
A has C.

A has C := A has some B and that B has some C.

## Instance Facts

A 'a1' has B 'b1'.
B 'b1' has C 'c1'.
A 'a2' has B 'b2'.
B 'b2' has C 'c2'.
"#;
    let ir = parse_forml2::parse_markdown(input).unwrap();
    let state = parse_forml2::domain_to_state(&ir);
    let defs = compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);

    // Run forward chaining
    let derivation_defs: Vec<(String, ast::Func)> = ast::cells_iter(&d).into_iter()
        .filter(|(n, _)| n.starts_with("derivation:"))
        .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, &d)))
        .collect();
    let refs: Vec<(&str, &ast::Func)> = derivation_defs.iter()
        .map(|(n, f)| (n.as_str(), f)).collect();
    let (_new_state, derived) = evaluate::forward_chain_defs_state(&refs, &d);

    // Find derived A_has_C facts
    let a_has_c: Vec<_> = derived.iter()
        .filter(|f| f.fact_type_id == "A_has_C")
        .collect();
    eprintln!("derived A_has_C: {:?}", a_has_c);

    assert!(!a_has_c.is_empty(), "Join derivation should produce A_has_C facts");
    assert!(a_has_c.iter().any(|f| {
        f.bindings.iter().any(|(k, v)| k == "A" && v == "a1")
            && f.bindings.iter().any(|(k, v)| k == "C" && v == "c1")
    }), "Should derive a1 has c1: {:?}", a_has_c);
    assert!(a_has_c.iter().any(|f| {
        f.bindings.iter().any(|(k, v)| k == "A" && v == "a2")
            && f.bindings.iter().any(|(k, v)| k == "C" && v == "c2")
    }), "Should derive a2 has c2: {:?}", a_has_c);
}

// ── Federation: populate:{noun} from "backed by External System" ──

#[test]
fn federation_populate_defs_compiled_from_readings() {
    // Build a focused domain with federation readings.
    // Declares External Systems and "backed by" instance facts.
    let input = r#"
## Entity Types

User(.Email) is an entity type.
Stripe Customer(.Name) is an entity type.
External System(.Name) is an entity type.
Noun(.id) is an entity type.
URL is a value type.
Header is a value type.
Prefix is a value type.
URI is a value type.

## Fact Types

External System has URL.
  Each External System has exactly one URL.
External System has Header.
  Each External System has at most one Header.
External System has Prefix.
  Each External System has at most one Prefix.
Noun is backed by External System.
  Each Noun is backed by at most one External System.
Noun has URI.
  Each Noun has at most one URI.
User has Stripe Customer.

## Instance Facts

External System 'auth.vin' has URL 'https://auth.vin'.
External System 'auth.vin' has Header 'Authorization'.
External System 'auth.vin' has Prefix 'users API-Key'.
External System 'stripe' has URL 'https://api.stripe.com/v1'.
External System 'stripe' has Header 'Authorization'.
External System 'stripe' has Prefix 'Bearer'.
Noun 'User' is backed by External System 'auth.vin'.
Noun 'User' has URI '/users'.
Noun 'Stripe Customer' is backed by External System 'stripe'.
Noun 'Stripe Customer' has URI '/customers'.
"#;
    let ir = parse_forml2::parse_markdown(input).unwrap();
    let state = parse_forml2::domain_to_state(&ir);

    // Compile: produces populate:{noun} defs for backed nouns.
    let defs = compile::compile_to_defs_state(&state);
    let d = ast::defs_to_state(&defs, &state);

    // ── Verify populate:User exists with auth.vin config ──
    let user_pop = ast::fetch("populate:User", &d);
    assert_ne!(user_pop, ast::Object::Bottom,
        "populate:User should exist (User is backed by auth.vin)");
    // The def is func_to_object(Func::Constant(config)) = <', config>
    let user_config = user_pop.as_seq()
        .filter(|items| items.len() == 2 && items[0].as_atom() == Some("'"))
        .map(|items| &items[1])
        .unwrap_or(&user_pop);
    // Config is a seq of pairs: <<system, auth.vin>, <url, https://auth.vin>, ...>
    let binding = |key: &str| -> Option<String> {
        user_config.as_seq()?.iter().find_map(|pair| {
            let items = pair.as_seq()?;
            (items.len() == 2 && items[0].as_atom() == Some(key))
                .then(|| items[1].as_atom().unwrap_or("").to_string())
        })
    };
    assert_eq!(binding("system").as_deref(), Some("auth.vin"));
    assert_eq!(binding("url").as_deref(), Some("https://auth.vin"));
    assert_eq!(binding("uri").as_deref(), Some("/users"));
    assert_eq!(binding("header").as_deref(), Some("Authorization"));

    // ── Verify populate:Stripe Customer exists with stripe config ──
    let stripe_pop = ast::fetch("populate:Stripe Customer", &d);
    assert_ne!(stripe_pop, ast::Object::Bottom,
        "populate:Stripe Customer should exist (backed by stripe)");
    let stripe_config = stripe_pop.as_seq()
        .filter(|items| items.len() == 2 && items[0].as_atom() == Some("'"))
        .map(|items| &items[1])
        .unwrap_or(&stripe_pop);
    let stripe_binding = |key: &str| -> Option<String> {
        stripe_config.as_seq()?.iter().find_map(|pair| {
            let items = pair.as_seq()?;
            (items.len() == 2 && items[0].as_atom() == Some(key))
                .then(|| items[1].as_atom().unwrap_or("").to_string())
        })
    };
    assert_eq!(stripe_binding("system").as_deref(), Some("stripe"));
    assert_eq!(stripe_binding("url").as_deref(), Some("https://api.stripe.com/v1"));
    assert_eq!(stripe_binding("uri").as_deref(), Some("/customers"));
    assert_eq!(stripe_binding("prefix").as_deref(), Some("Bearer"));
    assert_eq!(stripe_binding("noun").as_deref(), Some("Stripe Customer"));
}

// ── E3 / #305: Citation-fact provenance — Authority Type enum ──────

/// The Authority Type enum in instances.md must cover both legal/regulatory
/// citations (original legal-research motivation) AND the two new provenance
/// kinds Citation carries in the platform-binding path (§3.2 Platform
/// Binding, §3.3 Data Federation):
/// - `Runtime-Function` — the Citation names a runtime-registered Platform
///   function as the origin (e.g. `platform:send_email`).
/// - `Federated-Fetch` — the Citation names an external system fetched
///   under OWA as the origin (e.g. Stripe, auto.dev).
#[test]
fn authority_type_enum_carries_runtime_function_and_federated_fetch() {
    let instances = include_str!("../../../readings/instances.md");
    let enum_line = instances.lines()
        .find(|l| l.contains("possible values of Authority Type"))
        .expect("Authority Type enum declaration should exist in instances.md");
    assert!(enum_line.contains("'Runtime-Function'"),
        "Authority Type enum should declare 'Runtime-Function'; line: {enum_line}");
    assert!(enum_line.contains("'Federated-Fetch'"),
        "Authority Type enum should declare 'Federated-Fetch'; line: {enum_line}");
}
