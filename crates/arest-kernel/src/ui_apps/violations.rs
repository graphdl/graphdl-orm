// crates/arest-kernel/src/ui_apps/violations.rs
//
// Inline constraint violations on the current screen (#590, EPIC #496).
//
// EEEEE #516 landed the breadcrumb (where am I), this session's #517
// landed cell-type help (what kind of thing is this). The third
// introspection question — what's wrong here? — closes the loop.
//
// # Theorem 4 — violations are first-class facts
//
// Per `readings/core/outcomes.md`, every constraint violation
// materialises as a `Violation` entity with these fact types:
//
//   * Violation_has_Text                  — human-readable explanation
//   * Violation_is_of_Constraint          — which rule fired
//   * Violation_has_Severity              — severity tag (error / warn / …)
//   * Violation_is_triggered_by_Resource  — pointer to the offending cell
//   * Violation_occurred_at_Timestamp     — when (omitted for first pass)
//   * Violation_belongs_to_Domain         — scope (omitted for first pass)
//
// This module reads those cells, joins them by the Violation id, and
// optionally filters to violations whose Resource references the
// current screen's cell. Pure `&Object` → owned-data, no state plumbing
// inside this module — the caller (`unified_repl::submit`) wraps the
// query in `crate::system::with_state(...)`.
//
// # Why a separate module
//
// Same shape as the four sibling modules — `cell_renderer`,
// `navigation`, `actions`, `breadcrumb`, `help`. Each answers one
// HATEOAS question by querying live state and returning owned data:
//
//   cell_renderer  → what does this cell look like?
//   navigation     → what cells can I reach from here?
//   actions        → what SYSTEM calls can I make from here?
//   breadcrumb     → where did I come from?
//   help           → what kind of screen is this?
//   violations     → what's wrong here?  ← this module
//
// # Resource matching is intentionally permissive
//
// The Resource binding on a Violation is the engine's pointer to
// whatever triggered the constraint. Today the apply pipeline writes
// it in different shapes depending on the constraint kind: a fully-
// qualified instance ref (`Order::ord-1`) for entity-level violations,
// a bare cell name (`Order_has_Customer`) for fact-cell violations, a
// noun name for noun-level violations. Rather than enforce a single
// canonical format, this module does substring matching in either
// direction so all three shapes light up the right screen. A future
// task can normalise the Resource format and tighten the match.

#![allow(dead_code)]

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use arest::ast::{self, Object};

use crate::ui_apps::cell_renderer::CurrentCell;

// ── ViolationView ──────────────────────────────────────────────────

/// A flattened view of one Violation entity, joined across the four
/// reading-defined fact types. `text` and `id` are mandatory (every
/// Violation must have both per the readings); the others may be
/// empty when the corresponding fact has not been written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViolationView {
    /// Violation entity id — the join key across the per-attribute
    /// fact cells.
    pub id: String,
    /// Human-readable explanation. Sourced from `Violation_has_Text`.
    pub text: String,
    /// Identifier of the Constraint that fired. Sourced from
    /// `Violation_is_of_Constraint`. Empty when the cell has no fact
    /// pairing this Violation to a Constraint (which would itself be
    /// a deontic violation per outcomes.md's mandatory constraint).
    pub constraint: String,
    /// Severity tag (`error`, `warn`, …). Sourced from
    /// `Violation_has_Severity`. Empty when no fact pairs the
    /// Violation to a severity.
    pub severity: String,
    /// Pointer to the offending cell / instance / noun, in whatever
    /// shape the apply pipeline wrote it. `None` when the Violation
    /// does not (yet) carry a `Violation_is_triggered_by_Resource`
    /// fact — that fact type is at-most-one per outcomes.md so a
    /// violation without a resource is a system-level violation
    /// (constraint failed without a localisable target).
    pub resource: Option<String>,
}

// ── Public query API ───────────────────────────────────────────────

