// crates/arest/src/declared_writes.rs
//
// apply_with_declared_writes (#186) — opt-in variant of apply that lets
// the caller pre-declare which cells the op will write. Skips the
// O(all_cells) diff scan that try_commit_diff (#163) performs today.
//
// Surfaced by FPGA work: on silicon the reducer drives write-enable
// lines directly — no diff step. Bringing that model to the CPU means
// callers that KNOW their write targets (system verbs `create:{noun}`,
// `update:{noun}`, `transition:{noun}`) can commit in O(|writes|)
// instead of O(|all_cells|).
//
// Correctness discipline: the pre-declared write_targets form a strict
// superset of cells actually modified. If the apply result touches a
// cell NOT in write_targets, the extra change is silently dropped and
// a debug_assert! fires — catching the bug in dev without breaking
// release. Opaque ops whose writes aren't known up front (compile,
// user-defined Defs) keep using the existing diff path.
//
// Sec-5 (#322): declared-writes is ALSO a capability check. When a
// user-authored def runs under `apply_with_caps`, any `Func::Store` to
// a cell outside the declared allow-list collapses to ⊥ — not a silent
// drop. The top of the capability stack is consulted on every store;
// when the stack is empty, stores are unrestricted (system/kernel
// mode, preserving legacy apply() behavior). `Func::Def(name)` auto-
// scopes: if DEFS contains `allowed_writes:{name}`, that cap set is
// pushed for the body. Metamodel cells (Noun, FactType, Role, ...)
// form a protected set — no user scope may write them even if their
// declaration names them; only system mode (or the "*" wildcard frame
// reserved for compile-machinery) may touch them.

use crate::ast::{self, Func, Object};
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};
use hashbrown::HashSet;

// ── Capability stack (thread-local) ──────────────────────────────────
//
// A frame on the stack is a HashSet<String> of cell names the current
// scope may write. `apply_with_caps` pushes; the Drop on CapGuard pops
// on return (exception-safe, supports nested user-def calls).
//
// Empty stack = "system mode" (no check). Non-empty = user mode; the
// top frame is the effective allow-list for store checks.

#[cfg(not(feature = "no_std"))]
std::thread_local! {
    static CAP_STACK: core::cell::RefCell<Vec<HashSet<String>>> =
        const { core::cell::RefCell::new(Vec::new()) };
}

/// Push `allowed` onto the capability stack. Returned guard pops on drop.
/// Public for `ast.rs` so `Func::Def` can swap caps for a body.
#[cfg(not(feature = "no_std"))]
pub(crate) fn push_caps(allowed: HashSet<String>) -> CapGuard {
    CAP_STACK.with(|s| s.borrow_mut().push(allowed));
    CapGuard { _priv: () }
}

/// Drop guard — pops the capability stack on scope exit.
#[cfg(not(feature = "no_std"))]
pub(crate) struct CapGuard { _priv: () }

#[cfg(not(feature = "no_std"))]
impl Drop for CapGuard {
    fn drop(&mut self) {
        CAP_STACK.with(|s| { s.borrow_mut().pop(); });
    }
}

/// Metamodel cells that are exclusively compile-machinery territory.
/// Under any user-scoped capability frame (non-empty stack without the
/// "*" wildcard), writes to these cells are refused even if the user's
/// declared allow-list names them — the protected set trumps forged
/// declarations. Compile/kernel paths run either with no frame (system
/// mode) or with the "*" wildcard (explicit escalation), both of which
/// bypass the protected check.
///
/// Statement_* and Role_Reference_* are prefix-matched: Stage-1/2
/// emit per-token cells like `Statement_has_Classification` and
/// `Role_Reference_<id>` that belong to the same protected lane.
const PROTECTED_METAMODEL_CELLS: &[&str] = &[
    "Noun", "FactType", "Role", "Constraint", "Subtype",
    "DerivationRule", "InstanceFact", "EnumValues",
];
const PROTECTED_METAMODEL_PREFIXES: &[&str] = &[
    "Statement_", "Role_Reference_",
];

fn is_protected_metamodel(cell: &str) -> bool {
    PROTECTED_METAMODEL_CELLS.iter().any(|p| *p == cell)
        || PROTECTED_METAMODEL_PREFIXES.iter().any(|p| cell.starts_with(p))
}

