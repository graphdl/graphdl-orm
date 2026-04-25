// crates/arest-kernel/src/linuxkpi/workqueue.rs
//
// Linux workqueue shim. Linux drivers use workqueues to defer work
// from interrupt context (where allocation / sleeping is forbidden)
// to a process context. The pattern is:
//
//   INIT_WORK(&dev->work, my_work_fn);    // bind handler
//   queue_work(system_wq, &dev->work);    // enqueue, returns immediately
//
// Later, in the workqueue thread:
//   my_work_fn(&dev->work);
//
// AREST's "workqueue thread" doesn't exist — we have no scheduler,
// no kernel threads. Instead we run a ring of pending work items
// drained per-frame from the launcher super-loop (see
// `crate::ui_apps::launcher::run`'s `linuxkpi::tick()` call site).
// Same pattern that worked for net::poll() and slint's per-frame
// `update_timers_and_animations`.
//
// Storage
// -------
// `work_struct` is a C-ABI struct embedded in the driver's per-device
// state. It carries a function pointer (the handler) plus a free-form
// data pointer that's set by INIT_WORK. The shim's ring stores
// (`*mut work_struct`) values; on drain, each entry's handler is
// invoked with itself as the argument.
//
// `delayed_work` is a `work_struct` + a millisecond delay. We track
// the deadline in the entry and drain only entries whose deadline has
// elapsed, using `crate::arch::time::now_ms()`. Same single-thread
// safety story as the rest of the shim.

use alloc::collections::VecDeque;
use core::ffi::c_int;
use core::ptr;
use spin::{Mutex, Once};

/// `struct work_struct` — Linux's basic work item. The C side
/// declares it via `vendor/linux/include/linux/workqueue.h` matching
/// this layout exactly.
///
/// `func` is the handler called when the work fires.
/// `data` is opaque — drivers stash a state pointer there with
/// container_of() at handler time.
#[repr(C)]
pub struct WorkStruct {
    pub data: *mut core::ffi::c_void,
    pub entry_next: *mut WorkStruct, // unused on AREST — Linux uses for list_head
    pub entry_prev: *mut WorkStruct,
    pub func: Option<unsafe extern "C" fn(*mut WorkStruct)>,
}

unsafe impl Send for WorkStruct {}
unsafe impl Sync for WorkStruct {}

/// `struct delayed_work` — `work_struct` + a millisecond timer. The
/// `timer` field is the Linux `struct timer_list`; we model only the
/// `expires_ms` field as the absolute deadline (in our `now_ms()`
/// time base).
#[repr(C)]
pub struct DelayedWork {
    pub work: WorkStruct,
    pub expires_ms: u64,
}

/// Pending work entry on the shim ring. Stored as a raw pointer
/// because the lifetime is owned by the driver (it embeds the
/// `work_struct` in its own state and guarantees the storage lives
/// at least until the work fires).
///
/// Single-threaded kernel — no concurrent access; the Send/Sync
/// impls only exist to satisfy the static Once's Sync bound.
#[repr(C)]
struct PendingWork {
    work: *mut WorkStruct,
    expires_ms: u64, // 0 = ready now (immediate queue_work)
}

unsafe impl Send for PendingWork {}
unsafe impl Sync for PendingWork {}

/// Ring of pending work. Capacity grows on demand; in practice
/// drivers rarely have more than a handful in flight at once, but
/// VecDeque amortises growth to O(1).
static QUEUE: Once<Mutex<VecDeque<PendingWork>>> = Once::new();

pub fn init() {
    QUEUE.call_once(|| Mutex::new(VecDeque::new()));
}

/// `INIT_WORK(work, func)` — bind a handler to a work_struct. In
/// Linux this is a macro that does `work->func = func; INIT_LIST_
/// HEAD(&work->entry);`. Both are fine to do on us — no allocation,
/// just two field stores.
///
/// Most drivers call this exactly once per work_struct, at probe
/// time. Calling INIT_WORK on a work that's already pending is
/// undefined behaviour in real Linux too (the work would dequeue
/// twice); we don't try to defend against it.
#[no_mangle]
pub extern "C" fn INIT_WORK(work: *mut WorkStruct, func: unsafe extern "C" fn(*mut WorkStruct)) {
    if work.is_null() {
        return;
    }
    // SAFETY: caller guarantees `work` is a valid storage location
    // (driver-embedded). Single store, no aliasing concern.
    unsafe {
        (*work).func = Some(func);
        (*work).data = ptr::null_mut();
        (*work).entry_next = ptr::null_mut();
        (*work).entry_prev = ptr::null_mut();
    }
}

/// `INIT_DELAYED_WORK(dwork, func)` — same as `INIT_WORK` for the
/// embedded `work` plus zeroing the deadline.
#[no_mangle]
pub extern "C" fn INIT_DELAYED_WORK(
    dwork: *mut DelayedWork,
    func: unsafe extern "C" fn(*mut WorkStruct),
) {
    if dwork.is_null() {
        return;
    }
    // SAFETY: same as INIT_WORK; embedded `work` field is at offset 0
    // (matches the C layout).
    unsafe {
        INIT_WORK(&mut (*dwork).work as *mut WorkStruct, func);
        (*dwork).expires_ms = 0;
    }
}

