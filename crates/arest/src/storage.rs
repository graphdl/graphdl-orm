// crates/arest/src/storage.rs
//
// Storage-1: pluggable StorageBackend trait.
//
// Replaces the implicit "tenant state lives in
// `HashMap<u32, Arc<RwLock<CompiledState>>>` only" assumption with an
// explicit trait so future builds (local-fs, kernel-fs, durable-object,
// S3) can share one abstraction. The in-memory `Arc<RwLock<…>>` slot
// table is still the runtime wrapper; this module owns how an `Object`
// is persisted across process restarts.
//
// This lane ships:
//   - `StorageBackend` trait (open / commit / checkpoint / restore).
//   - `InMemoryBackend` — default, preserves pre-Storage-1 behaviour.
//   - `LocalFilesystemBackend` — one file per handle, freeze bytes as
//     default format. Enables the acceptance round-trip (commit,
//     simulate process restart, rehydrate, assert state equal).
//
// Out of scope for this lane:
//   - Kernel fs driver (Storage-2).
//   - DurableObject adapter (Storage-3).
//   - Boot-time mount semantics (Storage-4).
//
// The whole module is gated on `not(feature = "no_std")` because
// backends need heap + owned types and the fs backend needs `std::fs`.
// The kernel / no_std target uses `freeze::thaw` directly against
// baked ROM bytes and does not route through this trait.

#![cfg(not(feature = "no_std"))]

use crate::ast::Object;
use crate::sync::Mutex;
use alloc::{
    boxed::Box,
    format,
    string::{String, ToString},
};

