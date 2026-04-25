// crates/arest-kernel/src/ui_apps/registry.rs
//
// Slint adapter — runtime registration of Component facts (#486 Track FFFF).
//
// DDDD's #485 (commit 80f62ae) landed `readings/ui/components.md`
// declaring 12 well-known Components × 4 Toolkits = ~40
// ImplementationBinding instance facts as STATIC text in the readings
// stream. Static text alone doesn't populate the live SYSTEM cells
// — at boot the kernel needs to actually emit `Component`,
// `ImplementationBinding`, `ComponentProperty`, `ComponentEvent`, and
// `ComponentTrait` facts so the selection logic in #492 has cells to
// query at runtime.
//
// This module is the lowest-risk adapter slice: Slint is already
// kernel-resident (no linuxkpi shim involved), so we just walk the
// MMM #436 component surface (`ui/components/{Button,Input,Card,
// List,Dialog}.slint`) plus the four launcher app symbols
// (`ui/apps/{AppLauncher,HateoasBrowser,Repl,Doom}.slint` from
// SSS/TTT/UUU/VVV) and emit the FORML 2 facts that mirror the
// readings.
//
// # Spec coverage
//
// 9 of the 12 Component cells declared in DDDD's reading have Slint
// implementations:
//   * Button (`Button`)
//   * TextInput (`Input`)
//   * ListView (`List`)
//   * Card (`Card`)
//   * Dialog (`Dialog`)
//   * Image — ships in Slint's std prelude as `Image`
//   * Slider — ships in Slint's std prelude as `Slider`
//   * ProgressBar — ships in Slint's std prelude as `ProgressIndicator`
//   * CheckBox — ships in Slint's std prelude as `CheckBox`
//
// 3 are intentionally absent in this slice (date-picker, combo-box,
// tab); for those we emit a `Notice` fact pointing #492's
// selection / gap-detection logic at the missing implementation. A
// future track can fill in the gap by adding a `.slint` component
// and seeding bindings here.
//
// # Fact shape
//
// Mirrors `readings/ui/components.md` exactly:
//
//   Component_has_ComponentRole          { Component, ComponentRole }
//   Component_has_displayTitle           { Component, displayTitle }
//   Component_has_Description            { Component, Description }
//   Component_is_implemented_by_Toolkit_at_ToolkitSymbol
//                                        { Component, Toolkit, ToolkitSymbol }
//   ImplementationBinding_pivots_Component_is_implemented_by_Toolkit
//                                        { ImplementationBinding, Component, Toolkit }
//   Component_has_Property_of_PropertyType_with_PropertyDefault
//                                        { Component, PropertyName, PropertyType, PropertyDefault }
//   Component_emits_Event_with_EventPayloadType
//                                        { Component, EventName, EventPayloadType }
//   Component_has_Slot                   { Component, SlotName }
//   Component_has_Trait                  { Component, ComponentTrait }
//   ImplementationBinding_has_Trait      { ImplementationBinding, ComponentTrait }
//   Notice_has_NoticeText                { Notice, NoticeText }
//   ComponentRole_requires_Notice        { ComponentRole, Notice }
//
// The cell names follow the `<Noun>_has_<Attr>` convention that
// `ast::cells_iter` + the HATEOAS browser (#429) recognise; the
// HATEOAS browser will surface every Component / ImplementationBinding
// instance under its sidebar after `register_slint_components()` runs.
//
// # Wiring
//
// `register_slint_components()` is `pub fn` so the call site can
// land independently of this commit. The natural call site is
// `entry_uefi.rs::kernel_run_uefi` (after `system::init()` and
// before the launcher super-loop starts) but that file is owned by
// Track EEEE in #464; the call site lands in a follow-up once both
// EEEE and GGGG (which has `main.rs` for #487) merge. Until the
// call site is wired, the function is dormant — but every Slint
// component constructor still works against the empty registry.
//
// # Property / event extraction
//
// MMM's `.slint` files are hand-read once and the property /
// callback names hardcoded into the per-component builders below.
// A `.slint` parser doesn't exist on the host build path (slint-build
// emits Rust, not a parseable AST we can re-walk), and the property
// surface changes infrequently — comments tag each component with
// the source `.slint` path so future audits catch drift.

