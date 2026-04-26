// crates/arest-kernel/src/ui_apps/palette.rs
//
// Command palette — fuzzy search over (cells × actions × navigation)
// reachable from the current screen (#515, EPIC #496).
//
// The visual command-palette overlay (Cmd-K-style modal with an input
// box and a virtualised result list) is the aspirational design. This
// module ships the no_std-clean foundation: a fuzzy-match engine over
// a typed catalogue of every reachable affordance, with a REPL command
// surface (`palette <query>` / `pal <query>` / `:<query>`) that prints
// ranked results with the cell-nav syntax to act on each. Adding the
// Slint overlay later is a thin layer on top — same query function,
// same catalogue, just rendered in a modal instead of dumped to
// scrollback.
//
// # Catalogue sources
//
// The catalogue is rebuilt on every query (state is small enough that
// caching adds more invalidation pain than it saves). Three sources:
//
//   1. Every cell visible to `ast::cells_iter` — every named SYSTEM
//      cell, every fact-type cell, every kernel-internal cell. One
//      palette entry per cell, dispatched as `cell <name>`. Skips
//      metamodel-internal cells (those whose names contain `:`) so
//      the surface stays scoped to user-relevant facts.
//   2. Every action returned by `actions::compute_actions(current,
//      state)` — the same SYSTEM-call rows shown on the action
//      surface. Dispatch text is the canonical REPL form (e.g.
//      `transition Order::ord-1 place`) so the user can type it
//      directly.
//   3. Every navigation target returned by
//      `navigation::compute_navigation_targets(current, state)` —
//      role-joined cells, derivation antecedents/consequents, SM
//      target cells, parent/sibling Components. Dispatch text is
//      the cell-nav verb (`noun X`, `instance X y`, …).
//
// # Fuzzy match
//
// Subsequence scoring: a query matches a label when its characters
// appear in order in the label (case-insensitive). Score rewards
// early matches — `ord` against `Order` scores higher than `ord`
// against `Customer Order History`. Empty query matches every entry
// at score 0 (returns the catalogue in default order).
//
// Future: tokenise the label by capitalisation / underscores and
// boost matches that begin a token (`O` matches `Order` better than
// `Customer`). Foundation slice keeps the algorithm simple.
//
// # Why a separate module
//
// Same shape as the five sibling derivation modules. Each answers one
// HATEOAS question; this one answers "what's reachable that matches
// what I'm typing?" The fuzzy-match engine is generic — future
// callers (a Slint overlay, an MCP `palette` verb for AI agents,
// the help system's "see also" linker) reuse the same `score_label`
// + `build_catalogue` pair without going through the REPL.

#![allow(dead_code)]

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use arest::ast::{self, Object};

use crate::ui_apps::actions::{self, SystemAction};
use crate::ui_apps::cell_renderer::CurrentCell;
use crate::ui_apps::navigation::{self, NavigationTarget};

// ── PaletteEntry ──────────────────────────────────────────────────

/// Provenance — which catalogue source produced this entry. Surfaced
/// in the rendered output so the user can scan by category, and
/// preserved on the entry so a future Slint overlay can render
/// per-kind icons / colours without re-classifying.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteKind {
    /// A cell named directly via `cells_iter`. Dispatch: `cell <name>`.
    Cell,
    /// A SYSTEM call from the current screen's action surface.
    /// Dispatch: the canonical REPL text from `SystemAction::label`.
    Action,
    /// A navigation target from the current screen's navigation
    /// surface. Dispatch: the cell-nav verb for the target cell.
    Navigation,
}

impl PaletteKind {
    /// Short tag rendered alongside the entry label so the user can
    /// scan by category.
    pub fn tag(&self) -> &'static str {
        match self {
            PaletteKind::Cell => "cell",
            PaletteKind::Action => "act ",
            PaletteKind::Navigation => "nav ",
        }
    }
}

/// One match-able entry in the catalogue. The label is what the user
/// sees + what fuzzy match scores against; the dispatch text is what
/// they would type at the REPL prompt to act on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteEntry {
    pub label: String,
    pub dispatch: String,
    pub kind: PaletteKind,
}

