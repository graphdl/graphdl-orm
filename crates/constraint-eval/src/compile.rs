// crates/constraint-eval/src/compile.rs
//
// Compilation: ConstraintIR → CompiledModel
//
// Following exec-symbols: constraints ARE predicates, not data that gets matched.
// The match on constraint kind happens once at compile time. After compilation,
// evaluation is pure function application — no dispatch, no branching on kind.
//
// exec-symbols architecture:
//   Constraint(modality)(predicate)
//   evaluate_constraint(constraint)(population)
//   StateMachine(transition)(initial)
//   run_machine(machine)(stream) = fold(transition)(initial)(stream)

use std::sync::Arc;
use std::collections::{HashMap, HashSet};
use crate::types::*;

// ── Core Functional Types ──────────────────────────────────────────

/// A predicate is a pure function from evaluation context to violations.
/// This is the fundamental type. Constraints ARE predicates.
pub type Predicate = Arc<dyn Fn(&EvalContext) -> Vec<Violation> + Send + Sync>;

/// Immutable evaluation context — the only input predicates receive.
pub struct EvalContext<'a> {
    pub response: &'a ResponseContext,
    pub population: &'a Population,
}

#[derive(Clone, Debug)]
pub enum Modality {
    Alethic,
    Deontic(DeonticOp),
}

#[derive(Clone, Debug)]
pub enum DeonticOp {
    Forbidden,
    Obligatory,
    Permitted,
}

/// A compiled constraint: identity + modality + predicate.
pub struct CompiledConstraint {
    pub id: String,
    pub text: String,
    pub modality: Modality,
    pub predicate: Predicate,
}

/// A compiled state machine: transition function + initial state.
/// exec-symbols: StateMachine(transition)(initial)
pub struct CompiledStateMachine {
    pub noun_name: String,
    pub initial: String,
    /// Transition: (current_state, event, ctx) → Option<next_state>
    /// Guard passes iff guard predicate produces zero violations.
    pub transition: Arc<dyn Fn(&str, &str, &EvalContext) -> Option<String> + Send + Sync>,
}

/// The compiled model — all constraints and state machines as executable functions.
pub struct CompiledModel {
    pub constraints: Vec<CompiledConstraint>,
    pub state_machines: Vec<CompiledStateMachine>,
}

// ── Population Primitives ──────────────────────────────────────────
// Composable building blocks. Each is a pure function over Population.

/// All instances of a noun across the entire population.
fn instances_of(noun_name: &str, population: &Population) -> HashSet<String> {
    population.facts.values()
        .flat_map(|facts| facts.iter())
        .flat_map(|f| &f.bindings)
        .filter(|(name, _)| name == noun_name)
        .map(|(_, val)| val.clone())
        .collect()
}

/// Whether an entity instance participates in a specific fact type.
fn participates_in(entity: &str, noun_name: &str, fact_type_id: &str, population: &Population) -> bool {
    population.facts.get(fact_type_id).map_or(false, |facts| {
        facts.iter().any(|f| {
            f.bindings.iter().any(|(name, val)| name == noun_name && val == entity)
        })
    })
}

// ── Span Resolution ────────────────────────────────────────────────
// Resolves IR references at compile time so predicates capture only what they need.

#[derive(Clone)]
struct ResolvedSpan {
    fact_type_id: String,
    noun_name: String,
    reading: String,
}

fn resolve_spans(ir: &ConstraintIR, spans: &[SpanDef]) -> Vec<ResolvedSpan> {
    spans.iter().filter_map(|span| {
        let ft = ir.fact_types.get(&span.fact_type_id)?;
        let role = ft.roles.get(span.role_index)?;
        Some(ResolvedSpan {
            fact_type_id: span.fact_type_id.clone(),
            noun_name: role.noun_name.clone(),
            reading: ft.reading.clone(),
        })
    }).collect()
}

/// Collect (noun_name, enum_values) for value-type nouns in spanned fact types.
fn collect_enum_values(ir: &ConstraintIR, spans: &[SpanDef]) -> Vec<(String, Vec<String>)> {
    let mut result = Vec::new();
    for span in spans {
        if let Some(ft) = ir.fact_types.get(&span.fact_type_id) {
            for role in &ft.roles {
                if let Some(noun_def) = ir.nouns.get(&role.noun_name) {
                    if noun_def.object_type == "value" {
                        if let Some(vals) = &noun_def.enum_values {
                            if !vals.is_empty() {
                                result.push((role.noun_name.clone(), vals.clone()));
                            }
                        }
                    }
                }
            }
        }
    }
    result
}

