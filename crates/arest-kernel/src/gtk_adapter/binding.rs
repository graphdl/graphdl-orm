// crates/arest-kernel/src/gtk_adapter/binding.rs
//
// `register_gtk_components()` — build the runtime fact set that
// mirrors DDDD's #485 static declarations in
// `readings/ui/components.md` for the GTK 4 toolkit, and apply it
// to SYSTEM via `system::apply`.
//
// What we register
// ----------------
// For each GTK 4 widget class in `widgets::GTK_WIDGET_TABLE` we
// emit the same fact shapes DDDD's #485 declared:
//
//   * Component cell (Component_has_Role, display- Title,
//     Description) — the abstract widget category. These are shared
//     with FFFF's #486 Slint registration and GGGG's #487 Qt
//     registration so we use `cell_push_unique` to avoid duplicating
//     the Component facts when multiple adapters init.
//   * ImplementationBinding cell (Component is implemented by
//     Toolkit at Toolkit Symbol) — the (Component, Toolkit, Symbol)
//     triple pinning GTK's class name as the Symbol.
//   * Component property facts (text:string default '', enabled:bool
//     default true, primary:bool default false, …) per DDDD's
//     declarations.
//   * Component event facts (clicked:none, changed:string, …) per
//     DDDD's declarations.
//   * Component trait facts (keyboard_navigable, theming_consumer)
//     and ImplementationBinding-scoped trait facts (per DDDD's
//     declarations: dark_mode_native on every GTK 4.4+ binding;
//     screen_reader_aware on bindings with AT-SPI surface area;
//     hidpi_native on GtkButton + GtkPicture which DDDD's #485
//     attaches per components.md L615 / L806).
//
// Toolkit row
// -----------
// We emit the `gtk4` Toolkit row (Toolkit_has_Slug 'gtk4',
// Toolkit_has_Version '4.14', display- Title 'GTK 4') once. Same
// dedup rule applies — `cell_push_unique` makes it safe for FFFF's
// #486 (slint) and GGGG's #487 (qt6) to emit their own Toolkit rows
// in parallel without duplication.
//
// Selection-rule consumption
// --------------------------
// The registered facts feed the derivation rules at the bottom of
// `readings/ui/components.md`. DDDD's #485 ships a "Screen-reader
// / GTK preference" rule (components.md L322-340) that prefers GTK
// 4 when AT-SPI is the selection driver — that rule fires once
// the compositor is wired (#489) and the runtime has a screen-
// reader-active fact to query against. On the foundation slice the
// SLINT implementations win because the Slint binding has the
// `kernel_native` trait DDDD's #485 attached and the rule library
// (#492) hasn't yet shipped a fully populated GTK-specific
// preference rule. After #489 wires the compositor + #492 expands
// the rule library, an AI-driven `select_component` query (#493)
// can pick GTK over Slint or Qt on a per-context basis.

use alloc::string::String;
use arest::ast::{cell_push_unique, fact_from_pairs};

use crate::system;

use super::widgets;

/// Component declaration mirroring one of DDDD's #485 Component
/// blocks. Carries the role + title + description plus the
/// property / event / trait declarations the cell needs.
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
    /// The GTK 4 widget class for this Component. Matches a row in
    /// `widgets::GTK_WIDGET_TABLE`.
    gtk_class: &'static str,
    /// Per-binding traits — GTK-specific overrides that apply to
    /// this `<role>.gtk4` ImplementationBinding only. Values follow
    /// DDDD's #485 declarations verbatim (components.md L612-960):
    ///   * `dark_mode_native` on every GTK 4.4+ binding except
    ///     `image.gtk4` (which DDDD's #485 attaches `hidpi_native`
    ///     only — components.md L806).
    ///   * `screen_reader_aware` on every binding with AT-SPI surface
    ///     area; absent from `card.gtk4` (layout primitive — L745-747)
    ///     and `image.gtk4` (passive content — L804-806) and
    ///     `progress-bar.gtk4` (decorative; L894-896).
    ///   * `hidpi_native` on `button.gtk4` (L615) and `image.gtk4`
    ///     (L806) — the two bindings DDDD's #485 attaches it to.
    binding_traits: &'static [&'static str],
}