#![allow(dead_code)]

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use arest::ast::{cell_push, fact_from_pairs, Object};

/// Toolkit slug constant — matches `Toolkit Slug` value-type
/// enumeration in `readings/ui/components.md` line 76.
const TOOLKIT_SLINT: &str = "slint";

/// One canonical Slint Component to seed at boot. Internal
/// shape used only by `slint_components()` to build the fact
/// stream below.
struct ComponentSpec {
    /// Component identifier (FORML role-instance name).
    /// Lowercase slug matching `Component Role` enumeration in
    /// `components.md` line 65.
    name: &'static str,
    /// Component Role enumeration value. Always equal to `name`
    /// for the well-known seeded components.
    role: &'static str,
    /// `display- Title` value (UI-visible label).
    display_title: &'static str,
    /// `Description` value (one-line summary).
    description: &'static str,
    /// `Toolkit Symbol` for the Slint binding — the .slint type
    /// name the adapter resolves at instantiation time.
    slint_symbol: &'static str,
    /// `(name, type, default)` triples for each `in` / `in-out`
    /// property declared on the .slint component.
    properties: &'static [(&'static str, &'static str, &'static str)],
    /// `(name, payload_type)` pairs for each `callback` declared.
    events: &'static [(&'static str, &'static str)],
    /// Slot names exposed (matches `readings/ui/components.md`'s
    /// `Component has Slot` facts).
    slots: &'static [&'static str],
    /// Universal traits applied to the Component (toolkit-agnostic).
    component_traits: &'static [&'static str],
    /// Slint-binding-specific traits that ride the
    /// `ImplementationBinding has Trait` ternary.
    binding_traits: &'static [&'static str],
}

