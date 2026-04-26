// crates/arest/src/select_component_core.rs
//
// Pure-FORML cell-walker for the #493 `select_component` MCP verb.
//
// ## Why this lives outside `command.rs`
//
// `command.rs` carries the `Command` enum + serde adapters + the rest of
// the imperative-input dispatch surface. The whole module is gated to
// the std build because of the serde derive on `Command` and the
// dispatch glue around it.
//
// `select_component` itself is pure FORML: walk the
// `Component_has_Component_Role` cell, join with
// `Component_is_implemented_by_Toolkit_at_Toolkit_Symbol`, score under
// the constraint axes, return the top-N. There is no I/O, no serde,
// no global state — every input is already in the `ast::Object` that
// the caller hands in.
//
// VVVV's cell-renderer (#511) ported a copy of this logic into
// `arest-kernel/src/ui_apps/cell_renderer.rs` because the engine
// version was unreachable under no_std. With the core extracted here
// (no_std-clean module), the kernel can call the engine version
// directly and the port collapses to a thin caller wrapper.
//
// `command.rs::select_component` (std build) still exists as the
// public re-export the engine uses; it forwards to the core. The
// JSON adapter (`select_component_json`) stays in `command.rs`
// because it pulls `serde_json` which is std-deps-only.

#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, format};
use crate::ast::{self, Object};

// ── Public types ────────────────────────────────────────────────────

/// Constraint axes for `select_component`. Mirrors the MonoView
/// "Interaction Mode" / "Density" / "A11y" / "Theme" / "Surface Tier"
/// dimensions the rules condition on. Every field is optional so
/// callers can supply only the axes they care about; unspecified axes
/// contribute no scoring boosts and no penalties.
///
/// The serde derives are gated on `std-deps` so the no_std build does
/// not pull `serde::{Deserialize, Serialize}`. Callers under no_std
/// (the kernel) construct this struct field-by-field; callers under
/// std (the MCP layer) deserialize from a JSON request body.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "std-deps", derive(serde::Deserialize, serde::Serialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct SelectComponentConstraints {
    /// Interaction mode. Values mirror MonoView.Interaction Mode:
    /// 'pointer', 'keyboard', 'touch'. Convenience boolean `touch`
    /// (below) sets this to 'touch' when true.
    #[cfg_attr(feature = "std-deps", serde(default))]
    pub interaction_mode: Option<String>,
    /// Density. Values: 'compact', 'regular', 'spacious'.
    #[cfg_attr(feature = "std-deps", serde(default))]
    pub density: Option<String>,
    /// A11y profiles. Values: 'high-contrast', 'reduced-motion',
    /// 'screen-reader-aware'. JS callers may also pass the loose
    /// short forms ("screen_reader", "screen-reader") — we normalise.
    #[cfg_attr(feature = "std-deps", serde(default))]
    pub a11y: Vec<String>,
    /// Theme mode. Values: 'dark', 'light'.
    #[cfg_attr(feature = "std-deps", serde(default))]
    pub theme: Option<String>,
    /// Surface tier. Values: 'backdrop', 'panel', 'overlay',
    /// 'drop-shadow'.
    #[cfg_attr(feature = "std-deps", serde(default))]
    pub surface: Option<String>,
    /// Convenience: `touch=true` is sugar for `interaction_mode='touch'`.
    #[cfg_attr(feature = "std-deps", serde(default))]
    pub touch: bool,
    /// Maximum number of (Component, Toolkit) pairs to return. Default 5.
    #[cfg_attr(feature = "std-deps", serde(default))]
    pub limit: Option<usize>,
}

/// One ranked Component implementation returned by `select_component`.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "std-deps", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "std-deps", serde(rename_all = "camelCase"))]
pub struct SelectedComponent {
    /// Component name (e.g. "button").
    pub component: String,
    /// Component role (e.g. "button"). Always set for well-formed populations.
    pub role: String,
    /// Toolkit slug (e.g. "gtk4").
    pub toolkit: String,
    /// Toolkit-side identifier (e.g. "GtkButton").
    pub symbol: String,
    /// Score under the supplied constraints. Higher is better.
    pub score: u32,
}

