// crates/arest/src/viewproj.rs
//
// The View projection (Theorem 4, view layer) — extracted from
// `command.rs` so the NO_STD kernel can consume it directly
// (viewproj-client-render: the in-kernel Slint surface is just another
// Render Target consumer, but `pub mod command` is std-gated
// wholesale). `command` re-exports everything here, so every existing
// `command::ViewProjection` / `command::view_via_rho` reference is
// unchanged. Serde derives are std-deps-gated (the structs ride JSON
// only on std hosts; the kernel consumes them as plain structs).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::ast;

/// One element of a projected `View` — an abstract widget bound to a Fact
/// Type. The iFactr/MonoTouch.Dialog "member-type → Element" analogue, but
/// keyed off the rendered Fact Type's value-type Format rather than a CLR
/// member type. Platform-neutral: the `component_role` is bound to a native
/// widget at render time via `select_component` (the iFactr binding layer).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "std-deps", derive(serde::Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct ViewElementProjection {
    /// The `ve_<fnv>` ViewElement id (deterministic over the skolem frontier).
    pub id: String,
    /// The rendered Fact Type.
    pub fact_type: String,
    /// The widget kind ('text-input', 'date-picker', 'checkbox', 'combo-box').
    pub component_role: String,
    /// For an enumerated widget ('combo-box'), the value type's allowed values
    /// — the `EnumValues` cell rows for the value-role noun — so the rendered
    /// `<select>` offers every option, not just the current value. Empty for
    /// non-enumerated widgets (and serialized only when non-empty, so a plain
    /// text/date/checkbox element's JSON is unchanged).
    #[cfg_attr(feature = "std-deps",
        serde(default, skip_serializing_if = "Vec::is_empty"))]
    pub options: Vec<String>,
}

/// The abstract control tree projected for a fetched entity — the
/// iFactr/MonoView "abstract UI" half of the Theorem-4 HATEOAS
/// representation. `source` records which override tier produced it:
/// 'synthesized' (iFactr default, auto from value types), 'authored'
/// (MonoView abstract override declared in the population), or 'platform'
/// (MonoCross IoC, a `Func::Platform` custom view — reserved).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "std-deps", derive(serde::Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct ViewProjection {
    /// The View id (synthesized slug or authored View name).
    pub view: String,
    /// View Kind — 'instance' (form) for the per-entity detail view.
    pub kind: String,
    /// Which override tier produced this view.
    pub source: String,
    /// The widgets, ordered by rendered Fact Type.
    pub elements: Vec<ViewElementProjection>,
    /// §5.2 Platform Binding (pb-render-fn-contract): rendered output per
    /// Render Target, keyed by the target slug ('html' → markup). One entry
    /// per `Render Target has Platform Function Name` fact whose Platform fn
    /// has an installed body (`render_via_targets`); empty when no targets
    /// are declared, no bodies are installed, or under no_std.
    #[cfg_attr(feature = "std-deps",
        serde(default, skip_serializing_if = "alloc::collections::BTreeMap::is_empty"))]
    pub representations: alloc::collections::BTreeMap<String, String>,
}

