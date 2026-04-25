// crates/arest-kernel/src/gtk_adapter/widgets.rs
//
// GTK 4 widget table — for each widget class DDDD's #485 declared an
// `ImplementationBinding` for in `readings/ui/components.md`, hold a
// `*const GType` resolved via `dlsym(handle, "g_<class>_get_type")`
// against the appropriate library (libgobject-2.0.so.0 for the
// foundational GObject type system; libgtk-4.so.1 for every concrete
// widget — GTK's library split).
//
// GObject's reflection model
// --------------------------
// GTK 4 builds on GObject's type system: every widget class
// registers a `GType` (an opaque `gsize` ID returned by
// `g_<class>_get_type()`) at first call to its accessor, with the
// returned ID describing the class's properties via
// `GParamSpec`-tagged entries (registered at class-init time via
// `g_object_class_install_property`) and its signals via
// `g_signal_new`. AREST's marshalling layer (`marshalling.rs`)
// reaches the property table to implement `g_object_set_property
// (obj, name, GValue)` reflection, and the signal table to wire
// `connect_signal` callbacks via `g_signal_connect`.
//
// On the foundation slice we only need the *pointer to* (or rather
// the address of the get-type accessor function for) each widget
// class — we don't decode the property table (that's #491's job).
// The pointer is what `binding.rs` records as the resolved Symbol
// value for each `ImplementationBinding` cell so that future
// composition work (#489) can hand it back to `marshalling.rs`.
//
// Symbol naming
// -------------
// GTK is a pure-C library: every widget class's get-type accessor
// is exported as `g_<lowercase_class_with_underscores>_get_type`,
// e.g. `g_button_get_type` for GtkButton, `g_drop_down_get_type`
// for GtkDropDown. No name mangling — unlike Qt's
// `_ZN11QPushButton16staticMetaObjectE`. (GTK's class registration
// uses the `gtk_<class>_get_type` form too in modern releases —
// `g_<class>_get_type` is the older accessor; modern GTK 4 ships
// both variants in the dynamic table for ABI compatibility. We use
// the `gtk_<class>_get_type` form here — it's the canonical
// accessor across GTK 4 minor releases.)
//
// Foundation-slice behaviour
// --------------------------
// `init()` walks the `GTK_WIDGET_TABLE` constant, calls
// `loader::dlsym` for each entry, and writes the resolved pointer
// into the `RESOLVED_SYMBOLS` map. On the foundation slice every
// entry comes back null. `binding.rs` consumes the table by
// class-name key, not by pointer, so null pointers don't break
// registration — they just mean the cells point at nothing
// executable until the loader extension lands.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use core::ffi::c_void;
use spin::{Mutex, Once};

use super::loader;

/// Where in the GTK 4 library tree a class's get-type accessor
/// lives. Drives which `LibHandle` we hand to `dlsym`. Today every
/// widget class lives in `libgtk-4.so.1`; the `GObject` variant is
/// reserved for the future when a non-widget reflection target
/// (GValue, GParamSpec) gets added to the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GtkLibrary {
    /// libglib-2.0.so.0 — base data structures (GHashTable, GList).
    GLib,
    /// libgobject-2.0.so.0 — GType, GValue, GObject, GParamSpec
    /// reflection machinery.
    GObject,
    /// libgtk-4.so.1 — every concrete widget class DDDD's #485
    /// registered.
    Gtk,
}

/// One row of the GTK widget table. `class_name` is the C class
/// name (matches DDDD's #485 ImplementationBinding Symbol values
/// verbatim — "GtkButton", "GtkEntry", etc.). `library` picks
/// which loaded library to `dlsym` against. `get_type_symbol` is
/// the C symbol name for the class's `g_<class>_get_type()`
/// accessor function; passed through to `loader::dlsym` as-is.
#[derive(Clone, Copy, Debug)]
pub struct GtkWidget {
    pub class_name: &'static str,
    pub library: GtkLibrary,
    pub get_type_symbol: &'static str,
}