/// Hard-coded Component specs for the 9 Slint-implemented roles
/// from DDDD's #485 reading. Read from MMM's `.slint` files —
/// each entry comments its source path so a future audit catches
/// drift.
fn slint_components() -> Vec<ComponentSpec> {
    vec![
        // From `ui/components/Button.slint` — `in property <string>
        // text`, `in property <ButtonVariant> variant`, `in property
        // <bool> enabled`, `callback clicked()`. The reading's
        // `primary` boolean is collapsed into the `variant` enum on
        // the Slint side (Button.slint line 21); we keep the
        // reading's name (`primary`) and type (`bool`) so the
        // selection logic doesn't see a divergent surface — the
        // adapter (#491) is responsible for projecting `primary:
        // true` onto `variant: ButtonVariant.primary`.
        ComponentSpec {
            name: "button",
            role: "button",
            display_title: "Button",
            description: "Plain push button — primary control for triggering an action.",
            slint_symbol: "Button",
            properties: &[
                ("text", "string", ""),
                ("enabled", "bool", "true"),
                ("primary", "bool", "false"),
            ],
            events: &[("clicked", "none")],
            slots: &["leading", "trailing"],
            component_traits: &["keyboard_navigable", "theming_consumer"],
            binding_traits: &["kernel_native", "hidpi_native"],
        },
        // From `ui/components/Input.slint` — `in-out property
        // <string> text`, `in property <string> placeholder`, `in
        // property <bool> read-only`, `callback edited(string)`,
        // `callback accepted(string)`. The reading uses `enabled`
        // and `maxlength`; the .slint surface uses `read-only` for
        // disable semantics and exposes `password` for masked
        // input. We follow the reading's nominal shape (so #492
        // sees the canonical shape across toolkits) — the adapter
        // (#491) is responsible for projecting `enabled: false`
        // onto `read-only: true`. The reading's `submitted` event
        // maps to Slint's `accepted`.
        ComponentSpec {
            name: "text-input",
            role: "text-input",
            display_title: "Text Input",
            description: "Single-line text entry field.",
            slint_symbol: "Input",
            properties: &[
                ("text", "string", ""),
                ("placeholder", "string", ""),
                ("enabled", "bool", "true"),
                ("maxlength", "int", "0"),
            ],
            events: &[
                ("changed", "string"),
                ("submitted", "string"),
            ],
            slots: &["leading", "trailing"],
            component_traits: &["keyboard_navigable", "theming_consumer"],
            binding_traits: &["kernel_native"],
        },
        // From `ui/components/List.slint` — `in property <[string]>
        // items`, `in property <int> selected-index`, `callback
        // item-clicked(int, string)`. The reading's event name is
        // `selection-changed` — the adapter renames `item-clicked`
        // to fit the canonical surface.
        ComponentSpec {
            name: "list",
            role: "list",
            display_title: "List View",
            description: "Vertically-scrolling list of homogeneous items.",
            slint_symbol: "List",
            properties: &[
                ("items", "string", ""),
                ("selected", "int", "-1"),
            ],
            events: &[("selection-changed", "int")],
            slots: &["children", "header", "footer"],
            component_traits: &["keyboard_navigable", "theming_consumer"],
            binding_traits: &["kernel_native"],
        },
        // From `ui/components/Card.slint` — `in property <string>
        // title`. The reading uses `elevation` and `padding` which
        // the Slint Card declares via inline `Theme` token reads
        // rather than as in-properties. We follow the reading
        // surface; the adapter (#491) maps the abstract properties
        // onto Card's surface shape (no-op on `elevation`, set the
        // VerticalLayout `padding` for `padding`).
        ComponentSpec {
            name: "card",
            role: "card",
            display_title: "Card",
            description: "Surfaced container with optional header / footer chrome. The Slint binding is the MMM #436 stock card.",
            slint_symbol: "Card",
            properties: &[
                ("elevation", "int", "1"),
                ("padding", "length", "16"),
            ],
            events: &[],
            slots: &["children", "header", "footer"],
            component_traits: &["theming_consumer"],
            binding_traits: &["kernel_native", "hidpi_native"],
        },
        // From `ui/components/Dialog.slint` — `in property <bool>
        // open`, `in property <string> title`, `callback
        // confirmed()`, `callback cancelled()`. The reading uses
        // `closed` instead of `cancelled`; we keep the reading's
        // name. The adapter renames cancelled <-> closed.
        ComponentSpec {
            name: "dialog",
            role: "dialog",
            display_title: "Dialog",
            description: "Modal overlay window for transient interaction (confirm, alert, form-on-overlay).",
            slint_symbol: "Dialog",
            properties: &[
                ("title", "string", ""),
                ("open", "bool", "false"),
            ],
            events: &[
                ("closed", "none"),
                ("confirmed", "none"),
            ],
            slots: &["children", "footer"],
            component_traits: &["keyboard_navigable", "theming_consumer"],
            binding_traits: &["kernel_native"],
        },
        // From slint's standard widget set — `Image` is the stock
        // image element. The kernel uses it via the slint compiled
        // surfaces (e.g. Doom.slint's `Image { source: frame; }`).
        // No MMM .slint wrapper exists today; the binding points
        // straight at slint::Image.
        ComponentSpec {
            name: "image",
            role: "image",
            display_title: "Image",
            description: "Static raster or vector image. Qt 6 reuses QLabel + pixmap because QImage is the data type, not the widget.",
            slint_symbol: "Image",
            properties: &[
                ("source", "image", ""),
                ("fit", "enum", "contain"),
            ],
            events: &[],
            slots: &[],
            component_traits: &["theming_consumer"],
            binding_traits: &["kernel_native", "hidpi_native"],
        },
        // From slint's standard widget set — `Slider`. No MMM
        // wrapper today; the binding points at the slint stdlib.
        ComponentSpec {
            name: "slider",
            role: "slider",
            display_title: "Slider",
            description: "Continuous numeric value selection along a track. Slint binding name is the expected MMM #436 surface; #486 will TODO if missing.",
            slint_symbol: "Slider",
            properties: &[
                ("value", "int", "0"),
                ("minimum", "int", "0"),
                ("maximum", "int", "100"),
            ],
            events: &[("changed", "int")],
            slots: &[],
            component_traits: &["keyboard_navigable", "theming_consumer"],
            binding_traits: &["kernel_native"],
        },
        // From slint's standard widget set — `ProgressIndicator`.
        // The reading explicitly cites `ProgressIndicator` as the
        // Slint Symbol (components.md line 672) so this matches
        // the canonical reading surface verbatim.
        ComponentSpec {
            name: "progress-bar",
            role: "progress-bar",
            display_title: "Progress Bar",
            description: "Linear progress indicator with optional indeterminate mode.",
            slint_symbol: "ProgressIndicator",
            properties: &[
                ("value", "int", "0"),
                ("maximum", "int", "100"),
                ("indeterminate", "bool", "false"),
            ],
            events: &[],
            slots: &[],
            component_traits: &["theming_consumer"],
            binding_traits: &["kernel_native"],
        },
        // From slint's standard widget set — `CheckBox`. The MMM
        // surface doesn't wrap it (no `ui/components/CheckBox.slint`
        // today), but the slint stdlib `CheckBox` is reachable
        // from any compiled .slint without an extra import.
        ComponentSpec {
            name: "checkbox",
            role: "checkbox",
            display_title: "Check Box",
            description: "Bistate (or tristate) toggle bound to a label.",
            slint_symbol: "CheckBox",
            properties: &[
                ("checked", "bool", "false"),
                ("label", "string", ""),
                ("enabled", "bool", "true"),
            ],
            events: &[("toggled", "bool")],
            slots: &[],
            component_traits: &["keyboard_navigable", "theming_consumer"],
            binding_traits: &["kernel_native"],
        },
    ]
}

