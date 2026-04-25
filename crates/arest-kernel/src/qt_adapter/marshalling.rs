// crates/arest-kernel/src/qt_adapter/marshalling.rs
//
// Property + signal marshalling between AREST values and Qt's
// `QObject::setProperty` / `QObject::connect` reflection APIs.
//
// Qt's reflection model
// ---------------------
// QObject's `setProperty(const char *name, const QVariant &value)`
// returns bool — true on success. The implementation walks the
// `QMetaObject::propertyCount()` table looking for a property with
// the matching name; if found, it invokes the property's setter via
// the `QMetaProperty::write` thunk after coercing the QVariant. Same
// surface across every QObject subclass — no per-class FFI bindings
// needed; the QMetaObject pointer (cached per widget class in
// `widgets.rs`) is enough to drive the lookup at runtime.
//
// `QObject::connect(sender, signal, receiver, slot)` wires a callback
// — the AREST side wants to bridge a Qt signal (e.g. QPushButton's
// `clicked()`) into an AREST event handler. The connect call goes
// through `QMetaObject::indexOfSignal` + `QMetaObject::indexOfMethod`
// to resolve string signal names to method indices, then registers
// the connection in QObject's internal dispatch table.
//
// Foundation-slice scope
// ----------------------
// Real reflection requires the QMetaObject pointer to be non-null —
// which on the foundation slice it never is, because `loader.rs`
// can't load libqt6 yet. Both `set_property` and `connect_signal`
// are therefore no-op stubs that return `Err(Stub)` so callers can
// detect "marshalling not yet implemented" without panicking. The
// signature shapes match what #491's full property/signal binding
// will land — the body-swap when the loader extension lands is
// a focused change.
//
// Variant shape
// -------------
// `ComponentValue` is the AREST-side value type — a sum type of the
// Property Type categories DDDD's #485 declared (string, int, bool,
// enum, color, length, image, callback). On the foundation slice
// the type carries the value variants without yet implementing any
// of the QVariant marshalling — those land in #491.

use alloc::boxed::Box;
use alloc::string::String;
use core::ffi::c_void;

use super::widgets;

/// Opaque pointer to a Qt QObject instance. Real Qt code holds this
/// as `QObject *`; on the AREST side we treat it as a raw `*mut
/// c_void` because we never dereference the underlying memory layout
/// — every interaction goes through the QMetaObject thunks.
pub type QObjectPtr = *mut c_void;

/// AREST-side projection of Qt's `QVariant`. Mirrors DDDD's #485
/// `Property Type` enumeration (components.md L103-110): string, int,
/// bool, enum, color, length, image, callback. The marshalling layer
/// (#491) coerces between this and the Qt `QVariant` type tag system
/// at the FFI boundary.
///
/// On the foundation slice we only need the Rust-side ADT; the
/// QVariant coercion is unimplemented (the conversion table is
/// substantial and lands with the loader extension).
#[derive(Clone, Debug)]
pub enum ComponentValue {
    String(String),
    Int(i64),
    Bool(bool),
    /// `enum`-typed property — the value is the enum-case slug
    /// (e.g. `"contain"` / `"cover"` for the `image.fit` property
    /// DDDD's #485 declared in components.md L580).
    Enum(String),
    /// `color`-typed property — resolves to a #432 ColorToken slug
    /// (e.g. `"surface"` / `"primary"`). The marshalling layer
    /// coerces the token to an actual Qt `QColor` via the design-
    /// token engine (#432) before handing to QVariant.
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

/// Outcome of a marshalling call. `Ok` on a successful round-trip
/// (set_property accepted, connect_signal wired); `Err(Stub)` on
/// the foundation slice (no library loaded → no QMetaObject to
/// drive the reflection); `Err(UnknownProperty)` when the property
/// name doesn't exist on the resolved QMetaObject (future-state
/// branch); `Err(TypeMismatch)` when the QVariant coercion fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarshalError {
    /// Foundation-slice fallback — the QMetaObject pointer for the
    /// requested widget class is null because the library never
    /// loaded. Callers can interpret this as "marshalling deferred
    /// to a later boot stage" and silently skip.
    Stub,
    /// The widget class isn't in `widgets::QT_WIDGET_TABLE`. Caller
    /// supplied a class name that the adapter never registered.
    UnknownClass,
    /// The property name isn't in the QMetaObject's property table
    /// (real-loader branch). Foundation slice never reaches here.
    UnknownProperty,
    /// `ComponentValue` variant doesn't coerce to the property's
    /// declared `Property Type` (real-loader branch).
    TypeMismatch,
}

/// `setProperty(name, value)` on a Qt widget. Resolves the widget's
/// QMetaObject pointer via `widgets::resolved(class_name)`, then
/// (on a future-state real-loader branch) walks the property table
/// and invokes the setter thunk after coercing `value` to a QVariant.
///
/// Foundation-slice behaviour: returns `Err(Stub)` because
/// `widgets::resolved` always reports null on the foundation slice
/// (no library loaded → no real QMetaObject). Callers can detect
/// this and silently skip the marshalling call until the loader
/// extension lands.
///
/// SAFETY: `widget` is treated as opaque — we never dereference it
/// in this stub. The future-state real-loader branch will require
/// the caller to guarantee `widget` is a live QObject of the
/// declared `class_name`; we'll thread the SAFETY contract there.
pub fn set_property(
    _widget: QObjectPtr,
    class_name: &str,
    _name: &str,
    _value: ComponentValue,
) -> Result<(), MarshalError> {
    let sym = widgets::resolved(class_name).ok_or(MarshalError::UnknownClass)?;
    if sym.is_null() {
        // Foundation-slice path — no library → no QMetaObject →
        // nothing to drive setProperty against.
        return Err(MarshalError::Stub);
    }
    // Real loader path — would call `meta->indexOfProperty(name)`
    // then `meta->property(idx).write(widget, qvariant)`. Future
    // work; the foundation slice never reaches this branch because
    // `sym.is_null()` short-circuits above.
    Err(MarshalError::Stub)
}

/// `connect(sender, signal, receiver, slot)` — wire `callback` to be
/// invoked when `widget` emits `signal`. Resolves the QMetaObject
/// pointer the same way `set_property` does; on the future-state
/// real-loader branch the call goes through
/// `QMetaObject::indexOfSignal` + an internal slot that re-enters
/// the AREST runtime with the captured callback.
///
/// Foundation-slice behaviour: returns `Err(Stub)` for the same
/// reason `set_property` does — no QMetaObject pointer to drive
/// the connection.
///
/// SAFETY: same caveat as `set_property` for the future-state branch.
pub fn connect_signal(
    _widget: QObjectPtr,
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
    use crate::qt_adapter::{loader, widgets};

    /// Foundation-slice: `set_property` against a registered class
    /// returns `Err(Stub)` because no QMetaObject pointer is
    /// resolved.
    #[test]
    fn set_property_stub_on_foundation_slice() {
        loader::init();
        widgets::init();
        let r = set_property(
            core::ptr::null_mut(),
            "QPushButton",
            "text",
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
            "QSpinBox", // not in QT_WIDGET_TABLE
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
            "QPushButton",
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
            "QSpinBox",
            "valueChanged",
            alloc::boxed::Box::new(|| {}),
        );
        assert_eq!(r, Err(MarshalError::UnknownClass));
    }

    /// `ComponentValue` covers DDDD's #485 Property Type enumeration
    /// — eight variants matching components.md L103-110.
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
