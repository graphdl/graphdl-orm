// crates/arest-kernel/src/ui_apps/unified_repl.rs
//
// Unified REPL — Rust glue (#510, EPIC #496).
//
// Foundational structural merge of the previous Track SSS #429
// (`hateoas.rs`) + Track TTT #430 (`repl.rs`) modules into a single
// app. The Slint surface (`ui/apps/UnifiedRepl.slint`) declares both
// pane's properties + callbacks on one Window; this module wires
// them all to a single shared `Rc<RefCell<UnifiedReplState>>`.
//
// THIS commit lands ONLY the structural merge: the left pane drives
// the same HATEOAS resource browsing the prior `hateoas.rs` did, and
// the right pane drives the same REPL scrollback + history line
// editor `repl.rs` did. The follow-up sub-tasks (#511 cell-as-screen,
// #512 navigation actions, #513 SYSTEM-as-actions, …) layer the
// cross-pane behaviours on top — those are NOT done here.
//
// State model (all in one `UnifiedReplState`):
//
//   * `nav_stack: Vec<Breadcrumb>` — HATEOAS navigation, mirrors
//     the prior `hateoas::BrowserState::stack`.
//   * `subscriber_id: Option<SubscriberId>` — `system::subscribe_changes`
//     handle for live updates (HATEOAS pane). Mirrors prior glue.
//   * `scrollback: Vec<String>` — REPL scrollback, capped at
//     `SCROLLBACK_MAX`. Mirrors prior `repl::ReplState::scrollback`.
//   * `history: Vec<String>` — REPL command history.
//   * `history_idx: Option<usize>` — Up/Down browse cursor.
//   * `pending_input: String` — snapshot of in-progress text on first
//     Up press, restored when Down walks off the end.
//
// The Rust state lives behind a `Rc<RefCell<...>>` shared by every
// callback closure. The kernel's `unsafe-single-threaded` slint
// feature (Cargo.toml L205) makes the lack of `Send` on `Rc` /
// `RefCell` sound — boot is single-threaded.
//
// Wiring vs. invocation: `crate::ui_apps::launcher` constructs this
// app via `build_app()` and shows / hides its Window. Until the
// launcher swaps to it, this module is dormant from the boot flow's
// perspective (smoke-test-callable through the constructor only).

#![allow(dead_code)]

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

use arest::ast::{self, Object};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::arch::uefi::slint_backend::UnifiedRepl;
use crate::system::SubscriberId;
use crate::ui_apps::actions::{self, SystemAction};
use crate::ui_apps::breadcrumb::{BreadcrumbState, CrumbEntry};
use crate::ui_apps::cell_renderer::{self, CurrentCell, RenderedScreen};
use crate::ui_apps::navigation::NavigationTarget;

/// Convenience alias — every Slint `[string]` property is bridged
/// through a `ModelRc<SharedString>` backed by a `VecModel`. Same
/// alias used by the prior `hateoas` and `repl` modules.
type StringModel = ModelRc<SharedString>;

/// Hard ceiling on `scrollback` length. Mirrors the prior
/// `repl::SCROLLBACK_MAX` (`1000`) — at ~20px per rendered line, this
/// is enough scrollback for hours of interaction without unbounded
/// memory growth.
const SCROLLBACK_MAX: usize = 1000;

/// First line shown in the scrollback panel before the user types.
/// Updated for the unified panel — the user sees both the REPL and
/// the HATEOAS browse on first launch, so the welcome calls them out.
const WELCOME: &str =
    "AREST Unified REPL — type `help` for commands; navigate the system on the left.";

// ── Breadcrumb (same shape as the prior hateoas.rs) ────────────────

/// One step in the HATEOAS navigation trail. Each forward click
/// pushes one of these; the back button pops.
#[derive(Debug, Clone)]
enum Breadcrumb {
    /// Root — initial "Resources" landing. Always sits at index 0.
    Root,
    /// User picked a Noun from the sidebar.
    Noun { noun: String },
    /// User picked an instance from the centre column.
    Instance { noun: String, instance: String },
}

impl Breadcrumb {
    /// Display label for the breadcrumb bar.
    fn label(&self) -> String {
        match self {
            Breadcrumb::Root => "Resources".to_string(),
            Breadcrumb::Noun { noun } => noun.clone(),
            Breadcrumb::Instance { instance, .. } => instance.clone(),
        }
    }
}

// ── Unified state ──────────────────────────────────────────────────

/// Mutable state shared between every Slint callback closure.
struct UnifiedReplState {
    // HATEOAS-side fields (mirror the prior `hateoas::BrowserState`).
    /// Navigation stack. Always non-empty; index 0 is `Root`.
    nav_stack: Vec<Breadcrumb>,
    /// `system::subscribe_changes` handle. `None` until `build_app`
    /// finishes registering the live-update handler. Drop unsubscribes.
    subscriber_id: Option<SubscriberId>,

    // REPL-side fields (mirror the prior `repl::ReplState`).
    /// Visible scrollback. Each entry is one rendered line.
    scrollback: Vec<String>,
    /// Command history, oldest first.
    history: Vec<String>,
    /// `None` = user is editing a fresh line; `Some(i)` = the input
    /// field currently shows `history[i]`.
    history_idx: Option<usize>,
    /// Snapshot of in-progress input at the moment the user first
    /// pressed Up. Restored when Down walks past the newest entry.
    pending_input: String,

    // Cell-as-screen field (#511, EPIC #496). Tracks the cell the
    // user is currently looking at. Every screen IS a cell — the
    // typed-surface area on the right pane reads this to decide which
    // Component to surface (via `cell_renderer::select_component_for`)
    // and which data to project. Mirrors the breadcrumb model
    // `nav_stack` already maintains, but typed: nav_stack is the
    // breadcrumb trail; current_cell is the live target.
    current_cell: CurrentCell,

    // Navigation actions as cells (#512, EPIC #496). Cache of the most
    // recently rendered navigation catalogue, indexed by the same
    // ordering the Slint surface displays. The click handler reads
    // `nav_targets[idx]` to translate a click into a cell-jump.
    // Recomputed on every `redraw` so live cell-graph updates are
    // reflected in subsequent click handling.
    nav_targets: Vec<NavigationTarget>,

    // SYSTEM calls as actions on current screen (#513, EPIC #496).
    // Cache of the most recently rendered action catalogue, indexed
    // by the same ordering the Slint surface displays. The click
    // handler reads `system_actions[idx]` and dispatches the SYSTEM
    // verb with `default_args` pre-bound from the cell context.
    // Recomputed on every `redraw` so live cell-graph updates are
    // reflected in subsequent click handling — same shape as
    // `nav_targets` above.
    system_actions: Vec<SystemAction>,

    // "You are here" breadcrumb + back/forward navigation history
    // (#516, EPIC #496). The path through the cell graph IS itself a
    // sequence of cells; this state captures it for the persistent
    // breadcrumb strip across the top of the panel + the back /
    // forward buttons + the Bookmark Card on the right pane.
    //
    // Cursor semantics: `push(cell)` moves the cursor to the new
    // entry and clears the forward stack (browser-style); `back()`
    // and `forward()` walk the cursor without mutating the history.
    // `set_current_cell` and the `parse_cell_nav` REPL surface push
    // here so every navigation surface contributes to the trail.
    //
    // Bookmarks are in-memory today (`RmapMap<String, CurrentCell>`)
    // and reset across reboots. A future task can reify them as
    // `bookmark_has_target` facts in the cell graph; the API surface
    // is shaped to make that swap drop-in.
    breadcrumb: BreadcrumbState,
    /// True when the next `set_current_cell` call should NOT push to
    /// the breadcrumb history. Set transiently by `back()` / `forward()`
    /// / `goto_bookmark()` so the cursor walks an existing entry
    /// rather than appending a fresh one. Always reset to `false` at
    /// the end of `set_current_cell`.
    suppress_breadcrumb_push: bool,
}

impl UnifiedReplState {
    fn new() -> Self {
        // Seed the breadcrumb history with the initial Root cell so
        // the persistent strip across the top of the panel has
        // something to render on first paint and so the first
        // navigation push lands on top of a real entry rather than an
        // empty ring.
        let mut breadcrumb = BreadcrumbState::new();
        breadcrumb.push(CurrentCell::Root);
        Self {
            nav_stack: vec![Breadcrumb::Root],
            subscriber_id: None,
            scrollback: vec![WELCOME.to_string()],
            history: Vec::new(),
            history_idx: None,
            pending_input: String::new(),
            current_cell: CurrentCell::Root,
            nav_targets: Vec::new(),
            system_actions: Vec::new(),
            breadcrumb,
            suppress_breadcrumb_push: false,
        }
    }

    // ---- HATEOAS-side helpers --------------------------------------

    /// Top-of-stack — what the HATEOAS pane is currently displaying.
    fn current_nav(&self) -> &Breadcrumb {
        self.nav_stack.last().expect("nav_stack always has Root")
    }

    fn nav_push(&mut self, crumb: Breadcrumb) {
        self.nav_stack.push(crumb);
        self.sync_current_cell();
    }

    /// Pop one step. Refuses to drop the `Root` entry.
    fn nav_pop(&mut self) {
        if self.nav_stack.len() > 1 {
            self.nav_stack.pop();
            self.sync_current_cell();
        }
    }