/// True when a store to `cell` is permitted by the current capability
/// frame. Decision lattice (top to bottom, first match wins):
///   1. Empty stack   → system mode, unrestricted.
///   2. "*" in frame  → trusted compile-machinery scope, unrestricted.
///   3. Protected     → refused (user scope cannot touch metamodel,
///                      even if the declaration names it).
///   4. In frame      → allowed (user scope with matching declaration).
///   5. Otherwise     → refused.
///
/// Called from the `Func::Store` dispatch in `ast::apply_nonbottom`.
/// The low-level `ast::store`/`cell_push` helpers intentionally do NOT
/// consult this — they're shared by trusted engine paths that should
/// bypass caps (write to `runtime_registered_names`, audit trails,
/// Citation provenance, etc.).
#[cfg(not(feature = "no_std"))]
pub fn is_store_allowed(cell: &str) -> bool {
    CAP_STACK.with(|s| {
        match s.borrow().last() {
            None => true,
            Some(frame) if frame.contains("*") => true,
            Some(_) if is_protected_metamodel(cell) => false,
            Some(frame) => frame.contains(cell),
        }
    })
}

// no_std build: capabilities are a no-op. The kernel image runs only
// compile-authored code, so there's no user-code threat surface there.
#[cfg(feature = "no_std")]
pub(crate) fn push_caps(_allowed: HashSet<String>) -> CapGuard {
    CapGuard { _priv: () }
}
#[cfg(feature = "no_std")]
pub(crate) struct CapGuard { _priv: () }
#[cfg(feature = "no_std")]
pub fn is_store_allowed(_cell: &str) -> bool { true }

/// Apply `func` with `allowed` pushed as the current capability frame.
/// Restores the previous stack on return. Use this as the entry point
/// when executing a user-authored def whose allowed_writes are known.
///
/// Intended callers: runtime dispatch of `derivation:*`, `constraint:*`
/// (from `create_via_defs` / `update_via_defs` paths), and tests.
pub fn apply_with_caps(
    func: &Func,
    x: &Object,
    d: &Object,
    allowed: &HashSet<String>,
) -> Object {
    let _guard = push_caps(allowed.clone());
    ast::apply(func, x, d)
}

/// Run `apply(func, x, d)` and return an Object containing ONLY the
/// cells named in `write_targets`. Other cells in the apply result
/// are dropped. A debug-assert fires if a changed-but-undeclared cell
/// is observed, which signals a broken declared-writes contract.
///
/// The return shape is an Object::Map with exactly the declared keys
/// present (missing keys come through as Object::Bottom). Callers
/// pair this with a commit step that writes only those cells.
pub fn apply_with_declared_writes(
    func: &Func,
    x: &Object,
    d: &Object,
    write_targets: &[&str],
) -> Object {
    let full = ast::apply(func, x, d);
    prune_to_declared(&full, d, write_targets)
}

