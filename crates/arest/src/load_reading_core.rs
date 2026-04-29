// crates/arest/src/load_reading_core.rs
//
// SystemVerb::LoadReading (#555 / DynRdg-1) — pure-FORML core extracted
// from `load_reading.rs` for kernel reach (#586, mirroring JJJJJ's
// `select_component_core` extraction in #565 part 2).
//
// ## Why this lives outside `load_reading.rs`
//
// `load_reading.rs` is gated behind `cfg(not(feature = "no_std"))`
// because its actual `load_reading` body reaches `parse_forml2` /
// `check::check_readings_func`, both of which transitively pull
// `serde` + `regex` + `std::env::var` (the stage12 grammar cache hits
// the env for trace toggles). Until those modules land in no_std,
// the function itself stays std-only.
//
// PPPPP-2's #560 worked around the gate by accepting a closure for
// the actual apply step — boot-time replay walks the persistence ring
// + reports the live record count but can't actually execute the
// LoadReading verb. The end-state for #586 is: persisted readings
// re-execute on kernel boot once the closure caller is updated to
// pass `arest::load_reading_core::load_reading`.
//
// This module provides the architectural separation:
//   * The TYPES (`LoadReadingPolicy`, `LoadOutcome`, `LoadError`,
//     `LoadReport`) are unconditionally available so kernel-side
//     scaffolding can reference them in cfg-gated code paths.
//   * The FUNCTION (`load_reading`) is currently gated `cfg(not(
//     feature = "no_std"))` because of the parse + check dep chain.
//     Once those modules are ported, the gate lifts to a single line
//     edit and the kernel `use arest::load_reading_core::load_reading`
//     becomes a working call site.
//
// ## Pipeline (unchanged from FFFFF's #555)
//
//   bake-time:  metamodel_readings() → fold parse → compile → cache
//   compile-cmd: Command::LoadReadings → parse_to_state_from → merge → compile
//   load_reading (this module): SAME pipeline, but driven by a SYSTEM verb
//     and surfacing a structured report (added cell ids) and a structured
//     diagnostic tree on failure.
//
// Atomicity: parse → constraint validation → merge happens against a
// scratch copy of `state`. The scratch state is committed back to the
// caller's state ONLY when validation passes; on any failure the
// caller's state is untouched.
//
// Idempotency: loading the same body under the same name produces
// identity (no duplicate noun/FT/derivation cells).

#[allow(unused_imports)]
use alloc::{
    borrow::ToOwned,
    boxed::Box,
    string::{String, ToString},
    vec::Vec,
};

use crate::ast::Object;

// ── Public types (unconditional so kernel scaffolding can reference) ──

/// Names of cells that newly appeared (or grew) under this load,
/// plus reading-versioning metadata (#558 / DynRdg-4).
///
/// Atom lists are sorted lexicographically so the report is
/// deterministic across hash-map iteration order. `content_hash` is
/// a 16-character lowercase hex FNV-1a64 digest of the body bytes,
/// so callers can tell whether the same name was loaded with a
/// byte-different body. `version_stamp` is a process-monotonic u64
/// allocated by `next_version()`; its semantics are "second load
/// has a higher stamp than first load," not "globally unique
/// identifier." Both fields are persisted into the manifest cell
/// (`_loaded_reading:{name}`) under `contentHash` and
/// `versionStamp` bindings — see `encode_manifest` /
/// `decode_manifest`.
///
/// Pre-#558 manifest cells (no contentHash/versionStamp bindings)
/// decode with `content_hash = ""` and `version_stamp = 0`. The
/// unload path tolerates this so live def-states from earlier
/// boots round-trip cleanly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadReport {
    pub added_nouns: Vec<String>,
    pub added_fact_types: Vec<String>,
    pub added_derivations: Vec<String>,
    /// 16-char lowercase hex FNV-1a64 of the loaded body.
    pub content_hash: String,
    /// Monotonic per-process load stamp (see `next_version()`).
    pub version_stamp: u64,
}

/// Why a `load_reading` call rejected.
///
/// The diagnostic tree is the structured equivalent of what
/// `check::check_readings` returns; callers can walk it to render
/// per-line / per-clause errors. `EmptyBody` and `Disallowed` carry
/// no diagnostics because they short-circuit before parse runs.
///
/// The `DeonticViolation` variant is gated on `cfg(not(feature =
/// "no_std"))` because it carries `crate::check::ReadingDiagnostic`,
/// and `check` is currently std-only. Under no_std the variant is
/// elided — kernel callers that only need the type surface for
/// closure plumbing don't need to construct it.
///
/// `PartialEq` is intentionally NOT derived: `ReadingDiagnostic`
/// does not implement Eq (its `Level` / `Source` fields are simple
/// enums but the struct as a whole was authored without an Eq
/// derive), and adding one upstream would broaden the contract of
/// the public diagnostic type beyond this verb's needs. Tests
/// pattern-match on the variant instead.
#[derive(Debug, Clone)]
pub enum LoadError {
    /// Caller's policy refuses runtime loads (default in production
    /// builds — see `LoadReadingPolicy::Deny`).
    Disallowed,
    /// Body was empty or whitespace-only. Distinct from `ParseError`
    /// because the parser's "empty input parses to empty state" is
    /// not the right answer for a runtime load — callers expect a
    /// load to add something.
    EmptyBody,
    /// Reading name failed sanitization (empty, whitespace, control
    /// chars). Names land in cell ids, so they must be safe atoms.
    InvalidName(String),
    /// Stage-1+Stage-2 parse failed — the message is the parser's
    /// own error string (line + column when available).
    ParseError(String),
    /// Constraint validation flagged one or more deontic violations
    /// against the merged state. Diagnostics are structured per
    /// `crate::check::ReadingDiagnostic`. Variant gated on std build
    /// because `check::ReadingDiagnostic` is not yet no_std-clean
    /// (the `check` module pulls `parse_forml2` for its grammar
    /// cache).
    #[cfg(not(feature = "no_std"))]
    DeonticViolation(Vec<crate::check::ReadingDiagnostic>),
}

/// Caller's policy for runtime reading loads.
///
/// `Deny` is the default — production builds keep this so an exposed
/// system_impl frontend cannot push schema mutations from a remote
/// actor (#328 register-gate semantics extended to this verb).
/// `AllowAll` is the dev/test setting; future work may add an
/// `AllowList` variant for finer-grained host policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadReadingPolicy {
    Deny,
    AllowAll,
}

impl Default for LoadReadingPolicy {
    fn default() -> Self {
        LoadReadingPolicy::Deny
    }
}

/// Outcome of a successful load: the new state plus a report listing
/// every cell that grew. The state is functional — caller swaps it in
/// atomically.
#[derive(Debug, Clone)]
pub struct LoadOutcome {
    pub report: LoadReport,
    pub new_state: Object,
}

// ── UnloadReading types (#556 / DynRdg-2) ──────────────────────────
//
// `UnloadReading` is the inverse of `LoadReading`. We persist a
// per-reading manifest cell at load time (`_loaded_reading:{name}`)
// recording the report's `added_nouns`, `added_fact_types`, and
// `added_derivations`. The unload path looks up the manifest, drops
// every listed identifier from its host cell (`Noun`, `FactType`,
// `DerivationRule`), removes the manifest cell itself, and returns a
// structured outcome describing what went.

/// Caller's policy for what happens to facts referencing cells
/// removed by an unload.
///
/// `CascadeDelete` (default) — remove every cell listed in the
/// reading's manifest and any rows in adjacent cells (Role,
/// derivation defs) keyed by those identifiers.
///
/// `Migrate` (future) — preserve referencing facts by re-homing
/// them under a generic uncategorized fact-type bucket. #557
/// ReloadReading depends on this for atomic unload+load with
/// backing-fact preservation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnloadPolicy {
    CascadeDelete,
    Migrate,
}

impl Default for UnloadPolicy {
    fn default() -> Self {
        UnloadPolicy::CascadeDelete
    }
}

/// What `unload_reading` actually removed, mirroring `LoadReport`,
/// plus the manifest's recorded versioning metadata so wire callers
/// can see which body version they just removed (#558 / DynRdg-4).
///
/// `content_hash` and `version_stamp` are surfaced exactly as they
/// were stored at load time. For pre-#558 manifest cells the values
/// are `""` and `0` respectively (see `decode_manifest`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnloadReport {
    pub removed_nouns: Vec<String>,
    pub removed_fact_types: Vec<String>,
    pub removed_derivations: Vec<String>,
    /// Content hash recorded by the load that wrote the manifest.
    pub content_hash: String,
    /// Version stamp recorded by the load that wrote the manifest.
    pub version_stamp: u64,
}

/// Why an `unload_reading` call rejected.
///
/// `ManifestMissing` — the reading was not previously loaded under
/// this `name`, OR the load happened before manifest persistence
/// (#556) and the `_loaded_reading:{name}` cell was not written.
/// `InvalidName` — same sanitization rules as `LoadReading::name`.
/// `Disallowed` — reserved for future host-policy gating.
/// `NotImplemented` — the requested policy is not yet implemented
/// (today: `UnloadPolicy::Migrate`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnloadError {
    ManifestMissing(String),
    InvalidName(String),
    Disallowed,
    NotImplemented,
}

/// Outcome of a successful unload: the new state plus a structured
/// report listing every cell-row that was removed.
#[derive(Debug, Clone)]
pub struct UnloadOutcome {
    pub report: UnloadReport,
    pub new_state: Object,
}