    /// Recompute `current_cell` from the top of `nav_stack`. Called
    /// after every nav mutation so the cell-as-screen rendering layer
    /// stays consistent with the breadcrumb trail. The two views are
    /// equivalent (one breadcrumb top → one CurrentCell variant) but
    /// stored separately so future #512 navigation actions can replace
    /// the breadcrumb stack wholesale (e.g. jump-to-cell from the
    /// command palette) without losing the current-cell invariant.
    ///
    /// Also pushes onto the navigation-history breadcrumb (#516) so
    /// HATEOAS-side resource / instance picks contribute to the trail
    /// alongside REPL-driven cell jumps. Suppressed during back /
    /// forward stepping.
    fn sync_current_cell(&mut self) {
        let new_cell = match self.current_nav() {
            Breadcrumb::Root => CurrentCell::Root,
            Breadcrumb::Noun { noun } => CurrentCell::Noun { noun: noun.clone() },
            Breadcrumb::Instance { noun, instance } => CurrentCell::Instance {
                noun: noun.clone(),
                instance: instance.clone(),
            },
        };
        let unchanged = new_cell == self.current_cell;
        self.current_cell = new_cell.clone();
        if self.suppress_breadcrumb_push {
            self.suppress_breadcrumb_push = false;
        } else if !unchanged {
            // Only push when the cell actually changed — avoid
            // polluting the trail when nav_pop unwinds back to a
            // breadcrumb position that already matches current_cell.
            self.breadcrumb.push(new_cell);
        }
    }

    /// Set the current cell directly without going through breadcrumb
    /// navigation. Used when REPL input resolves to a specific cell
    /// (e.g. `cell Foo_has_Bar` would jump to a FactCell). The
    /// breadcrumb trail is also updated to keep the left-pane visual
    /// in sync; FactCell / ComponentInstance variants don't have a
    /// breadcrumb mapping and just leave the trail as-is.
    ///
    /// Also pushes onto the navigation-history breadcrumb (#516) so
    /// the persistent strip + back/forward stepping reflect the new
    /// position. Suppressed during back / forward / goto_bookmark
    /// flows (see `suppress_breadcrumb_push`) — those advance the
    /// cursor over existing entries rather than appending fresh ones.
    fn set_current_cell(&mut self, cell: CurrentCell) {
        self.current_cell = cell.clone();
        match &cell {
            CurrentCell::Root => {
                self.nav_stack.truncate(1);
            }
            CurrentCell::Noun { noun } => {
                self.nav_stack.truncate(1);
                self.nav_stack.push(Breadcrumb::Noun { noun: noun.clone() });
            }
            CurrentCell::Instance { noun, instance } => {
                self.nav_stack.truncate(1);
                self.nav_stack.push(Breadcrumb::Noun { noun: noun.clone() });
                self.nav_stack.push(Breadcrumb::Instance {
                    noun: noun.clone(),
                    instance: instance.clone(),
                });
            }
            // FactCell + ComponentInstance: keep nav_stack as-is —
            // the breadcrumb model can't represent them today, but
            // current_cell carries the truth for the typed surface.
            // #512 will extend the breadcrumb to cover these.
            CurrentCell::FactCell { .. } | CurrentCell::ComponentInstance { .. } => {}
        }
        if self.suppress_breadcrumb_push {
            self.suppress_breadcrumb_push = false;
        } else {
            self.breadcrumb.push(cell);
        }
    }

    // ---- Navigation history (#516) helpers -------------------------

    /// Step the breadcrumb cursor one entry toward the oldest. When
    /// the cursor moves, the corresponding cell becomes the current
    /// cell (without pushing a fresh history entry). No-op when the
    /// cursor is already at the oldest entry.
    fn back(&mut self) -> Option<CurrentCell> {
        let target = self.breadcrumb.back()?;
        self.suppress_breadcrumb_push = true;
        self.set_current_cell(target.clone());
        Some(target)
    }

    /// Step the breadcrumb cursor one entry toward the tip. Same
    /// shape as `back` — the destination cell becomes current
    /// without appending a new history entry.
    fn forward(&mut self) -> Option<CurrentCell> {
        let target = self.breadcrumb.forward()?;
        self.suppress_breadcrumb_push = true;
        self.set_current_cell(target.clone());
        Some(target)
    }

    /// Bookmark the current cell under `label`. Overwrites any prior
    /// bookmark with the same label. Empty labels are accepted (the
    /// caller is responsible for trimming).
    fn bookmark(&mut self, label: String) {
        let cell = self.current_cell.clone();
        self.breadcrumb.bookmark(label, cell);
    }

    /// Bookmark an explicit cell under `label`. Used by the REPL
    /// `bookmark <label>` command and the Bookmark Card's "save
    /// current" affordance.
    fn bookmark_cell(&mut self, label: String, cell: CurrentCell) {
        self.breadcrumb.bookmark(label, cell);
    }

    /// Look up a bookmark by label and jump to it. Pushes a fresh
    /// history entry — bookmarks count as navigation events so the
    /// user can `back` from the destination back to where they were.
    fn goto_bookmark(&mut self, label: &str) -> Option<CurrentCell> {
        let target = self.breadcrumb.goto_bookmark(label)?;
        // Bookmarks ARE navigation events — push, don't suppress.
        self.set_current_cell(target.clone());
        Some(target)
    }

    // ---- REPL-side helpers -----------------------------------------

    /// Append one line to scrollback, dropping the oldest if at cap.
    fn push_line(&mut self, line: String) {
        if self.scrollback.len() >= SCROLLBACK_MAX {
            self.scrollback.remove(0);
        }
        self.scrollback.push(line);
    }

    /// Push a multi-line response onto scrollback by splitting on `\n`.
    fn push_response(&mut self, response: &str) {
        for line in response.split('\n') {
            self.push_line(line.to_string());
        }
    }

    /// Wholesale clear scrollback. Bound to Ctrl+L. History is preserved.
    fn clear_scrollback(&mut self) {
        self.scrollback.clear();
    }

    /// Walk one step into the past. Returns the new `current_input`
    /// the Slint side should show, or `None` for no-op.
    fn history_prev(&mut self, current: &str) -> Option<String> {
        if self.history.is_empty() {
            return None;
        }
        let new_idx = match self.history_idx {
            None => {
                // First Up press — snapshot the in-progress line so
                // Down can later restore it.
                self.pending_input = current.to_string();
                self.history.len() - 1
            }
            Some(0) => return None, // Already at oldest.
            Some(i) => i - 1,
        };
        self.history_idx = Some(new_idx);
        Some(self.history[new_idx].clone())
    }

    /// Walk one step toward the present. Returns the new
    /// `current_input` the Slint side should show.
    fn history_next(&mut self) -> Option<String> {
        match self.history_idx {
            None => None,
            Some(i) if i + 1 < self.history.len() => {
                self.history_idx = Some(i + 1);
                Some(self.history[i + 1].clone())
            }
            Some(_) => {
                // Walked past the newest — restore the pre-browse
                // snapshot.
                self.history_idx = None;
                let restored = self.pending_input.clone();
                self.pending_input.clear();
                Some(restored)
            }
        }
    }

    /// Submit a line: render it into scrollback, evaluate, render the
    /// response, push to history, reset history browsing.
    ///
    /// Cell-as-screen extension (#511): if the line is a recognised
    /// cell-navigation command (`cell <name>`, `noun <name>`,
    /// `instance <noun> <id>`, `home`), the current cell is advanced
    /// and a one-line "Now showing: …" annotation is pushed in lieu
    /// of (or alongside) the underlying REPL response. The typed-
    /// surface area on the right pane reads `current_cell` on the
    /// next redraw and re-selects its Component.
    ///
    /// Lines that do NOT match a navigation prefix flow through to
    /// the existing `crate::repl::evaluate_line` dispatcher unchanged
    /// — backward-compatible with every command the prior repl
    /// surface understood (`help`, `heap`, `quit`, …).
    fn submit(&mut self, prompt: &str, line: String) {
        self.push_line(format!("{prompt}{line}"));

        let trimmed = line.trim();
        // #517: screen-aware help intercept. Runs before the cell-nav
        // parser AND before the legacy `crate::repl::evaluate_line`
        // dispatcher so `help` and `?` pick up the cell-type-aware
        // body from `crate::ui_apps::help::screen_help` regardless of
        // what the legacy REPL's static help would have said. Other
        // legacy verbs (`heap`, `quit`, …) still flow through unchanged.
        let lower = trimmed.to_ascii_lowercase();
        if lower == "help" || lower == "?" {
            for help_line in crate::ui_apps::help::screen_help(&self.current_cell) {
                self.push_line(help_line);
            }
        } else if lower == "violations" || lower == "wrong" {
            // #590: screen-aware violation rendering. Reads
            // Violation_* cells from current state per Theorem 4 and
            // filters to violations whose Resource references the
            // current cell (or system-wide for Root). Falls back to
            // an explanatory line when state is not yet installed
            // (pre-init or test-harness path).
            let lines = crate::system::with_state(|state| {
                crate::ui_apps::violations::render_for_cell(
                    &self.current_cell,
                    state,
                )
            })
            .unwrap_or_else(|| vec!["State unavailable — kernel not yet initialised.".to_string()]);
            for line in lines {
                self.push_line(line);
            }
        } else if let Some(cell) = parse_cell_nav(trimmed) {
            let label = cell.label();
            self.set_current_cell(cell);
            self.push_line(format!("Now showing: {label}"));
        } else {
            let response = crate::repl::evaluate_line(&line);
            if !response.is_empty() {
                self.push_response(&response);
            }
        }

        if !trimmed.is_empty() {
            let is_dup = self.history.last().map(|s| s == trimmed).unwrap_or(false);
            if !is_dup {
                self.history.push(trimmed.to_string());
            }
        }

        self.history_idx = None;
        self.pending_input.clear();
    }
}

