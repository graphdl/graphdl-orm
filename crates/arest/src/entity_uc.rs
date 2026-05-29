// crates/arest/src/entity_uc.rs
//! In-memory entity-cell uniqueness enforcement (rework P0 spike).
//!
//! Proves alethic UC enforcement holds when runtime population lives in
//! `<Noun>:<id>` entity cells (3NF rows) rather than fact-type-keyed
//! cells. `no_std`-clean: `hashbrown` + `alloc` only — no `std`, no
//! `rusqlite` (must stay kernel/wasm-clean per docs/11-portability.md).
//!
//! Three UC regimes, per design spec §5/§9
//! (docs/superpowers/specs/2026-05-29-runtime-entity-cell-storage-design.md):
//!   * functional UC      -> structural: a single-valued entity-cell field
//!                           (a re-write is an update, never a conflict)
//!   * reference scheme   -> `entity_exists`: a namespace lookup
//!                           (replaces the raw scan at command.rs:958)
//!   * non-functional UC  -> `EntityUniquenessIndex`: a cross-entity index
//!                           (1:1-reverse, external/spanning, junction)
//!
//! All violations carry the same `uc:{name}` family + `alethic:true`
//! shape that `command.rs:445 uc_violation_from_conflict` produces, so
//! they plug into the existing apply-rejection path unchanged.

use alloc::{string::{String, ToString}, vec::Vec};
use hashbrown::HashMap;
use crate::ast::Object;
use crate::types::Violation;

/// Read a single-valued field from a 3NF entity row. The row is an
/// `Object::Map<field, Object::Atom>` exactly as
/// `augment_delta_with_entity_cells` (lib.rs:2375) builds it.
fn row_field<'a>(row: &'a Object, field: &str) -> Option<&'a str> {
    row.as_map()?.get(field).and_then(|v| v.as_atom())
}

/// Reference-scheme uniqueness as a namespace lookup: does an entity
/// cell `<Noun>:<id>` already exist in the entity store? Replaces the
/// raw full-population scan at `command.rs:958`.
pub fn entity_exists(store: &HashMap<String, Object>, noun: &str, id: &str) -> bool {
    store.contains_key(&format!("{}:{}", noun, id))
}

/// A non-functional uniqueness constraint enforced across entity rows:
/// the entity `table` must hold unique values across the `columns` set.
/// Covers 1:1-reverse, external/spanning, and junction-spanning UCs —
/// the ones NOT enforceable by a single-valued entity field.
#[derive(Clone, Debug)]
pub struct EntityUc {
    /// The `uc:{name}` family, e.g. `"Customer_has_APIKey"`.
    pub name: String,
    /// The snake-cased entity table, e.g. `"customer"`.
    pub table: String,
    /// The spanning column set, e.g. `["api_key"]`.
    pub columns: Vec<String>,
}

/// Per-snapshot index for one [`EntityUc`]: joined column key -> the id
/// of the entity that currently owns it. Built once per snapshot;
/// checked per candidate write.
pub struct EntityUniquenessIndex {
    seen: HashMap<String, String>,
}

impl EntityUniquenessIndex {
    /// Build from the current entity rows. Rows missing any UC column
    /// are skipped (a partially-keyed row cannot collide).
    pub fn build<'a>(
        uc: &EntityUc,
        rows: impl Iterator<Item = &'a Object>,
        id_field: &str,
    ) -> Self {
        let mut seen = HashMap::new();
        for r in rows {
            if let Some(key) = join_key(r, &uc.columns) {
                let id = row_field(r, id_field).unwrap_or("").to_string();
                seen.insert(key, id);
            }
        }
        EntityUniquenessIndex { seen }
    }

    /// Check a candidate row. Returns the `uc:{name}` [`Violation`]
    /// (`alethic:true`) when the candidate's key is already owned by a
    /// *different* entity; same-owner re-assertion is admissible.
    pub fn check(&self, uc: &EntityUc, candidate: &Object, id_field: &str) -> Option<Violation> {
        let key = join_key(candidate, &uc.columns)?;
        let cand_id = row_field(candidate, id_field).unwrap_or("");
        match self.seen.get(&key) {
            Some(owner) if owner != cand_id => Some(Violation {
                constraint_id: format!("uc:{}", uc.name),
                constraint_text: format!("Each {} is unique by {:?}", uc.table, uc.columns),
                detail: format!(
                    "Uniqueness violation: key '{}' in {} is owned by '{}', not '{}'",
                    key, uc.table, owner, cand_id),
                alethic: true,
            }),
            _ => None,
        }
    }
}

/// Join a row's UC-column values into a collision-safe key. Returns
/// `None` if any column is absent (the row isn't fully keyed). The
/// ASCII unit separator (`\u{1f}`) is not a legal value character, so
/// distinct column tuples cannot alias to the same joined key.
fn join_key(row: &Object, columns: &[String]) -> Option<String> {
    let mut parts: Vec<&str> = Vec::with_capacity(columns.len());
    for c in columns {
        parts.push(row_field(row, c)?);
    }
    Some(parts.join("\u{1f}"))
}

