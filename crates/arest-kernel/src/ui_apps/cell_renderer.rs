// crates/arest-kernel/src/ui_apps/cell_renderer.rs
//
// Cell-as-screen rendering (#511, EPIC #496).
//
// SSSS-2's #510 (commit f6c1808) merged the prior HateoasBrowser +
// Repl modules into a single Slint surface. That commit was a pure
// structural merge — the right pane still renders REPL scrollback as
// raw text and the left pane still renders raw `<Noun>_has_<Attr>`
// bindings as `key = value` lines.
//
// This module supplies the next layer per the user's vision:
//
//   "the repl should be easy to use. It should render the system as
//    the current screen of an app so that the user is never lost. It
//    provides SYSTEM calls, but it should be easy to navigate the
//    system using its own principles"
//
// In other words: every screen IS a cell of the system. The current
// screen is some cell (or a derived view of one); the rendering rule
// looks up the best Component for that cell type and pushes the cell's
// data through it. When no specialised Component is registered, a
// generic key-value list view is the fallback.
//
// # Cell-type taxonomy (UnifiedReplState::current_cell)
//
// The unified REPL holds one `CurrentCell` value at a time:
//
//   * `CurrentCell::Root` — initial landing. Lists the system's nouns
//     (the same list HATEOAS already shows). Component intent: `"list"`.
//   * `CurrentCell::Noun { noun }` — a fact-type cell. Lists every
//     instance for that noun. Component intent: `"list"`.
//   * `CurrentCell::Instance { noun, instance }` — one instance row.
//     Renders the instance's bindings as a typed surface. Component
//     intent: `"card"` (instance detail looks like a card / form).
//   * `CurrentCell::FactCell { cell_name }` — a derivation / SYSTEM
//     cell named directly (e.g. `Component_has_ComponentRole`). Renders
//     all the cell's facts as a list. Component intent: `"list"`.
//   * `CurrentCell::ComponentInstance { component_id }` — a live
//     Component instance from #491's binder registry. Component
//     intent: matches the Component's role.
//
// Each cell-type is dispatched through `select_component` (a thin,
// kernel-internal port of `arest::command::select_component` — the
// host crate's `command` module is `cfg(not(feature = "no_std"))`
// gated, so this module re-implements the ranking here). Result:
// the Component handle map (PPPP's #491 `lookup_component`) tells us
// which live widget to drive, and the Slint surface picks up the
// `current-component` property the Rust side sets.
//
// # Surface contract (UnifiedRepl.slint)
//
// The Slint right pane gains a typed-surface area driven by these
// new properties:
//
//   * `current-cell-name: string`     — human-friendly title
//   * `current-cell-component: string` — selected Component role
//   * `current-cell-toolkit: string`  — selected toolkit slug
//   * `current-cell-symbol: string`   — toolkit symbol (e.g. "List")
//   * `current-cell-fields: [string]` — generic fallback render
//
// The Rust side computes all five via `render_current_cell` and
// pushes them on every redraw. The Slint side displays them via a
// labelled card. When no Component is matched, the fallback fields
// list is what the user sees — a typed surface IS still rendered, but
// with the generic-key-value layout rather than a specialised widget.
//
// # Why not a full widget instantiation?
//
// On the foundation slice (today's commit), the binder dispatch to
// real Qt/GTK/Web widgets is a no-op stub (PPPP's #491 forwards the
// call but the underlying library never loaded). The Slint binder
// IS live, but every Slint Window is fixed-shape — we can't swap a
// `Card` widget into the right pane at runtime without recompiling.
// So the typed-surface area renders the *selection result* + the
// *cell's projected data*; future commits can swap the area to a
// dynamically-loaded Slint sub-component once the loader supports it.
//
// This is consistent with #511's task scope ("rendering rule") vs.
// #512+ (navigation, palette). The Component selection happens here;
// the dynamic instantiation lands later.

#![allow(dead_code)]

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use arest::ast::{self, Object};

use crate::ui_apps::navigation::{self, NavigationTarget};

// ── Cell-type discriminator ────────────────────────────────────────

