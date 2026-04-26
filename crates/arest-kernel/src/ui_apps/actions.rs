// crates/arest-kernel/src/ui_apps/actions.rs
//
// SYSTEM calls as actions on the current screen (#513, EPIC #496).
//
// VVVV (#511) landed cell-as-screen rendering and ZZZZ (#512) landed
// navigation actions as cells. The third leg of the cell-graph
// HATEOAS triangle, per #496:
//
//   "It provides SYSTEM calls, but it should be easy to navigate the
//    system using its own principles"
//
// Navigation answers "where can I go from here?". Actions answer
// "what can I do here?" — and every "do" is a SYSTEM verb (apply,
// transition, fetch, FetchOrPhi, Store, Def, Platform, Native — the
// Func primitive set in `arest::ast::Func`). Clicking an action row
// dispatches the verb against the current cell context, with default
// arguments pre-bound from the cell's data (the property-binding
// principle PPPP introduced in #491: cell value ↔ widget input).
//
// # Conceptual model — every action IS a cell
//
// Each `SystemAction` produced here corresponds to a *fact* of the
// implicit fact type:
//
//     <CurrentCellRef> has system_action <Verb, Args>
//
// Same shape ZZZZ used for navigation: the action catalogue is
// computed from the cell graph rather than hand-listed per screen,
// and the kind tag preserves provenance (Apply / Transition /
// Fetch / Store / Def / Platform). The free-text REPL still works
// (typed verb invocations); the action panel is the *canonical*
// surface — each action row IS a SYSTEM call grounded in cell
// context, ready to dispatch.
//
// # Per-cell-type action mapping
//
// Mirrors the matrix in #513's task scope:
//
//   * Root            — `apply create <Noun>` per known noun;
//                       `Def` to introspect a noun's FactType graph.
//   * Noun            — `apply create <Noun>` (open the form);
//                       `apply destroy <Noun>::<id>` per instance;
//                       `Fetch <Noun>_has_…` to inspect any FT cell.
//   * Instance        — `apply update <Noun>::<id>` (edit form);
//                       `apply destroy <Noun>::<id>`;
//                       `transition <SM>::<id> <next-state>` per SM
//                       fact whose `forResource` matches the instance.
//   * FactCell        — `apply remove fact <fact-id>` per fact in cell;
//                       `Fetch <cell_name>` to inspect raw contents.
//   * ComponentInstance — `apply update Component_property <name>`
//                       per declared property of the Component.
//
// Each `SystemAction` carries `default_args` — pre-bound parameters
// from the current cell context. Free-text REPL input is still valid
// (the user can type the same verb manually), but the canonical
// surface is one click per row with the args already filled in.
//
// # Why a separate module
//
// The derivation logic mirrors `navigation.rs` in shape: pure
// function over `&Object` returning owned data, sorted/deduped for
// stable click-by-index dispatch. Extending `cell_renderer.rs` would
// crowd out the rendering rule. The two modules read the same
// `Object` shape but have orthogonal concerns: `navigation` answers
// "what cells can I reach?"; `actions` answers "what SYSTEM calls
// can I make?". Both feed into `RenderedScreen` so the typed-surface
// area picks them up in one redraw.

#![allow(dead_code)]

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use arest::ast::{self, Func, Object};

use crate::ui_apps::cell_renderer::CurrentCell;

// ── SystemAction ───────────────────────────────────────────────────

/// Whether an action is currently fireable, and (if not) why. #514
/// extends the action surface for state-machine instances: each legal
/// outgoing transition becomes its own button, and any guard-violating
/// transition surfaces here as `BlockedByGuard(violation_text)` so the
/// renderer can grey out the row and surface the explanation on hover.
///
/// The same pattern generalises to any action whose preconditions can
/// be checked statically before dispatch — guards on transitions today,
/// alethic / deontic constraints on Apply* verbs in a future commit
/// (#288 wires the same constraint pass through this enum).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GuardStatus {
    /// The action's preconditions are satisfied — clicking the row
    /// dispatches the verb. Default for every action without a
    /// declared guard.
    Enabled,
    /// One or more guards block the action. The carried text is the
    /// human-readable explanation surfaced as a tooltip on the disabled
    /// row. Multiple violations are joined with `"; "` so the tooltip
    /// shows everything at once.
    BlockedByGuard(String),
}

impl GuardStatus {
    /// Returns true when the action is currently fireable. Used by the
    /// dispatch path to short-circuit clicks against disabled rows and
    /// by the Slint side to set the `enabled` property on the row.
    pub fn is_enabled(&self) -> bool {
        matches!(self, GuardStatus::Enabled)
    }

    /// Returns the violation text for a blocked action, or the empty
    /// string for an enabled one. The Slint surface pushes this string
    /// into the tooltip property regardless of state — Slint will not
    /// render the tooltip when the row is enabled.
    pub fn tooltip(&self) -> &str {
        match self {
            GuardStatus::Enabled => "",
            GuardStatus::BlockedByGuard(text) => text.as_str(),
        }
    }
}

/// One legal "SYSTEM call from this screen" affordance derived from
/// the cell graph + the current cell context. Each action IS a cell
/// of the implicit fact type
/// `<current_cell> has system_action <Verb, Args>`. The renderer
/// surfaces these as clickable rows; the click handler dispatches the
/// SYSTEM verb with `default_args` pre-bound from cell context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemAction {
    /// The SYSTEM verb to dispatch when the affordance fires.
    pub verb: SystemVerb,
    /// Default arguments pre-bound from the cell context. The
    /// dispatcher splices these into the verb invocation; the user
    /// can override any of them via the (future) parameter editor in
    /// the action panel. Format: ordered list of (param_name, value)
    /// pairs so the dispatcher and the editor agree on positional
    /// semantics.
    pub default_args: Vec<(String, String)>,
    /// Human-readable label for the affordance row. Composed by the
    /// constructor from verb + first-arg hint so the Slint side
    /// doesn't need to do any string assembly. Kept stable across
    /// redraws (sorted by `compute_actions`) so click-by-index works.
    pub label: String,
    /// Whether the action is currently fireable. Default `Enabled`.
    /// State-machine transitions (#514) flip this to `BlockedByGuard`
    /// when a `Guard_prevents_Transition` fact references the
    /// transition AND the guard's predicate evaluates positively in
    /// current state — the renderer greys the row out and surfaces the
    /// guard's explanation as a tooltip.
    pub guard_status: GuardStatus,
}

impl SystemAction {
    /// Construct an action with auto-derived label. The label format
    /// is `[<verb-prefix>] <verb-text>` where `verb-text` is the
    /// canonical free-text REPL form a power user would type
    /// (e.g. `apply create File`, `transition Order::o1 submit`),
    /// so the action panel doubles as documentation for the REPL.
    /// Returned action is `Enabled` by default; use
    /// `with_guard_status` to surface a blocked transition.
    pub fn new(verb: SystemVerb, default_args: Vec<(String, String)>) -> Self {
        let prefix = verb.label_prefix();
        let text = verb.canonical_text(&default_args);
        let label = format!("[{prefix}] {text}");
        Self { verb, default_args, label, guard_status: GuardStatus::Enabled }
    }

    /// Construct an action with an explicit `guard_status` and an
    /// explicit override label. Used by the SM-action enumerator
    /// (#514) so the per-transition button shows the event name as
    /// the principal text rather than the canonical REPL form.
    pub fn with_label_and_guard(
        verb: SystemVerb,
        default_args: Vec<(String, String)>,
        label: String,
        guard_status: GuardStatus,
    ) -> Self {
        Self { verb, default_args, label, guard_status }
    }
}

/// SYSTEM verb space (#161). Mirrors the kernel-accessible
/// `arest::ast::Func` primitives PLUS the high-level "apply" verb
/// (the host crate's `command::Command`). The host crate's full
/// `Command` enum is `cfg(not(feature = "no_std"))`-gated (see
/// VVVV's #511 finding); on the kernel side we represent the same
/// surface as a flat enum and dispatch through `system_impl` (the
/// in-kernel verb dispatcher used by the legacy REPL).
///
/// Provenance / per-screen filter:
///   * Apply is the highest-level verb — create / update / destroy /
///     remove / transition. The dispatcher unpacks the kind from the
///     first arg.
///   * Transition is sugar for `Apply { kind: Transition, … }` so
///     the action panel can surface it distinctly per SM fact.
///   * Fetch / FetchOrPhi / Store / Def / Platform / Native map
///     1-1 onto `arest::ast::Func` variants of the same name —
///     they're the cell-level read/write primitives.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SystemVerb {
    /// `apply create <Noun> { … }` — instantiate a new Noun.
    ApplyCreate,
    /// `apply update <Noun>::<id>` — edit an existing instance.
    ApplyUpdate,
    /// `apply destroy <Noun>::<id>` — delete an instance.
    ApplyDestroy,
    /// `apply remove fact <fact-id>` — drop one fact from a cell.
    ApplyRemoveFact,
    /// `transition <SM>::<id> <next-state>` — fire a SM transition.
    Transition,
    /// `fetch <cell_name>` — read a cell, ⊥ if absent.
    Fetch,
    /// `fetch_or_phi <cell_name>` — read a cell, φ if absent.
    FetchOrPhi,
    /// `store <cell_name> <contents>` — write a cell.
    Store,
    /// `def <name>` — introspect a definition.
    Def,
    /// `platform <name>` — invoke a Platform primitive by name.
    Platform,
    /// `native <name>` — invoke a Native escape-hatch closure.
    Native,
}

