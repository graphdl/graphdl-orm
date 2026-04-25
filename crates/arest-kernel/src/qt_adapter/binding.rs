// crates/arest-kernel/src/qt_adapter/binding.rs
//
// `register_qt_components()` — build the runtime fact set that mirrors
// DDDD's #485 static declarations in `readings/ui/components.md` for
// the Qt 6 toolkit, and apply it to SYSTEM via `system::apply`.
//
// What we register
// ----------------
// For each Qt 6 widget class in `widgets::QT_WIDGET_TABLE` we emit
// the same fact shapes DDDD's #485 declared:
//
//   * Component cell (Component_has_Role, display- Title, Description)
//     — the abstract widget category. These are shared with FFFF's
//     #486 Slint registration so we use `cell_push_unique` to avoid
//     duplicating the Component facts when both adapters init.
//   * ImplementationBinding cell (Component is implemented by Toolkit
//     at Toolkit Symbol) — the (Component, Toolkit, Symbol) triple
//     pinning Qt's class name as the Symbol.
//   * Component property facts (text:string default '', enabled:bool
//     default true, primary:bool default false, …) per DDDD's
//     declarations.
//   * Component event facts (clicked:none, changed:string, …) per
//     DDDD's declarations.
//   * Component trait facts (keyboard_navigable, theming_consumer)
//     and ImplementationBinding-scoped trait facts
//     (screen_reader_aware on every Qt binding because QAccessible
//     is wired across the widget set; hidpi_native on QPushButton
//     because Qt 6.0+ ships HiDPI-clean by default).
//
// Toolkit row
// -----------
// We emit the `qt6` Toolkit row (Toolkit_has_Slug 'qt6',
// Toolkit_has_Version '6.6', display- Title 'Qt 6') once. Same Slint
// rule applies — `cell_push_unique` makes it safe for FFFF's #486 to
// emit its own Toolkit 'slint' row in parallel without duplication.
//
// Selection-rule consumption
// --------------------------
// The registered facts feed the derivation rules at the bottom of
// `readings/ui/components.md` (touch density preference, screen-
// reader / GTK preference). On the foundation slice the SLINT
// implementations win because the Slint binding has the
// `kernel_native` trait DDDD's #485 attached and the rule library
// (#492) hasn't yet shipped a Qt-specific preference rule. After
// #489 wires the compositor + #492 expands the rule library, an
// AI-driven `select_component` query (#493) can pick Qt over Slint
// on a per-context basis.

use alloc::string::String;
use arest::ast::{fact_from_pairs, cell_push_unique};

use crate::system;

use super::widgets;

/// Component declaration mirroring one of DDDD's #485 Component
/// blocks. Carries the role + title + description plus the property /
/// event / trait declarations the cell needs.
struct ComponentDecl {
    /// Stable slug — matches DDDD's Component '<slug>' key. Same
    /// across every toolkit binding for the role.
    slug: &'static str,
    role: &'static str,
    title: &'static str,
    description: &'static str,
    /// Properties: (name, type, default).
    properties: &'static [(&'static str, &'static str, &'static str)],
    /// Events: (name, payload-type).
    events: &'static [(&'static str, &'static str)],
    /// Component-scoped traits — universal across every toolkit
    /// implementation of this role.
    traits: &'static [&'static str],
    /// The Qt 6 widget class for this Component. Matches a row in
    /// `widgets::QT_WIDGET_TABLE`.
    qt_class: &'static str,
    /// Per-binding traits — Qt-specific overrides that apply to this
    /// `<role>.qt6` ImplementationBinding only. DDDD's #485 puts
    /// `screen_reader_aware` on every Qt binding (QAccessible bridge);
    /// some bindings get `hidpi_native` (QPushButton) or
    /// `compact_native` (QDateEdit) per DDDD's declarations.
    binding_traits: &'static [&'static str],
}