/// task-viewproj: View projection (Theorem 4, view layer) — the iFactr/
/// MonoView "abstract UI" half of the HATEOAS representation for ONE fetched
/// entity. Widget kinds are derived from the Noun's value types (the
/// MonoTouch.Dialog member-type → Element idea) via the lazy `view:` rules in
/// readings/ui/view-detail.md. Resolution is a precedence walk:
///
///   1. Platform (MonoCross IoC) — a `Func::Platform`-injected custom view
///      wins entirely. RESERVED: the `PLATFORM_FALLBACK` registry is the IoC
///      container; no view hook is registered yet, so this tier is a seam.
///   2. Authored (MonoView) — an instance `View` declared for this Noun in
///      the population overrides the default.
///   3. Synthesized (iFactr default) — mint a transient instance View for the
///      Noun so the same value-type → widget rules derive a default form.
///
/// Returns None when nothing materializes — notably when `ui-readings` is
/// compiled out (no `view:` defs → `resolve_view` yields None), so a pure-CRM
/// engine no-ops cleanly. Structure only (field + widget); instance VALUES are
/// filled at render time (view-detail.md "Remaining Work (2)").
pub fn view_via_rho(d: &ast::Object, noun: &str, _entity_id: &str) -> Option<ViewProjection> {
    // Tier 2 vs 3: is there an AUTHORED instance View for this Noun?
    let authored = ast::fetch_cell_seq("View_is_for_Noun", d).as_seq()
        .and_then(|facts| facts.iter().find_map(|f| {
            if ast::binding(f, "Noun") != Some(noun) { return None; }
            let v = ast::binding(f, "View")?;
            (view_kind_of(d, v).as_deref() == Some("instance")).then(|| v.to_string())
        }));

    let synth_id = format!("instance-view-{}", noun);
    let (view_id, source): (String, &str) = match authored {
        Some(v) => (v, "authored"),
        None    => (synth_id, "synthesized"),
    };

    // Synthesized tier injects a transient instance View so the SAME lazy
    // view-detail rules fire; the authored tier reads the population as-is.
    let injected;
    let pop: &ast::Object = if source == "synthesized" {
        let s = ast::cell_push("View_is_for_Noun",
            ast::fact_from_pairs(&[("View", view_id.as_str()), ("Noun", noun)]), d);
        injected = ast::cell_push("View_has_View_Kind",
            ast::fact_from_pairs(&[("View", view_id.as_str()), ("View Kind", "instance")]), &s);
        &injected
    } else {
        d
    };

    // Resolve the lazy view: rules. None here ⇒ ui-readings off ⇒ no view.
    let renders = ast::resolve_view("ViewElement_renders_Fact_Type", pop, pop)?;
    let roles = ast::resolve_view("ViewElement_has_Component_Role", pop, pop)
        .unwrap_or_else(ast::Object::phi);

    // ViewElement → Component Role (the widget per element).
    let role_of: hashbrown::HashMap<String, String> = roles.as_seq()
        .map(|items| items.iter().filter_map(|f| Some((
            ast::binding(f, "ViewElement")?.to_string(),
            ast::binding(f, "Component Role")?.to_string(),
        ))).collect())
        .unwrap_or_default();

    // Keep only elements rendered under OUR View; join the widget role.
    let mut elements: Vec<ViewElementProjection> = renders.as_seq()
        .map(|items| items.iter().filter_map(|f| {
            if ast::binding(f, "View") != Some(view_id.as_str()) { return None; }
            let id = ast::binding(f, "ViewElement")?.to_string();
            let fact_type = ast::binding(f, "Fact Type")?.to_string();
            let component_role = role_of.get(&id).cloned().unwrap_or_default();
            // Enumerated widget → surface the value type's allowed values from
            // the runtime population, so the rendered <select> is a real
            // dropdown. Read from `d` (the live cells), not `pop` (which only
            // adds the transient synth View facts).
            let options = if component_role == "combo-box" {
                enum_options_for_fact_type(d, noun, &fact_type)
            } else {
                Vec::new()
            };
            Some(ViewElementProjection { id, fact_type, component_role, options })
        }).collect())
        .unwrap_or_default();

    if elements.is_empty() { return None; }
    // Deterministic order: the reading carries an optional `Order`; absent at
    // this slice, the rendered Fact Type name is the stable tiebreak.
    elements.sort_by(|a, b| a.fact_type.cmp(&b.fact_type));

    Some(ViewProjection {
        view: view_id, kind: "instance".to_string(),
        source: source.to_string(), elements,
        representations: Default::default(),
    })
}

/// The View Kind bound to a View id (scans `View_has_View_Kind`).
fn view_kind_of(d: &ast::Object, view: &str) -> Option<String> {
    ast::fetch_cell_seq("View_has_View_Kind", d).as_seq()
        .and_then(|facts| facts.iter().find_map(|f| {
            (ast::binding(f, "View") == Some(view))
                .then(|| ast::binding(f, "View Kind").map(String::from)).flatten()
        }))
}

/// Allowed values for an enumerated value-type, read from the runtime
/// `EnumValues` cell (rows keyed `noun` + `value0`, `value1`, …). The
/// combo-box's value-role noun is recovered the same way the renderer derives
/// the field label: strip the `{noun}_has_` prefix off the Fact Type and
/// restore spaces (the binary `Noun has ValueNoun` shape). Empty when the Fact
/// Type isn't that shape or the noun has no `EnumValues` row — the renderer
/// then falls back to showing just the current value, i.e. prior behaviour.
/// Mirrors `induce::enum_values_for_noun`, kept local so viewproj stays
/// no_std-consumable without depending on the std-gated induce module.
fn enum_options_for_fact_type(d: &ast::Object, noun: &str, fact_type: &str) -> Vec<String> {
    let prefix = format!("{}_has_", noun.replace(' ', "_"));
    let value_noun = match fact_type.strip_prefix(prefix.as_str()) {
        Some(rest) => rest.replace('_', " "),
        None => return Vec::new(),
    };
    let cell = ast::fetch_cell_seq("EnumValues", d);
    let Some(seq) = cell.as_seq() else { return Vec::new() };
    for f in seq.iter() {
        if ast::binding(f, "noun") != Some(value_noun.as_str()) { continue; }
        return (0..)
            .map_while(|i| ast::binding(f, &format!("value{i}")).map(String::from))
            .collect();
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The enum reader recovers the value-role noun from the binary Fact Type
    /// and returns its `EnumValues` row in declaration order; a non-binary
    /// shape or a missing row yields no options (renderer falls back).
    #[test]
    fn enum_options_read_from_enumvalues_cell() {
        let d = ast::cell_push("EnumValues", ast::fact_from_pairs(&[
            ("noun", "Task Status"),
            ("value0", "pending"), ("value1", "in_progress"), ("value2", "done"),
        ]), &ast::Object::phi());

        assert_eq!(
            enum_options_for_fact_type(&d, "Task", "Task_has_Task_Status"),
            alloc::vec!["pending", "in_progress", "done"],
            "binary fact type resolves its value-role noun's enum row");
        assert!(enum_options_for_fact_type(&d, "Task", "Task_is_done").is_empty(),
            "a unary (no `{{noun}}_has_` prefix) yields no options");
        assert!(enum_options_for_fact_type(&d, "Task", "Task_has_Unlisted").is_empty(),
            "a noun with no EnumValues row yields no options");
    }
}
