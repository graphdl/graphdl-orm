// crates/arest/src/row_shape.rs
//
// Task #187 — typed row shape for Seq.
//
// `TypedSeq` is a lightweight decoration that wraps an Object::Seq whose
// every element is a homogeneous row (a Seq of [role_name, value] pairs
// whose keys are the same across all rows).  It does NOT change the Object
// enum; it lives beside it and is zero-cost to create from an existing Seq.
//
// Typical usage
// -------------
//   let ts = TypedSeq::from_cell("Order", &state_obj)?;
//   let idx = ts.column_index("status")?;   // O(n) on shape, usually ≤ 10 cols
//   let sub = ts.project(&["id", "status"]);
//   let back: Object = sub.to_object();

#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

use crate::ast::{Object, binding, fetch_or_phi};
use crate::sync::Arc;

// ── TypedSeq ────────────────────────────────────────────────────────────────

/// A `Seq` whose rows all have the same ordered set of column names.
///
/// Each row in `rows` is expected to be an `Object::Seq` of
/// `<[col_name, value]>` pairs — the same layout produced by
/// `ast::fact_from_pairs`.  The `shape` records the column names in
/// first-occurrence order drawn from the first row.
#[derive(Clone, Debug, PartialEq)]
pub struct TypedSeq {
    /// Column names, in the order they appear in each row.
    pub shape: Vec<String>,
    /// The raw rows; each row is an `Object` (typically `Object::Seq`).
    pub rows: Arc<[Object]>,
}

impl TypedSeq {
    // ── Construction ──────────────────────────────────────────────────────

    /// Build a `TypedSeq` from a named cell in a state object.
    ///
    /// `cell_name` is looked up via `fetch_or_phi`; the cell's contents must
    /// be an `Object::Seq`.  The shape is inferred from the **first row**:
    /// every `[key, _]` pair inside that row contributes one column name.
    ///
    /// Returns `None` when:
    ///  - the cell does not exist or is empty,
    ///  - the cell contents are not a `Seq`, or
    ///  - the first row has no recognisable key-value pairs.
    pub fn from_cell(cell_name: &str, obj: &Object) -> Option<Self> {
        let cell_obj = fetch_or_phi(cell_name, obj);
        let rows = cell_obj.as_seq()?;
        if rows.is_empty() {
            return None;
        }
        let first = &rows[0];
        let shape = infer_shape(first)?;
        if shape.is_empty() {
            return None;
        }
        Some(TypedSeq {
            shape,
            rows: rows.into(),
        })
    }

    /// Build a `TypedSeq` directly from an `Object::Seq` value (no cell lookup).
    ///
    /// Shape is inferred from the first row, same as `from_cell`.
    /// Returns `None` when the object is not a non-empty Seq or the first row
    /// carries no key-value pairs.
    pub fn from_seq(obj: &Object) -> Option<Self> {
        let rows = obj.as_seq()?;
        if rows.is_empty() {
            return None;
        }
        let shape = infer_shape(&rows[0])?;
        if shape.is_empty() {
            return None;
        }
        Some(TypedSeq {
            shape,
            rows: rows.into(),
        })
    }

    // ── Inspection ────────────────────────────────────────────────────────