/// The 11 Component declarations matching DDDD's #485 `qt6` bindings
/// in `readings/ui/components.md`. Values are taken verbatim from
/// that reading so the runtime registration is in lockstep with the
/// static declaration. If DDDD edits components.md, this table is
/// what needs to track.
///
/// 11 rows because `image` shares QLabel with whatever future
/// QPixmap-driven Image widget Qt's image-loading path lands;
/// DDDD's #485 declaration `Component 'image' is implemented by
/// Toolkit 'qt6' at Toolkit Symbol 'QLabel'` (components.md L590)
/// uses QLabel directly.
///
/// `card` is intentionally absent from this table — DDDD's #485
/// has no Qt binding for `card` (components.md L527-535), so we
/// don't emit one here either. Same for the seven Slint-only
/// declarations and the GTK / web-only bindings — those land in
/// #488 / #494.
const QT_COMPONENT_DECLS: &[ComponentDecl] = &[
    // Button → QPushButton
    ComponentDecl {
        slug: "button",
        role: "button",
        title: "Button",
        description: "Plain push button — primary control for triggering an action.",
        properties: &[
            ("text", "string", ""),
            ("enabled", "bool", "true"),
            ("primary", "bool", "false"),
        ],
        events: &[("clicked", "none")],
        traits: &["keyboard_navigable", "theming_consumer"],
        qt_class: "QPushButton",
        binding_traits: &["screen_reader_aware", "hidpi_native"],
    },
    // TextInput → QLineEdit
    ComponentDecl {
        slug: "text-input",
        role: "text-input",
        title: "Text Input",
        description: "Single-line text entry field.",
        properties: &[
            ("text", "string", ""),
            ("placeholder", "string", ""),
            ("enabled", "bool", "true"),
            ("maxlength", "int", "0"),
        ],
        events: &[("changed", "string"), ("submitted", "string")],
        traits: &["keyboard_navigable", "theming_consumer"],
        qt_class: "QLineEdit",
        binding_traits: &["screen_reader_aware"],
    },
    // ListView → QListView
    ComponentDecl {
        slug: "list",
        role: "list",
        title: "List View",
        description: "Vertically-scrolling list of homogeneous items.",
        properties: &[("items", "string", ""), ("selected", "int", "-1")],
        events: &[("selection-changed", "int")],
        traits: &["keyboard_navigable", "theming_consumer"],
        qt_class: "QListView",
        binding_traits: &["screen_reader_aware"],
    },
    // DatePicker → QDateEdit
    ComponentDecl {
        slug: "date-picker",
        role: "date-picker",
        title: "Date Picker",
        description: "Calendar-driven date selection.",
        properties: &[("value", "string", ""), ("enabled", "bool", "true")],
        events: &[("changed", "string")],
        traits: &["keyboard_navigable"],
        qt_class: "QDateEdit",
        binding_traits: &["screen_reader_aware", "compact_native"],
    },
    // Dialog → QDialog
    ComponentDecl {
        slug: "dialog",
        role: "dialog",
        title: "Dialog",
        description: "Modal overlay window for transient interaction (confirm, alert, form-on-overlay).",
        properties: &[("title", "string", ""), ("open", "bool", "false")],
        events: &[("closed", "none"), ("confirmed", "none")],
        traits: &["keyboard_navigable", "theming_consumer"],
        qt_class: "QDialog",
        binding_traits: &["screen_reader_aware"],
    },
    // Image → QLabel (Qt reuses QLabel + pixmap because QImage is
    // the data type, not the widget — DDDD's #485 note in
    // components.md L577).
    ComponentDecl {
        slug: "image",
        role: "image",
        title: "Image",
        description: "Static raster or vector image. Qt 6 reuses QLabel + pixmap because QImage is the data type, not the widget.",
        properties: &[("source", "image", ""), ("fit", "enum", "contain")],
        events: &[],
        traits: &["theming_consumer"],
        qt_class: "QLabel",
        binding_traits: &[],
    },
    // Slider → QSlider
    ComponentDecl {
        slug: "slider",
        role: "slider",
        title: "Slider",
        description: "Continuous numeric value selection along a track.",
        properties: &[
            ("value", "int", "0"),
            ("minimum", "int", "0"),
            ("maximum", "int", "100"),
        ],
        events: &[("changed", "int")],
        traits: &["keyboard_navigable", "theming_consumer"],
        qt_class: "QSlider",
        binding_traits: &["screen_reader_aware"],
    },
    // ComboBox → QComboBox
    ComponentDecl {
        slug: "combo-box",
        role: "combo-box",
        title: "Combo Box",
        description: "Dropdown selection from a closed list.",
        properties: &[("items", "string", ""), ("selected", "int", "-1")],
        events: &[("selection-changed", "int")],
        traits: &["keyboard_navigable", "theming_consumer"],
        qt_class: "QComboBox",
        binding_traits: &["screen_reader_aware"],
    },
    // ProgressBar → QProgressBar
    ComponentDecl {
        slug: "progress-bar",
        role: "progress-bar",
        title: "Progress Bar",
        description: "Linear progress indicator with optional indeterminate mode.",
        properties: &[
            ("value", "int", "0"),
            ("maximum", "int", "100"),
            ("indeterminate", "bool", "false"),
        ],
        events: &[],
        traits: &["theming_consumer"],
        qt_class: "QProgressBar",
        binding_traits: &[],
    },
    // CheckBox → QCheckBox
    ComponentDecl {
        slug: "checkbox",
        role: "checkbox",
        title: "Check Box",
        description: "Bistate (or tristate) toggle bound to a label.",
        properties: &[
            ("checked", "bool", "false"),
            ("label", "string", ""),
            ("enabled", "bool", "true"),
        ],
        events: &[("toggled", "bool")],
        traits: &["keyboard_navigable", "theming_consumer"],
        qt_class: "QCheckBox",
        binding_traits: &["screen_reader_aware"],
    },
    // Tab → QTabBar
    ComponentDecl {
        slug: "tab",
        role: "tab",
        title: "Tab Bar",
        description: "Horizontal tab strip selecting one of N child surfaces.",
        properties: &[("selected", "int", "0"), ("tabs", "string", "")],
        events: &[("selection-changed", "int")],
        traits: &["keyboard_navigable", "theming_consumer"],
        qt_class: "QTabBar",
        binding_traits: &["screen_reader_aware"],
    },
];

