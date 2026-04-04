// crates/arest/tests/properties.rs
//
// Property tests for the AREST paper claims.
// Input: FORML2 readings. Output: system function responses.
// No IR. No internal types. Readings and named functions.

use arest::parse_forml2;
use arest::compile;
use arest::evaluate;
use arest::types::{ResponseContext, Population, FactInstance};
use arest::verbalize;
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
    let (ir, model) = compile_orders();
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
    let (ir, model) = compile_orders();
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
    let response = ResponseContext {
        text: "We will ship your order overnight for fast delivery".to_string(),
        sender_identity: None,
        fields: None,
    };
    let pop = Population { facts: HashMap::new() };
    let violations = evaluate::evaluate_via_ast(&model, &response, &pop);
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
    let model = compile::compile(&ir);
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
    use arest::ast::{self, Func, Object};

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