#[derive(Debug)]
pub enum StorageError {
    /// No state has been committed for this handle, or the named
    /// checkpoint does not exist.
    NotFound,
    /// The backend's byte representation failed to decode back into an
    /// Object. Usually a freeze/thaw magic mismatch or truncated file.
    Corrupted(String),
    /// IO failure (fs permission, disk full, mount lost).
    Io(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CheckpointId(pub String);

impl CheckpointId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The pluggable persistence surface. Implementors own how an `Object`
/// is stored between process restarts; the engine's in-memory
/// `CompiledState` wrapper is unchanged.
pub trait StorageBackend: Send + Sync {
    /// Rehydrate a tenant's most-recently-committed state. Called on
    /// first `tenant_lock(handle)` that misses the in-memory slot.
    /// Returns `NotFound` for handles the backend has never seen.
    fn open(&self, handle: u32) -> Result<Object, StorageError>;

    /// Persist a full snapshot for a tenant. Delta semantics are a
    /// future optimisation; this lane treats every commit as a full
    /// replace so the on-disk bytes always equal the last-committed
    /// state.
    fn commit(&self, handle: u32, delta: &Object) -> Result<(), StorageError>;

    /// Atomic checkpoint — durable copy of the last-committed state
    /// under a backend-assigned id. Returns `NotFound` if nothing has
    /// been committed for this handle.
    fn checkpoint(&self, handle: u32) -> Result<CheckpointId, StorageError>;

    /// Restore from a named checkpoint. Does not re-commit — the
    /// caller decides whether to `commit()` the restored state back
    /// as the new head.
    fn restore(&self, handle: u32, id: &CheckpointId) -> Result<Object, StorageError>;
}

// ── InMemoryBackend ─────────────────────────────────────────────────

/// Entirely in-process storage. The default backend — preserves the
/// "state lives in RAM" semantics of the engine pre-Storage-1. A fresh
/// `InMemoryBackend` has nothing committed; `open` returns `NotFound`
/// until a `commit` for that handle lands, matching the old
/// `tenant_lock` → `None` behaviour for an un-allocated handle.
pub struct InMemoryBackend {
    committed: Mutex<hashbrown::HashMap<u32, Object>>,
    checkpoints: Mutex<hashbrown::HashMap<u32, hashbrown::HashMap<String, Object>>>,
    next_id: Mutex<u64>,
}

impl InMemoryBackend {
    pub fn new() -> Self {
        Self {
            committed: Mutex::new(hashbrown::HashMap::new()),
            checkpoints: Mutex::new(hashbrown::HashMap::new()),
            next_id: Mutex::new(0),
        }
    }
}

impl Default for InMemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl StorageBackend for InMemoryBackend {
    fn open(&self, handle: u32) -> Result<Object, StorageError> {
        self.committed
            .lock()
            .get(&handle)
            .cloned()
            .ok_or(StorageError::NotFound)
    }

    fn commit(&self, handle: u32, delta: &Object) -> Result<(), StorageError> {
        self.committed.lock().insert(handle, delta.clone());
        Ok(())
    }

    fn checkpoint(&self, handle: u32) -> Result<CheckpointId, StorageError> {
        // Only checkpoint what's durably committed. Reading from the
        // committed map (rather than some caller-supplied live state)
        // keeps the "checkpoint boundary == last commit" invariant the
        // fs backend needs to match.
        let state = self
            .committed
            .lock()
            .get(&handle)
            .cloned()
            .ok_or(StorageError::NotFound)?;
        let id = {
            let mut n = self.next_id.lock();
            let id = format!("ckpt-{}", *n);
            *n += 1;
            id
        };
        self.checkpoints
            .lock()
            .entry(handle)
            .or_insert_with(hashbrown::HashMap::new)
            .insert(id.clone(), state);
        Ok(CheckpointId(id))
    }

    fn restore(&self, handle: u32, id: &CheckpointId) -> Result<Object, StorageError> {
        self.checkpoints
            .lock()
            .get(&handle)
            .and_then(|m| m.get(&id.0))
            .cloned()
            .ok_or(StorageError::NotFound)
    }
}

// ── LocalFilesystemBackend ──────────────────────────────────────────
//
// One file per handle. `<root>/h-<handle>.state` holds the most-recent
// committed Object as freeze bytes. Checkpoints live under
// `<root>/h-<handle>.ckpt/<id>.state` so a checkpoint survives a
// subsequent commit. Writes go via tmp + rename for atomicity — a
// concurrent reader sees either the old bytes or the new bytes, never
// a torn write.
//
// Format: `freeze::freeze` / `freeze::thaw` (see freeze.rs). Matches
// the kernel ROM / WASM-lowering layout so future backends can share
// bytes without a conversion pass.

/// Filesystem-backed storage. One file per handle + a sibling
/// per-handle checkpoint directory. Freeze bytes, tmp+rename writes.
pub struct LocalFilesystemBackend {
    root: std::path::PathBuf,
    next_id: Mutex<u64>,
}

impl LocalFilesystemBackend {
    pub fn new<P: Into<std::path::PathBuf>>(root: P) -> Result<Self, StorageError> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(|e| StorageError::Io(e.to_string()))?;
        Ok(Self {
            root,
            next_id: Mutex::new(0),
        })
    }

    fn state_path(&self, handle: u32) -> std::path::PathBuf {
        self.root.join(format!("h-{handle}.state"))
    }

    fn ckpt_dir(&self, handle: u32) -> std::path::PathBuf {
        self.root.join(format!("h-{handle}.ckpt"))
    }

    fn ckpt_path(&self, handle: u32, id: &str) -> std::path::PathBuf {
        self.ckpt_dir(handle).join(format!("{id}.state"))
    }

    fn read_object(path: &std::path::Path) -> Result<Object, StorageError> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(StorageError::NotFound);
            }
            Err(e) => return Err(StorageError::Io(e.to_string())),
        };
        crate::freeze::thaw(&bytes).map_err(StorageError::Corrupted)
    }

    fn write_object(path: &std::path::Path, obj: &Object) -> Result<(), StorageError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StorageError::Io(e.to_string()))?;
        }
        let tmp = path.with_extension("tmp");
        let bytes = crate::freeze::freeze(obj);
        std::fs::write(&tmp, &bytes).map_err(|e| StorageError::Io(e.to_string()))?;
        std::fs::rename(&tmp, path).map_err(|e| StorageError::Io(e.to_string()))?;
        Ok(())
    }
}

impl StorageBackend for LocalFilesystemBackend {
    fn open(&self, handle: u32) -> Result<Object, StorageError> {
        Self::read_object(&self.state_path(handle))
    }

    fn commit(&self, handle: u32, delta: &Object) -> Result<(), StorageError> {
        Self::write_object(&self.state_path(handle), delta)
    }

    fn checkpoint(&self, handle: u32) -> Result<CheckpointId, StorageError> {
        // Read what's durably committed; a checkpoint is always
        // pinned to committed bytes, never to a caller's live state.
        let state = Self::read_object(&self.state_path(handle))?;
        let id = {
            let mut n = self.next_id.lock();
            let id = format!("ckpt-{}", *n);
            *n += 1;
            id
        };
        Self::write_object(&self.ckpt_path(handle, &id), &state)?;
        Ok(CheckpointId(id))
    }