// ── select_component ────────────────────────────────────────────────

/// `select_component` — engine-side handler for #493 MCP verb.
///
/// Walks every Component whose Role substring-matches `intent`,
/// enumerates that Component's ImplementationBindings (one per Toolkit),
/// scores each pair under the supplied constraints, and returns the top
/// N (default 5) sorted by score descending. Within equal scores the
/// order is stable per (component, toolkit) sort which keeps output
/// reproducible across runs.
///
/// Returns an empty vec if no Component matches the intent — the
/// caller (MCP layer) renders this as `[]` so the LLM sees the gap.
///
/// Pure: no I/O, no global state. Same signature and semantics as
/// the older `command::select_component`; that fn now forwards here
/// under the std build. Kernel code (#511's cell renderer) calls
/// this directly.
pub fn select_component(
    state: &Object,
    intent: &str,
    constraints: &SelectComponentConstraints,
) -> Vec<SelectedComponent> {
    let intent_lc = intent.trim().to_lowercase();
    let limit = constraints.limit.unwrap_or(5);

    // (Component name, Component Role) pairs.
    let role_cell = ast::fetch_or_phi("Component_has_Component_Role", state);
    let candidates: Vec<(String, String)> = role_cell.as_seq().map(|facts| facts.iter()
        .filter_map(|f| {
            let comp = ast::binding(f, "Component")?.to_string();
            let role = ast::binding(f, "Component Role")?.to_string();
            // Empty intent matches everything; otherwise containment match
            // both directions — "I need a date picker" contains "date-picker"
            // (after normalising hyphens) and vice versa.
            let role_norm = role.replace('-', " ").to_lowercase();
            let intent_norm = intent_lc.replace('-', " ");
            let matches = intent_lc.is_empty()
                || intent_norm.contains(&role_norm)
                || role_norm.contains(&intent_norm)
                || intent_norm.split_whitespace().any(|w|
                    !w.is_empty() && role_norm.contains(w) && w.len() >= 4);
            matches.then_some((comp, role))
        }).collect()).unwrap_or_default();

    // For each candidate Component, enumerate its ImplementationBindings.
    let bind_cell = ast::fetch_or_phi(
        "Component_is_implemented_by_Toolkit_at_Toolkit_Symbol",
        state,
    );
    let mut results: Vec<SelectedComponent> = candidates.iter()
        .flat_map(|(comp, role)| {
            bind_cell.as_seq().unwrap_or(&[]).iter().filter_map(move |f| {
                if !ast::binding_matches(f, "Component", comp) { return None; }
                let toolkit = ast::binding(f, "Toolkit")?.to_string();
                let symbol = ast::binding(f, "Toolkit Symbol")?.to_string();
                let bname = binding_name_for(comp, &toolkit, state)
                    .unwrap_or_else(|| format!("{}.{}", comp, toolkit));
                let score = score_binding(comp, &toolkit, &bname, constraints, state);
                Some(SelectedComponent {
                    component: comp.clone(),
                    role: role.clone(),
                    toolkit,
                    symbol,
                    score,
                })
            })
        })
        .collect();

    // Sort by score desc; tie-break by (component, toolkit) for reproducibility.
    results.sort_by(|a, b| b.score.cmp(&a.score)
        .then_with(|| a.component.cmp(&b.component))
        .then_with(|| a.toolkit.cmp(&b.toolkit)));
    results.truncate(limit);
    results
}

// ── helpers ─────────────────────────────────────────────────────────
//
// All helpers are crate-pub so command.rs's `score_binding` /
// `binding_name_for` test fixtures (and any other in-crate consumer
// that wants the same FORML walks) can keep reaching them via the
// `crate::select_component_core::*` path. Outside the crate, the
// `select_component` entry point is the supported surface.