/// The "current cell" the unified REPL is rendering. Every screen IS
/// one of these. Switching between variants is the primary user
/// action under the cell-as-screen model.
///
/// Note this is a strict subset of the full cell space: cells named
/// directly via `FactCell` cover everything `cells_iter` exposes; the
/// other variants are sugar for the most common navigation paths
/// (Root / Noun / Instance) so the UI can render a stable breadcrumb
/// trail without losing the cell-as-screen invariant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CurrentCell {
    /// Initial landing — lists every Noun the system knows about.
    /// Conceptually the "Resources" cell from HATEOAS.
    Root,
    /// A fact-type — every cell of the form `<Noun>_has_<Attr>`
    /// belongs to this Noun. Listing this cell shows every instance.
    Noun { noun: String },
    /// One concrete instance — the "card" view rendering its bindings
    /// + back-references.
    Instance { noun: String, instance: String },
    /// A directly-named SYSTEM cell — derivation cells, kernel-internal
    /// cells, anything `cells_iter` returns. Renders raw facts as a
    /// list of key-value lines.
    FactCell { cell_name: String },
    /// A registered Component instance from #491's binder registry.
    /// The Component's role determines which Slint Component the
    /// selection rules surface.
    ComponentInstance { component_id: String },
}

impl CurrentCell {
    /// Display name for the breadcrumb / typed-surface header.
    pub fn label(&self) -> String {
        match self {
            CurrentCell::Root => "Resources".to_string(),
            CurrentCell::Noun { noun } => noun.clone(),
            CurrentCell::Instance { noun, instance } => format!("{noun}/{instance}"),
            CurrentCell::FactCell { cell_name } => cell_name.clone(),
            CurrentCell::ComponentInstance { component_id } => component_id.clone(),
        }
    }

    /// Component-selection intent for this cell. Mirrors the
    /// canonical Component Roles in `readings/ui/components.md`.
    /// Different cell types want different surfaces:
    ///   * Root / Noun / FactCell — `"list"` (each row is a clickable
    ///     navigation target).
    ///   * Instance — `"card"` (a form-like detail view).
    ///   * ComponentInstance — the Component's own role (echoes back
    ///     to its registered surface).
    pub fn intent(&self) -> &str {
        match self {
            CurrentCell::Root | CurrentCell::Noun { .. } | CurrentCell::FactCell { .. } => "list",
            CurrentCell::Instance { .. } => "card",
            CurrentCell::ComponentInstance { .. } => "card",
        }
    }
}

// ── Selection result for the typed-surface header ─────────────────

/// Result of dispatching a cell to the Component registry. Mirrors
/// `arest::command::SelectedComponent` but kernel-internal (the host
/// type lives behind `cfg(not(feature = "no_std"))`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SelectedComponent {
    /// Component name (e.g. `"list"`, `"card"`, `"button"`).
    pub component: String,
    /// Toolkit slug — typically `"slint"` on the kernel side because
    /// kernel-resident bindings always win the tie-break.
    pub toolkit: String,
    /// Toolkit-side symbol (e.g. `"List"`, `"Card"`). What Slint
    /// codegen emitted at build time.
    pub symbol: String,
    /// Score under the (currently empty) constraints. Higher is
    /// better; ties broken deterministically.
    pub score: u32,
}

// ── Component selection (kernel port of arest::command::select_component) ──

