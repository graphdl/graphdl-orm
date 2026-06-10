// crates/arest/src/platform/render_html.rs
//
// `render:html` — the reference §5.2 render function (pb-render-fn-contract).
//
// A Render Target (readings/ui/render-target.md) names a Platform fn;
// this module supplies the body for the 'html' target. It is a PURE
// function of its operand: no filesystem, no network, no state reach
// beyond the D it is handed (and it only reads that for nothing — the
// operand carries everything). It knows NOUNS and WIDGETS — never apps;
// if rendering an app ever requires touching this file, the §5.2 seam
// has leaked (that is the pb-zero-glue-acceptance criterion).
//
// Operand shape (built by `command::encode_render_input`, kept in
// lockstep by the unit tests here):
//
//   < <'view',        <view_id, kind, source>>,
//     <'entity',      <entity_id, noun>>,
//     <'elements',    <<ve_id, fact_type, component_role>, ...>>,
//     <'fields',      <<name, value>, ...>>            (name-sorted)
//     <'affordances', <<event, target_status, href>, ...>> >
//
// Output: Object::Atom(html) — one <form> per entity view, one labelled
// widget per ViewElement (Component Role → input kind, the same closed
// vocabulary readings/ui/view-projection.md resolves), one
// rel="transition" anchor per HATEOAS affordance. Object::Bottom on a
// malformed operand (apply() stays total).

use crate::ast::{self, Object};
use crate::sync::Arc;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Register the `render:html` body in `PLATFORM_FALLBACK`. Pre-approved
/// in `ast::APPROVED_PLATFORM_FN_NAMES` (sec-2 audit). Production wiring
/// calls this at CLI/MCP boot beside `install_rebuild_fns`; tests call
/// it directly.
pub fn install() {
    let f: ast::PlatformFn = Arc::new(|x: &Object, d: &Object| render_html_apply(x, d));
    ast::install_platform_fn("render:html", f);
}

/// `apply_platform` adapter for `"render:html"`.
fn render_html_apply(x: &Object, _d: &Object) -> Object {
    match render_html(x) {
        Some(html) => Object::atom(&html),
        None => Object::Bottom,
    }
}

/// Pure operand → markup. None on malformed input.
fn render_html(x: &Object) -> Option<String> {
    let sections = x.as_seq()?;
    let section = |tag: &str| -> Option<Vec<Object>> {
        sections.iter().find_map(|s| {
            let pair = s.as_seq()?;
            (pair.first()?.as_atom()? == tag).then(|| pair.get(1))??
                .as_seq().map(|v| v.to_vec())
        })
    };

    let view = section("view")?;
    let view_id = view.first()?.as_atom()?.to_string();
    let kind = view.get(1).and_then(|o| o.as_atom()).unwrap_or("instance").to_string();

    let entity = section("entity")?;
    let entity_id = entity.first()?.as_atom()?.to_string();
    let noun = entity.get(1).and_then(|o| o.as_atom()).unwrap_or("").to_string();
    let noun_prefix = format!("{}_has_", noun.replace(' ', "_"));

    // Field values by role name, for widget value lookup.
    let fields: Vec<(String, String)> = section("fields").map(|rows| {
        rows.iter().filter_map(|r| {
            let pair = r.as_seq()?;
            Some((pair.first()?.as_atom()?.to_string(),
                  pair.get(1)?.as_atom()?.to_string()))
        }).collect()
    }).unwrap_or_default();
    let value_of = |label: &str| -> String {
        fields.iter().find(|(k, _)| k == label)
            .map(|(_, v)| v.clone()).unwrap_or_default()
    };

    let mut html = String::new();
    html.push_str(&format!(
        "<form data-view=\"{}\" data-kind=\"{}\" data-entity=\"{}\">",
        esc(&view_id), esc(&kind), esc(&entity_id)));
    html.push_str(&format!("<h1>{}</h1>", esc(&entity_id)));

    // One labelled widget per ViewElement, in projection order. The
    // label is the rendered fact type's value-role name (noun prefix
    // stripped, underscores back to spaces) — the same join the fields
    // section is keyed by.
    for el in section("elements").unwrap_or_default() {
        let Some(parts) = el.as_seq() else { continue };
        let fact_type = match parts.get(1).and_then(|o| o.as_atom()) {
            Some(s) => s, None => continue,
        };
        let role = match parts.get(2).and_then(|o| o.as_atom()) {
            Some(s) => s, None => continue,
        };
        let label = fact_type.strip_prefix(noun_prefix.as_str())
            .unwrap_or(fact_type).replace('_', " ");
        let value = value_of(&label);
        let widget = match role {
            "text-input" => format!(
                "<input type=\"text\" name=\"{}\" value=\"{}\">",
                esc(&label), esc(&value)),
            "date-picker" => format!(
                "<input type=\"date\" name=\"{}\" value=\"{}\">",
                esc(&label), esc(&value)),
            "checkbox" => format!(
                "<input type=\"checkbox\" name=\"{}\"{}>",
                esc(&label),
                if value == "true" { " checked" } else { "" }),
            "combo-box" => format!(
                "<select name=\"{}\"><option selected>{}</option></select>",
                esc(&label), esc(&value)),
            // Unknown widget kinds degrade to readonly text — additive
            // vocabulary growth in components.md must not break targets.
            other => format!(
                "<input type=\"text\" name=\"{}\" value=\"{}\" readonly data-role=\"{}\">",
                esc(&label), esc(&value), esc(other)),
        };
        html.push_str(&format!("<label>{}{}</label>", esc(&label), widget));
    }

    // HATEOAS affordances: the legal transitions as rel=transition
    // anchors — the §5.2 link IS an unevaluated effect application.
    let affordances = section("affordances").unwrap_or_default();
    if !affordances.is_empty() {
        html.push_str("<nav>");
        for a in &affordances {
            let Some(parts) = a.as_seq() else { continue };
            let (Some(event), Some(href)) = (
                parts.first().and_then(|o| o.as_atom()),
                parts.get(2).and_then(|o| o.as_atom()),
            ) else { continue };
            html.push_str(&format!(
                "<a rel=\"transition\" href=\"{}\">{}</a>",
                esc(href), esc(event)));
        }
        html.push_str("</nav>");
    }

    html.push_str("</form>");
    Some(html)
}

