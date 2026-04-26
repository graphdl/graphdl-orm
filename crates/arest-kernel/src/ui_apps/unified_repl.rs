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
use crate::ui_apps::cell_renderer::{self, CurrentCell, RenderedScreen};

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
}

impl UnifiedReplState {
    fn new() -> Self {
        Self {
            nav_stack: vec![Breadcrumb::Root],
            subscriber_id: None,
            scrollback: vec![WELCOME.to_string()],
            history: Vec::new(),
            history_idx: None,
            pending_input: String::new(),
            current_cell: CurrentCell::Root,
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
    fn sync_current_cell(&mut self) {
        self.current_cell = match self.current_nav() {
            Breadcrumb::Root => CurrentCell::Root,
            Breadcrumb::Noun { noun } => CurrentCell::Noun { noun: noun.clone() },
            Breadcrumb::Instance { noun, instance } => CurrentCell::Instance {
                noun: noun.clone(),
                instance: instance.clone(),
            },
        };
    }

    /// Set the current cell directly without going through breadcrumb
    /// navigation. Used when REPL input resolves to a specific cell
    /// (e.g. `cell Foo_has_Bar` would jump to a FactCell). The
    /// breadcrumb trail is also updated to keep the left-pane visual
    /// in sync; FactCell / ComponentInstance variants don't have a
    /// breadcrumb mapping and just leave the trail as-is.
    fn set_current_cell(&mut self, cell: CurrentCell) {
        self.current_cell = cell.clone();
        match cell {
            CurrentCell::Root => {
                self.nav_stack.truncate(1);
            }
            CurrentCell::Noun { noun } => {
                self.nav_stack.truncate(1);
                self.nav_stack.push(Breadcrumb::Noun { noun });
            }
            CurrentCell::Instance { noun, instance } => {
                self.nav_stack.truncate(1);
                self.nav_stack.push(Breadcrumb::Noun { noun: noun.clone() });
                self.nav_stack.push(Breadcrumb::Instance { noun, instance });
            }
            // FactCell + ComponentInstance: keep nav_stack as-is —
            // the breadcrumb model can't represent them today, but
            // current_cell carries the truth for the typed surface.
            // #512 will extend the breadcrumb to cover these.
            CurrentCell::FactCell { .. } | CurrentCell::ComponentInstance { .. } => {}
        }
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
        if let Some(cell) = parse_cell_nav(trimmed) {
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
fn redraw(window: &UnifiedRepl, ui: &UnifiedReplState) {
    let snapshot = crate::system::with_state(|s| Snapshot::collect(s, ui));
    let snap = snapshot.unwrap_or_else(Snapshot::empty);

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

    // ---- Combined status footer ----
    let combined_status = format!(
        "{} \u{2022} {} repl line(s) \u{2022} Up/Down history \u{2022} Ctrl+L clear \u{2022} Esc back",
        snap.hateoas_status,
        ui.scrollback.len(),
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
    redraw(&window, &state.borrow());
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
            redraw(&window, &state.borrow());
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
            redraw(&window, &state.borrow());
        });
    }

    // Back button — pop one level off the breadcrumb stack.
    {
        let weak = window.as_weak();
        let state = state.clone();
        window.on_back_clicked(move || {
            let Some(window) = weak.upgrade() else { return };
            state.borrow_mut().nav_pop();
            redraw(&window, &state.borrow());
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
            redraw(&window, &state.borrow());
        });
    }

    // Ctrl+L — wholesale scrollback clear.
    {
        let weak = window.as_weak();
        let state = state.clone();
        window.on_clear(move || {
            let Some(window) = weak.upgrade() else { return };
            state.borrow_mut().clear_scrollback();
            redraw(&window, &state.borrow());
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
            redraw(&window, &state_rc.borrow());
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
}