/// Walk the registered Component_* facts and return the best
/// (component, toolkit, symbol) triple for `intent`. Empty when no
/// Component matches — the caller falls back to the generic
/// key-value list view.
///
/// Mirrors `arest::command::select_component` but with kernel-side
/// cell names (`Component_has_ComponentRole` /
/// `Component_is_implemented_by_Toolkit_at_ToolkitSymbol`) and the
/// kernel-side trait cells. Constraints are intentionally empty in
/// this slice — #511's scope is the dispatch path; #512+ wires
/// per-screen constraint sets (touch / a11y / theme).
///
/// The tie-breaker rule from #492 — Slint always wins under equal
/// scores — survives intact: every Slint binding gets a +1 floor
/// point so it ranks above other toolkits when no scoring criterion
/// distinguishes them.
pub fn select_component_for(intent: &str, state: &Object) -> Option<SelectedComponent> {
    let intent_lc = intent.trim().to_lowercase();

    // (Component name, Component Role) pairs.
    let role_cell = ast::fetch_or_phi("Component_has_ComponentRole", state);
    let candidates: Vec<(String, String)> = role_cell
        .as_seq()
        .map(|facts| {
            facts
                .iter()
                .filter_map(|f| {
                    let comp = ast::binding(f, "Component")?.to_string();
                    let role = ast::binding(f, "ComponentRole")?.to_string();
                    let role_norm = role.replace('-', " ").to_lowercase();
                    let intent_norm = intent_lc.replace('-', " ");
                    let matches = intent_lc.is_empty()
                        || intent_norm.contains(&role_norm)
                        || role_norm.contains(&intent_norm);
                    matches.then_some((comp, role))
                })
                .collect()
        })
        .unwrap_or_default();

    if candidates.is_empty() {
        return None;
    }

    // Enumerate ImplementationBindings for each candidate Component.
    let bind_cell = ast::fetch_or_phi(
        "Component_is_implemented_by_Toolkit_at_ToolkitSymbol",
        state,
    );
    let mut results: Vec<SelectedComponent> = candidates
        .iter()
        .flat_map(|(comp, _role)| {
            bind_cell
                .as_seq()
                .unwrap_or(&[])
                .iter()
                .filter_map(move |f| {
                    if !ast::binding_matches(f, "Component", comp) {
                        return None;
                    }
                    let toolkit = ast::binding(f, "Toolkit")?.to_string();
                    let symbol = ast::binding(f, "ToolkitSymbol")?.to_string();
                    let score = score_binding(comp, &toolkit, state);
                    Some(SelectedComponent {
                        component: comp.clone(),
                        toolkit,
                        symbol,
                        score,
                    })
                })
        })
        .collect();

    // Sort: score desc, then (component, toolkit) asc for reproducibility.
    results.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.component.cmp(&b.component))
            .then_with(|| a.toolkit.cmp(&b.toolkit))
    });
    results.into_iter().next()
}

/// Score one (Component × Toolkit) pair. Mirrors the unconstrained
/// fragment of `arest::command::score_binding`:
///   * +1 unconditional for any Slint binding (the tie-break).
///   * +1 for Slint bindings that carry the `kernel_native` trait
///     (kernel-resident wins when the fact stream advertises it).
///
/// More elaborate constraint axes (touch / a11y / theme) are deferred
/// to #512+; this slice's scope is the rendering rule, not the
/// per-screen constraint set.
fn score_binding(component: &str, toolkit: &str, state: &Object) -> u32 {
    let mut score = 0u32;
    if toolkit == "slint" {
        score += 1;
    }
    let binding_id = format!("{component}.{toolkit}");
    if toolkit == "slint" && binding_has_trait(&binding_id, "kernel_native", state) {
        score += 1;
    }
    score
}

/// True iff the named ImplementationBinding carries the trait `name`.
fn binding_has_trait(binding_id: &str, name: &str, state: &Object) -> bool {
    let cell = ast::fetch_or_phi("ImplementationBinding_has_Trait", state);
    cell.as_seq()
        .map(|facts| {
            facts.iter().any(|f| {
                ast::binding(f, "ImplementationBinding") == Some(binding_id)
                    && ast::binding(f, "ComponentTrait") == Some(name)
            })
        })
        .unwrap_or(false)
}

// ── Cell projection (cell type → field list for typed surface) ────

/// Project the current cell into a list of human-readable lines that
/// the typed-surface area renders. Same shape `detail_lines_for`
/// produced for HATEOAS, generalised across every CurrentCell variant.
///
/// Pure function over `&Object` — the `with_state` caller can drop
/// the read lock the moment this returns.
pub fn project_cell_fields(cell: &CurrentCell, state: &Object) -> Vec<String> {
    match cell {
        CurrentCell::Root => project_root(state),
        CurrentCell::Noun { noun } => project_noun(noun, state),
        CurrentCell::Instance { noun, instance } => project_instance(noun, instance, state),
        CurrentCell::FactCell { cell_name } => project_fact_cell(cell_name, state),
        CurrentCell::ComponentInstance { component_id } => {
            project_component_instance(component_id, state)
        }
    }
}

/// Root view: one line per discoverable Noun. Mirrors
/// `discover_nouns` from `unified_repl.rs` — the typed-surface area
/// doubles as the resource picker when the user lands on Root.
fn project_root(state: &Object) -> Vec<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    for (cell_name, _) in ast::cells_iter(state) {
        if cell_name.contains(':') {
            continue;
        }
        if let Some((noun, _)) = cell_name.split_once("_has_") {
            if noun != "D" {
                set.insert(noun.to_string());
            }
        }
    }
    if set.is_empty() {
        return vec!["(no resources — system is empty)".to_string()];
    }
    set.into_iter().map(|n| format!("- {n}")).collect()
}

