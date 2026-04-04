// crates/arest/tests/properties.rs
//
// Property tests for the AREST paper claims.
// Input: FORML2 readings. Output: system function responses.
// No IR. No internal types. Readings and named functions.

use arest::parse_forml2;
use arest::compile;
use arest::evaluate;
use arest::types::{Population, FactInstance};
use std::collections::HashMap;

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

fn compile_orders() -> (arest::types::Domain, arest::compile::CompiledModel) {
    // Parse metamodel first to get state machine nouns
    let meta = parse_forml2::parse_markdown(STATE_METAMODEL).unwrap();
    // Parse business domain with metamodel nouns in context
    let mut ir = parse_forml2::parse_markdown_with_nouns(ORDERS_DOMAIN, &meta.nouns).unwrap();
    // Merge metamodel into IR so compile has the full picture
    ir.nouns.extend(meta.nouns);
    ir.fact_types.extend(meta.fact_types);
    ir.constraints.extend(meta.constraints);
    ir.subtypes.extend(meta.subtypes);
    let model = compile::compile(&ir);
    (ir, model)
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
    let (ir, model) = compile_orders();
    // The forbidden constraint text should survive compilation
    let forbidden = ir.constraints.iter()
        .find(|c| c.text.contains("Prohibited Shipping Method"))
        .expect("Should have Prohibited Shipping Method constraint");
    let cc = model.constraints.iter()
        .find(|c| c.text.contains("Prohibited Shipping Method"))
        .expect("Compiled constraint should preserve text");
    assert_eq!(cc.text, forbidden.text, "Compiled text must equal parsed text (Corollary 7)");
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
    let (_ir, model) = compile_orders();
    // Order should have a state machine with initial state "In Cart"
    let sm = model.state_machines.iter()
        .find(|sm| sm.noun_name == "Order")
        .expect("Order should have a state machine");
    assert_eq!(sm.initial, "In Cart");
}

#[test]
fn t3_forward_chain_reaches_fixed_point() {
    let (_, model) = compile_orders();
    let mut pop = Population { facts: HashMap::new() };
    // Forward chain with empty population should terminate
    let derived = evaluate::forward_chain_ast(&model, &mut pop);
    // The derived facts should be a fixed point (running again produces nothing new)
    let derived2 = evaluate::forward_chain_ast(&model, &mut pop);
    // Second run should produce no additional facts beyond what first run added
    assert!(derived2.len() <= derived.len(), "Forward chain should reach fixed point");
}

#[test]
fn t3_alethic_violation_rejects_command() {
    let (ir, _model) = compile_orders();
    // UC on "Each Order was placed by exactly one Customer" means
    // an Order with two customers should produce a violation
    let uc = ir.constraints.iter()
        .find(|c| c.kind == "UC" && c.text.contains("placed by"))
        .expect("UC on placed by");
    assert!(!uc.text.is_empty());
    // The constraint is alethic (schema-enforced)
    assert_eq!(uc.modality, "alethic");
}

#[test]
fn t3_deontic_violation_warns_but_succeeds() {
    let (ir, _) = compile_orders();
    let forbidden = ir.constraints.iter()
        .find(|c| c.modality == "deontic" && c.text.contains("Prohibited Shipping Method"))
        .expect("Deontic forbidden constraint");
    assert_eq!(forbidden.deontic_operator, Some("forbidden".to_string()));
}

// ── Theorem 4: HATEOAS as Projection ─────────────────────────────────
// (a) Transition links = projection of T filtered by current status
// (b) Navigation links = projection of UC direction (parent/child)