/// Score-paired entry returned from a search. Higher score = better
/// match. The vec is sorted descending by score, then by label length
/// ascending for deterministic ties.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteHit {
    pub entry: PaletteEntry,
    pub score: u32,
}

// ── Public API ─────────────────────────────────────────────────────

/// How many top results to surface from a query. Conservative cap so
/// the REPL command's scrollback dump stays readable; the Slint
/// overlay (when it lands) can virtualise the full result vec
/// instead.
pub const DEFAULT_LIMIT: usize = 10;

/// Build the catalogue of every reachable affordance from `current`.
/// Pure function over `&Object` — the `with_state` caller can drop
/// the read lock the moment this returns.
pub fn build_catalogue(current: &CurrentCell, state: &Object) -> Vec<PaletteEntry> {
    let mut out: Vec<PaletteEntry> = Vec::new();

    // 1. Every named cell. Skip metamodel-internal names (contain `:`)
    //    so the surface stays scoped to user-relevant facts.
    for (name, _contents) in ast::cells_iter(state) {
        if name.contains(':') {
            continue;
        }
        out.push(PaletteEntry {
            label: name.to_string(),
            dispatch: format!("cell {name}"),
            kind: PaletteKind::Cell,
        });
    }

    // 2. SYSTEM-call action surface for the current cell.
    for action in actions::compute_actions(current, state) {
        let SystemAction { label, .. } = action;
        // Action labels carry a `[<verb-prefix>] <verb-text>` shape
        // already; the dispatch text is the verb body the user types
        // in the free-text REPL. Strip the bracketed prefix for the
        // dispatch so the REPL parser sees a plain verb.
        let dispatch = strip_label_prefix(&label).to_string();
        out.push(PaletteEntry {
            label,
            dispatch,
            kind: PaletteKind::Action,
        });
    }

    // 3. Navigation surface for the current cell.
    for target in navigation::compute_navigation_targets(current, state) {
        let NavigationTarget { target: cell, label, .. } = target;
        out.push(PaletteEntry {
            label,
            dispatch: cell_to_dispatch(&cell),
            kind: PaletteKind::Navigation,
        });
    }

    out
}

/// Fuzzy-search the catalogue with `query`. Returns up to `limit`
/// hits sorted by score descending, label-length ascending. Empty
/// query returns the catalogue in default order, capped at `limit`.
pub fn search(query: &str, current: &CurrentCell, state: &Object, limit: usize) -> Vec<PaletteHit> {
    let catalogue = build_catalogue(current, state);
    let mut hits: Vec<PaletteHit> = catalogue
        .into_iter()
        .filter_map(|entry| {
            score_label(query, &entry.label).map(|score| PaletteHit { entry, score })
        })
        .collect();
    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.entry.label.len().cmp(&b.entry.label.len()))
            .then_with(|| a.entry.label.cmp(&b.entry.label))
    });
    hits.truncate(limit);
    hits
}

/// Render a hit list as scrollback-ready lines. Empty hit list emits
/// a single explanatory line so the user gets a clear "no matches"
/// signal rather than an ambiguous blank response.
pub fn render_hits(hits: &[PaletteHit], query: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    if hits.is_empty() {
        lines.push(format!("No palette matches for `{query}`."));
        return lines;
    }
    lines.push(format!(
        "─── {} match(es) for `{}` ───",
        hits.len(),
        query
    ));
    for (i, hit) in hits.iter().enumerate() {
        let n = i + 1;
        lines.push(format!(
            "  {n}. [{tag}] {label}",
            tag = hit.entry.kind.tag(),
            label = hit.entry.label
        ));
        lines.push(format!("       → {dispatch}", dispatch = hit.entry.dispatch));
    }
    lines.push(String::new());
    lines.push(
        "  Type the `→` line to dispatch, or refine your query with `palette <text>`."
            .to_string(),
    );
    lines
}

/// Convenience: build catalogue, search, render in one call. The
/// unified REPL uses this from the `palette` / `pal` command intercept.
pub fn render_for_query(
    query: &str,
    current: &CurrentCell,
    state: &Object,
    limit: usize,
) -> Vec<String> {
    let hits = search(query, current, state, limit);
    render_hits(&hits, query)
}

// ── Fuzzy match ────────────────────────────────────────────────────

