// crates/arest/src/lib.rs
//
// AREST: Applicative REpresentational State Transfer
//
// SYSTEM:x = <o, D'>
// One function. Readings in, applications out.
// State = P (facts) + DEFS (named Func).

#![cfg_attr(feature = "no_std", no_std)]

// `alloc` is brought in unconditionally so `alloc::sync::Arc` /
// `alloc::boxed::Box` / `alloc::vec::Vec` resolve in both default
// (std) and `no_std` builds. Under std, `alloc` is part of the
// sysroot and re-exported through `std::*`; under `no_std` the same
// crate is the only source of heap-allocated types.
//
// `#[macro_use]` pulls the `vec!` and `format!` macros into the
// crate root so call sites can use them bare. Under std these are
// also in the prelude (via `std::vec` / `std::format`), so the
// macro_use import is a no-op there; under `no_std` it is the only
// way to get those macros without a per-file `use alloc::vec;`.
#[macro_use]
extern crate alloc;

/// Conditional diagnostic macro. Under std, forwards to `eprintln!`.
/// Under no_std, it's a no-op — kernel callers wire their own serial
/// sink via the `check` system verb instead of relying on stderr.
#[cfg(not(feature = "no_std"))]
#[macro_export]
macro_rules! diag {
    ($($arg:tt)*) => { eprintln!($($arg)*) }
}

#[cfg(feature = "no_std")]
#[macro_export]
macro_rules! diag {
    ($($arg:tt)*) => { }
}

pub mod sync;
// `sync` exports `Arc` (via `alloc::sync::Arc`) and spin-based
// `Mutex`/`RwLock`/`OnceLock` that work on both std and no_std builds
// — the cfg gate these imports used to carry was stale. Ungate so the
// type appears in scope for the `ast` module's Arc<[Object]> Seq and
// `Func::Native`'s Arc<dyn Fn>, even when the big std-only engine
// block below is elided.
use crate::sync::Arc;
#[cfg(not(feature = "no_std"))]
use crate::sync::Mutex;
#[cfg(not(feature = "no_std"))]
use crate::sync::OnceLock;
#[cfg(not(feature = "no_std"))]
use crate::sync::RwLock;
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

pub mod ast;
// types.rs uses serde Serialize/Deserialize — excluded from no_std build
#[cfg(not(feature = "no_std"))]
pub mod types;
pub mod freeze;
pub mod row_shape;

// Modules that depend on serde / serde_json / regex / hmac / std are
// excluded from the no_std (kernel) build. The kernel only needs
// `ast` (Object + Func + apply) and `freeze` (thaw from baked bytes).
#[cfg(not(feature = "no_std"))]
pub mod compile;
#[cfg(not(feature = "no_std"))]
pub mod evaluate;
#[cfg(not(feature = "no_std"))]
pub mod query;
#[cfg(not(feature = "no_std"))]
// induce.rs deleted — zero production callers, tests were self-referential.
#[cfg(not(feature = "no_std"))]
pub mod rmap;
#[cfg(not(feature = "no_std"))]
pub mod naming;
#[cfg(not(feature = "no_std"))]
// validate.rs deleted — zero production callers, tests were self-referential.
#[cfg(not(feature = "no_std"))]
pub mod conceptual_query;
#[cfg(not(feature = "no_std"))]
pub mod parse_forml2;
#[cfg(not(feature = "no_std"))]
pub mod parse_forml2_stage1;
#[cfg(not(feature = "no_std"))]
pub mod parse_forml2_stage2;
#[cfg(not(feature = "no_std"))]
// verbalize.rs deleted — zero production callers, tests were self-referential.
#[cfg(not(feature = "no_std"))]
pub mod command;
#[cfg(not(feature = "no_std"))]
pub mod crypto;
#[cfg(not(feature = "no_std"))]
pub mod generators;
#[cfg(not(feature = "no_std"))]
pub mod quota;
#[cfg(not(feature = "no_std"))]
pub mod scheduler;
#[cfg(not(feature = "no_std"))]
pub mod ring;
#[cfg(not(feature = "no_std"))]
pub mod declared_writes;
#[cfg(not(feature = "no_std"))]
pub mod check;

// Stress harness for compile_explicit_derivation (#296). Test-only; not
// shipped in any build.
#[cfg(all(test, not(feature = "no_std")))]
mod compile_explicit_derivation_tests;

#[cfg(feature = "wasm-lower")]
pub mod wasm_lower;

#[cfg(feature = "cloudflare")]
pub mod cloudflare;

// The DOMAINS / CompiledState / system_impl machinery requires serde,
// serde_json, regex, and std — excluded from the no_std kernel build.
// The kernel uses only `ast` and `freeze` directly.
//
// Everything from `struct CompiledState` through the end of this file
// is gated behind `not(feature = "no_std")` so the kernel build links
// only the ast + freeze surface. The repetition of the cfg attribute
// is deliberate: top-level-item gating keeps error messages in the
// same file as the items themselves, and avoids shuffling 1500 lines
// into a sub-module just to place one shared cfg.

#[cfg(not(feature = "no_std"))]
/// D: the unified state — population cells + def cells, split into
/// per-cell `Arc<RwLock<Object>>`. Backus Sec. 14.3 state-as-cells,
/// but with each cell independently lockable so disjoint writers
/// don't serialize through a single tenant-wide lock.
///
/// Access patterns:
///   - Reads:  `snapshot_d(&self)` builds a consistent Object::Map
///             view by acquiring every cell's read lock briefly.
///   - Whole-state writes (compile, rollback): `replace_d(&mut self,
///             new_d)` rebuilds the cells map. Requires the outer
///             `RwLock<CompiledState>::write()` guard.
///   - Targeted writes (create/update/transition): `try_commit_diff(
///             &self, snapshot, new_d)` acquires per-cell write
///             locks for only the cells that changed. CAS-checks
///             each against the snapshot; returns an error if any
///             cell changed meanwhile (caller retries) or if new
///             cells must be added (caller escalates to `write()`).
///             Needs only the outer `read()` guard, so two disjoint-
///             cell writers run in parallel.
///
/// `snapshots` holds named captures of `d` taken via `system(h,
/// "snapshot", "")` and restorable via `system(h, "rollback", id)`.
/// Cheap in memory because cells share `Arc` storage — a snapshot
/// is one map insert plus an Arc ref bump per cell.
struct CompiledState {
    cells: hashbrown::HashMap<String, Arc<RwLock<ast::Object>>>,
    snapshots: hashbrown::HashMap<String, ast::Object>,
    /// Sec-4: per-tenant secret that HMAC-signs snapshot ids so
    /// `system(h, "rollback", …)` cannot be driven by brute-forcing
    /// raw labels. Filled from a boot-time nonce in `new()`; never
    /// leaves the process. See the `snapshot:` / `rollback:` branches
    /// of `system_impl` for the sign/verify flow.
    snapshot_secret: [u8; 32],
}

/// Outcome of a targeted-write attempt via `try_commit_diff`.
#[cfg(not(feature = "no_std"))]
enum CommitOutcome {
    /// All cell-level CAS checks passed; the writes have been applied.
    Committed,
    /// One or more cells changed between snapshot and commit. The
    /// caller should re-snapshot, re-run apply(), and retry.
    StaleSnapshot,
    /// The new state introduces cells that don't exist yet, or
    /// removes existing cells. The cells map itself must be mutated,
    /// which requires the outer write guard. Caller should escalate.
    StructuralChange,
}

/// Sec-4: per-tenant 32-byte secret used to HMAC-sign snapshot ids.
///
/// `getrandom` is not a dep of this crate, so we tap `std::collections::
/// hash_map::RandomState` which std seeds from OS entropy
/// (getrandom/BCryptGenRandom) for hashmap iteration-order
/// randomization. Four fresh `RandomState::new()` calls cover four
/// 8-byte chunks of the secret. A SHA-256 final mix absorbs a
/// `SystemTime` nanosecond stamp, the process id, and a
/// per-tenant monotonic counter so two tenants instantiated in the
/// same thread at the same nanosecond still land on distinct secrets.
///
/// This is a "boot-time nonce" per the Sec-4 handoff: not a drop-in
/// replacement for a CSPRNG, but enough entropy (>> 128 bits under
/// any sane std platform) that a caller who can only reach `system`
/// cannot feasibly forge a valid tag for any raw id.
#[cfg(not(feature = "no_std"))]
fn boot_time_snapshot_secret() -> [u8; 32] {
    use core::sync::atomic::{AtomicU64, Ordering};
    use sha2::{Digest, Sha256};
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};

    let mut h = Sha256::new();
    // Four OS-seeded RandomState draws, each hashed with a distinct
    // salt so we extract 4 × 64 bits of output dependent on the
    // OS-seeded keys. std uses getrandom / BCryptGenRandom under the
    // hood to seed these keys.
    for salt in 0u8..4 {
        let rs = RandomState::new();
        let mut hh = rs.build_hasher();
        hh.write(&[0xA5, salt, 0x5A]);
        h.update(hh.finish().to_le_bytes());
    }
    // SystemTime nanos: defense in depth against platforms where
    // RandomState might be weak — the absolute boot instant is not
    // predictable to an outside attacker.
    let now_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u128)
        .unwrap_or(0);
    h.update(now_nanos.to_le_bytes());
    h.update((std::process::id() as u64).to_le_bytes());
    // Monotonic counter differentiates tenants created in the same
    // nanosecond within the same process.
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    h.update(COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());

    let digest = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

#[cfg(not(feature = "no_std"))]
impl CompiledState {
    fn new(initial_d: ast::Object) -> Self {
        let mut s = Self {
            cells: hashbrown::HashMap::new(),
            snapshots: hashbrown::HashMap::new(),
            snapshot_secret: boot_time_snapshot_secret(),
        };
        s.replace_d(initial_d);
        s
    }

    /// Assemble an `Object::Map` view of the full state. Each cell's
    /// read lock is held briefly for the clone; readers don't block
    /// each other, but a writer on that cell will block the snapshot
    /// momentarily.
    fn snapshot_d(&self) -> ast::Object {
        let mut map = hashbrown::HashMap::with_capacity(self.cells.len());
        for (name, lock) in &self.cells {
            map.insert(name.clone(), lock.read().clone());
        }
        ast::Object::Map(map)
    }

    /// Wholesale rebuild the cell map from a new D. Reuses existing
    /// cell locks where possible (so concurrent readers still see a
    /// live `Arc<RwLock>` rather than a freed one), then prunes any
    /// cells absent from the new state.
    fn replace_d(&mut self, new_d: ast::Object) {
        let new_map: hashbrown::HashMap<String, ast::Object> = match new_d {
            ast::Object::Map(m) => m,
            ast::Object::Seq(seq) => {
                // CELL-triple representation: <<CELL, name, contents>, …>.
                // Fall through to an empty map if the shape doesn't match.
                let mut m = hashbrown::HashMap::new();
                for cell in seq.iter() {
                    if let Some(items) = cell.as_seq() {
                        if items.len() == 3 {
                            if let (Some(_), Some(name)) = (
                                items[0].as_atom(),
                                items[1].as_atom(),
                            ) {
                                m.insert(name.to_string(), items[2].clone());
                            }
                        }
                    }
                }
                m
            }
            ast::Object::Bottom => hashbrown::HashMap::new(),
            other => {
                // Unexpected shape — store the whole thing under a
                // sentinel cell so we don't silently drop it.
                let mut m = hashbrown::HashMap::new();
                m.insert("__unshaped__".to_string(), other);
                m
            }
        };
        // Reuse existing locks where possible; replace contents under
        // the per-cell write lock.
        let mut next_cells: hashbrown::HashMap<String, Arc<RwLock<ast::Object>>> =
            hashbrown::HashMap::with_capacity(new_map.len());
        for (name, value) in new_map {
            match self.cells.remove(&name) {
                Some(existing) => {
                    *existing.write() = value;
                    next_cells.insert(name, existing);
                }
                None => {
                    next_cells.insert(name, Arc::new(RwLock::new(value)));
                }
            }
        }
        // Any cell still in self.cells was removed by the new state;
        // dropped implicitly.
        self.cells = next_cells;
    }

