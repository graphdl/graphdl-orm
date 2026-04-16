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

use crate::ast::{self, Func, Object};

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
pub fn prune_to_declared(result: &Object, snapshot: &Object, write_targets: &[&str]) -> Object {
    let result_map = match result.as_map() {
        Some(m) => m,
        None => return Object::Bottom,
    };
    let snap_map = snapshot.as_map();

    // Debug check: every changed cell in result must be in write_targets.
    // Release builds skip this entirely so the fast path stays cheap.
    #[cfg(debug_assertions)]
    {
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
}