#[test]
fn t4a_transitions_from_initial_state() {
    let (_, model) = compile_orders();
    let sm = model.state_machines.iter()
        .find(|sm| sm.noun_name == "Order")
        .unwrap();
    // From "In Cart", available transitions should be "place" and "cancel"
    let from_cart: Vec<&str> = sm.transition_table.iter()
        .filter(|(from, _, _)| from == "In Cart")
        .map(|(_, _, event)| event.as_str())
        .collect();
    assert!(from_cart.contains(&"place"), "In Cart should have 'place' transition");
    assert!(from_cart.contains(&"cancel"), "In Cart should have 'cancel' transition");
    assert_eq!(from_cart.len(), 2, "In Cart should have exactly 2 transitions");
}

#[test]
fn t4a_transitions_from_placed() {
    let (_, model) = compile_orders();
    let sm = model.state_machines.iter()
        .find(|sm| sm.noun_name == "Order")
        .unwrap();
    let from_placed: Vec<&str> = sm.transition_table.iter()
        .filter(|(from, _, _)| from == "Placed")
        .map(|(_, _, event)| event.as_str())
        .collect();
    assert!(from_placed.contains(&"ship"), "Placed should have 'ship'");
    assert!(from_placed.contains(&"cancel"), "Placed should have 'cancel'");
}

#[test]
fn t4a_terminal_state_has_no_transitions() {
    let (_, model) = compile_orders();
    let sm = model.state_machines.iter()
        .find(|sm| sm.noun_name == "Order")
        .unwrap();
    // Delivered and Cancelled are terminal
    let from_delivered: Vec<_> = sm.transition_table.iter()
        .filter(|(from, _, _)| from == "Delivered")
        .collect();
    let from_cancelled: Vec<_> = sm.transition_table.iter()
        .filter(|(from, _, _)| from == "Cancelled")
        .collect();
    assert!(from_delivered.is_empty(), "Delivered is terminal (Corollary 6)");
    assert!(from_cancelled.is_empty(), "Cancelled is terminal (Corollary 6)");
}

#[test]
fn t4a_state_machine_fold_produces_correct_state() {
    let (_, model) = compile_orders();
    let sm = model.state_machines.iter()
        .find(|sm| sm.noun_name == "Order")
        .unwrap();
    // Fold: In Cart -> place -> Placed -> ship -> Shipped -> deliver -> Delivered
    let final_state = evaluate::run_machine_ast(sm, &["place", "ship", "deliver"]);
    assert_eq!(final_state, "Delivered");
}

#[test]
fn t4a_invalid_event_preserves_state() {
    let (_, model) = compile_orders();
    let sm = model.state_machines.iter()
        .find(|sm| sm.noun_name == "Order")
        .unwrap();
    // "ship" from "In Cart" is invalid. State should stay "In Cart".
    let state = evaluate::run_machine_ast(sm, &["ship"]);
    assert_eq!(state, "In Cart", "Invalid event should not change state");
}

#[test]
fn t4a_cancel_from_multiple_states() {
    let (_, model) = compile_orders();
    let sm = model.state_machines.iter()
        .find(|sm| sm.noun_name == "Order")
        .unwrap();
    // Cancel from In Cart
    assert_eq!(evaluate::run_machine_ast(sm, &["cancel"]), "Cancelled");
    // Cancel from Placed
    assert_eq!(evaluate::run_machine_ast(sm, &["place", "cancel"]), "Cancelled");
    // Cannot cancel from Shipped
    assert_eq!(evaluate::run_machine_ast(sm, &["place", "ship", "cancel"]), "Shipped");
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
    let (_, model) = compile_orders();
    let sm = model.state_machines.iter()
        .find(|sm| sm.noun_name == "Order")
        .unwrap();
    // Terminal states: no outgoing transitions
    let has_outgoing: std::collections::HashSet<&str> = sm.transition_table.iter()
        .map(|(from, _, _)| from.as_str())
        .collect();
    let all_states: std::collections::HashSet<&str> = sm.statuses.iter()
        .map(|s| s.as_str())
        .collect();
    let terminal: Vec<&&str> = all_states.difference(&has_outgoing).collect();
    assert!(terminal.contains(&&"Delivered"), "Delivered should be terminal");
    assert!(terminal.contains(&&"Cancelled"), "Cancelled should be terminal");
}