/// Minimal HTML escaper for text + attribute positions.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command;

    fn sample_view() -> command::ViewProjection {
        command::ViewProjection {
            view: "instance-view-Task".to_string(),
            kind: "instance".to_string(),
            source: "synthesized".to_string(),
            elements: alloc::vec![
                command::ViewElementProjection {
                    id: "ve_1".to_string(),
                    fact_type: "Task_has_Task_Description".to_string(),
                    component_role: "text-input".to_string(),
                },
                command::ViewElementProjection {
                    id: "ve_2".to_string(),
                    fact_type: "Task_is_done".to_string(),
                    component_role: "checkbox".to_string(),
                },
            ],
            representations: Default::default(),
        }
    }

    fn sample_input() -> Object {
        let view = sample_view();
        let fields = [
            ("Task Description".to_string(), "fix <the> bug".to_string()),
            ("Task is done".to_string(), "true".to_string()),
        ];
        let affordances = alloc::vec![command::TransitionAction {
            event: "Task is started".to_string(),
            target_status: "in_progress".to_string(),
            method: "GET".to_string(),
            href: "/api/entities/Task/t1/transition?event=Task%20is%20started"
                .to_string(),
            component_role: None,
        }];
        command::encode_render_input(&view, "t1", "Task", &fields, &affordances)
    }

    /// The reference renderer derives widgets, labels, values, and
    /// affordances purely from the operand — and escapes markup-unsafe
    /// values.
    #[test]
    fn renders_widgets_values_and_affordances() {
        let html = render_html(&sample_input()).expect("well-formed operand renders");
        for needle in [
            "data-view=\"instance-view-Task\"",
            "data-entity=\"t1\"",
            "<label>Task Description<input type=\"text\" name=\"Task Description\" \
             value=\"fix &lt;the&gt; bug\"></label>",
            "<a rel=\"transition\" \
             href=\"/api/entities/Task/t1/transition?event=Task%20is%20started\">\
             Task is started</a>",
        ] {
            assert!(html.contains(needle), "missing {:?} in:\n{}", needle, html);
        }
        // Checkbox element: label is the full fact type (no Task_has_
        // prefix to strip), spaces restored, checked from the field value.
        assert!(html.contains("<input type=\"checkbox\" name=\"Task is done\" checked>"),
            "checkbox widget missing in:\n{}", html);
    }

    /// Malformed operands bottom out instead of panicking — apply() totality.
    #[test]
    fn malformed_operand_returns_bottom() {
        assert_eq!(render_html_apply(&Object::atom("nonsense"), &Object::Bottom),
            Object::Bottom);
        assert_eq!(render_html_apply(&Object::seq(alloc::vec![]), &Object::Bottom),
            Object::Bottom);
    }

    /// install() + dispatch through the real Platform-apply path: the
    /// operand round-trips encode → registry → markup. (No uninstall —
    /// the registry is process-global and lib tests run in parallel;
    /// the sec-2 audit lives in its own integration binary.)
    #[test]
    fn installed_body_dispatches_via_platform_apply() {
        install();
        let out = ast::apply(
            &ast::Func::Platform("render:html".to_string()),
            &sample_input(),
            &Object::Bottom,
        );
        let html = out.as_atom().expect("render:html returns an Atom");
        assert!(html.starts_with("<form") && html.ends_with("</form>"),
            "unexpected markup envelope: {}", html);
    }

    /// The §5.2 dispatch seam end-to-end: a `Render Target has Platform
    /// Function Name` fact + an installed body ⇒ `render_via_targets`
    /// returns the rendering keyed by target slug; a declared target
    /// with NO installed body is skipped, not an error.
    #[test]
    fn render_via_targets_walks_the_render_target_population() {
        install();
        let view = sample_view();
        let mut fields = hashbrown::HashMap::new();
        fields.insert("Task Description".to_string(), "fix it".to_string());
        let transitions: alloc::vec::Vec<command::TransitionAction> = alloc::vec![];

        // Two declared targets: 'html' (installed) + 'pdf' (no body).
        let d = ast::cell_push("Render_Target_has_Platform_Function_Name",
            ast::fact_from_pairs(&[
                ("Render Target", "html"),
                ("Platform Function Name", "render:html"),
            ]), &Object::phi());
        let d = ast::cell_push("Render_Target_has_Platform_Function_Name",
            ast::fact_from_pairs(&[
                ("Render Target", "pdf"),
                ("Platform Function Name", "render:pdf"),
            ]), &d);

        let reps = command::render_via_targets(
            &d, &view, "t1", "Task", &fields, &transitions);
        assert_eq!(reps.len(), 1, "only the installed target renders: {:?}",
            reps.keys().collect::<alloc::vec::Vec<_>>());
        let html = reps.get("html").expect("'html' target keyed by slug");
        assert!(html.contains("value=\"fix it\""),
            "field value missing from dispatched rendering:\n{}", html);
    }
}