    /// Targeted commit: write only the cells whose contents differ
    /// between `snapshot` (what apply() saw) and `new_d` (what apply()
    /// returned). Each changed cell is CAS-checked against the
    /// snapshot value before writing to detect stale snapshots.
    ///
    /// Requires only `&self` because the cells-map structure isn't
    /// mutated — only per-cell contents. Callers should therefore
    /// hold `RwLock<CompiledState>::read()`, which lets concurrent
    /// writers to disjoint cells proceed without contending on the
    /// outer lock.
    ///
    /// Returns `Committed` on success, `StaleSnapshot` when another
    /// writer modified one of the target cells between snapshot and
    /// commit (caller should retry), or `StructuralChange` when new
    /// cells must be introduced or existing cells removed (caller
    /// must escalate to `write()` and use `replace_d`).
    fn try_commit_diff(&self, snapshot: &ast::Object, new_d: &ast::Object) -> CommitOutcome {
        let snap_map = match snapshot.as_map() {
            Some(m) => m,
            None => return CommitOutcome::StructuralChange,
        };
        let new_map = match new_d.as_map() {
            Some(m) => m,
            None => return CommitOutcome::StructuralChange,
        };
        // Detect structural change: added or removed cells require
        // the outer write lock to mutate the cells map.
        for key in new_map.keys() {
            if !self.cells.contains_key(key) {
                return CommitOutcome::StructuralChange;
            }
        }
        for key in self.cells.keys() {
            if !new_map.contains_key(key) {
                return CommitOutcome::StructuralChange;
            }
        }
        // Collect changed cells.
        let mut changed: Vec<&String> = new_map
            .iter()
            .filter(|(k, v)| snap_map.get(*k) != Some(*v))
            .map(|(k, _)| k)
            .collect();
        if changed.is_empty() {
            return CommitOutcome::Committed; // no-op
        }
        // Sort for deterministic lock acquisition (deadlock avoidance
        // between concurrent writers with overlapping cell sets).
        changed.sort();
        // Acquire write locks in order.
        let mut guards: Vec<(&String, crate::sync::RwLockWriteGuard<'_, ast::Object>)> =
            Vec::with_capacity(changed.len());
        for key in changed {
            let lock = self.cells.get(key).expect("membership was checked above");
            let guard = lock.write();
            guards.push((key, guard));
        }
        // CAS: every changed cell's current contents must still match
        // the snapshot; otherwise someone committed under us.
        for (key, guard) in &guards {
            let expected = snap_map.get(*key);
            if Some(&**guard) != expected {
                return CommitOutcome::StaleSnapshot;
            }
        }
        // Apply the writes under the already-held guards.
        for (key, guard) in guards.iter_mut() {
            let new_value = new_map.get(*key).expect("membership was checked above").clone();
            **guard = new_value;
        }
        CommitOutcome::Committed
    }

    /// Declared-writes fast path (#186). Like `try_commit_diff` but
    /// only inspects the cells named in `targets` instead of diffing
    /// every cell in the state. O(|targets|) instead of O(|all_cells|).
    ///
    /// Returns `StructuralChange` if any target cell doesn't exist in
    /// the current state (rare — means the noun was never compiled).
    /// Returns `StaleSnapshot` if a concurrent writer modified a
    /// target cell since the snapshot. Returns `Committed` on success.
    #[allow(dead_code)]
    fn try_commit_declared(
        &self,
        snapshot: &ast::Object,
        new_d: &ast::Object,
        targets: &[&str],
    ) -> CommitOutcome {
        let snap_map = match snapshot.as_map() {
            Some(m) => m,
            None => return CommitOutcome::StructuralChange,
        };
        let new_map = match new_d.as_map() {
            Some(m) => m,
            None => return CommitOutcome::StructuralChange,
        };
        let mut changed: Vec<&str> = targets.iter().copied()
            .filter(|k| {
                let old = snap_map.get(*k);
                let new = new_map.get(*k);
                old != new
            })
            .collect();
        if changed.is_empty() {
            return CommitOutcome::Committed;
        }
        changed.sort();
        let mut guards: Vec<(&str, crate::sync::RwLockWriteGuard<'_, ast::Object>)> =
            Vec::with_capacity(changed.len());
        for key in &changed {
            match self.cells.get(*key) {
                Some(lock) => guards.push((*key, lock.write())),
                None => return CommitOutcome::StructuralChange,
            }
        }
        for (key, guard) in &guards {
            let expected = snap_map.get(*key);
            if Some(&**guard) != expected {
                return CommitOutcome::StaleSnapshot;
            }
        }
        for (key, guard) in guards.iter_mut() {
            if let Some(new_value) = new_map.get(*key) {
                **guard = new_value.clone();
            }
        }
        CommitOutcome::Committed
    }

    /// Return all cell names related to `noun`. Scans `self.cells`
    /// for names that equal the noun, start with `"<noun>_"`, or
    /// contain `"_<noun>_"` / `"_<noun>"` (handles RMAP-derived FT
    /// cells like `Order_has_total`, `Order_has_Amount`). `audit_log`
    /// is always included.
    #[allow(dead_code)]
    fn cells_for_noun(&self, noun: &str) -> Vec<String> {
        let prefix = format!("{}_", noun);
        let infix  = format!("_{}_", noun);
        let suffix = format!("_{}", noun);
        let mut targets: Vec<String> = self
            .cells
            .keys()
            .filter(|k| {
                *k == noun
                    || k.starts_with(&prefix)
                    || k.contains(&infix)
                    || k.ends_with(&suffix)
            })
            .cloned()
            .collect();
        // audit_log is always a write target for system verbs.
        if !targets.iter().any(|t| t == "audit_log") {
            targets.push("audit_log".to_string());
        }
        targets
    }
}

// The per-handle process table:
//
// Outer Mutex protects slot allocation/recycling (Vec mutations).
// Inner RwLock<CompiledState> protects per-tenant state, held only
// for the duration of one operation. Two tenants run concurrently —
// the outer lock is held only for slot lookup, then dropped; the
// inner lock is per-Arc, so different tenants don't contend.
//
// This realises Cell Isolation (Definition 2) at the per-tenant
// granularity. Per-cell concurrency within a tenant is a follow-up
// that needs apply() to acquire cell-level locks just-in-time.
#[cfg(not(feature = "no_std"))]
static DOMAINS: OnceLock<Mutex<Vec<Option<Arc<RwLock<CompiledState>>>>>> = OnceLock::new();
#[cfg(not(feature = "no_std"))]
fn ds() -> &'static Mutex<Vec<Option<Arc<RwLock<CompiledState>>>>> {
    DOMAINS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Look up a slot's tenant lock by handle. Returns None for invalid
/// handles or freed slots. The outer Vec mutex is held only for the
/// duration of the lookup and Arc clone, then released.
#[cfg(not(feature = "no_std"))]
fn tenant_lock(handle: u32) -> Option<Arc<RwLock<CompiledState>>> {
    let s = ds().lock();
    s.get(handle as usize).and_then(|x| x.as_ref()).map(Arc::clone)
}

#[cfg(not(feature = "no_std"))]
#[allow(dead_code)] // used by tests and the cloudflare feature
fn allocate(state: ast::Object, defs: Vec<(String, ast::Func)>) -> u32 {
    let d = ast::defs_to_state(&defs, &state);
    let mut s = ds().lock();
    let h = s.iter().position(|x| x.is_none()).unwrap_or_else(|| { s.push(None); s.len() - 1 });
    s[h] = Some(Arc::new(RwLock::new(CompiledState::new(d))));
    h as u32
}

// ── SYSTEM is the only function ─────────────────────────────────────

/// Bundled metamodel readings. Compiled into the binary at build time.
/// Loaded by `create_impl` so every fresh domain starts with the
/// self-describing metamodel available. Use `create_bare_impl` to skip
/// the auto-load when experimenting with a replacement core.
///
/// Load order matters: core defines the base object types (Noun, Fact
/// Schema, Role, Constraint) that every later reading references.
pub const METAMODEL_READINGS: &[(&str, &str)] = &[
    ("core",          include_str!("../../../readings/core.md")),
    ("state",         include_str!("../../../readings/state.md")),
    ("instances",     include_str!("../../../readings/instances.md")),
    ("outcomes",      include_str!("../../../readings/outcomes.md")),
    ("validation",    include_str!("../../../readings/validation.md")),
    ("evolution",     include_str!("../../../readings/evolution.md")),
    ("organizations", include_str!("../../../readings/organizations.md")),
    ("agents",        include_str!("../../../readings/agents.md")),
    ("ui",            include_str!("../../../readings/ui.md")),
];

/// The bundled metamodel concatenated into a single corpus, with
/// blank-line separators between files. Used to preload the checker
/// so a user app corpus doesn't flag metamodel-declared nouns
/// (Noun, FactType, Domain, App, …) as undeclared. Deterministic —
/// the fold is over the `METAMODEL_READINGS` table in declaration
/// order, matching `create_impl`'s load order.
#[cfg(not(feature = "no_std"))]
pub fn metamodel_corpus() -> String {
    METAMODEL_READINGS.iter().fold(String::new(), |mut acc, (_, text)| {
        acc.push_str(text);
        acc.push_str("\n\n");
        acc
    })
}

/// Check a user readings corpus with the bundled metamodel preloaded.
///
/// The metamodel is parsed as context so references to metamodel nouns
/// resolve, but diagnostics that originate purely from the metamodel
/// text are filtered out — only diagnostics whose `reading` text is
/// absent from the metamodel survive. This is the default mode for
/// check-cli; use `check::check_readings` directly when validating
/// the metamodel itself or a replacement core.
#[cfg(not(feature = "no_std"))]
pub fn check_readings_with_metamodel(user_text: &str) -> Vec<check::ReadingDiagnostic> {
    let metamodel = metamodel_corpus();
    let metamodel_only: std::collections::HashSet<String> = check::check_readings(&metamodel)
        .into_iter().map(|d| d.reading).collect();
    let combined = format!("{metamodel}\n\n{user_text}");
    check::check_readings(&combined)
        .into_iter()
        .filter(|d| !metamodel_only.contains(&d.reading))
        .collect()
}

#[cfg(test)]
mod check_metamodel_tests {
    use super::*;

    /// The raw checker on a user corpus that references metamodel
    /// nouns (Domain, App, Organization) should flood with undeclared
    /// / unresolved diagnostics. The with-metamodel variant should
    /// emit strictly fewer diagnostics on the same text, because the
    /// baseline resolves the references.
    #[test]
    fn metamodel_preload_reduces_user_corpus_diagnostics() {
        // A user snippet that leans on metamodel-declared nouns.
        let user = "\
# Support domain

Ticket(.id) is an entity type.
Ticket belongs to Organization.
  Each Ticket belongs to exactly one Organization.
Ticket is opened by User.
  Each Ticket is opened by exactly one User.
App 'support' navigates Domain.
";

        let bare = check::check_readings(user);
        let with_meta = check_readings_with_metamodel(user);

        assert!(with_meta.len() <= bare.len(),
            "preloading metamodel must not add diagnostics: bare={} with_meta={}",
            bare.len(), with_meta.len());
    }

    /// Diagnostics that originate solely from the metamodel text
    /// must not surface in the user-facing output. We approximate by
    /// checking that no diagnostic from the with-metamodel result
    /// matches a diagnostic produced by checking the metamodel in
    /// isolation.
    #[test]
    fn metamodel_only_diagnostics_are_filtered_out() {
        let metamodel_only = check::check_readings(&metamodel_corpus());
        let metamodel_only_readings: std::collections::HashSet<String> =
            metamodel_only.iter().map(|d| d.reading.clone()).collect();

        // Empty user corpus — every surviving diagnostic must have come
        // from somewhere outside the metamodel. None should remain.
        let diags = check_readings_with_metamodel("");
        for d in &diags {
            assert!(!metamodel_only_readings.contains(&d.reading),
                "metamodel-only diagnostic leaked through: {:?}", d.reading);
        }
    }
}

/// create_bare: allocate empty D with ONLY the platform primitives
/// registered in DEFS. Use this when testing a new core or rebuilding
/// the metamodel from scratch. Most apps should use `create_impl`.
#[cfg(not(feature = "no_std"))]
#[allow(dead_code)] // used by tests and the cloudflare feature
fn create_bare_impl() -> u32 {
    let state = ast::Object::phi();
    let defs = vec![
        ("compile".to_string(), ast::Func::Platform("compile".to_string())),
        ("apply".to_string(), ast::Func::Platform("apply_command".to_string())),
        ("verify_signature".to_string(), ast::Func::Platform("verify_signature".to_string())),
        ("audit".to_string(), ast::Func::Platform("audit".to_string())),
    ];
    allocate(state, defs)
}

/// create: allocate D with platform primitives AND the bundled metamodel
/// readings (core, state, instances, outcomes, validation, evolution,
/// organizations, agents, ui). One call yields a fully self-describing
/// engine ready to ingest user domain readings via `system(h, "compile", ...)`.
///
/// Use `create_bare_impl` to opt out when experimenting with a new core.
/// Cached metamodel state — PARSED cells + platform primitives only.
///
/// We deliberately skip `compile_to_defs_state` at cache build time because
/// `platform_compile` already runs it on every user compile, taking the
/// metamodel cells as context. Pre-compiling would be wasted work and slows
/// down `create_impl` by seconds. The expensive per-def construction (CWA
/// negation, per-constraint validate funcs, query/schema/resolve defs)
/// happens lazily on first user compile.
///
/// What IS in the cache:
///   - Metamodel Noun cell (self-describing types)
///   - Metamodel Fact Type cell
///   - Metamodel Role cell
///   - Metamodel Constraint cell
///   - 3 platform primitive defs (compile, apply, verify_signature)
///
/// Bootstrap mode (#23 guard bypass) wraps the parse fold.
#[cfg(not(feature = "no_std"))]
static METAMODEL_STATE: OnceLock<ast::Object> = OnceLock::new();

#[cfg(not(feature = "no_std"))]
fn metamodel_state() -> &'static ast::Object {
    METAMODEL_STATE.get_or_init(|| {
        struct BootstrapGuard;
        impl BootstrapGuard {
            fn enter() -> Self {
                parse_forml2::set_bootstrap_mode(true);
                BootstrapGuard
            }
        }
        impl Drop for BootstrapGuard {
            fn drop(&mut self) { parse_forml2::set_bootstrap_mode(false); }
        }
        let _guard = BootstrapGuard::enter();

        // Fold all 9 readings into a single merged state (parser only).
        let merged = METAMODEL_READINGS.iter().fold(ast::Object::phi(), |acc, (name, text)| {
            let parsed = parse_forml2::parse_to_state_from(text, &acc)
                .unwrap_or_else(|e| panic!("metamodel parse failed at readings/{}.md: {}", name, e));
            ast::merge_states(&acc, &parsed)
        });

        // Compile the metamodel once and bake the full def set into the
        // cached state. With `Object::Seq(Arc<[Object]>)`, cloning this
        // fat state on every `create_impl` is a ref-count bump per cell
        // instead of a deep Object copy — the cost that blocked the
        // previous baked-defs attempt is gone.
        //
        // Fresh handles now start with all metamodel constraint /
        // derivation / per-noun-validate defs already compiled; the
        // first `compile` command on a new handle incurs zero
        // metamodel re-compile cost. User readings still trigger a
        // full recompile when added (future optimization: splitting
        // the compile pipeline so the metamodel pass is a no-op when
        // the cached defs are already present).
        let mut defs = crate::compile::compile_to_defs_state(&merged);
        defs.extend([
            ("compile".to_string(), ast::Func::Platform("compile".to_string())),
            ("apply".to_string(), ast::Func::Platform("apply_command".to_string())),
            ("verify_signature".to_string(), ast::Func::Platform("verify_signature".to_string())),
            ("audit".to_string(), ast::Func::Platform("audit".to_string())),
        ]);
        ast::defs_to_state(&defs, &merged)
    })
}

#[cfg(not(feature = "no_std"))]
fn create_impl() -> u32 {
    // Clone the cached metamodel state into a fresh handle. First call
    // builds the cache (parses 9 metamodel readings + runs the full
    // compile pipeline to bake every constraint/derivation/per-noun-
    // validate def into the state); subsequent calls are just a handle
    // allocation + Object clone.
    //
    // The clone is cheap because Object::Seq is Arc<[Object]> — each
    // cell clone is a ref-count bump, not a deep copy. Before the Arc
    // refactor, an earlier attempt at baking defs into this cache was
    // slower net because the ~MB state paid a Vec deep-copy per handle
    // create. With Arc-sharing that tax is gone and the baked-defs
    // approach lands naturally: new handles start with zero metamodel
    // compile cost.
    let d = metamodel_state().clone();
    let mut s = ds().lock();
    let h = s.iter().position(|x| x.is_none()).unwrap_or_else(|| { s.push(None); s.len() - 1 });
    s[h] = Some(Arc::new(RwLock::new(CompiledState::new(d))));
    h as u32
}

/// Legacy: parse_and_compile as create + compile for each readings pair.
#[cfg(not(feature = "no_std"))]
fn parse_and_compile_impl(readings: Vec<(String, String)>) -> Result<u32, String> {
    let h = create_impl();
    readings.iter().try_fold(h, |h, (_name, text)| {
        let result = system_impl(h, "compile", text);
        if result.starts_with("⊥") { Err(result) } else { Ok(h) }
    })
}

#[cfg(not(feature = "no_std"))]
fn release_impl(handle: u32) {
    let mut s = ds().lock();
    s.get_mut(handle as usize).into_iter().for_each(|slot| *slot = None);
}

/// Classify an op as read-only by key prefix. Read-only ops take the
/// per-tenant RwLock in shared (read) mode, so two concurrent `list:X`
/// or `debug` calls on the same handle don't block each other.
///
/// Conservative list — when in doubt, a key falls through to the write
/// path, which is still correct (just serializes). Extending this list
/// is the right way to unlock more per-tenant concurrency.
#[cfg(not(feature = "no_std"))]
fn is_read_only_op(key: &str) -> bool {
    matches!(
        key,
        "debug" | "audit" | "verify_signature" | "snapshots"
    )
    || key.starts_with("list:")
    || key.starts_with("get:")
    || key.starts_with("query:")
    || key.starts_with("explain:")
}

/// SYSTEM:x = ⟨o, D'⟩. Pure ρ-dispatch + state transition.
///
/// The FPGA core: look up key in D via ρ, beta-reduce, update state.
/// No match arms. No if-branches. Every operation is a def in D.
///
/// Concurrency:
///   - Outer process-table mutex: held briefly to clone the per-tenant
///     Arc<RwLock<CompiledState>>. Two tenants run concurrently.
///   - Per-tenant RwLock: read-only ops take `read()` (shared);
///     write-path ops take `write()` (exclusive). Licenses Definition 2
///     at the tenant granularity — parallel queries on a handle don't
///     contend with each other, only with writers. Full per-cell
///     concurrency (parallel disjoint writes within one handle) is a
///     follow-up; it needs apply() to acquire cell locks just-in-time.
#[cfg(not(feature = "no_std"))]
fn system_impl(handle: u32, key: &str, input: &str) -> String {
    let tenant = match tenant_lock(handle) {
        Some(t) => t,
        None => return "⊥".into(),
    };

    // ── CompiledState-level intercepts ──────────────────────────────
    //
    // `snapshot` and `rollback` mutate the tenant's snapshot map or
    // replace `d`; they need a write lock. `snapshots` only reads the
    // map and can share with concurrent readers.
    //
    //   system(h, "snapshot", "")      → <snap-id>                (fresh id)
    //   system(h, "snapshot", "label") → label                    (caller-named)
    //   system(h, "rollback", "id")    → id on success, ⊥ on miss
    //   system(h, "snapshots", "")     → <id₁, id₂, ...> FFP seq
    // ── ↓DEFS FFI: runtime registers a Platform function (#305) ─────
    //
    //   system(h, "register:<name>", "") → <name> on success, ⊥ on failure
    //
    // Pushes Func::Platform(<name>) into DEFS and records <name> in
    // `runtime_registered_names`. This is the FFI form of
    // ast::register_runtime_fn — it gives hosts (JS, wasm, browser) a
    // way to mark which Platform primitives they own so Citation
    // provenance can cite them with Authority Type 'Runtime-Function'.
    //
    // Input is currently empty. Future revisions may accept a
    // serialized Func body (freeze-encoded, same scheme as the
    // thaw FFI) to register composable FFP bodies instead of
    // Platform stubs.
    if let Some(name) = key.strip_prefix("register:") {
        if name.is_empty() {
            return "⊥".into();
        }
        // Determine the body: empty input → Func::Platform(name) stub
        // (host owns dispatch elsewhere). Non-empty input → hex-encoded
        // freeze image of a Func-encoded Object, thawed and
        // metacomposed. Malformed hex / bad freeze → ⊥.
        let body = if input.is_empty() {
            ast::Func::Platform(name.to_string())
        } else {
            let nibble = |b: u8| -> Option<u8> {
                match b {
                    b'0'..=b'9' => Some(b - b'0'),
                    b'a'..=b'f' => Some(b - b'a' + 10),
                    b'A'..=b'F' => Some(b - b'A' + 10),
                    _ => None,
                }
            };
            let clean: String = input.chars().filter(|c| !c.is_whitespace()).collect();
            if clean.len() % 2 != 0 {
                return "⊥".into();
            }
            let bs = clean.as_bytes();
            let mut bytes: Vec<u8> = Vec::with_capacity(clean.len() / 2);
            let mut i = 0;
            while i + 1 < bs.len() {
                match (nibble(bs[i]), nibble(bs[i + 1])) {
                    (Some(h), Some(l)) => bytes.push((h << 4) | l),
                    _ => return "⊥".into(),
                }
                i += 2;
            }
            let obj = match crate::freeze::thaw(&bytes) {
                Ok(o) => o,
                Err(_) => return "⊥".into(),
            };
            let snapshot_read = tenant.read().snapshot_d();
            ast::metacompose(&obj, &snapshot_read)
        };
        let mut st = tenant.write();
        let snapshot = st.snapshot_d();
        let new_d = ast::register_runtime_fn(name, body, &snapshot);
        st.replace_d(new_d);
        return name.to_string();
    }

    // ── Federated ingest FFI (#305) ──────────────────────────────────
    //
    //   system(h, "federated_ingest:<noun>", <JSON>) → <cite-id> | ⊥
    //
    // Full ρ(populate_n) end-to-end: the host supplies the pre-fetched
    // response along with origin metadata; the engine pushes facts to
    // their declared FT cells and emits a Citation with Authority Type
    // 'Federated-Fetch'. All facts from the same fetch share the
    // returned Citation id.
    //
    // Payload shape:
    //   {
    //     "externalSystem": "stripe",
    //     "url": "https://api.stripe.com/v1/customers",
    //     "retrievalDate": "2026-04-20T12:00:00Z",
    //     "facts": [
    //       {"factTypeId": "Stripe_Customer_has_Email",
    //        "bindings": {"Stripe Customer": "cus_1", "Email": "a@x.com"}}
    //     ]
    //   }
    if let Some(noun) = key.strip_prefix("federated_ingest:") {
        let parsed: serde_json::Value = match serde_json::from_str(input) {
            Ok(v) => v,
            Err(_) => return "⊥".into(),
        };
        let external_system = parsed.get("externalSystem").and_then(|v| v.as_str()).unwrap_or("");
        let url = parsed.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let retrieval_date = parsed.get("retrievalDate").and_then(|v| v.as_str()).unwrap_or("");
        if url.is_empty() || external_system.is_empty() || retrieval_date.is_empty() {
            return "⊥".into();
        }

        // Cross-check against the compile-emitted populate:{noun} config
        // (#305): if the engine has a populate config for this noun, the
        // payload's externalSystem MUST match the declared `system`
        // value. This prevents a buggy / malicious caller from citing
        // origin other than what the domain's readings declare. Nouns
        // without a populate config (ad-hoc ingest) are unrestricted.
        let snapshot = tenant.read().snapshot_d();
        let config_obj = ast::apply(
            &ast::Func::Def(format!("populate:{}", noun)),
            &ast::Object::phi(),
            &snapshot,
        );
        let declared_system = config_obj.as_seq().and_then(|pairs| {
            pairs.iter().find_map(|pair| {
                let kv = pair.as_seq()?;
                let k = kv.first()?.as_atom()?;
                (k == "system").then(|| kv.get(1)?.as_atom().map(String::from)).flatten()
            })
        });
        if let Some(expected) = declared_system {
            if expected != external_system {
                return "⊥".into();
            }
        }

        let facts: Vec<(String, Vec<(String, String)>)> = parsed.get("facts")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|entry| {
                let ft_id = entry.get("factTypeId")?.as_str()?.to_string();
                let bindings = entry.get("bindings")?.as_object()?;
                let pairs: Vec<(String, String)> = bindings.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect();
                Some((ft_id, pairs))
            }).collect())
            .unwrap_or_default();
        let mut st = tenant.write();
        let snapshot = st.snapshot_d();
        let (cite_id, new_d) = ast::ingest_federated_facts(
            external_system, url, retrieval_date, &facts, &snapshot,
        );
        st.replace_d(new_d);
        return cite_id;
    }

    // ── Re-derive FFI (#305 follow-up to #14/#15) ────────────────────
    //
    //   system(h, "re_derive", "") → "<count>"
    //
    // Runs forward chaining (derivation rules) to lfp over the current
    // D, writing all newly-derived facts into their declared
    // consequent cells. Returns the count of newly-derived facts as a
    // decimal string.
    //
    // The standard path (command::create_via_defs) runs forward
    // chaining inline when a create command fires. federated_ingest
    // and register_runtime_fn bypass that path — they push facts
    // directly without retriggering lfp. This FFI is the opt-in
    // trigger for hosts that want derivations to fire over federated
    // or runtime-ingested facts.
    if key == "re_derive" {
        let mut st = tenant.write();
        let snapshot = st.snapshot_d();
        let derivation_defs_owned: Vec<(String, ast::Func)> = ast::cells_iter(&snapshot)
            .into_iter()
            .filter(|(n, _)| n.starts_with("derivation:"))
            .map(|(n, contents)| (n.to_string(), ast::metacompose(contents, &snapshot)))
            .collect();
        let refs: Vec<(&str, &ast::Func)> = derivation_defs_owned.iter()
            .map(|(n, f)| (n.as_str(), f))
            .collect();
        let (new_d, derived) = crate::evaluate::forward_chain_defs_state(&refs, &snapshot);
        st.replace_d(new_d);
        return format!("{}", derived.len());
    }

    if key == "snapshot" {
        let mut st = tenant.write();
        let label = if input.is_empty() {
            format!("snap-{}", st.snapshots.len())
        } else {
            input.to_string()
        };
        let snap = st.snapshot_d();
        st.snapshots.insert(label.clone(), snap);
        // Sec-4: append an HMAC tag over the raw label under the
        // tenant's secret. Caller keeps the signed form and must
        // hand it back unmodified to `rollback`. 16 hex chars = first
        // 64 bits of the HMAC-SHA256 digest — large enough that a
        // forger cannot enumerate tags even with unlimited rollback
        // attempts (2^64 guesses at one round-trip each).
        let digest = crate::crypto::sign_with(&st.snapshot_secret, label.as_bytes());
        return format!("{}.{}", label, &digest[..16]);
    }
    if key == "rollback" {
        let mut st = tenant.write();
        // Sec-4: accept only `<raw>.<tag>` form. Split on the LAST
        // dot so labels may contain dots ("release-v1.2.3" → raw =
        // "release-v1.2.3", tag = "…"). Unsigned ids are refused so
        // a caller that only reaches `system` cannot rewind state by
        // guessing or reading raw labels out of `snapshots`.
        let Some(dot_at) = input.rfind('.') else {
            return "⊥".into();
        };
        let raw = &input[..dot_at];
        let tag = &input[dot_at + 1..];
        if tag.len() != 16 || !tag.chars().all(|c| c.is_ascii_hexdigit()) {
            return "⊥".into();
        }
        // Constant-time tag compare against HMAC-SHA256(secret, raw).
        // Rejected tags never advance to the snapshot-map lookup, so
        // an attacker can't probe which raw labels exist through
        // timing of the rollback path.
        let expected = crate::crypto::sign_with(&st.snapshot_secret, raw.as_bytes());
        if !crate::crypto::ct_eq_str(&expected[..16], tag) {
            return "⊥".into();
        }
        return match st.snapshots.get(raw).cloned() {
            Some(snap) => {
                st.replace_d(snap);
                input.to_string()
            }
            None => "⊥".into(),
        };
    }
    if key == "snapshots" {
        let st = tenant.read();
        let mut ids: Vec<&String> = st.snapshots.keys().collect();
        ids.sort();
        return format!(
            "<{}>",
            ids.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
        );
    }

    // `check` (#199) — run the readings diagnostic pass without
    // committing. Input is the readings text; output is a pretty-
    // printed diagnostic list (one line per diagnostic, newline-
    // separated). Empty output means the readings parse cleanly AND
    // every reference resolves AND no deontic rule fires. Read-only;
    // no tenant-state mutation. LLM agents call this before `compile`
    // to self-correct schema authoring.
    if key == "check" {
        let diags = crate::check::check_readings(input);
        if diags.is_empty() {
            return String::new();
        }
        return diags.iter().map(|d| {
            let lvl = match d.level {
                crate::check::Level::Error => "ERROR",
                crate::check::Level::Warning => "WARN",
                crate::check::Level::Hint => "HINT",
            };
            let src = match d.source {
                crate::check::Source::Parse => "parse",
                crate::check::Source::Resolve => "resolve",
                crate::check::Source::Deontic => "deontic",
            };
            let suffix = d.suggestion.as_deref()
                .map(|s| format!(" (suggestion: {s})"))
                .unwrap_or_default();
            format!("[{lvl} {src}] {}: {}{}", d.reading, d.message, suffix)
        }).collect::<Vec<_>>().join("\n");
    }

    // `freeze` / `thaw` (#203) — byte-level state round-trip through
    // the system bridge. Encodes `freeze(snapshot_d())` bytes as hex
    // for string-only transport (wasm-bindgen + MCP both hand strings);
    // hex is chosen over base64 to avoid adding a dep. DO storage,
    // HTTP export/import, and FPGA ROM burn all read the same bytes.
    //
    //   system(h, "freeze", "")     → hex-encoded freeze image
    //   system(h, "thaw", "<hex>")  → replaces d; returns "ok" / "⊥"
    if key == "freeze" {
        let st = tenant.read();
        let d = st.snapshot_d();
        let bytes = crate::freeze::freeze(&d);
        // Lowercase hex, no separators. Stable, byte-deterministic.
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in &bytes {
            use core::fmt::Write;
            let _ = write!(&mut out, "{:02x}", b);
        }
        return out;
    }
    if key == "thaw" {
        // Parse hex input → bytes → thaw → Object → replace_d.
        let nibble = |b: u8| -> Option<u8> {
            match b {
                b'0'..=b'9' => Some(b - b'0'),
                b'a'..=b'f' => Some(b - b'a' + 10),
                b'A'..=b'F' => Some(b - b'A' + 10),
                _ => None,
            }
        };
        let clean: String = input.chars().filter(|c| !c.is_whitespace()).collect();
        if clean.len() % 2 != 0 {
            return "⊥".into();
        }
        let mut bytes: Vec<u8> = Vec::with_capacity(clean.len() / 2);
        let bs = clean.as_bytes();
        let mut i = 0;
        while i + 1 < bs.len() {
            match (nibble(bs[i]), nibble(bs[i + 1])) {
                (Some(h), Some(l)) => bytes.push((h << 4) | l),
                _ => return "⊥".into(),
            }
            i += 2;
        }
        return match crate::freeze::thaw(&bytes) {
            Ok(obj) => {
                let mut st = tenant.write();
                st.replace_d(obj);
                "ok".into()
            }
            Err(_) => "⊥".into(),
        };
    }

    // ── Read-only dispatch path ─────────────────────────────────────
    //
    // Known-read ops (list / get / query / debug / audit / explain /
    // verify_signature) take a shared lock. Result can never be a
    // "new D"; if apply() somehow returns a store-shaped Object for
    // one of these keys we silently don't persist it — that's a bug
    // in the op's definition, not a concurrency issue.
    if is_read_only_op(key) {
        let st = tenant.read();
        let obj = ast::Object::parse(input);
        let snapshot = st.snapshot_d();
        let result = ast::apply(&ast::Func::Def(key.to_string()), &obj, &snapshot);
        return result.to_json_string();
    }

    // ── Write dispatch path ─────────────────────────────────────────
    //
    // Two-tier commit:
    //   Tier 1 (shared-lock fast path): acquire tenant.read(), snapshot,
    //     apply, classify the result, and commit. For keys with known
    //     write targets (create:*, update:*, transition:*), use
    //     try_commit_declared (#186) which is O(|targets|) instead of
    //     O(|all_cells|). Opaque ops use try_commit_diff.
    //   Tier 2 (exclusive-lock escalation): on Stale/Structural outcome,
    //     drop the read, take tenant.write(), re-snapshot + re-apply +
    //     `replace_d`. Structural = new or removed cells; Stale = a
    //     concurrent writer's CAS check detected that our snapshot is
    //     no longer current.
    //
    // Re-running apply() on the escalated path is idempotent: apply is
    // functional on `&Object`; the cost is the second evaluation, paid
    // only on contention.
    let obj = ast::Object::parse(input);

    // Tier 1: shared-lock fast path.
    //
    // For create/update/transition verbs the write targets are
    // derived from the cell index via `write_targets_for_key`, which
    // calls `cells_for_noun` to include all RMAP-derived FT cells.
    // `try_commit_declared` then locks only those O(|targets|) cells
    // instead of diffing all O(|cells|). Opaque ops still fall back
    // to `try_commit_diff`.
    {
        let st = tenant.read();
        let snapshot = st.snapshot_d();
        let apply_result = ast::apply(&ast::Func::Def(key.to_string()), &obj, &snapshot);
        match classify_writer_result(&apply_result) {
            WriterResult::NoCommit { response } => return response,
            WriterResult::Commit { new_d, response } => {
                // Full-state commit (compile paths). Diff against the
                // snapshot under shared lock; if clean, CAS the changed
                // cells only. Schema-changing ops fall back to Tier 2.
                let outcome = st.try_commit_diff(&snapshot, &new_d);
                match outcome {
                    CommitOutcome::Committed => return response,
                    CommitOutcome::StaleSnapshot | CommitOutcome::StructuralChange => {
                        // fall through to Tier 2
                    }
                }
            }
            WriterResult::CommitDelta { delta, response } => {
                // #209: per-command delta. Merge onto snapshot, then
                // try_commit_diff against the same snapshot so the CAS
                // only touches the delta cells. If the snapshot is
                // stale, escalate to Tier 2 with a fresh re-apply.
                let new_d = ast::merge_delta(&snapshot, &delta);
                let outcome = st.try_commit_diff(&snapshot, &new_d);
                match outcome {
                    CommitOutcome::Committed => return response,
                    CommitOutcome::StaleSnapshot | CommitOutcome::StructuralChange => {
                        // fall through to Tier 2
                    }
                }
            }
        }
    }

    // Tier 2: exclusive-lock escalation.
    let mut st = tenant.write();
    let snapshot = st.snapshot_d();
    let apply_result = ast::apply(&ast::Func::Def(key.to_string()), &obj, &snapshot);
    match classify_writer_result(&apply_result) {
        WriterResult::NoCommit { response } => response,
        WriterResult::Commit { new_d, response } => {
            st.replace_d(new_d);
            response
        }
        WriterResult::CommitDelta { delta, response } => {
            let new_d = ast::merge_delta(&snapshot, &delta);
            st.replace_d(new_d);
            response
        }
    }
}