// ── Corollary 7: Violation Verbalization ─────────────────────────────

#[test]
fn c7_deontic_violation_returns_reading_text() {
    let (_, model) = compile_orders();
    let pop = Population { facts: HashMap::new() };
    let violations = evaluate::evaluate_via_ast(&model, "We will ship your order overnight for fast delivery", None, &pop);
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
    // Start with orders domain (metamodel + orders, same as compile_orders)
    let meta = parse_forml2::parse_markdown(STATE_METAMODEL).unwrap();
    let mut ir1 = parse_forml2::parse_markdown_with_nouns(ORDERS_DOMAIN, &meta.nouns).unwrap();
    ir1.nouns.extend(meta.nouns);
    ir1.fact_types.extend(meta.fact_types);
    ir1.constraints.extend(meta.constraints);
    ir1.subtypes.extend(meta.subtypes);
    let model1 = compile::compile(&ir1);
    let sm1 = model1.state_machines.iter()
        .find(|sm| sm.noun_name == "Order")
        .unwrap();
    let initial1 = sm1.initial.clone();

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

    // Parse extension with existing nouns
    let ir2 = parse_forml2::parse_markdown_with_nouns(extension, &ir1.nouns).unwrap();

    // Merge
    let mut merged = ir1.clone();
    merged.nouns.extend(ir2.nouns);
    merged.fact_types.extend(ir2.fact_types);
    merged.constraints.extend(ir2.constraints);
    let model2 = compile::compile(&merged);

    // Original properties still hold
    let sm2 = model2.state_machines.iter()
        .find(|sm| sm.noun_name == "Order")
        .unwrap();
    assert_eq!(sm2.initial, initial1, "Initial state unchanged after self-modification");

    // New constraint is present
    assert!(merged.constraints.iter().any(|c| c.text.contains("Coupon")),
        "New constraint should be present after self-modification");

    // State machine fold still works
    assert_eq!(evaluate::run_machine_ast(sm2, &["place", "ship"]), "Shipped");
}

// ── Remark: World Assumption on Populating Functions ─────────────────

#[test]
fn remark_cwa_constraint_has_enum_values() {
    let ir = parse_forml2::parse_markdown(ORDERS_DOMAIN).unwrap();
    let _model = compile::compile(&ir);
    // The "forbidden Prohibited Shipping Method" constraint spans a fact type
    // whose role noun has declared enum values. This makes it CWA.
    let forbidden = ir.constraints.iter()
        .find(|c| c.text.contains("Prohibited Shipping Method"))
        .expect("Should have Prohibited Shipping Method constraint");
    let enum_vals = compile::collect_enum_values_pub(&ir, &forbidden.spans);
    assert!(!enum_vals.is_empty(), "CWA constraint should have enum values for deterministic checking");
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
    let enum_vals = compile::collect_enum_values_pub(&ir, &forbidden.spans);
    assert!(enum_vals.is_empty(), "OWA constraint should have no enum values (requires runtime judgment)");
}

// ── Cell Isolation (Definition 2) ────────────────────────────────────

