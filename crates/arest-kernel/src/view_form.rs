// crates/arest-kernel/src/view_form.rs
//
// §5.2 viewproj-client-render — the widget-form projection consumed
// by the UnifiedRepl Detail pane: `view_fields_for` joins the
// engine's `arest::viewproj::view_via_rho` STRUCTURE (one
// ViewElement per Fact Type, Component Role per §4.2 value-type →
// widget mapping) against the noun's own cells for instance VALUES.
//
// Lives OUTSIDE `ui_apps` (which is target-gated to
// `x86_64-unknown-uefi` + `feature = "slint"`) so the projection
// logic is host-testable — the same extraction move as
// `unified_repl_regions` and `linuxkpi_virtio_tablet`. No Slint
// types in here: pure `&Object` → owned rows; `ui_apps::unified_repl`
// maps the rows into the generated `ViewField` Slint struct at
// redraw time.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use arest::ast::{self, Object};

/// Extract the leading `<Noun>` token from a `<Noun>_has_<Attribute>`
/// cell name. Returns `None` when the cell name doesn't contain
/// `_has_`.
pub(crate) fn noun_of(cell_name: &str) -> Option<&str> {
    cell_name.split_once("_has_").map(|(noun, _)| noun)
}

/// Every cell that belongs to `noun` in `state`, returned as
/// `(attribute, &cell_contents)` pairs.
pub(crate) fn cells_for_noun<'a>(noun: &str, state: &'a Object) -> Vec<(&'a str, &'a Object)> {
    let prefix_full = format!("{noun}_has_");
    let mut out: Vec<(&str, &Object)> = Vec::new();
    for (cell_name, contents) in ast::cells_iter(state) {
        if let Some(attr) = cell_name.strip_prefix(&prefix_full[..]) {
            out.push((attr, contents));
        }
    }
    out.sort_by(|a, b| a.0.cmp(b.0));
    out
}

/// §5.2 viewproj-client-render: the synthesized instance view's widget
/// rows for one entity — `(label, widget, value)` triples the Slint
/// `ViewField` form renders. Structure from the engine's
/// `view_via_rho` (the same lazy view: rules every other render target
/// consumes — the kernel is just another Render Target consumer);
/// values from the noun's own cells. Empty when the noun derives no
/// view (ui-readings carry the rules; value types need their Format
/// declared — see render-target.md / the csdp precedent).
pub(crate) fn view_fields_for(
    noun: &str,
    instance: &str,
    state: &Object,
) -> Vec<(String, String, String)> {
    let Some(vp) = arest::viewproj::view_via_rho(state, noun, instance) else {
        return Vec::new();
    };
    let noun_prefix = format!("{}_has_", noun.replace(' ', "_"));
    let cells = cells_for_noun(noun, state);
    let value_of = |attr: &str| -> String {
        let spaced = attr.replace('_', " ");
        for (cell_attr, cell) in &cells {
            if *cell_attr != attr {
                continue;
            }
            let Some(facts) = cell.as_seq() else { continue };
            for fact in facts {
                if ast::binding(fact, noun) != Some(instance) {
                    continue;
                }
                // Fact role keys may be spaced ("Task Description") or
                // underscored depending on the writer — try both.
                if let Some(v) = ast::binding(fact, &spaced)
                    .or_else(|| ast::binding(fact, attr))
                {
                    return v.to_string();
                }
            }
        }
        String::new()
    };
    vp.elements
        .iter()
        .map(|el| {
            let attr = el
                .fact_type
                .strip_prefix(noun_prefix.as_str())
                .unwrap_or(&el.fact_type);
            (
                attr.replace('_', " "),
                el.component_role.clone(),
                value_of(attr),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arest::ast::{cell_push, fact_from_pairs};

    // ---- §5.2 viewproj-client-render: boot seed → widget form -----

    /// End-to-end over the EXACT state the QEMU boot serves
    /// (`system::build_boot_state`, which runs the seed last): the
    /// widget form for App/demo must yield one (label, widget, value)
    /// row per seeded Fact Type with the §4.2 value-type → widget
    /// mapping and the instance values joined in. Pinning the REAL
    /// boot build (not a fixture slice) is the point — the first
    /// QEMU run rendered an empty form because the fixture context
    /// diverged from the boot context.
    #[test]
    fn view_projection_demo_seed_yields_widget_form_for_app() {
        let seeded = crate::system::build_boot_state();

        // The seed must report success through its marker cell.
        let status = ast::fetch_cell_seq("viewseed:status", &seeded);
        let status_txt = format!("{status:?}");
        assert!(
            status_txt.contains("ok"),
            "viewseed:status must be ok; got {status_txt}"
        );

        let fields = view_fields_for("App", "demo", &seeded);
        let rows: Vec<(&str, &str, &str)> = fields
            .iter()
            .map(|(l, w, v)| (l.as_str(), w.as_str(), v.as_str()))
            .collect();
        assert_eq!(
            rows,
            alloc::vec![
                ("Active", "checkbox", "true"),
                ("Channel", "combo-box", "stable"),
                ("Description", "text-input",
                 "Illustrates AREST facts: Organizations and Support Requests. \
                  Click a resource on the left to explore."),
                ("Install Date", "date-picker", "2026-01-15"),
                ("Name", "text-input", "Demo"),
            ],
            "demo App form must carry all four §4.2 widget kinds with joined values",
        );

        // The second instance renders the SAME structure with ITS values
        // (structure from schema, values per entity).
        let tasks_fields = view_fields_for("App", "tasks", &seeded);
        let tasks_rows: Vec<(&str, &str)> = tasks_fields
            .iter()
            .map(|(l, _, v)| (l.as_str(), v.as_str()))
            .collect();
        assert!(
            tasks_rows.contains(&("Channel", "beta")) && tasks_rows.contains(&("Name", "Tasks")),
            "tasks App form must join ITS instance values; got {tasks_rows:?}"
        );
    }

    /// A noun with no schema rows derives no view — the form must be
    /// EMPTY (graceful no-op), never a panic or a junk row.
    #[test]
    fn view_fields_empty_for_unseeded_noun() {
        let s = cell_push(
            "Organization_has_Name",
            fact_from_pairs(&[("Organization", "acme"), ("Name", "Acme Corp")]),
            &Object::phi(),
        );
        let seeded = crate::system::seed_view_projection_demo(s);
        assert!(
            view_fields_for("Organization", "acme", &seeded).is_empty(),
            "Organization has no Fact_Type_has_Role rows — no view must derive"
        );
    }
}
