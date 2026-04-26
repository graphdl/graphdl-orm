// crates/arest-kernel/src/ui_apps/help.rs
//
// Help derived from the current screen (#517, EPIC #496).
//
// EEEEE's #516 landed the breadcrumb so the user always knows WHERE
// they are. The dual is WHAT they can do here. SSSS-2's #510 / VVVV's
// #511 / ZZZZ's #512 / BBBBB's #513 / IIIII's #514 already render the
// action surface and navigation surface as clickable rows on the right
// pane — clicking dispatches the SYSTEM verb, hovering shows tooltips.
//
// What this module adds: the same answer in *prose*, addressable from
// the REPL prompt by typing `help` (or `?`). Rather than duplicate the
// concrete action / navigation lists already visible on screen, the
// help text explains the *kind* of screen the user is on (Root vs.
// Noun vs. Instance vs. FactCell vs. ComponentInstance), the FORML
// concepts behind each kind, and the navigation verbs that move
// between them.
//
// # Conceptual model — every help body IS a derivation
//
// The help text for a `CurrentCell` is a pure function of the cell's
// variant — no `&Object` access required for the first pass. This is
// intentional: the answer to "what kind of thing is this and what can
// I do with it" depends on the FORML metamodel, not on the contents of
// any particular tenant state. State-aware enrichment (verbalising the
// concrete action labels into "transition this Order from In Cart to
// Placed by clicking 'place'") is a follow-up; the foundation slice
// here gets the introspection affordance discoverable from the REPL.
//
// # Output shape
//
// `screen_help(cell)` returns a `Vec<String>` of lines ready to push
// into the REPL scrollback via `push_response`. Each line is a
// pre-rendered pure-text string; no Slint surface changes required.
// `unified_repl::submit` intercepts `help` / `?` lines and pushes the
// returned lines.
//
// # Why a separate module
//
// The classification logic mirrors the shape of `cell_renderer`,
// `actions`, and `navigation` — pure function over `&CurrentCell` (and
// optionally `&Object`) returning owned data. Inlining it into
// `unified_repl` would crowd out the dispatch glue. The four ui_apps
// modules each answer one HATEOAS question: where am I (breadcrumb),
// what's here (cell_renderer), what can I do (actions), where can I go
// (navigation), and now — what does this all mean (help).

#![allow(dead_code)]

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::ui_apps::cell_renderer::CurrentCell;

/// Help text for the current screen, returned as one line per `Vec<String>`
/// entry. The unified REPL pushes each line through `push_response` so
/// scrollback wrapping behaves identically to other multi-line replies
/// (verify, validate, def, etc.).
///
/// First pass is variant-only — the cell's identifier components (noun
/// name, instance id, cell name) are formatted into the prose so the
/// help is concrete enough to act on, but the engine state itself is
/// not consulted. State-aware enrichment (resolving SM cells to their
/// transition lists, verbalising guard conditions) is a follow-up.
pub fn screen_help(cell: &CurrentCell) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    push_header(&mut lines, cell);
    push_what(&mut lines, cell);
    push_can_do(&mut lines, cell);
    push_can_go(&mut lines, cell);
    push_repl_commands(&mut lines);
    lines
}

/// Top-of-help header — names the cell so the user can correlate the
/// help with the breadcrumb.
fn push_header(lines: &mut Vec<String>, cell: &CurrentCell) {
    let label = cell.label();
    lines.push(alloc::format!("─── help: {label} ───"));
    lines.push(String::new());
}

/// "What this is" section — explains the cell variant in FORML terms.
fn push_what(lines: &mut Vec<String>, cell: &CurrentCell) {
    lines.push("What this is:".to_string());
    match cell {
        CurrentCell::Root => {
            lines.push(
                "  The root screen lists every Noun the system knows — entity types and"
                    .to_string(),
            );
            lines.push(
                "  any FORML cell namespace registered through the metamodel. Pick a Noun"
                    .to_string(),
            );
            lines.push("  to see its instances; pick `cell` to walk a named SYSTEM cell directly.".to_string());
        }
        CurrentCell::Noun { noun } => {
            lines.push(alloc::format!(
                "  Noun '{noun}' — a fact-type backbone. Every cell of the form"
            ));
            lines.push(alloc::format!(
                "  `{noun}_has_<Attr>` participates in this Noun. Listing it shows every"
            ));
            lines.push(
                "  instance plus the FactType graph rooted at the Noun's primary key."
                    .to_string(),
            );
        }
        CurrentCell::Instance { noun, instance } => {
            lines.push(alloc::format!(
                "  Instance '{instance}' of Noun '{noun}' — one row in the conceptual table."
            ));
            lines.push(
                "  Bindings shown on the typed-surface area are this instance's facts;"
                    .to_string(),
            );
            lines.push(
                "  back-references to other instances surface as navigation rows."
                    .to_string(),
            );
        }
        CurrentCell::FactCell { cell_name } => {
            lines.push(alloc::format!(
                "  FactCell '{cell_name}' — a directly-named SYSTEM cell. May be a fact-"
            ));
            lines.push(
                "  type cell, a derivation cell, an audit log, or any kernel-internal"
                    .to_string(),
            );
            lines.push(
                "  cell that `cells_iter` exposes. Renders as the raw fact list."
                    .to_string(),
            );
        }
        CurrentCell::ComponentInstance { component_id } => {
            lines.push(alloc::format!(
                "  Component '{component_id}' — a live widget instance from the Component"
            ));
            lines.push(
                "  registry (#491). Shows the Component's declared properties and the"
                    .to_string(),
            );
            lines.push(
                "  toolkit binding (Slint / Qt / GTK / Web) currently driving it."
                    .to_string(),
            );
        }
    }
    lines.push(String::new());
}