/// Build the runtime Component / ImplementationBinding fact set for
/// the Qt 6 toolkit and apply it to SYSTEM. Returns `Ok(n)` with the
/// count of registered Component cells on success;
/// `Err(message)` if `system::init()` hasn't run.
///
/// Idempotent at the cell-content level: every push uses
/// `cell_push_unique` so a second call against an already-populated
/// SYSTEM is a no-op rather than a duplicate-insert. This matches
/// FFFF's #486 Slint registration pattern — both adapters can init
/// in any order without stepping on each other's Toolkit row or on
/// the shared Component cells.
///
/// Note: the resolved QMetaObject pointer for each widget class is
/// fetched from `widgets::resolved` for a debug log line; the cell
/// content itself stores the unmangled class name as the Symbol
/// value (which matches DDDD's `Toolkit Symbol` value type — string,
/// not pointer). The pointer table lives in `widgets::RESOLVED_SYMBOLS`
/// so marshalling.rs can reach it by class name when invoking
/// `setProperty` / `connect_signal`.
pub fn register_qt_components() -> Result<usize, &'static str> {
    let initial = system::with_state(|s| s.clone()).ok_or("system::init() not called")?;
    let mut state = initial;

    // 1. Toolkit row — emit the qt6 toolkit facts. `cell_push_unique`
    //    keeps the Toolkit cell deduped against FFFF's #486 Slint
    //    registration which emits its own 'slint' row.
    state = cell_push_unique(
        "Toolkit_has_Slug",
        fact_from_pairs(&[("Toolkit", "qt6"), ("Slug", "qt6")]),
        &state,
    );
    state = cell_push_unique(
        "Toolkit_has_Version",
        fact_from_pairs(&[("Toolkit", "qt6"), ("Version", "6.6")]),
        &state,
    );
    state = cell_push_unique(
        "Toolkit_has_Title",
        fact_from_pairs(&[("Toolkit", "qt6"), ("Title", "Qt 6")]),
        &state,
    );

    // 2. Per-Component facts. For each declaration:
    //    a. Component cell (role + title + description) — shared
    //       across every toolkit binding for the role, deduped via
    //       `cell_push_unique`.
    //    b. Per-property fact (Component_has_Property, ternary).
    //    c. Per-event fact (Component_emits_Event, ternary).
    //    d. Per-Component-trait fact.
    //    e. ImplementationBinding cell — the (Component, Toolkit,
    //       Symbol) triple. The Symbol value is the unmangled Qt
    //       class name string; the resolved QMetaObject pointer
    //       (or null on the foundation slice) is logged to debug
    //       but not stored in the cell itself.
    //    f. Per-binding-trait fact.
    let mut count = 0usize;
    for decl in QT_COMPONENT_DECLS {
        // (a) Component cell — these match FFFF's #486 Slint
        // declarations word-for-word so cell_push_unique dedupes.
        state = cell_push_unique(
            "Component_has_Role",
            fact_from_pairs(&[("Component", decl.slug), ("Role", decl.role)]),
            &state,
        );
        state = cell_push_unique(
            "Component_has_Title",
            fact_from_pairs(&[("Component", decl.slug), ("Title", decl.title)]),
            &state,
        );
        state = cell_push_unique(
            "Component_has_Description",
            fact_from_pairs(&[("Component", decl.slug), ("Description", decl.description)]),
            &state,
        );

        // (b) Properties. Ternary (Component, Property, Type, Default).
        for (name, ty, default) in decl.properties {
            state = cell_push_unique(
                "Component_has_Property",
                fact_from_pairs(&[
                    ("Component", decl.slug),
                    ("Property", *name),
                    ("Type", *ty),
                    ("Default", *default),
                ]),
                &state,
            );
        }

        // (c) Events. Ternary (Component, Event, Payload).
        for (name, payload) in decl.events {
            state = cell_push_unique(
                "Component_emits_Event",
                fact_from_pairs(&[
                    ("Component", decl.slug),
                    ("Event", *name),
                    ("Payload", *payload),
                ]),
                &state,
            );
        }

        // (d) Component-scoped traits.
        for trait_name in decl.traits {
            state = cell_push_unique(
                "Component_has_Trait",
                fact_from_pairs(&[("Component", decl.slug), ("Trait", *trait_name)]),
                &state,
            );
        }

        // (e) ImplementationBinding cell. The binding name is
        // `<slug>.qt6` per DDDD's #485 derived-slug convention
        // (components.md L205). The Symbol value is the unmangled
        // Qt class name.
        let binding_name = binding_slug(decl.slug);
        state = cell_push_unique(
            "Component_is_implemented_by_Toolkit_at_Symbol",
            fact_from_pairs(&[
                ("Component", decl.slug),
                ("Toolkit", "qt6"),
                ("Symbol", decl.qt_class),
            ]),
            &state,
        );
        state = cell_push_unique(
            "ImplementationBinding_pivots_Component_Toolkit",
            fact_from_pairs(&[
                ("ImplementationBinding", binding_name.as_str()),
                ("Component", decl.slug),
                ("Toolkit", "qt6"),
            ]),
            &state,
        );

        // (f) Per-binding traits.
        for trait_name in decl.binding_traits {
            state = cell_push_unique(
                "ImplementationBinding_has_Trait",
                fact_from_pairs(&[
                    ("ImplementationBinding", binding_name.as_str()),
                    ("Trait", *trait_name),
                ]),
                &state,
            );
        }

        count += 1;
    }

    // 3. Commit the new state. Subscribers (HateoasBrowser, future
    //    selection-rule cache) get notified of every changed cell.
    system::apply(state).map_err(|e| e)?;
    let _ = log_resolved_pointers();
    Ok(count)
}