/// One launcher app instance to register as a `LaunchableApp` fact.
/// These are the four Slint Window-derived components that act as
/// app surfaces under the launcher (Track UUU #431). Each one
/// already has an ImplementationBinding in DDDD's reading (the
/// `<app>.slint` binding is the launcher app symbol); this entry
/// just emits a `LaunchableApp_has_Symbol` fact at boot so #492's
/// selection rules see what's actually loadable.
struct LauncherAppSpec {
    /// App identifier slug (matches `crate::ui_apps::*` module
    /// names by convention).
    name: &'static str,
    /// Slint Window component type-name (the `export component
    /// <Symbol> inherits Window {}` declaration in the .slint).
    slint_symbol: &'static str,
    /// One-line description used by future menu surfaces.
    description: &'static str,
}

/// Hard-coded launcher app specs. The four .slint files under
/// `ui/apps/` define these. Each one is a Window-derived
/// component instantiated by the matching `crate::ui_apps::<name>::
/// build_app()` (or `crate::ui_apps::launcher::run` for
/// AppLauncher).
fn launcher_apps() -> Vec<LauncherAppSpec> {
    vec![
        // From `ui/apps/AppLauncher.slint` — splash + app launcher.
        // SSS/TTT/UUU's #431 anchor.
        LauncherAppSpec {
            name: "app-launcher",
            slint_symbol: "AppLauncher",
            description: "Boot UI splash with app picker (#431, Track UUU).",
        },
        // From `ui/apps/HateoasBrowser.slint` — three-pane
        // resource browser (Track SSS #429).
        LauncherAppSpec {
            name: "hateoas-browser",
            slint_symbol: "HateoasBrowser",
            description: "Three-pane resource browser over the live SYSTEM state (#429, Track SSS).",
        },
        // From `ui/apps/Repl.slint` — REPL toolkit (Track TTT
        // #430). Scrollback + history line editor.
        LauncherAppSpec {
            name: "repl",
            slint_symbol: "Repl",
            description: "REPL with scrollback + history (#430, Track TTT).",
        },
        // From `ui/apps/Doom.slint` — Doom WASM guest surface
        // (Track VVV #455 + #456). Only loadable when
        // `--features doom` is enabled at build time.
        LauncherAppSpec {
            name: "doom",
            slint_symbol: "Doom",
            description: "Doom WASM guest surface (#455 + #456, Track VVV; --features doom).",
        },
    ]
}

