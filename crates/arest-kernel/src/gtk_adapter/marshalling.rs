// crates/arest-kernel/src/gtk_adapter/marshalling.rs
//
// Property + signal marshalling between AREST values and GTK 4's
// `g_object_set_property` / `g_signal_connect` reflection APIs.
//
// GObject's reflection model
// --------------------------
// Every GTK widget is a GObject subclass; GObject ships a uniform
// `g_object_set_property(GObject *obj, const gchar *name, const
// GValue *value)` that walks the class's `GParamSpec` table looking
// for a property with the matching name. If found, it invokes the
// property's setter via the GParamSpec's `set_property` thunk after
// coercing the GValue (g_value_transform when the source type
// doesn't match the target). Same surface across every GObject
// subclass — no per-class FFI bindings needed; the GType (cached
// per widget class in `widgets.rs`) is enough to drive the lookup
// at runtime.
//
// `g_signal_connect(sender, signal_name, callback, user_data)`
// wires a callback — the AREST side wants to bridge a GTK signal
// (e.g. GtkButton's `clicked`) into an AREST event handler. The
// connect call goes through `g_signal_lookup` to resolve string
// signal names to signal IDs, then registers the connection in
// GObject's internal dispatch table.
//
// GObject vs Qt's QMetaObject
// ---------------------------
// Same problem (string-name → type-tagged-value setter), different
// shape:
//   * Qt's QVariant is a single tagged-union with built-in coercion.
//     `setProperty` returns bool — true on success.
//   * GObject's GValue is a flat (GType, union { gchar*, gint, … })
//     struct with explicit setters per type
//     (g_value_set_string, g_value_set_int, g_value_set_object, …)
//     and explicit transform callbacks for cross-type coercion
//     (g_value_register_transform_func). `g_object_set_property`
//     returns void — emits a runtime warning on type mismatch
//     instead of a return code, which the AREST marshaller has to
//     pre-check via `g_param_spec_get_value_type` before the call.
// The AREST-side `ComponentValue` ADT is shared with GGGG's
// qt_adapter (re-exported when `qt-adapter` is also enabled, see
// the cfg block below). When only `gtk-adapter` is enabled, we
// define a local copy with the same shape — the type surface stays
// consistent across adapters either way.
//
// Foundation-slice scope
// ----------------------
// Real reflection requires the GType pointer to be non-null —
// which on the foundation slice it never is, because `loader.rs`
// can't load libgtk-4 yet. Both `set_property` and `connect_signal`
// are therefore no-op stubs that return `Err(Stub)` so callers can
// detect "marshalling not yet implemented" without panicking. The
// signature shapes match what #491's full property/signal binding
// will land — the body-swap when the loader extension lands is a
// focused change.

use alloc::boxed::Box;
use core::ffi::c_void;

use super::widgets;

// ── ComponentValue ADT ────────────────────────────────────────────
//
// When `qt-adapter` is also enabled, re-export the ADT from
// GGGG's qt_adapter::marshalling so the type surface stays shared
// across adapters (a future cross-adapter coercion path can hand
// the same value to either backend without conversion). When only
// `gtk-adapter` is enabled, define a local copy with the same shape
// — the marshalling sites only depend on the variants, not the
// path. No feature-coupling either way.

#[cfg(feature = "qt-adapter")]
pub use crate::qt_adapter::marshalling::ComponentValue;

#[cfg(not(feature = "qt-adapter"))]
mod component_value {
    use alloc::string::String;