/// Prune an apply result to the declared set of write targets.
/// Pure — no state mutation, no I/O. Exposed for direct testing.
pub fn prune_to_declared(
    result: &Object,
    #[cfg_attr(not(debug_assertions), allow(unused_variables))] snapshot: &Object,
    write_targets: &[&str],
) -> Object {
    let result_map = match result.as_map() {
        Some(m) => m,
        None => return Object::Bottom,
    };
    // Debug check: every changed cell in result must be in write_targets.
    // Release builds skip this entirely so the fast path stays cheap.
    #[cfg(debug_assertions)]
    {
        let snap_map = snapshot.as_map();
        for (key, new_val) in result_map.iter() {
            let snap_val = snap_map.and_then(|m| m.get(key));
            let changed = snap_val.map_or(true, |old| old != new_val);
            if changed {
                debug_assert!(
                    write_targets.iter().any(|t| *t == key.as_str()),
                    "declared-writes contract violated: cell `{}` changed but is not in write_targets ({:?})",
                    key, write_targets,
                );
            }
        }
    }

    let mut out = hashbrown::HashMap::with_capacity(write_targets.len());
    for target in write_targets {
        let val = result_map
            .get(*target)
            .cloned()
            .unwrap_or(Object::Bottom);
        out.insert((*target).to_string(), val);
    }
    Object::Map(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map_of(pairs: &[(&str, Object)]) -> Object {
        let m: hashbrown::HashMap<String, Object> = pairs.iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        Object::Map(m)
    }

    #[test]
    fn prune_returns_declared_cells_from_result() {
        let snap = map_of(&[
            ("a", Object::Atom("1".to_string())),
            ("b", Object::Atom("2".to_string())),
            ("c", Object::Atom("3".to_string())),
        ]);
        // Result changes only `b`; declare `b` as the write target.
        let result = map_of(&[
            ("a", Object::Atom("1".to_string())),
            ("b", Object::Atom("99".to_string())),
            ("c", Object::Atom("3".to_string())),
        ]);
        let pruned = prune_to_declared(&result, &snap, &["b"]);
        let pm = pruned.as_map().unwrap();
        assert_eq!(pm.len(), 1, "only `b` must be in the pruned output");
        assert_eq!(pm.get("b"), Some(&Object::Atom("99".to_string())));
    }

    #[test]
    fn prune_fills_missing_declared_with_bottom() {
        let snap = map_of(&[("a", Object::Atom("1".to_string()))]);
        // Result has no `missing` cell at all.
        let result = map_of(&[("a", Object::Atom("1".to_string()))]);
        let pruned = prune_to_declared(&result, &snap, &["a", "missing"]);
        let pm = pruned.as_map().unwrap();
        assert_eq!(pm.len(), 2);
        assert_eq!(pm.get("a"), Some(&Object::Atom("1".to_string())));
        assert_eq!(pm.get("missing"), Some(&Object::Bottom));
    }

    #[test]
    fn prune_with_empty_declared_returns_empty_map() {
        let snap = map_of(&[("a", Object::Atom("1".to_string()))]);
        let result = map_of(&[("a", Object::Atom("1".to_string()))]);
        let pruned = prune_to_declared(&result, &snap, &[]);
        assert_eq!(pruned.as_map().unwrap().len(), 0);
    }

    #[test]
    fn prune_of_non_map_result_returns_bottom() {
        // Apply can return a non-Map (Atom, Seq, Bottom) when the func
        // doesn't shape as a state transition. Declared-writes is a
        // state-commit helper only; non-Map → Bottom signals "no cells
        // to commit" cleanly.
        let snap = map_of(&[("a", Object::Atom("1".to_string()))]);
        assert_eq!(
            prune_to_declared(&Object::Atom("nope".to_string()), &snap, &["a"]),
            Object::Bottom,
        );
        assert_eq!(
            prune_to_declared(&Object::Bottom, &snap, &["a"]),
            Object::Bottom,
        );
    }

    #[test]
    fn prune_emits_same_value_when_declared_cell_didnt_change() {
        let snap = map_of(&[("a", Object::Atom("1".to_string()))]);
        let result = map_of(&[("a", Object::Atom("1".to_string()))]);
        let pruned = prune_to_declared(&result, &snap, &["a"]);
        assert_eq!(pruned.as_map().unwrap().get("a"), Some(&Object::Atom("1".to_string())));
    }

    #[test]
    #[cfg_attr(not(debug_assertions), ignore)]
    #[should_panic(expected = "declared-writes contract violated")]
    fn undeclared_change_panics_in_debug_builds() {
        // Declared `a` only but the result changes `b` — debug-assert fires.
        let snap = map_of(&[
            ("a", Object::Atom("1".to_string())),
            ("b", Object::Atom("2".to_string())),
        ]);
        let result = map_of(&[
            ("a", Object::Atom("1".to_string())),
            ("b", Object::Atom("BAD".to_string())),
        ]);
        let _ = prune_to_declared(&result, &snap, &["a"]);
    }

    // ── Sec-5: declared-writes as enforced capability ────────────────

    /// apply_with_caps pushes an allow-list of cell names onto the
    /// capability stack for the duration of `apply`. Any Func::Store
    /// attempt to a cell outside the allow-list collapses to ⊥.
    /// Baseline: a store to an out-of-set cell must return Bottom.
    #[test]
    fn apply_with_caps_refuses_store_outside_allowed_set() {
        let state = Object::Map(hashbrown::HashMap::new());
        // Func::Store:<name, contents, D> — attempts to write "Noun".
        let input = Object::seq(vec![
            Object::atom("Noun"),
            Object::atom("malicious"),
            state.clone(),
        ]);
        let allowed: hashbrown::HashSet<String> =
            ["my_cell".to_string()].into_iter().collect();
        let result = apply_with_caps(&Func::Store, &input, &state, &allowed);
        assert_eq!(
            result, Object::Bottom,
            "Func::Store to 'Noun' under caps={{'my_cell'}} must return Bottom"
        );
    }

    /// Complement: a store to a cell INSIDE the allow-list succeeds
    /// normally. The effect of the store (cell present in the returned
    /// Object) is identical to plain `apply(Func::Store, ...)`.
    #[test]
    fn apply_with_caps_permits_store_inside_allowed_set() {
        let state = Object::Map(hashbrown::HashMap::new());
        let input = Object::seq(vec![
            Object::atom("my_cell"),
            Object::atom("ok"),
            state.clone(),
        ]);
        let allowed: hashbrown::HashSet<String> =
            ["my_cell".to_string()].into_iter().collect();
        let result = apply_with_caps(&Func::Store, &input, &state, &allowed);
        assert_eq!(
            ast::fetch("my_cell", &result),
            Object::atom("ok"),
            "store to declared cell must succeed; got result = {:?}",
            result
        );
    }

    /// Legacy apply (no caps pushed) must remain permissive — otherwise
    /// every existing caller that doesn't know about caps would regress.
    /// Empty capability stack = system mode = unrestricted.
    #[test]
    fn apply_without_caps_permits_all_stores() {
        let state = Object::Map(hashbrown::HashMap::new());
        let input = Object::seq(vec![
            Object::atom("Noun"),
            Object::atom("anything"),
            state.clone(),
        ]);
        let result = ast::apply(&Func::Store, &input, &state);
        assert_eq!(
            ast::fetch("Noun", &result),
            Object::atom("anything"),
            "apply() without caps must be unrestricted (system mode)",
        );
    }

    /// Capability stack must pop on drop even when the body returns
    /// Bottom — otherwise an escape via early-return would leak caps
    /// to subsequent unrelated apply() calls on the same thread.
    #[test]
    fn caps_pop_after_apply_with_caps_returns() {
        let state = Object::Map(hashbrown::HashMap::new());
        let denied_input = Object::seq(vec![
            Object::atom("x"),
            Object::atom("v"),
            state.clone(),
        ]);
        // Run once under an empty allow-list so the store is refused.
        let empty: hashbrown::HashSet<String> = hashbrown::HashSet::new();
        let _bottom = apply_with_caps(&Func::Store, &denied_input, &state, &empty);

        // Now the same store via plain apply() must succeed — no
        // leftover frame on the stack.
        let after = ast::apply(&Func::Store, &denied_input, &state);
        assert_eq!(
            ast::fetch("x", &after),
            Object::atom("v"),
            "caps must be popped when apply_with_caps returns"
        );
    }

    /// Func::Def(name) self-scopes: when DEFS contains
    /// `allowed_writes:{name}` as a Seq of atoms, that cell is the
    /// cap frame for the body's duration. A body that tries to store
    /// outside the declared set collapses to ⊥ even under a plain
    /// top-level apply().
    #[test]
    fn def_scopes_caps_from_allowed_writes_cell_in_defs() {
        // Body: store("Noun", "malicious", state) — tries to clobber
        // a metamodel cell. With allowed_writes:evil declaring only
        // ["good"], the store must be refused.
        let body = Func::Compose(
            Box::new(Func::Store),
            Box::new(Func::construction(vec![
                Func::constant(Object::atom("Noun")),
                Func::constant(Object::atom("malicious")),
                Func::Id,
            ])),
        );
        // Seed DEFS with the def body and the allowed_writes cell.
        let d0 = Object::Map(hashbrown::HashMap::new());
        let d1 = ast::store("evil", ast::func_to_object(&body), &d0);
        let d2 = ast::store(
            "allowed_writes:evil",
            Object::seq(vec![Object::atom("good")]),
            &d1,
        );
        let result = ast::apply(&Func::Def("evil".to_string()), &d2, &d2);
        assert_eq!(
            result, Object::Bottom,
            "Func::Def('evil') with allowed_writes=['good'] must refuse a body-store to 'Noun'"
        );
    }

    /// Complement: a Def whose body stores to a cell that IS in its
    /// declared allowed_writes succeeds. The returned state has the
    /// cell written.
    #[test]
    fn def_permits_body_store_to_declared_cell() {
        let body = Func::Compose(
            Box::new(Func::Store),
            Box::new(Func::construction(vec![
                Func::constant(Object::atom("good")),
                Func::constant(Object::atom("yes")),
                Func::Id,
            ])),
        );
        let d0 = Object::Map(hashbrown::HashMap::new());
        let d1 = ast::store("good_def", ast::func_to_object(&body), &d0);
        let d2 = ast::store(
            "allowed_writes:good_def",
            Object::seq(vec![Object::atom("good")]),
            &d1,
        );
        let result = ast::apply(&Func::Def("good_def".to_string()), &d2, &d2);
        assert_eq!(
            ast::fetch("good", &result),
            Object::atom("yes"),
            "Func::Def with matching allowed_writes must permit the store"
        );
    }

    /// Absence of `allowed_writes:{name}` means "no declaration" —
    /// that preserves legacy behavior for existing compiled defs, so
    /// the established test baseline doesn't regress. Caps only apply
    /// when the def record explicitly declares them.
    #[test]
    fn def_without_allowed_writes_is_unrestricted() {
        let body = Func::Compose(
            Box::new(Func::Store),
            Box::new(Func::construction(vec![
                Func::constant(Object::atom("Noun")),
                Func::constant(Object::atom("x")),
                Func::Id,
            ])),
        );
        let d0 = Object::Map(hashbrown::HashMap::new());
        let d1 = ast::store("legacy", ast::func_to_object(&body), &d0);
        let result = ast::apply(&Func::Def("legacy".to_string()), &d1, &d1);
        assert_eq!(
            ast::fetch("Noun", &result),
            Object::atom("x"),
            "legacy defs (no allowed_writes cell) must be unrestricted"
        );
    }

    /// Protected metamodel cells are never writable from user scope,
    /// even if the reading declares one in its allowed_writes. A
    /// malicious reading with consequent = "Noun" must not be able to
    /// forge its way past the protected-set check. Compile-machinery
    /// bypasses this by running with the "*" wildcard frame (or no
    /// frame at all — system mode).
    #[test]
    fn user_frame_cannot_write_protected_metamodel_cell_even_if_declared() {
        let state = Object::Map(hashbrown::HashMap::new());
        let input = Object::seq(vec![
            Object::atom("Noun"),
            Object::atom("forged"),
            state.clone(),
        ]);
        let forged_caps: hashbrown::HashSet<String> =
            ["Noun".to_string()].into_iter().collect();
        let result = apply_with_caps(&Func::Store, &input, &state, &forged_caps);
        assert_eq!(
            result, Object::Bottom,
            "protected metamodel cell 'Noun' must not be writable from user scope even if declared"
        );
    }

    /// `"*"` wildcard marks a system/compile-machinery scope — those
    /// are permitted to touch metamodel cells. The wildcard is only
    /// emitted by the compiler for kernel defs; user-authored readings
    /// never produce it.
    #[test]
    fn wildcard_frame_permits_writes_to_protected_cells() {
        let state = Object::Map(hashbrown::HashMap::new());
        let input = Object::seq(vec![
            Object::atom("Noun"),
            Object::atom("compile-write"),
            state.clone(),
        ]);
        let sys_caps: hashbrown::HashSet<String> =
            ["*".to_string()].into_iter().collect();
        let result = apply_with_caps(&Func::Store, &input, &state, &sys_caps);
        assert_eq!(
            ast::fetch("Noun", &result),
            Object::atom("compile-write"),
            "'*' wildcard (system/compile scope) must allow metamodel writes"
        );
    }

    /// Cover the full protected-set roster from the handoff: Noun,
    /// FactType, Role, Constraint, Subtype, DerivationRule,
    /// InstanceFact, EnumValues, Statement_*, Role_Reference_*. A
    /// lookup table drives one store per name — regression guard
    /// against the set shrinking on a future refactor.
    #[test]
    fn protected_cell_set_covers_all_metamodel_names() {
        let protected = [
            "Noun", "FactType", "Role", "Constraint", "Subtype",
            "DerivationRule", "InstanceFact", "EnumValues",
            "Statement_has_Classification",  // Statement_* prefix
            "Role_Reference_something",       // Role_Reference_* prefix
        ];
        let declared_for_each: hashbrown::HashSet<String> =
            protected.iter().map(|s| s.to_string()).collect();
        for name in &protected {
            let state = Object::Map(hashbrown::HashMap::new());
            let input = Object::seq(vec![
                Object::atom(name),
                Object::atom("x"),
                state.clone(),
            ]);
            let result = apply_with_caps(&Func::Store, &input, &state, &declared_for_each);
            assert_eq!(
                result, Object::Bottom,
                "protected metamodel cell `{}` must be refused under user scope",
                name
            );
        }
    }
}