/// Extract declared write targets for known system verbs. Returns
/// `Some(vec)` when the verb's write-set is predictable, `None` for
/// opaque ops (compile, user-defined defs) that must use the full diff.
///
/// Uses `CompiledState::cells_for_noun` to include all RMAP-derived
/// FT cells (e.g. `Order_has_total`, `Order_has_Amount`) in the
/// declared set so that `try_commit_declared` covers every cell the
/// verb may touch. Extra targets cost one no-op CAS each — cheap.
#[cfg(not(feature = "no_std"))]
#[allow(dead_code)]
fn write_targets_for_key(key: &str, st: &CompiledState) -> Option<Vec<String>> {
    let (verb, noun) = key.split_once(':')?;
    match verb {
        "create" | "update" | "transition" => Some(st.cells_for_noun(noun)),
        _ => None,
    }
}

/// Outcome of classifying an `ast::apply` result in the write path.
#[cfg(not(feature = "no_std"))]
enum WriterResult {
    /// Result is a full new D (a bare store with a Noun cell), to be
    /// persisted with replace semantics. Used by platform_compile,
    /// where the schema itself may change.
    Commit { new_d: ast::Object, response: String },
    /// Result is a per-command delta (a Map of modified cells only,
    /// extracted from a `__state_delta` carrier), to be merged onto
    /// the current snapshot before commit. Used by create / update /
    /// transition / load_readings, which are encoded via
    /// `encode_command_result` and should touch only their RMAP cells
    /// (#209).
    CommitDelta { delta: ast::Object, response: String },
    /// Result is a query / non-D response; nothing to persist.
    NoCommit { response: String },
}