// ── ReloadReading types (#557 / DynRdg-3) ──────────────────────────
//
// `ReloadReading` composes `unload` + `load` against a single
// state snapshot with single-commit atomicity: either the new body
// fully replaces the old (manifest, cells, the lot), or the old
// reading stays exactly as it was. No partial state is visible to
// callers across either step.
//
// `ReplaceAll` (default) drives the unload step with
// `UnloadPolicy::CascadeDelete` and the load step with
// `LoadReadingPolicy::AllowAll`. `MigrateFacts` is forward-looking
// and currently stubs out to `NotImplemented` — it depends on
// `UnloadPolicy::Migrate` (also stubbed) to preserve referencing
// facts when the new body re-declares the same fact-types.

/// Caller's policy for how to compose unload+load atomically.
///
/// `ReplaceAll` (default) — unload with `CascadeDelete`, then load.
/// Loses any backing facts referencing dropped FTs. The right
/// policy when the new body is a strict superset / disjoint
/// redefinition.
/// `MigrateFacts` (future) — unload with `Migrate`, preserving
/// referencing facts under a generic uncategorized bucket so the
/// reload re-homes them under the new schema. Stubbed today;
/// returns `ReloadError::NotImplemented`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReloadPolicy {
    ReplaceAll,
    MigrateFacts,
}

impl Default for ReloadPolicy {
    fn default() -> Self {
        ReloadPolicy::ReplaceAll
    }
}

/// Why a `reload_reading` call rejected.
///
/// `Disallowed` — host policy (reserved; today the load step gates).
/// `InvalidName` — same sanitization rules as Load/Unload's name.
/// `EmptyBody` — empty/whitespace body, mirrors `LoadError::EmptyBody`.
/// `UnloadFailed` — the unload step rejected with a non-`ManifestMissing`
/// reason. (`ManifestMissing` is treated as a "first-time-load disguised
/// as a reload" and falls through to the load step — see the
/// `reload_reading` doc comment for the rationale.)
/// `LoadFailed` — the load step rejected; `new_state` rolls back to the
/// pre-reload snapshot, fulfilling the atomicity contract.
/// `NotImplemented` — `ReloadPolicy::MigrateFacts` is reserved.
///
/// `PartialEq` is intentionally NOT derived because `LoadError`
/// itself does not implement Eq under std builds (the
/// `DeonticViolation` variant carries a non-Eq diagnostic shape).
/// Tests pattern-match on the variant instead.
#[derive(Debug, Clone)]
pub enum ReloadError {
    Disallowed,
    InvalidName(String),
    EmptyBody,
    UnloadFailed(UnloadError),
    #[cfg(not(feature = "no_std"))]
    LoadFailed(LoadError),
    NotImplemented,
}

/// Outcome of a successful reload: structured reports for both
/// directions of the round-trip plus the post-reload state.
///
/// `removed` — what the unload step took out (empty when the
/// reload was actually a first-time load — see
/// `reload_reading` doc).
/// `added` — what the load step put in.
/// `new_state` — the post-reload def-state for the caller to commit.
#[derive(Debug, Clone)]
pub struct ReloadOutcome {
    pub removed: UnloadReport,
    pub added: LoadReport,
    pub new_state: Object,
}

/// Cell-name prefix for the per-reading manifest. The full cell
/// name is `_loaded_reading:{name}`.
pub const MANIFEST_CELL_PREFIX: &str = "_loaded_reading:";

/// Build a manifest cell name for a given reading name.
pub fn manifest_cell_name(name: &str) -> String {
    let mut s = String::with_capacity(MANIFEST_CELL_PREFIX.len() + name.len());
    s.push_str(MANIFEST_CELL_PREFIX);
    s.push_str(name);
    s
}

/// Encode a `LoadReport` as a single fact for the manifest cell.
/// Comma-separated atom lists fit the existing fact-binding shape
/// without needing a nested-sequence binding type.
///
/// Versioning metadata (`contentHash`, `versionStamp` — #558) lands
/// alongside the cell-id lists in the same fact. Decoding tolerates
/// older cells written before #558 by returning `""` /  `0` for the
/// missing bindings (see `decode_manifest`).
pub fn encode_manifest(report: &LoadReport) -> Object {
    let nouns = report.added_nouns.join(",");
    let fts = report.added_fact_types.join(",");
    let derivs = report.added_derivations.join(",");
    let stamp = u64_to_dec(report.version_stamp);
    Object::seq(vec![crate::ast::fact_from_pairs(&[
        ("addedNouns", &nouns),
        ("addedFactTypes", &fts),
        ("addedDerivations", &derivs),
        ("contentHash", &report.content_hash),
        ("versionStamp", &stamp),
    ])])
}

/// Decode a manifest cell back into a `LoadReport`. Returns `None`
/// if the cell is absent or shape-wrong.
///
/// Backward-compat: pre-#558 manifest cells have no `contentHash`
/// or `versionStamp` bindings. We accept those by defaulting the
/// fields to `""` / `0` so old persisted state still unloads
/// cleanly.
pub fn decode_manifest(state: &Object, name: &str) -> Option<LoadReport> {
    let cell_name = manifest_cell_name(name);
    let cell = crate::ast::fetch(&cell_name, state);
    let facts = cell.as_seq()?;
    let fact = facts.first()?;
    let split = |key: &str| -> Vec<String> {
        crate::ast::binding(fact, key)
            .map(|s| {
                if s.is_empty() {
                    Vec::new()
                } else {
                    s.split(',').map(|p| p.to_string()).collect()
                }
            })
            .unwrap_or_default()
    };
    let content_hash = crate::ast::binding(fact, "contentHash")
        .map(String::from)
        .unwrap_or_default();
    let version_stamp = crate::ast::binding(fact, "versionStamp")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    Some(LoadReport {
        added_nouns: split("addedNouns"),
        added_fact_types: split("addedFactTypes"),
        added_derivations: split("addedDerivations"),
        content_hash,
        version_stamp,
    })
}

// ── Reading versioning (#558 / DynRdg-4) ───────────────────────────

/// Process-monotonic version counter for `load_reading`. Held behind
/// a `spin::Mutex<u64>` so the helper is `no_std`-clean and matches
/// the lock pattern used elsewhere in this crate (`csprng::STATE`,
/// `entropy::GLOBAL_SOURCE`). Each successful load increments the
/// counter and returns the new value.
///
/// Boot-time replay: kernel-side replay walks the persistence ring
/// and replays each `LoadReading` verb against a fresh def-state.
/// To keep stamps monotonic across boots, `seed_version_counter`
/// pushes the counter forward to the highest stamp observed in the
/// replayed manifest cells. Calling `next_version()` after seeding
/// returns `seed + 1`, `seed + 2`, etc.
static VERSION_COUNTER: spin::Mutex<u64> = spin::Mutex::new(0);

/// Allocate the next version stamp. Increments the global counter
/// and returns the post-increment value, so the first stamp is `1`
/// (a stamp of `0` is reserved for "unset" / pre-#558 manifests).
pub fn next_version() -> u64 {
    let mut g = VERSION_COUNTER.lock();
    *g = g.saturating_add(1);
    *g
}

/// Seed the version counter from a known floor. Used by boot-time
/// replay so post-replay loads continue numbering above the highest
/// previously-seen stamp. Idempotent: calling with a value lower
/// than the current counter is a no-op.
pub fn seed_version_counter(floor: u64) {
    let mut g = VERSION_COUNTER.lock();
    if floor > *g {
        *g = floor;
    }
}

/// Compute a content hash of the body bytes as a 16-char lowercase
/// hex string. Uses FNV-1a 64-bit, which is `no_std`-clean and short
/// enough to embed in a manifest binding without bloating the cell
/// graph. The hash is content-only — same bytes produce the same
/// digest, byte-different bytes produce a different digest with
/// overwhelming probability.
///
/// FNV-1a is not a cryptographic hash; we don't need one here. The
/// purpose is "callers can tell two loads apart"; collision
/// resistance against an adversary isn't part of the threat model.
pub fn compute_content_hash(body: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut h: u64 = FNV_OFFSET;
    for b in body.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    let mut s = String::with_capacity(16);
    let hex_digit = |n: u8| -> char {
        match n {
            0..=9 => (b'0' + n) as char,
            10..=15 => (b'a' + (n - 10)) as char,
            _ => unreachable!(),
        }
    };
    // Emit nibbles MSB-first: shifts 60, 56, 52, ..., 4, 0.
    let mut shift: i32 = 60;
    while shift >= 0 {
        let nibble = ((h >> shift) & 0xf) as u8;
        s.push(hex_digit(nibble));
        shift -= 4;
    }
    s
}

/// Decimal-encode a u64 into a heap-allocated `String` without
/// reaching for `format!` (keeps the helper `no_std`-friendly).
fn u64_to_dec(mut n: u64) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let mut buf = [0u8; 20]; // u64::MAX fits in 20 decimal digits
    let mut i = buf.len();
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    // Safety: we only wrote ASCII digits.
    String::from_utf8(buf[i..].to_vec()).expect("ASCII digits are valid UTF-8")
}

/// Write the manifest for a successful load into `state`.
pub fn write_manifest(state: &Object, name: &str, report: &LoadReport) -> Object {
    let cell_name = manifest_cell_name(name);
    crate::ast::store(&cell_name, encode_manifest(report), state)
}