impl SystemVerb {
    /// Short tag shown in the action label.
    pub fn label_prefix(&self) -> &'static str {
        match self {
            SystemVerb::ApplyCreate => "create",
            SystemVerb::ApplyUpdate => "update",
            SystemVerb::ApplyDestroy => "destroy",
            SystemVerb::ApplyRemoveFact => "remove",
            SystemVerb::Transition => "transition",
            SystemVerb::Fetch => "fetch",
            SystemVerb::FetchOrPhi => "fetch?",
            SystemVerb::Store => "store",
            SystemVerb::Def => "def",
            SystemVerb::Platform => "platform",
            SystemVerb::Native => "native",
        }
    }

    /// Canonical REPL text for this verb + args, suitable for the
    /// action label and (later) for the parameter editor's preview
    /// strip. Mirrors what a power user would type in the free-text
    /// REPL to get the same effect.
    pub fn canonical_text(&self, args: &[(String, String)]) -> String {
        let arg_value = |key: &str| -> String {
            args.iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        match self {
            SystemVerb::ApplyCreate => {
                let noun = arg_value("noun");
                format!("apply create {noun}")
            }
            SystemVerb::ApplyUpdate => {
                let noun = arg_value("noun");
                let id = arg_value("id");
                format!("apply update {noun}::{id}")
            }
            SystemVerb::ApplyDestroy => {
                let noun = arg_value("noun");
                let id = arg_value("id");
                format!("apply destroy {noun}::{id}")
            }
            SystemVerb::ApplyRemoveFact => {
                let cell = arg_value("cell");
                let fact = arg_value("fact");
                format!("apply remove fact {cell}#{fact}")
            }
            SystemVerb::Transition => {
                let sm = arg_value("sm");
                let id = arg_value("id");
                let next = arg_value("next");
                let event = arg_value("event");
                if event.is_empty() {
                    format!("transition {sm}::{id} {next}")
                } else {
                    // #514: when the event name is bound (the rich SM
                    // shape with `Transition_is_triggered_by_Event_Type`),
                    // include it in the canonical REPL text so a power
                    // user typing the verb manually can target the same
                    // transition by its event-name handle.
                    format!("transition {sm}::{id} {event} (\u{2192} {next})")
                }
            }
            SystemVerb::Fetch => {
                let name = arg_value("name");
                format!("fetch {name}")
            }
            SystemVerb::FetchOrPhi => {
                let name = arg_value("name");
                format!("fetch_or_phi {name}")
            }
            SystemVerb::Store => {
                let name = arg_value("name");
                format!("store {name}")
            }
            SystemVerb::Def => {
                let name = arg_value("name");
                format!("def {name}")
            }
            SystemVerb::Platform => {
                let name = arg_value("name");
                format!("platform {name}")
            }
            SystemVerb::Native => {
                let name = arg_value("name");
                format!("native {name}")
            }
        }
    }
}

// ── Public entry ───────────────────────────────────────────────────

/// Walk the cell graph and return every SYSTEM call surfaced as an
/// action on `current_cell`. Pure function — `with_state` callers
/// can drop the read lock the moment this returns.
///
/// Result is sorted (verb then label) so successive redraws emit
/// identical orderings; the Slint side relies on the index into this
/// vector to identify which action the user clicked.
pub fn compute_actions(current_cell: &CurrentCell, state: &Object) -> Vec<SystemAction> {
    let mut out = match current_cell {
        CurrentCell::Root => actions_for_root(state),
        CurrentCell::Noun { noun } => actions_for_noun(noun, state),
        CurrentCell::Instance { noun, instance } => {
            actions_for_instance(noun, instance, state)
        }
        CurrentCell::FactCell { cell_name } => actions_for_fact_cell(cell_name, state),
        CurrentCell::ComponentInstance { component_id } => {
            actions_for_component_instance(component_id, state)
        }
    };

    // Stable, deterministic ordering. The Slint side keys clicks by
    // index; the kernel must emit the same order on every redraw.
    //
    // Sort key (#514): primary is the SystemVerb ordinal (existing).
    // Secondary, *within* the Transition verb group, is enabled-first
    // (so the user's natural next moves come before disabled ones)
    // then alphabetical by event name. For non-Transition verbs the
    // secondary key is the existing alphabetical-by-label tiebreaker.
    out.sort_by(|a, b| {
        a.verb
            .cmp(&b.verb)
            .then_with(|| {
                if matches!(a.verb, SystemVerb::Transition)
                    && matches!(b.verb, SystemVerb::Transition)
                {
                    let a_enabled = a.guard_status.is_enabled();
                    let b_enabled = b.guard_status.is_enabled();
                    // `true` sorts after `false` in default Ord, so
                    // we negate to put enabled first.
                    b_enabled.cmp(&a_enabled)
                } else {
                    core::cmp::Ordering::Equal
                }
            })
            .then_with(|| a.label.cmp(&b.label))
    });
    // Dedupe identical (verb, label) pairs that two different walks
    // produce.
    out.dedup_by(|a, b| a.verb == b.verb && a.label == b.label);
    out
}

// ── Per-cell-type action derivation ────────────────────────────────

/// Root actions: `apply create <Noun>` per known noun + a `Def`
/// introspection action per noun. Mirrors `actions_for_root` in
/// shape to `targets_for_root`: one row per noun, derived from
/// `discover_nouns`.
fn actions_for_root(state: &Object) -> Vec<SystemAction> {
    let mut out: Vec<SystemAction> = Vec::new();
    for noun in discover_nouns(state) {
        out.push(SystemAction::new(
            SystemVerb::ApplyCreate,
            vec![("noun".to_string(), noun.clone())],
        ));
        out.push(SystemAction::new(
            SystemVerb::Def,
            vec![("name".to_string(), format!("resolve:{noun}"))],
        ));
    }
    out
}

/// Noun actions: a `create` form-opener for the noun + a per-instance
/// `destroy` action + a `fetch?` row per FT cell the noun owns.
fn actions_for_noun(noun: &str, state: &Object) -> Vec<SystemAction> {
    let mut out: Vec<SystemAction> = Vec::new();

    // Open the create form for this noun.
    out.push(SystemAction::new(
        SystemVerb::ApplyCreate,
        vec![("noun".to_string(), noun.to_string())],
    ));

    // Destroy each instance.
    for instance in instances_of(noun, state) {
        out.push(SystemAction::new(
            SystemVerb::ApplyDestroy,
            vec![
                ("noun".to_string(), noun.to_string()),
                ("id".to_string(), instance),
            ],
        ));
    }

    // Inspect each owned FT cell via FetchOrPhi.
    let prefix = format!("{noun}_has_");
    let cells: BTreeSet<String> = ast::cells_iter(state)
        .into_iter()
        .filter_map(|(cn, _)| {
            if cn.starts_with(&prefix[..]) && !cn.contains(':') {
                Some(cn.to_string())
            } else {
                None
            }
        })
        .collect();
    for cell_name in cells {
        out.push(SystemAction::new(
            SystemVerb::FetchOrPhi,
            vec![("name".to_string(), cell_name)],
        ));
    }

    out
}