/// One Component Role with no Slint implementation. The
/// gap-detection rule in `readings/ui/components.md` (line
/// 339-348) emits a `Component Role requires Notice` fact when
/// no binding exists; we mirror that at runtime so the cell is
/// visible to #492 even before the rule engine fires.
struct MissingSpec {
    /// Component Role enumeration value (no Slint impl).
    role: &'static str,
    /// Slug of the `Notice` instance to emit alongside.
    notice_slug: &'static str,
    /// User-facing notice body.
    notice_text: &'static str,
}

/// The three Component Roles seeded by DDDD's reading that have
/// no Slint binding today. A future track that adds e.g. a
/// `DateView.slint` deletes the matching entry here.
fn missing_slint_roles() -> &'static [MissingSpec] {
    &[
        MissingSpec {
            role: "date-picker",
            notice_slug: "todo-slint-date-picker",
            notice_text: "TODO: Slint implementation missing for Component Role 'date-picker'. The Qt 6 / GTK 4 / Web Components bindings cover this role; #486 + future track will add a kernel-native Slint DateView.",
        },
        MissingSpec {
            role: "combo-box",
            notice_slug: "todo-slint-combo-box",
            notice_text: "TODO: Slint implementation missing for Component Role 'combo-box'. Qt 6 / GTK 4 / Web Components cover this role; future track adds the Slint ComboBox.",
        },
        MissingSpec {
            role: "tab",
            notice_slug: "todo-slint-tab",
            notice_text: "TODO: Slint implementation missing for Component Role 'tab'. Qt 6 / GTK 4 cover this role; future track adds the Slint TabBar.",
        },
    ]
}

/// Build the new SYSTEM state by layering Slint Component facts
/// on top of `state`. Pure function — the caller decides whether
/// to commit via `system::apply` (production wiring) or to
/// inspect (the test harness).
///
/// The builder follows the standard `cell_push` chain shape used
/// elsewhere in the kernel (compare `file_upload::build_file_facts`
/// at `file_upload.rs::1450`). Each fact threads through one
/// `cell_push` call; the resulting Object is the next argument.
/// The order within a Component is:
///   1. Component_has_ComponentRole (the anchor)
///   2. Component_has_displayTitle
///   3. Component_has_Description
///   4. Component_is_implemented_by_Toolkit_at_ToolkitSymbol
///   5. ImplementationBinding_pivots_Component_is_implemented_by_Toolkit
///   6. Component_has_Property_of_PropertyType_with_PropertyDefault (xN)
///   7. Component_emits_Event_with_EventPayloadType (xM)
///   8. Component_has_Slot (xK)
///   9. Component_has_Trait (xT)
///   10. ImplementationBinding_has_Trait (xT)
///
/// Roles missing a Slint impl emit:
///   * Notice_has_NoticeText
///   * ComponentRole_requires_Notice
///
/// Launcher apps emit:
///   * LaunchableApp_has_Symbol
///   * LaunchableApp_has_Description
pub fn build_slint_component_state(state: &Object) -> Object {
    let mut new_state = state.clone();

    // --- Components --------------------------------------------------
    for spec in slint_components() {
        new_state = push_component(&spec, &new_state);
    }

    // --- Missing-role notices ---------------------------------------
    for missing in missing_slint_roles() {
        new_state = push_missing(missing, &new_state);
    }

    // --- Launcher apps ----------------------------------------------
    for app in launcher_apps() {
        new_state = push_launcher_app(&app, &new_state);
    }

    new_state
}