/// Noun view: one line per instance for `noun`. Mirrors
/// `instances_of` from `unified_repl.rs`.
fn project_noun(noun: &str, state: &Object) -> Vec<String> {
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
    if set.is_empty() {
        return vec![format!("(no instances of {noun})")];
    }
    set.into_iter().map(|i| format!("- {i}")).collect()
}

/// Instance view: every binding for the (noun, instance) pair.
/// Same shape `unified_repl::detail_lines_for` produces but stripped
/// of the back-reference section — the typed-surface area is a "card"
/// view, back-references live in #512's navigation actions.
fn project_instance(noun: &str, instance: &str, state: &Object) -> Vec<String> {
    let prefix = format!("{noun}_has_");
    let mut out: Vec<String> = Vec::new();
    for (cell_name, contents) in ast::cells_iter(state) {
        let Some(attr) = cell_name.strip_prefix(&prefix[..]) else {
            continue;
        };
        let Some(facts) = contents.as_seq() else {
            continue;
        };
        for fact in facts {
            if ast::binding(fact, noun) != Some(instance) {
                continue;
            }
            if let Some(value) = ast::binding(fact, attr) {
                out.push(format!("{attr}: {value}"));
            }
        }
    }
    out.sort();
    if out.is_empty() {
        out.push("(no bindings)".to_string());
    }
    out
}

/// Fact-cell view: one line per fact in the named cell. Each fact's
/// roles are rendered as `role=value, role=value, …`. Used by the
/// FactCell variant for derivation cells / SYSTEM cells the user
/// drilled into directly (e.g. `Component_has_Property`).
fn project_fact_cell(cell_name: &str, state: &Object) -> Vec<String> {
    let cell = ast::fetch_or_phi(cell_name, state);
    let Some(facts) = cell.as_seq() else {
        return vec![format!("(cell {cell_name} not present)")];
    };
    if facts.is_empty() {
        return vec![format!("(cell {cell_name} is empty)")];
    }
    facts
        .iter()
        .map(|fact| {
            let Some(pairs) = fact.as_seq() else {
                return "?".to_string();
            };
            let parts: Vec<String> = pairs
                .iter()
                .filter_map(|p| {
                    let items = p.as_seq()?;
                    if items.len() != 2 {
                        return None;
                    }
                    let k = items[0].as_atom()?;
                    let v = items[1].as_atom()?;
                    Some(format!("{k}={v}"))
                })
                .collect();
            parts.join(", ")
        })
        .collect()
}

/// Component-instance view: surface the registered handle (if any) +
/// the Component's declared properties. Useful for poking live
/// widgets from the REPL.
fn project_component_instance(component_id: &str, state: &Object) -> Vec<String> {
    let mut out: Vec<String> = vec![format!("Component: {component_id}")];

    // Live registration record (#491 binder map).
    if let Some(rec) = crate::component_binding::lookup_component(component_id) {
        out.push(format!("toolkit: {}", rec.toolkit));
        out.push(format!("handle: {:?}", rec.handle));
    } else {
        out.push("(not registered with binder)".to_string());
    }

    // Declared properties from the Component_has_Property_of_…
    // cell that the registry seeds.
    let prop_cell = ast::fetch_or_phi(
        "Component_has_Property_of_PropertyType_with_PropertyDefault",
        state,
    );
    let Some(facts) = prop_cell.as_seq() else {
        return out;
    };
    let comp_root = component_id.split('.').next().unwrap_or(component_id);
    let mut props: Vec<String> = Vec::new();
    for fact in facts {
        if !ast::binding_matches(fact, "Component", comp_root) {
            continue;
        }
        let name = ast::binding(fact, "PropertyName").unwrap_or("?");
        let ty = ast::binding(fact, "PropertyType").unwrap_or("?");
        let default = ast::binding(fact, "PropertyDefault").unwrap_or("");
        props.push(format!("  {name}: {ty} = {default}"));
    }
    if !props.is_empty() {
        out.push("properties:".to_string());
        out.extend(props);
    }
    out
}

// ── Top-level render: cell + selected component + projected fields ─