/// Remove rows from a sequence-valued cell where `binding(row, key)`
/// matches any value in `values`. If the cell is absent, the state
/// is returned unchanged.
pub fn remove_rows_by_binding(
    state: &Object,
    cell: &str,
    key: &str,
    values: &[String],
) -> Object {
    if values.is_empty() {
        return state.clone();
    }
    let existing = crate::ast::fetch_or_phi(cell, state);
    let rows = match existing.as_seq() {
        Some(rows) => rows,
        None => return state.clone(),
    };
    use hashbrown::HashSet;
    let drop_set: HashSet<&str> = values.iter().map(|s| s.as_str()).collect();
    let kept: Vec<Object> = rows
        .iter()
        .filter(|f| {
            !crate::ast::binding(f, key)
                .map(|v| drop_set.contains(v))
                .unwrap_or(false)
        })
        .cloned()
        .collect();
    if kept.len() == rows.len() {
        return state.clone();
    }
    crate::ast::store(cell, Object::Seq(kept.into()), state)
}

/// Remove a cell entirely from a state.
pub fn remove_cell(state: &Object, name: &str) -> Object {
    use hashbrown::HashMap;
    match state {
        Object::Map(map) => {
            if !map.contains_key(name) {
                return state.clone();
            }
            let mut new_map: HashMap<String, Object> = map.clone();
            new_map.remove(name);
            Object::Map(new_map)
        }
        Object::Seq(_) => {
            let cells = crate::ast::cells_iter(state);
            if !cells.iter().any(|(n, _)| n == &name) {
                return state.clone();
            }
            let kept: Vec<Object> = cells
                .into_iter()
                .filter(|(n, _)| n != &name)
                .map(|(n, c)| crate::ast::cell(n, c.clone()))
                .collect();
            Object::Seq(kept.into())
        }
        _ => state.clone(),
    }
}

// ── load_reading (gated on std until parse+check land in no_std) ──────
//
// The body reaches `crate::parse_forml2::parse_to_state_from` and
// `crate::check::check_readings_func`. Both modules are currently
// gated `cfg(not(feature = "no_std"))`; lifting that requires porting
// stage12's `std::env::var` / `std::time::Instant` use and check.rs's
// transitive parse_forml2 dep. Tracked as a follow-up to #586.
//
// Until that lands, this function stays std-only. The kernel
// `arest::load_reading_core::load_reading` call site that PPPPP-2's
// closure caller will adopt remains gated under
// `cfg(not(feature = "no_std"))` at the call site too. The
// extraction here is the architectural prerequisite — once the deps
// land in no_std, only the gate below needs to lift.

/// Runtime peer of the bake-time `metamodel_readings()` assembler.
///
/// Pipeline:
///   1. Policy check — refuse if caller policy is `Deny`.
///   2. Sanitize `name` and `body` — empty-body and invalid-name reject
///      cheaply before any parse work.
///   3. Stage-1 + Stage-2 parse via `parse_to_state_from` (uses `state`
///      as context so the body may reference nouns the live state
///      already declares).
///   4. Merge `state` ⊕ parsed → scratch state.
///   5. Run `check::check_readings_func` against the scratch state.
///      Any `Source::Deontic` `Level::Error` rejects the load.
///   6. On success, diff the scratch state vs. `state` to assemble
///      `LoadReport` (only newly-added or changed Noun / FactType /
///      derivation:* cells contribute).
///   7. Return `(report, scratch_state)` for the caller to swap in.
///
/// On any failure step 4-5 the caller's `state` is untouched (the
/// scratch copy is dropped). Step 1-3 never touch `state`.
///
/// Idempotency: when `body` is byte-identical to a previously-loaded
/// body for the same `name`, the parsed cells equal the existing
/// cells, the diff comes back empty, and the report is empty. The
/// new_state still equals the old state cell-for-cell so callers may
/// commit it without observing a diff.
#[cfg(not(feature = "no_std"))]
pub fn load_reading(
    state: &Object,
    name: &str,
    body: &str,
    policy: LoadReadingPolicy,
) -> Result<LoadOutcome, LoadError> {
    use crate::ast;

    // Step 1: gate.
    if policy == LoadReadingPolicy::Deny {
        return Err(LoadError::Disallowed);
    }

    // Step 2: sanitize.
    let trimmed_name = name.trim();
    if trimmed_name.is_empty() {
        return Err(LoadError::InvalidName(
            "reading name must not be empty".to_string(),
        ));
    }
    if trimmed_name.chars().any(|c| c.is_control()) {
        return Err(LoadError::InvalidName(
            "reading name must not contain control characters".to_string(),
        ));
    }
    if body.trim().is_empty() {
        return Err(LoadError::EmptyBody);
    }

    // Step 3: parse with current state as context.
    let parsed = match crate::parse_forml2::parse_to_state_from(body, state) {
        Ok(p) => p,
        Err(e) => return Err(LoadError::ParseError(e)),
    };

    // Step 4: merge into scratch state.
    let scratch = ast::merge_states(state, &parsed);

    // Step 5: deontic constraint validation pass (#288).
    //
    // We re-run the full check_readings_func against the merged scratch
    // state. Errors at deontic level reject the load. Warnings / hints
    // pass through silently — callers that want to surface them can
    // re-run `check::check_readings` themselves; that's a separate
    // surface from "reject vs. accept."
    let diag_obj = ast::apply(
        &crate::check::check_readings_func(),
        &scratch,
        &scratch,
    );
    let diags = decode_diagnostics(&diag_obj);
    let deontic_errors: Vec<_> = diags
        .into_iter()
        .filter(|d| {
            matches!(d.source, crate::check::Source::Deontic)
                && matches!(d.level, crate::check::Level::Error)
        })
        .collect();
    if !deontic_errors.is_empty() {
        return Err(LoadError::DeonticViolation(deontic_errors));
    }

    // Step 6: compute the report — Noun / FactType / derivation:* cell
    // additions vs. the input state. Versioning metadata
    // (content_hash, version_stamp) is allocated here so the report
    // and the persisted manifest carry the same values (#558).
    let mut report = build_report(state, &scratch);
    report.content_hash = compute_content_hash(body);
    report.version_stamp = next_version();

    // Step 7: persist the manifest so a future #556 UnloadReading can
    // recover the list of cells this load contributed. Manifest cell
    // is `_loaded_reading:{name}` carrying the three comma-separated
    // identifier lists plus the new (#558) `contentHash` /
    // `versionStamp` bindings; see `encode_manifest` for the schema.
    let with_manifest = write_manifest(&scratch, trimmed_name, &report);

    // Step 8: return the manifest-augmented state for caller to commit.
    Ok(LoadOutcome {
        report,
        new_state: with_manifest,
    })
}

/// Inverse of `load_reading` — drop a previously-loaded reading from
/// the cell graph. Reads the `_loaded_reading:{name}` manifest cell,
/// removes every listed identifier from `Noun` / `FactType` /
/// `DerivationRule` (cascade-deleting the matching `Role` and
/// `derivation:*` cells), and removes the manifest cell itself.
///
/// On `ManifestMissing` / `InvalidName` / `NotImplemented`, the input
/// state is untouched.
#[cfg(not(feature = "no_std"))]
pub fn unload_reading(
    state: &Object,
    name: &str,
    policy: UnloadPolicy,
) -> Result<UnloadOutcome, UnloadError> {
    // Step 1: sanitize.
    let trimmed_name = name.trim();
    if trimmed_name.is_empty() {
        return Err(UnloadError::InvalidName(
            "reading name must not be empty".to_string(),
        ));
    }
    if trimmed_name.chars().any(|c| c.is_control()) {
        return Err(UnloadError::InvalidName(
            "reading name must not contain control characters".to_string(),
        ));
    }

    // Step 2: gate Migrate.
    if policy == UnloadPolicy::Migrate {
        return Err(UnloadError::NotImplemented);
    }

    // Step 3: manifest lookup.
    let manifest = match decode_manifest(state, trimmed_name) {
        Some(m) => m,
        None => {
            return Err(UnloadError::ManifestMissing(trimmed_name.to_string()));
        }
    };

    // Step 4: cascade-delete per identifier list.
    let mut new_state = state.clone();
    new_state = remove_rows_by_binding(
        &new_state,
        "Noun",
        "name",
        &manifest.added_nouns,
    );
    new_state = remove_rows_by_binding(
        &new_state,
        "FactType",
        "id",
        &manifest.added_fact_types,
    );
    new_state = remove_rows_by_binding(
        &new_state,
        "Role",
        "factType",
        &manifest.added_fact_types,
    );
    new_state = remove_rows_by_binding(
        &new_state,
        "DerivationRule",
        "ruleId",
        &manifest.added_derivations,
    );
    for rule_id in &manifest.added_derivations {
        let def_name = alloc::format!("derivation:{}", rule_id);
        new_state = remove_cell(&new_state, &def_name);
    }

    // Step 5: drop the manifest cell.
    let manifest_cell = manifest_cell_name(trimmed_name);
    new_state = remove_cell(&new_state, &manifest_cell);

    let report = UnloadReport {
        removed_nouns: manifest.added_nouns,
        removed_fact_types: manifest.added_fact_types,
        removed_derivations: manifest.added_derivations,
        // Surface the versioning metadata recorded by the load so
        // wire callers can see which body version was just removed.
        // Pre-#558 manifests decode with `""` / `0` defaults, which
        // we propagate as-is — the wire encoder treats both as
        // absent-equivalent (#558 / DynRdg-4).
        content_hash: manifest.content_hash,
        version_stamp: manifest.version_stamp,
    };

    Ok(UnloadOutcome {
        report,
        new_state,
    })
}