/// `queue_work(wq, work)` — enqueue immediately. Returns true (1) if
/// the work was queued, false (0) if it was already pending. We don't
/// detect re-queue; always returns 1.
///
/// `wq` (workqueue handle) is ignored — we have one global ring.
#[no_mangle]
pub extern "C" fn queue_work(_wq: *mut core::ffi::c_void, work: *mut WorkStruct) -> c_int {
    if work.is_null() {
        return 0;
    }
    if let Some(q) = QUEUE.get() {
        q.lock().push_back(PendingWork {
            work,
            expires_ms: 0,
        });
    }
    1
}

/// `queue_delayed_work(wq, dwork, delay_jiffies)` — enqueue with a
/// delay. Linux's `delay` is in jiffies (HZ-relative); we treat the
/// scalar as plain milliseconds, which matches what every driver
/// does on a HZ=1000 kernel (the AREST PIT runs at 1 kHz too —
/// `arch::uefi::time` ticks once per ms).
#[no_mangle]
pub extern "C" fn queue_delayed_work(
    _wq: *mut core::ffi::c_void,
    dwork: *mut DelayedWork,
    delay_ms: u64,
) -> c_int {
    if dwork.is_null() {
        return 0;
    }
    let now = current_time_ms();
    let deadline = now.saturating_add(delay_ms);
    // SAFETY: caller guarantees `dwork` is valid.
    unsafe {
        (*dwork).expires_ms = deadline;
        if let Some(q) = QUEUE.get() {
            q.lock().push_back(PendingWork {
                work: &mut (*dwork).work as *mut WorkStruct,
                expires_ms: deadline,
            });
        }
    }
    1
}

/// `cancel_work_sync(work)` — yank the work off the queue (if
/// pending) and wait for any in-flight invocation to complete. Same
/// single-CPU story as `synchronize_irq`: nothing can be in-flight
/// from a different context. Just remove pending entries.
///
/// Returns 1 if the work was cancelled while pending, 0 otherwise.
#[no_mangle]
pub extern "C" fn cancel_work_sync(work: *mut WorkStruct) -> c_int {
    if let Some(q) = QUEUE.get() {
        let mut q = q.lock();
        let before = q.len();
        q.retain(|pw| pw.work != work);
        if q.len() < before {
            return 1;
        }
    }
    0
}

/// `cancel_delayed_work_sync(dwork)` — cancel a delayed work item.
/// Same shape as `cancel_work_sync` but operates on the embedded
/// `work` pointer (which is at offset 0 in `DelayedWork`).
#[no_mangle]
pub extern "C" fn cancel_delayed_work_sync(dwork: *mut DelayedWork) -> c_int {
    if dwork.is_null() {
        return 0;
    }
    cancel_work_sync(dwork as *mut WorkStruct)
}

/// `flush_work(work)` — wait for the work to finish if currently
/// running, otherwise no-op. Single-CPU: the only way to be "running"
/// is to be on this very stack frame, in which case waiting would
/// deadlock. So this is just a no-op return success.
#[no_mangle]
pub extern "C" fn flush_work(_work: *mut WorkStruct) -> c_int {
    1
}

/// Drain ready work items. Called once per launcher super-loop tick
/// from `linuxkpi::tick()`. Pops every entry whose deadline has
/// elapsed (or which has no deadline), and invokes its handler.
///
/// Bounded — drains at most `MAX_PER_TICK` entries to keep one tick
/// from monopolising the CPU if a handler enqueues more work.
pub fn drain() {
    const MAX_PER_TICK: usize = 32;
    let now = current_time_ms();
    let mut to_run: alloc::vec::Vec<*mut WorkStruct> = alloc::vec::Vec::new();
    if let Some(q) = QUEUE.get() {
        let mut q = q.lock();
        let mut idx = 0;
        while idx < q.len() && to_run.len() < MAX_PER_TICK {
            if q[idx].expires_ms <= now {
                let pw = q.remove(idx).unwrap();
                to_run.push(pw.work);
                // Don't advance idx — remove shifted next entry into
                // current slot.
            } else {
                idx += 1;
            }
        }
    }
    for w in to_run {
        // SAFETY: the work pointer was valid when queue_work was
        // called; the driver owns the storage and is responsible for
        // not freeing it before the handler fires (Linux's contract
        // too — drivers always call cancel_work_sync before freeing).
        unsafe {
            if let Some(func) = (*w).func {
                func(w);
            }
        }
    }
}

/// Read the current millisecond clock. Routes through
/// `arch::uefi::time::now_ms()` on x86_64 UEFI; falls back to a
/// monotonic counter that never advances on other arches (the
/// linuxkpi feature gates the build to x86_64 UEFI in practice, so
/// the fallback is dead code but kept for cargo-check parity on the
/// other arms).
fn current_time_ms() -> u64 {
    #[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
    {
        crate::arch::uefi::time::now_ms()
    }
    #[cfg(not(all(target_os = "uefi", target_arch = "x86_64")))]
    {
        0
    }
}