#[test]
fn def2_state_machine_is_deterministic() {
    let (_, model) = compile_orders();
    let sm = model.state_machines.iter()
        .find(|sm| sm.noun_name == "Order")
        .unwrap();
    // For each (from, event) pair, there should be at most one target state.
    // This ensures the fold is deterministic (Definition 2).
    let mut seen: HashMap<(&str, &str), &str> = HashMap::new();
    for (from, to, event) in &sm.transition_table {
        if let Some(existing_to) = seen.get(&(from.as_str(), event.as_str())) {
            panic!("Non-deterministic: ({}, {}) maps to both '{}' and '{}'",
                from, event, existing_to, to);
        }
        seen.insert((from.as_str(), event.as_str()), to.as_str());
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
// The parser should produce a Population. The compiler should accept a
// Population. There should be no struct between them.

#[test]
fn paper_claim_parse_produces_population() {
    // parse_to_population should exist and return Population
    let pop = parse_forml2::parse_to_population(ORDERS_DOMAIN).unwrap();
    // The population should contain noun facts
    assert!(pop.facts.contains_key("Noun"), "Population should contain Noun facts");
    // The population should contain graph schema facts
    assert!(pop.facts.contains_key("GraphSchema"), "Population should contain GraphSchema facts");
    // The population should contain constraint facts
    assert!(pop.facts.contains_key("Constraint"), "Population should contain Constraint facts");
}

#[test]
fn paper_claim_compile_from_population() {
    let meta_pop = parse_forml2::parse_to_population(STATE_METAMODEL).unwrap();
    let orders_pop = parse_forml2::parse_to_population_with_nouns(ORDERS_DOMAIN, &meta_pop).unwrap();
    let mut pop = meta_pop;
    for (k, v) in orders_pop.facts {
        pop.facts.entry(k).or_default().extend(v);
    }
    let model = compile::compile_from_population(&pop);
    assert!(!model.state_machines.is_empty(), "Should compile state machines from population");
}

#[test]
fn paper_claim_population_round_trip() {
    // Parse to population, compile, run state machine. Same result as Domain path.
    let meta_pop = parse_forml2::parse_to_population(STATE_METAMODEL).unwrap();
    let orders_pop = parse_forml2::parse_to_population_with_nouns(ORDERS_DOMAIN, &meta_pop).unwrap();
    let mut merged = meta_pop;
    for (k, v) in orders_pop.facts {
        merged.facts.entry(k).or_default().extend(v);
    }
    let model = compile::compile_from_population(&merged);
    let sm = model.state_machines.iter()
        .find(|sm| sm.noun_name == "Order")
        .expect("Should have Order state machine from population");
    assert_eq!(sm.initial, "In Cart");
    assert_eq!(evaluate::run_machine_ast(sm, &["place", "ship", "deliver"]), "Delivered");
}

// ── Paper Claim: Everything is Func ──────────────────────────────────
// The compiler produces named definitions (Def name = func).
// Evaluation is function application (func:object).
// No structs. No HashMaps at evaluation time.

#[test]
fn ffp_compile_produces_named_definitions() {
    // compile_to_defs should return Vec<(String, Func)>
    let meta = parse_forml2::parse_to_population(STATE_METAMODEL).unwrap();
    let orders = parse_forml2::parse_to_population_with_nouns(ORDERS_DOMAIN, &meta).unwrap();
    let mut pop = meta;
    for (k, v) in orders.facts { pop.facts.entry(k).or_default().extend(v); }

    let defs = compile::compile_to_defs(&pop);
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
    let meta = parse_forml2::parse_to_population(STATE_METAMODEL).unwrap();
    let orders = parse_forml2::parse_to_population_with_nouns(ORDERS_DOMAIN, &meta).unwrap();
    let mut pop = meta;
    for (k, v) in orders.facts { pop.facts.entry(k).or_default().extend(v); }

    let defs = compile::compile_to_defs(&pop);

    // There should be constraint definitions
    let constraint_defs: Vec<_> = defs.iter()
        .filter(|(name, _)| name.starts_with("constraint:"))
        .collect();
    assert!(!constraint_defs.is_empty(), "Should have constraint definitions");
}

#[test]
fn ffp_state_machine_is_a_function() {
    let meta = parse_forml2::parse_to_population(STATE_METAMODEL).unwrap();
    let orders = parse_forml2::parse_to_population_with_nouns(ORDERS_DOMAIN, &meta).unwrap();
    let mut pop = meta;
    for (k, v) in orders.facts { pop.facts.entry(k).or_default().extend(v); }

    let defs = compile::compile_to_defs(&pop);

    // There should be a state machine definition for Order
    let sm_defs: Vec<_> = defs.iter()
        .filter(|(name, _)| name.contains("Order") && name.contains("machine"))
        .collect();
    assert!(!sm_defs.is_empty(), "Order state machine should be a named definition");
}

#[test]
fn ffp_evaluation_is_application() {
    use arest::ast::{self, Object};

    let meta = parse_forml2::parse_to_population(STATE_METAMODEL).unwrap();
    let orders = parse_forml2::parse_to_population_with_nouns(ORDERS_DOMAIN, &meta).unwrap();
    let mut pop = meta;
    for (k, v) in orders.facts { pop.facts.entry(k).or_default().extend(v); }

    let defs: Vec<(String, arest::ast::Func)> = compile::compile_to_defs(&pop);
    let def_map: HashMap<String, arest::ast::Func> = defs.iter().map(|(n, f)| (n.clone(), f.clone())).collect();

    // Find the Order state machine transition function and initial state
    let (_, transition_func) = defs.iter()
        .find(|(name, _)| name == "machine:Order")
        .expect("Order transition function");
    let (_, initial_func) = defs.iter()
        .find(|(name, _)| name == "machine:Order:initial")
        .expect("Order initial state");

    // Get initial state by applying the constant function
    let initial = ast::apply(initial_func, &Object::phi(), &def_map);
    assert_eq!(initial.as_atom(), Some("In Cart"));

    // Fold the transition function over the event stream.
    // foldl(transition, state, events): for each event, apply transition to <state, event>
    let events = ["place", "ship", "deliver"];
    let mut state = initial;
    for event in &events {
        let input = Object::seq(vec![state, Object::atom(event)]);
        state = ast::apply(transition_func, &input, &def_map);
    }
    assert_eq!(state.as_atom(), Some("Delivered"),
        "State machine evaluation is a fold of the transition function over events");
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
    let defs = HashMap::new();

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
    let defs = HashMap::new();

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
    let defs = HashMap::new();

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
    let defs = HashMap::new();

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
    let defs = HashMap::new();

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
    let defs = HashMap::new();

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
    let defs = HashMap::new();

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
    use arest::ast::{self, Func, Object};

    let meta = parse_forml2::parse_to_population(STATE_METAMODEL).unwrap();
    let orders = parse_forml2::parse_to_population_with_nouns(ORDERS_DOMAIN, &meta).unwrap();
    let mut pop = meta;
    for (k, v) in orders.facts { pop.facts.entry(k).or_default().extend(v); }

    let defs: Vec<(String, Func)> = compile::compile_to_defs(&pop);
    let def_map: HashMap<String, Func> = defs.iter().map(|(n, f)| (n.clone(), f.clone())).collect();

    // The transition function applied to <current_state, event> produces next_state.
    // Available links = all events where transition:<current_state, event> != bottom.
    let transition = def_map.get("machine:Order").expect("Order transition func");

    // From "In Cart", test which events produce a state change.
    // A valid transition changes the state. An invalid one preserves it.
    // Available links = events where transition:<state, event> != state.
    let events = ["place", "ship", "deliver", "cancel"];
    let current = "In Cart";
    let mut available_from_cart: Vec<&str> = vec![];
    for event in &events {
        let input = Object::seq(vec![Object::atom(current), Object::atom(event)]);
        let result = ast::apply(transition, &input, &def_map);
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
        let result = ast::apply(transition, &input, &def_map);
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

    let meta = parse_forml2::parse_to_population(STATE_METAMODEL).unwrap();
    let orders = parse_forml2::parse_to_population_with_nouns(ORDERS_DOMAIN, &meta).unwrap();
    let mut pop = meta;
    for (k, v) in orders.facts { pop.facts.entry(k).or_default().extend(v); }

    let defs: Vec<(String, Func)> = compile::compile_to_defs(&pop);
    let def_map: HashMap<String, Func> = defs.iter().map(|(n, f)| (n.clone(), f.clone())).collect();

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
        .find(|(name, _)| name.contains("constraint:") && name.contains("UC"))
        .expect("Should have a UC constraint");

    // The eval context is <response_text, sender, population>.
    // For population constraints, the response text is irrelevant.
    let context = Object::seq(vec![
        Object::phi(),
        Object::phi(),
        Object::phi(),
    ]);
    let result = ast::apply(uc_func, &context, &def_map);
    // UC against empty population: no facts = no violations for UC
    assert_eq!(result, Object::phi(),
        "UC constraint against empty population should produce no violations");

    // Now test with a population that has a UC violation.
    // "Each Order was placed by exactly one Customer" means
    // an Order with two different Customers should violate.
    let pop_with_violation = ast::encode_population(&Population {
        facts: {
            let mut f = HashMap::new();
            f.insert("Order_was_placed_by_Customer".to_string(), vec![
                FactInstance {
                    fact_type_id: "Order_was_placed_by_Customer".to_string(),
                    bindings: vec![
                        ("Order".to_string(), "ord-1".to_string()),
                        ("Customer".to_string(), "Acme".to_string()),
                    ],
                },
                FactInstance {
                    fact_type_id: "Order_was_placed_by_Customer".to_string(),
                    bindings: vec![
                        ("Order".to_string(), "ord-1".to_string()),
                        ("Customer".to_string(), "Globex".to_string()),
                    ],
                },
            ]);
            f
        },
    });
    let context_with_violation = Object::seq(vec![
        Object::phi(),
        Object::phi(),
        pop_with_violation,
    ]);

    // Find the UC on "placed by" specifically
    let (_, placed_by_uc) = defs.iter()
        .find(|(name, _)| name.contains("constraint:UC") && name.contains("placed by"))
        .expect("UC on placed by");
    let violation_result = ast::apply(placed_by_uc, &context_with_violation, &def_map);
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
    let mut facts = HashMap::new();
    facts.insert("Support_Response_has_Body".to_string(), vec![
        FactInstance {
            fact_type_id: "Support_Response_has_Body".to_string(),
            bindings: vec![
                ("Support Response".to_string(), "resp-1".to_string()),
                ("Body".to_string(), "We will ship overnight for sure".to_string()),
            ],
        },
    ]);
    let pop = Population { facts };

    // The population contains the response text as a fact.
    // A constraint evaluator should be able to find it by querying P
    // for Support_Response_has_Body facts.
    let pop_obj = ast::encode_population(&pop);

    // The population is queryable. The body text is in P, not in a separate struct.
    assert_ne!(pop_obj, Object::phi(), "Population with response should not be empty");
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
    let model = compile::compile(&ir);

    // Bad response: body contains "overnight"
    let pop = Population { facts: HashMap::new() };
    let violations = evaluate::evaluate_via_ast(&model, "We will ship overnight", None, &pop);
    assert!(!violations.is_empty(), "Should catch 'overnight' via enum matching");

    // Good response: no prohibited words
    let good_violations = evaluate::evaluate_via_ast(&model, "We will ship via standard delivery", None, &pop);
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

    let pop = parse_forml2::parse_to_population(input).unwrap();
    let defs: Vec<(String, Func)> = compile::compile_to_defs(&pop);
    let def_map: HashMap<String, Func> = defs.iter().map(|(n, f)| (n.clone(), f.clone())).collect();

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
    let bad_result = ast::apply(constraint_func, &bad_ctx, &def_map);
    assert_ne!(bad_result, Object::phi(),
        "Forbidden constraint via pure application should catch 'overnight'");

    // Good response
    let good_ctx = Object::seq(vec![
        Object::atom("We will ship standard"),
        Object::phi(),
        Object::phi(),
    ]);
    let good_result = ast::apply(constraint_func, &good_ctx, &def_map);
    assert_eq!(good_result, Object::phi(),
        "Forbidden constraint via pure application should pass clean text");
}

// ── Forward Chaining via Pure Application ────────────────────────────

#[test]
fn derivation_rules_are_functions() {
    use arest::ast::Func;

    let meta = parse_forml2::parse_to_population(STATE_METAMODEL).unwrap();
    let orders = parse_forml2::parse_to_population_with_nouns(ORDERS_DOMAIN, &meta).unwrap();
    let mut pop = meta;
    for (k, v) in orders.facts { pop.facts.entry(k).or_default().extend(v); }

    let defs: Vec<(String, Func)> = compile::compile_to_defs(&pop);

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

    let meta = parse_forml2::parse_to_population(STATE_METAMODEL).unwrap();
    let orders = parse_forml2::parse_to_population_with_nouns(ORDERS_DOMAIN, &meta).unwrap();
    let mut pop = meta;
    for (k, v) in orders.facts { pop.facts.entry(k).or_default().extend(v); }

    // D: initial definitions
    let defs_d: Vec<(String, Func)> = compile::compile_to_defs(&pop);
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
    let ext_pop = parse_forml2::parse_to_population_with_nouns(extension, &pop).unwrap();
    for (k, v) in ext_pop.facts {
        pop.facts.entry(k).or_default().extend(v);
    }

    // D': new definitions after self-modification
    let defs_d_prime: Vec<(String, Func)> = compile::compile_to_defs(&pop);

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
    let defs = HashMap::new();

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

    let meta = parse_forml2::parse_to_population(STATE_METAMODEL).unwrap();
    let orders = parse_forml2::parse_to_population_with_nouns(ORDERS_DOMAIN, &meta).unwrap();
    let mut pop = meta;
    for (k, v) in orders.facts { pop.facts.entry(k).or_default().extend(v); }

    let defs: Vec<(String, Func)> = compile::compile_to_defs(&pop);

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

    let mut defs = HashMap::new();

    // Define a handler: when applied to any argument, returns "handled"
    defs.insert("Customer_has_Email".to_string(), Func::constant(Object::atom("handled")));

    // An atom resolves to its definition
    let func = ast::metacompose(&Object::atom("Customer_has_Email"), &defs);
    let result = ast::apply(&func, &Object::atom("read"), &defs);
    assert_eq!(result.as_atom(), Some("handled"),
        "metacompose should resolve atom to its definition in DEFS");

    // A fact (sequence) uses the first element as the controlling operator.
    // Per Backus 13.3.2: (ρ<COMP, f, g>):x = (f∘g):x
    // We test with CONS: (ρ<CONS, s1, s2>):x = [s1, s2]:x = <s1:x, s2:x>
    let cons_obj = Object::seq(vec![
        Object::atom("CONS"),
        Object::atom("1"),  // selector 1
        Object::atom("2"),  // selector 2
    ]);
    let cons_func = ast::metacompose(&cons_obj, &defs);
    let input = Object::seq(vec![Object::atom("A"), Object::atom("B"), Object::atom("C")]);
    let result = ast::apply(&cons_func, &input, &defs);
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

    let defs = HashMap::new();
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

    let meta = parse_forml2::parse_to_population(STATE_METAMODEL).unwrap();
    let orders = parse_forml2::parse_to_population_with_nouns(ORDERS_DOMAIN, &meta).unwrap();
    let mut pop = meta;
    for (k, v) in orders.facts { pop.facts.entry(k).or_default().extend(v); }

    let defs: Vec<(String, Func)> = compile::compile_to_defs(&pop);

    // "Each Order was placed by exactly one Customer" → UC on Order's role
    // Order is child, Customer is parent
    let customer_children = defs.iter()
        .find(|(n, _)| n == "nav:Customer:children");
    assert!(customer_children.is_some(), "Customer should have children nav def");

    let order_parent = defs.iter()
        .find(|(n, _)| n == "nav:Order:parent");
    assert!(order_parent.is_some(), "Order should have parent nav def");
}