/// Classify an apply() result according to the writer-path shapes the
/// system recognises. Pure: no tenant mutation; callers decide whether
/// to commit under Tier-1 or Tier-2 locks.
///
/// Shapes:
///   1. Delta carrier `{__state_delta, __result}` — used by
///      create / update / transition (#209). Merge the delta cells
///      onto the snapshot, then commit.
///   2. Bare store with a Noun cell — used by platform_compile.
///      Commit the result; return a compact summary.
///   3. Anything else — pure query result; return as JSON.
#[cfg(not(feature = "no_std"))]
fn classify_writer_result(result: &ast::Object) -> WriterResult {
    if let Some(map) = result.as_map() {
        // Shape 1: delta carrier (#209).
        if map.contains_key("__state_delta") && map.contains_key("__result") {
            let delta = map.get("__state_delta").cloned().unwrap_or(ast::Object::Bottom);
            let response_obj = map.get("__result").cloned().unwrap_or(ast::Object::Bottom);
            let response = response_obj.as_atom().map(|s| s.to_string())
                .unwrap_or_else(|| response_obj.to_string());
            if delta.as_map().is_some() {
                return WriterResult::CommitDelta { delta, response };
            }
            return WriterResult::NoCommit { response };
        }
    }
    // Shape 2: bare store with a Noun cell.
    let looks_like_store = result.as_seq().is_some() || result.as_map().is_some();
    let is_new_d = looks_like_store
        && ast::fetch("Noun", result) != ast::Object::Bottom;
    if is_new_d {
        return WriterResult::Commit {
            new_d: result.clone(),
            response: r#"{"ok":true,"compiled":true}"#.to_string(),
        };
    }
    WriterResult::NoCommit { response: result.to_json_string() }
}