    /// AREST-side projection of GObject's `GValue`. Mirrors DDDD's
    /// #485 `Property Type` enumeration (components.md L103-110):
    /// string, int, bool, enum, color, length, image, callback. The
    /// marshalling layer (#491) coerces between this and the GValue
    /// type tag system at the FFI boundary (g_value_set_string for
    /// String, g_value_set_int for Int / Length, g_value_set_boolean
    /// for Bool, g_value_set_string for Enum + Color + Image since
    /// they're slug-typed, and the Callback variant routes through
    /// g_signal_connect rather than g_object_set_property).
    ///
    /// Shape matches GGGG's `qt_adapter::marshalling::ComponentValue`
    /// byte-for-byte so the two ADTs are interchangeable when both
    /// adapters are enabled (a future cross-adapter coercion path
    /// would re-export from one and use the other unchanged). On
    /// the foundation slice we only need the Rust-side ADT; the
    /// GValue coercion is unimplemented (the conversion table is
    /// substantial and lands with the loader extension).
    #[derive(Clone, Debug)]
    pub enum ComponentValue {
        String(String),
        Int(i64),
        Bool(bool),
        /// `enum`-typed property — the value is the enum-case slug
        /// (e.g. `"contain"` / `"cover"` for the `image.fit`
        /// property DDDD's #485 declared in components.md L580).
        Enum(String),
        /// `color`-typed property — resolves to a #432 ColorToken
        /// slug (e.g. `"surface"` / `"primary"`). The marshalling
        /// layer coerces the token to an actual GdkRGBA via the
        /// design-token engine (#432) before handing to GValue.
        Color(String),
        /// `length`-typed property — pixels.
        Length(i64),
        /// `image`-typed property — asset handle (e.g. `"icon-disk"`
        /// from `crate::icons::by_name`).
        Image(String),
        /// `callback` — opaque to the marshaller, passed through to
        /// the connect_signal pathway.
        Callback,
    }
}

#[cfg(not(feature = "qt-adapter"))]
pub use component_value::ComponentValue;

/// Opaque pointer to a GTK GObject instance. Real GTK code holds
/// this as `GObject *`; on the AREST side we treat it as a raw
/// `*mut c_void` because we never dereference the underlying memory
/// layout — every interaction goes through the GObject thunks.
pub type GObjectPtr = *mut c_void;

/// Outcome of a marshalling call. `Ok` on a successful round-trip
/// (set_property accepted, connect_signal wired); `Err(Stub)` on
/// the foundation slice (no library loaded → no GType pointer to
/// drive the reflection); `Err(UnknownProperty)` when the property
/// name doesn't exist on the resolved GParamSpec table (future-
/// state branch); `Err(TypeMismatch)` when the GValue coercion
/// fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarshalError {
    /// Foundation-slice fallback — the GType pointer for the
    /// requested widget class is null because the library never
    /// loaded. Callers can interpret this as "marshalling deferred
    /// to a later boot stage" and silently skip.
    Stub,
    /// The widget class isn't in `widgets::GTK_WIDGET_TABLE`. Caller
    /// supplied a class name that the adapter never registered.
    UnknownClass,
    /// The property name isn't in the GParamSpec table for the
    /// resolved class (real-loader branch). Foundation slice never
    /// reaches here.
    UnknownProperty,
    /// `ComponentValue` variant doesn't coerce to the property's
    /// declared `Property Type` (real-loader branch).
    TypeMismatch,
}

/// `g_object_set_property(obj, name, GValue)` on a GTK widget.
/// Resolves the widget's GType pointer via
/// `widgets::resolved(class_name)`, then (on a future-state real-
/// loader branch) walks the GParamSpec table and invokes the
/// setter thunk after coercing `value` to a GValue.
///
/// Foundation-slice behaviour: returns `Err(Stub)` because
/// `widgets::resolved` always reports null on the foundation slice
/// (no library loaded → no real GType). Callers can detect this
/// and silently skip the marshalling call until the loader
/// extension lands.
///
/// SAFETY: `widget` is treated as opaque — we never dereference it
/// in this stub. The future-state real-loader branch will require
/// the caller to guarantee `widget` is a live GObject of the
/// declared `class_name`; we'll thread the SAFETY contract there.
pub fn set_property(
    _widget: GObjectPtr,
    class_name: &str,
    _name: &str,
    _value: ComponentValue,
) -> Result<(), MarshalError> {
    let sym = widgets::resolved(class_name).ok_or(MarshalError::UnknownClass)?;
    if sym.is_null() {
        // Foundation-slice path — no library → no GType → nothing
        // to drive g_object_set_property against.
        return Err(MarshalError::Stub);
    }
    // Real loader path — would call `g_object_class_find_property
    // (klass, name)` then construct a GValue via
    // `g_value_init(&v, pspec->value_type) + g_value_set_<type>(&v,
    // …)` then `g_object_set_property(obj, name, &v)`. Future work;
    // the foundation slice never reaches this branch because
    // `sym.is_null()` short-circuits above.
    Err(MarshalError::Stub)
}

