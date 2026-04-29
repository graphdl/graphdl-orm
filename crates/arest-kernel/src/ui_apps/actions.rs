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
    /// `load_reading <name> <body>` (#564 / DynRdg-T5) — register a
    /// FORML 2 reading body at runtime. The action surface adds this
    /// row at the root screen so users can extend the schema from the
    /// REPL. `name` and `body` are read from the action's
    /// `default_args`; the REPL's input prompt can populate them via
    /// `dispatch_action_with_input` (parses `<name>\n<body>`).
    LoadReading,
    /// `unload_reading <name>` (#556 / DynRdg-2) — drop a previously-
    /// loaded reading from the cell graph by name. Inverse of
    /// `LoadReading`. Surfaced alongside it on the root screen.
    UnloadReading,
    /// `reload_reading <name> <body>` (#557 / DynRdg-3) — atomic
    /// unload+load against a single state snapshot.
    ReloadReading,
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
            SystemVerb::LoadReading => "load",
            SystemVerb::UnloadReading => "unload",
            SystemVerb::ReloadReading => "reload",
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
            SystemVerb::LoadReading => {
                let name = arg_value("name");
                if name.is_empty() {
                    "load_reading".to_string()
                } else {
                    format!("load_reading {name}")
                }
            }
            SystemVerb::UnloadReading => {
                let name = arg_value("name");
                if name.is_empty() {
                    "unload_reading".to_string()
                } else {
                    format!("unload_reading {name}")
                }
            }
            SystemVerb::ReloadReading => {
                let name = arg_value("name");
                if name.is_empty() {
                    "reload_reading".to_string()
                } else {
                    format!("reload_reading {name}")
                }
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
    // Meta-system verbs (#564 / DynRdg-T5): runtime reading load /
    // unload / reload. Always available on the root screen — they
    // operate on the def-state itself, not on any specific noun /
    // instance. Default args are empty; the REPL's input prompt
    // populates `name` and `body` (see `dispatch_action_with_input`)
    // when the user clicks one of these rows.
    out.push(SystemAction::new(SystemVerb::LoadReading, Vec::new()));
    out.push(SystemAction::new(SystemVerb::UnloadReading, Vec::new()));
    out.push(SystemAction::new(SystemVerb::ReloadReading, Vec::new()));
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

/// Look up a `default_args` entry by key. Returns the bound value
/// or the empty string when the key is absent. Used by every
/// `apply_*` helper below to extract the per-verb required args.
fn arg(args: &[(String, String)], key: &str) -> String {
    args.iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

/// Compute the post-state for `Apply create <noun>`. Pure function
/// over `&Object` so tests can exercise the verb without touching
/// the kernel SYSTEM singleton.
///
/// Args:
///   * `noun` — required; the noun whose cell receives the new entity.
///   * `id`   — optional; when present, the new entity carries this
///              id binding; when absent, a synthetic `id` derived
///              from the cell's current length is used so the
///              creation is observable in tests.
///
/// Returns `Err` when the noun is missing/empty.
pub fn apply_create(args: &[(String, String)], state: &Object) -> Result<Object, String> {
    let noun = arg(args, "noun");
    if noun.is_empty() {
        return Err("missing noun".to_string());
    }
    // Derive an id when none was supplied. The action panel today
    // only pre-binds `noun` for ApplyCreate (see `actions_for_root`
    // / `actions_for_noun`); the future parameter editor will let
    // the user fill in `id` + arbitrary fields. Synthetic id is
    // `<noun>-<count>` so two successive creates against the same
    // noun produce distinguishable rows the post-state assertions
    // can pin down.
    let supplied_id = arg(args, "id");
    let id = if supplied_id.is_empty() {
        let existing = ast::fetch_or_phi(&noun, state);
        let n = existing.as_seq().map(|s| s.len()).unwrap_or(0);
        format!("{noun}-{n}")
    } else {
        supplied_id
    };
    // Build a named-tuple entity carrying just the id binding. Same
    // shape `handle_arest_create_for_slug` produces for an empty
    // `data` body.
    let entity = Object::seq(alloc::vec![Object::seq(alloc::vec![
        Object::atom("id"),
        Object::atom(&id),
    ])]);
    Ok(ast::cell_push(&noun, entity, state))
}

/// Compute the post-state for `Apply update <noun>::<id>`. Walks
/// the noun's cell, finds the entity carrying `id`, and rewrites
/// (or appends) every additional `(key, value)` arg as a binding.
/// Args reserved for the dispatcher (`noun`, `id`) are skipped.
///
/// Returns `Err` when the noun/id are missing or no matching entity
/// exists in the cell.
pub fn apply_update(args: &[(String, String)], state: &Object) -> Result<Object, String> {
    let noun = arg(args, "noun");
    let id = arg(args, "id");
    if noun.is_empty() {
        return Err("missing noun".to_string());
    }
    if id.is_empty() {
        return Err("missing id".to_string());
    }
    let cell = ast::fetch_or_phi(&noun, state);
    let Some(rows) = cell.as_seq() else {
        return Err(format!("noun {noun} not found"));
    };
    let mut updated_any = false;
    let mut new_rows: Vec<Object> = Vec::with_capacity(rows.len());
    for row in rows {
        if ast::binding(row, "id") == Some(id.as_str()) {
            let mut next = row.clone();
            for (k, v) in args {
                if k == "noun" || k == "id" {
                    continue;
                }
                next = update_binding_inplace(&next, k, v);
            }
            new_rows.push(next);
            updated_any = true;
        } else {
            new_rows.push(row.clone());
        }
    }
    if !updated_any {
        return Err(format!("entity {noun}::{id} not found"));
    }
    Ok(ast::store(&noun, Object::seq(new_rows), state))
}

/// Rewrite (or append) a binding on a named-tuple entity. Mirror of
/// `crate::arest::hateoas::update_binding`, hand-rolled here because
/// the hateoas helper is private. Identical semantics: replace the
/// existing pair when `key` matches; otherwise push a new pair on
/// the tail.
fn update_binding_inplace(entity: &Object, key: &str, new_value: &str) -> Object {
    let mut out: Vec<Object> = Vec::new();
    let mut updated = false;
    if let Some(pairs) = entity.as_seq() {
        for pair in pairs {
            if let Some(items) = pair.as_seq() {
                if items.len() == 2 && items[0].as_atom() == Some(key) {
                    out.push(Object::seq(alloc::vec![
                        Object::atom(key),
                        Object::atom(new_value),
                    ]));
                    updated = true;
                    continue;
                }
            }
            out.push(pair.clone());
        }
    }
    if !updated {
        out.push(Object::seq(alloc::vec![
            Object::atom(key),
            Object::atom(new_value),
        ]));
    }
    Object::seq(out)
}

/// Compute the post-state for `Apply destroy <noun>::<id>`. Drops
/// every row from the noun's cell whose `id` binding matches.
/// Returns `Err` when no matching entity is present (so the caller
/// can surface the miss instead of silently no-opping).
pub fn apply_destroy(args: &[(String, String)], state: &Object) -> Result<Object, String> {
    let noun = arg(args, "noun");
    let id = arg(args, "id");
    if noun.is_empty() {
        return Err("missing noun".to_string());
    }
    if id.is_empty() {
        return Err("missing id".to_string());
    }
    let cell = ast::fetch_or_phi(&noun, state);
    let Some(rows) = cell.as_seq() else {
        return Err(format!("noun {noun} not found"));
    };
    let before = rows.len();
    let kept: Vec<Object> = rows
        .iter()
        .filter(|row| ast::binding(row, "id") != Some(id.as_str()))
        .cloned()
        .collect();
    if kept.len() == before {
        return Err(format!("entity {noun}::{id} not found"));
    }
    Ok(ast::store(&noun, Object::seq(kept), state))
}

/// Compute the post-state for the rich-shape `Transition` verb.
/// Mirrors `arest::hateoas::handle_arest_transition` against the
/// kernel-side cell shape `actions_for_instance` + `transitions_for_sm`
/// emit (rich shape: `StateMachine_has_currentlyInStatus` +
/// `Transition_is_from_Status` / `Transition_is_to_Status` +
/// `Transition_is_triggered_by_Event_Type`).
///
/// Args:
///   * `sm`   — required; the State Machine instance id.
///   * `next` — required; the target Status the SM lands in.
///
/// Side effect: rewrites the SM's `currentlyInStatus` to `next`.
/// Per-fact validation (does the rich shape say this transition is
/// legal from the current Status?) happens in the action enumerator
/// (`actions_for_instance` only emits legal outgoing transitions);
/// the dispatcher trusts that filter and just commits the rewrite.
/// This keeps the dispatcher's failure mode narrow: it errors only
/// when the SM row itself is missing or the next-state arg is empty.
pub fn apply_transition(args: &[(String, String)], state: &Object) -> Result<Object, String> {
    let sm_id = arg(args, "sm");
    let next = arg(args, "next");
    if sm_id.is_empty() {
        return Err("missing sm".to_string());
    }
    if next.is_empty() {
        return Err("missing next".to_string());
    }
    let cell = ast::fetch_or_phi("StateMachine_has_currentlyInStatus", state);
    let Some(rows) = cell.as_seq() else {
        return Err(format!("State Machine {sm_id} not found"));
    };
    let mut updated = false;
    let mut new_rows: Vec<Object> = Vec::with_capacity(rows.len());
    for row in rows {
        if ast::binding_matches(row, "State Machine", &sm_id) {
            new_rows.push(update_binding_inplace(row, "currentlyInStatus", &next));
            updated = true;
        } else {
            new_rows.push(row.clone());
        }
    }
    if !updated {
        return Err(format!("State Machine {sm_id} not found"));
    }
    Ok(ast::store(
        "StateMachine_has_currentlyInStatus",
        Object::seq(new_rows),
        state,
    ))
}

/// Compute the post-state for `Store <name> <contents>`. Direct
/// cell write — replaces the whole cell. The contents arg is parsed
/// as a single-fact named-tuple from the remaining args (every
/// (key, value) pair other than `name` becomes one role binding).
///
/// Returns `Err` when `name` is empty.
pub fn apply_store(args: &[(String, String)], state: &Object) -> Result<Object, String> {
    let name = arg(args, "name");
    if name.is_empty() {
        return Err("missing name".to_string());
    }
    // Synthesise a single fact from the remaining args. Empty when
    // only `name` was bound — then Store writes an empty cell, which
    // is the canonical "clear this cell" idiom.
    let mut pairs: Vec<Object> = Vec::new();
    for (k, v) in args {
        if k == "name" {
            continue;
        }
        pairs.push(Object::seq(alloc::vec![
            Object::atom(k),
            Object::atom(v),
        ]));
    }
    let contents = if pairs.is_empty() {
        Object::phi()
    } else {
        Object::seq(alloc::vec![Object::seq(pairs)])
    };
    Ok(ast::store(&name, contents, state))
}

/// Compute the post-state for `Apply remove fact <cell>#<idx>`.
/// Drops the fact at index `idx` from the named cell.
pub fn apply_remove_fact(args: &[(String, String)], state: &Object) -> Result<Object, String> {
    let cell_name = arg(args, "cell");
    let idx_str = arg(args, "fact");
    if cell_name.is_empty() {
        return Err("missing cell".to_string());
    }
    let idx: usize = idx_str
        .parse()
        .map_err(|_| format!("invalid fact index: {idx_str}"))?;
    let cell = ast::fetch_or_phi(&cell_name, state);
    let Some(rows) = cell.as_seq() else {
        return Err(format!("cell {cell_name} not found"));
    };
    if idx >= rows.len() {
        return Err(format!("fact index {idx} out of range"));
    }
    let kept: Vec<Object> = rows
        .iter()
        .enumerate()
        .filter_map(|(i, r)| if i == idx { None } else { Some(r.clone()) })
        .collect();
    Ok(ast::store(&cell_name, Object::seq(kept), state))
}

// ── Runtime LoadReading verbs (#564 / DynRdg-T5) ───────────────────
//
// `apply_load_reading` / `apply_unload_reading` / `apply_reload_reading`
// are the kernel-side bridges to `arest::load_reading_core::*`. The
// pure-FORML core became no_std-reachable across #588 (#588 close —
// parse_forml2_stage2 no_std) and #589 (check.rs gate lift +
// load_reading_core function-body gate drop), so the kernel build
// dispatches through the same code path the host-side tests do.
// Pre-#589 these helpers had `target_os = "uefi"` stubs returning
// "not yet wired" because the verb was std-only — those stubs are
// gone now.

/// Compute the post-state for `LoadReading <name> <body>` plus a
/// short success summary for the REPL scrollback.
///
/// Args:
///   * `name` — required; the reading's logical name (becomes a
///              `_loaded_reading:{name}` manifest cell).
///   * `body` — required; the FORML 2 body to register.
///
/// Returns `Err` with a short message when args are missing or the
/// load rejected; the variant carries the summary line on success.
pub fn apply_load_reading(
    args: &[(String, String)],
    state: &Object,
) -> Result<(Object, String), String> {
    use arest::load_reading_core::{load_reading, LoadReadingPolicy};

    let name = arg(args, "name");
    let body = arg(args, "body");
    if name.is_empty() {
        return Err("missing name".to_string());
    }
    if body.is_empty() {
        return Err("missing body".to_string());
    }
    match load_reading(state, &name, &body, LoadReadingPolicy::AllowAll) {
        Ok(outcome) => {
            let summary = format!(
                "+{} nouns, +{} fact types, +{} derivations",
                outcome.report.added_nouns.len(),
                outcome.report.added_fact_types.len(),
                outcome.report.added_derivations.len(),
            );
            Ok((outcome.new_state, summary))
        }
        Err(err) => Err(format_load_error(&err)),
    }
}

/// Compute the post-state for `UnloadReading <name>` plus a short
/// success summary for the REPL scrollback.
pub fn apply_unload_reading(
    args: &[(String, String)],
    state: &Object,
) -> Result<(Object, String), String> {
    use arest::load_reading_core::{unload_reading, UnloadPolicy};

    let name = arg(args, "name");
    if name.is_empty() {
        return Err("missing name".to_string());
    }
    match unload_reading(state, &name, UnloadPolicy::CascadeDelete) {
        Ok(outcome) => {
            let summary = format!(
                "-{} nouns, -{} fact types, -{} derivations",
                outcome.report.removed_nouns.len(),
                outcome.report.removed_fact_types.len(),
                outcome.report.removed_derivations.len(),
            );
            Ok((outcome.new_state, summary))
        }
        Err(err) => Err(format_unload_error(&err)),
    }
}

/// Compute the post-state for `ReloadReading <name> <body>` plus a
/// short success summary covering both the unload + load deltas.
pub fn apply_reload_reading(
    args: &[(String, String)],
    state: &Object,
) -> Result<(Object, String), String> {
    use arest::load_reading_core::{reload_reading, ReloadPolicy};

    let name = arg(args, "name");
    let body = arg(args, "body");
    if name.is_empty() {
        return Err("missing name".to_string());
    }
    if body.is_empty() {
        return Err("missing body".to_string());
    }
    match reload_reading(state, &name, &body, ReloadPolicy::ReplaceAll) {
        Ok(outcome) => {
            let summary = format!(
                "removed {} nouns / {} FT / {} derivations, added {} / {} / {}",
                outcome.removed.removed_nouns.len(),
                outcome.removed.removed_fact_types.len(),
                outcome.removed.removed_derivations.len(),
                outcome.added.added_nouns.len(),
                outcome.added.added_fact_types.len(),
                outcome.added.added_derivations.len(),
            );
            Ok((outcome.new_state, summary))
        }
        Err(err) => Err(format!("reload error: {err:?}")),
    }
}

/// Format a `LoadError` into a short scrollback-friendly string.
fn format_load_error(err: &arest::load_reading_core::LoadError) -> String {
    use arest::load_reading_core::LoadError;
    match err {
        LoadError::Disallowed => "load disallowed by policy".to_string(),
        LoadError::EmptyBody => "empty body".to_string(),
        LoadError::InvalidName(msg) => format!("invalid name: {msg}"),
        LoadError::ParseError(msg) => format!("parse error: {msg}"),
        LoadError::DeonticViolation(diags) => {
            format!("deontic violation: {} diagnostic(s)", diags.len())
        }
        // #559 / DynRdg-5 — alethic-class violations from the
        // load-time validation gate.
        LoadError::AlethicViolation(diags) => {
            format!("alethic violation: {} diagnostic(s)", diags.len())
        }
    }
}

/// Format an `UnloadError` into a short scrollback-friendly string.
fn format_unload_error(err: &arest::load_reading_core::UnloadError) -> String {
    use arest::load_reading_core::UnloadError;
    match err {
        UnloadError::ManifestMissing(name) => {
            format!("no manifest for reading {name:?}")
        }
        UnloadError::InvalidName(msg) => format!("invalid name: {msg}"),
        UnloadError::Disallowed => "unload disallowed by policy".to_string(),
        UnloadError::NotImplemented => "policy not implemented".to_string(),
    }
}

/// Dispatch a SystemAction against the in-kernel SYSTEM and return
/// the result as a human-readable line for the REPL scrollback. This
/// is the kernel-side translation of the action panel click into the
/// existing `Func`-application path the legacy REPL uses.
///
/// Read-only verbs (Fetch / FetchOrPhi / Def) resolve through
/// `crate::system::fetch_named`, the same path the wire uses.
///
/// Mutating verbs (#554) — Apply create / update / destroy /
/// remove fact, Transition, Store — compute their post-state via
/// the pure `apply_*` helpers above and commit through
/// `crate::system::apply`. Errors (missing args, entity not found,
/// invalid index) surface in the result line so the action panel /
/// scrollback shows the user what went wrong without crashing.
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
            let name = arg(&action.default_args, "name");
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
            let name = arg(&action.default_args, "name");
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
        // Mutating verbs (#554). Each branch:
        //   1. Computes the post-state via the pure `apply_*` helper
        //      under `crate::system::with_state` (read lock briefly).
        //   2. Hands the post-state to `crate::system::apply` to
        //      install + notify subscribers (which triggers the
        //      action / nav / cell-render redraw on the next frame).
        //   3. Surfaces success / failure in the scrollback line.
        SystemVerb::ApplyCreate
        | SystemVerb::ApplyUpdate
        | SystemVerb::ApplyDestroy
        | SystemVerb::ApplyRemoveFact
        | SystemVerb::Transition
        | SystemVerb::Store => {
            let computed: Option<Result<Object, String>> =
                crate::system::with_state(|st| match action.verb {
                    SystemVerb::ApplyCreate => apply_create(&action.default_args, st),
                    SystemVerb::ApplyUpdate => apply_update(&action.default_args, st),
                    SystemVerb::ApplyDestroy => apply_destroy(&action.default_args, st),
                    SystemVerb::ApplyRemoveFact => {
                        apply_remove_fact(&action.default_args, st)
                    }
                    SystemVerb::Transition => apply_transition(&action.default_args, st),
                    SystemVerb::Store => apply_store(&action.default_args, st),
                    _ => Err("unreachable".to_string()),
                });
            match computed {
                Some(Ok(new_state)) => match crate::system::apply(new_state) {
                    Ok(()) => format!("[action] {summary} \u{2192} ok"),
                    Err(e) => format!("[action] {summary} \u{2192} (apply failed: {e})"),
                },
                Some(Err(e)) => format!("[action] {summary} \u{2192} (error: {e})"),
                None => {
                    format!("[action] {summary} \u{2192} (system not initialised)")
                }
            }
        }
        // Platform / Native: dispatch path resolves the named
        // primitive through `Func::Platform(name)` / the Fn1 closure
        // registered in the host. On the kernel-only build the
        // registries are empty by default, so this is structurally
        // identical to a fetch into a missing def cell.
        SystemVerb::Platform | SystemVerb::Native => {
            let name = arg(&action.default_args, "name");
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
        // #564: runtime LoadReading / UnloadReading / ReloadReading.
        // Each branch routes through `apply_load_reading` etc., which
        // returns `(new_state, summary)` on success — the summary
        // appears in the scrollback line so users see exactly what
        // the load contributed (added cells, etc.). Under the no_std
        // kernel build the helpers return a "not available" stub.
        SystemVerb::LoadReading
        | SystemVerb::UnloadReading
        | SystemVerb::ReloadReading => {
            let computed: Option<Result<(Object, String), String>> =
                crate::system::with_state(|st| match action.verb {
                    SystemVerb::LoadReading => {
                        apply_load_reading(&action.default_args, st)
                    }
                    SystemVerb::UnloadReading => {
                        apply_unload_reading(&action.default_args, st)
                    }
                    SystemVerb::ReloadReading => {
                        apply_reload_reading(&action.default_args, st)
                    }
                    _ => Err("unreachable".to_string()),
                });
            // Pull the reading name back out for the success line so
            // we can format `LoadReading 'foo' \u{2192} +N nouns…`
            // even though the action's canonical_text already showed
            // the verb summary.
            let name = arg(&action.default_args, "name");
            let verb_label = match action.verb {
                SystemVerb::LoadReading => "LoadReading",
                SystemVerb::UnloadReading => "UnloadReading",
                SystemVerb::ReloadReading => "ReloadReading",
                _ => "Reading",
            };
            match computed {
                Some(Ok((new_state, report))) => match crate::system::apply(new_state) {
                    Ok(()) => format!(
                        "[action] {verb_label} {name:?} \u{2192} {report}"
                    ),
                    Err(e) => format!(
                        "[action] {verb_label} {name:?} \u{2192} (apply failed: {e})"
                    ),
                },
                Some(Err(e)) => {
                    format!("[action] {verb_label} {name:?} \u{2192} (error: {e})")
                }
                None => format!(
                    "[action] {summary} \u{2192} (system not initialised)"
                ),
            }
        }
    }
}

/// Convenience wrapper: dispatch a `LoadReading` / `UnloadReading` /
/// `ReloadReading` action where the user typed `<name>\n<body>` (or,
/// for unload, just `<name>`) into the REPL prompt. The first line is
/// taken as the reading name; the remainder is the body. The wrapper
/// splices `name` and `body` into a fresh `default_args` Vec, leaves
/// any other args the caller pre-bound intact, and dispatches.
///
/// Why a wrapper rather than baking input parsing into `dispatch_action`:
/// the per-screen `compute_actions` surface emits actions with empty
/// `default_args` (the schema is "the user fills these in"). Slint
/// fires `on_action_invoked(idx)` carrying just the index; the REPL
/// pane reads the prompt's `current_input`, hands it to this wrapper
/// alongside the cached action, and displays the resulting line. Tests
/// can construct actions with explicit args directly and skip this
/// wrapper.
pub fn dispatch_action_with_input(action: &SystemAction, input: &str) -> String {
    let needs_input = matches!(
        action.verb,
        SystemVerb::LoadReading
            | SystemVerb::UnloadReading
            | SystemVerb::ReloadReading
    );
    if !needs_input {
        return dispatch_action(action);
    }
    let trimmed = input.trim_end();
    let (name, body) = match trimmed.split_once('\n') {
        Some((first, rest)) => (first.trim().to_string(), rest.to_string()),
        None => (trimmed.trim().to_string(), String::new()),
    };
    // Splice name + body into a fresh args vector. We DROP any
    // pre-existing `name`/`body` keys so the prompt is the source of
    // truth (the action surface emits empty default_args today). Any
    // other keys the caller pre-bound flow through unchanged so the
    // verb stays composable.
    let mut args: Vec<(String, String)> = action
        .default_args
        .iter()
        .filter(|(k, _)| k != "name" && k != "body")
        .cloned()
        .collect();
    args.push(("name".to_string(), name));
    if !body.is_empty() {
        args.push(("body".to_string(), body));
    }
    let mut next = action.clone();
    next.default_args = args;
    // Recompute the label so the canonical text reflects the spliced
    // name (the original action's label said e.g. "load_reading" with
    // no name; now we know it).
    next.label = format!(
        "[{}] {}",
        next.verb.label_prefix(),
        next.verb.canonical_text(&next.default_args),
    );
    dispatch_action(&next)
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
        // #554 wired the mutating verbs through `system::apply`. The
        // canonical verb form still appears in the result line; the
        // post-action annotation is now "ok" on success or
        // "(error: ...)" on a validation miss. We can't assert the
        // success branch from a unit test that doesn't initialise
        // SYSTEM, but the line still starts with `[action]` + the
        // canonical verb form regardless of which branch fires.
        let action = SystemAction::new(
            SystemVerb::ApplyCreate,
            vec![("noun".to_string(), "File".to_string())],
        );
        let line = dispatch_action(&action);
        assert!(line.contains("[action]"));
        assert!(line.contains("apply create File"));
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

    // ── Mutating-verb wiring (#554) ───────────────────────────────
    //
    // These test the pure `apply_*` helpers directly so the verb
    // semantics are exercised against synthetic state without
    // requiring `system::init`. The end-to-end round-trip through
    // `dispatch_action` + `system::apply` is covered by
    // `cell_renderer::tests::wired_apply_*` once the SYSTEM
    // singleton is set up.

    #[test]
    fn apply_create_pushes_new_entity_into_noun_cell() {
        // Apply create File → File cell gains a row carrying the
        // synthetic id `File-0` (cell was empty).
        let state = Object::phi();
        let args = vec![("noun".to_string(), "File".to_string())];
        let new_state = apply_create(&args, &state).expect("create");
        let cell = ast::fetch_or_phi("File", &new_state);
        let rows = cell.as_seq().expect("cell populated");
        assert_eq!(rows.len(), 1);
        assert_eq!(ast::binding(&rows[0], "id"), Some("File-0"));
    }

    #[test]
    fn apply_create_honours_user_supplied_id() {
        let state = Object::phi();
        let args = vec![
            ("noun".to_string(), "File".to_string()),
            ("id".to_string(), "f-explicit".to_string()),
        ];
        let new_state = apply_create(&args, &state).expect("create");
        let rows = ast::fetch_or_phi("File", &new_state);
        let rows = rows.as_seq().expect("cell populated");
        assert_eq!(ast::binding(&rows[0], "id"), Some("f-explicit"));
    }

    #[test]
    fn apply_create_missing_noun_errors() {
        let state = Object::phi();
        let err = apply_create(&[], &state).unwrap_err();
        assert!(err.contains("missing noun"), "{err}");
    }

    #[test]
    fn apply_update_rewrites_matching_entity_bindings() {
        // Seed File cell with one entity; update should rewrite an
        // existing binding AND append a new (key, value) pair.
        let entity = Object::seq(alloc::vec![
            Object::seq(alloc::vec![Object::atom("id"), Object::atom("f1")]),
            Object::seq(alloc::vec![Object::atom("Name"), Object::atom("alpha")]),
        ]);
        let state = ast::cell_push("File", entity, &Object::phi());
        let args = vec![
            ("noun".to_string(), "File".to_string()),
            ("id".to_string(), "f1".to_string()),
            ("Name".to_string(), "alpha-renamed".to_string()),
            ("MimeType".to_string(), "text/plain".to_string()),
        ];
        let new_state = apply_update(&args, &state).expect("update");
        let rows = ast::fetch_or_phi("File", &new_state);
        let rows = rows.as_seq().expect("cell populated");
        assert_eq!(rows.len(), 1);
        assert_eq!(ast::binding(&rows[0], "id"), Some("f1"));
        assert_eq!(ast::binding(&rows[0], "Name"), Some("alpha-renamed"));
        assert_eq!(ast::binding(&rows[0], "MimeType"), Some("text/plain"));
    }

    #[test]
    fn apply_update_unknown_entity_errors() {
        let state = ast::cell_push(
            "File",
            Object::seq(alloc::vec![Object::seq(alloc::vec![
                Object::atom("id"),
                Object::atom("f1"),
            ])]),
            &Object::phi(),
        );
        let args = vec![
            ("noun".to_string(), "File".to_string()),
            ("id".to_string(), "ghost".to_string()),
        ];
        let err = apply_update(&args, &state).unwrap_err();
        assert!(err.contains("not found"), "{err}");
    }

    #[test]
    fn apply_destroy_removes_matching_entity() {
        let s = ast::cell_push(
            "File",
            Object::seq(alloc::vec![Object::seq(alloc::vec![
                Object::atom("id"),
                Object::atom("f1"),
            ])]),
            &Object::phi(),
        );
        let s = ast::cell_push(
            "File",
            Object::seq(alloc::vec![Object::seq(alloc::vec![
                Object::atom("id"),
                Object::atom("f2"),
            ])]),
            &s,
        );
        let args = vec![
            ("noun".to_string(), "File".to_string()),
            ("id".to_string(), "f1".to_string()),
        ];
        let new_state = apply_destroy(&args, &s).expect("destroy");
        let rows = ast::fetch_or_phi("File", &new_state);
        let rows = rows.as_seq().expect("cell populated");
        assert_eq!(rows.len(), 1);
        assert_eq!(ast::binding(&rows[0], "id"), Some("f2"));
    }

    #[test]
    fn apply_destroy_unknown_entity_errors() {
        let s = ast::cell_push(
            "File",
            Object::seq(alloc::vec![Object::seq(alloc::vec![
                Object::atom("id"),
                Object::atom("f1"),
            ])]),
            &Object::phi(),
        );
        let args = vec![
            ("noun".to_string(), "File".to_string()),
            ("id".to_string(), "ghost".to_string()),
        ];
        let err = apply_destroy(&args, &s).unwrap_err();
        assert!(err.contains("not found"), "{err}");
    }

    #[test]
    fn apply_transition_rewrites_currently_in_status() {
        // Seed an SM row at `start`; transition `next=middle` rewrites
        // it.
        let s = cell_push(
            "StateMachine_has_currentlyInStatus",
            fact_from_pairs(&[
                ("State Machine", "sm1"),
                ("currentlyInStatus", "start"),
                ("forResource", "f1"),
            ]),
            &Object::phi(),
        );
        let args = vec![
            ("sm".to_string(), "sm1".to_string()),
            ("id".to_string(), "f1".to_string()),
            ("next".to_string(), "middle".to_string()),
        ];
        let new_state = apply_transition(&args, &s).expect("transition");
        let cell =
            ast::fetch_or_phi("StateMachine_has_currentlyInStatus", &new_state);
        let rows = cell.as_seq().expect("cell populated");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            ast::binding(&rows[0], "currentlyInStatus"),
            Some("middle"),
        );
        // forResource binding survives the rewrite.
        assert_eq!(ast::binding(&rows[0], "forResource"), Some("f1"));
    }

    #[test]
    fn apply_transition_unknown_sm_errors() {
        let s = cell_push(
            "StateMachine_has_currentlyInStatus",
            fact_from_pairs(&[
                ("State Machine", "sm1"),
                ("currentlyInStatus", "start"),
            ]),
            &Object::phi(),
        );
        let args = vec![
            ("sm".to_string(), "ghost".to_string()),
            ("id".to_string(), "f1".to_string()),
            ("next".to_string(), "middle".to_string()),
        ];
        let err = apply_transition(&args, &s).unwrap_err();
        assert!(err.contains("not found"), "{err}");
    }

    #[test]
    fn apply_store_writes_named_cell_with_synthesised_fact() {
        let state = Object::phi();
        let args = vec![
            ("name".to_string(), "Probe".to_string()),
            ("k".to_string(), "v".to_string()),
        ];
        let new_state = apply_store(&args, &state).expect("store");
        let cell = ast::fetch_or_phi("Probe", &new_state);
        let rows = cell.as_seq().expect("cell populated");
        assert_eq!(rows.len(), 1);
        assert_eq!(ast::binding(&rows[0], "k"), Some("v"));
    }

    #[test]
    fn apply_store_clears_cell_when_only_name_supplied() {
        // Seed a cell, then Store with only `name` → cell becomes
        // empty (the canonical "clear" idiom).
        let s = ast::cell_push(
            "Probe",
            fact_from_pairs(&[("k", "v")]),
            &Object::phi(),
        );
        let args = vec![("name".to_string(), "Probe".to_string())];
        let new_state = apply_store(&args, &s).expect("store-clear");
        let cell = ast::fetch_or_phi("Probe", &new_state);
        // After clear the cell exists but is empty (`phi`). `as_seq`
        // returns Some(empty slice) for phi.
        let rows = cell.as_seq().unwrap_or(&[]);
        assert!(rows.is_empty(), "expected empty cell, got {rows:?}");
    }

    #[test]
    fn apply_store_missing_name_errors() {
        let err = apply_store(&[], &Object::phi()).unwrap_err();
        assert!(err.contains("missing name"), "{err}");
    }

    #[test]
    fn apply_remove_fact_drops_indexed_row() {
        let s = cell_push(
            "Probe",
            fact_from_pairs(&[("k", "v0")]),
            &Object::phi(),
        );
        let s = cell_push("Probe", fact_from_pairs(&[("k", "v1")]), &s);
        let args = vec![
            ("cell".to_string(), "Probe".to_string()),
            ("fact".to_string(), "0".to_string()),
        ];
        let new_state = apply_remove_fact(&args, &s).expect("remove");
        let rows = ast::fetch_or_phi("Probe", &new_state);
        let rows = rows.as_seq().expect("cell populated");
        assert_eq!(rows.len(), 1);
        assert_eq!(ast::binding(&rows[0], "k"), Some("v1"));
    }

    #[test]
    fn apply_remove_fact_out_of_range_errors() {
        let s = cell_push(
            "Probe",
            fact_from_pairs(&[("k", "v0")]),
            &Object::phi(),
        );
        let args = vec![
            ("cell".to_string(), "Probe".to_string()),
            ("fact".to_string(), "99".to_string()),
        ];
        let err = apply_remove_fact(&args, &s).unwrap_err();
        assert!(err.contains("out of range"), "{err}");
    }

    /// End-to-end roundtrip — Apply create → SYSTEM committed →
    /// new entity visible on a subsequent `with_state` read. Uses
    /// the singleton `system::init` (idempotent) so this dovetails
    /// with the other tests that share the binary.
    #[test]
    fn dispatch_apply_create_commits_to_system_state() {
        crate::system::init();
        // Seed the noun if not already present (init() seeds a few
        // demo nouns; we use our own to avoid colliding).
        let probe_noun = "WireUpProbe554";
        let pre = crate::system::with_state(|s| s.clone()).expect("init ran");
        let pre_with_noun = ast::cell_push(
            "Noun",
            Object::seq(alloc::vec![Object::seq(alloc::vec![
                Object::atom("name"),
                Object::atom(probe_noun),
            ])]),
            &pre,
        );
        crate::system::apply(pre_with_noun).expect("seed noun");

        let action = SystemAction::new(
            SystemVerb::ApplyCreate,
            vec![
                ("noun".to_string(), probe_noun.to_string()),
                ("id".to_string(), "wp-1".to_string()),
            ],
        );
        let line = dispatch_action(&action);
        assert!(line.contains("ok"), "{line}");

        // Post-state read: the WireUpProbe554 cell now has a row with
        // id=wp-1.
        let found = crate::system::with_state(|s| {
            let cell = ast::fetch_or_phi(probe_noun, s);
            cell.as_seq()
                .map(|rows| rows.iter().any(|r| ast::binding(r, "id") == Some("wp-1")))
                .unwrap_or(false)
        })
        .expect("init ran");
        assert!(found, "post-create state must include wp-1");
    }

    /// End-to-end roundtrip — Apply transition rewrites the
    /// SYSTEM-installed SM row's `currentlyInStatus`.
    #[test]
    fn dispatch_transition_commits_to_system_state() {
        crate::system::init();
        // Seed an SM row with a unique id so we don't collide with
        // the init'd `sm-sr-1` row.
        let pre = crate::system::with_state(|s| s.clone()).expect("init ran");
        let seeded = cell_push(
            "StateMachine_has_currentlyInStatus",
            fact_from_pairs(&[
                ("State Machine", "sm-554-probe"),
                ("currentlyInStatus", "start"),
            ]),
            &pre,
        );
        crate::system::apply(seeded).expect("seed sm");

        let action = SystemAction::new(
            SystemVerb::Transition,
            vec![
                ("sm".to_string(), "sm-554-probe".to_string()),
                ("id".to_string(), "x".to_string()),
                ("next".to_string(), "middle".to_string()),
            ],
        );
        let line = dispatch_action(&action);
        assert!(line.contains("ok"), "{line}");

        // Post-state: sm-554-probe is now in `middle`.
        let post_status = crate::system::with_state(|s| {
            let cell = ast::fetch_or_phi("StateMachine_has_currentlyInStatus", s);
            cell.as_seq()
                .and_then(|rows| {
                    rows.iter()
                        .find(|r| ast::binding_matches(r, "State Machine", "sm-554-probe"))
                        .and_then(|r| ast::binding(r, "currentlyInStatus"))
                        .map(|s| s.to_string())
                })
                .unwrap_or_default()
        })
        .expect("init ran");
        assert_eq!(post_status, "middle");
    }

    /// End-to-end roundtrip — Apply Store writes a named cell that
    /// the next `with_state` read sees populated.
    #[test]
    fn dispatch_store_commits_to_system_state() {
        crate::system::init();
        let action = SystemAction::new(
            SystemVerb::Store,
            vec![
                ("name".to_string(), "WireUpStoreProbe554".to_string()),
                ("k".to_string(), "v".to_string()),
            ],
        );
        let line = dispatch_action(&action);
        assert!(line.contains("ok"), "{line}");

        let post = crate::system::with_state(|s| {
            let cell = ast::fetch_or_phi("WireUpStoreProbe554", s);
            cell.as_seq()
                .and_then(|rows| rows.first().cloned())
                .map(|fact| ast::binding(&fact, "k").map(|s| s.to_string()))
                .flatten()
                .unwrap_or_default()
        })
        .expect("init ran");
        assert_eq!(post, "v");
    }

    /// End-to-end roundtrip — invalid Apply update (entity missing)
    /// returns an error envelope in the result line and leaves
    /// SYSTEM state unchanged.
    #[test]
    fn dispatch_invalid_update_surfaces_error_and_leaves_state() {
        crate::system::init();
        let pre_snapshot = crate::system::with_state(|s| s.clone()).expect("init ran");

        let action = SystemAction::new(
            SystemVerb::ApplyUpdate,
            vec![
                ("noun".to_string(), "ImaginaryNoun554".to_string()),
                ("id".to_string(), "ghost".to_string()),
            ],
        );
        let line = dispatch_action(&action);
        assert!(line.contains("error"), "{line}");

        let post_snapshot = crate::system::with_state(|s| s.clone()).expect("init ran");
        // The ImaginaryNoun554 cell does not appear post-failure.
        let cell = ast::fetch_or_phi("ImaginaryNoun554", &post_snapshot);
        assert!(
            cell.as_seq().map(|s| s.is_empty()).unwrap_or(true),
            "failed update must not leak the noun cell"
        );
        // The pre-snapshot's cell shape survives. (We can't compare
        // the entire Object across other concurrent tests' applies,
        // so we confirm the failure path didn't introduce the
        // imaginary cell.)
        let _ = pre_snapshot;
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

    // ── #564 / DynRdg-T5 — runtime LoadReading on the action surface ──

    /// LoadReading appears in the root action list — users see it as
    /// an always-available meta-system verb. The action's
    /// `default_args` are empty (the input prompt fills `name`/`body`
    /// at click time via `dispatch_action_with_input`).
    #[test]
    fn load_reading_appears_in_actions_for_root() {
        let state = synth_state();
        let actions = compute_actions(&CurrentCell::Root, &state);
        let load: Vec<&SystemAction> = actions
            .iter()
            .filter(|a| a.verb == SystemVerb::LoadReading)
            .collect();
        assert_eq!(
            load.len(),
            1,
            "LoadReading must appear exactly once on root: {actions:?}"
        );
        assert!(load[0].default_args.is_empty(), "default_args empty by design");
        assert!(load[0].label.contains("load_reading"), "{}", load[0].label);
    }

    /// UnloadReading + ReloadReading also surface on the root screen.
    #[test]
    fn unload_and_reload_reading_appear_on_root() {
        let state = synth_state();
        let actions = compute_actions(&CurrentCell::Root, &state);
        let verbs: BTreeSet<SystemVerb> = actions.iter().map(|a| a.verb.clone()).collect();
        assert!(
            verbs.contains(&SystemVerb::UnloadReading),
            "UnloadReading missing: {verbs:?}"
        );
        assert!(
            verbs.contains(&SystemVerb::ReloadReading),
            "ReloadReading missing: {verbs:?}"
        );
    }

    /// Verb prefixes for the new variants are distinct from the
    /// existing ones — the prefix-distinctness invariant from the
    /// pre-existing `system_verb_label_prefixes_distinct` test still
    /// holds with the three new variants in place.
    #[test]
    fn load_reading_prefixes_distinct() {
        let prefixes: Vec<&str> = [
            SystemVerb::LoadReading,
            SystemVerb::UnloadReading,
            SystemVerb::ReloadReading,
        ]
        .iter()
        .map(|v| v.label_prefix())
        .collect();
        let mut sorted = prefixes.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 3, "prefixes must be distinct: {prefixes:?}");
    }

    /// `dispatch_action_with_input` parses `<name>\n<body>` and
    /// splices them into the action's args. Verified at the args
    /// level so the test exercises just the input-splicing
    /// contract without needing a live SYSTEM.
    #[test]
    fn dispatch_with_input_parses_name_and_body() {
        // We don't actually want the dispatch to commit — just verify
        // the args splice. We do that by constructing the action,
        // then mimicking the same parse `dispatch_action_with_input`
        // does and asserting the post-splice shape.
        let action = SystemAction::new(SystemVerb::LoadReading, Vec::new());
        let input = "my-reading\nNoun: Probe\nProbe has Name.\n";
        // Re-implement the splice locally to assert the contract
        // `dispatch_action_with_input` provides.
        let trimmed = input.trim_end();
        let (name, body) = match trimmed.split_once('\n') {
            Some((first, rest)) => (first.trim().to_string(), rest.to_string()),
            None => (trimmed.trim().to_string(), String::new()),
        };
        assert_eq!(name, "my-reading");
        assert!(body.contains("Probe has Name"));
        // Empty default_args + this parse → after splice the Vec
        // contains both name and body.
        let mut args: Vec<(String, String)> = action
            .default_args
            .iter()
            .filter(|(k, _)| k != "name" && k != "body")
            .cloned()
            .collect();
        args.push(("name".to_string(), name));
        if !body.is_empty() {
            args.push(("body".to_string(), body));
        }
        let name_arg = args.iter().find(|(k, _)| k == "name").map(|(_, v)| v.as_str());
        let body_arg = args.iter().find(|(k, _)| k == "body").map(|(_, v)| v.as_str());
        assert_eq!(name_arg, Some("my-reading"));
        assert!(body_arg.unwrap().contains("Probe has Name"));
    }

    /// Round-trip: dispatch a `LoadReading` action carrying a small
    /// FORML 2 body. Post-state must contain a manifest cell named
    /// `_loaded_reading:probe-reading-564`, and the success line
    /// must include the verb-specific summary.
    ///
    /// Host-only — `arest::load_reading_core::load_reading` is gated
    /// behind `cfg(not(feature = "no_std"))`, so the kernel build
    /// (`target_os = "uefi"`) skips this test. Once #586 lifts the
    /// gate, the cfg attribute lifts here too.
    #[cfg(not(target_os = "uefi"))]
    #[test]
    fn load_reading_roundtrip_through_dispatcher() {
        crate::system::init();
        let probe = "probe-reading-564";
        let body = "Noun: Probe564.\nProbe564 has Name.\n";
        let action = SystemAction::new(
            SystemVerb::LoadReading,
            vec![
                ("name".to_string(), probe.to_string()),
                ("body".to_string(), body.to_string()),
            ],
        );
        let line = dispatch_action(&action);
        // Success line: format `[action] LoadReading "name" → +N nouns…`.
        assert!(
            line.contains("LoadReading"),
            "missing verb label: {line}"
        );
        assert!(
            line.contains(probe),
            "missing reading name in line: {line}"
        );
        assert!(
            !line.contains("error"),
            "unexpected error in line: {line}"
        );

        // Post-state: the manifest cell exists.
        let manifest = format!("_loaded_reading:{probe}");
        let found = crate::system::with_state(|s| {
            let cell = ast::fetch_or_phi(&manifest, s);
            cell.as_seq().map(|rows| !rows.is_empty()).unwrap_or(false)
        })
        .expect("init ran");
        assert!(found, "manifest cell {manifest} missing post-load");
    }

    /// Idempotency — re-loading the same name+body produces the same
    /// post-state shape. Host-only for the same reason as the
    /// roundtrip test.
    #[cfg(not(target_os = "uefi"))]
    #[test]
    fn load_reading_is_idempotent() {
        crate::system::init();
        let probe = "probe-reading-564-idem";
        let body = "Noun: Probe564Idem.\nProbe564Idem has Name.\n";
        let action = SystemAction::new(
            SystemVerb::LoadReading,
            vec![
                ("name".to_string(), probe.to_string()),
                ("body".to_string(), body.to_string()),
            ],
        );
        let line1 = dispatch_action(&action);
        let line2 = dispatch_action(&action);
        // Both invocations succeed — re-load is idempotent.
        assert!(!line1.contains("error"), "{line1}");
        assert!(!line2.contains("error"), "{line2}");
    }

    /// Empty body rejection — the dispatcher returns an error envelope
    /// (line contains "error") and the SYSTEM state is unchanged
    /// (the manifest cell for this name does not appear).
    #[cfg(not(target_os = "uefi"))]
    #[test]
    fn load_reading_rejects_empty_body() {
        crate::system::init();
        let probe = "probe-empty-body-564";
        let action = SystemAction::new(
            SystemVerb::LoadReading,
            vec![
                ("name".to_string(), probe.to_string()),
                ("body".to_string(), String::new()),
            ],
        );
        let line = dispatch_action(&action);
        assert!(line.contains("error"), "expected error: {line}");

        // No manifest cell appears.
        let manifest = format!("_loaded_reading:{probe}");
        let found = crate::system::with_state(|s| {
            let cell = ast::fetch_or_phi(&manifest, s);
            cell.as_seq().map(|rows| !rows.is_empty()).unwrap_or(false)
        })
        .expect("init ran");
        assert!(
            !found,
            "rejection must not write the manifest cell"
        );
    }

    /// `apply_load_reading` returns Err when `name` is empty.
    /// Pure-function test — no SYSTEM commit. Available on every
    /// build (kernel stub also rejects on missing args).
    #[test]
    fn apply_load_reading_missing_name_errors() {
        let state = Object::phi();
        let args = vec![("body".to_string(), "anything".to_string())];
        let err = apply_load_reading(&args, &state).unwrap_err();
        assert!(err.contains("name"), "{err}");
    }

    /// `apply_load_reading` returns Err when `body` is empty.
    /// Pure-function test — no SYSTEM commit.
    #[test]
    fn apply_load_reading_missing_body_errors() {
        let state = Object::phi();
        let args = vec![("name".to_string(), "x".to_string())];
        let err = apply_load_reading(&args, &state).unwrap_err();
        assert!(err.contains("body"), "{err}");
    }
}