/// Render a single screen. Bundles the typed-surface header
/// (selected Component) with the projected field list AND the
/// derived navigation catalogue — the Slint side reads each field
/// off `RenderedScreen` and pushes them into the property bag.
///
/// Navigation actions as cells (#512, EPIC #496): the `navigation`
/// vector holds every legal "navigate to X" affordance for this
/// screen, computed by `navigation::compute_navigation_targets`
/// from the FT graph itself. The Slint side renders them as
/// clickable rows in the right pane; clicks fire a callback with
/// the index into this vector.
#[derive(Debug, Clone)]
pub struct RenderedScreen {
    /// Human-friendly cell name (the title of the typed surface).
    pub cell_label: String,
    /// Selected Component for this cell. `None` when no Component
    /// matched the cell's intent — the Slint side renders the
    /// generic key-value fallback.
    pub selected: Option<SelectedComponent>,
    /// Cell projected to a list of human-readable lines. Always
    /// populated; even with a matched Component, the fields list is
    /// shown alongside the typed surface header (transparency: the
    /// user can always see what data drives the surface).
    pub fields: Vec<String>,
    /// Navigation catalogue for this screen — every "navigate to X"
    /// affordance derived from the cell graph. Each entry corresponds
    /// to one clickable row in the right-pane navigation list. The
    /// Slint side keys clicks by index into this vector, so the
    /// ordering is stable across redraws (enforced by
    /// `navigation::compute_navigation_targets`).
    pub navigation: Vec<NavigationTarget>,
}

/// Render the current cell into a `RenderedScreen` ready to push
/// into the Slint property bag. Pure function — the `with_state`
/// caller drops the read lock the moment this returns.
///
/// Bundles three derivations: the Component selection (#511), the
/// field projection (#511), and the navigation-target catalogue
/// (#512). All three read the same `Object` slice; computing them
/// in one call lets the caller hold the read lock once.
pub fn render_current_cell(cell: &CurrentCell, state: &Object) -> RenderedScreen {
    let selected = select_component_for(cell.intent(), state);
    let fields = project_cell_fields(cell, state);
    let navigation = navigation::compute_navigation_targets(cell, state);
    RenderedScreen {
        cell_label: cell.label(),
        selected,
        fields,
        navigation,
    }
}

// ── Tests ─────────────────────────────────────────────────────────
//
// `arest-kernel`'s bin target has `test = false` (Cargo.toml L33),
// so these `#[cfg(test)]` cases are reachable only when the crate is
// re-shaped into a lib for hosted testing. Same gating shape as the
// sibling modules; structurally-present tests document the spec
// for future enabling.

#[cfg(test)]
mod tests {
    use super::*;
    use arest::ast::{cell_push, fact_from_pairs};

    /// Synthetic state with a single Slint Component registered the
    /// same shape `registry::build_slint_component_state` produces.
    fn synth_components() -> Object {
        let s = Object::phi();
        let s = cell_push(
            "Component_has_ComponentRole",
            fact_from_pairs(&[("Component", "list"), ("ComponentRole", "list")]),
            &s,
        );
        let s = cell_push(
            "Component_has_ComponentRole",
            fact_from_pairs(&[("Component", "card"), ("ComponentRole", "card")]),
            &s,
        );
        let s = cell_push(
            "Component_is_implemented_by_Toolkit_at_ToolkitSymbol",
            fact_from_pairs(&[
                ("Component", "list"),
                ("Toolkit", "slint"),
                ("ToolkitSymbol", "List"),
            ]),
            &s,
        );
        let s = cell_push(
            "Component_is_implemented_by_Toolkit_at_ToolkitSymbol",
            fact_from_pairs(&[
                ("Component", "card"),
                ("Toolkit", "slint"),
                ("ToolkitSymbol", "Card"),
            ]),
            &s,
        );
        let s = cell_push(
            "Component_is_implemented_by_Toolkit_at_ToolkitSymbol",
            fact_from_pairs(&[
                ("Component", "card"),
                ("Toolkit", "qt6"),
                ("ToolkitSymbol", "QFrame"),
            ]),
            &s,
        );
        let s = cell_push(
            "ImplementationBinding_has_Trait",
            fact_from_pairs(&[
                ("ImplementationBinding", "list.slint"),
                ("ComponentTrait", "kernel_native"),
            ]),
            &s,
        );
        cell_push(
            "ImplementationBinding_has_Trait",
            fact_from_pairs(&[
                ("ImplementationBinding", "card.slint"),
                ("ComponentTrait", "kernel_native"),
            ]),
            &s,
        )
    }