/// Parse a REPL line as a cell-as-screen navigation command. Returns
/// `Some(CurrentCell)` if the line is a recognised navigation form;
/// `None` otherwise (the caller falls through to the legacy REPL
/// dispatcher).
///
/// Recognised forms (case-insensitive on the verb, case-preserving
/// on the args):
///   * `home`                          — CurrentCell::Root
///   * `noun <Noun>`                   — CurrentCell::Noun
///   * `instance <Noun> <Id>`          — CurrentCell::Instance
///   * `cell <CellName>`               — CurrentCell::FactCell
///   * `component <ComponentId>`       — CurrentCell::ComponentInstance
///
/// #515's command palette will reuse this parser as one of its
/// primary actions; keeping the surface here rather than inside
/// `crate::repl::dispatch` lets navigation work on cells that REPL's
/// dispatcher knows nothing about.
fn parse_cell_nav(line: &str) -> Option<CurrentCell> {
    let mut parts = line.split_whitespace();
    let verb = parts.next()?.to_ascii_lowercase();
    match verb.as_str() {
        "home" => {
            if parts.next().is_none() {
                Some(CurrentCell::Root)
            } else {
                None
            }
        }
        "noun" => {
            let noun = parts.next()?.to_string();
            if parts.next().is_some() {
                return None;
            }
            Some(CurrentCell::Noun { noun })
        }
        "instance" => {
            let noun = parts.next()?.to_string();
            let instance = parts.next()?.to_string();
            if parts.next().is_some() {
                return None;
            }
            Some(CurrentCell::Instance { noun, instance })
        }
        "cell" => {
            let name = parts.next()?.to_string();
            if parts.next().is_some() {
                return None;
            }
            Some(CurrentCell::FactCell { cell_name: name })
        }
        "component" => {
            let id = parts.next()?.to_string();
            if parts.next().is_some() {
                return None;
            }
            Some(CurrentCell::ComponentInstance { component_id: id })
        }
        _ => None,
    }
}

/// Drop the change-subscription registered by `build_app` so the
/// `system::SUBSCRIBERS` registry doesn't grow unbounded across
/// app re-launches. Idempotent — `system::unsubscribe` is a no-op
/// on an unknown id.
impl Drop for UnifiedReplState {
    fn drop(&mut self) {
        if let Some(id) = self.subscriber_id.take() {
            crate::system::unsubscribe(id);
        }
    }
}

// ── HATEOAS-side cell-walk helpers (pure functions over &Object) ───
//
// These mirror the prior `hateoas.rs` helpers verbatim — same shape,
// same docs. They are `&Object` -> owned data so the read-side
// `RwLock` guard inside `system::with_state` can drop the moment the
// helper returns.

/// Walk every cell in `state` and return the sorted, deduplicated set
/// of Noun names (the leading token before `_has_`). Filters cell
/// names containing `:` (schema shards) and the synthetic `D` def
/// cell (kernel-internal welcome / echo / list:* / get:* names).
fn discover_nouns(state: &Object) -> Vec<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    for (cell_name, _) in ast::cells_iter(state) {
        if cell_name.contains(':') {
            continue;
        }
        let Some(noun) = noun_of(cell_name) else { continue };
        if noun == "D" {
            continue;
        }
        set.insert(noun.to_string());
    }
    set.into_iter().collect()
}

/// Extract the leading `<Noun>` token from a `<Noun>_has_<Attribute>`
/// cell name. Returns `None` when the cell name doesn't contain
/// `_has_`.
fn noun_of(cell_name: &str) -> Option<&str> {
    cell_name.split_once("_has_").map(|(noun, _)| noun)
}

/// Every cell that belongs to `noun` in `state`, returned as
/// `(attribute, &cell_contents)` pairs.
fn cells_for_noun<'a>(noun: &str, state: &'a Object) -> Vec<(&'a str, &'a Object)> {
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

/// Every distinct instance identifier for `noun` in `state`.
fn instances_of(noun: &str, state: &Object) -> Vec<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    for (_attr, cell) in cells_for_noun(noun, state) {
        let Some(facts) = cell.as_seq() else { continue };
        for fact in facts {
            if let Some(id) = ast::binding(fact, noun) {
                set.insert(id.to_string());
            }
        }
    }
    set.into_iter().collect()
}

/// Build the detail view for one instance of `noun`.
fn detail_lines_for(noun: &str, instance: &str, state: &Object) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();

    out.push(format!("# {noun}/{instance}"));
    let cells = cells_for_noun(noun, state);
    let mut binding_count = 0usize;
    for (attr, cell) in &cells {
        let Some(facts) = cell.as_seq() else { continue };
        for fact in facts {
            if ast::binding(fact, noun) != Some(instance) {
                continue;
            }
            if let Some(value) = ast::binding(fact, attr) {
                out.push(format!("{attr} = {value}"));
                binding_count += 1;
            }
        }
    }
    if binding_count == 0 {
        out.push("(no bindings)".to_string());
    }

    out.push(String::new());
    out.push("\u{2190} back-references".to_string());
    let self_cell_names: BTreeSet<String> = cells
        .iter()
        .map(|(attr, _)| format!("{noun}_has_{attr}"))
        .collect();
    let mut backref_count = 0usize;
    let mut backrefs: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (cell_name, cell) in ast::cells_iter(state) {
        if self_cell_names.contains(cell_name) {
            continue;
        }
        if cell_name.contains(':') {
            continue;
        }
        let Some(facts) = cell.as_seq() else { continue };
        for fact in facts {
            let Some(pairs) = fact.as_seq() else { continue };
            for pair in pairs {
                let Some(items) = pair.as_seq() else { continue };
                if items.len() != 2 {
                    continue;
                }
                let Some(role) = items[0].as_atom() else { continue };
                let Some(value) = items[1].as_atom() else { continue };
                if value == instance {
                    backrefs
                        .entry(cell_name.to_string())
                        .or_default()
                        .push(role.to_string());
                    backref_count += 1;
                    break;
                }
            }
        }
    }
    for (cell_name, roles) in &backrefs {
        let roles_joined = roles.join(", ");
        out.push(format!("  {cell_name} (as {roles_joined})"));
    }
    if backref_count == 0 {
        out.push("  (no back-references)".to_string());
    }

    out
}

// ── Redraw: project state into all Slint properties ────────────────

/// One redraw's worth of derived data (HATEOAS side + cell-as-screen).
/// Computed inside the `with_state` closure so the read lock is
/// released the moment `Snapshot::collect` returns.
struct Snapshot {
    resources: Vec<String>,
    selected_resource_index: i32,
    instances: Vec<String>,
    selected_instance_index: i32,
    detail_lines: Vec<String>,
    breadcrumbs: Vec<String>,
    /// Status fragment for HATEOAS half (combined with REPL fragment
    /// in `redraw`).
    hateoas_status: String,
    /// Cell-as-screen render (#511): the typed-surface header + the
    /// projected field list for the current cell. The Slint side reads
    /// every field of this for the right-pane typed surface.
    rendered: RenderedScreen,
}

impl Snapshot {
    fn empty() -> Self {
        Self {
            resources: Vec::new(),
            selected_resource_index: -1,
            instances: Vec::new(),
            selected_instance_index: -1,
            detail_lines: vec![
                "SYSTEM not initialised \u{2014} call system::init() first.".to_string(),
            ],
            breadcrumbs: vec!["Resources".to_string()],
            hateoas_status: "system::init() not yet called".to_string(),
            rendered: RenderedScreen {
                cell_label: "Resources".to_string(),
                selected: None,
                fields: vec![
                    "(SYSTEM not initialised — call system::init() first)".to_string(),
                ],
                navigation: Vec::new(),
                actions: Vec::new(),
            },
        }
    }

    fn collect(state: &Object, ui: &UnifiedReplState) -> Self {
        let resources = discover_nouns(state);

        let active_noun: Option<&str> = ui.nav_stack.iter().rev().find_map(|c| match c {
            Breadcrumb::Noun { noun } => Some(noun.as_str()),
            Breadcrumb::Instance { noun, .. } => Some(noun.as_str()),
            Breadcrumb::Root => None,
        });
        let selected_resource_index = match active_noun {
            Some(n) => resources
                .iter()
                .position(|r| r == n)
                .map(|p| p as i32)
                .unwrap_or(-1),
            None => -1,
        };

        let instances: Vec<String> = match active_noun {
            Some(n) => instances_of(n, state),
            None => Vec::new(),
        };

        let selected_instance_id: Option<&str> = match ui.current_nav() {
            Breadcrumb::Instance { instance, .. } => Some(instance.as_str()),
            _ => None,
        };
        let selected_instance_index = match selected_instance_id {
            Some(id) => instances
                .iter()
                .position(|i| i == id)
                .map(|p| p as i32)
                .unwrap_or(-1),
            None => -1,
        };

        let detail_lines: Vec<String> = match ui.current_nav() {
            Breadcrumb::Instance { noun, instance } => {
                detail_lines_for(noun, instance, state)
            }
            _ => Vec::new(),
        };

        let breadcrumbs: Vec<String> = ui.nav_stack.iter().map(|c| c.label()).collect();

        let hateoas_status = match ui.current_nav() {
            Breadcrumb::Root => format!("{} resources", resources.len()),
            Breadcrumb::Noun { noun } => {
                format!("{} \u{2014} {} instance(s)", noun, instances.len())
            }
            Breadcrumb::Instance { noun, instance } => format!("{noun}/{instance}"),
        };

        let rendered = cell_renderer::render_current_cell(&ui.current_cell, state);

        Self {
            resources,
            selected_resource_index,
            instances,
            selected_instance_index,
            detail_lines,
            breadcrumbs,
            hateoas_status,
            rendered,
        }
    }
}