// ── WIT Component exports ───────────────────────────────────────────

#[cfg(feature = "wit")]
wit_bindgen::generate!({ world: "arest", path: "wit" });

#[cfg(feature = "wit")]
struct E;

#[cfg(feature = "wit")]
export!(E);

#[cfg(feature = "wit")]
impl exports::arest::engine::engine::Guest for E {
    fn parse_and_compile(readings: Vec<(String, String)>) -> Result<u32, String> {
        parse_and_compile_impl(readings)
    }
    fn release(handle: u32) { release_impl(handle) }
    fn system(handle: u32, key: String, input: String) -> String {
        system_impl(handle, &key, &input)
    }
}

// ── Security #15: WASM handle isolation ─────────────────────────────
//
// DOMAINS is a process-global Vec<Option<CompiledState>> guarded by a Mutex.
// Each create_impl() call allocates a fresh slot (reusing holes left by
// release_impl) and returns its index as the opaque handle. State is stored
// by-value (ast::Object is owned — no Arc, no &'static references escape),
// and every system_impl() read scopes its snapshot to the lifetime of
// the Mutex guard, so no cross-handle aliasing is possible.
//
// Invariants verified below:
//   1. Two create_impl() calls return distinct indices.
//   2. Mutations on handle A never touch handle B's slot.
//   3. An invalid handle (never-allocated, out-of-bounds) returns ⊥ from
//      every system_impl() dispatch and has no stored state. Released
//      handles' slot contents are not asserted directly because slot
//      recycling races with parallel tests — the freshness invariant (5)
//      is the real guarantee.
//   4. release_impl() on any handle — live, recently freed, or out of
//      bounds — is a safe no-op and never panics.
//   5. A freed slot's index may be recycled, but the new handle starts with
//      a fresh CompiledState — no residual state from the previous tenant.
//
// Cross-runtime coverage: src/tests/security/authorization.test.ts exercises
// the same invariants through the TS/WASM boundary via compileDomainReadings
// / releaseDomain / systemRaw under `describe('Handle isolation', ...)`.
#[cfg(test)]
mod handle_isolation_tests {
    use super::*;

    /// Test-only peek at a handle's compiled state. Takes a shared
    /// (read) lock and assembles an Object::Map snapshot from the
    /// per-cell locks; no DOMAINS / tenant references held after
    /// return. Read-only; doesn't block other readers on the same
    /// handle.
    fn peek(handle: u32) -> Option<ast::Object> {
        let tenant = tenant_lock(handle)?;
        let st = tenant.read();
        Some(st.snapshot_d())
    }

    /// Install a Noun cell with the given payload directly, bypassing the
    /// compile pipeline. Returns a fresh handle owning that state.
    fn alloc_with_noun(payload: &str) -> u32 {
        let state = ast::store("Noun", ast::Object::atom(payload), &ast::Object::phi());
        allocate(state, vec![])
    }

    #[test]
    fn two_creates_return_distinct_handles() {
        let h1 = create_bare_impl();
        let h2 = create_bare_impl();
        assert_ne!(h1, h2, "create must return distinct handle indices");
        release_impl(h1);
        release_impl(h2);
    }

    #[test]
    fn state_mutation_on_one_handle_does_not_leak_to_another() {
        let h_a = alloc_with_noun("tenant-a-secret");
        let h_b = alloc_with_noun("tenant-b-secret");
        assert_ne!(h_a, h_b);

        let d_a = peek(h_a).expect("handle A must be live");
        let d_b = peek(h_b).expect("handle B must be live");
        assert_eq!(ast::fetch("Noun", &d_a), ast::Object::atom("tenant-a-secret"));
        assert_eq!(ast::fetch("Noun", &d_b), ast::Object::atom("tenant-b-secret"));

        // Mutate A's slot directly (simulating what system_impl does on a
        // state-transition def) and re-check B to prove no aliasing.
        {
            let tenant_a = tenant_lock(h_a).expect("handle A must be live");
            let mut st = tenant_a.write();
            let snapshot = st.snapshot_d();
            let new_d = ast::store(
                "Noun",
                ast::Object::atom("tenant-a-mutated"),
                &snapshot,
            );
            st.replace_d(new_d);
        }

        let d_a2 = peek(h_a).unwrap();
        let d_b2 = peek(h_b).unwrap();
        assert_eq!(ast::fetch("Noun", &d_a2), ast::Object::atom("tenant-a-mutated"));
        assert_eq!(
            ast::fetch("Noun", &d_b2),
            ast::Object::atom("tenant-b-secret"),
            "handle B must be unaffected by mutations on handle A",
        );

        release_impl(h_a);
        release_impl(h_b);
    }

    #[test]
    fn invalid_handle_returns_bottom_for_all_operations() {
        // u32::MAX is beyond any allocation (Vec<CompiledState> never grows
        // that large), so the slot is guaranteed absent. A released handle's
        // slot may be recycled by a parallel test before we read it, so
        // asserting ⊥ post-release races with the allocator. u32::MAX dodges
        // that entirely while covering the same invariant: any handle not
        // currently owning a live slot returns ⊥ from every system dispatch.
        let h = u32::MAX;
        assert_eq!(system_impl(h, "compile", "anything"), "⊥");
        assert_eq!(system_impl(h, "apply", "<x>"), "⊥");
        assert_eq!(system_impl(h, "any_def_name", ""), "⊥");
        assert!(peek(h).is_none(), "invalid handle must have no stored state");
    }

    #[test]
    fn release_is_idempotent_and_bounds_safe() {
        // The safety property is "release never panics" — on a live slot,
        // a recently-freed slot, or a completely-out-of-bounds index. A
        // slot's post-release content is covered by the invalid_handle
        // test above; asserting it here races with recycling under
        // cargo's default parallel test runner.
        let h = create_bare_impl();
        release_impl(h);
        release_impl(h); // double-release
        release_impl(u32::MAX);
        release_impl(999_999);
    }

    #[test]
    fn recycled_slot_has_no_residual_state() {
        // Install a tenant, release it, then create a fresh bare handle.
        // The new handle may reuse the same index — it must NOT observe
        // stale state from the previous tenant.
        let h_old = alloc_with_noun("leaked-secret");
        let stale = ast::fetch("Noun", &peek(h_old).unwrap());
        assert_eq!(stale, ast::Object::atom("leaked-secret"));
        release_impl(h_old);

        let h_new = create_bare_impl();
        let fresh_d = peek(h_new).expect("new handle must be live");
        // create_bare_impl starts from Object::phi() with only platform
        // defs; no Noun cell should be present.
        assert_eq!(
            ast::fetch("Noun", &fresh_d),
            ast::Object::Bottom,
            "recycled bare slot must not carry prior tenant's Noun cell",
        );
        release_impl(h_new);
    }

    /// create_impl loads the bundled metamodel, so a fresh handle MUST
    /// have a populated Noun cell (from core.md at minimum).
    #[test]
    fn create_impl_loads_metamodel() {
        let h = create_impl();
        let d = peek(h).expect("handle must be live");
        let nouns = ast::fetch("Noun", &d);
        assert_ne!(nouns, ast::Object::Bottom,
            "create_impl must load the metamodel — Noun cell should be populated");
        // The metamodel defines at least Noun, Fact Type, Role, Constraint
        // as reserved noun names. Verify the cell has multiple entries.
        let count = nouns.as_seq().map(|s| s.len()).unwrap_or(0);
        assert!(count > 5,
            "metamodel should populate at least a handful of noun entries, got {}", count);
        release_impl(h);
    }

    #[test]
    fn create_bare_impl_skips_metamodel() {
        let h = create_bare_impl();
        let d = peek(h).expect("handle must be live");
        // Bare mode: no Noun cell, no metamodel facts at all — just the
        // three platform primitives.
        assert_eq!(ast::fetch("Noun", &d), ast::Object::Bottom,
            "create_bare_impl must NOT load the metamodel");
        release_impl(h);
    }

    #[test]
    fn no_static_aliasing_across_handles() {
        // Pointer-identity check: the two tenants stored under distinct
        // handles must not share the same Arc — distinct allocations.
        // The per-tenant inner Mutex is per-Arc; if the Arcs aliased,
        // tenant A's lock would also block tenant B.
        let h_a = alloc_with_noun("alpha");
        let h_b = alloc_with_noun("beta");

        let arc_a = tenant_lock(h_a).expect("h_a must be live");
        let arc_b = tenant_lock(h_b).expect("h_b must be live");
        assert!(!Arc::ptr_eq(&arc_a, &arc_b),
            "each handle must own a distinct tenant Arc<RwLock<CompiledState>>");

        release_impl(h_a);
        release_impl(h_b);
    }