/// Layer one Component's full fact stream onto `state`. See the
/// `build_slint_component_state` doc-comment for the ordering.
fn push_component(spec: &ComponentSpec, state: &Object) -> Object {
    let binding_id = component_binding_id(spec.name);

    // 1. Component_has_ComponentRole
    let s = cell_push(
        "Component_has_ComponentRole",
        fact_from_pairs(&[("Component", spec.name), ("ComponentRole", spec.role)]),
        state,
    );

    // 2. Component_has_displayTitle
    let s = cell_push(
        "Component_has_displayTitle",
        fact_from_pairs(&[("Component", spec.name), ("displayTitle", spec.display_title)]),
        &s,
    );

    // 3. Component_has_Description
    let s = cell_push(
        "Component_has_Description",
        fact_from_pairs(&[("Component", spec.name), ("Description", spec.description)]),
        &s,
    );

    // 4. The ternary: Component is implemented by Toolkit at Toolkit Symbol.
    let s = cell_push(
        "Component_is_implemented_by_Toolkit_at_ToolkitSymbol",
        fact_from_pairs(&[
            ("Component", spec.name),
            ("Toolkit", TOOLKIT_SLINT),
            ("ToolkitSymbol", spec.slint_symbol),
        ]),
        &s,
    );

    // 5. The pivot: ImplementationBinding pivots the ternary.
    let s = cell_push(
        "ImplementationBinding_pivots_Component_is_implemented_by_Toolkit",
        fact_from_pairs(&[
            ("ImplementationBinding", &binding_id),
            ("Component", spec.name),
            ("Toolkit", TOOLKIT_SLINT),
        ]),
        &s,
    );

    // 6. Properties (Component, PropertyName, PropertyType, PropertyDefault).
    let mut s = s;
    for (name, ty, default) in spec.properties {
        s = cell_push(
            "Component_has_Property_of_PropertyType_with_PropertyDefault",
            fact_from_pairs(&[
                ("Component", spec.name),
                ("PropertyName", name),
                ("PropertyType", ty),
                ("PropertyDefault", default),
            ]),
            &s,
        );
    }

    // 7. Events (Component, EventName, EventPayloadType).
    for (name, payload) in spec.events {
        s = cell_push(
            "Component_emits_Event_with_EventPayloadType",
            fact_from_pairs(&[
                ("Component", spec.name),
                ("EventName", name),
                ("EventPayloadType", payload),
            ]),
            &s,
        );
    }

    // 8. Slots (Component, SlotName).
    for slot in spec.slots {
        s = cell_push(
            "Component_has_Slot",
            fact_from_pairs(&[("Component", spec.name), ("SlotName", slot)]),
            &s,
        );
    }

    // 9. Component-level traits (universal across every toolkit
    //    implementation of this role).
    for trait_name in spec.component_traits {
        s = cell_push(
            "Component_has_Trait",
            fact_from_pairs(&[("Component", spec.name), ("ComponentTrait", trait_name)]),
            &s,
        );
    }

    // 10. Binding-scoped traits (Slint-implementation-specific).
    for trait_name in spec.binding_traits {
        s = cell_push(
            "ImplementationBinding_has_Trait",
            fact_from_pairs(&[
                ("ImplementationBinding", &binding_id),
                ("ComponentTrait", trait_name),
            ]),
            &s,
        );
    }

    s
}

/// Emit the Notice + Component-Role-requires-Notice pair for a
/// Slint-missing role.
fn push_missing(missing: &MissingSpec, state: &Object) -> Object {
    let s = cell_push(
        "Notice_has_NoticeText",
        fact_from_pairs(&[
            ("Notice", missing.notice_slug),
            ("NoticeText", missing.notice_text),
        ]),
        state,
    );
    cell_push(
        "ComponentRole_requires_Notice",
        fact_from_pairs(&[
            ("ComponentRole", missing.role),
            ("Notice", missing.notice_slug),
        ]),
        &s,
    )
}

/// Emit the LaunchableApp facts for one app surface.
fn push_launcher_app(app: &LauncherAppSpec, state: &Object) -> Object {
    let s = cell_push(
        "LaunchableApp_has_Symbol",
        fact_from_pairs(&[("LaunchableApp", app.name), ("Symbol", app.slint_symbol)]),
        state,
    );
    cell_push(
        "LaunchableApp_has_Description",
        fact_from_pairs(&[("LaunchableApp", app.name), ("Description", app.description)]),
        &s,
    )
}

/// Build the canonical `<component>.slint` slug used as the
/// `ImplementationBinding` reference-mode value. Matches DDDD's
/// reading convention (see `components.md` line 395 — `'button.
/// slint'`, line 437 — `'text-input.slint'`, etc.).
fn component_binding_id(component_name: &str) -> String {
    let mut s = component_name.to_string();
    s.push_str(".slint");
    s
}