/// Runtime ↑FILE for one absorbed binary FT: project the absorbing
/// `<Noun>:<id>` entity rows back into elementary facts
/// `<<subjectRole, id>, <valueRole, value>>`. The runtime-regime
/// counterpart of `rmap::reconstitute_absorbed_ft` (which reads
/// metamodel registry cells) — the rmap.rs:918 follow-up. Presence-
/// driven: a fact is emitted only where the value field is present, so
/// `P = ⋃ₙ ↑FILE:Dₙ` (arest.tex eq:pop) is reproduced from the entity
/// rows without a serialized-blob round-trip.
pub fn reconstitute_ft_from_entity_rows<'a>(
    rows: impl Iterator<Item = &'a Object>,
    subject_role: &str,
    id_field: &str,
    value_role: &str,
    value_field: &str,
) -> Vec<Object> {
    let mut facts: Vec<Object> = Vec::new();
    for row in rows {
        if let (Some(id), Some(val)) =
            (row_field(row, id_field), row_field(row, value_field))
        {
            facts.push(crate::ast::fact_from_pairs(&[
                (subject_role, id),
                (value_role, val),
            ]));
        }
    }
    facts
}

/// How to reconstitute one absorbed binary FT from its entity table's
/// rows: the subject entity role + its id field, and the value role +
/// the row field the value lives under. In P2 these are derived from
/// `rmap`; here they are explicit so the census stays a pure unit.
#[derive(Clone, Debug)]
pub struct AbsorbedFtRoute<'a> {
    pub subject_role: &'a str,
    pub id_field: &'a str,
    pub value_role: &'a str,
    pub value_field: &'a str,
}