// ── Compilation ────────────────────────────────────────────────────
// The match on kind happens here, once. After this, everything is Predicate.

/// Compile an entire ConstraintIR into executable form.
pub fn compile(ir: &ConstraintIR) -> CompiledModel {
    let constraints: Vec<CompiledConstraint> = ir.constraints.iter()
        .map(|def| compile_constraint(ir, def))
        .collect();

    let constraint_predicates: HashMap<String, Predicate> = constraints.iter()
        .map(|c| (c.id.clone(), c.predicate.clone()))
        .collect();

    let state_machines = ir.state_machines.values()
        .map(|sm_def| compile_state_machine(sm_def, &constraint_predicates))
        .collect();

    CompiledModel { constraints, state_machines }
}

fn compile_constraint(ir: &ConstraintIR, def: &ConstraintDef) -> CompiledConstraint {
    let modality = match def.modality.as_str() {
        "Deontic" => {
            let op = match def.deontic_operator.as_deref() {
                Some("forbidden") => DeonticOp::Forbidden,
                Some("obligatory") => DeonticOp::Obligatory,
                Some("permitted") => DeonticOp::Permitted,
                _ => DeonticOp::Obligatory,
            };
            Modality::Deontic(op)
        }
        _ => Modality::Alethic,
    };

    let predicate = match &modality {
        Modality::Deontic(DeonticOp::Permitted) => {
            Arc::new(|_: &EvalContext| Vec::new()) as Predicate
        }
        Modality::Deontic(DeonticOp::Forbidden) => compile_forbidden(ir, def),
        Modality::Deontic(DeonticOp::Obligatory) => compile_obligatory(ir, def),
        Modality::Alethic => match def.kind.as_str() {
            "UC" => compile_uniqueness(ir, def),
            "MC" => compile_mandatory(ir, def),
            "RC" => compile_ring(ir, def),
            "XO" => compile_set_comparison(ir, def, |n| n != 1, "exactly one"),
            "XC" => compile_set_comparison(ir, def, |n| n > 1, "at most one"),
            "OR" => compile_set_comparison(ir, def, |n| n < 1, "at least one"),
            "SS" => compile_subset(ir, def),
            "EQ" => compile_equality(ir, def),
            _ => Arc::new(|_: &EvalContext| Vec::new()) as Predicate,
        },
    };

    CompiledConstraint {
        id: def.id.clone(),
        text: def.text.clone(),
        modality,
        predicate,
    }
}

// ── Alethic Predicates ─────────────────────────────────────────────
// Each returns a Predicate that captures all needed data from the IR.

fn compile_uniqueness(_ir: &ConstraintIR, def: &ConstraintDef) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();
    let spans = resolve_spans(_ir, &def.spans);

    Arc::new(move |ctx: &EvalContext| {
        spans.iter().flat_map(|span| {
            let facts = ctx.population.facts.get(&span.fact_type_id)
                .map(|f| f.as_slice()).unwrap_or(&[]);

            let mut seen: HashMap<String, usize> = HashMap::new();
            for fact in facts {
                if let Some((_, val)) = fact.bindings.iter().find(|(name, _)| *name == span.noun_name) {
                    *seen.entry(val.clone()).or_insert(0) += 1;
                }
            }

            seen.into_iter()
                .filter(|(_, count)| *count > 1)
                .map(|(val, count)| Violation {
                    constraint_id: id.clone(),
                    constraint_text: text.clone(),
                    detail: format!(
                        "Uniqueness violation: {} '{}' appears {} times in {}",
                        span.noun_name, val, count, span.reading
                    ),
                })
                .collect::<Vec<_>>()
        }).collect()
    })
}

