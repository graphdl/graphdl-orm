// crates/arest/src/sync.rs
//
// Crate-internal synchronisation primitives. One set of types,
// one set of call sites — usable from both the default std build
// and the `no_std` build without cfg_if at every lock.
//
// Design: use spin-based locks everywhere. `spin::{Mutex,RwLock,Once}`
// work on std too (they cost spinning instead of OS-managed blocking),
// and AREST's locks are short-held — cell reads/writes under a single
// apply, DOMAINS slot lookups. There is no IO inside a lock, so the
// wait times are microseconds and spinning is cheap.
//
// Going spin-only also drops poison recovery: std::sync::Mutex taints
// a lock when a holder panics, forcing every caller to decide what to
// do with the taint. AREST treats a panic inside a reducer as a bug to
// fix, not a recoverable runtime condition, so poison recovery would
// never fire anyway.
//
// `Arc` comes from `alloc::sync::Arc` which works identically on std
// and no_std. `OnceLock` wraps `spin::Once` so call sites keep the
// `get_or_init` shape std users expect.

pub use alloc::sync::Arc;
pub use spin::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// std-flavoured `OnceLock` façade over `spin::Once`. The wrapper
/// only exists to keep `get_or_init` on the API; `spin::Once` spells
/// the same operation `call_once`.
pub struct OnceLock<T>(spin::Once<T>);

impl<T> OnceLock<T> {
    pub const fn new() -> Self {
        Self(spin::Once::new())
    }

    pub fn get(&self) -> Option<&T> {
        self.0.get()
    }

    pub fn get_or_init<F: FnOnce() -> T>(&self, init: F) -> &T {
        self.0.call_once(init)
    }
}

impl<T> Default for OnceLock<T> {
    fn default() -> Self {
        Self::new()
    }
}