/// Runtime ↑FILE for a whole entity table: union the reconstituted
/// elementary facts of every absorbed FT over the table's rows — the
/// `P_after` side of the migration census (`P = ⋃ₙ ↑FILE:Dₙ`).
pub fn reconstitute_table_population(
    rows: &[Object],
    routes: &[AbsorbedFtRoute<'_>],
) -> Vec<Object> {
    let mut p: Vec<Object> = Vec::new();
    for r in routes {
        p.extend(reconstitute_ft_from_entity_rows(
            rows.iter(),
            r.subject_role,
            r.id_field,
            r.value_role,
            r.value_field,
        ));
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a 3NF entity row `Object::Map<field, Atom>`.
    fn row(pairs: &[(&str, &str)]) -> Object {
        let mut m: HashMap<String, Object> = HashMap::new();
        for &(k, v) in pairs {
            m.insert(k.to_string(), Object::atom(v));
        }
        Object::map(m)
    }

    fn apikey_uc() -> EntityUc {
        EntityUc {
            name: "Customer_has_APIKey".to_string(),
            table: "customer".to_string(),
            columns: vec!["api_key".to_string()],
        }
    }

    #[test]
    fn reference_scheme_uniqueness_is_a_namespace_lookup() {
        let mut store: HashMap<String, Object> = HashMap::new();
        store.insert(
            "Task:909".to_string(),
            row(&[("id", "909"), ("task_description", "fix the core")]),
        );
        // A second create of Task:909 must be rejected (entity exists)…
        assert!(entity_exists(&store, "Task", "909"));
        // …while a fresh id is admissible.
        assert!(!entity_exists(&store, "Task", "910"));
    }

    #[test]
    fn functional_uc_field_update_is_not_a_conflict() {
        // "Each Task has at most one Task Description" is functional.
        // Updating Task:909's description replaces the single-valued
        // field — never a uniqueness conflict (structural).
        let before = row(&[("id", "909"), ("task_description", "old")]);
        let after = row(&[("id", "909"), ("task_description", "new")]);
        assert_eq!(row_field(&before, "id"), row_field(&after, "id"));
        assert_ne!(
            row_field(&before, "task_description"),
            row_field(&after, "task_description")
        );
        // No index, no constraint, no violation: structural by construction.
    }

    #[test]
    fn one_to_one_reverse_uc_detects_cross_entity_duplicate() {
        // "Each APIKey belongs to at most one Customer" — a 1:1 reverse
        // UC; needs an index over api_key across all Customer rows.
        let uc = apikey_uc();
        let rows = [
            row(&[("id", "c1"), ("api_key", "K-AAA")]),
            row(&[("id", "c2"), ("api_key", "K-BBB")]),
        ];
        let idx = EntityUniquenessIndex::build(&uc, rows.iter(), "id");

        // c3 taking c1's key → alethic violation.
        let candidate = row(&[("id", "c3"), ("api_key", "K-AAA")]);
        let v = idx.check(&uc, &candidate, "id").expect("expected a UC violation");
        assert_eq!(v.constraint_id, "uc:Customer_has_APIKey");
        assert!(v.alethic);

        // A genuinely new key is admissible.
        let ok = row(&[("id", "c4"), ("api_key", "K-CCC")]);
        assert!(idx.check(&uc, &ok, "id").is_none());
    }

    #[test]
    fn reassertion_is_idempotent_and_external_uc_spans_columns() {
        // Re-assertion: c1 re-writing its own key is not a conflict.
        let uc = apikey_uc();
        let rows = [row(&[("id", "c1"), ("api_key", "K-AAA")])];
        let idx = EntityUniquenessIndex::build(&uc, rows.iter(), "id");
        let same = row(&[("id", "c1"), ("api_key", "K-AAA")]);
        assert!(idx.check(&uc, &same, "id").is_none());

        // External/spanning UC across two columns (account unique by
        // customer+provider).
        let acct = EntityUc {
            name: "Account_ref".to_string(),
            table: "account".to_string(),
            columns: vec!["customer_id".to_string(), "oauth_provider".to_string()],
        };
        let arows = [row(&[("id", "a1"), ("customer_id", "c1"), ("oauth_provider", "google")])];
        let aidx = EntityUniquenessIndex::build(&acct, arows.iter(), "id");
        let dup = row(&[("id", "a2"), ("customer_id", "c1"), ("oauth_provider", "google")]);
        assert!(aidx.check(&acct, &dup, "id").is_some());
        let ok = row(&[("id", "a3"), ("customer_id", "c1"), ("oauth_provider", "github")]);
        assert!(aidx.check(&acct, &ok, "id").is_none());
    }

    #[test]
    fn violation_matches_existing_uc_family_shape() {
        // Downstream (apply rejection, MCP) keys off the "uc:" prefix
        // and the alethic flag. Assert both.
        let uc = apikey_uc();
        let rows = [row(&[("id", "c1"), ("api_key", "K")])];
        let idx = EntityUniquenessIndex::build(&uc, rows.iter(), "id");
        let v = idx
            .check(&uc, &row(&[("id", "c2"), ("api_key", "K")]), "id")
            .unwrap();
        assert!(v.constraint_id.starts_with("uc:"));
        assert_eq!(v.constraint_id, "uc:Customer_has_APIKey");
        assert!(v.alethic, "UC violations are alethic — apply must reject (D'=D)");
        assert!(!v.detail.is_empty());
    }

    #[test]
    fn p0_acceptance_three_uc_regimes() {
        // 1) Reference-scheme: namespace lookup.
        let mut store: HashMap<String, Object> = HashMap::new();
        store.insert("Task:909".to_string(), row(&[("id", "909")]));
        assert!(entity_exists(&store, "Task", "909"));

        // 2) Functional: single-valued field update, never a conflict
        //    (no index participates — structural).

        // 3) Non-functional: index rejects a cross-entity duplicate.
        let uc = apikey_uc();
        let rows = [row(&[("id", "c1"), ("api_key", "K")])];
        let idx = EntityUniquenessIndex::build(&uc, rows.iter(), "id");
        assert!(idx
            .check(&uc, &row(&[("id", "c2"), ("api_key", "K")]), "id")
            .is_some());
    }

    #[test]
    fn runtime_uffile_reconstitutes_elementary_facts_from_entity_rows() {
        use crate::ast::binding;
        // Three Task entity rows; the third has no description.
        let rows = [
            row(&[("id", "909"), ("task_description", "fix the core")]),
            row(&[("id", "910"), ("task_description", "write P1")]),
            row(&[("id", "911")]), // presence-driven: no description → skipped
        ];
        let facts = reconstitute_ft_from_entity_rows(
            rows.iter(),
            "Task",
            "id",
            "Task Description",
            "task_description",
        );
        // ↑FILE emits one elementary fact per present value (eq:pop).
        assert_eq!(facts.len(), 2);
        assert_eq!(binding(&facts[0], "Task"), Some("909"));
        assert_eq!(binding(&facts[0], "Task Description"), Some("fix the core"));
        assert_eq!(binding(&facts[1], "Task"), Some("910"));
    }

    #[test]
    fn census_table_population_unions_all_absorbed_fts() {
        use crate::ast::binding;
        let rows = [
            row(&[("id", "909"), ("task_description", "fix core"), ("task_subject", "Core")]),
            row(&[("id", "910"), ("task_description", "write P1"), ("task_subject", "P1")]),
        ];
        let routes = [
            AbsorbedFtRoute { subject_role: "Task", id_field: "id", value_role: "Task Description", value_field: "task_description" },
            AbsorbedFtRoute { subject_role: "Task", id_field: "id", value_role: "Task Subject", value_field: "task_subject" },
        ];
        let p = reconstitute_table_population(&rows, &routes);
        // 2 rows × 2 absorbed FTs = 4 elementary facts (eq:pop union).
        assert_eq!(p.len(), 4);
        // Deterministic order: route order, then row order.
        assert_eq!(binding(&p[0], "Task"), Some("909"));
        assert_eq!(binding(&p[0], "Task Description"), Some("fix core"));
        assert_eq!(binding(&p[1], "Task Description"), Some("write P1"));
        assert_eq!(binding(&p[2], "Task"), Some("909"));
        assert_eq!(binding(&p[2], "Task Subject"), Some("Core"));
        assert_eq!(binding(&p[3], "Task Subject"), Some("P1"));
    }
}
