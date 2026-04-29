// crates/arest-kernel/src/ui_apps/navigation.rs
//
// Navigation actions as cells (#512, EPIC #496).
//
// VVVV's #511 (`5792b53`) landed cell-as-screen rendering: every screen
// IS a cell. The next REPL EPIC sub-task asks for the dual: every
// "navigate to X" action IS *also* a cell. The user's vision in #496
// frames the principle as HATEOAS realised through the cell graph
// itself — the catalogue of legal next moves from a screen is computed
// by reading the cell graph, not hand-listed per screen:
//
//   "It provides SYSTEM calls, but it should be easy to navigate the
//    system using its own principles"
//
// In other words: the unified REPL never has to maintain a per-screen
// menu of "where can I go from here?". The legal-next-moves catalogue
// for the current cell is recomputed from the FT graph on every redraw:
//
//   * Roles  — the joinable cells that share a Noun with the current
//              cell (Noun → its FactTypes; FactType → its role nouns).
//   * Derivations — antecedents (cells consumed) + consequents (cells
//                   produced); modelled here as the back-references
//                   off the cell's instance values (a fact in cell B
//                   that uses an instance from cell A IS a directed
//                   edge A → B in the derivation graph).
//   * State Machine — for any instance whose noun participates in a
//                     `StateMachine_has_currentlyInStatus` fact,
//                     navigate to the SM cell so the user can see /
//                     act on the next-states. The actual transition
//                     invocation is #514's job; #512 only surfaces the
//                     navigation step.
//
// This module is the kernel-side derivation engine for that catalogue.
// It is intentionally pure: `compute_navigation_targets(cell, state)
// -> Vec<NavigationTarget>` reads `&Object` and emits owned data so
// the `system::with_state` caller can drop the read lock the moment
// the function returns. The Slint surface in `unified_repl` picks up
// the targets and renders each as a clickable affordance row.
//
// # Why a separate module
//
// The derivation logic is ~300 lines once the per-cell-type rules are
// fully captured + tested. Inlining it into `cell_renderer.rs` would
// crowd out the rendering rule that module is named for. The two
// modules read the same `Object` shape but have orthogonal concerns:
// `cell_renderer` answers "what does this cell look like on screen?";
// `navigation` answers "what cells can I reach from here?".
//
// # Conceptual model — every navigation IS a cell
//
// The HATEOAS-via-cell-graph principle is more than a UI shortcut.
// Each `NavigationTarget` produced here corresponds to a *fact* of the
// implicit fact type:
//
//     <CurrentCellRef> has navigation_target <TargetCellRef>
//
// We do not actually materialise these into a `current_cell_has_navigation_target`
// cell on D today (the current cell is ephemeral UI state, not a
// persisted concept), but the shape is intentional: a future commit
// could reify the catalogue as a SYSTEM-level cell so an external
// caller could query "what's reachable from this cell?" through the
// same MCP / SYSTEM verbs that read every other cell. The kind tag on
// each target (`Role`, `Derivation`, `StateMachine`, etc.) preserves
// the navigational provenance — clients (the REPL today, an LLM agent
// tomorrow) can filter by category.

#![allow(dead_code)]

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use arest::ast::{self, Object};

use crate::ui_apps::cell_renderer::CurrentCell;

// ── NavigationTarget ───────────────────────────────────────────────

/// One legal "navigate to X" affordance derived from the cell graph.
/// Each target IS a cell of the implicit fact type
/// `<current_cell> has navigation_target <target>`. The renderer
/// surfaces these as clickable rows; the click handler swaps the
/// REPL's current cell to `target`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationTarget {
    /// The cell to navigate to when this affordance fires.
    pub target: CurrentCell,
    /// Provenance — *why* this target appears in the catalogue.
    /// Surfaced in the affordance label (e.g. "[role] File") so the
    /// user can see the structural reason for the navigation.
    pub kind: NavigationKind,
    /// Human-readable label for the affordance row. Composed by
    /// `target.label()` plus a kind hint. Pre-computed here so the
    /// Slint side doesn't need to do any string assembly.
    pub label: String,
}