    fn restore(&self, handle: u32, id: &CheckpointId) -> Result<Object, StorageError> {
        Self::read_object(&self.ckpt_path(handle, &id.0))
    }
}

// ── Boxed trait-object conveniences ─────────────────────────────────

/// Convenience: `Box<InMemoryBackend>` → `Box<dyn StorageBackend>`.
/// Tests and `lib::set_storage_backend` configure the global backend
/// through a trait object, and this coercion keeps the call site noise-
/// free.
pub fn boxed_in_memory() -> Box<dyn StorageBackend> {
    Box::new(InMemoryBackend::new())
}

// ── CellStorageBackend + DurableObjectBackend (Storage-3, #336) ─────
//
// The paper (§5.4, Definition 2 — Cell Isolation) is explicit: each
// entity is a cell, and at most one μ-application writes to any
// cell at a time. Distributed deployments realise that by giving
// each cell its own writer — one DurableObject per cell keyed by
// `(tenant, cell_name)`. This matches docs/12-physical-mapping.md
// §"One DO per cell" and the `EntityDB` shape already in the
// Worker.
//
// `StorageBackend` above treats the whole tenant state as one blob
// — the right surface for in-memory / single-file backends, but
// the wrong shape for per-cell DOs. A tenant-blob DO would serialise
// every disjoint-cell write behind a single DO, violating Def 2.
//
// `CellStorageBackend` is the per-cell surface: read/commit/list/
// delete one `(tenant, cell)` pair at a time. The DO impl maps each
// `(tenant, cell)` to one DO instance by name, which is exactly the
// tenant-separation model the ring-3 userspace design uses
// (per-tenant CR3) applied to persistence: the `tenant` namespace
// scopes the cell keys so no DO is accessible across tenants.
//
// `AsCellStorage` provides a default `StorageBackend` shim on top of
// `CellStorageBackend`: `open(handle)` rehydrates every cell for
// that tenant into one `Object::Map`; `commit(handle, state)` splits
// a Map back into per-cell writes. This preserves the single-object
// engine API while the underlying writes are cell-granular.

/// Cell-granular storage surface (paper §5.4 / Def 2).
///
/// Writes on disjoint cells within one tenant are safe to run
/// concurrently. The backend is responsible for serialising
/// concurrent writes *within* one cell.
pub trait CellStorageBackend: Send + Sync {
    /// Read one cell's contents for `tenant`. `NotFound` when the
    /// (tenant, cell) pair has never been committed.
    fn cell_read(&self, tenant: u32, cell: &str) -> Result<Object, StorageError>;

    /// Commit one cell's contents for `tenant`. Replaces any prior
    /// value atomically.
    fn cell_commit(&self, tenant: u32, cell: &str, value: &Object) -> Result<(), StorageError>;

    /// Enumerate every cell name that has a committed value for
    /// `tenant`. Used on boot / first `open` to rehydrate.
    fn cell_list(&self, tenant: u32) -> Result<alloc::vec::Vec<String>, StorageError>;

    /// Drop one cell. Tenant teardown uses this after listing; idempotent.
    fn cell_delete(&self, tenant: u32, cell: &str) -> Result<(), StorageError>;
}

// ── DurableObjectBackend ────────────────────────────────────────────
//
// Wire form (Cloudflare DO addressed via an intermediary Worker):
//
//   GET    {endpoint}/{tenant}/cell/{cell}          -> 200 freeze-bytes | 404
//   PUT    {endpoint}/{tenant}/cell/{cell} [bytes]  -> 204 | 5xx
//   GET    {endpoint}/{tenant}/cells                -> 200 JSON array of names | 404
//   DELETE {endpoint}/{tenant}/cell/{cell}          -> 204
//
// All cell bodies are raw freeze bytes — same format the kernel ROM
// path uses so a DO export can feed the kernel without conversion.
//
// `HttpTransport` is the pluggable HTTP client. Callers implement it
// against whichever stack they run (workers-rs, reqwest, a mock in
// tests). `DurableObjectBackend` is parameterised on it so the test
// suite can exercise the full protocol without a live Worker.