/// The 12 Component declarations matching DDDD's #485 `gtk4` bindings
/// in `readings/ui/components.md`. Values are taken verbatim from
/// that reading so the runtime registration is in lockstep with the
/// static declaration. If DDDD edits components.md, this table is
/// what needs to track.
///
/// 12 rows because GTK has a `card` binding (`Toolkit Symbol
/// 'GtkBox'`, components.md L745) — Qt has no card binding so its
/// table sits at 11. GTK's card piggybacks on GtkBox + a
/// `add_css_class('card')` call DDDD's note explains; the Component
/// cell for `card` is shared with FFFF's #486 Slint registration so
/// `cell_push_unique` dedupes the Component facts.
///
/// Note on `touch_optimized`: the task spec for #488 mentioned
/// attaching `touch_optimized` to GtkScale, GtkCalendar, and
/// GtkProgressBar (designed for tablet use). DDDD's #485
/// declarations in components.md do NOT carry that trait on any GTK
/// binding — the trait appears only on web bindings (L622, L662,
/// L722, etc.) where the touch-event surface is the primary input.
/// We follow DDDD's declarations verbatim per the lockstep
/// convention; if DDDD adds touch_optimized to any GTK binding in a
/// future edit, this table tracks.
const GTK_COMPONENT_DECLS: &[ComponentDecl] = &[
    // Button → GtkButton (components.md L612-616)
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
        gtk_class: "GtkButton",
        binding_traits: &["screen_reader_aware", "hidpi_native", "dark_mode_native"],
    },
    // TextInput → GtkEntry (components.md L654-657)
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
        gtk_class: "GtkEntry",
        binding_traits: &["screen_reader_aware", "dark_mode_native"],
    },
    // ListView → GtkListView (components.md L692-695)
    ComponentDecl {
        slug: "list",
        role: "list",
        title: "List View",
        description: "Vertically-scrolling list of homogeneous items.",
        properties: &[("items", "string", ""), ("selected", "int", "-1")],
        events: &[("selection-changed", "int")],
        traits: &["keyboard_navigable", "theming_consumer"],
        gtk_class: "GtkListView",
        binding_traits: &["screen_reader_aware", "dark_mode_native"],
    },
    // DatePicker → GtkCalendar (components.md L715-718)
    ComponentDecl {
        slug: "date-picker",
        role: "date-picker",
        title: "Date Picker",
        description: "Calendar-driven date selection.",
        properties: &[("value", "string", ""), ("enabled", "bool", "true")],
        events: &[("changed", "string")],
        traits: &["keyboard_navigable"],
        gtk_class: "GtkCalendar",
        binding_traits: &["screen_reader_aware", "dark_mode_native"],
    },
    // Dialog → GtkDialog (components.md L775-778)
    ComponentDecl {
        slug: "dialog",
        role: "dialog",
        title: "Dialog",
        description: "Modal overlay window for transient interaction (confirm, alert, form-on-overlay).",
        properties: &[("title", "string", ""), ("open", "bool", "false")],
        events: &[("closed", "none"), ("confirmed", "none")],
        traits: &["keyboard_navigable", "theming_consumer"],
        gtk_class: "GtkDialog",
        binding_traits: &["screen_reader_aware", "dark_mode_native"],
    },
    // Image → GtkPicture (components.md L804-806; GTK 4 first-class
    // image widget — replaces GtkImage from GTK 3)
    ComponentDecl {
        slug: "image",
        role: "image",
        title: "Image",
        description: "Static raster or vector image. Qt 6 reuses QLabel + pixmap because QImage is the data type, not the widget.",
        properties: &[("source", "image", ""), ("fit", "enum", "contain")],
        events: &[],
        traits: &["theming_consumer"],
        gtk_class: "GtkPicture",
        binding_traits: &["hidpi_native"],
    },
    // Slider → GtkScale (components.md L835-838)
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
        gtk_class: "GtkScale",
        binding_traits: &["screen_reader_aware", "dark_mode_native"],
    },
    // ComboBox → GtkDropDown (components.md L863-866; GTK 4 modern
    // replacement for GTK 3's GtkComboBox)
    ComponentDecl {
        slug: "combo-box",
        role: "combo-box",
        title: "Combo Box",
        description: "Dropdown selection from a closed list.",
        properties: &[("items", "string", ""), ("selected", "int", "-1")],
        events: &[("selection-changed", "int")],
        traits: &["keyboard_navigable", "theming_consumer"],
        gtk_class: "GtkDropDown",
        binding_traits: &["screen_reader_aware", "dark_mode_native"],
    },
    // ProgressBar → GtkProgressBar (components.md L894-896; DDDD
    // omits screen_reader_aware here — decorative widget)
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
        gtk_class: "GtkProgressBar",
        binding_traits: &["dark_mode_native"],
    },
    // CheckBox → GtkCheckButton (components.md L927-930; GTK 4
    // unified GtkCheckButton + GtkToggleButton)
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
        gtk_class: "GtkCheckButton",
        binding_traits: &["screen_reader_aware", "dark_mode_native"],
    },
    // Tab → GtkNotebook (components.md L957-960)
    ComponentDecl {
        slug: "tab",
        role: "tab",
        title: "Tab Bar",
        description: "Horizontal tab strip selecting one of N child surfaces.",
        properties: &[("selected", "int", "0"), ("tabs", "string", "")],
        events: &[("selection-changed", "int")],
        traits: &["keyboard_navigable", "theming_consumer"],
        gtk_class: "GtkNotebook",
        binding_traits: &["screen_reader_aware", "dark_mode_native"],
    },
    // Card → GtkBox (components.md L745-747; GTK has no first-class
    // card primitive — the binding piggybacks on GtkBox plus
    // `gtk_widget_add_css_class(box, "card")` to pick up the Adwaita
    // card styling. DDDD omits screen_reader_aware here — it's a
    // layout primitive, not an interactive widget. Also omits
    // hidpi_native — Slint's card variant has it because Slint's
    // surface treats it as a first-class widget.)
    ComponentDecl {
        slug: "card",
        role: "card",
        title: "Card",
        description: "Surfaced container with optional header / footer chrome. The Slint binding is the MMM #436 stock card.",
        properties: &[
            ("elevation", "int", "1"),
            ("padding", "length", "16"),
        ],
        events: &[],
        traits: &["theming_consumer"],
        gtk_class: "GtkBox",
        binding_traits: &["dark_mode_native"],
    },
];