impl NavigationTarget {
    pub(crate) fn new(target: CurrentCell, kind: NavigationKind) -> Self {
        let prefix = kind.label_prefix();
        let label = format!("[{prefix}] {}", target.label());
        Self { target, kind, label }
    }
}

/// Provenance tag — *why* a `NavigationTarget` appears in the
/// catalogue. Mirrors the navigation derivation rules in the module
/// docstring. Carried through to the UI label so the user can tell
/// "navigate to File because this cell uses a File role" apart from
/// "navigate to File because this is a back-reference from a
/// Tag_is_on_File fact".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NavigationKind {
    /// The target cell shares a Noun with the current cell — joinable
    /// via the FT graph's role positions. The most common kind.
    Role,
    /// The target cell is the parent type of the current instance.
    Type,
    /// The target cell is one of the current cell's instances.
    Instance,
    /// The target cell consumes the current cell (derivation antecedent).
    Antecedent,
    /// The target cell is produced from the current cell (derivation consequent).
    Consequent,
    /// The target is the current cell's State Machine entry — surfaced
    /// when an instance's noun participates in a SM fact. The actual
    /// transition invocation is #514's job; this just navigates so
    /// the user can see the SM's current status + next-states.
    StateMachine,
}

impl NavigationKind {
    /// Short tag shown in the affordance label.
    pub fn label_prefix(&self) -> &'static str {
        match self {
            NavigationKind::Role => "role",
            NavigationKind::Type => "type",
            NavigationKind::Instance => "instance",
            NavigationKind::Antecedent => "antecedent",
            NavigationKind::Consequent => "consequent",
            NavigationKind::StateMachine => "sm",
        }
    }
}

// ── Public entry ───────────────────────────────────────────────────

/// Walk the cell graph and return every legal navigation target
/// reachable from `current_cell`. Pure function — `with_state` callers
/// can drop the read lock the moment this returns.
///
/// The result is sorted (kind then label) so successive redraws emit
/// identical orderings; the Slint side relies on the index into this
/// vector to identify which affordance the user clicked.
pub fn compute_navigation_targets(
    current_cell: &CurrentCell,
    state: &Object,
) -> Vec<NavigationTarget> {
    let mut out = match current_cell {
        CurrentCell::Root => targets_for_root(state),
        CurrentCell::Noun { noun } => targets_for_noun(noun, state),
        CurrentCell::Instance { noun, instance } => {
            targets_for_instance(noun, instance, state)
        }
        CurrentCell::FactCell { cell_name } => {
            targets_for_fact_cell(cell_name, state)
        }
        CurrentCell::ComponentInstance { component_id } => {
            targets_for_component_instance(component_id, state)
        }
    };

    // Stable, deterministic ordering. The Slint side keys clicks by
    // index; the kernel must emit the same order on every redraw.
    out.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.label.cmp(&b.label)));
    // Dedupe identical (target, kind) pairs that two different walks
    // produce (e.g. a noun that shows up via both role + back-ref).
    out.dedup_by(|a, b| a.target == b.target && a.kind == b.kind);
    out
}

// ── Per-cell-type derivation ───────────────────────────────────────

/// Root navigation: every Noun the system knows about. Mirrors
/// `cell_renderer::project_root` in shape; one navigation target per
/// noun. The kind is `Instance` because Root → Noun is a "drill into
/// instances" navigation in spirit (every Noun cell IS the index of
/// its instances).
fn targets_for_root(state: &Object) -> Vec<NavigationTarget> {
    discover_nouns(state)
        .into_iter()
        .map(|n| NavigationTarget::new(CurrentCell::Noun { noun: n }, NavigationKind::Instance))
        .collect()
}

/// Noun navigation: every instance the noun has + every fact-type
/// (cell of the form `<Noun>_has_<Attr>`) the noun participates in.
/// The Role targets are the FT cells; the Instance targets are the
/// drill-down per individual.
fn targets_for_noun(noun: &str, state: &Object) -> Vec<NavigationTarget> {
    let mut out: Vec<NavigationTarget> = Vec::new();

    // Instances: per-instance drill-downs.
    for instance in instances_of(noun, state) {
        out.push(NavigationTarget::new(
            CurrentCell::Instance {
                noun: noun.to_string(),
                instance,
            },
            NavigationKind::Instance,
        ));
    }

    // Fact types this noun is a role in. The "owned" fact types
    // (`<Noun>_has_…`) are the most natural; we also pick up any cell
    // whose facts contain a binding keyed by `Noun` (back-reference
    // direction).
    for cell_name in cells_mentioning_role(noun, state) {
        out.push(NavigationTarget::new(
            CurrentCell::FactCell { cell_name },
            NavigationKind::Role,
        ));
    }

    out
}