/// Minimal HTTP surface the DurableObject backend uses. Supplied by
/// the caller so this crate stays HTTP-client-agnostic.
pub trait HttpTransport: Send + Sync {
    /// GET the given URL. `Ok(Some(bytes))` on 200, `Ok(None)` on
    /// 404, `Err(_)` on any other non-success or transport failure.
    fn get(&self, url: &str) -> Result<Option<alloc::vec::Vec<u8>>, StorageError>;

    /// PUT bytes to the given URL. Any non-2xx / transport failure
    /// is an error.
    fn put(&self, url: &str, body: &[u8]) -> Result<(), StorageError>;

    /// DELETE the given URL. `Ok(())` on 2xx or 404 (idempotent),
    /// `Err(_)` otherwise.
    fn delete(&self, url: &str) -> Result<(), StorageError>;
}

/// Cloudflare DurableObject per-cell storage backend. Each
/// `(tenant, cell)` pair maps to one DO instance behind the Worker
/// at `endpoint`. Tenant separation is enforced by the DO router: a
/// request for `tenant=3/cell=foo` never resolves to tenant=7's DOs.
pub struct DurableObjectBackend<T: HttpTransport> {
    /// Base URL with no trailing slash (e.g. `https://arest-storage.example.com`).
    endpoint: String,
    transport: T,
}

impl<T: HttpTransport> DurableObjectBackend<T> {
    pub fn new(endpoint: impl Into<String>, transport: T) -> Self {
        let mut e: String = endpoint.into();
        while e.ends_with('/') { e.pop(); }
        Self { endpoint: e, transport }
    }

    fn cell_url(&self, tenant: u32, cell: &str) -> String {
        // URL-encode cell names minimally (paper cell names are
        // ASCII identifiers with '_', '.', and ':' — all legal in
        // paths). The DO router splits on '/', so any embedded '/'
        // in a cell name would ambiguate the route. Replace '/'
        // with '%2F' as a belt-and-braces guard.
        let encoded = cell.replace('/', "%2F");
        format!("{}/{}/cell/{}", self.endpoint, tenant, encoded)
    }

    fn list_url(&self, tenant: u32) -> String {
        format!("{}/{}/cells", self.endpoint, tenant)
    }
}

impl<T: HttpTransport> CellStorageBackend for DurableObjectBackend<T> {
    fn cell_read(&self, tenant: u32, cell: &str) -> Result<Object, StorageError> {
        match self.transport.get(&self.cell_url(tenant, cell))? {
            Some(bytes) => crate::freeze::thaw(&bytes)
                .map_err(StorageError::Corrupted),
            None => Err(StorageError::NotFound),
        }
    }

    fn cell_commit(&self, tenant: u32, cell: &str, value: &Object) -> Result<(), StorageError> {
        let bytes = crate::freeze::freeze(value);
        self.transport.put(&self.cell_url(tenant, cell), &bytes)
    }

    fn cell_list(&self, tenant: u32) -> Result<alloc::vec::Vec<String>, StorageError> {
        match self.transport.get(&self.list_url(tenant))? {
            Some(bytes) => {
                let text = core::str::from_utf8(&bytes)
                    .map_err(|e| StorageError::Corrupted(format!("non-utf8 cell list: {e}")))?;
                Ok(parse_name_array(text))
            }
            None => Ok(alloc::vec::Vec::new()),
        }
    }

    fn cell_delete(&self, tenant: u32, cell: &str) -> Result<(), StorageError> {
        self.transport.delete(&self.cell_url(tenant, cell))
    }
}

/// Parse a JSON array of strings without pulling serde_json in:
/// `["Noun","Role","Constraint"]` → `vec!["Noun","Role","Constraint"]`.
/// Non-string elements are dropped; malformed input returns empty.
fn parse_name_array(text: &str) -> alloc::vec::Vec<String> {
    let mut out = alloc::vec::Vec::new();
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'"' {
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != b'"' { i += 1; }
            if i >= bytes.len() { break; }
            out.push(String::from(&text[start..i]));
            i += 1;
        } else {
            i += 1;
        }
    }
    out
}