    /// Return the 0-based index of `name` in the shape, or `None` if absent.
    ///
    /// Complexity: O(|shape|).  Shapes are typically small (≤ 20 columns),
    /// so a linear scan beats a HashMap for the common case.
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.shape.iter().position(|s| s == name)
    }

    /// Return the number of rows.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Return `true` when there are no rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    // ── Projection ────────────────────────────────────────────────────────

    /// Return a new `TypedSeq` containing only the named columns, in the
    /// order given by `columns`.
    ///
    /// Columns that don't exist in the shape are silently skipped both from
    /// the new shape and from every projected row.  If none of the requested
    /// columns exist, returns a `TypedSeq` with an empty shape and empty rows
    /// (all rows project to `Object::phi()`).
    pub fn project(&self, columns: &[&str]) -> TypedSeq {
        // Resolve which columns actually exist in our shape.
        let kept: Vec<&str> = columns
            .iter()
            .copied()
            .filter(|c| self.column_index(c).is_some())
            .collect();

        let new_shape: Vec<String> = kept.iter().map(|c| c.to_string()).collect();

        let new_rows: Vec<Object> = self
            .rows
            .iter()
            .map(|row| project_row(row, &kept))
            .collect();

        TypedSeq {
            shape: new_shape,
            rows: new_rows.into(),
        }
    }

    // ── Conversion ────────────────────────────────────────────────────────

    /// Convert back to an `Object::Seq` (the rows unchanged).
    ///
    /// This is the identity when the `TypedSeq` was constructed from a Seq;
    /// it allows callers to round-trip through `TypedSeq` without allocating
    /// new Objects for the rows.
    pub fn to_object(&self) -> Object {
        Object::Seq(Arc::clone(&self.rows))
    }

    /// Fetch the value of `column` from `row_index`, or `None` if out of
    /// range or the row doesn't carry that column.
    pub fn get(&self, row_index: usize, column: &str) -> Option<&str> {
        let row = self.rows.get(row_index)?;
        binding(row, column)
    }
}

// ── Private helpers ─────────────────────────────────────────────────────────

/// Extract the ordered column names from a single fact row.
///
/// A fact row is `Object::Seq([<key1,val1>, <key2,val2>, ...])`.
/// We pull out the `key` atom from each `[key, val]` pair.
fn infer_shape(row: &Object) -> Option<Vec<String>> {
    let pairs = row.as_seq()?;
    let shape: Vec<String> = pairs
        .iter()
        .filter_map(|pair| {
            let items = pair.as_seq()?;
            if items.len() == 2 {
                items[0].as_atom().map(|s| s.to_string())
            } else {
                None
            }
        })
        .collect();
    Some(shape)
}