    fn synth_user_facts() -> Object {
        let s = synth_components();
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
        cell_push(
            "File_has_MimeType",
            fact_from_pairs(&[("File", "f1"), ("MimeType", "text/plain")]),
            &s,
        )
    }

    // ── CurrentCell label / intent round-trip ──────────────────────

    #[test]
    fn current_cell_label_rounds_trip() {
        assert_eq!(CurrentCell::Root.label(), "Resources");
        assert_eq!(
            CurrentCell::Noun { noun: "File".into() }.label(),
            "File"
        );
        assert_eq!(
            CurrentCell::Instance { noun: "File".into(), instance: "f1".into() }.label(),
            "File/f1"
        );
        assert_eq!(
            CurrentCell::FactCell { cell_name: "Component_has_Property".into() }.label(),
            "Component_has_Property"
        );
        assert_eq!(
            CurrentCell::ComponentInstance { component_id: "btn.qt6".into() }.label(),
            "btn.qt6"
        );
    }

    #[test]
    fn current_cell_intent_distinguishes_list_vs_card() {
        assert_eq!(CurrentCell::Root.intent(), "list");
        assert_eq!(CurrentCell::Noun { noun: "X".into() }.intent(), "list");
        assert_eq!(
            CurrentCell::FactCell { cell_name: "X".into() }.intent(),
            "list"
        );
        assert_eq!(
            CurrentCell::Instance { noun: "X".into(), instance: "i".into() }.intent(),
            "card"
        );
        assert_eq!(
            CurrentCell::ComponentInstance { component_id: "x.slint".into() }.intent(),
            "card"
        );
    }

    // ── Component selection ────────────────────────────────────────

    #[test]
    fn select_component_for_list_picks_slint_list() {
        let state = synth_components();
        let r = select_component_for("list", &state).expect("matched");
        assert_eq!(r.component, "list");
        assert_eq!(r.toolkit, "slint");
        assert_eq!(r.symbol, "List");
    }

    #[test]
    fn select_component_for_card_prefers_slint_over_qt() {
        // Both qt6 and slint expose the card Component — tie-breaker
        // gives Slint a +1 floor + +1 for kernel_native trait.
        let state = synth_components();
        let r = select_component_for("card", &state).expect("matched");
        assert_eq!(r.toolkit, "slint", "Slint must outscore Qt under tie-break + kernel_native");
        assert!(r.score >= 2, "expected at least slint floor + kernel_native bonus");
    }

    #[test]
    fn select_component_for_unknown_intent_returns_none() {
        let state = synth_components();
        assert!(select_component_for("holographic", &state).is_none());
    }

    #[test]
    fn select_component_for_empty_intent_matches_anything() {
        let state = synth_components();
        // Empty intent matches every Component; the highest-scoring
        // Slint binding wins.
        let r = select_component_for("", &state).expect("any match");
        assert_eq!(r.toolkit, "slint");
    }

    // ── Cell projection ────────────────────────────────────────────

    #[test]
    fn project_root_lists_user_nouns_and_skips_d() {
        let s = cell_push(
            "D_has_welcome",
            fact_from_pairs(&[("D", "x"), ("welcome", "y")]),
            &synth_user_facts(),
        );
        let lines = project_root(&s);
        assert!(lines.iter().any(|l| l == "- File"), "missing File: {lines:?}");
        assert!(!lines.iter().any(|l| l.contains("- D")), "D leaked: {lines:?}");
    }