/// `g_signal_connect(sender, signal_name, callback, user_data)` —
/// wire `callback` to be invoked when `widget` emits `signal`.
/// Resolves the GType pointer the same way `set_property` does;
/// on the future-state real-loader branch the call goes through
/// `g_signal_lookup(signal_name, gtype)` to obtain the signal ID,
/// then `g_signal_connect_data(obj, signal_name, G_CALLBACK
/// (trampoline), user_data, destroy_notify, 0)` with a trampoline
/// that re-enters the AREST runtime with the captured callback.
///
/// Foundation-slice behaviour: returns `Err(Stub)` for the same
/// reason `set_property` does — no GType pointer to drive the
/// connection.
///
/// SAFETY: same caveat as `set_property` for the future-state
/// branch.
pub fn connect_signal(
    _widget: GObjectPtr,
    class_name: &str,
    _signal: &str,
    _callback: Box<dyn Fn() + Send + Sync>,
) -> Result<(), MarshalError> {
    let sym = widgets::resolved(class_name).ok_or(MarshalError::UnknownClass)?;
    if sym.is_null() {
        return Err(MarshalError::Stub);
    }
    Err(MarshalError::Stub)
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gtk_adapter::{loader, widgets};

    /// Foundation-slice: `set_property` against a registered class
    /// returns `Err(Stub)` because no GType pointer is resolved.
    #[test]
    fn set_property_stub_on_foundation_slice() {
        loader::init();
        widgets::init();
        let r = set_property(
            core::ptr::null_mut(),
            "GtkButton",
            "label",
            ComponentValue::String(alloc::string::String::from("OK")),
        );
        assert_eq!(r, Err(MarshalError::Stub));
    }

    /// `set_property` against a non-registered class returns
    /// `Err(UnknownClass)` — the caller asked for a widget class
    /// the adapter never registered.
    #[test]
    fn set_property_unknown_class() {
        loader::init();
        widgets::init();
        let r = set_property(
            core::ptr::null_mut(),
            "GtkSpinButton", // not in GTK_WIDGET_TABLE
            "value",
            ComponentValue::Int(0),
        );
        assert_eq!(r, Err(MarshalError::UnknownClass));
    }

    /// `connect_signal` against a registered class also stubs to
    /// `Err(Stub)` on the foundation slice.
    #[test]
    fn connect_signal_stub_on_foundation_slice() {
        loader::init();
        widgets::init();
        let r = connect_signal(
            core::ptr::null_mut(),
            "GtkButton",
            "clicked",
            alloc::boxed::Box::new(|| {}),
        );
        assert_eq!(r, Err(MarshalError::Stub));
    }

    /// `connect_signal` against a non-registered class returns
    /// `Err(UnknownClass)`.
    #[test]
    fn connect_signal_unknown_class() {
        loader::init();
        widgets::init();
        let r = connect_signal(
            core::ptr::null_mut(),
            "GtkSpinButton",
            "value-changed",
            alloc::boxed::Box::new(|| {}),
        );
        assert_eq!(r, Err(MarshalError::UnknownClass));
    }

    /// `ComponentValue` covers DDDD's #485 Property Type enumeration
    /// — eight variants matching components.md L103-110. Same shape
    /// as GGGG's qt_adapter::marshalling::ComponentValue (the cfg
    /// block at the top of this file picks one or the other).
    #[test]
    fn component_value_covers_dddd_property_types() {
        let _: ComponentValue = ComponentValue::String(alloc::string::String::new());
        let _: ComponentValue = ComponentValue::Int(0);
        let _: ComponentValue = ComponentValue::Bool(false);
        let _: ComponentValue = ComponentValue::Enum(alloc::string::String::new());
        let _: ComponentValue = ComponentValue::Color(alloc::string::String::new());
        let _: ComponentValue = ComponentValue::Length(0);
        let _: ComponentValue = ComponentValue::Image(alloc::string::String::new());
        let _: ComponentValue = ComponentValue::Callback;
    }
}