/// Public entry: register every Slint Component fact on top of
/// the live SYSTEM state via `crate::system::apply`.
///
/// Returns `Err` only when `system::init()` hasn't run yet (a
/// programmer error — the call site must order this after init).
/// Idempotent in spirit but not in the cell-content sense:
/// re-calling appends duplicate facts because `cell_push` is
/// O(1) append, not set-semantics. The launcher's call site
/// runs this once at boot.
///
/// Wiring note: this `pub fn` is currently dormant — no caller
/// invokes it in this commit (the natural call sites in
/// `entry_uefi.rs::kernel_run_uefi` and `main.rs` are owned by
/// other tracks). Once Track EEEE (#464) and Track GGGG (#487)
/// land, a one-line follow-up wires this in next to
/// `system::init()`.
pub fn register_slint_components() -> Result<(), &'static str> {
    let new_state = crate::system::with_state(|s| build_slint_component_state(s))
        .ok_or("system::init() not called before register_slint_components()")?;
    crate::system::apply(new_state)
}

// ── Tests ─────────────────────────────────────────────────────────
//
// `arest-kernel`'s bin target has `test = false` (Cargo.toml L33),
// so these `#[cfg(test)]` cases are reachable only when the crate
// is re-shaped into a lib for hosted testing — the same pattern
// `system.rs`, `file_serve.rs`, and `slint_backend.rs` use. They
// document the intended behaviour and form a smoke battery for
// the day the kernel grows a lib facade.
//
// The pure builder (`build_slint_component_state`) is exercised
// against `Object::phi()` so no `system::init()` is needed.

#[cfg(test)]
mod tests {
    use super::*;
    use arest::ast::{self, fetch_or_phi};

    /// Helper: count facts in a cell.
    fn cell_count(state: &Object, cell: &str) -> usize {
        fetch_or_phi(cell, state)
            .as_seq()
            .map(|v| v.len())
            .unwrap_or(0)
    }

    /// Spec assertion: 9 Slint components are seeded — Button,
    /// TextInput, ListView, Card, Dialog, Image, Slider,
    /// ProgressBar, CheckBox. Matches DDDD's #485 reading minus
    /// the 3 missing roles.
    #[test]
    fn nine_slint_components_emit_anchor_facts() {
        let state = build_slint_component_state(&Object::phi());
        // Each Component contributes exactly one
        // Component_has_ComponentRole fact (its anchor).
        assert_eq!(cell_count(&state, "Component_has_ComponentRole"), 9);
    }

    /// Each of the 9 Slint components has exactly one
    /// ImplementationBinding fact rooted at Toolkit slint.
    #[test]
    fn nine_implementation_bindings_for_toolkit_slint() {
        let state = build_slint_component_state(&Object::phi());
        assert_eq!(
            cell_count(&state, "Component_is_implemented_by_Toolkit_at_ToolkitSymbol"),
            9
        );
        assert_eq!(
            cell_count(&state, "ImplementationBinding_pivots_Component_is_implemented_by_Toolkit"),
            9
        );
    }

    /// Total property facts across the 9 components.
    /// 3 (button) + 4 (text-input) + 2 (list) + 2 (card) + 2 (dialog)
    /// + 2 (image) + 3 (slider) + 3 (progress-bar) + 3 (checkbox)
    /// = 24. The exact count fence-posts against accidental drift
    /// when MMM extends a .slint surface.
    #[test]
    fn property_fact_count_matches_extracted_surface() {
        let state = build_slint_component_state(&Object::phi());
        assert_eq!(
            cell_count(&state, "Component_has_Property_of_PropertyType_with_PropertyDefault"),
            24
        );
    }