/// Project a single row down to `kept` columns, rebuilding the `[key, val]`
/// pairs in the requested column order.
fn project_row(row: &Object, kept: &[&str]) -> Object {
    let pairs: Vec<Object> = kept
        .iter()
        .filter_map(|col| {
            // Find the pair in the row whose first element matches *col*.
            let items = row.as_seq()?;
            items.iter().find_map(|pair| {
                let p = pair.as_seq()?;
                if p.len() == 2 && p[0].as_atom() == Some(col) {
                    Some(pair.clone())
                } else {
                    None
                }
            })
        })
        .collect();

    if pairs.is_empty() {
        Object::phi()
    } else {
        Object::seq(pairs)
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{fact_from_pairs, store, Object};

    /// Build a small state with an "Order" cell containing three fact rows.
    fn order_state() -> Object {
        let rows = vec![
            fact_from_pairs(&[("id", "ord-1"), ("status", "Draft"),  ("customer", "acme")]),
            fact_from_pairs(&[("id", "ord-2"), ("status", "Active"), ("customer", "beta")]),
            fact_from_pairs(&[("id", "ord-3"), ("status", "Draft"),  ("customer", "gamma")]),
        ];
        let seq = Object::Seq(rows.into());
        store("Order", seq, &Object::phi())
    }

    // ── from_cell ─────────────────────────────────────────────────────────

    #[test]
    fn from_cell_infers_shape_from_first_row() {
        let state = order_state();
        let ts = TypedSeq::from_cell("Order", &state).expect("should succeed");
        assert_eq!(ts.shape, vec!["id", "status", "customer"]);
        assert_eq!(ts.len(), 3);
    }

    #[test]
    fn from_cell_returns_none_for_missing_cell() {
        let state = Object::phi();
        assert!(TypedSeq::from_cell("NoSuchCell", &state).is_none());
    }

    #[test]
    fn from_cell_returns_none_for_non_tuple_rows() {
        // A cell whose rows are plain atoms, not [key,val] pairs.
        let seq = Object::Seq(vec![Object::atom("foo"), Object::atom("bar")].into());
        let state = store("Flat", seq, &Object::phi());
        // infer_shape on Object::Atom("foo") returns Some([]) → None branch
        assert!(TypedSeq::from_cell("Flat", &state).is_none());
    }

    // ── column_index ──────────────────────────────────────────────────────

    #[test]
    fn column_index_finds_existing_columns() {
        let state = order_state();
        let ts = TypedSeq::from_cell("Order", &state).unwrap();
        assert_eq!(ts.column_index("id"),       Some(0));
        assert_eq!(ts.column_index("status"),   Some(1));
        assert_eq!(ts.column_index("customer"), Some(2));
    }

    #[test]
    fn column_index_returns_none_for_missing_column() {
        let state = order_state();
        let ts = TypedSeq::from_cell("Order", &state).unwrap();
        assert_eq!(ts.column_index("price"), None);
    }

    // ── project ───────────────────────────────────────────────────────────

    #[test]
    fn project_keeps_only_requested_columns() {
        let state = order_state();
        let ts = TypedSeq::from_cell("Order", &state).unwrap();
        let sub = ts.project(&["id", "status"]);

        assert_eq!(sub.shape, vec!["id", "status"]);
        assert_eq!(sub.len(), 3);

        // Each projected row must carry exactly two pairs.
        for row in sub.rows.iter() {
            let pairs = row.as_seq().expect("row must be a Seq");
            assert_eq!(pairs.len(), 2);
        }
    }

    #[test]
    fn project_skips_nonexistent_columns() {
        let state = order_state();
        let ts = TypedSeq::from_cell("Order", &state).unwrap();
        let sub = ts.project(&["id", "nonexistent"]);
        // Only "id" survived — shape has one column.
        assert_eq!(sub.shape, vec!["id"]);
    }

    #[test]
    fn project_preserves_column_order_from_request() {
        let state = order_state();
        let ts = TypedSeq::from_cell("Order", &state).unwrap();
        // Request columns in reverse order.
        let sub = ts.project(&["customer", "id"]);
        assert_eq!(sub.shape, vec!["customer", "id"]);

        // First row: customer=acme, id=ord-1
        assert_eq!(sub.get(0, "customer"), Some("acme"));
        assert_eq!(sub.get(0, "id"),       Some("ord-1"));
    }

    // ── to_object / round-trip ────────────────────────────────────────────

    #[test]
    fn to_object_round_trips_through_from_seq() {
        let state = order_state();
        let ts = TypedSeq::from_cell("Order", &state).unwrap();
        let obj = ts.to_object();

        // Reconstruct from the converted object.
        let ts2 = TypedSeq::from_seq(&obj).expect("round-trip must succeed");
        assert_eq!(ts2.shape, ts.shape);
        assert_eq!(ts2.rows,  ts.rows);
    }

    #[test]
    fn to_object_produces_seq_with_correct_row_count() {
        let state = order_state();
        let ts = TypedSeq::from_cell("Order", &state).unwrap();
        let obj = ts.to_object();
        assert_eq!(obj.as_seq().unwrap().len(), 3);
    }

    // ── get helper ────────────────────────────────────────────────────────

    #[test]
    fn get_returns_value_by_row_and_column() {
        let state = order_state();
        let ts = TypedSeq::from_cell("Order", &state).unwrap();
        assert_eq!(ts.get(0, "id"),       Some("ord-1"));
        assert_eq!(ts.get(1, "status"),   Some("Active"));
        assert_eq!(ts.get(2, "customer"), Some("gamma"));
    }

    #[test]
    fn get_returns_none_for_out_of_bounds_row() {
        let state = order_state();
        let ts = TypedSeq::from_cell("Order", &state).unwrap();
        assert_eq!(ts.get(99, "id"), None);
    }

    // ── from_seq ──────────────────────────────────────────────────────────

    #[test]
    fn from_seq_works_on_bare_object() {
        let rows = vec![
            fact_from_pairs(&[("role", "admin"), ("user", "alice")]),
            fact_from_pairs(&[("role", "viewer"), ("user", "bob")]),
        ];
        let obj = Object::Seq(rows.into());
        let ts = TypedSeq::from_seq(&obj).expect("must succeed");
        assert_eq!(ts.shape, vec!["role", "user"]);
        assert_eq!(ts.len(), 2);
    }

    #[test]
    fn from_seq_returns_none_for_atom() {
        assert!(TypedSeq::from_seq(&Object::atom("x")).is_none());
    }

    #[test]
    fn from_seq_returns_none_for_bottom() {
        assert!(TypedSeq::from_seq(&Object::Bottom).is_none());
    }
}