/// Instance actions: `apply update <Noun>::<id>`, `apply destroy
/// <Noun>::<id>`, plus a `transition` row per next-state reachable
/// from the SM fact (when one exists for this instance).
fn actions_for_instance(
    noun: &str,
    instance: &str,
    state: &Object,
) -> Vec<SystemAction> {
    let mut out: Vec<SystemAction> = Vec::new();

    // Update + Destroy — the canonical Instance verbs.
    out.push(SystemAction::new(
        SystemVerb::ApplyUpdate,
        vec![
            ("noun".to_string(), noun.to_string()),
            ("id".to_string(), instance.to_string()),
        ],
    ));
    out.push(SystemAction::new(
        SystemVerb::ApplyDestroy,
        vec![
            ("noun".to_string(), noun.to_string()),
            ("id".to_string(), instance.to_string()),
        ],
    ));

    // State machine: per legal outgoing transition (#514, EPIC #496).
    // For any Instance with an SM (detected via
    // `StateMachine_has_currentlyInStatus`), surface the *specific*
    // legal next transitions as one-click actions, with their event
    // names as labels. Each outgoing Transition becomes its own
    // button; disabled ones (guard violations) carry the violation
    // text on their `guard_status` so the renderer can grey them out
    // and surface the explanation on hover.
    //
    // Backwards compatibility: when the richer Transition cells
    // (`Transition_is_defined_in_State_Machine_Definition`,
    // `Transition_is_from_Status`, `Transition_is_triggered_by_Event_Type`)
    // are absent ENTIRELY for this SM, fall back to BBBBB's #513 shape
    // — one generic Transition action per next-state value reachable
    // from `Transition_is_to_Status`. The two distinct empty cases
    // matter: (a) the rich shape is bound but the current Status is
    // terminal (no outgoing) → emit *nothing* (correct: nowhere to
    // go); (b) the rich shape isn't bound at all → fall back to the
    // legacy enumeration so the Action Card still surfaces something.
    if let Some(sm_id) = state_machine_for(noun, instance, state) {
        if rich_sm_shape_present(&sm_id, state) {
            // Rich-shape path: one action per legal outgoing
            // transition. The label is the event name — that's the
            // user-facing verb the action panel surfaces. When the
            // current Status is terminal, this path emits nothing
            // (correct: no buttons because there's nowhere to go).
            for t in transitions_for_sm(&sm_id, state) {
                let label = format_transition_label(&t);
                out.push(SystemAction::with_label_and_guard(
                    SystemVerb::Transition,
                    vec![
                        ("sm".to_string(), sm_id.clone()),
                        ("id".to_string(), instance.to_string()),
                        ("next".to_string(), t.target_status.clone()),
                        ("event".to_string(), t.event_name.clone()),
                        ("transition".to_string(), t.transition_id.clone()),
                    ],
                    label,
                    t.guard_status,
                ));
            }
        } else {
            // Reduced-shape fallback: walk the legacy
            // `Transition_is_to_Status` cell. Mirrors BBBBB's #513.
            let next_states = next_states_for(&sm_id, state);
            if next_states.is_empty() {
                out.push(SystemAction::new(
                    SystemVerb::Transition,
                    vec![
                        ("sm".to_string(), sm_id.clone()),
                        ("id".to_string(), instance.to_string()),
                        ("next".to_string(), String::new()),
                    ],
                ));
            } else {
                for next in next_states {
                    out.push(SystemAction::new(
                        SystemVerb::Transition,
                        vec![
                            ("sm".to_string(), sm_id.clone()),
                            ("id".to_string(), instance.to_string()),
                            ("next".to_string(), next),
                        ],
                    ));
                }
            }
        }
    }

    out
}

/// Returns true when the rich SM shape is populated for this SM
/// instance — at least one Transition cell binds to the SM's
/// Definition. Distinguishes the two empty cases for `transitions_for_sm`:
///   * Rich shape present + terminal Status → no actions (correct).
///   * Rich shape absent → fall back to legacy `next_states_for`.
fn rich_sm_shape_present(sm_id: &str, state: &Object) -> bool {
    let sm_def = state_machine_def_for(sm_id, state);
    let cell = ast::fetch_or_phi(
        "Transition_is_defined_in_State_Machine_Definition",
        state,
    );
    let Some(facts) = cell.as_seq() else {
        return false;
    };
    facts
        .iter()
        .any(|fact| ast::binding_matches(fact, "State Machine Definition", &sm_def))
}

/// One legal outgoing Transition for a SM instance. Carries the
/// labelling data (event name, target Status) plus the precomputed
/// `guard_status` so `actions_for_instance` can build the
/// corresponding `SystemAction` without re-walking the cell graph.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TransitionInfo {
    /// The Transition entity id (the principal key from the
    /// `Transition` entity type — e.g. `'submit'`, `'review'`).
    transition_id: String,
    /// The Event Type name that fires this transition. This is the
    /// principal user-facing label the Action Card surfaces (e.g.
    /// `"submit"`, `"approved"`). Defaults to the transition id when
    /// no `Transition_is_triggered_by_Event_Type` fact is bound.
    event_name: String,
    /// The Status the SM lands in after this transition fires
    /// (`Transition_is_to_Status`). Pre-filled into the dispatch
    /// args' `next` slot.
    target_status: String,
    /// Whether the transition is fireable today. `BlockedByGuard`
    /// when one or more guards prevent it AND those guards' predicates
    /// evaluate positively in current state.
    guard_status: GuardStatus,
}

/// Compose the human-readable button label for a transition action.
/// Format: `[transition] <event> (→ <target_status>)`. Disabled
/// transitions append `" — disabled"` so screen readers / log
/// scrapes can spot the state without consulting the tooltip arrow.
fn format_transition_label(t: &TransitionInfo) -> String {
    let base = format!(
        "[{}] {} (\u{2192} {})",
        SystemVerb::Transition.label_prefix(),
        t.event_name,
        t.target_status,
    );
    match &t.guard_status {
        GuardStatus::Enabled => base,
        GuardStatus::BlockedByGuard(_) => format!("{base} \u{2014} disabled"),
    }
}

/// Fact-cell actions: `apply remove fact <id>` per fact in the cell
/// + a single `fetch` action to inspect the raw contents. The
/// "remove fact" rows give the user a per-row delete affordance
/// without leaving the cell view.
fn actions_for_fact_cell(cell_name: &str, state: &Object) -> Vec<SystemAction> {
    let mut out: Vec<SystemAction> = Vec::new();

    // Always offer raw fetch as a baseline introspection action.
    out.push(SystemAction::new(
        SystemVerb::Fetch,
        vec![("name".to_string(), cell_name.to_string())],
    ));

    // Per-fact remove. We use the fact's index into the cell as the
    // synthetic id (the engine doesn't carry a stable per-fact id
    // today — #513 surfaces the affordance, #514+ may seed a
    // ProvenanceId if needed).
    let cell = ast::fetch_or_phi(cell_name, state);
    if let Some(facts) = cell.as_seq() {
        for (idx, _fact) in facts.iter().enumerate() {
            out.push(SystemAction::new(
                SystemVerb::ApplyRemoveFact,
                vec![
                    ("cell".to_string(), cell_name.to_string()),
                    ("fact".to_string(), idx.to_string()),
                ],
            ));
        }
    }

    out
}

/// Component-instance actions: `apply update Component_property
/// <name>` per declared property of the Component. Mirrors the
/// "edit each prop" affordance the prior `project_component_instance`
/// only listed read-only.
fn actions_for_component_instance(
    component_id: &str,
    state: &Object,
) -> Vec<SystemAction> {
    let mut out: Vec<SystemAction> = Vec::new();
    let comp_root = component_id.split('.').next().unwrap_or(component_id);

    let prop_cell = ast::fetch_or_phi(
        "Component_has_Property_of_PropertyType_with_PropertyDefault",
        state,
    );
    let Some(facts) = prop_cell.as_seq() else {
        return out;
    };
    for fact in facts {
        if !ast::binding_matches(fact, "Component", comp_root) {
            continue;
        }
        let Some(name) = ast::binding(fact, "PropertyName") else {
            continue;
        };
        out.push(SystemAction::new(
            SystemVerb::ApplyUpdate,
            vec![
                ("noun".to_string(), "Component_property".to_string()),
                ("id".to_string(), format!("{component_id}#{name}")),
            ],
        ));
    }
    out
}

// ── Cell-graph helpers (pure over &Object) ────────────────────────

/// Sorted, deduplicated set of Noun names — same shape as
/// `navigation::discover_nouns`. Filtered to skip `:`-shards and
/// the synthetic `D` cell.
fn discover_nouns(state: &Object) -> Vec<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    for (cell_name, _) in ast::cells_iter(state) {
        if cell_name.contains(':') {
            continue;
        }
        let Some((noun, _)) = cell_name.split_once("_has_") else {
            continue;
        };
        if noun == "D" {
            continue;
        }
        set.insert(noun.to_string());
    }
    set.into_iter().collect()
}

/// Distinct instance identifiers for `noun` — every value bound to
/// the noun's role across every cell of the form `<Noun>_has_…`.
/// Mirrors `navigation::instances_of`.
fn instances_of(noun: &str, state: &Object) -> Vec<String> {
    let prefix = format!("{noun}_has_");
    let mut set: BTreeSet<String> = BTreeSet::new();
    for (cell_name, contents) in ast::cells_iter(state) {
        if !cell_name.starts_with(&prefix[..]) {
            continue;
        }
        let Some(facts) = contents.as_seq() else {
            continue;
        };
        for fact in facts {
            if let Some(id) = ast::binding(fact, noun) {
                set.insert(id.to_string());
            }
        }
    }
    set.into_iter().collect()
}

/// Look up the SM identifier for a (noun, instance) pair if any
/// `StateMachine_has_currentlyInStatus` fact references the instance
/// (either as the SM itself or as the `forResource`). Returns the SM
/// identifier so the action panel can populate the `transition` verb's
/// `sm` arg.
fn state_machine_for(noun: &str, instance: &str, state: &Object) -> Option<String> {
    let cell = ast::fetch_or_phi("StateMachine_has_currentlyInStatus", state);
    let facts = cell.as_seq()?;
    facts.iter().find_map(|fact| {
        if ast::binding_matches(fact, "forResource", instance) {
            ast::binding(fact, "State Machine").map(|s| s.to_string())
        } else if ast::binding_matches(fact, "State Machine", instance) {
            Some(instance.to_string())
        } else if ast::binding_matches(fact, noun, instance) {
            ast::binding(fact, "State Machine").map(|s| s.to_string())
        } else {
            None
        }
    })
}