    /// Total event facts.
    /// 1 (button.clicked) + 2 (text-input changed/submitted)
    /// + 1 (list.selection-changed) + 0 (card)
    /// + 2 (dialog closed/confirmed) + 0 (image)
    /// + 1 (slider.changed) + 0 (progress-bar)
    /// + 1 (checkbox.toggled)
    /// = 8.
    #[test]
    fn event_fact_count_matches_extracted_surface() {
        let state = build_slint_component_state(&Object::phi());
        assert_eq!(
            cell_count(&state, "Component_emits_Event_with_EventPayloadType"),
            8
        );
    }

    /// Three TODO Notices for date-picker / combo-box / tab.
    #[test]
    fn three_missing_slint_roles_emit_notices() {
        let state = build_slint_component_state(&Object::phi());
        assert_eq!(cell_count(&state, "Notice_has_NoticeText"), 3);
        assert_eq!(cell_count(&state, "ComponentRole_requires_Notice"), 3);
    }

    /// Four launcher apps emit the LaunchableApp facts.
    #[test]
    fn four_launcher_apps_register_symbols() {
        let state = build_slint_component_state(&Object::phi());
        assert_eq!(cell_count(&state, "LaunchableApp_has_Symbol"), 4);
        assert_eq!(cell_count(&state, "LaunchableApp_has_Description"), 4);
    }

    /// Sanity: the Button binding ID matches DDDD's reading
    /// convention (`'button.slint'` — `components.md` line 395).
    #[test]
    fn binding_id_matches_reading_convention() {
        assert_eq!(component_binding_id("button"), "button.slint");
        assert_eq!(component_binding_id("text-input"), "text-input.slint");
        assert_eq!(component_binding_id("progress-bar"), "progress-bar.slint");
    }

    /// Spec smoke: the Button anchor in the produced state has
    /// the right ComponentRole and Toolkit Symbol bindings.
    #[test]
    fn button_anchor_has_correct_bindings() {
        let state = build_slint_component_state(&Object::phi());
        let role_cell = fetch_or_phi("Component_has_ComponentRole", &state);
        let role_facts = role_cell.as_seq().expect("seq");
        let button_role = role_facts.iter().find(|f| {
            ast::binding(f, "Component") == Some("button")
        }).expect("button anchor present");
        assert_eq!(ast::binding(button_role, "ComponentRole"), Some("button"));

        let bind_cell = fetch_or_phi("Component_is_implemented_by_Toolkit_at_ToolkitSymbol", &state);
        let bind_facts = bind_cell.as_seq().expect("seq");
        let button_bind = bind_facts.iter().find(|f| {
            ast::binding(f, "Component") == Some("button")
                && ast::binding(f, "Toolkit") == Some("slint")
        }).expect("button slint binding present");
        assert_eq!(ast::binding(button_bind, "ToolkitSymbol"), Some("Button"));
    }

    /// Spec smoke: the Card binding has both `kernel_native` and
    /// `hidpi_native` traits — mirrors `components.md` line 530-
    /// 531.
    #[test]
    fn card_binding_has_kernel_native_and_hidpi_native_traits() {
        let state = build_slint_component_state(&Object::phi());
        let trait_cell = fetch_or_phi("ImplementationBinding_has_Trait", &state);
        let trait_facts = trait_cell.as_seq().expect("seq");
        let card_traits: Vec<&str> = trait_facts.iter().filter_map(|f| {
            if ast::binding(f, "ImplementationBinding") == Some("card.slint") {
                ast::binding(f, "ComponentTrait")
            } else {
                None
            }
        }).collect();
        assert!(card_traits.contains(&"kernel_native"), "got {card_traits:?}");
        assert!(card_traits.contains(&"hidpi_native"), "got {card_traits:?}");
    }

    /// Idempotent layering: the builder is a pure function over
    /// the input state — running it twice doubles the fact count
    /// (we use `cell_push`, not `cell_push_unique`, so callers can
    /// audit re-registration if it ever happens). This test
    /// pins the spec so a future hidden `cell_push_unique` swap
    /// is caught.
    #[test]
    fn double_application_doubles_facts() {
        let once = build_slint_component_state(&Object::phi());
        let twice = build_slint_component_state(&once);
        assert_eq!(cell_count(&twice, "Component_has_ComponentRole"), 18);
    }
}