/// Refresh every Slint property from the Rust state.
///
/// Pushes both the HATEOAS pane's models AND the REPL pane's
/// scrollback in a single call so callbacks can mutate either side
/// and call `redraw` once.
///
/// Takes `&mut` because the navigation catalogue (#512) is cached on
/// the state struct so the click handler can resolve "row N clicked"
/// to a `CurrentCell` jump deterministically; the cache is refreshed
/// every redraw so live cell-graph updates are reflected on the
/// next click.
fn redraw(window: &UnifiedRepl, ui: &mut UnifiedReplState) {
    let snapshot = crate::system::with_state(|s| Snapshot::collect(s, ui));
    let snap = snapshot.unwrap_or_else(Snapshot::empty);
    ui.nav_targets = snap.rendered.navigation.clone();
    ui.system_actions = snap.rendered.actions.clone();

    // ---- HATEOAS pane ----
    let resources_model: StringModel = ModelRc::new(VecModel::from_iter(
        snap.resources.iter().map(SharedString::from),
    ));
    window.set_resources(resources_model);
    window.set_selected_resource_index(snap.selected_resource_index);

    let instances_model: StringModel = ModelRc::new(VecModel::from_iter(
        snap.instances.iter().map(SharedString::from),
    ));
    window.set_instances(instances_model);
    window.set_selected_instance_index(snap.selected_instance_index);

    let detail_model: StringModel = ModelRc::new(VecModel::from_iter(
        snap.detail_lines.iter().map(SharedString::from),
    ));
    window.set_detail_lines(detail_model);

    let crumbs_model: StringModel = ModelRc::new(VecModel::from_iter(
        snap.breadcrumbs.iter().map(SharedString::from),
    ));
    window.set_breadcrumbs(crumbs_model);

    // ---- REPL pane ----
    let scrollback_model: StringModel = ModelRc::new(VecModel::from_iter(
        ui.scrollback.iter().map(SharedString::from),
    ));
    window.set_scrollback(scrollback_model);

    // ---- Cell-as-screen typed surface (#511) ----
    // Header: cell label + selected Component triple. The Slint side
    // renders these in a labelled card above the field list. When no
    // Component matched, `selected_*` properties are empty strings —
    // the Slint side branches on emptiness to switch to the generic
    // key-value fallback rendering.
    window.set_current_cell_name(SharedString::from(snap.rendered.cell_label.as_str()));
    let (sel_component, sel_toolkit, sel_symbol) = match &snap.rendered.selected {
        Some(s) => (s.component.clone(), s.toolkit.clone(), s.symbol.clone()),
        None => (String::new(), String::new(), String::new()),
    };
    window.set_current_cell_component(SharedString::from(sel_component));
    window.set_current_cell_toolkit(SharedString::from(sel_toolkit));
    window.set_current_cell_symbol(SharedString::from(sel_symbol));
    let fields_model: StringModel = ModelRc::new(VecModel::from_iter(
        snap.rendered.fields.iter().map(SharedString::from),
    ));
    window.set_current_cell_fields(fields_model);

    // ---- Navigation actions as cells (#512) ----
    // Each affordance row in the right pane corresponds to one
    // `NavigationTarget` derived from the cell graph. The Slint side
    // keys clicks by index into this vector — `compute_navigation_targets`
    // sorts deterministically so successive redraws emit identical
    // orderings, and the click handler resolves index → target via
    // the cached `nav_targets` field on `UnifiedReplState`.
    let nav_labels_model: StringModel = ModelRc::new(VecModel::from_iter(
        snap.rendered
            .navigation
            .iter()
            .map(|t| SharedString::from(t.label.as_str())),
    ));
    window.set_navigation_targets(nav_labels_model);

    // ---- SYSTEM calls as actions on current screen (#513) ----
    // Mirrors the navigation push above. Each row is one `SystemAction`
    // derived from the cell graph + the SYSTEM verb namespace; clicks
    // dispatch the verb with `default_args` pre-bound from the cell
    // context. The Slint side keys clicks by index into the cached
    // `system_actions` vector — same stability story as nav targets.
    //
    // State-machine surface (#514): the action enumerator marks any
    // guard-blocked transition with `GuardStatus::BlockedByGuard`. We
    // push two index-aligned parallel arrays alongside the labels —
    // `system-action-enabled` carries the enabled bool per row and
    // `system-action-tooltips` carries the violation-text per row.
    // The Slint side reads these to render the disabled state visually
    // (greyed out + hover tooltip with the violation text).
    let action_labels_model: StringModel = ModelRc::new(VecModel::from_iter(
        snap.rendered
            .actions
            .iter()
            .map(|a| SharedString::from(a.label.as_str())),
    ));
    window.set_system_actions(action_labels_model);
    let action_enabled_model: ModelRc<bool> = ModelRc::new(VecModel::from_iter(
        snap.rendered
            .actions
            .iter()
            .map(|a| a.guard_status.is_enabled()),
    ));
    window.set_system_action_enabled(action_enabled_model);
    let action_tooltips_model: StringModel = ModelRc::new(VecModel::from_iter(
        snap.rendered
            .actions
            .iter()
            .map(|a| SharedString::from(a.guard_status.tooltip())),
    ));
    window.set_system_action_tooltips(action_tooltips_model);

    // ---- "You are here" breadcrumb + back/forward (#516) ----
    // Tail of the navigation-history ring (last 5 entries) plus the
    // cursor's index within that tail and the can-go-back / forward
    // gates. The Slint side renders these as the persistent strip
    // across the top of the panel; the ◀ / ▶ buttons walk the cursor
    // without appending new entries.
    const NAV_HISTORY_TAIL: usize = 5;
    let nav_path = ui.breadcrumb.current_path_tail(NAV_HISTORY_TAIL);
    let nav_history_model: StringModel = ModelRc::new(VecModel::from_iter(
        nav_path.iter().map(|e: &CrumbEntry| SharedString::from(e.cell.label())),
    ));
    window.set_nav_history(nav_history_model);
    let nav_current_idx = nav_path
        .iter()
        .position(|e| e.is_current)
        .map(|p| p as i32)
        .unwrap_or(-1);
    window.set_nav_history_current_index(nav_current_idx);
    window.set_nav_can_go_back(ui.breadcrumb.can_go_back());
    window.set_nav_can_go_forward(ui.breadcrumb.can_go_forward());

    // ---- Bookmarks (#516) ----
    // Every registered bookmark, sorted by label. Each row is one
    // `(label, target)` pair; the Slint surface renders just the
    // labels and dispatches `bookmark-goto(idx)` on click — the
    // kernel resolves index → label via the same sorted order.
    let bookmarks_list = ui.breadcrumb.bookmark_list();
    let bookmarks_model: StringModel = ModelRc::new(VecModel::from_iter(
        bookmarks_list.iter().map(|(label, _)| SharedString::from(label.as_str())),
    ));
    window.set_bookmarks(bookmarks_model);

    // ---- Combined status footer ----
    let combined_status = format!(
        "{} \u{2022} {} repl line(s) \u{2022} {} bookmark(s) \u{2022} Up/Down history \u{2022} Ctrl+L clear \u{2022} Esc back",
        snap.hateoas_status,
        ui.scrollback.len(),
        ui.breadcrumb.bookmark_count(),
    );
    window.set_status_text(SharedString::from(combined_status));
}

// ── Live-update plumbing (mirrors hateoas::SendSyncWeak) ────────────
//
// The `system::subscribe_changes` handler signature
// (`Fn(&[String]) + Send + Sync`) forces every captured value to be
// `Send + Sync`. `slint::Weak<UnifiedRepl>` is `Send + Sync` under the
// kernel's `unsafe-single-threaded` slint feature; but
// `alloc::rc::Weak<RefCell<UnifiedReplState>>` is NOT — `Rc` is
// intentionally `!Send`. This newtype wraps it with manual unsafe
// `Send + Sync` impls, sound under the kernel's single-threaded boot
// model. Same shape the prior `hateoas` module used.
struct SendSyncWeak<T: ?Sized>(alloc::rc::Weak<T>);

impl<T: ?Sized> SendSyncWeak<T> {
    fn upgrade(&self) -> Option<Rc<T>> {
        self.0.upgrade()
    }
}

// SAFETY: kernel is single-threaded (mirrors the pattern used for
// `FramebufferBackend` in `arch/uefi/slint_backend.rs`). The handler
// is invoked from `system::apply` on the same super-loop thread.
unsafe impl<T: ?Sized> Send for SendSyncWeak<T> {}
unsafe impl<T: ?Sized> Sync for SendSyncWeak<T> {}

// ── The constructed Unified REPL app ───────────────────────────────

/// The constructed UnifiedRepl Window plus its mutable state. Returned
/// from `build_app` so the launcher can both `show()` the window and
/// read out diagnostics for tests + future host hooks.
pub struct UnifiedReplApp {
    /// The Slint window. `ComponentHandle` requires the inner
    /// `UnifiedRepl` component stay alive for the duration of the
    /// event loop; `UnifiedReplApp` holds it by value.
    pub window: UnifiedRepl,
    /// The shared mutable state.
    state: Rc<RefCell<UnifiedReplState>>,
}

impl UnifiedReplApp {
    /// Read-only access to the current scrollback length.
    pub fn scrollback_len(&self) -> usize {
        self.state.borrow().scrollback.len()
    }

    /// Read-only access to the current history length.
    pub fn history_len(&self) -> usize {
        self.state.borrow().history.len()
    }

    /// Read-only access to the current navigation depth (1 = Root only).
    pub fn nav_depth(&self) -> usize {
        self.state.borrow().nav_stack.len()
    }

    /// Read-only access to the current cell's display label. Useful
    /// for tests and future host-side instrumentation that wants to
    /// observe what screen the user is on without inspecting the
    /// breadcrumb stack directly.
    pub fn current_cell_label(&self) -> String {
        self.state.borrow().current_cell.label()
    }

    /// Read-only access to the cached navigation-target catalogue
    /// length (#512). After construction this is `0` because no
    /// `crate::system::with_state` snapshot has populated it yet
    /// (the smoke-test platform has no SYSTEM); after the first live
    /// `redraw` it reflects the cell-graph-derived row count.
    pub fn navigation_target_count(&self) -> usize {
        self.state.borrow().nav_targets.len()
    }

    /// Read-only access to the cached SYSTEM-action catalogue length
    /// (#513). Same accounting story as `navigation_target_count`:
    /// `0` after construction (no live SYSTEM snapshot), refreshed
    /// after the first `redraw`. Useful for tests that want to
    /// confirm the action surface is wired without driving the full
    /// Slint event loop.
    pub fn system_action_count(&self) -> usize {
        self.state.borrow().system_actions.len()
    }