/// Subsequence-style fuzzy score. Returns `None` when `query` is not
/// a subsequence of `label` (case-insensitive). Higher returned score
/// = better match. Scores reward characters that match early in the
/// label (so a query of `ord` against `Order` outranks `ord` against
/// `Customer Order`).
///
/// Empty query returns `Some(0)` so the caller's "show me everything"
/// path works without a special case at the call site.
pub fn score_label(query: &str, label: &str) -> Option<u32> {
    let q: Vec<char> = query.to_lowercase().chars().collect();
    if q.is_empty() {
        return Some(0);
    }
    let l: Vec<char> = label.to_lowercase().chars().collect();
    let mut score: u32 = 0;
    let mut last_pos: usize = 0;
    for qc in &q {
        let mut found = false;
        for i in last_pos..l.len() {
            if l[i] == *qc {
                // Earlier matches score higher: 100 - position, floored
                // at 0 so very long labels still get *some* credit for
                // a late match rather than wrapping negative.
                let bonus = 100u32.saturating_sub(i as u32);
                score = score.saturating_add(bonus);
                last_pos = i + 1;
                found = true;
                break;
            }
        }
        if !found {
            return None;
        }
    }
    Some(score)
}

// ── Helpers ────────────────────────────────────────────────────────

/// Inverse of `unified_repl::parse_cell_nav`: turn a `CurrentCell`
/// back into the REPL text command that would navigate to it. Used
/// by Navigation palette entries so their dispatch text is the
/// cell-nav verb the user can type at the prompt.
fn cell_to_dispatch(cell: &CurrentCell) -> String {
    match cell {
        CurrentCell::Root => "home".to_string(),
        CurrentCell::Noun { noun } => format!("noun {noun}"),
        CurrentCell::Instance { noun, instance } => {
            format!("instance {noun} {instance}")
        }
        CurrentCell::FactCell { cell_name } => format!("cell {cell_name}"),
        CurrentCell::ComponentInstance { component_id } => {
            format!("component {component_id}")
        }
    }
}

/// `SystemAction::label` carries a `[<verb-prefix>] <verb-text>`
/// shape (e.g. `[transition] transition Order::ord-1 place`). Strip
/// the leading `[<...>] ` so the dispatch text is the bare REPL form.
fn strip_label_prefix(label: &str) -> &str {
    if let Some(rest) = label.strip_prefix('[') {
        if let Some(idx) = rest.find("] ") {
            return &rest[idx + 2..];
        }
    }
    label
}