/// Normalise a free-form a11y label into the canonical MonoView token
/// the selection rules condition on. Tolerates the loose forms
/// callers (and LLM tool-call payloads) often produce.
pub(crate) fn normalize_a11y(token: &str) -> String {
    let t = token.trim().to_lowercase().replace('_', "-");
    match t.as_str() {
        "screen-reader" | "screenreader" | "screen-reader-aware" |
        "screenreader-aware" | "a11y" | "ax" => "screen-reader-aware".to_string(),
        "high-contrast" | "contrast" => "high-contrast".to_string(),
        "reduced-motion" | "no-motion" => "reduced-motion".to_string(),
        _ => t,
    }
}

/// Collect the trait set declared on the abstract Component.
pub(crate) fn component_traits(component: &str, state: &Object) -> Vec<String> {
    let cell = ast::fetch_or_phi("Component_has_Trait", state);
    cell.as_seq().map(|facts| facts.iter()
        .filter(|f| ast::binding_matches(f, "Component", component))
        .filter_map(|f| ast::binding(f, "Component Trait").map(|s| s.to_string()))
        .collect())
        .unwrap_or_default()
}

/// Collect the trait set declared on a specific ImplementationBinding.
pub(crate) fn binding_traits(binding_name: &str, state: &Object) -> Vec<String> {
    let cell = ast::fetch_or_phi("ImplementationBinding_has_Trait", state);
    cell.as_seq().map(|facts| facts.iter()
        .filter(|f| ast::binding_matches(f, "ImplementationBinding", binding_name))
        .filter_map(|f| ast::binding(f, "Component Trait").map(|s| s.to_string()))
        .collect())
        .unwrap_or_default()
}

/// Find the binding-anchor name for a (component, toolkit) pair.
/// Returns the `ImplementationBinding` row's name field (e.g. "button.qt6").
pub(crate) fn binding_name_for(component: &str, toolkit: &str, state: &Object) -> Option<String> {
    let cell = ast::fetch_or_phi(
        "ImplementationBinding_pivots_Component_is_implemented_by_Toolkit_at_Toolkit_Symbol",
        state,
    );
    cell.as_seq()?.iter().find_map(|f| {
        let matches = ast::binding_matches(f, "Component", component)
            && ast::binding_matches(f, "Toolkit", toolkit);
        matches.then(|| ast::binding(f, "ImplementationBinding").map(|s| s.to_string())).flatten()
    })
}

/// Look up the Toolkit Slug for a Toolkit row (slug == name in the
/// seeded population, but the rules condition on Slug specifically).
pub(crate) fn toolkit_slug(toolkit: &str, state: &Object) -> String {
    let cell = ast::fetch_or_phi("Toolkit_has_Toolkit_Slug", state);
    cell.as_seq().and_then(|facts| facts.iter()
        .find(|f| ast::binding_matches(f, "Toolkit", toolkit))
        .and_then(|f| ast::binding(f, "Toolkit Slug").map(|s| s.to_string())))
        .unwrap_or_else(|| toolkit.to_string())
}

