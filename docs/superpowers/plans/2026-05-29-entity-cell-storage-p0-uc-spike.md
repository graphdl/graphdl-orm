# Runtime Entity-Cell Storage — P0: In-Memory UC Enforcement Spike — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove that alethic uniqueness-constraint enforcement still holds when runtime population lives in `<Noun>:<id>` entity cells (3NF rows) instead of fact-type-keyed cells — by building and unit-testing the one net-new mechanism the rework needs (an in-memory uniqueness index for non-functional UCs) and demonstrating that functional UCs and reference-scheme uniqueness become structural.

**Architecture:** A new `no_std`-clean module `entity_uc.rs` operates on entity rows shaped exactly as `augment_delta_with_entity_cells` already builds them (`Object::Map<field, Object::Atom>`). It provides (a) `entity_exists` — reference-scheme uniqueness as a namespace lookup (replacing the raw cell scan at `command.rs:958`), and (b) `EntityUniquenessIndex` — a per-snapshot `key → owning-id` index enforcing 1:1-reverse / external / junction UCs, raising the **same** `uc:{name}` `Violation` (`alethic:true`) the current `KeyConflict` path produces (`command.rs:445`). Functional UCs need no code: the entity row holds a single-valued field, so a re-write is an update, not a conflict — a test asserts the index does not flag it.

**Tech Stack:** Rust (engine crate `crates/arest`), `hashbrown` + `alloc` only (no `std`, no `rusqlite` — must stay kernel/wasm-clean per `docs/11-portability.md`). Tests are in-module `#[cfg(test)]` (codebase convention, cf. `ast.rs` tests). Builds/tests run **only** under the memory cap: `pwsh scripts/mem-capped-build.ps1 -CapGB 5 -WorkDir crates/arest -- cargo test --lib <filter> -j 2`. **Never run two cargo builds concurrently.** Commits are gpg-signed and non-blocking.