/// Instance navigation: every fact cell this instance appears in
/// (front + back references) + a `Type` link back to the parent Noun.
fn targets_for_instance(
    noun: &str,
    instance: &str,
    state: &Object,
) -> Vec<NavigationTarget> {
    let mut out: Vec<NavigationTarget> = Vec::new();

    // Type — back to the parent Noun.
    out.push(NavigationTarget::new(
        CurrentCell::Noun { noun: noun.to_string() },
        NavigationKind::Type,
    ));

    // Every cell that mentions this instance (front-references via
    // the noun's role + back-references when the instance value
    // appears in any role of any fact). The walk is one pass over
    // every cell; the dedupe in `compute_navigation_targets` collapses
    // duplicates.
    for cell_name in cells_referencing_instance(noun, instance, state) {
        out.push(NavigationTarget::new(
            CurrentCell::FactCell { cell_name },
            NavigationKind::Role,
        ));
    }

    // State Machine surface: if any SM fact references this instance
    // (either as the SM itself or as the resource the SM is for), add
    // a navigation target to the SM cell so the user can drill in to
    // see the current state + next-states. The actual transition
    // invocation lives in #514; #512 only wires the navigation.
    if instance_has_state_machine(noun, instance, state) {
        out.push(NavigationTarget::new(
            CurrentCell::FactCell {
                cell_name: "StateMachine_has_currentlyInStatus".to_string(),
            },
            NavigationKind::StateMachine,
        ));
    }

    out
}

/// Fact-cell navigation: every distinct instance the cell mentions
/// (one target per (noun, instance) pair extracted from the cell's
/// facts) + every distinct role-noun the cell uses.
///
/// This unifies the "navigate to instances" and "navigate to roles"
/// rules from the module docstring into one walk over the cell's
/// facts. Derivation antecedents / consequents fall out naturally:
/// when a cell B's facts contain a value that is also an instance in
/// cell A, the user can drill from B to A via that instance — which
/// is exactly the Antecedent/Consequent navigation under a different
/// name.
fn targets_for_fact_cell(cell_name: &str, state: &Object) -> Vec<NavigationTarget> {
    let mut out: Vec<NavigationTarget> = Vec::new();
    let cell = ast::fetch_or_phi(cell_name, state);
    let Some(facts) = cell.as_seq() else {
        return out;
    };

    // Roles: every distinct (role, value) pair. The role names are
    // also the noun names by AREST convention; navigate to the Noun
    // cell for each role. Avoid emitting both Role and Instance
    // targets for the same value — Role drills to the Noun's index
    // (every instance), Instance drills directly to one row.
    let mut roles: BTreeSet<String> = BTreeSet::new();
    let mut instances: BTreeSet<(String, String)> = BTreeSet::new();
    for fact in facts {
        let Some(pairs) = fact.as_seq() else { continue };
        for pair in pairs {
            let Some(items) = pair.as_seq() else { continue };
            if items.len() != 2 {
                continue;
            }
            let Some(role) = items[0].as_atom() else { continue };
            let Some(value) = items[1].as_atom() else { continue };
            roles.insert(role.to_string());
            instances.insert((role.to_string(), value.to_string()));
        }
    }

    for role in &roles {
        out.push(NavigationTarget::new(
            CurrentCell::Noun { noun: role.clone() },
            NavigationKind::Role,
        ));
    }

    for (noun, instance) in &instances {
        out.push(NavigationTarget::new(
            CurrentCell::Instance {
                noun: noun.clone(),
                instance: instance.clone(),
            },
            NavigationKind::Instance,
        ));
    }

    out
}