/// Score one (Component × Toolkit) pair under the supplied constraints.
///
/// Each clause that fires under the supplied axes contributes +1, mirroring
/// HHHH's #492 derivation rules one-for-one. The tie-breaker rule (Slint
/// always wins) contributes +1 unconditionally for Slint bindings — its
/// purpose is to disambiguate ties, which we honour by giving every Slint
/// binding a single floor point regardless of constraints.
pub(crate) fn score_binding(
    component: &str,
    toolkit: &str,
    binding_name: &str,
    constraints: &SelectComponentConstraints,
    state: &Object,
) -> u32 {
    let comp_traits = component_traits(component, state);
    let bind_traits = binding_traits(binding_name, state);
    let slug = toolkit_slug(toolkit, state);
    // Effective trait set is the union, per the rule comments
    // ("The selection rule unions the abstract Component traits with
    // the binding-scoped traits before scoring.").
    let has_trait = |t: &str| comp_traits.iter().any(|x| x == t)
        || bind_traits.iter().any(|x| x == t);
    let bind_has_trait = |t: &str| bind_traits.iter().any(|x| x == t);

    let interaction = constraints.interaction_mode.as_deref()
        .or_else(|| constraints.touch.then_some("touch"))
        .map(|s| s.to_lowercase());
    let density = constraints.density.as_deref().map(|s| s.to_lowercase());
    let a11y: Vec<String> = constraints.a11y.iter().map(|s| normalize_a11y(s)).collect();
    let theme = constraints.theme.as_deref().map(|s| s.to_lowercase());
    let surface = constraints.surface.as_deref().map(|s| s.to_lowercase());

    let mut score = 0u32;

    // Touch density preference (#492): touch + Component.touch_optimized.
    if interaction.as_deref() == Some("touch") && has_trait("touch_optimized") {
        score += 1;
    }
    // Pointer-interaction preference: pointer + keyboard_navigable.
    if interaction.as_deref() == Some("pointer") && has_trait("keyboard_navigable") {
        score += 1;
    }
    // Keyboard-interaction preference: keyboard + keyboard_navigable.
    if interaction.as_deref() == Some("keyboard") && has_trait("keyboard_navigable") {
        score += 1;
    }
    // Keyboard density preference: keyboard + binding.compact_native.
    if interaction.as_deref() == Some("keyboard") && bind_has_trait("compact_native") {
        score += 1;
    }
    // Compact density preference: density.compact + binding.compact_native.
    if density.as_deref() == Some("compact") && bind_has_trait("compact_native") {
        score += 1;
    }
    // Spacious density preference: density.spacious + Component.touch_optimized.
    if density.as_deref() == Some("spacious") && has_trait("touch_optimized") {
        score += 1;
    }
    // High-contrast a11y preference: profile + Component.theming_consumer.
    if a11y.iter().any(|p| p == "high-contrast") && has_trait("theming_consumer") {
        score += 1;
    }
    // Screen-reader / GTK preference (DDDD's seed rule): A11y screen-reader
    // + Toolkit gtk4 + binding.screen_reader_aware. This is the strongest
    // signal in the screen-reader scenario — the rule conjuncts on
    // Toolkit Slug AND binding trait, and DDDD's #485 seed body comments
    // the GTK preference as the canonical accessibility target.
    if a11y.iter().any(|p| p == "screen-reader-aware") && slug == "gtk4"
        && bind_has_trait("screen_reader_aware") {
        score += 1;
    }
    // Reduced-motion / Slint preference.
    if a11y.iter().any(|p| p == "reduced-motion") && slug == "slint" {
        score += 1;
    }
    // Reduced-motion / GTK 4 preference.
    if a11y.iter().any(|p| p == "reduced-motion") && slug == "gtk4" {
        score += 1;
    }
    // Kernel-resident Slint preference: Slint + binding.kernel_native.
    if slug == "slint" && bind_has_trait("kernel_native") {
        score += 1;
    }
    // Surface Tier panel preference: panel + Component.theming_consumer.
    if surface.as_deref() == Some("panel") && has_trait("theming_consumer") {
        score += 1;
    }
    // Surface Tier overlay -> Web Components.
    if surface.as_deref() == Some("overlay") && slug == "web-components" {
        score += 1;
    }
    // Dark theme preference: theme.dark + binding.dark_mode_native.
    if theme.as_deref() == Some("dark") && bind_has_trait("dark_mode_native") {
        score += 1;
    }
    // Tie-breaker: deterministic Slint default. The rule body in #492
    // is unconditional ("ImplementationBinding is preferred for MonoView
    // if … Toolkit has Toolkit Slug 'slint'") — give Slint a single
    // tie-break point.
    if slug == "slint" {
        score += 1;
    }

    score
}