    /// Read-only access to the navigation-history length (#516).
    /// Starts at 1 after construction (the initial Root entry seeded
    /// in `UnifiedReplState::new`); grows as the user navigates.
    pub fn breadcrumb_history_len(&self) -> usize {
        self.state.borrow().breadcrumb.len()
    }

    /// Read-only access to the registered bookmark count (#516).
    pub fn bookmark_count(&self) -> usize {
        self.state.borrow().breadcrumb.bookmark_count()
    }

    /// Read-only access to the can-go-back gate (#516). Tests use
    /// this to confirm the cursor's relation to the ring's bounds
    /// without touching the breadcrumb directly.
    pub fn can_go_back(&self) -> bool {
        self.state.borrow().breadcrumb.can_go_back()
    }

    /// Read-only access to the can-go-forward gate (#516).
    pub fn can_go_forward(&self) -> bool {
        self.state.borrow().breadcrumb.can_go_forward()
    }
}

/// Construct the Unified REPL window and wire its callbacks.
///
/// The Slint platform must be installed before this is called
/// (`slint::platform::set_platform(Box::new(UefiSlintPlatform::new(...)))`)
/// — Slint refuses to instantiate components otherwise.
///
/// The window is *not* shown here; the caller (launcher) drives the
/// show / hide based on user navigation.
pub fn build_app() -> Result<UnifiedReplApp, slint::PlatformError> {
    let window = UnifiedRepl::new()?;
    let state = Rc::new(RefCell::new(UnifiedReplState::new()));

    // Initial paint — populates every Slint property from the empty
    // state before any user interaction.
    redraw(&window, &mut state.borrow_mut());
    window.set_prompt(SharedString::from("arest> "));

    // ---- HATEOAS pane callbacks ----------------------------------

    // Resource picked from the sidebar — push a `Noun` breadcrumb
    // (replacing any prior Noun/Instance) and re-render.
    {
        let weak = window.as_weak();
        let state = state.clone();
        window.on_resource_selected(move |idx| {
            let Some(window) = weak.upgrade() else { return };
            let noun: Option<String> = crate::system::with_state(|s| {
                discover_nouns(s).into_iter().nth(idx as usize)
            })
            .flatten();
            let Some(noun) = noun else { return };
            let mut s = state.borrow_mut();
            s.nav_stack.truncate(1);
            s.nav_push(Breadcrumb::Noun { noun });
            drop(s);
            redraw(&window, &mut state.borrow_mut());
        });
    }

    // Instance row picked — push an `Instance` breadcrumb on top of
    // the existing `Noun` (or replace any existing top `Instance`).
    {
        let weak = window.as_weak();
        let state = state.clone();
        window.on_instance_selected(move |idx| {
            let Some(window) = weak.upgrade() else { return };
            let active_noun: Option<String> = {
                let s = state.borrow();
                s.nav_stack.iter().rev().find_map(|c| match c {
                    Breadcrumb::Noun { noun } => Some(noun.clone()),
                    Breadcrumb::Instance { noun, .. } => Some(noun.clone()),
                    Breadcrumb::Root => None,
                })
            };
            let Some(noun) = active_noun else { return };
            let instance: Option<String> = crate::system::with_state(|st| {
                instances_of(&noun, st).into_iter().nth(idx as usize)
            })
            .flatten();
            let Some(instance) = instance else { return };
            let mut s = state.borrow_mut();
            // Sibling jump: replace top Instance rather than nest.
            if matches!(s.current_nav(), Breadcrumb::Instance { .. }) {
                s.nav_pop();
            }
            s.nav_push(Breadcrumb::Instance { noun, instance });
            drop(s);
            redraw(&window, &mut state.borrow_mut());
        });
    }

    // Back button — pop one level off the breadcrumb stack.
    {
        let weak = window.as_weak();
        let state = state.clone();
        window.on_back_clicked(move || {
            let Some(window) = weak.upgrade() else { return };
            state.borrow_mut().nav_pop();
            redraw(&window, &mut state.borrow_mut());
        });
    }

    // ---- REPL pane callbacks --------------------------------------

    // Submit (Enter pressed in the input row).
    {
        let weak = window.as_weak();
        let state = state.clone();
        window.on_submit(move |line| {
            let Some(window) = weak.upgrade() else { return };
            let prompt = window.get_prompt().to_string();
            let line_owned = line.to_string();
            {
                let mut s = state.borrow_mut();
                s.submit(&prompt, line_owned);
            }
            window.set_current_input(SharedString::from(""));
            redraw(&window, &mut state.borrow_mut());
        });
    }

    // Ctrl+L — wholesale scrollback clear.
    {
        let weak = window.as_weak();
        let state = state.clone();
        window.on_clear(move || {
            let Some(window) = weak.upgrade() else { return };
            state.borrow_mut().clear_scrollback();
            redraw(&window, &mut state.borrow_mut());
        });
    }

    // Up arrow — walk one step into the past.
    {
        let weak = window.as_weak();
        let state = state.clone();
        window.on_history_prev(move || {
            let Some(window) = weak.upgrade() else { return };
            let current = window.get_current_input().to_string();
            let new_input = state.borrow_mut().history_prev(&current);
            if let Some(text) = new_input {
                window.set_current_input(SharedString::from(text));
            }
        });
    }

    // Down arrow — walk one step toward the present.
    {
        let weak = window.as_weak();
        let state = state.clone();
        window.on_history_next(move || {
            let Some(window) = weak.upgrade() else { return };
            let new_input = state.borrow_mut().history_next();
            if let Some(text) = new_input {
                window.set_current_input(SharedString::from(text));
            }
        });
    }

    // Navigation actions as cells (#512). One affordance row per
    // entry in the cached `nav_targets`; clicking row N jumps the
    // current cell to `nav_targets[N].target`. The cache is refreshed
    // on every `redraw` so the Slint side and the click handler stay
    // in sync — the index the Slint side passes is always valid for
    // the cache the kernel last filled.
    {
        let weak = window.as_weak();
        let state = state.clone();
        window.on_navigation_target_selected(move |idx| {
            let Some(window) = weak.upgrade() else { return };
            let target: Option<CurrentCell> = {
                let s = state.borrow();
                s.nav_targets
                    .get(idx as usize)
                    .map(|t| t.target.clone())
            };
            let Some(target) = target else { return };
            {
                let mut s = state.borrow_mut();
                let label = target.label();
                s.set_current_cell(target);
                s.push_line(format!("Now showing: {label}"));
            }
            redraw(&window, &mut state.borrow_mut());
        });
    }

    // SYSTEM calls as actions on current screen (#513). One row per
    // entry in the cached `system_actions`; clicking row N invokes
    // the corresponding SYSTEM verb with the action's pre-bound
    // `default_args`. Result string is pushed to scrollback so the
    // user sees both the canonical verb form (the action label) and
    // the dispatch result without leaving the REPL surface.
    //
    // Cache + dispatch round-trip: the Slint side passes an index
    // that's always valid for the most-recent redraw because (1) the
    // action catalogue is recomputed on every redraw, and (2) the
    // Slint surface only fires this callback against the labels the
    // kernel last pushed. An out-of-bounds index from a stale Slint
    // event after a state swap is handled with a graceful no-op.
    {
        let weak = window.as_weak();
        let state = state.clone();
        window.on_action_invoked(move |idx| {
            let Some(window) = weak.upgrade() else { return };
            let action: Option<SystemAction> = {
                let s = state.borrow();
                s.system_actions.get(idx as usize).cloned()
            };
            let Some(action) = action else { return };
            let result = actions::dispatch_action(&action);
            {
                let mut s = state.borrow_mut();
                s.push_line(result);
            }
            redraw(&window, &mut state.borrow_mut());
        });
    }

    // ---- Navigation history: back / forward / bookmark (#516) -----
    //
    // ◀ button — walk the breadcrumb cursor one entry toward the
    // oldest. The cell at the new cursor position becomes current
    // without pushing a fresh history entry (browser-style).
    {
        let weak = window.as_weak();
        let state = state.clone();
        window.on_nav_back(move || {
            let Some(window) = weak.upgrade() else { return };
            let target = state.borrow_mut().back();
            if let Some(target) = target {
                let mut s = state.borrow_mut();
                s.push_line(format!("\u{2190} Back to: {}", target.label()));
            }
            redraw(&window, &mut state.borrow_mut());
        });
    }

    // ▶ button — walk the breadcrumb cursor one entry toward the
    // tip. Mirrors `nav_back` in reverse.
    {
        let weak = window.as_weak();
        let state = state.clone();
        window.on_nav_forward(move || {
            let Some(window) = weak.upgrade() else { return };
            let target = state.borrow_mut().forward();
            if let Some(target) = target {
                let mut s = state.borrow_mut();
                s.push_line(format!("\u{2192} Forward to: {}", target.label()));
            }
            redraw(&window, &mut state.borrow_mut());
        });
    }

    // "+ Bookmark current cell" — auto-label the bookmark with the
    // cell's display name. A future task can prompt for a label via
    // a Dialog Component (PPPP's #491 binder + #515 command palette).
    {
        let weak = window.as_weak();
        let state = state.clone();
        window.on_bookmark_current(move || {
            let Some(window) = weak.upgrade() else { return };
            let label = {
                let mut s = state.borrow_mut();
                let label = s.current_cell.label();
                s.bookmark(label.clone());
                label
            };
            {
                let mut s = state.borrow_mut();
                s.push_line(format!("Bookmarked: {label}"));
            }
            redraw(&window, &mut state.borrow_mut());
        });
    }

    // Bookmark Card row clicked — resolve the row index → label via
    // the same sorted ordering `bookmark_list` returned, then jump.
    // Pushes a fresh history entry (bookmarks count as navigation).
    {
        let weak = window.as_weak();
        let state = state.clone();
        window.on_bookmark_goto(move |idx| {
            let Some(window) = weak.upgrade() else { return };
            let label: Option<String> = {
                let s = state.borrow();
                s.breadcrumb
                    .bookmark_list()
                    .get(idx as usize)
                    .map(|(l, _)| l.clone())
            };
            let Some(label) = label else { return };
            let target = state.borrow_mut().goto_bookmark(&label);
            if let Some(target) = target {
                let mut s = state.borrow_mut();
                s.push_line(format!("Bookmark: {label} \u{2192} {}", target.label()));
            }
            redraw(&window, &mut state.borrow_mut());
        });
    }

    // Theme toggle — passive forward (Theme global already swapped
    // mode inside the Slint handler). Hook for future ThemePref
    // persistence.
    window.on_theme_toggled(|| {});

    // ---- Live-update plumbing (mirrors hateoas pattern) -----------
    {
        let id_slot: alloc::sync::Arc<spin::Mutex<Option<SubscriberId>>> =
            alloc::sync::Arc::new(spin::Mutex::new(None));
        let window_weak = window.as_weak();
        let state_weak = SendSyncWeak(Rc::downgrade(&state));
        let id_slot_handler = id_slot.clone();
        let id = crate::system::subscribe_changes(Box::new(move |changed: &[String]| {
            let Some(window) = window_weak.upgrade() else {
                if let Some(my_id) = id_slot_handler.lock().take() {
                    crate::system::unsubscribe(my_id);
                }
                return;
            };
            let Some(state_rc) = state_weak.upgrade() else {
                if let Some(my_id) = id_slot_handler.lock().take() {
                    crate::system::unsubscribe(my_id);
                }
                return;
            };
            let active_noun: Option<String> = {
                let s = state_rc.borrow();
                s.nav_stack.iter().rev().find_map(|c| match c {
                    Breadcrumb::Noun { noun } => Some(noun.clone()),
                    Breadcrumb::Instance { noun, .. } => Some(noun.clone()),
                    Breadcrumb::Root => None,
                })
            };
            let needs_redraw = match active_noun.as_deref() {
                None => true,
                Some(noun) => {
                    let prefix = alloc::format!("{noun}_has_");
                    changed.iter().any(|name| name.starts_with(&prefix))
                }
            };
            if !needs_redraw {
                return;
            }
            redraw(&window, &mut state_rc.borrow_mut());
        }));
        *id_slot.lock() = Some(id);
        state.borrow_mut().subscriber_id = Some(id);
    }

    Ok(UnifiedReplApp { window, state })
}