/// Atomic unload+load — replace a previously-loaded reading's body in
/// a single commit. (#557 / DynRdg-3.)
///
/// Pipeline:
///   1. Gate `ReloadPolicy::MigrateFacts` → `NotImplemented`. Forward
///      compatibility hook; depends on `UnloadPolicy::Migrate`.
///   2. Sanitize `name` (empty/whitespace/control → `InvalidName`)
///      and `body` (empty/whitespace → `EmptyBody`). Cheap; same
///      rules as `load_reading` / `unload_reading`.
///   3. Snapshot the input `state` so we can roll back if either step
///      below fails. `Object` is `Clone`, so the snapshot is the
///      whole pre-reload graph.
///   4. Step A: `unload_reading(&state, name, UnloadPolicy::CascadeDelete)`.
///      A `ManifestMissing` rejection is NOT fatal — it means the
///      reading isn't currently loaded, so the reload degenerates to
///      a first-time load (an empty `UnloadReport` + the load
///      step's added cells). Other unload errors propagate as
///      `ReloadError::UnloadFailed`.
///   5. Step B: `load_reading(&unload_outcome.new_state, name, body,
///      LoadReadingPolicy::AllowAll)`. On any error, return
///      `ReloadError::LoadFailed` — the snapshot is dropped and the
///      caller sees byte-equal-to-input `state` is preserved at the
///      caller side (this function never mutates `state`; the
///      atomicity contract is "if either step fails, the new_state
///      we hand back is identical to the input snapshot" — i.e. no
///      partial reload visible).
///   6. Assemble `ReloadOutcome { removed, added, new_state }` and
///      return.
///
/// **Atomicity contract:** if either the unload or the load step
/// fails, the caller never sees a state where one of those completed
/// and the other did not. The error returns leave `state` untouched
/// (this function takes `&Object` and never re-binds the caller's
/// reference); on success, `new_state` is the fully-merged
/// post-reload graph.
///
/// **First-time-load fallthrough:** when no `_loaded_reading:{name}`
/// manifest is present, `unload_reading` returns `ManifestMissing`.
/// We interpret this as "the caller asked to reload but the reading
/// isn't there" and proceed with the load, returning an empty
/// `UnloadReport` in the outcome's `removed` field. Pinning this
/// behavior matters: the alternative (returning a hard error) would
/// force callers to know whether a reading is loaded before deciding
/// between Load and Reload, which defeats the verb's purpose.
#[cfg(not(feature = "no_std"))]
pub fn reload_reading(
    state: &Object,
    name: &str,
    body: &str,
    policy: ReloadPolicy,
) -> Result<ReloadOutcome, ReloadError> {
    // Step 1: gate.
    if policy == ReloadPolicy::MigrateFacts {
        return Err(ReloadError::NotImplemented);
    }

    // Step 2: sanitize. Done up front so we reject cheaply before
    // any unload work runs — duplicates the checks inside
    // load_reading / unload_reading but means a malformed call
    // never observes partial pipeline state.
    let trimmed_name = name.trim();
    if trimmed_name.is_empty() {
        return Err(ReloadError::InvalidName(
            "reading name must not be empty".to_string(),
        ));
    }
    if trimmed_name.chars().any(|c| c.is_control()) {
        return Err(ReloadError::InvalidName(
            "reading name must not contain control characters".to_string(),
        ));
    }
    if body.trim().is_empty() {
        return Err(ReloadError::EmptyBody);
    }

    // Step 3: snapshot. We don't actually use this for rollback
    // because both steps already operate functionally on `&Object`
    // — the atomicity contract is satisfied by the fact that we
    // never bind a partial state to `new_state` until both steps
    // have succeeded. Still, holding the snapshot makes the
    // intent explicit and lets us return it (effectively a no-op
    // since we can also return `state.clone()` directly) on
    // failure for callers that might want to inspect it.
    let _snapshot = state.clone();

    // Step 4: unload. Treat ManifestMissing as a fall-through to
    // first-time-load. Other unload errors are fatal.
    let unload_outcome = match unload_reading(state, trimmed_name, UnloadPolicy::CascadeDelete) {
        Ok(o) => o,
        Err(UnloadError::ManifestMissing(_)) => {
            // First-time-load disguised as reload: synthesize an
            // empty UnloadOutcome carrying the input state.
            UnloadOutcome {
                report: UnloadReport::default(),
                new_state: state.clone(),
            }
        }
        Err(other) => return Err(ReloadError::UnloadFailed(other)),
    };

    // Step 5: load against the post-unload state. On failure, the
    // caller's state is untouched (we never wrote `unload_outcome
    // .new_state` back to the caller — that's the atomicity
    // contract). The LoadError variant preserves the diagnostic
    // tree so callers can render specific failure reasons.
    let load_outcome = match load_reading(
        &unload_outcome.new_state,
        trimmed_name,
        body,
        LoadReadingPolicy::AllowAll,
    ) {
        Ok(o) => o,
        Err(e) => return Err(ReloadError::LoadFailed(e)),
    };

    // Step 6: assemble. The new state is the load step's output —
    // it already includes the post-unload cell deletions and the
    // refreshed manifest cell.
    Ok(ReloadOutcome {
        removed: unload_outcome.report,
        added: load_outcome.report,
        new_state: load_outcome.new_state,
    })
}