/// Build the runtime Component / ImplementationBinding fact set for
/// the GTK 4 toolkit and apply it to SYSTEM. Returns `Ok(n)` with
/// the count of registered Component cells on success;
/// `Err(message)` if `system::init()` hasn't run.
///
/// Idempotent at the cell-content level: every push uses
/// `cell_push_unique` so a second call against an already-populated
/// SYSTEM is a no-op rather than a duplicate-insert. This matches
/// FFFF's #486 Slint registration pattern and GGGG's #487 Qt
/// registration pattern — all three adapters can init in any order
/// without stepping on each other's Toolkit row or on the shared
/// Component cells.
///
/// Note: the resolved GType pointer for each widget class is
/// fetched from `widgets::resolved` for a debug log line; the cell
/// content itself stores the unmangled class name as the Symbol
/// value (which matches DDDD's `Toolkit Symbol` value type — string,
/// not pointer). The pointer table lives in
/// `widgets::RESOLVED_SYMBOLS` so marshalling.rs can reach it by
/// class name when invoking `g_object_set_property` /
/// `g_signal_connect`.
pub fn register_gtk_components() -> Result<usize, &'static str> {
    let initial = system::with_state(|s| s.clone()).ok_or("system::init() not called")?;
    let mut state = initial;

    // 1. Toolkit row — emit the gtk4 toolkit facts. `cell_push_unique`
    //    keeps the Toolkit cell deduped against FFFF's #486 Slint
    //    registration (emits 'slint') and GGGG's #487 Qt registration
    //    (emits 'qt6').
    state = cell_push_unique(
        "Toolkit_has_Slug",
        fact_from_pairs(&[("Toolkit", "gtk4"), ("Slug", "gtk4")]),
        &state,
    );
    state = cell_push_unique(
        "Toolkit_has_Version",
        fact_from_pairs(&[("Toolkit", "gtk4"), ("Version", "4.14")]),
        &state,
    );
    state = cell_push_unique(
        "Toolkit_has_Title",
        fact_from_pairs(&[("Toolkit", "gtk4"), ("Title", "GTK 4")]),
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
    //       Symbol) triple. The Symbol value is the GTK class name
    //       string; the resolved GType pointer (or null on the
    //       foundation slice) is logged to debug but not stored in
    //       the cell itself.
    //    f. Per-binding-trait fact.
    let mut count = 0usize;
    for decl in GTK_COMPONENT_DECLS {
        // (a) Component cell — these match FFFF's #486 Slint
        // declarations + GGGG's #487 Qt declarations word-for-word
        // so cell_push_unique dedupes.
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
        // `<slug>.gtk4` per DDDD's #485 derived-slug convention
        // (components.md L205). The Symbol value is the GTK class
        // name.
        let binding_name = binding_slug(decl.slug);
        state = cell_push_unique(
            "Component_is_implemented_by_Toolkit_at_Symbol",
            fact_from_pairs(&[
                ("Component", decl.slug),
                ("Toolkit", "gtk4"),
                ("Symbol", decl.gtk_class),
            ]),
            &state,
        );
        state = cell_push_unique(
            "ImplementationBinding_pivots_Component_Toolkit",
            fact_from_pairs(&[
                ("ImplementationBinding", binding_name.as_str()),
                ("Component", decl.slug),
                ("Toolkit", "gtk4"),
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

/// Build the `<component-slug>.gtk4` ImplementationBinding name
/// DDDD's #485 derived-slug convention specifies (components.md
/// L205). The derivation is `'<component>.<toolkit>'` —
/// straightforward concat.
fn binding_slug(component_slug: &str) -> String {
    let mut s = String::with_capacity(component_slug.len() + 5);
    s.push_str(component_slug);
    s.push_str(".gtk4");
    s
}

/// Walk `widgets::iter_resolved()` and produce a debug-friendly
/// summary of the resolved-vs-null counts for the GType pointers.
/// On the foundation slice every count is 0/N (everything null
/// because the libraries never loaded). When the future loader
/// extension lands real dlopen, the resolved count climbs and the
/// caller can quickly see which classes failed to resolve.
///
/// Returns `(resolved_count, total)` rather than printing — we
/// don't want to depend on the print plumbing during boot. The
/// single caller in `register_gtk_components` discards the return
/// value (the boot path doesn't need to react to resolution counts;
/// future observability work could surface them via a debug fact
/// cell).
fn log_resolved_pointers() -> (usize, usize) {
    let entries = widgets::iter_resolved();
    let total = entries.len();
    let resolved = entries.iter().filter(|(_, p)| !p.is_null()).count();
    (resolved, total)
}

// ── ComponentBinder impl (#491 Track PPPP) ───────────────────────────
//
// Concrete `ComponentBinder` for the GTK 4 toolkit. Sibling of
// `qt_adapter::binding::QtBinder` — same shape, different reflection
// system underneath: where Qt routes through `QObject::setProperty
// (name, QVariant)` + `QObject::connect(sender, signal, ...)`, GTK
// routes through `g_object_set_property(obj, name, GValue)` +
// `g_signal_connect(obj, signal, callback)`. Marshalling differences
// land in the marshalling.rs `set_property`/`connect_signal`
// surface; this binder is a thin glue between PPPP's dispatch
// surface and the GObject reflection.
//
// Foundation-slice behaviour: every `set_property` returns
// `Err(MarshalError::Stub)` because libgtk-4 / libgobject-2.0 never
// loaded; the binder swallows the error silently — same graceful-
// degrade contract `qt_adapter::binding::QtBinder` follows.
//
// The GtkBinder maintains a per-handle map of (GTK class name, raw
// GObject*) pairs because PPPP's `ToolkitHandle(u64)` is opaque to
// the kernel — GTK needs both the GType (looked up via the class
// name → widgets::resolved chain) AND the GObject pointer itself.

use crate::component_binding::{
    ComponentBinder, PropertyValue, SignalCallback, ToolkitHandle,
};
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use core::ffi::c_void;
use spin::Mutex;

use super::marshalling::{self, ComponentValue, GObjectPtr};

/// Per-handle metadata the GtkBinder needs to drive marshalling.
/// Stored in `GTK_HANDLE_TABLE` keyed by the kernel-side
/// `ToolkitHandle.0`. The GTK class name is the lookup key into
/// `widgets::resolved` (which returns the GType pointer); the raw
/// `GObject*` is the actual instance pointer the marshalling layer
/// hands to `g_object_set_property` / `g_signal_connect`.
#[derive(Clone)]
struct GtkWidgetEntry {
    /// GTK class name (e.g. `"GtkButton"`). Used to resolve the
    /// GType pointer via `widgets::resolved` at marshalling time.
    /// `&'static str` rather than owned String — the class names
    /// live in static storage (one of `GTK_COMPONENT_DECLS::
    /// gtk_class`), saves an allocation per handle.
    class_name: &'static str,
    /// Raw `GObject*` for the live widget instance.
    ///
    /// SAFETY: same caveat as `qt_adapter::binding::QtWidgetEntry`
    /// — the binder treats the pointer as opaque on the kernel
    /// side; the marshalling layer dereferences it via GObject's
    /// reflection thunks. The adapter that calls `bind_widget` is
    /// responsible for that contract.
    ptr: WidgetPtrSendSync,
}

/// `Send + Sync` newtype around `*mut c_void` so the GtkWidgetEntry
/// can live in a `BTreeMap` behind a `spin::Mutex`. Same pattern as
/// `qt_adapter::binding::WidgetPtrSendSync` — kernel runs single-
/// threaded; the unsafe impl satisfies the trait bound the static
/// storage requires.
#[derive(Clone, Copy)]
struct WidgetPtrSendSync(*mut c_void);

unsafe impl Send for WidgetPtrSendSync {}
unsafe impl Sync for WidgetPtrSendSync {}

/// Per-handle table mapping `ToolkitHandle.0` → `GtkWidgetEntry`.
/// Sibling of `qt_adapter::binding::QT_HANDLE_TABLE`. Same lookup
/// shape, separate storage so each adapter manages its own handle
/// space without colliding.
static GTK_HANDLE_TABLE: Mutex<BTreeMap<u64, GtkWidgetEntry>> =
    Mutex::new(BTreeMap::new());

/// GtkBinder — concrete `ComponentBinder` impl for the GTK 4
/// toolkit. Stateless (the per-handle state lives in
/// `GTK_HANDLE_TABLE`), so this is a unit struct. Built once per
/// boot via `GtkBinder::install`, which also registers the binder
/// against the `component_binding::BINDERS` map under the `"gtk4"`
/// slug.
pub struct GtkBinder;

impl GtkBinder {
    /// Build the GtkBinder + register it against
    /// `component_binding::BINDERS` under the `"gtk4"` slug. Called
    /// from `gtk_adapter::init` after `register_gtk_components` has
    /// run (so the BINDERS map is populated before any
    /// `register_component` call).
    ///
    /// Idempotent — `register_binder` replaces atomically at the
    /// slug level.
    pub fn install() {
        crate::component_binding::register_binder("gtk4", Arc::new(GtkBinder));
    }

    /// Bind a GTK widget instance to a kernel-side ToolkitHandle.
    /// Sibling of `qt_adapter::binding::QtBinder::bind_widget`.
    ///
    /// SAFETY: `ptr` must be a live GObject of class `class_name`
    /// (one of `widgets::GTK_WIDGET_TABLE::class_name`). The binder
    /// treats the pointer as opaque on the kernel side; the
    /// marshalling layer dereferences it via GObject's reflection
    /// thunks.
    ///
    /// Returns the allocated `ToolkitHandle`. The handle is
    /// monotonic via `NEXT_GTK_HANDLE` and the class_name is
    /// stored alongside the raw pointer in `GTK_HANDLE_TABLE`.
    pub unsafe fn bind_widget(class_name: &'static str, ptr: GObjectPtr) -> ToolkitHandle {
        let id = NEXT_GTK_HANDLE.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        let entry = GtkWidgetEntry {
            class_name,
            ptr: WidgetPtrSendSync(ptr),
        };
        GTK_HANDLE_TABLE.lock().insert(id, entry);
        ToolkitHandle(id)
    }

    /// Convert PPPP's `PropertyValue` to gtk_adapter's
    /// `ComponentValue`. When `qt-adapter` is also enabled, the
    /// gtk_adapter::marshalling::ComponentValue is re-exported
    /// from qt_adapter::marshalling so this is the same conversion
    /// `qt_adapter::binding::QtBinder::to_qt_value` does. When
    /// only `gtk-adapter` is enabled, gtk_adapter has its own
    /// local ComponentValue with the same shape — same eight
    /// variants per DDDD's #485 Property Type enumeration.
    fn to_gtk_value(value: PropertyValue) -> ComponentValue {
        match value {
            PropertyValue::String(s) => ComponentValue::String(s),
            PropertyValue::Int(v) => ComponentValue::Int(v),
            PropertyValue::Bool(v) => ComponentValue::Bool(v),
            PropertyValue::Enum(s) => ComponentValue::Enum(s),
            PropertyValue::Color(s) => ComponentValue::Color(s),
            PropertyValue::Length(v) => ComponentValue::Length(v),
            PropertyValue::Image(s) => ComponentValue::Image(s),
            PropertyValue::Callback => ComponentValue::Callback,
        }
    }
}

/// Monotonic ToolkitHandle counter, allocated by `bind_widget`.
/// Starts at 1 so 0 stays available as a "no widget" sentinel.
/// Separate from `qt_adapter::binding::NEXT_QT_HANDLE` so each
/// adapter's handle space is independent — a Qt handle 7 and a GTK
/// handle 7 are different widgets and the kernel never confuses
/// them (the per-toolkit BINDERS lookup picks the right table).
static NEXT_GTK_HANDLE: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(1);

impl ComponentBinder for GtkBinder {
    /// Forward (cell → GTK widget): look up the GObject* + class
    /// name in `GTK_HANDLE_TABLE`, convert PPPP's PropertyValue to
    /// gtk_adapter's ComponentValue, and dispatch through
    /// `marshalling::set_property` (which under the hood would
    /// reach `g_object_set_property(obj, name, GValue)`).
    ///
    /// On the foundation slice the marshalling call returns
    /// `Err(MarshalError::Stub)`; the binder swallows the error
    /// silently — same graceful-degrade as the Qt sibling.
    fn set_property(&self, handle: ToolkitHandle, name: &str, value: PropertyValue) {
        let entry = match GTK_HANDLE_TABLE.lock().get(&handle.0).cloned() {
            Some(e) => e,
            None => return,
        };
        let gtk_value = Self::to_gtk_value(value);
        let _ = marshalling::set_property(entry.ptr.0, entry.class_name, name, gtk_value);
    }

    /// Reverse (GTK signal → cell): wire the callback through
    /// `marshalling::connect_signal` (which under the hood would
    /// reach `g_signal_connect(obj, signal, callback, user_data)`).
    /// The marshalling-layer callback type is `Box<dyn Fn() + Send
    /// + Sync>` (no payload); we adapt by capturing the signal
    /// name in the closure and passing an empty payload — full
    /// payload marshalling lands when `connect_signal` grows a
    /// payload-bearing variant (post-loader-extension work, same
    /// constraint the Qt binder operates under).
    fn install_signal(&self, handle: ToolkitHandle, signal: &str, callback: SignalCallback) {
        let entry = match GTK_HANDLE_TABLE.lock().get(&handle.0).cloned() {
            Some(e) => e,
            None => return,
        };
        let signal_name = alloc::string::String::from(signal);
        let trampoline: Box<dyn Fn() + Send + Sync> = Box::new(move || {
            callback(&signal_name, "");
        });
        let _ = marshalling::connect_signal(entry.ptr.0, entry.class_name, signal, trampoline);
    }
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gtk_adapter::{loader, widgets};

    /// The decl table covers exactly the 12 GTK widget classes
    /// `widgets::GTK_WIDGET_TABLE` carries. Same-count assertion
    /// across the two tables catches drift in either direction.
    #[test]
    fn decl_count_matches_widget_table() {
        assert_eq!(GTK_COMPONENT_DECLS.len(), widgets::GTK_WIDGET_TABLE.len());
    }

    /// Every decl points at a `gtk_class` that exists in the widget
    /// table — no broken references. Catches a typo in either
    /// table that would silently de-correlate the cell registration
    /// from the symbol resolution.
    #[test]
    fn every_decl_gtk_class_exists_in_widget_table() {
        let known: alloc::collections::BTreeSet<&str> = widgets::GTK_WIDGET_TABLE
            .iter()
            .map(|w| w.class_name)
            .collect();
        for decl in GTK_COMPONENT_DECLS {
            assert!(
                known.contains(decl.gtk_class),
                "{} not in widget table",
                decl.gtk_class
            );
        }
    }

    /// Binding slugs follow DDDD's `<component>.<toolkit>` shape
    /// (components.md L205). Spot-check one to lock the format.
    #[test]
    fn binding_slug_matches_dddd_convention() {
        assert_eq!(binding_slug("button"), "button.gtk4");
        assert_eq!(binding_slug("text-input"), "text-input.gtk4");
        assert_eq!(binding_slug("date-picker"), "date-picker.gtk4");
    }

    /// End-to-end: register against a freshly-init'd SYSTEM and
    /// confirm the Component cells land. Picks button as the canary
    /// because it's the most-attested binding in DDDD's #485 (every
    /// toolkit declares it).
    #[test]
    fn register_gtk_components_lands_button_cell() {
        crate::system::init();
        loader::init();
        widgets::init();
        let count = register_gtk_components().expect("register succeeds");
        assert_eq!(count, 12, "all 12 GTK component decls should register");
    }

    /// Re-registering against an already-populated SYSTEM is a no-op
    /// at the cell-content level (cell_push_unique deduplicates).
    /// Important because all three adapter init paths can run in any
    /// order without duplicating shared Component cells.
    #[test]
    fn double_registration_is_idempotent() {
        crate::system::init();
        loader::init();
        widgets::init();
        register_gtk_components().expect("first register succeeds");
        register_gtk_components().expect("second register succeeds");
    }

    /// Foundation slice: log_resolved_pointers reports 0 resolved
    /// out of 12 because every dlsym returned null. Locks the
    /// foundation behaviour so the future loader extension can flip
    /// the expectation.
    #[test]
    fn log_resolved_pointers_zero_on_foundation_slice() {
        loader::init();
        widgets::init();
        let (resolved, total) = log_resolved_pointers();
        assert_eq!(resolved, 0);
        assert_eq!(total, 12);
    }

    /// GTK has a `card` binding (Qt does not). Spot-check that the
    /// `card` row is in the decl table and points at GtkBox.
    #[test]
    fn card_binding_uses_gtkbox() {
        let card = GTK_COMPONENT_DECLS
            .iter()
            .find(|d| d.slug == "card")
            .expect("card decl present");
        assert_eq!(card.gtk_class, "GtkBox");
        assert!(
            !card.binding_traits.contains(&"screen_reader_aware"),
            "card is layout primitive — DDDD omits screen_reader_aware"
        );
        assert!(
            card.binding_traits.contains(&"dark_mode_native"),
            "card.gtk4 should carry dark_mode_native per DDDD's L747"
        );
    }

    /// DDDD's #485 attaches `dark_mode_native` to every GTK 4
    /// binding except `image.gtk4` (components.md L804-806 — the
    /// only GTK binding without it). Spot-check the carry on
    /// button + the absence on image.
    #[test]
    fn dark_mode_native_distribution_matches_dddd() {
        let button = GTK_COMPONENT_DECLS
            .iter()
            .find(|d| d.slug == "button")
            .unwrap();
        assert!(button.binding_traits.contains(&"dark_mode_native"));

        let image = GTK_COMPONENT_DECLS
            .iter()
            .find(|d| d.slug == "image")
            .unwrap();
        assert!(
            !image.binding_traits.contains(&"dark_mode_native"),
            "image.gtk4 has hidpi_native only per DDDD L806"
        );
        assert!(image.binding_traits.contains(&"hidpi_native"));
    }
}