// ── Tests ─────────────────────────────────────────────────────────
//
// `arest-kernel`'s bin target has `test = false` (Cargo.toml L33),
// so these `#[cfg(test)]` cases are reachable only when the crate is
// re-shaped into a lib for hosted testing. Coverage spans every
// helper that survived the merge so a future regression in either
// pane's behaviour gets caught.

#[cfg(test)]
mod tests {
    use super::*;
    use arest::ast::{cell_push, fact_from_pairs};

    /// Synthetic state — same shape as the prior `hateoas::tests`
    /// fixture so the migrated assertions match historical behaviour.
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
        s
    }

    // ---- HATEOAS pane: helper coverage ----------------------------

    #[test]
    fn discover_nouns_dedupes_and_sorts() {
        let state = synth_state();
        let nouns = discover_nouns(&state);
        assert_eq!(nouns, vec!["File".to_string(), "Tag".to_string()]);
    }

    #[test]
    fn discover_nouns_filters_d_def_cell() {
        let state = cell_push(
            "D_has_welcome",
            fact_from_pairs(&[("D", "x"), ("welcome", "y")]),
            &synth_state(),
        );
        let nouns = discover_nouns(&state);
        assert!(!nouns.contains(&"D".to_string()), "D filtered, got {nouns:?}");
        assert!(nouns.contains(&"File".to_string()));
    }

    #[test]
    fn instances_of_collects_distinct_ids() {
        let state = synth_state();
        let files = instances_of("File", &state);
        assert_eq!(files, vec!["f1".to_string(), "f2".to_string()]);
    }

    #[test]
    fn detail_lines_includes_bindings_and_back_references() {
        let state = synth_state();
        let lines = detail_lines_for("File", "f1", &state);
        assert_eq!(lines[0], "# File/f1");
        let blank_idx = lines.iter().position(|l| l.is_empty()).unwrap_or(lines.len());
        let bindings = &lines[..blank_idx];
        assert!(
            bindings.iter().any(|l| l == "Name = alpha.txt"),
            "expected Name binding, got {bindings:?}"
        );
        assert!(
            bindings.iter().any(|l| l == "MimeType = text/plain"),
            "expected MimeType binding, got {bindings:?}"
        );
        let backrefs = &lines[blank_idx..];
        assert!(
            backrefs.iter().any(|l| l.contains("Tag_is_on_File")),
            "expected Tag_is_on_File backref, got {backrefs:?}"
        );
    }

    #[test]
    fn breadcrumb_label_round_trip() {
        assert_eq!(Breadcrumb::Root.label(), "Resources");
        assert_eq!(Breadcrumb::Noun { noun: "File".into() }.label(), "File");
        assert_eq!(
            Breadcrumb::Instance { noun: "File".into(), instance: "f1".into() }.label(),
            "f1"
        );
    }

    #[test]
    fn nav_pop_refuses_to_drop_root() {
        let mut s = UnifiedReplState::new();
        assert!(matches!(s.current_nav(), Breadcrumb::Root));
        s.nav_pop();
        assert!(matches!(s.current_nav(), Breadcrumb::Root));
        assert_eq!(s.nav_stack.len(), 1);
    }

    // ---- REPL pane: helper coverage --------------------------------

    #[test]
    fn new_state_seeds_welcome_and_empty_history() {
        let s = UnifiedReplState::new();
        assert_eq!(s.scrollback.len(), 1);
        assert!(s.scrollback[0].contains("AREST"));
        assert!(s.history.is_empty());
        assert!(s.history_idx.is_none());
    }

    #[test]
    fn push_line_caps_at_scrollback_max() {
        let mut s = UnifiedReplState::new();
        for i in 0..(SCROLLBACK_MAX + 50) {
            s.push_line(format!("line {i}"));
        }
        assert_eq!(s.scrollback.len(), SCROLLBACK_MAX);
        assert!(s.scrollback[0].starts_with("line "));
    }

    #[test]
    fn push_response_splits_multiline() {
        let mut s = UnifiedReplState::new();
        s.push_response("alpha\nbeta\ngamma");
        assert_eq!(s.scrollback.len(), 4);
        assert_eq!(s.scrollback[1], "alpha");
        assert_eq!(s.scrollback[2], "beta");
        assert_eq!(s.scrollback[3], "gamma");
    }

    #[test]
    fn clear_scrollback_drops_all_lines() {
        let mut s = UnifiedReplState::new();
        s.push_line("foo".to_string());
        s.push_line("bar".to_string());
        s.clear_scrollback();
        assert!(s.scrollback.is_empty());
    }

    #[test]
    fn submit_pushes_prompt_line_and_response_to_scrollback() {
        let mut s = UnifiedReplState::new();
        s.scrollback.clear();
        s.submit("arest> ", "help".to_string());
        assert_eq!(s.scrollback[0], "arest> help");
        assert!(s.scrollback.len() > 1, "expected response lines, got {:?}", s.scrollback);
        let blob = s.scrollback.join("\n");
        assert!(blob.contains("help"), "missing help mention: {blob}");
    }

    #[test]
    fn submit_pushes_to_history_and_resets_browse_cursor() {
        let mut s = UnifiedReplState::new();
        s.submit("> ", "help".to_string());
        s.submit("> ", "heap".to_string());
        assert_eq!(s.history, vec!["help".to_string(), "heap".to_string()]);
        assert!(s.history_idx.is_none());
    }

    #[test]
    fn submit_dedups_consecutive_duplicates() {
        let mut s = UnifiedReplState::new();
        s.submit("> ", "help".to_string());
        s.submit("> ", "help".to_string());
        assert_eq!(s.history, vec!["help".to_string()]);
    }

    #[test]
    fn submit_ignores_blank_for_history() {
        let mut s = UnifiedReplState::new();
        s.submit("> ", "   ".to_string());
        assert!(s.history.is_empty());
    }

    #[test]
    fn history_prev_walks_back_then_stops_at_oldest() {
        let mut s = UnifiedReplState::new();
        s.history = vec!["one".to_string(), "two".to_string(), "three".to_string()];
        assert_eq!(s.history_prev(""), Some("three".to_string()));
        assert_eq!(s.history_prev(""), Some("two".to_string()));
        assert_eq!(s.history_prev(""), Some("one".to_string()));
        assert_eq!(s.history_prev(""), None);
    }

    #[test]
    fn history_prev_snapshots_in_progress_input() {
        let mut s = UnifiedReplState::new();
        s.history = vec!["one".to_string()];
        s.history_prev("partial-edit");
        assert_eq!(s.pending_input, "partial-edit");
    }

    #[test]
    fn history_next_walks_forward_and_restores_pending() {
        let mut s = UnifiedReplState::new();
        s.history = vec!["one".to_string(), "two".to_string()];
        s.history_prev("in-progress");
        s.history_prev("in-progress");
        assert_eq!(s.history_idx, Some(0));
        assert_eq!(s.history_next(), Some("two".to_string()));
        assert_eq!(s.history_next(), Some("in-progress".to_string()));
        assert!(s.history_idx.is_none());
    }

    #[test]
    fn history_next_with_no_browse_in_progress_is_noop() {
        let mut s = UnifiedReplState::new();
        s.history = vec!["one".to_string()];
        assert_eq!(s.history_next(), None);
    }

    #[test]
    fn history_prev_on_empty_history_returns_none() {
        let mut s = UnifiedReplState::new();
        assert_eq!(s.history_prev("x"), None);
    }

    /// Smoke test: `build_app()` constructs without panicking. Mirrors
    /// every prior `*::tests::build_app_constructs_under_minimal_window`.
    /// Installs a `UefiSlintPlatform` so the Slint codegen for
    /// `UnifiedRepl` runs through `register_bitmap_font` + the
    /// component constructor.
    ///
    /// `set_platform` returns Err on the second call within a single
    /// process — we ignore so any prior installer in the same test
    /// binary keeps its platform.
    #[test]
    fn build_app_constructs_under_minimal_window() {
        use crate::arch::uefi::slint_backend::UefiSlintPlatform;
        let platform = UefiSlintPlatform::new(1280, 800);
        let _ = slint::platform::set_platform(alloc::boxed::Box::new(platform));

        let app = build_app().expect("UnifiedRepl construction failed");
        // Welcome banner present after construction; navigation at Root.
        assert_eq!(app.scrollback_len(), 1);
        assert_eq!(app.history_len(), 0);
        assert_eq!(app.nav_depth(), 1);
        // Cell-as-screen invariant (#511): on construction the
        // current cell is Root.
        assert_eq!(app.current_cell_label(), "Resources");
    }

    // ---- Cell-as-screen pane (#511) coverage ----------------------

    #[test]
    fn new_state_seeds_root_as_current_cell() {
        let s = UnifiedReplState::new();
        assert_eq!(s.current_cell, CurrentCell::Root);
    }

    #[test]
    fn nav_push_noun_syncs_current_cell_to_noun() {
        let mut s = UnifiedReplState::new();
        s.nav_push(Breadcrumb::Noun { noun: "File".into() });
        assert_eq!(s.current_cell, CurrentCell::Noun { noun: "File".into() });
    }

    #[test]
    fn nav_push_instance_syncs_current_cell_to_instance() {
        let mut s = UnifiedReplState::new();
        s.nav_push(Breadcrumb::Noun { noun: "File".into() });
        s.nav_push(Breadcrumb::Instance {
            noun: "File".into(),
            instance: "f1".into(),
        });
        assert_eq!(
            s.current_cell,
            CurrentCell::Instance {
                noun: "File".into(),
                instance: "f1".into()
            }
        );
    }

    #[test]
    fn nav_pop_resyncs_current_cell_to_remaining_top() {
        let mut s = UnifiedReplState::new();
        s.nav_push(Breadcrumb::Noun { noun: "File".into() });
        s.nav_push(Breadcrumb::Instance {
            noun: "File".into(),
            instance: "f1".into(),
        });
        s.nav_pop(); // back to Noun
        assert_eq!(s.current_cell, CurrentCell::Noun { noun: "File".into() });
        s.nav_pop(); // back to Root
        assert_eq!(s.current_cell, CurrentCell::Root);
        s.nav_pop(); // refuses past Root → no change
        assert_eq!(s.current_cell, CurrentCell::Root);
    }

    #[test]
    fn set_current_cell_factcell_keeps_breadcrumb_unchanged() {
        // FactCell + ComponentInstance variants don't have a
        // breadcrumb mapping; current_cell carries the truth and the
        // nav stack stays where it was.
        let mut s = UnifiedReplState::new();
        s.nav_push(Breadcrumb::Noun { noun: "File".into() });
        let before_depth = s.nav_stack.len();
        s.set_current_cell(CurrentCell::FactCell {
            cell_name: "Component_has_Property".into(),
        });
        assert_eq!(s.nav_stack.len(), before_depth);
        assert_eq!(
            s.current_cell,
            CurrentCell::FactCell { cell_name: "Component_has_Property".into() }
        );
    }

    #[test]
    fn set_current_cell_instance_rebuilds_breadcrumb_trail() {
        // Direct jump to an Instance must rebuild the breadcrumb so
        // the left pane's trail is consistent.
        let mut s = UnifiedReplState::new();
        s.set_current_cell(CurrentCell::Instance {
            noun: "File".into(),
            instance: "f1".into(),
        });
        assert_eq!(s.nav_stack.len(), 3); // Root, Noun, Instance
        assert!(matches!(s.nav_stack[1], Breadcrumb::Noun { .. }));
        assert!(matches!(s.nav_stack[2], Breadcrumb::Instance { .. }));
    }

    // ---- parse_cell_nav ----------------------------------------

    #[test]
    fn parse_cell_nav_recognises_home() {
        assert_eq!(parse_cell_nav("home"), Some(CurrentCell::Root));
        assert_eq!(parse_cell_nav("HOME"), Some(CurrentCell::Root));
        assert_eq!(parse_cell_nav("home extra"), None);
    }

    #[test]
    fn parse_cell_nav_recognises_noun() {
        assert_eq!(
            parse_cell_nav("noun File"),
            Some(CurrentCell::Noun { noun: "File".into() })
        );
        assert_eq!(parse_cell_nav("noun"), None);
        assert_eq!(parse_cell_nav("noun File extra"), None);
    }

    #[test]
    fn parse_cell_nav_recognises_instance() {
        assert_eq!(
            parse_cell_nav("instance File f1"),
            Some(CurrentCell::Instance {
                noun: "File".into(),
                instance: "f1".into()
            })
        );
        assert_eq!(parse_cell_nav("instance File"), None);
    }

    #[test]
    fn parse_cell_nav_recognises_cell() {
        assert_eq!(
            parse_cell_nav("cell Component_has_Property"),
            Some(CurrentCell::FactCell {
                cell_name: "Component_has_Property".into()
            })
        );
    }

    #[test]
    fn parse_cell_nav_recognises_component() {
        assert_eq!(
            parse_cell_nav("component button.qt6"),
            Some(CurrentCell::ComponentInstance {
                component_id: "button.qt6".into()
            })
        );
    }

    #[test]
    fn parse_cell_nav_unknown_verb_returns_none() {
        assert_eq!(parse_cell_nav("help"), None);
        assert_eq!(parse_cell_nav(""), None);
        assert_eq!(parse_cell_nav("zzfrobnicate File"), None);
    }

    #[test]
    fn submit_cell_nav_advances_current_cell_and_skips_repl() {
        let mut s = UnifiedReplState::new();
        s.scrollback.clear();
        s.submit("> ", "noun File".to_string());
        assert_eq!(s.current_cell, CurrentCell::Noun { noun: "File".into() });
        // Annotation pushed; legacy repl::evaluate_line not invoked
        // because the line matched a navigation form.
        let blob = s.scrollback.join("\n");
        assert!(blob.contains("Now showing: File"), "missing nav annotation: {blob}");
        assert!(!blob.contains("unknown command"), "fell through to repl: {blob}");
    }

    #[test]
    fn submit_non_nav_line_falls_through_to_repl() {
        let mut s = UnifiedReplState::new();
        s.scrollback.clear();
        s.submit("> ", "help".to_string());
        // Current cell unchanged.
        assert_eq!(s.current_cell, CurrentCell::Root);
        // help response present.
        let blob = s.scrollback.join("\n");
        assert!(blob.contains("help"), "help response missing: {blob}");
    }

    #[test]
    fn submit_cell_nav_still_updates_history() {
        let mut s = UnifiedReplState::new();
        s.submit("> ", "noun File".to_string());
        assert_eq!(s.history, vec!["noun File".to_string()]);
    }

    // ---- Navigation actions as cells (#512) coverage --------------

    #[test]
    fn new_state_starts_with_empty_nav_targets() {
        // Before any redraw the cache is empty — the click handler
        // gracefully no-ops when given an out-of-bounds index.
        let s = UnifiedReplState::new();
        assert!(s.nav_targets.is_empty());
    }

    #[test]
    fn nav_targets_cache_drives_set_current_cell_jump() {
        // Simulate what `on_navigation_target_selected` does: read a
        // target out of the cache by index and `set_current_cell` to
        // it. The cell-as-screen invariant says the breadcrumb trail
        // is rebuilt for Noun / Instance variants.
        use crate::ui_apps::navigation::{NavigationKind, NavigationTarget};

        let mut s = UnifiedReplState::new();
        // Pre-populate the cache as if a redraw had filled it.
        s.nav_targets = vec![
            NavigationTarget::new(
                CurrentCell::Noun { noun: "File".into() },
                NavigationKind::Instance,
            ),
            NavigationTarget::new(
                CurrentCell::Instance {
                    noun: "File".into(),
                    instance: "f1".into(),
                },
                NavigationKind::Instance,
            ),
        ];

        // Picking row 1 (Instance) should rebuild the breadcrumb to
        // Root → Noun(File) → Instance(File/f1).
        let target = s.nav_targets[1].target.clone();
        s.set_current_cell(target);
        assert_eq!(s.nav_stack.len(), 3);
        assert!(matches!(s.nav_stack[1], Breadcrumb::Noun { .. }));
        assert!(matches!(s.nav_stack[2], Breadcrumb::Instance { .. }));
        assert_eq!(
            s.current_cell,
            CurrentCell::Instance {
                noun: "File".into(),
                instance: "f1".into()
            }
        );
    }

    #[test]
    fn nav_targets_out_of_bounds_index_no_op_safe() {
        // The Slint click handler reads `nav_targets.get(idx)`; an
        // out-of-bounds index (stale Slint event after a state swap)
        // must be handled without panic. Mirror that lookup pattern.
        let s = UnifiedReplState::new();
        assert!(s.nav_targets.get(0).is_none());
        assert!(s.nav_targets.get(usize::MAX).is_none());
    }

    // ---- SYSTEM calls as actions (#513) coverage ------------------

    #[test]
    fn new_state_starts_with_empty_system_actions() {
        // Mirror the nav cache invariant: before any redraw the
        // action cache is empty so the click handler gracefully
        // no-ops on out-of-bounds indices.
        let s = UnifiedReplState::new();
        assert!(s.system_actions.is_empty());
    }

    #[test]
    fn system_actions_cache_drives_dispatch_round_trip() {
        // Simulate what `on_action_invoked` does: read an action out
        // of the cache by index, hand it to `actions::dispatch_action`,
        // and observe the returned annotation line. Mirrors the
        // `nav_targets_cache_drives_set_current_cell_jump` shape.
        use crate::ui_apps::actions::{SystemAction, SystemVerb};

        let mut s = UnifiedReplState::new();
        s.system_actions = vec![
            SystemAction::new(
                SystemVerb::ApplyCreate,
                vec![("noun".to_string(), "File".to_string())],
            ),
            SystemAction::new(
                SystemVerb::Fetch,
                vec![("name".to_string(), "File_has_Name".to_string())],
            ),
        ];

        // Picking row 0 (ApplyCreate) returns an annotation that
        // reflects the canonical verb form.
        let action = s.system_actions[0].clone();
        let line = crate::ui_apps::actions::dispatch_action(&action);
        assert!(line.contains("apply create File"), "unexpected: {line}");
    }

    #[test]
    fn system_actions_out_of_bounds_index_no_op_safe() {
        let s = UnifiedReplState::new();
        assert!(s.system_actions.get(0).is_none());
        assert!(s.system_actions.get(usize::MAX).is_none());
    }

    // ---- State-machine action surface (#514) coverage --------------

    #[test]
    fn dispatch_blocked_transition_returns_blocked_annotation() {
        // The Slint side suppresses click delivery for disabled rows
        // (TouchArea.enabled = false), but defensive depth: when the
        // dispatch path runs anyway (stale event after state swap, or
        // a future REPL command that walks the same SystemAction
        // vector), `dispatch_action` short-circuits and returns the
        // blocked annotation rather than running the verb.
        use crate::ui_apps::actions::{GuardStatus, SystemAction, SystemVerb};

        let action = SystemAction::with_label_and_guard(
            SystemVerb::Transition,
            vec![
                ("sm".to_string(), "OrderSM".to_string()),
                ("id".to_string(), "o1".to_string()),
                ("next".to_string(), "submitted".to_string()),
                ("event".to_string(), "submit".to_string()),
            ],
            "[transition] submit (\u{2192} submitted) — disabled".to_string(),
            GuardStatus::BlockedByGuard("must be approved".to_string()),
        );
        let line = crate::ui_apps::actions::dispatch_action(&action);
        assert!(line.contains("blocked"), "{line}");
        assert!(line.contains("must be approved"), "{line}");
    }

    #[test]
    fn enabled_transition_dispatches_normally() {
        // Enabled actions take the normal "would dispatch" path even
        // when their verb is mutating (Transition is a mutating verb;
        // the foundation slice annotation is the expected response).
        use crate::ui_apps::actions::{SystemAction, SystemVerb};

        let action = SystemAction::new(
            SystemVerb::Transition,
            vec![
                ("sm".to_string(), "OrderSM".to_string()),
                ("id".to_string(), "o1".to_string()),
                ("next".to_string(), "submitted".to_string()),
            ],
        );
        let line = crate::ui_apps::actions::dispatch_action(&action);
        assert!(!line.contains("blocked"), "enabled action must not be blocked: {line}");
        assert!(line.contains("transition"), "{line}");
    }

    // ---- Navigation history integration (#516) coverage ----------

    #[test]
    fn new_state_seeds_breadcrumb_with_root() {
        // The constructor pushes Root so the persistent strip across
        // the top has something to render on first paint.
        let s = UnifiedReplState::new();
        assert_eq!(s.breadcrumb.len(), 1);
        let path = s.breadcrumb.current_path();
        assert!(path[0].is_current);
        assert_eq!(path[0].cell, CurrentCell::Root);
    }

    #[test]
    fn set_current_cell_pushes_to_breadcrumb_history() {
        // Navigation events flow through `set_current_cell`; each
        // call must contribute to the history trail so back / forward
        // can walk over it.
        let mut s = UnifiedReplState::new();
        s.set_current_cell(CurrentCell::Noun { noun: "File".into() });
        s.set_current_cell(CurrentCell::Instance {
            noun: "File".into(),
            instance: "f1".into(),
        });
        // Root (seeded) + Noun + Instance = 3 entries.
        assert_eq!(s.breadcrumb.len(), 3);
        assert!(s.breadcrumb.can_go_back());
        assert!(!s.breadcrumb.can_go_forward());
    }

    #[test]
    fn back_walks_breadcrumb_and_updates_current_cell() {
        let mut s = UnifiedReplState::new();
        s.set_current_cell(CurrentCell::Noun { noun: "File".into() });
        s.set_current_cell(CurrentCell::Instance {
            noun: "File".into(),
            instance: "f1".into(),
        });

        // Back to Noun.
        let prev = s.back();
        assert_eq!(prev, Some(CurrentCell::Noun { noun: "File".into() }));
        assert_eq!(s.current_cell, CurrentCell::Noun { noun: "File".into() });
        // History length unchanged — back walks the cursor, doesn't push.
        assert_eq!(s.breadcrumb.len(), 3);
        assert!(s.breadcrumb.can_go_forward());

        // Back to Root.
        let prev = s.back();
        assert_eq!(prev, Some(CurrentCell::Root));
        assert_eq!(s.current_cell, CurrentCell::Root);
        assert!(!s.breadcrumb.can_go_back());

        // At oldest — back is a no-op.
        assert_eq!(s.back(), None);
    }

    #[test]
    fn forward_walks_breadcrumb_back_to_tip() {
        let mut s = UnifiedReplState::new();
        s.set_current_cell(CurrentCell::Noun { noun: "File".into() });
        s.set_current_cell(CurrentCell::Instance {
            noun: "File".into(),
            instance: "f1".into(),
        });
        s.back();
        s.back();
        // Forward to Noun.
        let next = s.forward();
        assert_eq!(next, Some(CurrentCell::Noun { noun: "File".into() }));
        // Forward to Instance.
        let next = s.forward();
        assert_eq!(
            next,
            Some(CurrentCell::Instance {
                noun: "File".into(),
                instance: "f1".into()
            })
        );
        // At tip — forward is a no-op.
        assert_eq!(s.forward(), None);
    }

    #[test]
    fn navigate_after_back_clears_forward_stack() {
        // Browser-style: stepping back then navigating somewhere new
        // drops everything past the cursor.
        let mut s = UnifiedReplState::new();
        s.set_current_cell(CurrentCell::Noun { noun: "File".into() });
        s.set_current_cell(CurrentCell::Noun { noun: "Tag".into() });
        s.back();
        // Cursor at Noun(File). Navigate somewhere new.
        s.set_current_cell(CurrentCell::Noun { noun: "Component".into() });
        // History: Root, File, Component (Tag dropped).
        assert_eq!(s.breadcrumb.len(), 3);
        assert!(!s.breadcrumb.can_go_forward());
        let path = s.breadcrumb.current_path();
        assert_eq!(path[2].cell, CurrentCell::Noun { noun: "Component".into() });
        assert!(path[2].is_current);
    }

    #[test]
    fn bookmark_records_under_label_and_lookup_returns_cell() {
        let mut s = UnifiedReplState::new();
        s.set_current_cell(CurrentCell::Instance {
            noun: "File".into(),
            instance: "f1".into(),
        });
        s.bookmark("home".to_string());
        assert!(s.breadcrumb.has_bookmark("home"));
        assert_eq!(
            s.breadcrumb.goto_bookmark("home"),
            Some(CurrentCell::Instance {
                noun: "File".into(),
                instance: "f1".into()
            })
        );
    }

    #[test]
    fn goto_bookmark_jumps_and_pushes_to_history() {
        let mut s = UnifiedReplState::new();
        s.bookmark_cell(
            "saved".to_string(),
            CurrentCell::Instance {
                noun: "File".into(),
                instance: "f1".into(),
            },
        );
        let before_len = s.breadcrumb.len();
        let target = s.goto_bookmark("saved");
        assert_eq!(
            target,
            Some(CurrentCell::Instance {
                noun: "File".into(),
                instance: "f1".into()
            })
        );
        // Bookmark navigation IS a history event — pushed.
        assert_eq!(s.breadcrumb.len(), before_len + 1);
        assert!(s.breadcrumb.can_go_back());
    }

    #[test]
    fn goto_bookmark_unknown_label_is_no_op() {
        let mut s = UnifiedReplState::new();
        let before_len = s.breadcrumb.len();
        assert_eq!(s.goto_bookmark("doesnotexist"), None);
        assert_eq!(s.breadcrumb.len(), before_len);
    }

    #[test]
    fn back_does_not_push_to_breadcrumb() {
        // The cursor walks an existing entry; no fresh push.
        let mut s = UnifiedReplState::new();
        s.set_current_cell(CurrentCell::Noun { noun: "File".into() });
        let len_before = s.breadcrumb.len();
        s.back();
        assert_eq!(s.breadcrumb.len(), len_before);
    }

    #[test]
    fn nav_push_resource_selected_path_updates_breadcrumb() {
        // The HATEOAS-side flow drives navigation through `nav_push`
        // (not `set_current_cell`). It calls `sync_current_cell`
        // which we wired to push to breadcrumb when the cell actually
        // changes. Verify that path produces a history entry.
        let mut s = UnifiedReplState::new();
        let len_before = s.breadcrumb.len();
        s.nav_push(Breadcrumb::Noun { noun: "File".into() });
        assert_eq!(s.breadcrumb.len(), len_before + 1);
        assert_eq!(
            s.current_cell,
            CurrentCell::Noun { noun: "File".into() }
        );
    }

    #[test]
    fn submit_cell_nav_pushes_to_breadcrumb() {
        // The REPL `noun File` form routes through `set_current_cell`
        // which must push to history.
        let mut s = UnifiedReplState::new();
        let len_before = s.breadcrumb.len();
        s.submit("> ", "noun File".to_string());
        assert_eq!(s.breadcrumb.len(), len_before + 1);
        assert_eq!(s.current_cell, CurrentCell::Noun { noun: "File".into() });
    }
}