fn compile_mandatory(_ir: &ConstraintIR, def: &ConstraintDef) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();
    let spans = resolve_spans(_ir, &def.spans);

    Arc::new(move |ctx: &EvalContext| {
        spans.iter().flat_map(|span| {
            let facts = ctx.population.facts.get(&span.fact_type_id)
                .cloned().unwrap_or_default();

            // Collect all instances of this noun from ALL fact types
            let all_instances = instances_of(&span.noun_name, ctx.population);

            all_instances.into_iter()
                .filter(|instance| {
                    !facts.iter().any(|f| {
                        f.bindings.iter().any(|(name, val)| *name == span.noun_name && val == instance)
                    })
                })
                .map(|instance| Violation {
                    constraint_id: id.clone(),
                    constraint_text: text.clone(),
                    detail: format!(
                        "Mandatory violation: {} '{}' does not participate in {}",
                        span.noun_name, instance, span.reading
                    ),
                })
                .collect::<Vec<_>>()
        }).collect()
    })
}

fn compile_ring(_ir: &ConstraintIR, def: &ConstraintDef) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();
    let fact_type_ids: Vec<String> = def.spans.iter()
        .map(|s| s.fact_type_id.clone())
        .collect();

    Arc::new(move |ctx: &EvalContext| {
        fact_type_ids.iter().flat_map(|ft_id| {
            let facts = ctx.population.facts.get(ft_id)
                .map(|f| f.as_slice()).unwrap_or(&[]);

            facts.iter().filter_map(|fact| {
                if fact.bindings.len() >= 2 && fact.bindings[0].1 == fact.bindings[1].1 {
                    Some(Violation {
                        constraint_id: id.clone(),
                        constraint_text: text.clone(),
                        detail: format!(
                            "Ring constraint violation: '{}' references itself",
                            fact.bindings[0].1
                        ),
                    })
                } else {
                    None
                }
            }).collect::<Vec<_>>()
        }).collect()
    })
}

fn compile_set_comparison(
    _ir: &ConstraintIR,
    def: &ConstraintDef,
    violates: fn(usize) -> bool,
    requirement: &'static str,
) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();
    let entity_name = def.entity.clone().unwrap_or_default();
    let clause_ft_ids: Vec<String> = def.spans.iter()
        .map(|s| s.fact_type_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    Arc::new(move |ctx: &EvalContext| {
        let all_instances = instances_of(&entity_name, ctx.population);

        all_instances.into_iter()
            .filter_map(|instance| {
                let holding = clause_ft_ids.iter()
                    .filter(|ft_id| participates_in(&instance, &entity_name, ft_id, ctx.population))
                    .count();

                if violates(holding) {
                    Some(Violation {
                        constraint_id: id.clone(),
                        constraint_text: text.clone(),
                        detail: format!(
                            "Set-comparison violation: {} '{}' has {} of {} clause fact types holding, expected {}",
                            entity_name, instance, holding, clause_ft_ids.len(), requirement
                        ),
                    })
                } else {
                    None
                }
            })
            .collect()
    })
}

fn compile_subset(_ir: &ConstraintIR, def: &ConstraintDef) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();

    if def.spans.len() < 2 {
        return Arc::new(|_: &EvalContext| Vec::new());
    }

    let a_ft_id = def.spans[0].fact_type_id.clone();
    let b_ft_id = def.spans[1].fact_type_id.clone();

    // Resolve entity noun name from the first span's role
    let entity_name = _ir.fact_types.get(&a_ft_id)
        .and_then(|ft| ft.roles.get(def.spans[0].role_index))
        .map(|r| r.noun_name.clone())
        .unwrap_or_default();

    Arc::new(move |ctx: &EvalContext| {
        let a_facts = ctx.population.facts.get(&a_ft_id).cloned().unwrap_or_default();
        let b_facts = ctx.population.facts.get(&b_ft_id).cloned().unwrap_or_default();

        a_facts.iter().filter_map(|a_fact| {
            if let Some((_, entity_val)) = a_fact.bindings.iter().find(|(name, _)| *name == entity_name) {
                let b_holds = b_facts.iter().any(|bf| {
                    bf.bindings.iter().any(|(_, val)| val == entity_val)
                });
                if !b_holds {
                    Some(Violation {
                        constraint_id: id.clone(),
                        constraint_text: text.clone(),
                        detail: format!(
                            "Subset violation: entity '{}' has fact A but not fact B",
                            entity_val
                        ),
                    })
                } else {
                    None
                }
            } else {
                None
            }
        }).collect()
    })
}