/// The 12 GTK 4 widget classes DDDD's #485 ImplementationBinding
/// facts reference, in declaration order. Each row maps to exactly
/// one Component cell registered by `binding::register_gtk_components`.
/// The get-type accessor names follow the `gtk_<lowercase>_get_type`
/// convention GTK 4 ships across minor releases.
///
/// 12 classes (vs Qt's 11) because DDDD's #485 includes a `card`
/// binding for GTK (`Toolkit Symbol 'GtkBox'`, components.md L745) —
/// GTK has no first-class card primitive so the binding piggybacks
/// on GtkBox + add_css_class, which DDDD's binding-traits omit
/// `compact_native` for since it's a layout primitive rather than
/// a native-styled widget.
///
/// The 12 classes:
///   GtkButton       — button
///   GtkEntry        — text-input
///   GtkListView     — list
///   GtkCalendar     — date-picker
///   GtkDialog       — dialog
///   GtkPicture      — image (GTK 4 first-class image widget; GTK 3
///                     used GtkImage but #485 binds the GTK 4 surface)
///   GtkScale        — slider
///   GtkDropDown     — combo-box (GTK 4-specific; GTK 3 used GtkComboBox
///                     but DropDown is the modern 4.x replacement)
///   GtkProgressBar  — progress-bar
///   GtkCheckButton  — checkbox (GTK 4 unified GtkCheckButton + GtkToggleButton)
///   GtkNotebook     — tab
///   GtkBox          — card (GTK has no first-class card; #485
///                     reuses GtkBox + add_css_class('card'))
pub const GTK_WIDGET_TABLE: &[GtkWidget] = &[
    GtkWidget {
        class_name: "GtkButton",
        library: GtkLibrary::Gtk,
        get_type_symbol: "gtk_button_get_type",
    },
    GtkWidget {
        class_name: "GtkEntry",
        library: GtkLibrary::Gtk,
        get_type_symbol: "gtk_entry_get_type",
    },
    GtkWidget {
        class_name: "GtkListView",
        library: GtkLibrary::Gtk,
        get_type_symbol: "gtk_list_view_get_type",
    },
    GtkWidget {
        class_name: "GtkCalendar",
        library: GtkLibrary::Gtk,
        get_type_symbol: "gtk_calendar_get_type",
    },
    GtkWidget {
        class_name: "GtkDialog",
        library: GtkLibrary::Gtk,
        get_type_symbol: "gtk_dialog_get_type",
    },
    GtkWidget {
        class_name: "GtkPicture",
        library: GtkLibrary::Gtk,
        get_type_symbol: "gtk_picture_get_type",
    },
    GtkWidget {
        class_name: "GtkScale",
        library: GtkLibrary::Gtk,
        get_type_symbol: "gtk_scale_get_type",
    },
    GtkWidget {
        class_name: "GtkDropDown",
        library: GtkLibrary::Gtk,
        get_type_symbol: "gtk_drop_down_get_type",
    },
    GtkWidget {
        class_name: "GtkProgressBar",
        library: GtkLibrary::Gtk,
        get_type_symbol: "gtk_progress_bar_get_type",
    },
    GtkWidget {
        class_name: "GtkCheckButton",
        library: GtkLibrary::Gtk,
        get_type_symbol: "gtk_check_button_get_type",
    },
    GtkWidget {
        class_name: "GtkNotebook",
        library: GtkLibrary::Gtk,
        get_type_symbol: "gtk_notebook_get_type",
    },
    GtkWidget {
        class_name: "GtkBox",
        library: GtkLibrary::Gtk,
        get_type_symbol: "gtk_box_get_type",
    },
];

/// Send-safe wrapper around the resolved `*const c_void` symbol
/// pointer (the address of the `g_<class>_get_type` accessor
/// function). The pointer is either null (foundation-slice default)
/// or an address inside the loaded library's text segment (future
/// loader extension). Either way, single-threaded kernel boot —
/// Send/Sync only exist for the static `Once`/`Mutex` bounds.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SymbolPtr(pub *const c_void);

unsafe impl Send for SymbolPtr {}
unsafe impl Sync for SymbolPtr {}

impl SymbolPtr {
    pub const fn null() -> Self {
        Self(core::ptr::null())
    }

    pub fn is_null(&self) -> bool {
        self.0.is_null()
    }
}

/// Resolved-symbols table, keyed by the unmangled `class_name` from
/// `GTK_WIDGET_TABLE`. Populated once during `init`. Read by
/// `binding.rs` (to check whether a Symbol resolved cleanly — though
/// the cell registration is unconditional) and by `marshalling.rs`
/// (to find the GType accessor pointer for a given widget class
/// when `g_object_set_property` / `g_signal_connect` is invoked).
///
/// `BTreeMap` rather than a sized array so a future row addition
/// (e.g. GtkPopover, GtkSpinner) is a one-line table edit + a one-
/// line binding registration with no resize churn elsewhere.
static RESOLVED_SYMBOLS: Once<Mutex<BTreeMap<String, SymbolPtr>>> = Once::new();