/// Collect every live Violation in current state into flattened
/// `ViolationView`s. Spines on `Violation_has_Text` (mandatory per
/// the readings) and joins the other attribute cells by Violation id.
pub fn all_violations(state: &Object) -> Vec<ViolationView> {
    let text_cell = ast::fetch_or_phi("Violation_has_Text", state);
    let constraint_cell = ast::fetch_or_phi("Violation_is_of_Constraint", state);
    let severity_cell = ast::fetch_or_phi("Violation_has_Severity", state);
    let resource_cell = ast::fetch_or_phi("Violation_is_triggered_by_Resource", state);

    let constraints = facts_to_map(&constraint_cell, "Violation", "Constraint");
    let severities = facts_to_map(&severity_cell, "Violation", "Severity");
    let resources = facts_to_map(&resource_cell, "Violation", "Resource");

    text_cell
        .as_seq()
        .map(|facts| {
            facts
                .iter()
                .filter_map(|f| {
                    let id = ast::binding(f, "Violation")?.to_string();
                    let text = ast::binding(f, "Text")?.to_string();
                    let constraint = constraints.get(&id).cloned().unwrap_or_default();
                    let severity = severities.get(&id).cloned().unwrap_or_default();
                    let resource = resources.get(&id).cloned();
                    Some(ViolationView {
                        id,
                        text,
                        constraint,
                        severity,
                        resource,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Filter `all_violations` to those whose Resource binding references
/// the current cell. Returns the empty vec for `CurrentCell::Root`
/// (Root is the resource-less landing — no per-cell violations apply).
///
/// Match is bidirectional substring (`needle in resource OR resource
/// in needle`) so the three Resource shapes the apply pipeline emits
/// today (`Noun::Id` / `Cell_Name` / bare noun) all light up the
/// matching screen without enforcing a single canonical format. A
/// future task can normalise the Resource shape and tighten the match
/// to exact equality.
pub fn violations_for_cell(cell: &CurrentCell, state: &Object) -> Vec<ViolationView> {
    let needle = match cell {
        CurrentCell::Root => return Vec::new(),
        CurrentCell::Noun { noun } => noun.clone(),
        CurrentCell::Instance { noun, instance } => format!("{noun}::{instance}"),
        CurrentCell::FactCell { cell_name } => cell_name.clone(),
        CurrentCell::ComponentInstance { component_id } => component_id.clone(),
    };
    all_violations(state)
        .into_iter()
        .filter(|v| match v.resource.as_deref() {
            Some(r) => r.contains(&needle) || needle.contains(r),
            None => false,
        })
        .collect()
}

/// Format a list of ViolationViews as scrollback-ready lines. Returns
/// a single explanatory line when the slice is empty so the user gets
/// a clear "all clear" signal rather than an ambiguous blank response.
pub fn render_violations(views: &[ViolationView]) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    if views.is_empty() {
        lines.push("No violations against this screen.".to_string());
        return lines;
    }
    lines.push(format!(
        "─── {} violation(s) on this screen ───",
        views.len()
    ));
    for v in views {
        let prefix = if v.severity.is_empty() {
            "  [!]".to_string()
        } else {
            format!("  [!] [{}]", v.severity)
        };
        lines.push(format!("{prefix} {}", v.text));
        if !v.constraint.is_empty() {
            lines.push(format!("      rule: {}", v.constraint));
        }
        if let Some(res) = &v.resource {
            lines.push(format!("      on:   {res}"));
        }
        if !v.id.is_empty() {
            lines.push(format!("      id:   {} (navigate via `instance Violation {}`)", v.id, v.id));
        }
    }
    lines
}

/// Convenience: query + render in one call. The unified REPL uses
/// this from the `violations` / `wrong` command intercept.
pub fn render_for_cell(cell: &CurrentCell, state: &Object) -> Vec<String> {
    render_violations(&violations_for_cell(cell, state))
}

// ── Helpers ────────────────────────────────────────────────────────

/// Build a `BTreeMap<key_role_value, value_role_value>` from a fact-
/// cell whose facts each carry the named role pair. Values from
/// duplicate keys collapse to whichever the last fact provides; per
/// outcomes.md every secondary attribute is at-most-one so duplicates
/// do not arise in well-formed state.
fn facts_to_map(cell: &Object, key_role: &str, value_role: &str) -> BTreeMap<String, String> {
    cell.as_seq()
        .map(|facts| {
            facts
                .iter()
                .filter_map(|f| {
                    let key = ast::binding(f, key_role)?.to_string();
                    let val = ast::binding(f, value_role)?.to_string();
                    Some((key, val))
                })
                .collect()
        })
        .unwrap_or_default()
}

// ── Tests ────────────────────────────────────────────────────────────
//
// Pure-function tests over hand-built Objects. The shape mirrors what
// the apply pipeline writes: a `Violation_has_Text` cell with one fact
// per Violation entity, plus optional secondary cells the join picks
// up. Tests cover the empty case, a single Violation, multiple
// Violations, partial joins (missing severity / resource), and
// per-cell filtering for each CurrentCell variant.

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use arest::ast::Object;

    fn fact(roles: &[(&str, &str)]) -> Object {
        Object::seq(
            roles
                .iter()
                .map(|(role, value)| {
                    Object::seq(vec![Object::atom(role), Object::atom(value)])
                })
                .collect(),
        )
    }

    /// Build a state Object with the named cells. Mirrors the shape
    /// `system_impl` would produce after several applies.
    fn state_with_cells(cells: &[(&str, Object)]) -> Object {
        Object::seq(
            cells
                .iter()
                .map(|(name, contents)| {
                    Object::seq(vec![
                        Object::atom("CELL"),
                        Object::atom(name),
                        contents.clone(),
                    ])
                })
                .collect(),
        )
    }

    #[test]
    fn empty_state_yields_no_violations() {
        let state = state_with_cells(&[]);
        assert!(all_violations(&state).is_empty());
    }

    #[test]
    fn one_violation_with_full_attributes_joins_correctly() {
        let state = state_with_cells(&[
            (
                "Violation_has_Text",
                Object::seq(vec![fact(&[
                    ("Violation", "v1"),
                    ("Text", "Order is missing Customer"),
                ])]),
            ),
            (
                "Violation_is_of_Constraint",
                Object::seq(vec![fact(&[
                    ("Violation", "v1"),
                    ("Constraint", "Order_was_placed_by_Customer_MC"),
                ])]),
            ),
            (
                "Violation_has_Severity",
                Object::seq(vec![fact(&[("Violation", "v1"), ("Severity", "error")])]),
            ),
            (
                "Violation_is_triggered_by_Resource",
                Object::seq(vec![fact(&[
                    ("Violation", "v1"),
                    ("Resource", "Order::ord-1"),
                ])]),
            ),
        ]);
        let views = all_violations(&state);
        assert_eq!(views.len(), 1);
        let v = &views[0];
        assert_eq!(v.id, "v1");
        assert_eq!(v.text, "Order is missing Customer");
        assert_eq!(v.constraint, "Order_was_placed_by_Customer_MC");
        assert_eq!(v.severity, "error");
        assert_eq!(v.resource.as_deref(), Some("Order::ord-1"));
    }

    #[test]
    fn missing_secondary_attributes_default_to_empty() {
        // Only Text — no constraint / severity / resource cells.
        let state = state_with_cells(&[(
            "Violation_has_Text",
            Object::seq(vec![fact(&[("Violation", "v1"), ("Text", "broken")])]),
        )]);
        let views = all_violations(&state);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].constraint, "");
        assert_eq!(views[0].severity, "");
        assert!(views[0].resource.is_none());
    }

    #[test]
    fn root_cell_returns_empty_per_design() {
        // Root is the resource-less landing — no per-cell violations apply.
        let state = state_with_cells(&[(
            "Violation_has_Text",
            Object::seq(vec![fact(&[("Violation", "v1"), ("Text", "system-wide problem")])]),
        )]);
        assert!(violations_for_cell(&CurrentCell::Root, &state).is_empty());
    }

    #[test]
    fn instance_cell_filters_by_qualified_resource_ref() {
        let state = state_with_cells(&[
            (
                "Violation_has_Text",
                Object::seq(vec![
                    fact(&[("Violation", "v1"), ("Text", "Order ord-1 problem")]),
                    fact(&[("Violation", "v2"), ("Text", "Order ord-2 problem")]),
                ]),
            ),
            (
                "Violation_is_triggered_by_Resource",
                Object::seq(vec![
                    fact(&[("Violation", "v1"), ("Resource", "Order::ord-1")]),
                    fact(&[("Violation", "v2"), ("Resource", "Order::ord-2")]),
                ]),
            ),
        ]);
        let cell = CurrentCell::Instance {
            noun: "Order".to_string(),
            instance: "ord-1".to_string(),
        };
        let views = violations_for_cell(&cell, &state);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].id, "v1");
    }

    #[test]
    fn fact_cell_filters_by_cell_name_substring() {
        let state = state_with_cells(&[
            (
                "Violation_has_Text",
                Object::seq(vec![fact(&[
                    ("Violation", "v1"),
                    ("Text", "duplicate UC"),
                ])]),
            ),
            (
                "Violation_is_triggered_by_Resource",
                Object::seq(vec![fact(&[
                    ("Violation", "v1"),
                    ("Resource", "Order_has_Customer"),
                ])]),
            ),
        ]);
        let cell = CurrentCell::FactCell {
            cell_name: "Order_has_Customer".to_string(),
        };
        let views = violations_for_cell(&cell, &state);
        assert_eq!(views.len(), 1);
    }

    #[test]
    fn render_empty_emits_all_clear_message() {
        let lines = render_violations(&[]);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("No violations"));
    }

    #[test]
    fn render_one_includes_text_rule_resource_id() {
        let view = ViolationView {
            id: "v1".to_string(),
            text: "Order is missing Customer".to_string(),
            constraint: "Order_was_placed_by_Customer_MC".to_string(),
            severity: "error".to_string(),
            resource: Some("Order::ord-1".to_string()),
        };
        let lines = render_violations(&[view]);
        let blob = lines.join("\n");
        assert!(blob.contains("Order is missing Customer"));
        assert!(blob.contains("error"));
        assert!(blob.contains("Order_was_placed_by_Customer_MC"));
        assert!(blob.contains("Order::ord-1"));
        assert!(blob.contains("v1"));
    }
}