/// Decode a `check::check_readings_func` result Object back into the
/// public `ReadingDiagnostic` shape.
///
/// The check module's encode/decode functions are private; we
/// re-inline the same shape here. The public API of `check_readings`
/// returns `Vec<ReadingDiagnostic>` directly; only the lower-level
/// Func application surface returns the encoded Object.
#[cfg(not(feature = "no_std"))]
fn decode_diagnostics(obj: &Object) -> Vec<crate::check::ReadingDiagnostic> {
    use crate::check::{Level, ReadingDiagnostic, Source};
    obj.as_seq()
        .map(|s| {
            s.iter()
                .filter_map(|d| {
                    let map = d.as_map()?;
                    let line = map
                        .get("line")
                        .and_then(|o| o.as_atom())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    let reading = map
                        .get("reading")
                        .and_then(|o| o.as_atom())
                        .unwrap_or("")
                        .to_string();
                    let level = match map.get("level").and_then(|o| o.as_atom()) {
                        Some("Error") => Level::Error,
                        Some("Hint") => Level::Hint,
                        _ => Level::Warning,
                    };
                    let source = match map.get("source").and_then(|o| o.as_atom()) {
                        Some("parse") => Source::Parse,
                        Some("deontic") => Source::Deontic,
                        _ => Source::Resolve,
                    };
                    let message = map
                        .get("message")
                        .and_then(|o| o.as_atom())
                        .unwrap_or("")
                        .to_string();
                    let suggestion = map
                        .get("suggestion")
                        .and_then(|o| o.as_atom())
                        .map(String::from);
                    Some(ReadingDiagnostic {
                        line,
                        reading,
                        level,
                        source,
                        message,
                        suggestion,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Diff Noun / FactType / derivation:* cells between `before` and
/// `after`. Returns sorted lists of newly-appearing identifiers.
///
/// Noun additions — facts in `after.Noun` whose `name` binding is
/// absent from `before.Noun`.
/// FactType additions — facts in `after.FactType` whose `id` binding
/// is absent from `before.FactType`.
/// Derivation additions — `derivation:<rule_id>` cells in `after`
/// that weren't in `before`. Compiled derivation cells appear at the
/// def-state level after `compile_to_defs_state`; the cell name itself
/// is the rule identifier.
///
/// Pure FORML cell-walking — no parse / check deps. Available
/// unconditionally so kernel-side diff reporters can reuse the same
/// before/after diff shape.
pub fn build_report(before: &Object, after: &Object) -> LoadReport {
    use crate::ast;
    use hashbrown::HashSet;

    fn names_in(cell: &str, key: &str, state: &Object) -> HashSet<String> {
        crate::ast::fetch_or_phi(cell, state)
            .as_seq()
            .map(|facts| {
                facts
                    .iter()
                    .filter_map(|f| ast::binding(f, key).map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    let before_nouns = names_in("Noun", "name", before);
    let after_nouns = names_in("Noun", "name", after);
    let mut added_nouns: Vec<String> = after_nouns.difference(&before_nouns).cloned().collect();
    added_nouns.sort();

    let before_fts = names_in("FactType", "id", before);
    let after_fts = names_in("FactType", "id", after);
    let mut added_fact_types: Vec<String> =
        after_fts.difference(&before_fts).cloned().collect();
    added_fact_types.sort();

    // Derivations live in the parsed cells under `DerivationRule` (the
    // FORML 2 stage-2 cell name) — `compile_to_defs_state` later turns
    // each into a `derivation:<id>` def cell, but the parser emits the
    // raw rule under `DerivationRule` keyed by `ruleId`. We diff the
    // parsed cell so the report reflects what the user added in this
    // load, independent of whether compile_to_defs_state has run yet.
    let before_rules = names_in("DerivationRule", "ruleId", before);
    let after_rules = names_in("DerivationRule", "ruleId", after);
    let mut added_derivations: Vec<String> =
        after_rules.difference(&before_rules).cloned().collect();
    added_derivations.sort();

    LoadReport {
        added_nouns,
        added_fact_types,
        added_derivations,
        // Versioning fields are filled in by `load_reading` after
        // the diff is computed — the diff helper has no access to
        // the body bytes nor a reason to allocate a stamp on its
        // own. Default values (`""` / `0`) are the same shape that
        // pre-#558 manifests use, so callers that only care about
        // the cell-id diff can ignore them.
        content_hash: String::new(),
        version_stamp: 0,
    }
}

// ── Tests ───────────────────────────────────────────────────────────
//
// Tests reach `parse_forml2` via `load_reading` so they're inherently
// std-only; gated alongside the function they exercise. The shape
// mirrors what FFFFF wrote in `load_reading.rs::tests`; bodies are
// identical (the test surface didn't change — only the source file
// did).

#[cfg(all(test, not(feature = "no_std")))]
mod tests {
    use super::*;
    use crate::ast::{self, Object};

    /// Produce a minimal cell graph that mimics what the metamodel
    /// gives us: a Noun cell with one entity (`Order`). Tests build
    /// on top of this via `parse_to_state_from`.
    fn seed_state() -> Object {
        let nouns = ast::Object::seq(vec![ast::fact_from_pairs(&[
            ("name", "Order"),
            ("objectType", "entity"),
        ])]);
        ast::store("Noun", nouns, &Object::phi())
    }

    #[test]
    fn deny_policy_refuses_immediately() {
        let state = seed_state();
        let err = load_reading(
            &state,
            "x",
            "Customer(.Name) is an entity type.",
            LoadReadingPolicy::Deny,
        )
        .expect_err("Deny policy must reject");
        match err {
            LoadError::Disallowed => {}
            other => panic!("expected Disallowed, got {other:?}"),
        }
    }

    #[test]
    fn empty_name_is_invalid() {
        let state = seed_state();
        let err = load_reading(
            &state,
            "",
            "Customer(.Name) is an entity type.",
            LoadReadingPolicy::AllowAll,
        )
        .expect_err("empty name must reject");
        match err {
            LoadError::InvalidName(_) => {}
            other => panic!("expected InvalidName, got {other:?}"),
        }
    }

    #[test]
    fn whitespace_name_is_invalid() {
        let state = seed_state();
        let err = load_reading(
            &state,
            "   \t",
            "Customer(.Name) is an entity type.",
            LoadReadingPolicy::AllowAll,
        )
        .expect_err("whitespace-only name must reject");
        match err {
            LoadError::InvalidName(_) => {}
            other => panic!("expected InvalidName, got {other:?}"),
        }
    }

    #[test]
    fn control_chars_in_name_rejected() {
        let state = seed_state();
        let err = load_reading(
            &state,
            "bad\x00name",
            "Customer(.Name) is an entity type.",
            LoadReadingPolicy::AllowAll,
        )
        .expect_err("control chars in name must reject");
        match err {
            LoadError::InvalidName(_) => {}
            other => panic!("expected InvalidName, got {other:?}"),
        }
    }

    #[test]
    fn empty_body_rejected() {
        let state = seed_state();
        let err = load_reading(&state, "name", "", LoadReadingPolicy::AllowAll)
            .expect_err("empty body must reject");
        match err {
            LoadError::EmptyBody => {}
            other => panic!("expected EmptyBody, got {other:?}"),
        }
    }

    #[test]
    fn whitespace_body_rejected() {
        let state = seed_state();
        let err = load_reading(&state, "name", "   \n\t  ", LoadReadingPolicy::AllowAll)
            .expect_err("whitespace-only body must reject");
        match err {
            LoadError::EmptyBody => {}
            other => panic!("expected EmptyBody, got {other:?}"),
        }
    }

    /// Test (1): valid reading → cells appear in the new state and the
    /// report lists every added noun.
    #[test]
    fn valid_reading_adds_cells_and_reports_them() {
        let state = seed_state();
        let body = "\
Product(.SKU) is an entity type.
Category(.Name) is an entity type.
";
        let outcome = load_reading(&state, "catalog", body, LoadReadingPolicy::AllowAll)
            .expect("valid reading should load");

        // Both new nouns appear in the report (sorted).
        assert_eq!(
            outcome.report.added_nouns,
            vec!["Category".to_string(), "Product".to_string()]
        );
        // The Order noun is unchanged — must not appear as added.
        assert!(!outcome.report.added_nouns.contains(&"Order".to_string()));

        // The new state has the merged Noun cell (3 entries: Order +
        // Product + Category).
        let nouns_after = ast::fetch_or_phi("Noun", &outcome.new_state);
        let names: Vec<&str> = nouns_after
            .as_seq()
            .map(|s| s.iter().filter_map(|f| ast::binding(f, "name")).collect())
            .unwrap_or_default();
        assert!(names.contains(&"Order"));
        assert!(names.contains(&"Product"));
        assert!(names.contains(&"Category"));
    }

    /// Test (4): malformed FORML → `ParseError` in the diagnostic tree.
    ///
    /// The legacy stage-1 parser is permissive about unknown clauses —
    /// it emits an UnresolvedClause cell rather than a hard parse
    /// error. The hard-error path fires on grammar-keyword shadowing
    /// (#309). We trigger that path by declaring a noun whose name
    /// collides with a reserved grammar keyword (`each`).
    #[test]
    fn malformed_forml_yields_parse_error() {
        let state = seed_state();
        let bad_body = "each(.X) is an entity type.\n";
        let err = load_reading(&state, "bad", bad_body, LoadReadingPolicy::AllowAll)
            .expect_err("reserved-keyword noun declaration must reject");
        match err {
            LoadError::ParseError(_) => {}
            other => panic!("expected ParseError, got {other:?}"),
        }
    }

    /// Test (2): deontic violation → returns `DeonticViolation` and
    /// leaves the input state untouched.
    ///
    /// We synthesize the contradiction by declaring a binary FT whose
    /// roles share a noun (`Person likes Person`) without a ring
    /// constraint, then assert that adding such an FT triggers
    /// `check_ring_completeness`. Since that check fires at
    /// `Level::Hint`, we'd need to escalate it for this verb's purpose;
    /// instead we fabricate a state where `check_ring_validity`
    /// (Level::Error) fires by post-loading a ring-kind constraint that
    /// spans roles of different nouns.
    ///
    /// The test ensures the *path* works — error-class deontic
    /// violations reject the load and the input state is preserved.
    #[test]
    fn deontic_violation_rejects_and_preserves_state() {
        // Pre-seed a state with a binary FT whose two roles target
        // different nouns AND a ring-kind Constraint cell pointing at
        // it. `check_ring_validity` will fire Level::Error on the
        // merged scratch state.
        //
        // Cells expected by `check_ring_validity`:
        //   Constraint: kind=IR, span0_factTypeId=ft, text=...
        //   Role:       factType=ft, nounName=...
        let mut state = seed_state();
        state = ast::store(
            "FactType",
            ast::Object::seq(vec![ast::fact_from_pairs(&[
                ("id", "Person_likes_Animal"),
                ("reading", "Person likes Animal"),
            ])]),
            &state,
        );
        state = ast::store(
            "Role",
            ast::Object::seq(vec![
                ast::fact_from_pairs(&[
                    ("factType", "Person_likes_Animal"),
                    ("nounName", "Person"),
                    ("position", "0"),
                ]),
                ast::fact_from_pairs(&[
                    ("factType", "Person_likes_Animal"),
                    ("nounName", "Animal"),
                    ("position", "1"),
                ]),
            ]),
            &state,
        );
        state = ast::store(
            "Constraint",
            ast::Object::seq(vec![ast::fact_from_pairs(&[
                ("kind", "IR"),
                ("span0_factTypeId", "Person_likes_Animal"),
                ("text", "Person likes Animal is irreflexive"),
            ])]),
            &state,
        );

        // Take a baseline snapshot for the post-rejection equality
        // check. We're asserting the verb is non-mutating on rejection.
        let snapshot = state.clone();

        // Now load any non-empty body. Even a no-op reading triggers
        // the check pass against the merged state, which inherits the
        // pre-existing ring-validity error.
        let result = load_reading(
            &state,
            "noop",
            "# noop\n",
            LoadReadingPolicy::AllowAll,
        );
        match result {
            Err(LoadError::DeonticViolation(diags)) => {
                assert!(
                    !diags.is_empty(),
                    "DeonticViolation must carry at least one diagnostic"
                );
                for d in &diags {
                    assert_eq!(
                        d.level,
                        crate::check::Level::Error,
                        "only Level::Error deontic diags should reject the load"
                    );
                    assert_eq!(d.source, crate::check::Source::Deontic);
                }
            }
            Err(other) => panic!("expected DeonticViolation, got {other:?}"),
            Ok(_) => panic!("ring-validity error must reject the load"),
        }

        // State must be unchanged.
        assert_eq!(
            state, snapshot,
            "rejection must not mutate the input state"
        );
    }

    /// Idempotency: loading the same body twice produces the same
    /// post-state on each call. The second call's report has empty
    /// `added_nouns` because no new noun appears.
    #[test]
    fn re_load_same_body_is_idempotent() {
        let state = seed_state();
        let body = "Product(.SKU) is an entity type.\n";

        let first = load_reading(&state, "catalog", body, LoadReadingPolicy::AllowAll)
            .expect("first load succeeds");
        let second = load_reading(
            &first.new_state,
            "catalog",
            body,
            LoadReadingPolicy::AllowAll,
        )
        .expect("second load succeeds");

        // No new nouns the second time around.
        assert!(
            second.report.added_nouns.is_empty(),
            "re-loading the same body must not report duplicate nouns; got {:?}",
            second.report.added_nouns
        );

        // The Noun cell is identical between the two post-states (set
        // semantics; merge_states dedupes by identity key).
        let nouns1 = ast::fetch_or_phi("Noun", &first.new_state);
        let nouns2 = ast::fetch_or_phi("Noun", &second.new_state);
        assert_eq!(nouns1, nouns2);
    }

    /// Loading a different body under the same name overwrites
    /// (well-formed superset / disjoint additions). The combined state
    /// has both bodies' cells. (Versioning per #558 lands later; this
    /// test pins the current behavior.)
    #[test]
    fn re_load_with_different_body_adds_new_cells() {
        let state = seed_state();
        let first_body = "Product(.SKU) is an entity type.\n";
        let second_body = "Category(.Name) is an entity type.\n";

        let first = load_reading(&state, "catalog", first_body, LoadReadingPolicy::AllowAll)
            .expect("first load succeeds");
        let second = load_reading(
            &first.new_state,
            "catalog",
            second_body,
            LoadReadingPolicy::AllowAll,
        )
        .expect("second load succeeds");

        assert_eq!(second.report.added_nouns, vec!["Category".to_string()]);

        // Merged Noun cell contains all three.
        let nouns_after = ast::fetch_or_phi("Noun", &second.new_state);
        let names: Vec<&str> = nouns_after
            .as_seq()
            .map(|s| s.iter().filter_map(|f| ast::binding(f, "name")).collect())
            .unwrap_or_default();
        assert!(names.contains(&"Order"));
        assert!(names.contains(&"Product"));
        assert!(names.contains(&"Category"));
    }

    // ── #556 unload_reading tests ───────────────────────────────────

    /// Loading writes the manifest cell `_loaded_reading:{name}` so a
    /// future UnloadReading can recover the list of added cells.
    #[test]
    fn load_persists_manifest_cell() {
        let state = seed_state();
        let body = "Product(.SKU) is an entity type.\nCategory(.Name) is an entity type.\n";
        let outcome = load_reading(&state, "catalog", body, LoadReadingPolicy::AllowAll)
            .expect("valid reading should load");
        let decoded = decode_manifest(&outcome.new_state, "catalog")
            .expect("manifest cell must be persisted");
        assert_eq!(
            decoded.added_nouns,
            vec!["Category".to_string(), "Product".to_string()]
        );
        assert_eq!(decoded, outcome.report);
    }

    /// Successful unload of a previously-loaded reading returns the
    /// noun list in the report and removes those rows from the Noun
    /// cell.
    #[test]
    fn unload_reading_removes_added_cells() {
        let state = seed_state();
        let body = "Product(.SKU) is an entity type.\nCategory(.Name) is an entity type.\n";
        let loaded = load_reading(&state, "catalog", body, LoadReadingPolicy::AllowAll)
            .expect("load succeeds");

        // Sanity: nouns are present before the unload.
        let pre_nouns_obj = ast::fetch_or_phi("Noun", &loaded.new_state);
        let nouns_before: Vec<&str> = pre_nouns_obj
            .as_seq()
            .map(|s| s.iter().filter_map(|f| ast::binding(f, "name")).collect())
            .unwrap_or_default();
        assert!(nouns_before.contains(&"Product"));
        assert!(nouns_before.contains(&"Category"));

        let outcome = unload_reading(&loaded.new_state, "catalog", UnloadPolicy::CascadeDelete)
            .expect("unload succeeds");
        assert_eq!(
            outcome.report.removed_nouns,
            vec!["Category".to_string(), "Product".to_string()]
        );

        // Product / Category are gone, Order (seeded) survives.
        let post_obj = ast::fetch_or_phi("Noun", &outcome.new_state);
        let nouns_after: Vec<String> = post_obj
            .as_seq()
            .map(|s| {
                s.iter()
                    .filter_map(|f| ast::binding(f, "name").map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        assert!(nouns_after.contains(&"Order".to_string()));
        assert!(!nouns_after.contains(&"Product".to_string()));
        assert!(!nouns_after.contains(&"Category".to_string()));

        // The manifest cell must be gone.
        assert!(decode_manifest(&outcome.new_state, "catalog").is_none());
    }

    /// Unload of an unknown name → ManifestMissing. Input state is
    /// untouched.
    #[test]
    fn unload_unknown_name_yields_manifest_missing() {
        let state = seed_state();
        let snapshot = state.clone();
        let err = unload_reading(&state, "never-loaded", UnloadPolicy::CascadeDelete)
            .expect_err("unknown name must reject");
        match err {
            UnloadError::ManifestMissing(name) => {
                assert_eq!(name, "never-loaded");
            }
            other => panic!("expected ManifestMissing, got {other:?}"),
        }
        assert_eq!(state, snapshot, "rejection must not mutate input");
    }

    /// Empty/whitespace name rejects with InvalidName.
    #[test]
    fn unload_empty_name_invalid() {
        let state = seed_state();
        match unload_reading(&state, "", UnloadPolicy::CascadeDelete) {
            Err(UnloadError::InvalidName(_)) => {}
            other => panic!("expected InvalidName, got {other:?}"),
        }
        match unload_reading(&state, "   \t", UnloadPolicy::CascadeDelete) {
            Err(UnloadError::InvalidName(_)) => {}
            other => panic!("expected InvalidName, got {other:?}"),
        }
    }

    /// UnloadPolicy::Migrate is stubbed → NotImplemented.
    #[test]
    fn unload_migrate_policy_not_implemented() {
        let state = seed_state();
        let body = "Product(.SKU) is an entity type.\n";
        let loaded = load_reading(&state, "catalog", body, LoadReadingPolicy::AllowAll)
            .expect("load succeeds");
        match unload_reading(&loaded.new_state, "catalog", UnloadPolicy::Migrate) {
            Err(UnloadError::NotImplemented) => {}
            other => panic!("expected NotImplemented, got {other:?}"),
        }
    }

    /// Round-trip: load → unload returns the Noun cell to the
    /// pre-load state. Set semantics: every Noun row that wasn't in
    /// the seed disappears; the seed nouns survive.
    #[test]
    fn unload_after_load_round_trips() {
        let state = seed_state();
        let body = "Product(.SKU) is an entity type.\nCategory(.Name) is an entity type.\n";
        let loaded = load_reading(&state, "catalog", body, LoadReadingPolicy::AllowAll)
            .expect("load succeeds");
        let unloaded = unload_reading(&loaded.new_state, "catalog", UnloadPolicy::CascadeDelete)
            .expect("unload succeeds");

        let nouns_pre = ast::fetch_or_phi("Noun", &state);
        let nouns_post = ast::fetch_or_phi("Noun", &unloaded.new_state);
        assert_eq!(
            nouns_pre, nouns_post,
            "round-trip must restore the Noun cell"
        );
    }

    /// Cascade behavior: load A which adds a Product noun + a fact
    /// type "Product has SKU". After unloading A, the FT row is gone
    /// AND its Role rows are gone too (cascade by FT id on the Role
    /// cell's `factType` binding). We document the current cascade
    /// scope so #557 ReloadReading can decide whether to extend it.
    #[test]
    fn unload_cascades_role_rows_for_removed_fact_types() {
        let state = seed_state();
        let body_a = "\
Product(.SKU) is an entity type.
Product has SKU.
";
        let a = load_reading(&state, "A", body_a, LoadReadingPolicy::AllowAll)
            .expect("A loads");
        let added_ft = a.report.added_fact_types.first().cloned();

        let unloaded = unload_reading(&a.new_state, "A", UnloadPolicy::CascadeDelete)
            .expect("unload A");

        if let Some(ft) = &added_ft {
            let ft_obj = ast::fetch_or_phi("FactType", &unloaded.new_state);
            let ft_ids_after: Vec<String> = ft_obj
                .as_seq()
                .map(|s| {
                    s.iter()
                        .filter_map(|f| ast::binding(f, "id").map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            assert!(
                !ft_ids_after.contains(ft),
                "post-unload FactType must not include {}",
                ft
            );

            let role_obj = ast::fetch_or_phi("Role", &unloaded.new_state);
            let role_facttypes: Vec<String> = role_obj
                .as_seq()
                .map(|s| {
                    s.iter()
                        .filter_map(|f| ast::binding(f, "factType").map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            assert!(
                !role_facttypes.contains(ft),
                "post-unload Role must not reference {}",
                ft
            );
        }
    }

    /// `remove_rows_by_binding` on an empty value list is a no-op.
    #[test]
    fn remove_rows_by_binding_empty_values_noop() {
        let state = seed_state();
        let after = remove_rows_by_binding(&state, "Noun", "name", &[]);
        assert_eq!(state, after);
    }

    /// `remove_cell` on a missing name is a no-op.
    #[test]
    fn remove_cell_missing_noop() {
        let state = seed_state();
        let after = remove_cell(&state, "DoesNotExist");
        assert_eq!(state, after);
    }

    /// `remove_cell` on an existing Map cell drops the entry.
    #[test]
    fn remove_cell_drops_map_entry() {
        let state = seed_state();
        let with_extra = ast::store("Extra", Object::seq(vec![]), &state);
        let after = remove_cell(&with_extra, "Extra");
        assert!(matches!(ast::fetch("Extra", &after), Object::Bottom));
    }

    // ── #557 reload_reading tests ───────────────────────────────────

    /// Happy path: load reading A, then reload A with a modified
    /// body. New state has the new body's cells, no orphans from
    /// the old body, and the manifest reflects the new body.
    #[test]
    fn reload_reading_happy_path_replaces_body() {
        let state = seed_state();
        let body_v1 = "Product(.SKU) is an entity type.\n";
        let body_v2 = "Category(.Name) is an entity type.\n";

        let loaded = load_reading(&state, "catalog", body_v1, LoadReadingPolicy::AllowAll)
            .expect("v1 loads");

        let reloaded = reload_reading(&loaded.new_state, "catalog", body_v2, ReloadPolicy::ReplaceAll)
            .expect("reload to v2 succeeds");

        // Old body's noun is gone.
        assert_eq!(reloaded.removed.removed_nouns, vec!["Product".to_string()]);
        // New body's noun is added.
        assert_eq!(reloaded.added.added_nouns, vec!["Category".to_string()]);

        // Post-state has Category but not Product. (Order is from seed.)
        let nouns_obj = ast::fetch_or_phi("Noun", &reloaded.new_state);
        let nouns: Vec<String> = nouns_obj
            .as_seq()
            .map(|s| {
                s.iter()
                    .filter_map(|f| ast::binding(f, "name").map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        assert!(nouns.contains(&"Order".to_string()), "seed Order survives");
        assert!(nouns.contains(&"Category".to_string()), "v2 Category present");
        assert!(!nouns.contains(&"Product".to_string()), "v1 Product orphaned");

        // Manifest reflects v2's added cells, not v1's.
        let manifest = decode_manifest(&reloaded.new_state, "catalog")
            .expect("manifest must persist");
        assert_eq!(manifest.added_nouns, vec!["Category".to_string()]);
    }

    /// First-time load: reload a name that's never been loaded
    /// falls through to the load step. The `removed` report is
    /// empty; `added` matches a fresh load. We document this
    /// behavior in the function doc.
    #[test]
    fn reload_reading_first_time_load_fallthrough() {
        let state = seed_state();
        let body = "Product(.SKU) is an entity type.\n";

        let outcome = reload_reading(&state, "catalog", body, ReloadPolicy::ReplaceAll)
            .expect("first-time-load falls through to load");

        // Empty removed report — there was nothing to unload.
        assert!(outcome.removed.removed_nouns.is_empty());
        assert!(outcome.removed.removed_fact_types.is_empty());
        assert!(outcome.removed.removed_derivations.is_empty());

        // Added matches a normal first-time load.
        assert_eq!(outcome.added.added_nouns, vec!["Product".to_string()]);

        // Manifest is now present.
        assert!(decode_manifest(&outcome.new_state, "catalog").is_some());
    }

    /// Atomic on parse failure: reload with a body that triggers a
    /// parse error rejects with `LoadFailed(ParseError(_))` and
    /// the input state is byte-identical pre and post.
    #[test]
    fn reload_reading_atomic_on_parse_failure() {
        let state = seed_state();
        let body_v1 = "Product(.SKU) is an entity type.\n";
        let bad_body = "each(.X) is an entity type.\n";

        let loaded = load_reading(&state, "catalog", body_v1, LoadReadingPolicy::AllowAll)
            .expect("v1 loads");

        let snapshot = loaded.new_state.clone();
        let err = reload_reading(&loaded.new_state, "catalog", bad_body, ReloadPolicy::ReplaceAll)
            .expect_err("parse error must reject reload");

        match err {
            ReloadError::LoadFailed(LoadError::ParseError(_)) => {}
            other => panic!("expected LoadFailed(ParseError), got {other:?}"),
        }

        // The input state is untouched — the function takes &Object
        // and never bound a partial state to it. Atomicity contract.
        assert_eq!(loaded.new_state, snapshot, "input state must not mutate on failure");
    }

    /// Atomic on validation failure: reload with a body that triggers
    /// a deontic violation rejects with
    /// `LoadFailed(DeonticViolation(_))` and the input state is
    /// preserved.
    ///
    /// We build a state where check_ring_validity will fire on the
    /// post-load merged state (mirrors
    /// `deontic_violation_rejects_and_preserves_state`).
    #[test]
    fn reload_reading_atomic_on_validation_failure() {
        let mut state = seed_state();
        // Pre-seed ring-validity error scaffolding.
        state = ast::store(
            "FactType",
            ast::Object::seq(vec![ast::fact_from_pairs(&[
                ("id", "Person_likes_Animal"),
                ("reading", "Person likes Animal"),
            ])]),
            &state,
        );
        state = ast::store(
            "Role",
            ast::Object::seq(vec![
                ast::fact_from_pairs(&[
                    ("factType", "Person_likes_Animal"),
                    ("nounName", "Person"),
                    ("position", "0"),
                ]),
                ast::fact_from_pairs(&[
                    ("factType", "Person_likes_Animal"),
                    ("nounName", "Animal"),
                    ("position", "1"),
                ]),
            ]),
            &state,
        );
        state = ast::store(
            "Constraint",
            ast::Object::seq(vec![ast::fact_from_pairs(&[
                ("kind", "IR"),
                ("span0_factTypeId", "Person_likes_Animal"),
                ("text", "Person likes Animal is irreflexive"),
            ])]),
            &state,
        );

        // First-time-load fallthrough is fine: the deontic violation
        // fires inside the load step regardless of unload.
        let snapshot = state.clone();
        let err = reload_reading(&state, "catalog", "# noop\n", ReloadPolicy::ReplaceAll)
            .expect_err("ring-validity error must reject the reload");

        match err {
            ReloadError::LoadFailed(LoadError::DeonticViolation(_)) => {}
            other => panic!("expected LoadFailed(DeonticViolation), got {other:?}"),
        }

        // State preserved exactly.
        assert_eq!(state, snapshot, "input state must not mutate on validation failure");
    }

    /// Idempotency: reload A with byte-identical body. The outcome's
    /// `removed` and `added` reports describe the round-trip (both
    /// list the same cells), but `new_state` ends up byte-equal to a
    /// fresh single-load.
    #[test]
    fn reload_reading_idempotent_with_same_body() {
        let state = seed_state();
        let body = "Product(.SKU) is an entity type.\n";

        // Baseline: a fresh first-time load gives us the target
        // post-state shape.
        let fresh = load_reading(&state, "catalog", body, LoadReadingPolicy::AllowAll)
            .expect("first load succeeds");

        // Now run the round trip via reload starting from fresh.
        let reloaded = reload_reading(&fresh.new_state, "catalog", body, ReloadPolicy::ReplaceAll)
            .expect("idempotent reload succeeds");

        // Removed and added describe the round trip — both list Product.
        assert_eq!(reloaded.removed.removed_nouns, vec!["Product".to_string()]);
        assert_eq!(reloaded.added.added_nouns, vec!["Product".to_string()]);

        // The post-state Noun cell equals the fresh-load Noun cell.
        let nouns_fresh = ast::fetch_or_phi("Noun", &fresh.new_state);
        let nouns_reloaded = ast::fetch_or_phi("Noun", &reloaded.new_state);
        assert_eq!(
            nouns_fresh, nouns_reloaded,
            "Noun cell after reload must equal Noun cell after fresh load"
        );

        // Manifest cells equal on the cell-id lists. Since #558,
        // the version stamp advances on every successful load — so
        // a reload bumps `versionStamp` even when the body is
        // byte-identical. We compare the structural fields (added_*)
        // and the content hash (which IS deterministic on body
        // bytes) but not version_stamp.
        let m_fresh = decode_manifest(&fresh.new_state, "catalog").expect("fresh manifest");
        let m_reloaded = decode_manifest(&reloaded.new_state, "catalog")
            .expect("reloaded manifest");
        assert_eq!(m_fresh.added_nouns, m_reloaded.added_nouns);
        assert_eq!(m_fresh.added_fact_types, m_reloaded.added_fact_types);
        assert_eq!(m_fresh.added_derivations, m_reloaded.added_derivations);
        assert_eq!(
            m_fresh.content_hash, m_reloaded.content_hash,
            "byte-identical body must produce identical content hash"
        );
        assert!(
            m_fresh.version_stamp < m_reloaded.version_stamp,
            "reload bumps version_stamp even on identical body: {} → {}",
            m_fresh.version_stamp, m_reloaded.version_stamp,
        );

    }

    /// Migrate stub: `ReloadPolicy::MigrateFacts` returns
    /// `NotImplemented`. State is untouched.
    #[test]
    fn reload_reading_migrate_facts_not_implemented() {
        let state = seed_state();
        let snapshot = state.clone();
        let err = reload_reading(&state, "catalog", "# anything\n", ReloadPolicy::MigrateFacts)
            .expect_err("MigrateFacts must return NotImplemented");
        match err {
            ReloadError::NotImplemented => {}
            other => panic!("expected NotImplemented, got {other:?}"),
        }
        assert_eq!(state, snapshot, "MigrateFacts must not mutate input");
    }

    /// Empty/whitespace name rejects with `InvalidName` before any
    /// pipeline work. Mirrors the load/unload sanitization.
    #[test]
    fn reload_reading_empty_name_invalid() {
        let state = seed_state();
        match reload_reading(&state, "", "# x\n", ReloadPolicy::ReplaceAll) {
            Err(ReloadError::InvalidName(_)) => {}
            other => panic!("expected InvalidName, got {other:?}"),
        }
        match reload_reading(&state, "   \t", "# x\n", ReloadPolicy::ReplaceAll) {
            Err(ReloadError::InvalidName(_)) => {}
            other => panic!("expected InvalidName, got {other:?}"),
        }
    }

    /// Empty body rejects with `EmptyBody`. State untouched.
    #[test]
    fn reload_reading_empty_body_rejected() {
        let state = seed_state();
        match reload_reading(&state, "x", "", ReloadPolicy::ReplaceAll) {
            Err(ReloadError::EmptyBody) => {}
            other => panic!("expected EmptyBody, got {other:?}"),
        }
        match reload_reading(&state, "x", "   \n\t  ", ReloadPolicy::ReplaceAll) {
            Err(ReloadError::EmptyBody) => {}
            other => panic!("expected EmptyBody, got {other:?}"),
        }
    }

    // ── #558 reading versioning tests ──────────────────────────────

    /// FNV-1a64 is deterministic on identical input and differs on
    /// byte-different input. The output is always 16 lowercase hex
    /// chars (a pinned shape so wire callers can rely on length).
    #[test]
    fn content_hash_deterministic() {
        let h1 = compute_content_hash("Product(.SKU) is an entity type.\n");
        let h2 = compute_content_hash("Product(.SKU) is an entity type.\n");
        let h3 = compute_content_hash("Product(.SKU) is an entity type. \n"); // extra space
        assert_eq!(h1, h2, "same body → same hash");
        assert_ne!(h1, h3, "byte-different body → different hash");
        assert_eq!(h1.len(), 16, "hash must be 16 chars");
        assert!(
            h1.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "hash must be lowercase hex; got {h1}"
        );
    }

    /// Empty body still has a defined hash (even though the verb
    /// rejects empty bodies before computing it — the helper itself
    /// stays total).
    #[test]
    fn content_hash_empty_string_is_well_defined() {
        let h = compute_content_hash("");
        // FNV-1a64 of empty input is the offset basis as hex.
        assert_eq!(h, "cbf29ce484222325");
    }

    /// Three loads (A, B, A again) produce three strictly-increasing
    /// version stamps. Pin the monotonic invariant — note the test
    /// reads the stamp from the report rather than asserting absolute
    /// values, because other tests in the same process may have
    /// already advanced the counter.
    #[test]
    fn version_stamp_monotonic_across_loads() {
        let state = seed_state();
        let body_a = "Product(.SKU) is an entity type.\n";
        let body_b = "Category(.Name) is an entity type.\n";

        let load1 = load_reading(&state, "A", body_a, LoadReadingPolicy::AllowAll)
            .expect("A loads");
        let load2 = load_reading(&load1.new_state, "B", body_b, LoadReadingPolicy::AllowAll)
            .expect("B loads");
        let load3 = load_reading(&load2.new_state, "A", body_a, LoadReadingPolicy::AllowAll)
            .expect("A re-loads");

        assert!(
            load1.report.version_stamp < load2.report.version_stamp,
            "second load's stamp must be higher: {} vs {}",
            load1.report.version_stamp, load2.report.version_stamp,
        );
        assert!(
            load2.report.version_stamp < load3.report.version_stamp,
            "third load's stamp must be higher: {} vs {}",
            load2.report.version_stamp, load3.report.version_stamp,
        );
        // No stamp is ever zero — that's reserved for pre-#558
        // manifests.
        assert!(load1.report.version_stamp > 0);
    }

    /// `seed_version_counter` advances the floor but never rolls
    /// back. Idempotent on a value below the current counter.
    #[test]
    fn seed_version_counter_advances_floor() {
        // Bump the counter by issuing a load.
        let state = seed_state();
        let body = "Product(.SKU) is an entity type.\n";
        let outcome = load_reading(&state, "x", body, LoadReadingPolicy::AllowAll)
            .expect("load succeeds");
        let baseline = outcome.report.version_stamp;

        // Seeding with a lower value is a no-op.
        seed_version_counter(baseline - 1);
        let after_low = next_version();
        assert!(
            after_low > baseline,
            "next_version() after low seed must still advance"
        );

        // Seeding with a higher value pushes the counter forward.
        let target = after_low + 1000;
        seed_version_counter(target);
        let after_high = next_version();
        assert_eq!(
            after_high,
            target + 1,
            "next_version() after high seed = seed + 1"
        );
    }

    /// Round-trip: load → unload → unload report carries the same
    /// content hash and version stamp the load wrote.
    #[test]
    fn manifest_round_trip_preserves_versioning() {
        let state = seed_state();
        let body = "Product(.SKU) is an entity type.\n";
        let loaded = load_reading(&state, "catalog", body, LoadReadingPolicy::AllowAll)
            .expect("load succeeds");

        let unloaded = unload_reading(&loaded.new_state, "catalog", UnloadPolicy::CascadeDelete)
            .expect("unload succeeds");

        assert_eq!(
            unloaded.report.content_hash, loaded.report.content_hash,
            "unload must surface the load's content hash"
        );
        assert_eq!(
            unloaded.report.version_stamp, loaded.report.version_stamp,
            "unload must surface the load's version stamp"
        );
        assert_eq!(unloaded.report.content_hash, compute_content_hash(body));
    }

    /// Reload's `removed` carries the old version's stamp; `added`
    /// carries the new version's stamp; the new stamp is strictly
    /// higher.
    #[test]
    fn reload_reflects_new_version() {
        let state = seed_state();
        let body_v1 = "Product(.SKU) is an entity type.\n";
        let body_v2 = "Category(.Name) is an entity type.\n";

        let loaded = load_reading(&state, "catalog", body_v1, LoadReadingPolicy::AllowAll)
            .expect("v1 loads");

        let reloaded = reload_reading(
            &loaded.new_state,
            "catalog",
            body_v2,
            ReloadPolicy::ReplaceAll,
        )
        .expect("reload succeeds");

        // Removed report mirrors v1's metadata.
        assert_eq!(
            reloaded.removed.content_hash, loaded.report.content_hash,
            "removed.content_hash must equal the original load's hash"
        );
        assert_eq!(
            reloaded.removed.version_stamp, loaded.report.version_stamp,
            "removed.version_stamp must equal the original load's stamp"
        );

        // Added report mirrors v2's body.
        assert_eq!(
            reloaded.added.content_hash,
            compute_content_hash(body_v2),
            "added.content_hash must equal hash of v2 body"
        );
        assert_ne!(
            reloaded.added.content_hash, reloaded.removed.content_hash,
            "v2 hash must differ from v1 hash"
        );
        assert!(
            reloaded.removed.version_stamp < reloaded.added.version_stamp,
            "new stamp must be strictly higher: {} → {}",
            reloaded.removed.version_stamp, reloaded.added.version_stamp,
        );
    }

    /// Backward-compat: a manifest cell written before #558 has no
    /// `contentHash` / `versionStamp` bindings. Decoding must default
    /// those fields to `""` / `0`, and `unload_reading` must still
    /// remove the cells listed in the legacy manifest.
    #[test]
    fn manifest_backward_compat_pre_558() {
        let state = seed_state();

        // Synthesize a pre-#558 manifest cell shape: only the three
        // legacy bindings, no contentHash / versionStamp. This is
        // exactly what `encode_manifest` produced before #558.
        let legacy_cell = Object::seq(vec![ast::fact_from_pairs(&[
            ("addedNouns", "Widget"),
            ("addedFactTypes", ""),
            ("addedDerivations", ""),
        ])]);
        let mut state = state;
        // Also seed the Noun cell so cascade-delete has something to
        // remove.
        state = ast::store(
            "Noun",
            Object::seq(vec![
                ast::fact_from_pairs(&[("name", "Order"), ("objectType", "entity")]),
                ast::fact_from_pairs(&[("name", "Widget"), ("objectType", "entity")]),
            ]),
            &state,
        );
        state = ast::store(&manifest_cell_name("legacy"), legacy_cell, &state);

        // Decoder fills in defaults.
        let decoded = decode_manifest(&state, "legacy")
            .expect("legacy manifest must decode");
        assert_eq!(decoded.added_nouns, vec!["Widget".to_string()]);
        assert_eq!(decoded.content_hash, "", "missing binding → empty string");
        assert_eq!(decoded.version_stamp, 0, "missing binding → zero");

        // Unload still works against the legacy cell.
        let unloaded = unload_reading(&state, "legacy", UnloadPolicy::CascadeDelete)
            .expect("legacy manifest must unload");
        assert_eq!(unloaded.report.removed_nouns, vec!["Widget".to_string()]);
        assert_eq!(unloaded.report.content_hash, "");
        assert_eq!(unloaded.report.version_stamp, 0);

        // Widget is gone, Order survives.
        let nouns_after = ast::fetch_or_phi("Noun", &unloaded.new_state);
        let names: Vec<String> = nouns_after
            .as_seq()
            .map(|s| {
                s.iter()
                    .filter_map(|f| ast::binding(f, "name").map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        assert!(names.contains(&"Order".to_string()));
        assert!(!names.contains(&"Widget".to_string()));
    }

    /// `encode_manifest` includes the new bindings in the persisted
    /// fact, and `decode_manifest` round-trips them.
    #[test]
    fn manifest_encode_decode_round_trip_with_versioning() {
        let report = LoadReport {
            added_nouns: vec!["Foo".to_string(), "Bar".to_string()],
            added_fact_types: vec![],
            added_derivations: vec![],
            content_hash: "deadbeefcafef00d".to_string(),
            version_stamp: 42,
        };
        let cell = encode_manifest(&report);
        // Plumb it through a state so decode_manifest can find it.
        let state = ast::store(&manifest_cell_name("x"), cell, &Object::phi());
        let decoded = decode_manifest(&state, "x").expect("decodes");
        assert_eq!(decoded.content_hash, "deadbeefcafef00d");
        assert_eq!(decoded.version_stamp, 42);
        assert_eq!(decoded.added_nouns, vec!["Foo".to_string(), "Bar".to_string()]);
    }

    /// `u64_to_dec` covers the boundary cases.
    #[test]
    fn u64_to_dec_boundaries() {
        assert_eq!(u64_to_dec(0), "0");
        assert_eq!(u64_to_dec(1), "1");
        assert_eq!(u64_to_dec(123456789), "123456789");
        assert_eq!(u64_to_dec(u64::MAX), "18446744073709551615");
    }
}