/// "What you can do" section — points at the action panel + lists
/// the verb categories applicable to this cell type.
fn push_can_do(lines: &mut Vec<String>, cell: &CurrentCell) {
    lines.push("What you can do here:".to_string());
    lines.push(
        "  Concrete actions are clickable rows on the right pane (action surface)."
            .to_string(),
    );
    lines.push("  By cell type, the verbs that can apply:".to_string());
    match cell {
        CurrentCell::Root => {
            lines.push(
                "    apply create <Noun>            — instantiate a new Noun"
                    .to_string(),
            );
            lines.push(
                "    def <Name>                     — introspect a definition"
                    .to_string(),
            );
        }
        CurrentCell::Noun { .. } => {
            lines.push(
                "    apply create <Noun>            — open a new instance form"
                    .to_string(),
            );
            lines.push(
                "    apply destroy <Noun>::<id>     — drop an instance"
                    .to_string(),
            );
            lines.push(
                "    fetch <Noun>_has_<Attr>        — inspect any FactType cell"
                    .to_string(),
            );
        }
        CurrentCell::Instance { .. } => {
            lines.push(
                "    apply update <Noun>::<id>      — edit this instance"
                    .to_string(),
            );
            lines.push(
                "    apply destroy <Noun>::<id>     — drop this instance"
                    .to_string(),
            );
            lines.push(
                "    transition <SM>::<id> <event>  — fire a state-machine transition"
                    .to_string(),
            );
        }
        CurrentCell::FactCell { .. } => {
            lines.push(
                "    apply remove fact <cell>#<id>  — drop one fact from the cell"
                    .to_string(),
            );
            lines.push(
                "    fetch <cell>                   — inspect raw contents"
                    .to_string(),
            );
            lines.push(
                "    store <cell> <contents>        — overwrite the cell"
                    .to_string(),
            );
        }
        CurrentCell::ComponentInstance { .. } => {
            lines.push(
                "    apply update Component_property <name>"
                    .to_string(),
            );
            lines.push(
                "                                   — edit a declared property"
                    .to_string(),
            );
        }
    }
    lines.push(String::new());
}

/// "Where you can go" section — explains the navigation surface.
fn push_can_go(lines: &mut Vec<String>, cell: &CurrentCell) {
    lines.push("Where you can go from here:".to_string());
    lines.push(
        "  Navigation rows on the right pane are HATEOAS pointers to other cells."
            .to_string(),
    );
    match cell {
        CurrentCell::Root => {
            lines.push(
                "  From Root: every known Noun, plus any cell visible to `cells_iter`."
                    .to_string(),
            );
        }
        CurrentCell::Noun { .. } => {
            lines.push(
                "  From a Noun: instances of the Noun, FactType cells the Noun appears in,"
                    .to_string(),
            );
            lines.push(
                "  and join-reachable Nouns (any Noun that shares a FactType with this one)."
                    .to_string(),
            );
        }
        CurrentCell::Instance { .. } => {
            lines.push(
                "  From an Instance: role-joined instances (related rows), derivation antecedents"
                    .to_string(),
            );
            lines.push(
                "  and consequents (cells this instance feeds into), and any state-machine"
                    .to_string(),
            );
            lines.push(
                "  cell tracking this instance's status."
                    .to_string(),
            );
        }
        CurrentCell::FactCell { .. } => {
            lines.push(
                "  From a FactCell: the Noun whose FactType this cell projects, and any"
                    .to_string(),
            );
            lines.push(
                "  derived cells that consume this one in their consequent rule body."
                    .to_string(),
            );
        }
        CurrentCell::ComponentInstance { .. } => {
            lines.push(
                "  From a Component: its parent Container, sibling Components, and the"
                    .to_string(),
            );
            lines.push(
                "  Toolkit cell holding the binding."
                    .to_string(),
            );
        }
    }
    lines.push(String::new());
}