/// Component-instance navigation: surfaces the `Component` Noun (so
/// the user can see every other component) and any `ImplementationBinding`
/// fact-cell entries that mention this component. Mirrors the back-
/// reference walk for Instance but scoped to the component_binding
/// surface.
fn targets_for_component_instance(
    component_id: &str,
    state: &Object,
) -> Vec<NavigationTarget> {
    let mut out: Vec<NavigationTarget> = Vec::new();

    // Component noun index — list every Component.
    out.push(NavigationTarget::new(
        CurrentCell::Noun { noun: "Component".to_string() },
        NavigationKind::Type,
    ));

    // Component-related cells that mention this component_id (front +
    // back). Same shape as Instance navigation.
    let comp_root = component_id.split('.').next().unwrap_or(component_id);
    for cell_name in cells_referencing_instance("Component", comp_root, state) {
        out.push(NavigationTarget::new(
            CurrentCell::FactCell { cell_name },
            NavigationKind::Role,
        ));
    }

    out
}

// ── Cell-graph helpers (pure over &Object) ────────────────────────

/// Sorted, deduplicated set of Noun names — the leading token before
/// `_has_` in each cell name. Filters cell names containing `:` (def
/// shards) and the synthetic `D` cell.
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

/// Every cell that mentions `noun` as a role. Includes the obvious
/// `<Noun>_has_…` cells AND the back-reference shape (any cell whose
/// facts have a `Noun` binding key). Filtered to skip `:`-shards and
/// the synthetic `D` cell.
fn cells_mentioning_role(noun: &str, state: &Object) -> Vec<String> {
    let prefix = format!("{noun}_has_");
    let mut set: BTreeSet<String> = BTreeSet::new();
    for (cell_name, contents) in ast::cells_iter(state) {
        if cell_name.contains(':') {
            continue;
        }
        // Front-reference: cell named after this noun.
        if cell_name.starts_with(&prefix[..]) {
            set.insert(cell_name.to_string());
            continue;
        }
        // Back-reference: noun appears as a role in any fact.
        let Some(facts) = contents.as_seq() else { continue };
        let mentions = facts.iter().any(|fact| {
            let Some(pairs) = fact.as_seq() else { return false };
            pairs.iter().any(|pair| {
                let Some(items) = pair.as_seq() else { return false };
                items.len() == 2 && items[0].as_atom() == Some(noun)
            })
        });
        if mentions {
            set.insert(cell_name.to_string());
        }
    }
    set.into_iter().collect()
}

/// Every cell that references the (noun, instance) pair — either via
/// the front-reference `<Noun>_has_…` shape with `binding(noun) ==
/// instance`, or via the back-reference shape where any role's value
/// equals `instance`. Filtered to skip `:`-shards.
fn cells_referencing_instance(
    noun: &str,
    instance: &str,
    state: &Object,
) -> Vec<String> {
    let prefix = format!("{noun}_has_");
    let mut set: BTreeSet<String> = BTreeSet::new();
    for (cell_name, contents) in ast::cells_iter(state) {
        if cell_name.contains(':') {
            continue;
        }
        let Some(facts) = contents.as_seq() else { continue };
        let is_front = cell_name.starts_with(&prefix[..]);
        let mentions = facts.iter().any(|fact| {
            if is_front && ast::binding(fact, noun) == Some(instance) {
                return true;
            }
            // Back-reference: the instance value appears in any role
            // position (regardless of the role's name).
            let Some(pairs) = fact.as_seq() else { return false };
            pairs.iter().any(|pair| {
                let Some(items) = pair.as_seq() else { return false };
                items.len() == 2 && items[1].as_atom() == Some(instance)
            })
        });
        if mentions {
            set.insert(cell_name.to_string());
        }
    }
    set.into_iter().collect()
}