fn compile_equality(_ir: &ConstraintIR, def: &ConstraintDef) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();

    if def.spans.len() < 2 {
        return Arc::new(|_: &EvalContext| Vec::new());
    }

    let a_ft_id = def.spans[0].fact_type_id.clone();
    let b_ft_id = def.spans[1].fact_type_id.clone();

    Arc::new(move |ctx: &EvalContext| {
        let a_facts = ctx.population.facts.get(&a_ft_id).cloned().unwrap_or_default();
        let b_facts = ctx.population.facts.get(&b_ft_id).cloned().unwrap_or_default();

        let a_entities: HashSet<String> = a_facts.iter()
            .flat_map(|f| f.bindings.iter().map(|(_, v)| v.clone()))
            .collect();

        let b_entities: HashSet<String> = b_facts.iter()
            .flat_map(|f| f.bindings.iter().map(|(_, v)| v.clone()))
            .collect();

        let mut violations = Vec::new();

        for entity in a_entities.difference(&b_entities) {
            violations.push(Violation {
                constraint_id: id.clone(),
                constraint_text: text.clone(),
                detail: format!("Equality violation: '{}' has fact A but not fact B", entity),
            });
        }

        for entity in b_entities.difference(&a_entities) {
            violations.push(Violation {
                constraint_id: id.clone(),
                constraint_text: text.clone(),
                detail: format!("Equality violation: '{}' has fact B but not fact A", entity),
            });
        }

        violations
    })
}

// ── Deontic Predicates ─────────────────────────────────────────────

fn compile_forbidden(ir: &ConstraintIR, def: &ConstraintDef) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();
    let forbidden_values = collect_enum_values(ir, &def.spans);

    // Extract key phrases from the constraint text for text-based matching.
    // "It is forbidden that Customer resells AutomotiveData to ThirdParty"
    // → extract nouns and verbs as keywords to detect violations.
    let text_keywords = extract_constraint_keywords(&text);

    Arc::new(move |ctx: &EvalContext| {
        let mut violations = Vec::new();
        let mut seen = HashSet::new();
        let lower_text = ctx.response.text.to_lowercase();

        // Enum-based check (exact value match)
        for (noun_name, enum_vals) in &forbidden_values {
            for val in enum_vals {
                let lower_val = val.to_lowercase();
                if lower_text.contains(&lower_val) {
                    let detail = format!(
                        "Response contains forbidden {}: '{}'",
                        noun_name, val
                    );
                    if seen.insert(detail.clone()) {
                        violations.push(Violation {
                            constraint_id: id.clone(),
                            constraint_text: text.clone(),
                            detail,
                        });
                    }
                }
            }
        }

        // Text-based check: if no enum values, check keyword co-occurrence
        if forbidden_values.is_empty() && !text_keywords.is_empty() {
            let matched: Vec<&str> = text_keywords.iter()
                .filter(|kw| lower_text.contains(kw.as_str()))
                .map(|s| s.as_str())
                .collect();
            // Trigger if majority of keywords found (suggests the response discusses the forbidden topic)
            if matched.len() > text_keywords.len() / 2 && matched.len() >= 2 {
                violations.push(Violation {
                    constraint_id: id.clone(),
                    constraint_text: text.clone(),
                    detail: format!(
                        "Response may violate: '{}' (matched keywords: {})",
                        text, matched.join(", ")
                    ),
                });
            }
        }

        violations
    })
}

/// Extract lowercase keywords from a deontic constraint text.
/// Strips the "It is forbidden/obligatory/permitted that" prefix,
/// then extracts PascalCase and multi-word noun phrases.
fn extract_constraint_keywords(text: &str) -> Vec<String> {
    let stripped = text
        .replace("It is forbidden that ", "")
        .replace("It is obligatory that ", "")
        .replace("It is permitted that ", "");

    let mut keywords = Vec::new();
    for word in stripped.split_whitespace() {
        let clean = word.trim_matches(|c: char| !c.is_alphanumeric());
        if clean.is_empty() { continue; }
        // Split PascalCase: AutomotiveData → automotive, data
        let mut current = String::new();
        for ch in clean.chars() {
            if ch.is_uppercase() && !current.is_empty() {
                let lower = current.to_lowercase();
                if lower.len() > 2 { keywords.push(lower); }
                current.clear();
            }
            current.push(ch);
        }
        if !current.is_empty() {
            let lower = current.to_lowercase();
            if lower.len() > 2 { keywords.push(lower); }
        }
    }

    // Deduplicate
    keywords.sort();
    keywords.dedup();
    // Filter out common stop words
    keywords.retain(|w| !matches!(w.as_str(), "the" | "that" | "for" | "and" | "with" | "without" | "using" | "has" | "have" | "into" | "from"));
    keywords
}