    /// `audit_log` must be reachable as a system def — return the
    /// audit trail as a JSON array, and each entry for an entity-scoped
    /// apply must carry the entity id so `explain` can filter by it.
    #[test]
    fn audit_log_reachable_via_system_and_carries_entity_id() {
        let h = create_impl();

        let _ = system_impl(h, "compile", "\
Order(.id) is an entity type.
Order has total.
");
        let create_out = system_impl(h, "create:Order", "<<id, audit-ord-1>, <total, 7>>");
        assert!(!create_out.starts_with('⊥'), "create:Order must succeed, got: {create_out}");

        // Pass "0" as the (unused) input because apply() short-circuits on
        // Object::Bottom — an empty string parses to ⊥. The def is named
        // `audit` (not `audit_log`) to avoid shadowing the `audit_log` data
        // cell that cell_push overwrites on every create.
        let audit_out = system_impl(h, "audit", "0");
        assert!(!audit_out.starts_with('⊥'),
            "system('audit', '0') must not return ⊥; got: {audit_out}");
        assert!(audit_out.starts_with('['),
            "audit must return a JSON array; got: {audit_out}");
        assert!(audit_out.contains("apply:create"),
            "audit must record the apply:create operation; got: {audit_out}");
        assert!(audit_out.contains("audit-ord-1"),
            "audit entries for entity-scoped applies must carry the entity id; got: {audit_out}");

        release_impl(h);
    }

    /// After `create:Order` adds an entity to D via apply, both
    /// `list:Order` and `get:Order` must see it. Currently those defs
    /// are compile-time constants baked from Instance Facts, so they
    /// never observe runtime-created entities.
    ///
    /// Per whitepaper Eq 9: SYSTEM:x = (ρ(↑entity(x):D)):↑op(x). The
    /// read path is a ρ-application that fetches from the live D.
    #[test]
    fn list_and_get_see_runtime_created_entities() {
        let h = create_impl();

        let readings = "\
Order(.id) is an entity type.
Order has total.
  Each Order has at most one total.
";
        let compile_out = system_impl(h, "compile", readings);
        assert!(!compile_out.starts_with('⊥'),
            "compile must not reject simple Order schema, got: {compile_out}");

        let create_out = system_impl(h, "create:Order", "<<id, ord-1>, <total, 100>>");
        assert!(!create_out.starts_with('⊥'),
            "create:Order must not return ⊥, got: {create_out}");

        let list_out = system_impl(h, "list:Order", "");
        assert!(!list_out.starts_with('⊥'),
            "list:Order must not return ⊥ after an entity has been created");
        assert!(list_out.contains("ord-1"),
            "list:Order must surface the runtime-created entity 'ord-1'; got: {list_out}");

        let get_out = system_impl(h, "get:Order", "ord-1");
        assert!(!get_out.starts_with('⊥'),
            "get:Order must not return ⊥ for an entity that was just created");
        assert!(get_out.contains("ord-1"),
            "get:Order must return a payload containing the entity id; got: {get_out}");

        release_impl(h);
    }

    /// Profiling invocation — runs the same create/list/get workload as
    /// `list_and_get_see_runtime_created_entities` with the apply-
    /// variant profiler enabled, then dumps the histogram to stderr.
    /// #[ignore]'d by default because profiling adds ~20% overhead and
    /// clutters ordinary test runs. Invoke explicitly:
    ///
    ///   cargo test --lib profile_create_order -- --ignored --nocapture
    ///
    /// Read the dump to decide where each remaining perf cycle goes.
    #[cfg(feature = "profile")]
    #[test]
    #[ignore = "profiling run; invoke with --features profile --ignored --nocapture"]
    fn profile_create_order_dump_histogram() {
        ast::profile_reset();
        ast::profile_enable();

        let h = create_impl();
        let readings = "\
Order(.id) is an entity type.
Order has total.
  Each Order has at most one total.
";
        let _ = system_impl(h, "compile", readings);
        let _ = system_impl(h, "create:Order", "<<id, ord-1>, <total, 100>>");
        let _ = system_impl(h, "list:Order", "");
        let _ = system_impl(h, "get:Order", "ord-1");
        release_impl(h);

        ast::profile_disable();
        ast::profile_dump();
    }

    // ── Snapshot / rollback ─────────────────────────────────────

    #[test]
    fn snapshot_returns_auto_id_when_input_empty() {
        // Sec-4: the raw portion carries the monotonic counter; the
        // full id adds an HMAC tag. Compare on the raw prefix only —
        // the tag itself is exercised by the Sec-4 test block below.
        let h = create_bare_impl();
        let id1 = system_impl(h, "snapshot", "");
        let id2 = system_impl(h, "snapshot", "");
        let (raw1, _) = split_signed_id(&id1);
        let (raw2, _) = split_signed_id(&id2);
        assert_eq!(raw1, "snap-0", "first auto id");
        assert_eq!(raw2, "snap-1", "second auto id — monotonic counter");
        release_impl(h);
    }

    #[test]
    fn snapshot_accepts_caller_label_verbatim() {
        // Sec-4: the raw portion echoes the caller label; the HMAC tag
        // is deterministic per (secret, label), so a second snapshot
        // under the same label returns an identical signed id.
        let h = create_bare_impl();
        let first = system_impl(h, "snapshot", "before-migrate");
        let (raw, _) = split_signed_id(&first);
        assert_eq!(raw, "before-migrate", "raw portion echoes label");
        assert_eq!(system_impl(h, "snapshot", "before-migrate"), first,
            "same label is idempotent — tag is deterministic, so signed id is stable");
        release_impl(h);
    }

    #[test]
    fn snapshots_listing_is_sorted_and_ffp_sequence() {
        let h = create_bare_impl();
        let _ = system_impl(h, "snapshot", "b");
        let _ = system_impl(h, "snapshot", "a");
        let _ = system_impl(h, "snapshot", "c");
        assert_eq!(system_impl(h, "snapshots", ""), "<a, b, c>");
        release_impl(h);
    }

    #[test]
    fn rollback_to_missing_snapshot_returns_bottom() {
        let h = create_bare_impl();
        assert_eq!(system_impl(h, "rollback", "nonexistent"), "⊥");
        release_impl(h);
    }

    #[test]
    fn rollback_restores_state_to_snapshot() {
        // Snapshot a known-good state; mutate it via direct cell write;
        // rollback; confirm the cell is back to its pre-mutation content.
        let h = alloc_with_noun("before");
        let signed = system_impl(h, "snapshot", "v1");
        // Mutate the Noun cell by replacing the whole state.
        {
            let tenant = tenant_lock(h).unwrap();
            let mut st = tenant.write();
            st.replace_d(ast::store("Noun", ast::Object::atom("after"), &ast::Object::phi()));
        }
        assert_eq!(
            ast::fetch("Noun", &peek(h).unwrap()),
            ast::Object::atom("after"),
            "mutation landed"
        );
        // Roll back to v1 via the signed id returned by snapshot.
        assert_eq!(system_impl(h, "rollback", &signed), signed);
        assert_eq!(
            ast::fetch("Noun", &peek(h).unwrap()),
            ast::Object::atom("before"),
            "rollback restored the v1 payload"
        );
        release_impl(h);
    }

    #[test]
    fn rollback_is_repeatable_from_same_snapshot() {
        // One snapshot can be rolled back to many times — the snapshot
        // map is not drained on rollback.
        let h = alloc_with_noun("origin");
        let signed = system_impl(h, "snapshot", "anchor");
        for round in 0..3 {
            {
                let tenant = tenant_lock(h).unwrap();
                let mut st = tenant.write();
                st.replace_d(ast::store(
                    "Noun",
                    ast::Object::atom(&format!("mutation-{round}")),
                    &ast::Object::phi(),
                ));
            }
            assert_eq!(system_impl(h, "rollback", &signed), signed);
            assert_eq!(
                ast::fetch("Noun", &peek(h).unwrap()),
                ast::Object::atom("origin"),
                "round {round} rollback lands"
            );
        }
        release_impl(h);
    }

    #[test]
    fn snapshots_are_per_handle_not_shared() {
        // h1's snapshot must be invisible to h2. Taking snapshots under
        // the same label in different handles must not cross-contaminate.
        let h1 = alloc_with_noun("h1-payload");
        let h2 = alloc_with_noun("h2-payload");
        let _ = system_impl(h1, "snapshot", "shared-label");

        // h2 has no snapshot called "shared-label".
        assert_eq!(system_impl(h2, "rollback", "shared-label"), "⊥");
        assert_eq!(system_impl(h2, "snapshots", ""), "<>");

        // h1 still sees its own snapshot.
        assert_eq!(system_impl(h1, "snapshots", ""), "<shared-label>");
        release_impl(h1);
        release_impl(h2);
    }

    #[test]
    fn snapshot_and_rollback_on_invalid_handle_return_bottom() {
        // Invalid handles must not panic and must yield ⊥.
        assert_eq!(system_impl(u32::MAX, "snapshot", ""), "⊥");
        assert_eq!(system_impl(u32::MAX, "rollback", "whatever"), "⊥");
        assert_eq!(system_impl(u32::MAX, "snapshots", ""), "⊥");
    }

    // ── Sec-4: HMAC-signed snapshot ids ──────────────────────────────
    //
    // Without signatures, any caller that can reach `system` can pass
    // `rollback <guess>` and rewind tenant state by brute-forcing ids.
    // snapshot: now returns `<raw>.<16-hex-tag>` where the tag is an
    // HMAC-SHA256 of the raw id under a per-tenant secret generated at
    // CompiledState::new(). rollback: splits on the last `.`, validates
    // the tag in constant time, and only then consults the snapshot
    // map. Unsigned legacy ids are refused outright — the signed form
    // is the only path through.
    //
    // Split on the LAST dot so labels may themselves contain dots
    // (e.g. "release-v1.2.3" → raw="release-v1.2.3", tag="…").

    /// Parse the `<raw>.<hex-tag>` form for tests. Panics with a
    /// descriptive message if the shape is wrong.
    fn split_signed_id(id: &str) -> (&str, &str) {
        let dot = id.rfind('.').unwrap_or_else(||
            panic!("expected signed id '<raw>.<tag>', got {id:?}"));
        (&id[..dot], &id[dot + 1..])
    }

    #[test]
    fn snapshot_returns_signed_id_with_16_hex_tag() {
        let h = create_bare_impl();
        let id = system_impl(h, "snapshot", "v1");
        let (raw, tag) = split_signed_id(&id);
        assert_eq!(raw, "v1", "raw portion must echo the caller label");
        assert_eq!(tag.len(), 16,
            "tag must be 16 hex chars (64 bits of HMAC-SHA256); got {} in {id:?}",
            tag.len());
        assert!(tag.chars().all(|c| c.is_ascii_hexdigit()),
            "tag must be lowercase hex; got {tag:?}");
        release_impl(h);
    }

    #[test]
    fn snapshot_auto_id_is_also_signed() {
        let h = create_bare_impl();
        let id = system_impl(h, "snapshot", "");
        let (raw, tag) = split_signed_id(&id);
        assert_eq!(raw, "snap-0", "auto id numbering unchanged");
        assert_eq!(tag.len(), 16);
        release_impl(h);
    }

    #[test]
    fn rollback_with_raw_portion_alone_is_refused() {
        // Handoff acceptance: "rollback with raw portion alone fails."
        // An attacker who only learns the raw label (e.g. via
        // `snapshots`) must not be able to rewind — the tag is required.
        let h = alloc_with_noun("before");
        let signed = system_impl(h, "snapshot", "anchor");
        let (raw, _tag) = split_signed_id(&signed);
        assert_eq!(system_impl(h, "rollback", raw), "⊥",
            "rollback with only the raw label must return ⊥; \
             signed id was {signed:?}");
        release_impl(h);
    }

    #[test]
    fn rollback_with_signed_id_succeeds_and_restores_state() {
        // Handoff acceptance: "rollback with signed id succeeds."
        let h = alloc_with_noun("origin");
        let signed = system_impl(h, "snapshot", "v1");

        // Mutate the tenant, then roll back with the signed id.
        {
            let tenant = tenant_lock(h).unwrap();
            let mut st = tenant.write();
            st.replace_d(ast::store("Noun", ast::Object::atom("mutated"), &ast::Object::phi()));
        }
        assert_eq!(
            ast::fetch("Noun", &peek(h).unwrap()),
            ast::Object::atom("mutated"),
            "precondition: mutation landed"
        );

        assert_eq!(system_impl(h, "rollback", &signed), signed,
            "rollback must echo the signed id on success");
        assert_eq!(
            ast::fetch("Noun", &peek(h).unwrap()),
            ast::Object::atom("origin"),
            "rollback must restore the pre-mutation payload"
        );
        release_impl(h);
    }

    #[test]
    fn rollback_with_tampered_signature_returns_bottom() {
        // Handoff acceptance: "tampering the signature portion returns
        // Bottom." Any edit to the tag — even one hex nibble — must
        // fail HMAC verification and be rejected before the snapshot
        // map is consulted.
        let h = alloc_with_noun("origin");
        let signed = system_impl(h, "snapshot", "v1");
        let (raw, tag) = split_signed_id(&signed);

        // Flip the first hex char of the tag to a guaranteed-different
        // nibble (0<->1).
        let mut bytes = tag.as_bytes().to_vec();
        bytes[0] = match bytes[0] {
            b'0' => b'1',
            _    => b'0',
        };
        let tampered_tag = std::str::from_utf8(&bytes).unwrap();
        let tampered = format!("{raw}.{tampered_tag}");
        assert_ne!(tampered, signed, "test setup must produce a distinct tag");

        assert_eq!(system_impl(h, "rollback", &tampered), "⊥",
            "rollback with tampered tag must return ⊥; tampered id was {tampered:?}");
        release_impl(h);
    }

    #[test]
    fn rollback_rejects_unsigned_id_even_after_matching_snapshot_stored() {
        // Regression: even if the snapshot map DOES contain an entry
        // under the raw label, rollback with the raw label alone must
        // still fail. The tag check is the primary gate; the map
        // lookup only runs after successful verification.
        let h = alloc_with_noun("origin");
        let _signed = system_impl(h, "snapshot", "legacy");
        assert_eq!(system_impl(h, "rollback", "legacy"), "⊥",
            "raw label without tag must be refused");
        // State must be unchanged — rollback didn't fire.
        assert_eq!(
            ast::fetch("Noun", &peek(h).unwrap()),
            ast::Object::atom("origin"),
            "refused rollback must not mutate state"
        );
        release_impl(h);
    }

    #[test]
    fn signed_id_from_one_tenant_does_not_validate_on_another() {
        // The secret is per-tenant — leaking a signed id from h1 must
        // not let a caller rollback on h2. h2 has no matching snapshot
        // under the raw label anyway, but signature verification must
        // fail first (so even pre-loading the label on h2 wouldn't
        // help).
        let h1 = alloc_with_noun("h1");
        let h2 = alloc_with_noun("h2");
        let signed_on_h1 = system_impl(h1, "snapshot", "shared-label");

        // Stash a snapshot under the same raw label on h2 so any
        // lookup that bypasses signature verification would spuriously
        // succeed. The signed id from h1 must still fail on h2.
        let _h2_signed = system_impl(h2, "snapshot", "shared-label");

        assert_eq!(system_impl(h2, "rollback", &signed_on_h1), "⊥",
            "h1's signed id must not validate against h2's secret");
        release_impl(h1);
        release_impl(h2);
    }

    // ── freeze / thaw round-trip through system bridge (#203) ──────

    #[test]
    fn freeze_produces_hex_with_arest_magic() {
        let h = alloc_with_noun("payload");
        let hex = system_impl(h, "freeze", "");
        // Magic "AREST\x01" → first 12 hex chars "4152455354" + "01".
        assert!(hex.starts_with("41524553540"),
            "freeze output must begin with the AREST magic header, got: {}",
            &hex[..hex.len().min(32)]);
        // All hex-valid bytes, no whitespace.
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
        release_impl(h);
    }

    #[test]
    fn thaw_restores_frozen_state_across_handles() {
        // Snapshot on h1, freeze to hex, thaw into a fresh h2,
        // confirm h2 sees the same Noun cell payload. This is the
        // portability contract: the bytes alone reconstruct the tenant.
        let h1 = alloc_with_noun("original-payload");
        let hex = system_impl(h1, "freeze", "");
        assert!(!hex.is_empty());
        release_impl(h1);

        let h2 = create_bare_impl();
        assert_eq!(system_impl(h2, "thaw", &hex), "ok");
        // h2's Noun cell now carries the h1 payload.
        let noun_cell_after_thaw = {
            let tenant = tenant_lock(h2).unwrap();
            let st = tenant.read();
            st.snapshot_d()
        };
        let s = format!("{:?}", noun_cell_after_thaw);
        assert!(s.contains("original-payload"),
            "thawed state must carry the h1 payload, got: {}", s);
        release_impl(h2);
    }

    #[test]
    fn thaw_rejects_malformed_hex() {
        let h = create_bare_impl();
        assert_eq!(system_impl(h, "thaw", "not-hex-bytes"), "⊥");
        assert_eq!(system_impl(h, "thaw", "xyz"), "⊥");
        assert_eq!(system_impl(h, "thaw", "a"), "⊥",
            "odd-length hex must reject");
        release_impl(h);
    }

    #[test]
    fn thaw_rejects_non_arest_bytes() {
        // Even well-formed hex must produce an AREST freeze image
        // under the magic header — arbitrary bytes fail thaw cleanly.
        let h = create_bare_impl();
        assert_eq!(system_impl(h, "thaw", "deadbeef"), "⊥");
        release_impl(h);
    }

    #[test]
    fn freeze_is_byte_deterministic_across_snapshots() {
        // Two freezes of the same state must be byte-identical.
        // Required for reproducible DO storage (#203) and ROM hashing (#171).
        let h = alloc_with_noun("deterministic");
        let a = system_impl(h, "freeze", "");
        let b = system_impl(h, "freeze", "");
        assert_eq!(a, b);
        release_impl(h);
    }

    // ── Per-tenant read/write lock classification ──────────────────

    #[test]
    fn read_only_op_classification_covers_query_verbs() {
        assert!(is_read_only_op("debug"));
        assert!(is_read_only_op("audit"));
        assert!(is_read_only_op("verify_signature"));
        assert!(is_read_only_op("snapshots"));
        assert!(is_read_only_op("list:Order"));
        assert!(is_read_only_op("get:Customer"));
        assert!(is_read_only_op("query:order_has_total"));
        assert!(is_read_only_op("explain:123"));
        // Mutating ops stay on the write path.
        assert!(!is_read_only_op("compile"));
        assert!(!is_read_only_op("create:Order"));
        assert!(!is_read_only_op("update:Order"));
        assert!(!is_read_only_op("transition:Order"));
        assert!(!is_read_only_op("snapshot"));
        assert!(!is_read_only_op("rollback"));
    }

    #[test]
    fn two_concurrent_readers_hold_the_tenant_lock_simultaneously() {
        // The per-tenant RwLock should let two readers hold the shared
        // guard at the same instant. A Barrier(2) forces both threads
        // to be inside the read guard concurrently — under the prior
        // Mutex this would deadlock (wait would block the second
        // reader since the first hasn't released yet).
        use std::sync::Barrier;
        use std::thread;

        let h = alloc_with_noun("shared-payload");
        let barrier = Arc::new(Barrier::new(2));

        let reader = |h: u32, barrier: Arc<Barrier>| move || -> ast::Object {
            let tenant = tenant_lock(h).unwrap();
            let st = tenant.read();
            // Both readers reach the barrier while holding their read
            // guards. If the lock doesn't allow sharing, only one will
            // ever get here and the test hangs.
            barrier.wait();
            let d = st.snapshot_d();
            drop(st);
            d
        };

        let t1 = thread::spawn(reader(h, barrier.clone()));
        let t2 = thread::spawn(reader(h, barrier.clone()));
        let (d1, d2) = (t1.join().unwrap(), t2.join().unwrap());
        assert_eq!(d1, d2, "both readers saw the same state");
        release_impl(h);
    }

    // ── Per-cell write locks: parallel disjoint-cell writes ────────

    #[test]
    fn disjoint_cell_writers_run_in_parallel_via_try_commit_diff() {
        // Two threads attempt to write to DIFFERENT cells on the same
        // handle. Under the per-cell-lock design, both should hold
        // tenant.read() simultaneously (via a Barrier synchronization
        // point), then each writes only its target cell through
        // try_commit_diff. No tenant.write() escalation; both commit.
        use std::sync::Barrier;
        use std::thread;

        // Seed the handle with cells Order + Customer alongside the
        // Noun sentinel that `alloc_with_noun` installs. We need the
        // cells to pre-exist so try_commit_diff's structural-change
        // detector passes.
        let h = alloc_with_noun("seed");
        {
            let tenant = tenant_lock(h).unwrap();
            let mut st = tenant.write();
            let state = {
                let s = ast::store("Noun", ast::Object::atom("seed"), &ast::Object::phi());
                let s = ast::store("Order", ast::Object::atom("o0"), &s);
                ast::store("Customer", ast::Object::atom("c0"), &s)
            };
            st.replace_d(state);
        }

        let barrier = Arc::new(Barrier::new(2));
        let write = |h: u32, b: Arc<Barrier>, cell: &'static str, val: &'static str| move || -> CommitOutcome {
            let tenant = tenant_lock(h).unwrap();
            let st = tenant.read();
            let snapshot = st.snapshot_d();
            let new_d = ast::store(cell, ast::Object::atom(val), &snapshot);
            // Both writers reach the barrier while holding the shared
            // tenant lock. If per-cell commit didn't work, either the
            // snapshot or the commit would deadlock/serialize.
            b.wait();
            st.try_commit_diff(&snapshot, &new_d)
        };

        let t1 = thread::spawn(write(h, barrier.clone(), "Order", "o1"));
        let t2 = thread::spawn(write(h, barrier.clone(), "Customer", "c1"));
        let o1 = t1.join().unwrap();
        let o2 = t2.join().unwrap();
        assert!(matches!(o1, CommitOutcome::Committed),
            "Order writer committed (got {:?})", o1 as u8);
        assert!(matches!(o2, CommitOutcome::Committed),
            "Customer writer committed (got {:?})", o2 as u8);

        let d = peek(h).unwrap();
        assert_eq!(ast::fetch("Order", &d), ast::Object::atom("o1"));
        assert_eq!(ast::fetch("Customer", &d), ast::Object::atom("c1"));
        assert_eq!(ast::fetch("Noun", &d), ast::Object::atom("seed"),
            "untouched cell preserved");
        release_impl(h);
    }

    #[test]
    fn same_cell_cas_rejects_stale_snapshot() {
        // Write contention on the same cell must NOT silently lose an
        // update. Simulate: thread A snapshots at v0 and holds its
        // snapshot while thread B completes a full v0 → v1 write. A
        // then tries to commit v2 based on its stale snapshot.
        // try_commit_diff must return StaleSnapshot so A retries (or
        // escalates) rather than clobbering B's v1.
        let h = alloc_with_noun("v0");

        // A's snapshot, captured before B's write.
        let stale_snapshot = {
            let tenant = tenant_lock(h).unwrap();
            let st = tenant.read();
            st.snapshot_d()
        };

        // B commits a full replacement to "v1-other".
        {
            let tenant = tenant_lock(h).unwrap();
            let mut st = tenant.write();
            st.replace_d(ast::store(
                "Noun",
                ast::Object::atom("v1-other"),
                &ast::Object::phi(),
            ));
        }

        // A builds a new_d from its stale snapshot and tries to commit.
        let attempted_new_d = ast::store(
            "Noun",
            ast::Object::atom("v2-us"),
            &stale_snapshot,
        );
        let outcome = {
            let tenant = tenant_lock(h).unwrap();
            let st = tenant.read();
            st.try_commit_diff(&stale_snapshot, &attempted_new_d)
        };
        assert!(matches!(outcome, CommitOutcome::StaleSnapshot),
            "stale snapshot must be rejected by CAS check");

        // Noun still holds B's write; A's attempt was refused.
        let d = peek(h).unwrap();
        assert_eq!(ast::fetch("Noun", &d), ast::Object::atom("v1-other"));
        release_impl(h);
    }

    #[test]
    fn try_commit_diff_flags_structural_change_for_new_cells() {
        // A commit that introduces a cell name not present in the
        // current state must return StructuralChange — the cells map
        // itself needs mutation, which requires tenant.write().
        let h = alloc_with_noun("seed");
        let tenant = tenant_lock(h).unwrap();
        let st = tenant.read();
        let snapshot = st.snapshot_d();
        // Add a NEW cell not in the snapshot.
        let new_d = ast::store("Fresh", ast::Object::atom("unseen"), &snapshot);
        let outcome = st.try_commit_diff(&snapshot, &new_d);
        assert!(matches!(outcome, CommitOutcome::StructuralChange),
            "adding a cell requires the outer write lock");
        drop(st);
        release_impl(h);
    }

    #[test]
    fn concurrent_read_ops_via_system_impl_both_return() {
        // End-to-end: two `debug` calls on the same handle, both on
        // the read-path (is_read_only_op == true), both succeed. No
        // mutation happens, so neither thread's result shadows the
        // other.
        use std::thread;

        let h = create_bare_impl();
        let t1 = thread::spawn(move || system_impl(h, "debug", ""));
        let t2 = thread::spawn(move || system_impl(h, "debug", ""));
        let r1 = t1.join().unwrap();
        let r2 = t2.join().unwrap();
        assert!(!r1.is_empty(), "first reader got a debug projection");
        assert!(!r2.is_empty(), "second reader got a debug projection");
        release_impl(h);
    }

    // ── try_commit_declared wired via FT-cell index (#207) ────────────

    #[test]
    fn declared_writes_path_commits_order_without_structural_change_fallback() {
        // Verify that for create/update/transition verbs, Tier 1 uses
        // try_commit_declared (via write_targets_for_key + cells_for_noun)
        // and commits successfully without escalating to the Tier-2
        // exclusive-lock path.
        //
        // Strategy: pre-seed a tenant with Order, Order_has_total, and
        // audit_log cells (simulating a compiled domain with an RMAP-
        // derived FT cell), then call write_targets_for_key and
        // try_commit_declared directly to assert:
        //   1. cells_for_noun("Order") returns all Order-related cells.
        //   2. try_commit_declared commits successfully (Committed).
        //   3. No StructuralChange fallback occurs.
        let h = alloc_with_noun("Order");
        {
            // Extend the state with FT cells that RMAP would produce.
            let tenant = tenant_lock(h).unwrap();
            let mut st = tenant.write();
            let state = {
                let s = ast::store("Order",           ast::Object::atom("o0"),    &ast::Object::phi());
                let s = ast::store("Order_has_total", ast::Object::atom("0"),     &s);
                let s = ast::store("Order_has_Amount",ast::Object::atom("100"),   &s);
                ast::store("audit_log",               ast::Object::atom("[]"),    &s)
            };
            st.replace_d(state);
        }

        let tenant = tenant_lock(h).unwrap();
        let st = tenant.read();
        let snapshot = st.snapshot_d();

        // cells_for_noun must include all Order-related cells + audit_log.
        let targets = st.cells_for_noun("Order");
        assert!(targets.contains(&"Order".to_string()),           "Order cell included");
        assert!(targets.contains(&"Order_has_total".to_string()), "FT cell Order_has_total included");
        assert!(targets.contains(&"Order_has_Amount".to_string()),"FT cell Order_has_Amount included");
        assert!(targets.contains(&"audit_log".to_string()),       "audit_log always included");

        // Build a new_d that updates Order and one FT cell.
        let new_d = {
            let s = ast::store("Order",           ast::Object::atom("o1"),  &snapshot);
            let s = ast::store("Order_has_total", ast::Object::atom("50"),  &s);
            ast::store("audit_log",               ast::Object::atom("[e1]"),&s)
        };

        // Commit via the declared path — must succeed without StructuralChange.
        let target_refs: Vec<&str> = targets.iter().map(String::as_str).collect();
        let outcome = st.try_commit_declared(&snapshot, &new_d, &target_refs);
        assert!(
            matches!(outcome, CommitOutcome::Committed),
            "declared-writes path must commit without StructuralChange fallback"
        );

        // Confirm the cell contents were actually updated.
        drop(st);
        let d = peek(h).unwrap();
        assert_eq!(ast::fetch("Order",           &d), ast::Object::atom("o1"));
        assert_eq!(ast::fetch("Order_has_total", &d), ast::Object::atom("50"));
        assert_eq!(ast::fetch("audit_log",       &d), ast::Object::atom("[e1]"));
        // Untouched FT cell must be preserved.
        assert_eq!(ast::fetch("Order_has_Amount",&d), ast::Object::atom("100"));

        release_impl(h);
    }

    // ── FFI: ↓DEFS via system(h, "register:<name>", "") (#305) ─────
    // Exposes ast::register_runtime_fn to hosts (JS, wasm, browser)
    // through the system() surface. Key is "register:<name>"; input
    // is currently empty (stub body = Func::Platform(<name>), which
    // the engine dispatches via apply_platform — unknown names
    // return Bottom until a callback mechanism lands). The host's
    // job at this commit is only to mark which names it owns so
    // Citation provenance can cite them.

    #[test]
    fn system_register_key_records_name_in_runtime_registry() {
        let h = create_bare_impl();
        let result = system_impl(h, "register:send_email", "");
        assert_eq!(result, "send_email",
            "register:<name> should echo the registered name on success; got {result}");

        let d = peek(h).expect("handle must be live");
        let registry = ast::fetch("runtime_registered_names", &d);
        let names: Vec<String> = registry.as_seq()
            .map(|s| s.iter().filter_map(|o| o.as_atom().map(String::from)).collect())
            .unwrap_or_default();
        assert!(names.contains(&"send_email".to_string()),
            "runtime_registered_names must contain 'send_email' after system('register:send_email'); got {names:?}");
        release_impl(h);
    }

    #[test]
    fn system_register_key_binds_name_in_defs() {
        let h = create_bare_impl();
        let _ = system_impl(h, "register:http_fetch", "");

        let d = peek(h).expect("handle must be live");
        // apply(Func::Def("http_fetch"), ...) should resolve the binding.
        // The body is Func::Platform("http_fetch"), which apply_platform
        // falls through on (no arm for http_fetch yet) — that's a
        // callback-layer concern for a follow-up; this test only verifies
        // the DEFS entry exists.
        let def_obj = ast::fetch("http_fetch", &d);
        assert_ne!(def_obj, ast::Object::Bottom,
            "register:<name> must push a DEFS binding for the name");
        release_impl(h);
    }

    #[test]
    fn system_register_rejects_empty_name() {
        let h = create_bare_impl();
        let result = system_impl(h, "register:", "");
        assert_eq!(result, "⊥",
            "register: with empty name must return ⊥; got {result}");
        release_impl(h);
    }

    /// register:<name> with non-empty input decodes the payload as a
    /// hex-encoded freeze image of a Func-encoded Object, thaws it,
    /// metacomposes back to Func, and installs that as the body. This
    /// is what lets a host push a composable FFP body (Func::Constant,
    /// Func::Compose, etc.) rather than just marking the name as a
    /// Platform stub.
    #[test]
    fn system_register_with_hex_body_installs_composable_func() {
        let h = create_bare_impl();
        // Encode Func::Constant(atom("hello")) via func_to_object +
        // freeze + hex, matching what a JS host would do.
        let body = ast::Func::Constant(ast::Object::atom("hello"));
        let encoded_obj = ast::func_to_object(&body);
        let bytes = crate::freeze::freeze(&encoded_obj);
        let hex: String = bytes.iter().map(|b| alloc::format!("{:02x}", b)).collect();

        let result = system_impl(h, "register:greet", &hex);
        assert_eq!(result, "greet",
            "register:<name> with hex body should succeed and echo the name");

        // Dispatch via standard apply; the installed body fires.
        let tenant = tenant_lock(h).unwrap();
        let d = tenant.read().snapshot_d();
        drop(tenant);
        let out = ast::apply(&ast::Func::Def("greet".to_string()), &ast::Object::phi(), &d);
        assert_eq!(out, ast::Object::atom("hello"),
            "Func::Def('greet') should dispatch to the registered Func::Constant body");
        release_impl(h);
    }

    #[test]
    fn system_register_rejects_malformed_hex_body() {
        let h = create_bare_impl();
        let result = system_impl(h, "register:bad", "not valid hex");
        assert_eq!(result, "⊥",
            "register: with malformed hex payload must return ⊥; got {result}");
        release_impl(h);
    }

    /// re_derive FFI (#305): explicit opt-in trigger for forward-
    /// chaining after federated_ingest / register_runtime_fn bypass
    /// the create-time lfp loop. On a bare handle with zero derivation
    /// rules, re_derive is a no-op and returns "0".
    #[test]
    fn system_re_derive_returns_zero_when_no_rules_present() {
        let h = create_bare_impl();
        let result = system_impl(h, "re_derive", "");
        assert_eq!(result, "0",
            "re_derive on a bare handle must report 0 newly-derived facts; got {result}");
        release_impl(h);
    }

    /// re_derive is idempotent: re-running on a state that is already
    /// at lfp returns 0 — no new facts appear.
    #[test]
    fn system_re_derive_is_idempotent_at_lfp() {
        let h = create_bare_impl();
        let _first = system_impl(h, "re_derive", "");
        let second = system_impl(h, "re_derive", "");
        assert_eq!(second, "0",
            "second re_derive on stable state must be a no-op; got {second}");
        release_impl(h);
    }

    // ── FFI: federated_ingest: push fetched facts + Citation into P ─

    /// Full ρ(populate_n) end-to-end via FFI. The host (MCP server,
    /// Cloudflare worker) does the async HTTP fetch and maps JSON to
    /// facts, then hands the result to the engine through this key.
    #[test]
    fn system_federated_ingest_pushes_facts_and_citation() {
        let h = create_bare_impl();
        let payload = r#"{
          "externalSystem": "stripe",
          "url": "https://api.stripe.com/v1/customers",
          "retrievalDate": "2026-04-20T12:00:00Z",
          "facts": [
            {"factTypeId": "Stripe_Customer_has_Email",
             "bindings": {"Stripe Customer": "cus_1", "Email": "a@x.com"}},
            {"factTypeId": "Stripe_Customer_has_Name",
             "bindings": {"Stripe Customer": "cus_1", "Name": "Alice"}}
          ]
        }"#;

        let cite_id = system_impl(h, "federated_ingest:Stripe Customer", payload);
        assert!(cite_id.starts_with("cite:"),
            "federated_ingest should return the Citation id; got {cite_id}");

        let d = peek(h).expect("handle live");
        let uri_facts = ast::fetch("Citation_has_URI", &d).as_seq()
            .map(|s| s.to_vec()).unwrap_or_default();
        assert!(uri_facts.iter().any(|f|
            ast::binding(f, "URI") == Some("https://api.stripe.com/v1/customers")
        ), "Citation_has_URI must record the fetch URL");

        let email_cell = ast::fetch("Stripe_Customer_has_Email", &d).as_seq()
            .map(|s| s.to_vec()).unwrap_or_default();
        assert_eq!(email_cell.len(), 1);
        assert_eq!(ast::binding(&email_cell[0], "Email"), Some("a@x.com"));
        release_impl(h);
    }