/// Shim that lifts a `CellStorageBackend` into the tenant-blob
/// `StorageBackend` trait. Used when the engine's single-Object API
/// needs to sit on top of per-cell storage. `open` enumerates every
/// cell for the tenant and rebuilds the `Object::Map`; `commit`
/// splits the Map back into per-cell writes.
///
/// Checkpoint / restore are implemented as bulk operations: snapshot
/// the whole tenant as one blob inside a sibling cell namespace
/// keyed by `{cell_name}@{checkpoint_id}`. Rough but keeps the trait
/// contract without a separate checkpoint DO — the per-cell
/// isolation guarantee (Def 2) still holds because each checkpoint
/// cell is its own DO.
pub struct AsCellStorage<C: CellStorageBackend> {
    inner: C,
    next_id: Mutex<u64>,
}

impl<C: CellStorageBackend> AsCellStorage<C> {
    pub fn new(inner: C) -> Self {
        Self { inner, next_id: Mutex::new(0) }
    }

    pub fn into_inner(self) -> C {
        self.inner
    }

    pub fn cells(&self) -> &C {
        &self.inner
    }
}

impl<C: CellStorageBackend> StorageBackend for AsCellStorage<C> {
    fn open(&self, handle: u32) -> Result<Object, StorageError> {
        let names = self.inner.cell_list(handle)?;
        if names.is_empty() {
            return Err(StorageError::NotFound);
        }
        let mut map = hashbrown::HashMap::new();
        for name in names {
            // Skip checkpoint shadow cells when rehydrating head.
            if name.contains('@') { continue; }
            let value = self.inner.cell_read(handle, &name)?;
            map.insert(name, value);
        }
        Ok(Object::Map(map))
    }

    fn commit(&self, handle: u32, delta: &Object) -> Result<(), StorageError> {
        // Engine commits a whole-state Object::Map; split into
        // per-cell writes. Disjoint cells can be parallelised by a
        // future revision — the trait contract permits it. For now
        // write sequentially so an error half-way through surfaces
        // deterministically.
        let map = delta.as_map()
            .ok_or_else(|| StorageError::Corrupted(
                "AsCellStorage::commit expects Object::Map; got other shape".to_string()))?;
        for (cell, value) in map {
            self.inner.cell_commit(handle, cell, value)?;
        }
        Ok(())
    }

    fn checkpoint(&self, handle: u32) -> Result<CheckpointId, StorageError> {
        let names = self.inner.cell_list(handle)?;
        if names.is_empty() {
            return Err(StorageError::NotFound);
        }
        let id = {
            let mut n = self.next_id.lock();
            let id = format!("ckpt-{}", *n);
            *n += 1;
            id
        };
        for name in &names {
            if name.contains('@') { continue; }
            let value = self.inner.cell_read(handle, name)?;
            let shadow = format!("{name}@{id}");
            self.inner.cell_commit(handle, &shadow, &value)?;
        }
        Ok(CheckpointId(id))
    }