    #[test]
    fn project_root_empty_state_says_so() {
        let lines = project_root(&Object::phi());
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("(no resources"));
    }

    #[test]
    fn project_noun_lists_distinct_instances_sorted() {
        let lines = project_noun("File", &synth_user_facts());
        assert_eq!(lines, vec!["- f1".to_string(), "- f2".to_string()]);
    }

    #[test]
    fn project_noun_unknown_says_no_instances() {
        let lines = project_noun("Quux", &synth_user_facts());
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("(no instances of Quux)"));
    }

    #[test]
    fn project_instance_renders_attribute_value_pairs() {
        let lines = project_instance("File", "f1", &synth_user_facts());
        assert!(
            lines.iter().any(|l| l == "Name: alpha.txt"),
            "missing Name: {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l == "MimeType: text/plain"),
            "missing MimeType: {lines:?}"
        );
    }

    #[test]
    fn project_instance_empty_says_no_bindings() {
        let lines = project_instance("File", "ghost", &synth_user_facts());
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("(no bindings)"));
    }

    #[test]
    fn project_fact_cell_renders_each_fact_one_line() {
        let lines = project_fact_cell("File_has_Name", &synth_user_facts());
        // synth has two File_has_Name facts — both render.
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().any(|l| l.contains("Name=alpha.txt")));
        assert!(lines.iter().any(|l| l.contains("Name=beta.txt")));
    }

    #[test]
    fn project_fact_cell_missing_says_so() {
        let lines = project_fact_cell("Bogus_cell", &synth_user_facts());
        assert_eq!(lines.len(), 1);
        // Phi cell is empty, not missing — both branches render a
        // distinguishable message.
        assert!(
            lines[0].contains("Bogus_cell"),
            "expected cell name in stub: {}",
            lines[0]
        );
    }

    // ── Top-level render ──────────────────────────────────────────

    #[test]
    fn render_root_picks_list_component_and_lists_nouns() {
        let state = synth_user_facts();
        let r = render_current_cell(&CurrentCell::Root, &state);
        assert_eq!(r.cell_label, "Resources");
        let sel = r.selected.expect("Root must select a list Component");
        assert_eq!(sel.component, "list");
        assert!(r.fields.iter().any(|l| l == "- File"));
    }

    #[test]
    fn render_instance_picks_card_component_and_lists_bindings() {
        let state = synth_user_facts();
        let r = render_current_cell(
            &CurrentCell::Instance {
                noun: "File".into(),
                instance: "f1".into(),
            },
            &state,
        );
        assert_eq!(r.cell_label, "File/f1");
        let sel = r.selected.expect("Instance must select a card Component");
        assert_eq!(sel.component, "card");
        assert!(r.fields.iter().any(|l| l == "Name: alpha.txt"));
    }

    #[test]
    fn render_factcell_no_components_falls_back_to_field_list() {
        // No Component_* facts seeded — selection misses, fields
        // still render via the generic projection.
        let state = {
            let s = Object::phi();
            cell_push(
                "Foo_has_Bar",
                fact_from_pairs(&[("Foo", "f"), ("Bar", "b")]),
                &s,
            )
        };
        let r = render_current_cell(
            &CurrentCell::FactCell {
                cell_name: "Foo_has_Bar".into(),
            },
            &state,
        );
        assert!(r.selected.is_none(), "no Component registered → fallback");
        assert!(r.fields.iter().any(|l| l.contains("Foo=f")));
    }

    // ── Navigation actions as cells (#512) ────────────────────────

    #[test]
    fn render_root_emits_navigation_target_per_noun() {
        // Navigation actions as cells (#512): every noun the system
        // knows about appears as a "navigate to X" affordance off the
        // Root screen. The catalogue is computed from the cell graph,
        // not hand-listed.
        let state = synth_user_facts();
        let r = render_current_cell(&CurrentCell::Root, &state);
        // synth_user_facts seeds a File noun + Component-related cells.
        let labels: Vec<&str> = r.navigation.iter().map(|t| t.label.as_str()).collect();
        assert!(
            labels.iter().any(|l| l.contains("File")),
            "Root navigation must include the File noun: {labels:?}"
        );
    }

    #[test]
    fn render_instance_emits_type_navigation_target() {
        // Instance screens always offer a "back to type" affordance.
        let state = synth_user_facts();
        let r = render_current_cell(
            &CurrentCell::Instance {
                noun: "File".into(),
                instance: "f1".into(),
            },
            &state,
        );
        assert!(
            r.navigation.iter().any(|t| matches!(
                &t.target,
                CurrentCell::Noun { noun } if noun == "File"
            )),
            "Instance navigation must include a Noun(File) target: {:?}",
            r.navigation,
        );
    }

    #[test]
    fn render_includes_empty_navigation_for_unknown_cell() {
        // A FactCell that doesn't exist in state has no facts to walk;
        // the navigation catalogue is empty (the Slint side renders
        // "no navigation targets — this cell is a leaf").
        let state = Object::phi();
        let r = render_current_cell(
            &CurrentCell::FactCell {
                cell_name: "Bogus_cell".into(),
            },
            &state,
        );
        assert!(
            r.navigation.is_empty(),
            "missing cell must yield empty navigation: {:?}",
            r.navigation,
        );
    }
}