    #[test]
    fn system_federated_ingest_rejects_malformed_payload() {
        let h = create_bare_impl();
        let result = system_impl(h, "federated_ingest:X", "not json");
        assert_eq!(result, "⊥",
            "federated_ingest with invalid JSON must return ⊥; got {result}");
        release_impl(h);
    }

    /// Populate cross-check (#305 #6): when a populate:{noun} config
    /// is present in D, federated_ingest must verify the payload's
    /// externalSystem matches the compiled `system`. Mismatches are
    /// rejected with ⊥ so a buggy or malicious caller can't cite a
    /// different system than the one declared for the noun.
    #[test]
    fn system_federated_ingest_rejects_system_mismatch_with_populate_config() {
        let h = create_bare_impl();
        // Install a populate:Stripe Customer config declaring system = 'stripe'.
        {
            let tenant = tenant_lock(h).expect("live handle");
            let mut st = tenant.write();
            let snapshot = st.snapshot_d();
            let config = ast::Object::seq(alloc::vec![
                ast::Object::seq(alloc::vec![ast::Object::atom("system"), ast::Object::atom("stripe")]),
                ast::Object::seq(alloc::vec![ast::Object::atom("url"), ast::Object::atom("https://api.stripe.com/v1")]),
            ]);
            let new_d = ast::store("populate:Stripe Customer", ast::func_to_object(&ast::Func::constant(config)), &snapshot);
            st.replace_d(new_d);
        }

        let payload = r#"{
          "externalSystem": "evil.com",
          "url": "https://api.stripe.com/v1/customers",
          "retrievalDate": "2026-04-20T12:00:00Z",
          "facts": []
        }"#;
        let result = system_impl(h, "federated_ingest:Stripe Customer", payload);
        assert_eq!(result, "⊥",
            "externalSystem != populate config's system must return ⊥; got {result}");
        release_impl(h);
    }

    #[test]
    fn system_federated_ingest_accepts_system_match_with_populate_config() {
        let h = create_bare_impl();
        {
            let tenant = tenant_lock(h).expect("live handle");
            let mut st = tenant.write();
            let snapshot = st.snapshot_d();
            let config = ast::Object::seq(alloc::vec![
                ast::Object::seq(alloc::vec![ast::Object::atom("system"), ast::Object::atom("stripe")]),
                ast::Object::seq(alloc::vec![ast::Object::atom("url"), ast::Object::atom("https://api.stripe.com/v1")]),
            ]);
            let new_d = ast::store("populate:Stripe Customer", ast::func_to_object(&ast::Func::constant(config)), &snapshot);
            st.replace_d(new_d);
        }

        let payload = r#"{
          "externalSystem": "stripe",
          "url": "https://api.stripe.com/v1/customers",
          "retrievalDate": "2026-04-20T12:00:00Z",
          "facts": []
        }"#;
        let result = system_impl(h, "federated_ingest:Stripe Customer", payload);
        assert!(result.starts_with("cite:"),
            "matching system should succeed and return a citation id; got {result}");
        release_impl(h);
    }
}