fn compile_obligatory(ir: &ConstraintIR, def: &ConstraintDef) -> Predicate {
    let id = def.id.clone();
    let text = def.text.clone();
    let obligatory_values = collect_enum_values(ir, &def.spans);
    let checks_sender = def.text.to_lowercase().contains("senderidentity");

    // For text-based obligatory constraints, the constraint text itself is included
    // in the compiled model for semantic evaluation by the LLM layer.
    // WASM flags it as a rule the response should acknowledge.
    let text_keywords = if obligatory_values.is_empty() {
        extract_constraint_keywords(&text)
    } else {
        Vec::new()
    };

    Arc::new(move |ctx: &EvalContext| {
        let mut violations = Vec::new();
        let lower_text = ctx.response.text.to_lowercase();

        // Enum-based check
        for (noun_name, enum_vals) in &obligatory_values {
            let found = enum_vals.iter().any(|val| lower_text.contains(&val.to_lowercase()));
            if !found {
                violations.push(Violation {
                    constraint_id: id.clone(),
                    constraint_text: text.clone(),
                    detail: format!(
                        "Response missing obligatory {}: expected one of {:?}",
                        noun_name, enum_vals
                    ),
                });
            }
        }

        // Sender identity check
        if checks_sender {
            if let Some(sender) = &ctx.response.sender_identity {
                if sender.is_empty() {
                    violations.push(Violation {
                        constraint_id: id.clone(),
                        constraint_text: text.clone(),
                        detail: "Response missing obligatory SenderIdentity".to_string(),
                    });
                }
            }
        }

        // Text-based: include the obligation as metadata so the LLM layer can evaluate
        // (WASM can't determine semantic compliance, but it flags the rule exists)
        let _ = &text_keywords; // available for future keyword checks

        violations
    })
}

// ── State Machine Compilation ──────────────────────────────────────
// exec-symbols: StateMachine(transition)(initial)
// run_machine(machine)(stream) = fold(transition)(initial)(stream)

struct CompiledTransition {
    from: String,
    to: String,
    event: String,
    guard: Predicate,
}

fn compile_state_machine(
    def: &StateMachineDef,
    constraint_predicates: &HashMap<String, Predicate>,
) -> CompiledStateMachine {
    let transitions: Vec<CompiledTransition> = def.transitions.iter()
        .map(|t| {
            let guard_preds: Vec<Predicate> = t.guard.as_ref()
                .map(|g| g.constraint_ids.iter()
                    .filter_map(|cid| constraint_predicates.get(cid).cloned())
                    .collect())
                .unwrap_or_default();

            // Guard passes iff all constraint predicates produce zero violations
            let guard: Predicate = Arc::new(move |ctx: &EvalContext| {
                guard_preds.iter()
                    .flat_map(|p| p(ctx))
                    .collect()
            });

            CompiledTransition {
                from: t.from.clone(),
                to: t.to.clone(),
                event: t.event.clone(),
                guard,
            }
        })
        .collect();

    let initial = def.statuses.first().cloned().unwrap_or_default();

    // Transition function: find first matching (from, event) where guard passes
    let transition_fn: Arc<dyn Fn(&str, &str, &EvalContext) -> Option<String> + Send + Sync> =
        Arc::new(move |state: &str, event: &str, ctx: &EvalContext| {
            transitions.iter()
                .find(|t| t.from == state && t.event == event && (t.guard)(ctx).is_empty())
                .map(|t| t.to.clone())
        });

    CompiledStateMachine {
        noun_name: def.noun_name.clone(),
        initial,
        transition: transition_fn,
    }
}