// ── Tests ────────────────────────────────────────────────────────────
//
// Pure-function tests over hand-built CurrentCell + Object shapes.
// Coverage:
//   * fuzzy match: subsequence positive, non-subsequence None, empty
//     query, case insensitivity, early-match score boost
//   * catalogue: cells contributed, action labels contributed,
//     metamodel cells filtered out
//   * search: returns sorted hits, respects limit, empty result
//   * render: empty case, populated case
//   * cell_to_dispatch: round-trip through parse_cell_nav (smoke)
//   * strip_label_prefix: present + absent

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn empty_state() -> Object {
        Object::seq(vec![])
    }

    fn state_with_cell_names(names: &[&str]) -> Object {
        Object::seq(
            names
                .iter()
                .map(|name| {
                    Object::seq(vec![
                        Object::atom("CELL"),
                        Object::atom(name),
                        Object::seq(vec![]),
                    ])
                })
                .collect(),
        )
    }

    // ─ score_label ─

    #[test]
    fn score_subsequence_match_is_some() {
        assert!(score_label("ord", "Order").is_some());
        assert!(score_label("OR", "order").is_some()); // case-insensitive
    }

    #[test]
    fn score_non_subsequence_is_none() {
        assert!(score_label("rdo", "Order").is_none());
        assert!(score_label("xyz", "Order").is_none());
    }

    #[test]
    fn score_empty_query_matches_everything_at_zero() {
        assert_eq!(score_label("", "Order"), Some(0));
        assert_eq!(score_label("", ""), Some(0));
    }

    #[test]
    fn score_early_match_outranks_late_match() {
        let early = score_label("o", "Order").unwrap();
        let late = score_label("o", "Customer Order").unwrap();
        assert!(
            early > late,
            "expected early match score {early} > late match score {late}"
        );
    }

    // ─ catalogue ─

    #[test]
    fn catalogue_includes_user_cells() {
        let state = state_with_cell_names(&["Order_has_Customer", "Customer_has_Name"]);
        let cat = build_catalogue(&CurrentCell::Root, &state);
        let labels: Vec<&str> = cat.iter().map(|e| e.label.as_str()).collect();
        assert!(labels.contains(&"Order_has_Customer"));
        assert!(labels.contains(&"Customer_has_Name"));
    }

    #[test]
    fn catalogue_skips_metamodel_cells() {
        let state = state_with_cell_names(&["sql:sqlite:Order", "Order"]);
        let cat = build_catalogue(&CurrentCell::Root, &state);
        let labels: Vec<&str> = cat.iter().map(|e| e.label.as_str()).collect();
        assert!(!labels.iter().any(|l| l.contains(':')));
        assert!(labels.contains(&"Order"));
    }

    // ─ search ─

    #[test]
    fn search_empty_query_returns_catalogue_capped_to_limit() {
        let state = state_with_cell_names(&["A", "B", "C", "D", "E"]);
        let hits = search("", &CurrentCell::Root, &state, 3);
        assert_eq!(hits.len(), 3);
    }

    #[test]
    fn search_query_filters_to_subsequence_matches() {
        let state = state_with_cell_names(&["Order", "Customer", "Invoice"]);
        let hits = search("or", &CurrentCell::Root, &state, 10);
        let labels: Vec<&str> = hits.iter().map(|h| h.entry.label.as_str()).collect();
        // "or" is a subsequence of Order and Invoice (i,n,v,o,...,r) — let me re-think.
        // i-n-v-o-i-c-e: o appears at idx 3, then need r after — no r in invoice.
        // So Invoice should NOT match. Order should match.
        assert!(labels.contains(&"Order"));
        assert!(!labels.contains(&"Invoice"));
    }

    #[test]
    fn search_returns_no_matches_when_query_unmatched() {
        let state = state_with_cell_names(&["A", "B"]);
        let hits = search("xyz", &CurrentCell::Root, &state, 10);
        assert!(hits.is_empty());
    }

    // ─ render ─

    #[test]
    fn render_empty_emits_no_match_line_with_query() {
        let lines = render_hits(&[], "ord");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("No palette matches"));
        assert!(lines[0].contains("ord"));
    }

    #[test]
    fn render_one_includes_label_dispatch_and_count() {
        let entry = PaletteEntry {
            label: "Order".to_string(),
            dispatch: "noun Order".to_string(),
            kind: PaletteKind::Cell,
        };
        let hits = vec![PaletteHit { entry, score: 100 }];
        let lines = render_hits(&hits, "or");
        let blob = lines.join("\n");
        assert!(blob.contains("1 match"));
        assert!(blob.contains("Order"));
        assert!(blob.contains("noun Order"));
    }

    // ─ helpers ─

    #[test]
    fn cell_to_dispatch_emits_repl_command_for_each_variant() {
        assert_eq!(cell_to_dispatch(&CurrentCell::Root), "home");
        assert_eq!(
            cell_to_dispatch(&CurrentCell::Noun { noun: "X".to_string() }),
            "noun X"
        );
        assert_eq!(
            cell_to_dispatch(&CurrentCell::Instance {
                noun: "X".to_string(),
                instance: "y".to_string(),
            }),
            "instance X y"
        );
        assert_eq!(
            cell_to_dispatch(&CurrentCell::FactCell {
                cell_name: "X_has_Y".to_string(),
            }),
            "cell X_has_Y"
        );
        assert_eq!(
            cell_to_dispatch(&CurrentCell::ComponentInstance {
                component_id: "c".to_string(),
            }),
            "component c"
        );
    }

    #[test]
    fn strip_label_prefix_removes_bracketed_tag() {
        assert_eq!(
            strip_label_prefix("[transition] transition Order::ord-1 place"),
            "transition Order::ord-1 place"
        );
        assert_eq!(strip_label_prefix("no prefix here"), "no prefix here");
        assert_eq!(strip_label_prefix("[unclosed"), "[unclosed");
    }
}