/// Build the `<component-slug>.qt6` ImplementationBinding name DDDD's
/// #485 derived-slug convention specifies (components.md L205). The
/// derivation is `'<component>.<toolkit>'` — straightforward concat.
fn binding_slug(component_slug: &str) -> String {
    let mut s = String::with_capacity(component_slug.len() + 4);
    s.push_str(component_slug);
    s.push_str(".qt6");
    s
}

/// Walk `widgets::iter_resolved()` and produce a debug-friendly
/// summary of the resolved-vs-null counts for the QMetaObject
/// pointers. On the foundation slice every count is 0/N (everything
/// null because the libraries never loaded). When the future loader
/// extension lands real dlopen, the resolved count climbs and the
/// caller can quickly see which classes failed to resolve.
///
/// Returns `(resolved_count, total)` rather than printing — we don't
/// want to depend on the print plumbing during boot. The single
/// caller in `register_qt_components` discards the return value (the
/// boot path doesn't need to react to resolution counts; future
/// observability work could surface them via a debug fact cell).
fn log_resolved_pointers() -> (usize, usize) {
    let entries = widgets::iter_resolved();
    let total = entries.len();
    let resolved = entries.iter().filter(|(_, p)| !p.is_null()).count();
    (resolved, total)
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qt_adapter::{loader, widgets};

    /// The decl table covers exactly the 11 Qt widget classes
    /// `widgets::QT_WIDGET_TABLE` carries. Same-count assertion
    /// across the two tables catches drift in either direction.
    #[test]
    fn decl_count_matches_widget_table() {
        assert_eq!(QT_COMPONENT_DECLS.len(), widgets::QT_WIDGET_TABLE.len());
    }

    /// Every decl points at a `qt_class` that exists in the widget
    /// table — no broken references. Catches a typo in either
    /// table that would silently de-correlate the cell registration
    /// from the symbol resolution.
    #[test]
    fn every_decl_qt_class_exists_in_widget_table() {
        let known: alloc::collections::BTreeSet<&str> = widgets::QT_WIDGET_TABLE
            .iter()
            .map(|w| w.class_name)
            .collect();
        for decl in QT_COMPONENT_DECLS {
            assert!(
                known.contains(decl.qt_class),
                "{} not in widget table",
                decl.qt_class
            );
        }
    }

    /// Binding slugs follow DDDD's `<component>.<toolkit>` shape
    /// (components.md L205). Spot-check one to lock the format.
    #[test]
    fn binding_slug_matches_dddd_convention() {
        assert_eq!(binding_slug("button"), "button.qt6");
        assert_eq!(binding_slug("text-input"), "text-input.qt6");
        assert_eq!(binding_slug("date-picker"), "date-picker.qt6");
    }

    /// End-to-end: register against a freshly-init'd SYSTEM and
    /// confirm the Component cells land. Picks button as the canary
    /// because it's the most-attested binding in DDDD's #485 (every
    /// toolkit declares it).
    #[test]
    fn register_qt_components_lands_button_cell() {
        crate::system::init();
        loader::init();
        widgets::init();
        let count = register_qt_components().expect("register succeeds");
        assert_eq!(count, 11, "all 11 Qt component decls should register");
        // Read back: the button Component_has_Role cell should
        // contain a fact pairing 'button' with the 'button' role.
        let role_cell = system::with_state(|s| {
            ast::fetch_or_phi("Component_has_Role", s).clone()
        }).expect("init ran");
        let seq = role_cell.as_seq().unwrap_or(&[]);
        let has_button = seq.iter().any(|fact| {
            // Each fact is Object::Seq of pairs; convert to a flat
            // string and look for both 'button' tokens.
            let s = alloc::format!("{:?}", fact);
            s.contains("Component") && s.contains("button")
        });
        assert!(has_button, "Component_has_Role should contain a button fact");
    }

    /// Binding cell carries the Qt class name as the Symbol value
    /// (matches DDDD's #485 declaration `Component 'button' is
    /// implemented by Toolkit 'qt6' at Toolkit Symbol 'QPushButton'`,
    /// components.md L399).
    #[test]
    fn binding_cell_carries_qt_class_as_symbol() {
        crate::system::init();
        loader::init();
        widgets::init();
        register_qt_components().expect("register succeeds");
        let cell = system::with_state(|s| {
            ast::fetch_or_phi("Component_is_implemented_by_Toolkit_at_Symbol", s).clone()
        }).expect("init ran");
        let seq = cell.as_seq().unwrap_or(&[]);
        let has_qpushbutton = seq.iter().any(|fact| {
            let s = alloc::format!("{:?}", fact);
            s.contains("QPushButton") && s.contains("button") && s.contains("qt6")
        });
        assert!(has_qpushbutton, "binding cell should contain QPushButton triple");
    }

    /// The `qt6` Toolkit row is registered. DDDD's #485
    /// declarations carry Slug 'qt6', Version '6.6', Title 'Qt 6';
    /// we register the same triple.
    #[test]
    fn toolkit_row_registered() {
        crate::system::init();
        loader::init();
        widgets::init();
        register_qt_components().expect("register succeeds");
        let slug_cell = system::with_state(|s| {
            ast::fetch_or_phi("Toolkit_has_Slug", s).clone()
        }).expect("init ran");
        let s = alloc::format!("{:?}", slug_cell);
        assert!(s.contains("qt6"), "Toolkit_has_Slug should contain qt6 fact");
    }

    /// Re-registering against an already-populated SYSTEM is a no-op
    /// at the cell-content level (cell_push_unique deduplicates).
    /// Important because both adapter init paths can run in any
    /// order without duplicating shared Component cells.
    #[test]
    fn double_registration_is_idempotent() {
        crate::system::init();
        loader::init();
        widgets::init();
        register_qt_components().expect("first register succeeds");
        let pre = system::with_state(|s| {
            ast::fetch_or_phi("Component_has_Role", s).clone()
        }).expect("init ran");
        let pre_len = pre.as_seq().map(|s| s.len()).unwrap_or(0);

        register_qt_components().expect("second register succeeds");
        let post = system::with_state(|s| {
            ast::fetch_or_phi("Component_has_Role", s).clone()
        }).expect("init ran");
        let post_len = post.as_seq().map(|s| s.len()).unwrap_or(0);

        assert_eq!(pre_len, post_len, "double-register should not duplicate facts");
    }

    /// Foundation slice: log_resolved_pointers reports 0 resolved out
    /// of 11 because every dlsym returned null. Locks the foundation
    /// behaviour so the future loader extension can flip the
    /// expectation.
    #[test]
    fn log_resolved_pointers_zero_on_foundation_slice() {
        loader::init();
        widgets::init();
        let (resolved, total) = log_resolved_pointers();
        assert_eq!(resolved, 0);
        assert_eq!(total, 11);
    }
}