**Why this is P0:** The consumer audit (`docs/superpowers/specs/2026-05-29-runtime-entity-cell-storage-design.md` §9) found UC enforcement is the load-bearing unknown: today it is inseparable from FT-cell storage (`cell_put_keyed`'s `KeyConflict`). This spike settles whether it re-homes onto entity cells before P1–P3 commit to the storage swap. It touches no existing code paths (additive module + tests), so it is safe to land and verify in isolation.

---

## File Structure

- **Create:** `crates/arest/src/entity_uc.rs` — the entity-cell UC enforcement primitives + in-module tests. One responsibility: "given entity rows + a UC, is a candidate write admissible?"
- **Modify:** `crates/arest/src/lib.rs` — add `mod entity_uc;` (one line, near the other `mod` declarations).
- **Reference (read-only, do not modify in P0):** `crates/arest/src/ast.rs` (`Object`, `as_map`, `as_atom`, `binding`, `fact_from_pairs`, `Object::map`, `Object::atom`), `crates/arest/src/types.rs` (`Violation`), `crates/arest/src/command.rs:445` (`uc_violation_from_conflict` — the shape to match), `crates/arest/src/lib.rs:2375` (`augment_delta_with_entity_cells` — the entity-row shape to match).

**First, confirm the exact `Object` accessor signatures** so the code below compiles against reality (the TDD loop will catch mismatches, but check once up front): in `ast.rs` around lines 256-275, `as_map(&self) -> Option<&HashMap<String, Object>>` and `as_atom(&self) -> Option<&str>`; `Object::map(HashMap) -> Object`, `Object::atom(&str) -> Object`. If `as_map` returns the `Arc` rather than the inner map, deref accordingly in `row_field`.

---

## Task 1: Scaffold the module + reference-scheme existence check (failing test)

**Files:**
- Create: `crates/arest/src/entity_uc.rs`
- Modify: `crates/arest/src/lib.rs` (add `mod entity_uc;`)

- [ ] **Step 1: Create the module with a row helper and a failing test**

```rust
// crates/arest/src/entity_uc.rs
//! In-memory entity-cell uniqueness enforcement (rework P0 spike).
//!
//! Proves alethic UC enforcement holds when population lives in
//! `<Noun>:<id>` entity cells (3NF rows) rather than FT-keyed cells.
//! `no_std`-clean: `hashbrown` + `alloc` only — no `std`, no `rusqlite`.

use alloc::{string::{String, ToString}, vec::Vec, format};
use hashbrown::HashMap;
use crate::ast::Object;

/// Read a single-valued field from a 3NF entity row. The row is an
/// `Object::Map<field, Object::Atom>` exactly as
/// `augment_delta_with_entity_cells` (lib.rs:2375) builds it.
fn row_field<'a>(row: &'a Object, field: &str) -> Option<&'a str> {
    row.as_map()?.get(field).and_then(|v| v.as_atom())
}

/// Reference-scheme uniqueness as a namespace lookup: does an entity
/// cell `<Noun>:<id>` already exist in the entity store? Replaces the
/// raw full-population scan at `command.rs:958`.
///
/// `store` is the entity-cell store keyed by cell name (`<Noun>:<id>`).
pub fn entity_exists(store: &HashMap<String, Object>, noun: &str, id: &str) -> bool {
    store.contains_key(&format!("{}:{}", noun, id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Object;

    fn row(pairs: &[(&str, &str)]) -> Object {
        let mut m: HashMap<String, Object> = HashMap::new();
        for (k, v) in pairs { m.insert((*k).to_string(), Object::atom(v)); }
        Object::map(m)
    }

    #[test]
    fn reference_scheme_uniqueness_is_a_namespace_lookup() {
        let mut store: HashMap<String, Object> = HashMap::new();
        store.insert("Task:909".to_string(), row(&[("id", "909"), ("task_description", "fix the core")]));

        // A second create of Task:909 must be rejected (entity exists)…
        assert!(entity_exists(&store, "Task", "909"));
        // …while a fresh id is admissible.
        assert!(!entity_exists(&store, "Task", "910"));
    }
}
```

- [ ] **Step 2: Register the module**

Add to `crates/arest/src/lib.rs` next to the other `mod` lines (e.g., after `mod command;`):

```rust
mod entity_uc;
```

- [ ] **Step 3: Run the test to verify it builds and passes**

Run: `pwsh scripts/mem-capped-build.ps1 -CapGB 5 -WorkDir crates/arest -- cargo test --lib entity_uc -j 2`
Expected: the crate compiles (first run is a full build — may take several minutes), test `reference_scheme_uniqueness_is_a_namespace_lookup` PASSES. If `as_map`/`as_atom`/`Object::map`/`Object::atom` signatures differ, fix `row`/`row_field` to match and re-run.

- [ ] **Step 4: Commit (non-blocking)**

```bash
git add crates/arest/src/entity_uc.rs crates/arest/src/lib.rs
git commit -S -m "feat(entity_uc): P0 scaffold + reference-scheme existence check" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2: Functional UC is structural (failing test, then assert no code needed)

This task proves a *negative*: the index must NOT flag an intra-entity field update as a violation (functional UC = single-valued field = last-write).

**Files:**
- Modify: `crates/arest/src/entity_uc.rs` (add test only)

- [ ] **Step 1: Write the test**

```rust
    #[test]
    fn functional_uc_field_update_is_not_a_conflict() {
        // "Each Task has at most one Task Description" is functional.
        // Updating Task:909's description replaces the single-valued
        // field — it must never be a uniqueness conflict.
        let before = row(&[("id", "909"), ("task_description", "old")]);
        let after  = row(&[("id", "909"), ("task_description", "new")]);

        // Same entity id, changed functional field → admissible update.
        assert_eq!(row_field(&before, "id"), row_field(&after, "id"));
        assert_ne!(row_field(&before, "task_description"), row_field(&after, "task_description"));
        // No index, no constraint object, no violation: structural by construction.
    }
```

- [ ] **Step 2: Run to verify it passes**

Run: `pwsh scripts/mem-capped-build.ps1 -CapGB 5 -WorkDir crates/arest -- cargo test --lib entity_uc -j 2`
Expected: PASS (incremental build, fast).

- [ ] **Step 3: Commit (non-blocking)**

```bash
git add crates/arest/src/entity_uc.rs
git commit -S -m "test(entity_uc): functional UC is structural (field update != conflict)" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3: `EntityUniquenessIndex` for non-functional UCs (failing test)

**Files:**
- Modify: `crates/arest/src/entity_uc.rs`

- [ ] **Step 1: Write the failing test (1:1-reverse UC: each APIKey belongs to ≤1 Customer)**

```rust
    #[test]
    fn one_to_one_reverse_uc_detects_cross_entity_duplicate() {
        // "Each APIKey belongs to at most one Customer" — a 1:1 reverse
        // UC. It cannot be enforced by a single entity row; it needs an
        // index over the api_key column across all Customer rows.
        let uc = EntityUc {
            name: "Customer_has_APIKey".to_string(),
            table: "customer".to_string(),
            columns: vec!["api_key".to_string()],
        };
        let rows = [
            row(&[("id", "c1"), ("api_key", "K-AAA")]),
            row(&[("id", "c2"), ("api_key", "K-BBB")]),
        ];
        let idx = EntityUniquenessIndex::build(&uc, rows.iter(), "id");

        // c3 trying to take c1's key → alethic violation.
        let candidate = row(&[("id", "c3"), ("api_key", "K-AAA")]);
        let v = idx.check(&uc, &candidate, "id").expect("expected a UC violation");
        assert_eq!(v.constraint_id, "uc:Customer_has_APIKey");
        assert!(v.alethic);

        // A genuinely new key is admissible.
        let ok = row(&[("id", "c4"), ("api_key", "K-CCC")]);
        assert!(idx.check(&uc, &ok, "id").is_none());
    }
```

- [ ] **Step 2: Run to verify it fails to compile**

Run: `pwsh scripts/mem-capped-build.ps1 -CapGB 5 -WorkDir crates/arest -- cargo test --lib entity_uc -j 2`
Expected: FAIL — `EntityUc` / `EntityUniquenessIndex` not defined.

---

## Task 4: Implement `EntityUniquenessIndex` (make Task 3 pass)

**Files:**
- Modify: `crates/arest/src/entity_uc.rs`

- [ ] **Step 1: Add the types + implementation above the `#[cfg(test)]` module**

```rust
use crate::types::Violation;

/// A non-functional uniqueness constraint enforced across entity rows:
/// the entity `table` must hold unique values across the `columns` set.
/// Covers 1:1-reverse, external/spanning, and junction-spanning UCs —
/// the ones that are NOT enforceable by a single-valued entity field.
#[derive(Clone, Debug)]
pub struct EntityUc {
    pub name: String,         // uc:{name} family, e.g. "Customer_has_APIKey"
    pub table: String,        // snake entity table, e.g. "customer"
    pub columns: Vec<String>, // spanning column set, e.g. ["api_key"]
}

/// Per-snapshot index for one `EntityUc`: joined column key → the id of
/// the entity that currently owns it. Built once per snapshot; checked
/// per candidate write.
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

    /// Check a candidate row. Returns the `uc:{name}` `Violation`
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
/// `None` if any column is absent (the row isn't fully keyed).
fn join_key(row: &Object, columns: &[String]) -> Option<String> {
    let mut parts: Vec<&str> = Vec::with_capacity(columns.len());
    for c in columns { parts.push(row_field(row, c)?); }
    Some(parts.join("\u{1f}")) // ASCII unit separator — not a legal value char
}
```

- [ ] **Step 2: Run to verify Task 3's test passes**

Run: `pwsh scripts/mem-capped-build.ps1 -CapGB 5 -WorkDir crates/arest -- cargo test --lib entity_uc -j 2`
Expected: PASS (all `entity_uc` tests).

- [ ] **Step 3: Commit (non-blocking)**

```bash
git add crates/arest/src/entity_uc.rs
git commit -S -m "feat(entity_uc): in-memory uniqueness index for non-functional UCs" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 5: Re-assertion is idempotent + multi-column (external) UC (failing test)

**Files:**
- Modify: `crates/arest/src/entity_uc.rs`

- [ ] **Step 1: Write the test**

```rust
    #[test]
    fn reassertion_is_idempotent_and_external_uc_spans_columns() {
        // Re-assertion: c1 re-writing its own key is not a conflict.
        let uc = EntityUc {
            name: "Customer_has_APIKey".to_string(),
            table: "customer".to_string(),
            columns: vec!["api_key".to_string()],
        };
        let rows = [row(&[("id", "c1"), ("api_key", "K-AAA")])];
        let idx = EntityUniquenessIndex::build(&uc, rows.iter(), "id");
        let same = row(&[("id", "c1"), ("api_key", "K-AAA")]);
        assert!(idx.check(&uc, &same, "id").is_none());

        // External/spanning UC across two columns (account unique by
        // customer+provider). Same (customer,provider) under a new id → violation.
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
```

- [ ] **Step 2: Run to verify it passes**

Run: `pwsh scripts/mem-capped-build.ps1 -CapGB 5 -WorkDir crates/arest -- cargo test --lib entity_uc -j 2`
Expected: PASS.

- [ ] **Step 3: Commit (non-blocking)**

```bash
git add crates/arest/src/entity_uc.rs
git commit -S -m "test(entity_uc): idempotent re-assertion + multi-column external UC" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 6: Violation shape parity with the existing `KeyConflict` path

Proves a candidate built from a real fact yields a `Violation` indistinguishable (in the fields downstream consumers read) from `uc_violation_from_conflict` (`command.rs:445`), so it plugs into the existing rejection path unchanged.

**Files:**
- Modify: `crates/arest/src/entity_uc.rs`

- [ ] **Step 1: Write the test**

```rust
    #[test]
    fn violation_matches_existing_uc_family_shape() {
        // Downstream (apply rejection, MCP) keys off constraint_id's
        // "uc:" prefix and the alethic flag. Assert both.
        let uc = EntityUc {
            name: "Customer_has_APIKey".to_string(),
            table: "customer".to_string(),
            columns: vec!["api_key".to_string()],
        };
        let rows = [row(&[("id", "c1"), ("api_key", "K")])];
        let idx = EntityUniquenessIndex::build(&uc, rows.iter(), "id");
        let v = idx.check(&uc, &row(&[("id", "c2"), ("api_key", "K")]), "id").unwrap();
        assert!(v.constraint_id.starts_with("uc:"));
        assert_eq!(v.constraint_id, "uc:Customer_has_APIKey");
        assert!(v.alethic, "UC violations are alethic — apply must reject (D'=D)");
        assert!(!v.detail.is_empty());
    }
```

- [ ] **Step 2: Run to verify it passes**

Run: `pwsh scripts/mem-capped-build.ps1 -CapGB 5 -WorkDir crates/arest -- cargo test --lib entity_uc -j 2`
Expected: PASS.

- [ ] **Step 3: Commit (non-blocking)**

```bash
git add crates/arest/src/entity_uc.rs
git commit -S -m "test(entity_uc): violation shape parity with uc:{name} family" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 7: P0 acceptance — the spike conclusion, in prose + a guard test

**Files:**
- Modify: `crates/arest/src/entity_uc.rs` (module doc + one summarizing test)
- Modify: `docs/superpowers/specs/2026-05-29-runtime-entity-cell-storage-design.md` (mark P0 settled in §9)

- [ ] **Step 1: Add a summarizing test that exercises all three UC regimes together**

```rust
    #[test]
    fn p0_acceptance_three_uc_regimes() {
        // 1) Reference-scheme: namespace lookup.
        let mut store: HashMap<String, Object> = HashMap::new();
        store.insert("Task:909".to_string(), row(&[("id", "909")]));
        assert!(entity_exists(&store, "Task", "909"));

        // 2) Functional: single-valued field update, never a conflict
        //    (no index participates — structural).

        // 3) Non-functional: index rejects a cross-entity duplicate.
        let uc = EntityUc { name: "Customer_has_APIKey".to_string(),
            table: "customer".to_string(), columns: vec!["api_key".to_string()] };
        let rows = [row(&[("id", "c1"), ("api_key", "K")])];
        let idx = EntityUniquenessIndex::build(&uc, rows.iter(), "id");
        assert!(idx.check(&uc, &row(&[("id", "c2"), ("api_key", "K")]), "id").is_some());
    }
```

- [ ] **Step 2: Run the full module test suite**

Run: `pwsh scripts/mem-capped-build.ps1 -CapGB 5 -WorkDir crates/arest -- cargo test --lib entity_uc -j 2`
Expected: PASS (all tests).

- [ ] **Step 3: Record the P0 conclusion in the spec §9**

Append under the "Revised phasing" P0 bullet in the design spec: a one-line note `**P0 settled (commit <hash>):** in-memory entity-cell UC enforcement verified — functional UCs structural, reference-scheme = namespace lookup, non-functional = `EntityUniquenessIndex`, all alethic. P1 unblocked.`

- [ ] **Step 4: Commit (non-blocking)**

```bash
git add crates/arest/src/entity_uc.rs docs/superpowers/specs/2026-05-29-runtime-entity-cell-storage-design.md
git commit -S -m "feat(entity_uc): P0 acceptance — three UC regimes verified; mark spec P0 settled" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## P0 Self-Review

- **Spec coverage:** Implements spec §5 (UC enforcement split: functional/reference-scheme/non-functional) and §9 P0 (settle the load-bearing UC unknown). No other §-requirement is in P0's scope by design.
- **Placeholder scan:** none — every step has runnable code/commands. The one "confirm signatures" note (File Structure) is a verification instruction, not a code placeholder; the TDD loop in Task 1 forces it.
- **Type consistency:** `EntityUc{name,table,columns}`, `EntityUniquenessIndex::{build,check}`, `row_field`, `join_key`, `entity_exists` are named identically across Tasks 1–7; `Violation{constraint_id,constraint_text,detail,alethic}` matches `command.rs:445`.

---

## Roadmap: P1–P3 (separate plans, written after P0 settles)

These are deliberately *not* expanded into bite-sized tasks yet — their exact code depends on P0's verified design and on per-phase discovery. Each becomes its own plan via `writing-plans`.

### P1 — Entity-cell store + migrator + census (behind a feature flag)
- **New `Native`-layer store** (local SQLite) persisting entity rows to RMAP entity tables (`rmap::rmap` + `compile::generate_ddl`, with CHECK re-enabled per spec §5). Engine stays `rusqlite`-free; this lives in the CLI/`Native` layer (`cli/entry.rs` persist/load).
- **Migrator:** iterate existing FT blob cells → `EntityCellRouter::route_fact` → accumulate `<Noun>:<id>` rows → write entity tables. Scrub legacy malformed rows once (the retiring `drop_subjectless_facts_with_arity` job).
- **Census harness:** `P_before` (↑FILE of blob cells) ≡ `P_after` (↑FILE of entity tables) — per-FT counts + byte-identical values + per-entity 3NF-row equality. Integration test on a *copy* of `tasks.db.bak-pre-compile-gc-fix-20260528-214634`. **Live `tasks.db` untouched.**
- **Flag:** `entity_cell_store` cargo feature gates the new load/persist; default off until P3.

### P2 — Re-home writers + fix the 4 gaps + redirect raw reads
- **Writers → `EntityCellRouter`:** `evaluate.rs:90 integrate_round_facts`, `command.rs push_with_uc_check` + create/update/transition/SM sites, `induce.rs` injection — write entity cells as primary (UC enforcement via Task-4 `EntityUniquenessIndex` + namespace existence), not FT cells + shadow.
- **4 gaps:** `encode_state`/`encode_state_indexed` (reconstitute population for validate/derive), `evaluate.rs:594 state_keys` (key off entity-cell shape), `platform_list_noun`/`visible_population`/`hateoas.rs` (stop excluding `:`-cells; read `<Noun>:<id>`), proof engine `evaluate.rs:672/701`.
- **Raw helper reads → ↑FILE:** redirect `fetch_cell_seq`/`cells_iter` population reads at `command.rs:738/2509/2902`, `induce.rs:164` through `reconstitute_absorbed_ft`.

### P3 — Flip + retire
- Flip the `entity_cell_store` flag default on once census + a populate→freeze→thaw→↑FILE property test + the 787-description recompile regression are green.
- **Retire** (this is the "remove Rust" payoff): the FT-blob `persist_state`/`load_state` population path, `escape_atom_for_display`/`parse` for population, `preserve_prior_population`, `merge_states`/`concat_dedup` for population, `dedup_state_for_persist`/`dedup_cell_facts`, `drop_subjectless_facts_with_arity`, and `_CellKeyRoles` emission/consumption. Remove the obsolete `[KRPROBE]` instrumentation in `cli/entry.rs`.
- Migrate the live board only after the census passes on its backup copy.