/// True iff any `StateMachine_has_currentlyInStatus` fact references
/// the (noun, instance) pair — either as the State Machine itself or
/// as the `forResource` value. Mirrors the SM lookup pattern used in
/// `arest::command::extract_sm_status`.
fn instance_has_state_machine(noun: &str, instance: &str, state: &Object) -> bool {
    let cell = ast::fetch_or_phi("StateMachine_has_currentlyInStatus", state);
    let Some(facts) = cell.as_seq() else {
        return false;
    };
    facts.iter().any(|fact| {
        ast::binding_matches(fact, "State Machine", instance)
            || ast::binding_matches(fact, "forResource", instance)
            || ast::binding_matches(fact, noun, instance)
    })
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use arest::ast::{cell_push, fact_from_pairs};

    /// Synthetic cell graph: two Files (one with a MimeType binding),
    /// one Tag with a back-reference to a File, one StateMachine fact.
    /// Mirrors the fixture shape `unified_repl::tests::synth_state` uses
    /// so the navigation expectations match what the live REPL sees.
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
        cell_push(
            "StateMachine_has_currentlyInStatus",
            fact_from_pairs(&[
                ("State Machine", "f1"),
                ("currentlyInStatus", "draft"),
                ("forResource", "f1"),
            ]),
            &s,
        )
    }

    // ── NavigationKind label prefixes ──────────────────────────────

    #[test]
    fn navigation_kind_label_prefixes_distinct() {
        // Every kind has a unique short prefix so the user can tell
        // them apart in the affordance row.
        let kinds = [
            NavigationKind::Role,
            NavigationKind::Type,
            NavigationKind::Instance,
            NavigationKind::Antecedent,
            NavigationKind::Consequent,
            NavigationKind::StateMachine,
        ];
        let mut prefixes: Vec<&str> = kinds.iter().map(|k| k.label_prefix()).collect();
        prefixes.sort();
        prefixes.dedup();
        assert_eq!(prefixes.len(), kinds.len(), "prefixes must be distinct");
    }

    #[test]
    fn navigation_target_label_combines_kind_and_target() {
        let t = NavigationTarget::new(
            CurrentCell::Noun { noun: "File".into() },
            NavigationKind::Role,
        );
        assert_eq!(t.label, "[role] File");
    }

    // ── Root navigation ────────────────────────────────────────────

    #[test]
    fn root_navigation_lists_one_target_per_noun() {
        let state = synth_state();
        let targets = compute_navigation_targets(&CurrentCell::Root, &state);
        // File + Tag + StateMachine — three nouns visible in synth.
        let labels: Vec<&str> = targets.iter().map(|t| t.label.as_str()).collect();
        assert!(labels.iter().any(|l| *l == "[instance] File"), "missing File: {labels:?}");
        assert!(labels.iter().any(|l| *l == "[instance] Tag"), "missing Tag: {labels:?}");
        assert!(
            labels.iter().any(|l| *l == "[instance] StateMachine"),
            "missing StateMachine: {labels:?}"
        );
    }

    #[test]
    fn root_navigation_targets_are_noun_cells() {
        let state = synth_state();
        let targets = compute_navigation_targets(&CurrentCell::Root, &state);
        for t in &targets {
            assert!(
                matches!(t.target, CurrentCell::Noun { .. }),
                "Root → Noun only, got {:?}",
                t.target
            );
            assert_eq!(t.kind, NavigationKind::Instance);
        }
    }

    // ── Noun navigation ────────────────────────────────────────────

    #[test]
    fn noun_navigation_includes_each_instance() {
        let state = synth_state();
        let targets = compute_navigation_targets(
            &CurrentCell::Noun { noun: "File".into() },
            &state,
        );
        let instance_labels: Vec<&str> = targets
            .iter()
            .filter(|t| t.kind == NavigationKind::Instance)
            .map(|t| t.label.as_str())
            .collect();
        assert!(
            instance_labels.contains(&"[instance] File/f1"),
            "missing f1: {instance_labels:?}"
        );
        assert!(
            instance_labels.contains(&"[instance] File/f2"),
            "missing f2: {instance_labels:?}"
        );
    }

    #[test]
    fn noun_navigation_includes_owned_fact_types() {
        let state = synth_state();
        let targets = compute_navigation_targets(
            &CurrentCell::Noun { noun: "File".into() },
            &state,
        );
        let role_labels: Vec<&str> = targets
            .iter()
            .filter(|t| t.kind == NavigationKind::Role)
            .map(|t| t.label.as_str())
            .collect();
        assert!(
            role_labels.iter().any(|l| l.contains("File_has_Name")),
            "missing File_has_Name: {role_labels:?}"
        );
        assert!(
            role_labels.iter().any(|l| l.contains("File_has_MimeType")),
            "missing File_has_MimeType: {role_labels:?}"
        );
    }

    #[test]
    fn noun_navigation_includes_back_reference_fact_types() {
        // Tag_is_on_File mentions File as a role (back-reference). The
        // File noun should pick that up alongside its own File_has_…
        // cells.
        let state = synth_state();
        let targets = compute_navigation_targets(
            &CurrentCell::Noun { noun: "File".into() },
            &state,
        );
        let role_labels: Vec<&str> = targets
            .iter()
            .filter(|t| t.kind == NavigationKind::Role)
            .map(|t| t.label.as_str())
            .collect();
        assert!(
            role_labels.iter().any(|l| l.contains("Tag_is_on_File")),
            "missing back-ref Tag_is_on_File: {role_labels:?}"
        );
    }

    // ── Instance navigation ────────────────────────────────────────

    #[test]
    fn instance_navigation_includes_type_link() {
        let state = synth_state();
        let targets = compute_navigation_targets(
            &CurrentCell::Instance {
                noun: "File".into(),
                instance: "f1".into(),
            },
            &state,
        );
        let type_targets: Vec<&NavigationTarget> = targets
            .iter()
            .filter(|t| t.kind == NavigationKind::Type)
            .collect();
        assert_eq!(type_targets.len(), 1, "one Type link expected: {targets:?}");
        assert_eq!(
            type_targets[0].target,
            CurrentCell::Noun { noun: "File".into() }
        );
    }

    #[test]
    fn instance_navigation_includes_facts_referencing_instance() {
        let state = synth_state();
        let targets = compute_navigation_targets(
            &CurrentCell::Instance {
                noun: "File".into(),
                instance: "f1".into(),
            },
            &state,
        );
        let role_labels: Vec<&str> = targets
            .iter()
            .filter(|t| t.kind == NavigationKind::Role)
            .map(|t| t.label.as_str())
            .collect();
        assert!(
            role_labels.iter().any(|l| l.contains("File_has_Name")),
            "missing File_has_Name: {role_labels:?}"
        );
        assert!(
            role_labels.iter().any(|l| l.contains("File_has_MimeType")),
            "missing File_has_MimeType: {role_labels:?}"
        );
        assert!(
            role_labels.iter().any(|l| l.contains("Tag_is_on_File")),
            "missing back-ref Tag_is_on_File: {role_labels:?}"
        );
    }

    #[test]
    fn instance_navigation_excludes_facts_for_other_instances() {
        // f2 is a File but never appears in File_has_MimeType (only f1
        // does). The MimeType cell should still appear in f2's
        // navigation list because f2 IS a File (the front-reference
        // owns the cell), but the Tag_is_on_File back-ref should NOT
        // because it only mentions f1.
        let state = synth_state();
        let targets = compute_navigation_targets(
            &CurrentCell::Instance {
                noun: "File".into(),
                instance: "f2".into(),
            },
            &state,
        );
        let role_labels: Vec<&str> = targets
            .iter()
            .filter(|t| t.kind == NavigationKind::Role)
            .map(|t| t.label.as_str())
            .collect();
        assert!(
            role_labels.iter().any(|l| l.contains("File_has_Name")),
            "f2 has a Name binding so File_has_Name should appear: {role_labels:?}"
        );
        // Tag_is_on_File only references f1, not f2 — must NOT appear.
        assert!(
            !role_labels.iter().any(|l| l.contains("Tag_is_on_File")),
            "Tag_is_on_File leaked into f2's nav: {role_labels:?}"
        );
    }

    #[test]
    fn instance_navigation_surfaces_state_machine_when_present() {
        let state = synth_state();
        let targets = compute_navigation_targets(
            &CurrentCell::Instance {
                noun: "File".into(),
                instance: "f1".into(),
            },
            &state,
        );
        let sm_targets: Vec<&NavigationTarget> = targets
            .iter()
            .filter(|t| t.kind == NavigationKind::StateMachine)
            .collect();
        assert_eq!(
            sm_targets.len(),
            1,
            "f1 has a StateMachine fact, expected exactly one SM target: {targets:?}"
        );
    }

    #[test]
    fn instance_navigation_omits_state_machine_when_absent() {
        let state = synth_state();
        // f2 is a File but no StateMachine fact references it.
        let targets = compute_navigation_targets(
            &CurrentCell::Instance {
                noun: "File".into(),
                instance: "f2".into(),
            },
            &state,
        );
        let sm_targets: Vec<&NavigationTarget> = targets
            .iter()
            .filter(|t| t.kind == NavigationKind::StateMachine)
            .collect();
        assert!(sm_targets.is_empty(), "no SM expected for f2: {sm_targets:?}");
    }

    // ── FactCell navigation ───────────────────────────────────────

    #[test]
    fn fact_cell_navigation_lists_distinct_role_nouns() {
        let state = synth_state();
        let targets = compute_navigation_targets(
            &CurrentCell::FactCell {
                cell_name: "Tag_is_on_File".into(),
            },
            &state,
        );
        let roles: BTreeSet<String> = targets
            .iter()
            .filter(|t| t.kind == NavigationKind::Role)
            .filter_map(|t| match &t.target {
                CurrentCell::Noun { noun } => Some(noun.clone()),
                _ => None,
            })
            .collect();
        assert!(roles.contains("Tag"), "missing Tag: {roles:?}");
        assert!(roles.contains("File"), "missing File: {roles:?}");
    }

    #[test]
    fn fact_cell_navigation_lists_distinct_instances() {
        let state = synth_state();
        let targets = compute_navigation_targets(
            &CurrentCell::FactCell {
                cell_name: "Tag_is_on_File".into(),
            },
            &state,
        );
        // Tag_is_on_File has one fact (Tag=t1, File=f1) — two
        // (noun, instance) pairs.
        let instances: BTreeSet<(String, String)> = targets
            .iter()
            .filter(|t| t.kind == NavigationKind::Instance)
            .filter_map(|t| match &t.target {
                CurrentCell::Instance { noun, instance } => {
                    Some((noun.clone(), instance.clone()))
                }
                _ => None,
            })
            .collect();
        assert!(
            instances.contains(&("Tag".to_string(), "t1".to_string())),
            "missing Tag/t1: {instances:?}"
        );
        assert!(
            instances.contains(&("File".to_string(), "f1".to_string())),
            "missing File/f1: {instances:?}"
        );
    }

    #[test]
    fn fact_cell_navigation_empty_for_missing_cell() {
        let state = synth_state();
        let targets = compute_navigation_targets(
            &CurrentCell::FactCell {
                cell_name: "Bogus_cell".into(),
            },
            &state,
        );
        assert!(targets.is_empty(), "missing cell → no targets: {targets:?}");
    }

    // ── ComponentInstance navigation ──────────────────────────────

    #[test]
    fn component_instance_navigation_includes_component_noun_link() {
        let state = synth_state();
        let targets = compute_navigation_targets(
            &CurrentCell::ComponentInstance {
                component_id: "list.slint".into(),
            },
            &state,
        );
        let type_targets: Vec<&NavigationTarget> = targets
            .iter()
            .filter(|t| t.kind == NavigationKind::Type)
            .collect();
        assert_eq!(type_targets.len(), 1);
        assert_eq!(
            type_targets[0].target,
            CurrentCell::Noun { noun: "Component".into() }
        );
    }

    // ── Determinism / ordering ────────────────────────────────────

    #[test]
    fn navigation_targets_order_is_stable() {
        // Two successive calls must produce identical orderings — the
        // Slint side keys clicks by index into this vector.
        let state = synth_state();
        let a = compute_navigation_targets(&CurrentCell::Root, &state);
        let b = compute_navigation_targets(&CurrentCell::Root, &state);
        assert_eq!(a, b);
    }

    #[test]
    fn navigation_targets_dedupe_identical_kind_target_pairs() {
        // A noun that legitimately appears twice in the walk (front +
        // back ref to the same cell) should still only be listed once
        // per (kind, target) pair.
        let state = synth_state();
        let targets = compute_navigation_targets(
            &CurrentCell::Noun { noun: "File".into() },
            &state,
        );
        let mut seen: BTreeSet<(NavigationKind, String)> = BTreeSet::new();
        for t in &targets {
            let key = (t.kind, format!("{:?}", t.target));
            assert!(
                seen.insert(key),
                "duplicate (kind, target) pair: {} -> {:?}",
                t.label,
                t.target
            );
        }
    }
}