/// Look up the SM's currently-in Status. Reads
/// `StateMachine_has_currentlyInStatus` and returns the bound
/// `currentlyInStatus` for any fact that mentions `sm_or_resource_id`
/// in the SM, forResource, or instance role positions. Mirrors
/// `state_machine_for`'s tolerant matching shape.
fn current_status_for(sm_or_resource_id: &str, state: &Object) -> Option<String> {
    let cell = ast::fetch_or_phi("StateMachine_has_currentlyInStatus", state);
    let facts = cell.as_seq()?;
    facts.iter().find_map(|fact| {
        let mentions = ast::binding_matches(fact, "State Machine", sm_or_resource_id)
            || ast::binding_matches(fact, "forResource", sm_or_resource_id);
        if !mentions {
            return None;
        }
        ast::binding(fact, "currentlyInStatus").map(|s| s.to_string())
    })
}

/// Look up the SM Definition that this SM instance is an instance of.
/// Reads `StateMachine_is_instance_of_State_Machine_Definition` per
/// the canonical shape in `readings/core/instances.md`. Falls back to
/// using the SM id itself as the definition when the instance fact
/// isn't bound (the simplified test fixtures + the legacy "SM and
/// SMDef share id" convention used by some compile paths).
fn state_machine_def_for(sm_id: &str, state: &Object) -> String {
    let cell = ast::fetch_or_phi(
        "StateMachine_is_instance_of_State_Machine_Definition",
        state,
    );
    if let Some(facts) = cell.as_seq() {
        for fact in facts {
            if ast::binding_matches(fact, "State Machine", sm_id) {
                if let Some(def) = ast::binding(fact, "State Machine Definition")
                {
                    return def.to_string();
                }
            }
        }
    }
    sm_id.to_string()
}

/// Enumerate every legal outgoing Transition for a SM instance —
/// one entry per Transition cell whose `from Status` matches the SM's
/// current Status AND whose `defined in State Machine Definition`
/// matches the SM's def. Each entry carries the event name (label),
/// target Status (dispatch arg), and `guard_status` (Enabled or
/// BlockedByGuard with violation text).
///
/// Returns an empty Vec when no rich Transition cells are populated —
/// the caller falls back to BBBBB's #513 reduced shape.
fn transitions_for_sm(sm_id: &str, state: &Object) -> Vec<TransitionInfo> {
    let Some(current_status) = current_status_for(sm_id, state) else {
        return Vec::new();
    };
    let sm_def = state_machine_def_for(sm_id, state);

    // Walk `Transition_is_defined_in_State_Machine_Definition` to
    // find every Transition that belongs to this SM's def. This is
    // the rich-shape entry point; if the cell is missing we return
    // empty (caller falls back).
    let defined_cell = ast::fetch_or_phi(
        "Transition_is_defined_in_State_Machine_Definition",
        state,
    );
    let Some(defined_facts) = defined_cell.as_seq() else {
        return Vec::new();
    };
    let mut transition_ids: BTreeSet<String> = BTreeSet::new();
    for fact in defined_facts {
        if ast::binding_matches(fact, "State Machine Definition", &sm_def) {
            if let Some(tid) = ast::binding(fact, "Transition") {
                transition_ids.insert(tid.to_string());
            }
        }
    }
    if transition_ids.is_empty() {
        return Vec::new();
    }

    // Filter to outgoing transitions: those whose `is from Status`
    // matches the SM's current status.
    let from_cell = ast::fetch_or_phi("Transition_is_from_Status", state);
    let from_facts = from_cell.as_seq();
    let outgoing: BTreeSet<String> = transition_ids
        .iter()
        .filter(|tid| {
            let Some(facts) = from_facts else {
                // No `is_from` cell — conservatively accept every
                // defined transition (some fixtures omit `from`
                // entirely; the user can still see "what verbs exist").
                return true;
            };
            facts.iter().any(|fact| {
                ast::binding_matches(fact, "Transition", tid)
                    && ast::binding_matches(fact, "Status", &current_status)
            })
        })
        .cloned()
        .collect();
    if outgoing.is_empty() {
        return Vec::new();
    }

    // Build the (target Status, event name) lookups in one pass each.
    let to_cell = ast::fetch_or_phi("Transition_is_to_Status", state);
    let to_facts = to_cell.as_seq();
    let trigger_cell =
        ast::fetch_or_phi("Transition_is_triggered_by_Event_Type", state);
    let trigger_facts = trigger_cell.as_seq();

    let mut out: Vec<TransitionInfo> = Vec::new();
    for tid in &outgoing {
        let target_status = to_facts
            .and_then(|facts| {
                facts.iter().find(|f| ast::binding_matches(f, "Transition", tid))
            })
            .and_then(|f| ast::binding(f, "Status"))
            .map(|s| s.to_string())
            .unwrap_or_default();
        let event_name = trigger_facts
            .and_then(|facts| {
                facts.iter().find(|f| ast::binding_matches(f, "Transition", tid))
            })
            .and_then(|f| ast::binding(f, "Event Type"))
            .map(|s| s.to_string())
            .unwrap_or_else(|| tid.clone());
        let guard_status = guard_status_for_transition(tid, state);
        out.push(TransitionInfo {
            transition_id: tid.clone(),
            event_name,
            target_status,
            guard_status,
        });
    }
    out
}

/// Check whether any guard prevents the given Transition AND that
/// guard's predicate evaluates positively in current state. Returns
/// `Enabled` when no blocking guard applies, or `BlockedByGuard`
/// with the joined violation text(s) when one or more guards block.
///
/// Cell shape (per `readings/core/state.md`):
///   * `Guard_prevents_Transition { Guard, Transition }`
///   * `Guard_has_violation_text { Guard, violation_text }` (optional;
///      when absent the guard's id is used as the explanation)
///   * `Guard_is_active { Guard }` (optional gate; when present, only
///      guards listed here block — lets a future deontic-constraint
///      pass mark which guards have evaluated positively without
///      forcing this enumerator to re-run the predicate engine)
///
/// When `Guard_is_active` is absent, every guard that points at the
/// transition is treated as active — conservative default that errs
/// on the side of marking transitions disabled rather than letting
/// the user click into a guaranteed runtime failure.
fn guard_status_for_transition(transition_id: &str, state: &Object) -> GuardStatus {
    let prevents_cell = ast::fetch_or_phi("Guard_prevents_Transition", state);
    let Some(prevents_facts) = prevents_cell.as_seq() else {
        return GuardStatus::Enabled;
    };
    let blocking_guards: BTreeSet<String> = prevents_facts
        .iter()
        .filter_map(|fact| {
            if ast::binding_matches(fact, "Transition", transition_id) {
                ast::binding(fact, "Guard").map(|s| s.to_string())
            } else {
                None
            }
        })
        .collect();
    if blocking_guards.is_empty() {
        return GuardStatus::Enabled;
    }

    // Filter to currently-active guards (when the activity gate is
    // populated). Active gate semantics: `Guard_is_active { Guard }`
    // facts list every guard whose predicate currently evaluates
    // positively (i.e. the guard fires). When the cell is absent,
    // assume every guard is active (conservative).
    let active_cell = ast::fetch_or_phi("Guard_is_active", state);
    let active_set: Option<BTreeSet<String>> = active_cell.as_seq().map(|facts| {
        facts
            .iter()
            .filter_map(|f| ast::binding(f, "Guard").map(|s| s.to_string()))
            .collect()
    });
    let firing_guards: Vec<String> = blocking_guards
        .into_iter()
        .filter(|g| active_set.as_ref().map_or(true, |s| s.contains(g)))
        .collect();
    if firing_guards.is_empty() {
        return GuardStatus::Enabled;
    }

    // Resolve each firing guard to its violation text.
    let viol_cell = ast::fetch_or_phi("Guard_has_violation_text", state);
    let viol_facts = viol_cell.as_seq();
    let mut messages: Vec<String> = firing_guards
        .iter()
        .map(|guard| {
            viol_facts
                .and_then(|facts| {
                    facts.iter().find(|f| ast::binding_matches(f, "Guard", guard))
                })
                .and_then(|f| ast::binding(f, "violation_text"))
                .map(|s| s.to_string())
                .unwrap_or_else(|| guard.clone())
        })
        .collect();
    messages.sort();
    messages.dedup();
    GuardStatus::BlockedByGuard(messages.join("; "))
}

/// Enumerate next-state values reachable from the SM's transition
/// table. Reads `Transition_is_to_Status` (the canonical
/// transition-target cell shape) and returns every `Status` value.
/// Returns an empty Vec when no transition cell is present — the
/// caller emits a single Transition action with empty `next` so the
/// user can still type a next state manually.
fn next_states_for(sm_id: &str, state: &Object) -> Vec<String> {
    let cell = ast::fetch_or_phi("Transition_is_to_Status", state);
    let Some(facts) = cell.as_seq() else {
        return Vec::new();
    };
    let mut set: BTreeSet<String> = BTreeSet::new();
    for fact in facts {
        // The transition fact may also bind a SM Definition or a
        // Noun; we accept any fact that mentions the SM id in any
        // role position (the SM cell shape varies by version) AND
        // emit the Status. Conservative: when no SM filter matches,
        // drop the row (avoid spuriously listing every transition).
        let Some(pairs) = fact.as_seq() else { continue };
        let mentions_sm = pairs.iter().any(|p| {
            let Some(items) = p.as_seq() else { return false };
            items.len() == 2 && items[1].as_atom() == Some(sm_id)
        });
        if !mentions_sm {
            continue;
        }
        if let Some(status) = ast::binding(fact, "Status") {
            set.insert(status.to_string());
        }
    }
    set.into_iter().collect()
}