/// "REPL navigation commands" section — same parser the unified REPL
/// already exposes via `parse_cell_nav` (#511). Listed here so a user
/// who reaches the help screen can move between cell types without
/// hunting for the syntax in the source comments.
fn push_repl_commands(lines: &mut Vec<String>) {
    lines.push("REPL navigation commands:".to_string());
    lines.push(
        "  home                               — return to Root".to_string(),
    );
    lines.push(
        "  noun <Noun>                        — jump to a Noun screen".to_string(),
    );
    lines.push(
        "  instance <Noun> <Id>               — jump to an Instance screen".to_string(),
    );
    lines.push(
        "  cell <CellName>                    — jump to a directly-named SYSTEM cell".to_string(),
    );
    lines.push(
        "  component <ComponentId>            — jump to a Component instance".to_string(),
    );
    lines.push(
        "  help | ?                           — this screen".to_string(),
    );
    lines.push(String::new());
    lines.push(
        "  Plus every legacy REPL verb (heap, quit, …) flows through unchanged."
            .to_string(),
    );
}

// ── Tests ────────────────────────────────────────────────────────────
//
// Pure-function tests — no state needed. Asserts that each cell
// variant produces a help body that mentions the variant-relevant
// concepts (Noun name, instance id, transition / update / fetch
// verbs as applicable). Smoke-level only; the body text is allowed
// to change as long as the structural invariants hold.

#[cfg(test)]
mod tests {
    use super::*;

    fn body(cell: &CurrentCell) -> String {
        screen_help(cell).join("\n")
    }

    #[test]
    fn root_help_mentions_noun_navigation() {
        let blob = body(&CurrentCell::Root);
        assert!(blob.contains("Noun"), "root help missing Noun mention: {blob}");
        assert!(
            blob.contains("apply create"),
            "root help missing apply create verb: {blob}"
        );
        assert!(blob.contains("home"), "root help missing home command: {blob}");
    }

    #[test]
    fn noun_help_carries_noun_name_and_create_verb() {
        let blob = body(&CurrentCell::Noun { noun: "Order".to_string() });
        assert!(blob.contains("Order"), "noun help missing noun name: {blob}");
        assert!(
            blob.contains("apply create Order")
                || blob.contains("apply create <Noun>"),
            "noun help missing create verb: {blob}"
        );
    }

    #[test]
    fn instance_help_carries_ids_and_transition_verb() {
        let blob = body(&CurrentCell::Instance {
            noun: "Order".to_string(),
            instance: "ord-1".to_string(),
        });
        assert!(blob.contains("Order"), "instance help missing noun name: {blob}");
        assert!(blob.contains("ord-1"), "instance help missing instance id: {blob}");
        assert!(
            blob.contains("transition"),
            "instance help missing transition verb: {blob}"
        );
        assert!(
            blob.contains("apply update"),
            "instance help missing update verb: {blob}"
        );
    }

    #[test]
    fn fact_cell_help_names_the_cell_and_lists_remove_verb() {
        let blob = body(&CurrentCell::FactCell {
            cell_name: "Order_has_Customer".to_string(),
        });
        assert!(
            blob.contains("Order_has_Customer"),
            "factcell help missing cell name: {blob}"
        );
        assert!(
            blob.contains("apply remove fact"),
            "factcell help missing remove-fact verb: {blob}"
        );
    }

    #[test]
    fn component_help_names_the_component_and_lists_update_property() {
        let blob = body(&CurrentCell::ComponentInstance {
            component_id: "btn-1".to_string(),
        });
        assert!(blob.contains("btn-1"), "component help missing id: {blob}");
        assert!(
            blob.contains("Component_property") || blob.contains("update"),
            "component help missing property update verb: {blob}"
        );
    }

    #[test]
    fn every_cell_variant_emits_at_least_the_four_canonical_sections() {
        // Smoke — each help body should carry the four section headers
        // so the structural shape is consistent regardless of variant.
        for cell in [
            CurrentCell::Root,
            CurrentCell::Noun { noun: "X".to_string() },
            CurrentCell::Instance {
                noun: "X".to_string(),
                instance: "y".to_string(),
            },
            CurrentCell::FactCell {
                cell_name: "X_has_Y".to_string(),
            },
            CurrentCell::ComponentInstance {
                component_id: "c".to_string(),
            },
        ] {
            let blob = body(&cell);
            assert!(blob.contains("What this is:"), "missing 'What this is:' on {cell:?}");
            assert!(
                blob.contains("What you can do here:"),
                "missing 'What you can do here:' on {cell:?}"
            );
            assert!(
                blob.contains("Where you can go from here:"),
                "missing 'Where you can go from here:' on {cell:?}"
            );
            assert!(
                blob.contains("REPL navigation commands:"),
                "missing 'REPL navigation commands:' on {cell:?}"
            );
        }
    }
}