/// Walk `GTK_WIDGET_TABLE`, resolve each entry's `get_type_symbol`
/// against the matching loaded library, and record the result in
/// `RESOLVED_SYMBOLS`. On the foundation slice every result is
/// `SymbolPtr::null()` because `loader::dlsym` always returns null
/// when the library never loaded. Idempotent — `Once::call_once`
/// short-circuits a second call.
pub fn init() {
    RESOLVED_SYMBOLS.call_once(|| {
        let mut map: BTreeMap<String, SymbolPtr> = BTreeMap::new();
        for w in GTK_WIDGET_TABLE {
            let handle = match w.library {
                GtkLibrary::GLib => loader::glib(),
                GtkLibrary::GObject => loader::gobject(),
                GtkLibrary::Gtk => loader::gtk(),
            };
            let raw = loader::dlsym(&handle, w.get_type_symbol);
            map.insert(w.class_name.to_string(), SymbolPtr(raw));
        }
        Mutex::new(map)
    });
}

/// Look up the resolved `*const GType` accessor pointer for a
/// widget class by its C name (`"GtkButton"`, `"GtkEntry"`, …).
/// Returns `None` when `init` hasn't run; returns
/// `Some(SymbolPtr::null())` on the foundation slice (the row was
/// registered but the dlsym returned null). Marshalling code
/// differentiates the two cases by checking `SymbolPtr::is_null`
/// after unwrapping the Option.
pub fn resolved(class_name: &str) -> Option<SymbolPtr> {
    let map = RESOLVED_SYMBOLS.get()?;
    map.lock().get(class_name).copied()
}

/// Iterate the resolved-symbols table in declaration order. Returns
/// `Vec<(class_name, SymbolPtr)>` so callers (binding.rs at registration
/// time) don't have to hold the inner mutex while traversing.
///
/// Returns an empty vec when `init` hasn't run.
pub fn iter_resolved() -> alloc::vec::Vec<(String, SymbolPtr)> {
    match RESOLVED_SYMBOLS.get() {
        Some(m) => m.lock().iter().map(|(k, v)| (k.clone(), *v)).collect(),
        None => alloc::vec::Vec::new(),
    }
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Foundation-slice: every row resolves to a null pointer
    /// because the libraries never loaded. This is the assertion
    /// that flips when the future loader extension lands real
    /// dlopen — at that point at least the widget classes whose
    /// libraries ARE present should resolve to non-null pointers.
    #[test]
    fn init_resolves_every_row_to_null_on_foundation_slice() {
        loader::init();
        init();
        for w in GTK_WIDGET_TABLE {
            let p = resolved(w.class_name).expect("init populated row");
            assert!(
                p.is_null(),
                "{} resolved non-null on foundation slice",
                w.class_name
            );
        }
    }

    /// The widget table has the 12 entries DDDD's #485 declared.
    /// Locks the table size against accidental drift — adding a row
    /// here without updating binding.rs would emit a Symbol pointer
    /// for a class with no Component cell, which is harmless but
    /// silently wastes a registration cycle.
    #[test]
    fn widget_table_has_expected_count() {
        assert_eq!(GTK_WIDGET_TABLE.len(), 12);
    }

    /// Spot-check the class-name → get-type-symbol mapping for the
    /// most-likely-to-be-resolved class (GtkButton). The accessor
    /// name follows `gtk_<lowercase>_get_type`; a typo here would
    /// silently break the future dlopen path.
    #[test]
    fn gtkbutton_get_type_symbol_matches_gtk4_convention() {
        let row = GTK_WIDGET_TABLE
            .iter()
            .find(|w| w.class_name == "GtkButton")
            .expect("GtkButton in table");
        assert_eq!(row.get_type_symbol, "gtk_button_get_type");
        assert_eq!(row.library, GtkLibrary::Gtk);
    }

    /// `iter_resolved` returns the same set of class names as
    /// `GTK_WIDGET_TABLE`. Catches a missing `Mutex::insert` if the
    /// init body is ever refactored.
    #[test]
    fn iter_resolved_yields_every_class_name() {
        loader::init();
        init();
        let resolved: alloc::collections::BTreeSet<String> =
            iter_resolved().into_iter().map(|(k, _)| k).collect();
        for w in GTK_WIDGET_TABLE {
            assert!(
                resolved.contains(w.class_name),
                "{} missing from resolved set",
                w.class_name
            );
        }
    }

    /// GTK 4-specific: GtkDropDown (the modern combo-box
    /// replacement) uses `gtk_drop_down_get_type` — the underscore
    /// split between "drop" and "down" is the convention catch
    /// most likely to drift if someone hand-types a new entry.
    #[test]
    fn gtkdropdown_get_type_symbol_uses_underscore_split() {
        let row = GTK_WIDGET_TABLE
            .iter()
            .find(|w| w.class_name == "GtkDropDown")
            .expect("GtkDropDown in table");
        assert_eq!(row.get_type_symbol, "gtk_drop_down_get_type");
    }
}