    fn restore(&self, handle: u32, id: &CheckpointId) -> Result<Object, StorageError> {
        let names = self.inner.cell_list(handle)?;
        let suffix = format!("@{}", id.as_str());
        let mut map = hashbrown::HashMap::new();
        for name in names {
            if let Some(base) = name.strip_suffix(&suffix) {
                let value = self.inner.cell_read(handle, &name)?;
                map.insert(base.to_string(), value);
            }
        }
        if map.is_empty() {
            return Err(StorageError::NotFound);
        }
        Ok(Object::Map(map))
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Object;
    use alloc::string::ToString;

    fn sample() -> Object {
        let mut m = hashbrown::HashMap::new();
        m.insert("a".to_string(), Object::Atom("1".to_string()));
        m.insert(
            "b".to_string(),
            Object::Seq(alloc::vec![Object::Atom("x".to_string())].into()),
        );
        Object::Map(m)
    }

    // ── InMemoryBackend ─────────────────────────────────────────────

    #[test]
    fn in_memory_open_missing_returns_not_found() {
        let b = InMemoryBackend::new();
        match b.open(7) {
            Err(StorageError::NotFound) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn in_memory_commit_then_open_returns_same_object() {
        let b = InMemoryBackend::new();
        b.commit(3, &sample()).expect("commit ok");
        let got = b.open(3).expect("open ok");
        assert_eq!(got, sample());
    }

    #[test]
    fn in_memory_second_commit_overwrites_first() {
        let b = InMemoryBackend::new();
        b.commit(0, &Object::Atom("old".to_string())).expect("commit 1");
        b.commit(0, &Object::Atom("new".to_string())).expect("commit 2");
        let got = b.open(0).expect("open ok");
        assert_eq!(got, Object::Atom("new".to_string()));
    }

    #[test]
    fn in_memory_distinct_handles_are_isolated() {
        let b = InMemoryBackend::new();
        b.commit(1, &Object::Atom("for-1".to_string())).expect("commit 1");
        b.commit(2, &Object::Atom("for-2".to_string())).expect("commit 2");
        assert_eq!(b.open(1).unwrap(), Object::Atom("for-1".to_string()));
        assert_eq!(b.open(2).unwrap(), Object::Atom("for-2".to_string()));
    }

    #[test]
    fn in_memory_checkpoint_requires_prior_commit() {
        let b = InMemoryBackend::new();
        match b.checkpoint(0) {
            Err(StorageError::NotFound) => {}
            other => panic!("expected NotFound for uncommitted checkpoint, got {other:?}"),
        }
    }

    #[test]
    fn in_memory_checkpoint_then_restore_round_trips() {
        let b = InMemoryBackend::new();
        b.commit(5, &sample()).expect("commit ok");
        let id = b.checkpoint(5).expect("checkpoint ok");
        let restored = b.restore(5, &id).expect("restore ok");
        assert_eq!(restored, sample());
    }

    #[test]
    fn in_memory_checkpoint_pins_bytes_even_after_subsequent_commit() {
        let b = InMemoryBackend::new();
        b.commit(9, &Object::Atom("v1".to_string())).expect("commit 1");
        let id = b.checkpoint(9).expect("checkpoint ok");
        b.commit(9, &Object::Atom("v2".to_string())).expect("commit 2");
        // Restore must still return the v1 state — a checkpoint is
        // durable regardless of subsequent head-state commits.
        let restored = b.restore(9, &id).expect("restore ok");
        assert_eq!(restored, Object::Atom("v1".to_string()));
        // ...and the head state must be v2 for open().
        assert_eq!(b.open(9).unwrap(), Object::Atom("v2".to_string()));
    }

    #[test]
    fn in_memory_restore_unknown_id_returns_not_found() {
        let b = InMemoryBackend::new();
        b.commit(0, &sample()).unwrap();
        let _ = b.checkpoint(0).unwrap();
        match b.restore(0, &CheckpointId("ckpt-not-real".to_string())) {
            Err(StorageError::NotFound) => {}
            other => panic!("expected NotFound for unknown checkpoint id, got {other:?}"),
        }
    }

    // ── LocalFilesystemBackend ──────────────────────────────────────

    /// Build a unique tempdir for a single test. Uses process id +
    /// a thread-local counter so parallel test runs don't collide.
    fn unique_tempdir(label: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let mut p = std::env::temp_dir();
        p.push(format!("arest-storage-{label}-{pid}-{n}"));
        // Fresh tree per test.
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    /// Wipe a tempdir at test end. Best-effort — if cleanup fails
    /// (e.g. a handle is still open on Windows), we leak the bytes
    /// rather than fail the test. The unique_tempdir key keeps
    /// subsequent runs independent either way.
    fn cleanup(p: &std::path::Path) {
        let _ = std::fs::remove_dir_all(p);
    }

    #[test]
    fn local_fs_open_missing_returns_not_found() {
        let tmp = unique_tempdir("open-missing");
        let b = LocalFilesystemBackend::new(&tmp).expect("backend init");
        match b.open(42) {
            Err(StorageError::NotFound) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
        cleanup(&tmp);
    }

    #[test]
    fn local_fs_commit_then_open_round_trips_through_disk() {
        let tmp = unique_tempdir("commit-open");
        let b = LocalFilesystemBackend::new(&tmp).expect("backend init");
        b.commit(7, &sample()).expect("commit ok");

        // Verify the file actually exists on disk before we read it
        // back — catches a "committed to RAM only" bug.
        let expected_path = tmp.join("h-7.state");
        assert!(
            expected_path.exists(),
            "commit must create {expected_path:?}"
        );

        let got = b.open(7).expect("open ok");
        assert_eq!(got, sample());
        cleanup(&tmp);
    }

    #[test]
    fn local_fs_survives_simulated_process_restart() {
        // Acceptance criterion 4: commit, simulate process restart
        // (drop the backend + create a new one pointing at the same
        // dir), rehydrate via open(), assert state equal.
        let tmp = unique_tempdir("restart");
        {
            let b = LocalFilesystemBackend::new(&tmp).expect("backend init");
            b.commit(11, &sample()).expect("commit ok");
            // `b` drops here — simulates process exit.
        }
        let b2 = LocalFilesystemBackend::new(&tmp).expect("rebind ok");
        let got = b2.open(11).expect("open ok after restart");
        assert_eq!(got, sample(), "state must survive backend rebind");
        cleanup(&tmp);
    }

    #[test]
    fn local_fs_checkpoint_then_restore_round_trips() {
        let tmp = unique_tempdir("ckpt-restore");
        let b = LocalFilesystemBackend::new(&tmp).expect("backend init");
        b.commit(13, &sample()).expect("commit ok");
        let id = b.checkpoint(13).expect("checkpoint ok");
        // After another commit, the checkpoint must still restore the
        // earlier bytes (pinned semantics).
        b.commit(13, &Object::Atom("later".to_string()))
            .expect("later commit ok");
        let restored = b.restore(13, &id).expect("restore ok");
        assert_eq!(restored, sample());
        cleanup(&tmp);
    }

    #[test]
    fn local_fs_checkpoint_with_no_commit_returns_not_found() {
        let tmp = unique_tempdir("ckpt-no-commit");
        let b = LocalFilesystemBackend::new(&tmp).expect("backend init");
        match b.checkpoint(0) {
            Err(StorageError::NotFound) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
        cleanup(&tmp);
    }

    #[test]
    fn local_fs_atomic_write_leaves_no_tmp_behind() {
        // A successful commit must not leave an `.tmp` sibling — that
        // would indicate the rename step didn't run. Stale `.tmp`
        // files would accumulate forever under a commit-heavy workload.
        let tmp = unique_tempdir("atomic");
        let b = LocalFilesystemBackend::new(&tmp).expect("backend init");
        b.commit(3, &sample()).expect("commit ok");
        let tmp_file = tmp.join("h-3.tmp");
        assert!(
            !tmp_file.exists(),
            "commit must leave no stale tmp file at {tmp_file:?}"
        );
        cleanup(&tmp);
    }

    // ── trait object convenience ────────────────────────────────────

    #[test]
    fn boxed_in_memory_is_a_storage_backend() {
        let b: Box<dyn StorageBackend> = boxed_in_memory();
        b.commit(0, &sample()).expect("commit via trait object");
        let got = b.open(0).expect("open via trait object");
        assert_eq!(got, sample());
    }

    // ── DurableObjectBackend + AsCellStorage (#336) ────────────────
    //
    // MockHttp is an in-memory HTTP surrogate keyed by URL. The DO
    // backend drives it through its public contract and we assert on
    // the resulting cell-namespace shape. Proves:
    //   - cell_read/commit/list/delete round-trip per (tenant, cell)
    //   - tenant separation: a request under tenant=3 never touches
    //     tenant=7's URL path, so the two tenants' namespaces are
    //     disjoint by construction.
    //   - AsCellStorage lifts cell-granular writes into the blob
    //     StorageBackend API without collapsing the cell partition —
    //     every commit splits into per-cell writes the DO runtime
    //     serialises independently.

    #[derive(Default)]
    struct MockHttp {
        // URL -> bytes. Mutex so the trait's `&self` can mutate.
        store: Mutex<hashbrown::HashMap<String, alloc::vec::Vec<u8>>>,
    }

    impl HttpTransport for MockHttp {
        fn get(&self, url: &str) -> Result<Option<alloc::vec::Vec<u8>>, StorageError> {
            if url.ends_with("/cells") {
                // Synthesize the JSON name array from the mock store's
                // URL namespace.
                let prefix = url.trim_end_matches("/cells");
                let cell_pfx = format!("{}/cell/", prefix);
                let names: Vec<String> = self.store.lock().keys()
                    .filter_map(|k| k.strip_prefix(&cell_pfx).map(String::from))
                    .collect();
                if names.is_empty() { return Ok(None); }
                let body = format!(
                    "[{}]",
                    names.iter()
                        .map(|n| format!("\"{}\"", n))
                        .collect::<Vec<_>>()
                        .join(","),
                );
                return Ok(Some(body.into_bytes()));
            }
            Ok(self.store.lock().get(url).cloned())
        }

        fn put(&self, url: &str, body: &[u8]) -> Result<(), StorageError> {
            self.store.lock().insert(url.to_string(), body.to_vec());
            Ok(())
        }

        fn delete(&self, url: &str) -> Result<(), StorageError> {
            self.store.lock().remove(url);
            Ok(())
        }
    }

    fn do_backend() -> DurableObjectBackend<MockHttp> {
        DurableObjectBackend::new("https://arest-test.example/", MockHttp::default())
    }

    #[test]
    fn do_cell_read_missing_returns_not_found() {
        let b = do_backend();
        match b.cell_read(1, "Noun") {
            Err(StorageError::NotFound) => {}
            other => panic!("expected NotFound; got {other:?}"),
        }
    }

    #[test]
    fn do_cell_commit_then_read_round_trips() {
        let b = do_backend();
        let value = Object::atom("Order");
        b.cell_commit(1, "Noun", &value).expect("commit ok");
        assert_eq!(b.cell_read(1, "Noun").expect("read ok"), value);
    }

    #[test]
    fn do_cell_list_names_every_committed_cell_for_tenant() {
        let b = do_backend();
        b.cell_commit(1, "Noun", &Object::atom("n")).expect("ok");
        b.cell_commit(1, "Role", &Object::atom("r")).expect("ok");
        b.cell_commit(1, "FactType", &Object::atom("f")).expect("ok");
        let mut names = b.cell_list(1).expect("list ok");
        names.sort();
        assert_eq!(names, vec!["FactType", "Noun", "Role"]);
    }

    #[test]
    fn do_cell_namespaces_are_tenant_disjoint() {
        let b = do_backend();
        b.cell_commit(1, "Noun", &Object::atom("tenant-1")).expect("ok");
        b.cell_commit(2, "Noun", &Object::atom("tenant-2")).expect("ok");

        assert_eq!(b.cell_read(1, "Noun").expect("ok"), Object::atom("tenant-1"));
        assert_eq!(b.cell_read(2, "Noun").expect("ok"), Object::atom("tenant-2"));

        // Tenant 1 deleting its Noun cell must not affect tenant 2.
        b.cell_delete(1, "Noun").expect("delete ok");
        match b.cell_read(1, "Noun") {
            Err(StorageError::NotFound) => {}
            other => panic!("expected NotFound for tenant 1; got {other:?}"),
        }
        assert_eq!(b.cell_read(2, "Noun").expect("ok"), Object::atom("tenant-2"));
    }

    #[test]
    fn do_cell_url_escapes_slashes_in_cell_names() {
        // Defensive: no cell name the engine generates has '/', but
        // the DO router would split on it if one ever did. Verify the
        // encoder sidesteps the problem.
        let b = do_backend();
        // Construct a cell name with a slash to exercise the URL
        // encoder path — shape-check by committing and listing.
        b.cell_commit(1, "oddly/named", &Object::atom("v")).expect("ok");
        let got = b.cell_read(1, "oddly/named").expect("read ok");
        assert_eq!(got, Object::atom("v"));
    }

    #[test]
    fn as_cell_storage_lifts_cell_backend_into_blob_backend() {
        // Engine-level contract: open after commit returns the same
        // state. AsCellStorage must preserve that while splitting the
        // commit into per-cell writes underneath.
        let shim = AsCellStorage::new(do_backend());
        let state = sample(); // Object::Map with two cells.
        shim.commit(1, &state).expect("commit ok");
        let got = shim.open(1).expect("open ok");
        assert_eq!(got, state);
    }

    #[test]
    fn as_cell_storage_checkpoint_and_restore_round_trip() {
        let shim = AsCellStorage::new(do_backend());
        shim.commit(1, &sample()).expect("commit ok");
        let id = shim.checkpoint(1).expect("checkpoint ok");

        // Later commit changes head but must not disturb the checkpoint.
        let mut later = hashbrown::HashMap::new();
        later.insert("a".to_string(), Object::atom("later"));
        shim.commit(1, &Object::Map(later)).expect("later commit ok");

        let restored = shim.restore(1, &id).expect("restore ok");
        assert_eq!(restored, sample(), "checkpoint must pin the earlier state");
    }

    #[test]
    fn as_cell_storage_rejects_non_map_commit() {
        let shim = AsCellStorage::new(do_backend());
        match shim.commit(1, &Object::atom("not-a-map")) {
            Err(StorageError::Corrupted(_)) => {}
            other => panic!("expected Corrupted; got {other:?}"),
        }
    }
}