// ── Dispatch (action invocation → SYSTEM call) ────────────────────

/// Dispatch a SystemAction against the in-kernel SYSTEM and return
/// the result as a human-readable line for the REPL scrollback. This
/// is the kernel-side translation of the action panel click into the
/// existing `Func`-application path the legacy REPL uses.
///
/// The dispatcher is intentionally narrow on the foundation slice:
/// every verb resolves through `crate::system::with_state` + a
/// single `ast::apply` (the same shape `system::apply_named` /
/// `system::fetch_named` use). Verbs that mutate state (Apply*,
/// Transition, Store) are no-ops on the foundation slice — they
/// return a "would dispatch" annotation describing what the call
/// would have done, so the REPL scrollback shows the user the
/// effect without committing it. The full mutation path lands when
/// the host-side `command::apply_command` becomes reachable from
/// the kernel (#515+ wiring through Platform).
pub fn dispatch_action(action: &SystemAction) -> String {
    let summary = action.verb.canonical_text(&action.default_args);
    // #514: short-circuit blocked actions. The Slint side already
    // greys the row out + suppresses click delivery, but defensive
    // depth here guards against stale clicks (Slint event delivered
    // before the property bag refresh swaps the disabled state in)
    // and against direct callers (REPL command, future automation).
    if let GuardStatus::BlockedByGuard(reason) = &action.guard_status {
        return format!("[action] {summary} \u{2192} (blocked: {reason})");
    }
    match action.verb {
        // Read-only verbs: actually dispatch through the kernel's
        // existing fetch path so the user sees real cell contents.
        SystemVerb::Fetch | SystemVerb::FetchOrPhi => {
            let name = action
                .default_args
                .iter()
                .find(|(k, _)| k == "name")
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            if name.is_empty() {
                return format!("[action] {summary} → (missing name)");
            }
            let bytes = crate::system::fetch_named(&name);
            let body = core::str::from_utf8(&bytes)
                .map(|s| s.to_string())
                .unwrap_or_else(|_| format!("({} bytes)", bytes.len()));
            format!("[action] {summary} → {body}")
        }
        // Def is a fetch into the def cell; same shape as Fetch but
        // explicit about the verb in the result line.
        SystemVerb::Def => {
            let name = action
                .default_args
                .iter()
                .find(|(k, _)| k == "name")
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            if name.is_empty() {
                return format!("[action] {summary} → (missing name)");
            }
            // Defs live in the D cell as `D_has_<name>` shards; the
            // ρ-dispatch path uses FetchOrPhi(<name, D>) to look one
            // up. Mirror that.
            let bytes = crate::system::fetch_named(&name);
            let body = core::str::from_utf8(&bytes)
                .map(|s| s.to_string())
                .unwrap_or_else(|_| format!("({} bytes)", bytes.len()));
            format!("[action] {summary} → def: {body}")
        }
        // Mutating verbs: would-dispatch on the foundation slice.
        // The verb survives the round-trip through Func::* primitives
        // so a future commit can plumb `system::apply` here without
        // changing the action enumeration shape.
        SystemVerb::ApplyCreate
        | SystemVerb::ApplyUpdate
        | SystemVerb::ApplyDestroy
        | SystemVerb::ApplyRemoveFact
        | SystemVerb::Transition
        | SystemVerb::Store => {
            format!("[action] {summary} → (would dispatch — foundation slice is read-only)")
        }
        // Platform / Native: dispatch path resolves the named
        // primitive through `Func::Platform(name)` / the Fn1 closure
        // registered in the host. On the kernel-only build the
        // registries are empty by default, so this is structurally
        // identical to a fetch into a missing def cell.
        SystemVerb::Platform | SystemVerb::Native => {
            let name = action
                .default_args
                .iter()
                .find(|(k, _)| k == "name")
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            // Construct the Func and apply it to phi against current
            // state to exercise the same dispatch path the wire uses.
            let _func = match action.verb {
                SystemVerb::Platform => Func::Platform(name.clone()),
                SystemVerb::Native => Func::Id, // Native carries an Fn1; can't reconstruct from a name. Fall back to Id.
                _ => Func::Id,
            };
            let result = crate::system::with_state(|st| {
                ast::apply(&_func, &Object::phi(), st)
            });
            match result {
                Some(obj) => format!("[action] {summary} → {obj:?}"),
                None => format!("[action] {summary} → (system not initialised)"),
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use arest::ast::{cell_push, fact_from_pairs};

    /// Synthetic state mirroring `navigation::tests::synth_state` so
    /// the action expectations dovetail with the navigation tests.
    fn synth_state() -> Object {
        let s = Object::phi();
        let s = cell_push(
            "File_has_Name",
            fact_from_pairs(&[("File", "f1"), ("Name", "alpha.txt")]),
            &s,
        );
        let s = cell_push(
            "File_has_Name",
            fact_from_pairs(&[("File", "f2"), ("Name", "beta.txt")]),
            &s,
        );
        let s = cell_push(
            "File_has_MimeType",
            fact_from_pairs(&[("File", "f1"), ("MimeType", "text/plain")]),
            &s,
        );
        let s = cell_push(
            "Tag_has_Label",
            fact_from_pairs(&[("Tag", "t1"), ("Label", "important")]),
            &s,
        );
        let s = cell_push(
            "Tag_is_on_File",
            fact_from_pairs(&[("Tag", "t1"), ("File", "f1")]),
            &s,
        );
        let s = cell_push(
            "StateMachine_has_currentlyInStatus",
            fact_from_pairs(&[
                ("State Machine", "OrderSM"),
                ("currentlyInStatus", "draft"),
                ("forResource", "f1"),
            ]),
            &s,
        );
        cell_push(
            "Transition_is_to_Status",
            fact_from_pairs(&[
                ("State Machine", "OrderSM"),
                ("Status", "submitted"),
            ]),
            &s,
        )
    }

    // ── SystemVerb label prefixes ─────────────────────────────────

    #[test]
    fn system_verb_label_prefixes_distinct() {
        let verbs = [
            SystemVerb::ApplyCreate,
            SystemVerb::ApplyUpdate,
            SystemVerb::ApplyDestroy,
            SystemVerb::ApplyRemoveFact,
            SystemVerb::Transition,
            SystemVerb::Fetch,
            SystemVerb::FetchOrPhi,
            SystemVerb::Store,
            SystemVerb::Def,
            SystemVerb::Platform,
            SystemVerb::Native,
        ];
        let mut prefixes: Vec<&str> = verbs.iter().map(|v| v.label_prefix()).collect();
        prefixes.sort();
        prefixes.dedup();
        assert_eq!(prefixes.len(), verbs.len(), "prefixes must be distinct");
    }

    #[test]
    fn system_action_label_combines_prefix_and_text() {
        let action = SystemAction::new(
            SystemVerb::ApplyCreate,
            vec![("noun".to_string(), "File".to_string())],
        );
        assert_eq!(action.label, "[create] apply create File");
    }

    #[test]
    fn canonical_text_formats_each_verb() {
        let cases: &[(SystemVerb, Vec<(String, String)>, &str)] = &[
            (
                SystemVerb::ApplyCreate,
                vec![("noun".to_string(), "File".to_string())],
                "apply create File",
            ),
            (
                SystemVerb::ApplyUpdate,
                vec![
                    ("noun".to_string(), "File".to_string()),
                    ("id".to_string(), "f1".to_string()),
                ],
                "apply update File::f1",
            ),
            (
                SystemVerb::ApplyDestroy,
                vec![
                    ("noun".to_string(), "File".to_string()),
                    ("id".to_string(), "f1".to_string()),
                ],
                "apply destroy File::f1",
            ),
            (
                SystemVerb::Transition,
                vec![
                    ("sm".to_string(), "OrderSM".to_string()),
                    ("id".to_string(), "o1".to_string()),
                    ("next".to_string(), "submitted".to_string()),
                ],
                "transition OrderSM::o1 submitted",
            ),
            (
                SystemVerb::Fetch,
                vec![("name".to_string(), "File_has_Name".to_string())],
                "fetch File_has_Name",
            ),
        ];
        for (verb, args, want) in cases {
            let got = verb.canonical_text(args);
            assert_eq!(got, *want, "verb {verb:?} formatted as {got}, want {want}");
        }
    }

    // ── Root actions ──────────────────────────────────────────────

    #[test]
    fn root_actions_include_create_per_noun() {
        let state = synth_state();
        let actions = compute_actions(&CurrentCell::Root, &state);
        let creates: Vec<&str> = actions
            .iter()
            .filter(|a| a.verb == SystemVerb::ApplyCreate)
            .map(|a| a.label.as_str())
            .collect();
        assert!(
            creates.iter().any(|l| l.contains("apply create File")),
            "missing File: {creates:?}"
        );
        assert!(
            creates.iter().any(|l| l.contains("apply create Tag")),
            "missing Tag: {creates:?}"
        );
    }

    #[test]
    fn root_actions_include_def_per_noun() {
        let state = synth_state();
        let actions = compute_actions(&CurrentCell::Root, &state);
        let defs: Vec<&str> = actions
            .iter()
            .filter(|a| a.verb == SystemVerb::Def)
            .map(|a| a.label.as_str())
            .collect();
        assert!(
            defs.iter().any(|l| l.contains("resolve:File")),
            "missing resolve:File def: {defs:?}"
        );
    }

    // ── Noun actions ──────────────────────────────────────────────

    #[test]
    fn noun_actions_include_create_form_opener() {
        let state = synth_state();
        let actions = compute_actions(
            &CurrentCell::Noun { noun: "File".into() },
            &state,
        );
        let creates: Vec<&SystemAction> = actions
            .iter()
            .filter(|a| a.verb == SystemVerb::ApplyCreate)
            .collect();
        assert_eq!(creates.len(), 1, "one create form opener: {creates:?}");
        assert_eq!(creates[0].default_args[0].1, "File");
    }

    #[test]
    fn noun_actions_include_destroy_per_instance() {
        let state = synth_state();
        let actions = compute_actions(
            &CurrentCell::Noun { noun: "File".into() },
            &state,
        );
        let destroys: BTreeSet<String> = actions
            .iter()
            .filter(|a| a.verb == SystemVerb::ApplyDestroy)
            .filter_map(|a| {
                a.default_args
                    .iter()
                    .find(|(k, _)| k == "id")
                    .map(|(_, v)| v.clone())
            })
            .collect();
        assert!(destroys.contains("f1"), "missing f1: {destroys:?}");
        assert!(destroys.contains("f2"), "missing f2: {destroys:?}");
    }

    #[test]
    fn noun_actions_include_fetch_per_owned_ft_cell() {
        let state = synth_state();
        let actions = compute_actions(
            &CurrentCell::Noun { noun: "File".into() },
            &state,
        );
        let fetches: BTreeSet<String> = actions
            .iter()
            .filter(|a| a.verb == SystemVerb::FetchOrPhi)
            .filter_map(|a| {
                a.default_args
                    .iter()
                    .find(|(k, _)| k == "name")
                    .map(|(_, v)| v.clone())
            })
            .collect();
        assert!(
            fetches.contains("File_has_Name"),
            "missing File_has_Name: {fetches:?}"
        );
        assert!(
            fetches.contains("File_has_MimeType"),
            "missing File_has_MimeType: {fetches:?}"
        );
    }

    // ── Instance actions ──────────────────────────────────────────

    #[test]
    fn instance_actions_include_update_and_destroy() {
        let state = synth_state();
        let actions = compute_actions(
            &CurrentCell::Instance {
                noun: "File".into(),
                instance: "f1".into(),
            },
            &state,
        );
        assert!(
            actions.iter().any(|a| a.verb == SystemVerb::ApplyUpdate
                && a.default_args.iter().any(|(k, v)| k == "id" && v == "f1")),
            "missing update for f1: {actions:?}"
        );
        assert!(
            actions.iter().any(|a| a.verb == SystemVerb::ApplyDestroy
                && a.default_args.iter().any(|(k, v)| k == "id" && v == "f1")),
            "missing destroy for f1: {actions:?}"
        );
    }

    #[test]
    fn instance_actions_include_transition_per_next_state_when_sm_present() {
        let state = synth_state();
        let actions = compute_actions(
            &CurrentCell::Instance {
                noun: "File".into(),
                instance: "f1".into(),
            },
            &state,
        );
        let transitions: Vec<&SystemAction> = actions
            .iter()
            .filter(|a| a.verb == SystemVerb::Transition)
            .collect();
        assert!(
            !transitions.is_empty(),
            "f1 has SM, expected at least one transition: {actions:?}"
        );
        // Transition fact in synth has Status=submitted bound to OrderSM.
        assert!(
            transitions.iter().any(|a| a
                .default_args
                .iter()
                .any(|(k, v)| k == "next" && v == "submitted")),
            "missing submitted next-state: {transitions:?}"
        );
    }

    #[test]
    fn instance_actions_omit_transition_when_no_sm() {
        let state = synth_state();
        // f2 is a File but no StateMachine_has_currentlyInStatus fact
        // references it.
        let actions = compute_actions(
            &CurrentCell::Instance {
                noun: "File".into(),
                instance: "f2".into(),
            },
            &state,
        );
        assert!(
            !actions.iter().any(|a| a.verb == SystemVerb::Transition),
            "no transition expected for f2: {actions:?}"
        );
    }

    // ── FactCell actions ──────────────────────────────────────────

    #[test]
    fn fact_cell_actions_include_fetch_baseline() {
        let state = synth_state();
        let actions = compute_actions(
            &CurrentCell::FactCell {
                cell_name: "Tag_is_on_File".into(),
            },
            &state,
        );
        assert!(
            actions.iter().any(|a| a.verb == SystemVerb::Fetch),
            "missing baseline fetch: {actions:?}"
        );
    }

    #[test]
    fn fact_cell_actions_include_remove_per_fact() {
        let state = synth_state();
        let actions = compute_actions(
            &CurrentCell::FactCell {
                cell_name: "File_has_Name".into(),
            },
            &state,
        );
        let removes: Vec<&SystemAction> = actions
            .iter()
            .filter(|a| a.verb == SystemVerb::ApplyRemoveFact)
            .collect();
        // synth has two File_has_Name facts → two remove rows.
        assert_eq!(removes.len(), 2, "expected 2 remove rows: {removes:?}");
    }

    // ── ComponentInstance actions ─────────────────────────────────

    #[test]
    fn component_instance_actions_skip_when_no_props() {
        let state = synth_state();
        let actions = compute_actions(
            &CurrentCell::ComponentInstance {
                component_id: "list.slint".into(),
            },
            &state,
        );
        // No Component_has_Property_… facts in synth → empty.
        assert!(
            actions.is_empty(),
            "no props → no actions: {actions:?}"
        );
    }

    #[test]
    fn component_instance_actions_include_update_per_property() {
        let s = synth_state();
        let s = cell_push(
            "Component_has_Property_of_PropertyType_with_PropertyDefault",
            fact_from_pairs(&[
                ("Component", "list"),
                ("PropertyName", "items"),
                ("PropertyType", "list"),
                ("PropertyDefault", ""),
            ]),
            &s,
        );
        let actions = compute_actions(
            &CurrentCell::ComponentInstance {
                component_id: "list.slint".into(),
            },
            &s,
        );
        let updates: Vec<&SystemAction> = actions
            .iter()
            .filter(|a| a.verb == SystemVerb::ApplyUpdate)
            .collect();
        assert_eq!(updates.len(), 1);
        assert!(updates[0]
            .default_args
            .iter()
            .any(|(k, v)| k == "id" && v == "list.slint#items"));
    }

    // ── Determinism / dedup ───────────────────────────────────────

    #[test]
    fn actions_order_is_stable() {
        let state = synth_state();
        let a = compute_actions(&CurrentCell::Root, &state);
        let b = compute_actions(&CurrentCell::Root, &state);
        assert_eq!(a, b);
    }

    #[test]
    fn actions_dedupe_identical_verb_label_pairs() {
        let state = synth_state();
        let actions = compute_actions(&CurrentCell::Root, &state);
        let mut seen: BTreeSet<(SystemVerb, String)> = BTreeSet::new();
        for a in &actions {
            let key = (a.verb.clone(), a.label.clone());
            assert!(
                seen.insert(key),
                "duplicate (verb, label) pair: {} -> {:?}",
                a.label,
                a.verb
            );
        }
    }

    // ── Dispatch ──────────────────────────────────────────────────

    #[test]
    fn dispatch_action_produces_annotation_for_mutating_verbs() {
        let action = SystemAction::new(
            SystemVerb::ApplyCreate,
            vec![("noun".to_string(), "File".to_string())],
        );
        let line = dispatch_action(&action);
        assert!(line.contains("[action]"));
        assert!(line.contains("apply create File"));
        assert!(line.contains("foundation slice"));
    }

    #[test]
    fn dispatch_action_round_trips_fetch_via_system_impl() {
        // Round-trip: ensure the fetch path actually goes through
        // `crate::system::fetch_named` and produces a non-empty
        // result line. We can't assert the cell contents without
        // initialising SYSTEM (which requires the global one-shot
        // init the kernel boot path runs); the round-trip here
        // verifies the dispatch wiring is structurally sound — the
        // returned string contains the canonical annotation.
        let action = SystemAction::new(
            SystemVerb::Fetch,
            vec![("name".to_string(), "Probe_cell".to_string())],
        );
        let line = dispatch_action(&action);
        assert!(line.starts_with("[action]"));
        assert!(line.contains("fetch Probe_cell"));
    }

    #[test]
    fn dispatch_action_with_missing_required_arg_reports_gracefully() {
        let action = SystemAction {
            verb: SystemVerb::Fetch,
            default_args: Vec::new(),
            label: "[fetch] fetch ".to_string(),
            guard_status: GuardStatus::Enabled,
        };
        let line = dispatch_action(&action);
        assert!(line.contains("missing name"));
    }

    // ── State-machine action surface (#514) ────────────────────────
    //
    // Per #514 (#496e), Theorem 5: any entity with a state machine
    // surfaces its **specific** legal next transitions as one-click
    // actions, with their event names as labels. Disabled transitions
    // show their guard violations inline.

    /// Three-state SM fixture: `start → middle → end`. Each transition
    /// has an event-type label; `middle → end` carries a guard that
    /// the test toggles on / off via `Guard_is_active`.
    fn three_state_sm() -> Object {
        let s = Object::phi();
        // Resource the SM is for.
        let s = cell_push(
            "File_has_Name",
            fact_from_pairs(&[("File", "f1"), ("Name", "alpha.txt")]),
            &s,
        );
        // SM instance pointing at SM Definition.
        let s = cell_push(
            "StateMachine_is_instance_of_State_Machine_Definition",
            fact_from_pairs(&[
                ("State Machine", "sm1"),
                ("State Machine Definition", "FileLifecycle"),
            ]),
            &s,
        );
        let s = cell_push(
            "StateMachine_has_currentlyInStatus",
            fact_from_pairs(&[
                ("State Machine", "sm1"),
                ("currentlyInStatus", "start"),
                ("forResource", "f1"),
            ]),
            &s,
        );
        // Transitions defined in the SM Def.
        let s = cell_push(
            "Transition_is_defined_in_State_Machine_Definition",
            fact_from_pairs(&[
                ("Transition", "t_advance"),
                ("State Machine Definition", "FileLifecycle"),
            ]),
            &s,
        );
        let s = cell_push(
            "Transition_is_defined_in_State_Machine_Definition",
            fact_from_pairs(&[
                ("Transition", "t_finalise"),
                ("State Machine Definition", "FileLifecycle"),
            ]),
            &s,
        );
        // From / To wiring.
        let s = cell_push(
            "Transition_is_from_Status",
            fact_from_pairs(&[("Transition", "t_advance"), ("Status", "start")]),
            &s,
        );
        let s = cell_push(
            "Transition_is_to_Status",
            fact_from_pairs(&[("Transition", "t_advance"), ("Status", "middle")]),
            &s,
        );
        let s = cell_push(
            "Transition_is_from_Status",
            fact_from_pairs(&[("Transition", "t_finalise"), ("Status", "middle")]),
            &s,
        );
        let s = cell_push(
            "Transition_is_to_Status",
            fact_from_pairs(&[("Transition", "t_finalise"), ("Status", "end")]),
            &s,
        );
        // Event-type labels (the user-facing names).
        let s = cell_push(
            "Transition_is_triggered_by_Event_Type",
            fact_from_pairs(&[("Transition", "t_advance"), ("Event Type", "advance")]),
            &s,
        );
        cell_push(
            "Transition_is_triggered_by_Event_Type",
            fact_from_pairs(&[
                ("Transition", "t_finalise"),
                ("Event Type", "finalise"),
            ]),
            &s,
        )
    }

    /// Mutate `state` so that the SM instance `sm1` is currently in
    /// `status`. Used to walk the 3-state SM through start → middle →
    /// end and observe each step's action surface.
    fn set_current_status(state: &Object, status: &str) -> Object {
        // Drop facts in `StateMachine_has_currentlyInStatus` that
        // mention sm1, then push a fresh fact at the new status.
        let cleaned = ast::cell_filter(
            "StateMachine_has_currentlyInStatus",
            |f| !ast::binding_matches(f, "State Machine", "sm1"),
            state,
        );
        ast::cell_push(
            "StateMachine_has_currentlyInStatus",
            fact_from_pairs(&[
                ("State Machine", "sm1"),
                ("currentlyInStatus", status),
                ("forResource", "f1"),
            ]),
            &cleaned,
        )
    }

    #[test]
    fn three_state_sm_at_start_surfaces_advance_event() {
        let state = three_state_sm();
        let actions = compute_actions(
            &CurrentCell::Instance {
                noun: "File".into(),
                instance: "f1".into(),
            },
            &state,
        );
        let transitions: Vec<&SystemAction> = actions
            .iter()
            .filter(|a| a.verb == SystemVerb::Transition)
            .collect();
        assert_eq!(
            transitions.len(),
            1,
            "exactly one outgoing from start: {transitions:?}"
        );
        let advance = transitions[0];
        // Event name is the principal user-facing label.
        assert!(
            advance.label.contains("advance"),
            "label must include event name: {}",
            advance.label
        );
        // Target state surfaces in the label too.
        assert!(
            advance.label.contains("middle"),
            "label must include target status: {}",
            advance.label
        );
        // Default args carry both event + next for the dispatcher.
        let event_arg = advance
            .default_args
            .iter()
            .find(|(k, _)| k == "event")
            .map(|(_, v)| v.as_str());
        assert_eq!(event_arg, Some("advance"));
        let next_arg = advance
            .default_args
            .iter()
            .find(|(k, _)| k == "next")
            .map(|(_, v)| v.as_str());
        assert_eq!(next_arg, Some("middle"));
        // Enabled by default — no guard.
        assert_eq!(advance.guard_status, GuardStatus::Enabled);
    }

    #[test]
    fn three_state_sm_at_middle_surfaces_finalise_event() {
        let state = set_current_status(&three_state_sm(), "middle");
        let actions = compute_actions(
            &CurrentCell::Instance {
                noun: "File".into(),
                instance: "f1".into(),
            },
            &state,
        );
        let transitions: Vec<&SystemAction> = actions
            .iter()
            .filter(|a| a.verb == SystemVerb::Transition)
            .collect();
        assert_eq!(
            transitions.len(),
            1,
            "exactly one outgoing from middle: {transitions:?}"
        );
        assert!(
            transitions[0].label.contains("finalise"),
            "expected finalise label, got {}",
            transitions[0].label
        );
        assert!(transitions[0].label.contains("end"));
    }

    #[test]
    fn three_state_sm_at_end_surfaces_no_outgoing_transition() {
        let state = set_current_status(&three_state_sm(), "end");
        let actions = compute_actions(
            &CurrentCell::Instance {
                noun: "File".into(),
                instance: "f1".into(),
            },
            &state,
        );
        let transitions: Vec<&SystemAction> = actions
            .iter()
            .filter(|a| a.verb == SystemVerb::Transition)
            .collect();
        assert!(
            transitions.is_empty(),
            "terminal state has no outgoing transitions: {transitions:?}"
        );
    }

    #[test]
    fn guard_blocking_active_marks_transition_disabled() {
        let s = set_current_status(&three_state_sm(), "middle");
        // Add a guard preventing finalise — and mark it active.
        let s = cell_push(
            "Guard_prevents_Transition",
            fact_from_pairs(&[
                ("Guard", "needs-approval"),
                ("Transition", "t_finalise"),
            ]),
            &s,
        );
        let s = cell_push(
            "Guard_has_violation_text",
            fact_from_pairs(&[
                ("Guard", "needs-approval"),
                ("violation_text", "must be approved before finalising"),
            ]),
            &s,
        );
        let s = cell_push(
            "Guard_is_active",
            fact_from_pairs(&[("Guard", "needs-approval")]),
            &s,
        );

        let actions = compute_actions(
            &CurrentCell::Instance {
                noun: "File".into(),
                instance: "f1".into(),
            },
            &s,
        );
        let finalise = actions
            .iter()
            .find(|a| {
                a.verb == SystemVerb::Transition
                    && a.default_args.iter().any(|(k, v)| k == "event" && v == "finalise")
            })
            .expect("expected the finalise transition action");
        match &finalise.guard_status {
            GuardStatus::BlockedByGuard(reason) => {
                assert!(
                    reason.contains("must be approved"),
                    "expected violation text in tooltip, got {reason}"
                );
            }
            other => panic!("expected BlockedByGuard, got {other:?}"),
        }
        assert!(
            finalise.label.contains("disabled"),
            "label must indicate disabled state: {}",
            finalise.label
        );
    }

    #[test]
    fn guard_inactive_keeps_transition_enabled() {
        let s = set_current_status(&three_state_sm(), "middle");
        // Same guard wiring as the blocking-active test, but the
        // `Guard_is_active` cell is empty — the guard exists in the
        // schema but doesn't currently fire.
        let s = cell_push(
            "Guard_prevents_Transition",
            fact_from_pairs(&[
                ("Guard", "needs-approval"),
                ("Transition", "t_finalise"),
            ]),
            &s,
        );
        // Push an unrelated guard into the active cell so the cell
        // shape exists but doesn't list `needs-approval`.
        let s = cell_push(
            "Guard_is_active",
            fact_from_pairs(&[("Guard", "some-other-guard")]),
            &s,
        );

        let actions = compute_actions(
            &CurrentCell::Instance {
                noun: "File".into(),
                instance: "f1".into(),
            },
            &s,
        );
        let finalise = actions
            .iter()
            .find(|a| {
                a.verb == SystemVerb::Transition
                    && a.default_args.iter().any(|(k, v)| k == "event" && v == "finalise")
            })
            .expect("expected the finalise transition action");
        assert_eq!(
            finalise.guard_status,
            GuardStatus::Enabled,
            "guard not in Guard_is_active → action remains Enabled"
        );
    }

    #[test]
    fn transitions_sort_enabled_before_blocked_then_alphabetical() {
        // Build an SM with two outgoing transitions from `start`,
        // one of which is blocked by an active guard. Expectation:
        // the enabled one comes first regardless of alphabetical
        // order, then the blocked one.
        let s = three_state_sm();
        // Add a second outgoing from start: t_branch → branch.
        let s = cell_push(
            "Transition_is_defined_in_State_Machine_Definition",
            fact_from_pairs(&[
                ("Transition", "t_branch"),
                ("State Machine Definition", "FileLifecycle"),
            ]),
            &s,
        );
        let s = cell_push(
            "Transition_is_from_Status",
            fact_from_pairs(&[("Transition", "t_branch"), ("Status", "start")]),
            &s,
        );
        let s = cell_push(
            "Transition_is_to_Status",
            fact_from_pairs(&[("Transition", "t_branch"), ("Status", "branch")]),
            &s,
        );
        let s = cell_push(
            "Transition_is_triggered_by_Event_Type",
            fact_from_pairs(&[("Transition", "t_branch"), ("Event Type", "branch")]),
            &s,
        );
        // Block t_branch with an active guard.
        let s = cell_push(
            "Guard_prevents_Transition",
            fact_from_pairs(&[
                ("Guard", "branch-guard"),
                ("Transition", "t_branch"),
            ]),
            &s,
        );
        let s = cell_push(
            "Guard_is_active",
            fact_from_pairs(&[("Guard", "branch-guard")]),
            &s,
        );

        let actions = compute_actions(
            &CurrentCell::Instance {
                noun: "File".into(),
                instance: "f1".into(),
            },
            &s,
        );
        let transitions: Vec<&SystemAction> = actions
            .iter()
            .filter(|a| a.verb == SystemVerb::Transition)
            .collect();
        assert_eq!(transitions.len(), 2, "two outgoing: {transitions:?}");
        // First entry must be enabled (advance), second blocked
        // (branch) — even though `advance` > `branch` alphabetically.
        assert!(
            transitions[0].guard_status.is_enabled(),
            "first transition must be Enabled (advance), got {:?}",
            transitions[0]
        );
        assert!(
            transitions[0].label.contains("advance"),
            "first label must be advance, got {}",
            transitions[0].label
        );
        assert!(
            !transitions[1].guard_status.is_enabled(),
            "second transition must be blocked (branch), got {:?}",
            transitions[1]
        );
    }

    #[test]
    fn transitions_within_enabled_group_sort_alphabetically_by_event_name() {
        // Two enabled outgoing transitions must come out in
        // alphabetical order by event name.
        let s = Object::phi();
        let s = cell_push(
            "File_has_Name",
            fact_from_pairs(&[("File", "f1"), ("Name", "alpha.txt")]),
            &s,
        );
        let s = cell_push(
            "StateMachine_is_instance_of_State_Machine_Definition",
            fact_from_pairs(&[
                ("State Machine", "sm1"),
                ("State Machine Definition", "Demo"),
            ]),
            &s,
        );
        let s = cell_push(
            "StateMachine_has_currentlyInStatus",
            fact_from_pairs(&[
                ("State Machine", "sm1"),
                ("currentlyInStatus", "draft"),
                ("forResource", "f1"),
            ]),
            &s,
        );
        // Two outgoing transitions from draft, with intentionally
        // out-of-order alphabetisation: the transition ids start with
        // `t_zelda` / `t_alpha` so alphabetical-by-id would surface
        // `t_alpha` first; we expect alphabetical-by-event-name (the
        // user-facing label) to drive the sort instead.
        let s = [("t_zelda", "zelda", "z_state"), ("t_alpha", "alpha", "a_state")]
            .iter()
            .fold(s, |acc, (tid, event, target)| {
                let acc = cell_push(
                    "Transition_is_defined_in_State_Machine_Definition",
                    fact_from_pairs(&[
                        ("Transition", tid),
                        ("State Machine Definition", "Demo"),
                    ]),
                    &acc,
                );
                let acc = cell_push(
                    "Transition_is_from_Status",
                    fact_from_pairs(&[("Transition", tid), ("Status", "draft")]),
                    &acc,
                );
                let acc = cell_push(
                    "Transition_is_to_Status",
                    fact_from_pairs(&[("Transition", tid), ("Status", target)]),
                    &acc,
                );
                cell_push(
                    "Transition_is_triggered_by_Event_Type",
                    fact_from_pairs(&[("Transition", tid), ("Event Type", event)]),
                    &acc,
                )
            });

        let actions = compute_actions(
            &CurrentCell::Instance {
                noun: "File".into(),
                instance: "f1".into(),
            },
            &s,
        );
        let transitions: Vec<&SystemAction> = actions
            .iter()
            .filter(|a| a.verb == SystemVerb::Transition)
            .collect();
        assert_eq!(transitions.len(), 2);
        assert!(
            transitions[0].label.contains("alpha"),
            "expected alpha first, got {}",
            transitions[0].label
        );
        assert!(
            transitions[1].label.contains("zelda"),
            "expected zelda second, got {}",
            transitions[1].label
        );
    }

    #[test]
    fn dispatch_blocked_action_returns_blocked_annotation() {
        let action = SystemAction::with_label_and_guard(
            SystemVerb::Transition,
            vec![
                ("sm".to_string(), "sm1".to_string()),
                ("id".to_string(), "f1".to_string()),
                ("next".to_string(), "end".to_string()),
                ("event".to_string(), "finalise".to_string()),
            ],
            "[transition] finalise (\u{2192} end) \u{2014} disabled".to_string(),
            GuardStatus::BlockedByGuard("must be approved first".to_string()),
        );
        let line = dispatch_action(&action);
        assert!(line.contains("blocked"));
        assert!(line.contains("must be approved first"));
    }

    #[test]
    fn rich_sm_shape_overrides_legacy_next_states_path() {
        // When both the rich (`Transition_is_defined_in_*` etc.) and
        // the legacy (`Transition_is_to_Status` only) shapes are
        // present, the rich shape wins — we get one action per legal
        // outgoing transition (with event names), not the legacy
        // generic Transition action per next-state value.
        let s = three_state_sm();
        let actions = compute_actions(
            &CurrentCell::Instance {
                noun: "File".into(),
                instance: "f1".into(),
            },
            &s,
        );
        let transitions: Vec<&SystemAction> = actions
            .iter()
            .filter(|a| a.verb == SystemVerb::Transition)
            .collect();
        // Each rich-shape transition carries an `event` arg; the
        // legacy fallback would not.
        assert!(
            transitions.iter().all(|a| {
                a.default_args.iter().any(|(k, _)| k == "event")
            }),
            "rich shape: every action carries an event arg"
        );
    }

    #[test]
    fn legacy_sm_shape_still_works_when_rich_cells_absent() {
        // BBBBB's #513 fixture (synth_state in this test module) has
        // only the reduced `Transition_is_to_Status` shape. With no
        // rich Transition cells, the action surface falls back to the
        // legacy generic Transition action per next-state value.
        let state = synth_state();
        let actions = compute_actions(
            &CurrentCell::Instance {
                noun: "File".into(),
                instance: "f1".into(),
            },
            &state,
        );
        let transitions: Vec<&SystemAction> = actions
            .iter()
            .filter(|a| a.verb == SystemVerb::Transition)
            .collect();
        assert!(
            !transitions.is_empty(),
            "legacy shape: still surfaces a Transition action: {actions:?}"
        );
        // None of the legacy actions carry an `event` arg.
        assert!(
            transitions.iter().all(|a| {
                !a.default_args.iter().any(|(k, _)| k == "event")
            }),
            "legacy shape: no event arg"
        );
    }

    #[test]
    fn guard_status_helpers_round_trip() {
        let enabled = GuardStatus::Enabled;
        assert!(enabled.is_enabled());
        assert_eq!(enabled.tooltip(), "");
        let blocked = GuardStatus::BlockedByGuard("nope".to_string());
        assert!(!blocked.is_enabled());
        assert_eq!(blocked.tooltip(), "nope");
    }

    #[test]
    fn canonical_text_for_transition_with_event_includes_event() {
        let text = SystemVerb::Transition.canonical_text(&[
            ("sm".to_string(), "sm1".to_string()),
            ("id".to_string(), "f1".to_string()),
            ("next".to_string(), "middle".to_string()),
            ("event".to_string(), "advance".to_string()),
        ]);
        assert!(text.contains("advance"));
        assert!(text.contains("middle"));
    }
}
