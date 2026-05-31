// crates/arest-kernel/src/syscall/getpid.rs
//
// Linux x86_64 syscall 39: `getpid(void)`. Returns the calling
// process's pid as an i64. Per #501 (process-identity + TLS setup
// track), this is the first of three identity syscalls in the slice
// (`getpid`, `getuid`/`getgid`, `arch_prctl`).
//
// Linux x86_64 number: `__NR_getpid = 39`
// (`linux/arch/x86/include/uapi/asm/unistd_64.h`).
//
// Tier-1 scope
// ------------
// Returns `current_process.pid` cast to `i64`. The pid is a `u32`
// (Linux `pid_t` is signed i32, but tier-1 pids are small and
// non-negative, so the cast is infallible in practice). The handler
// is a no-op if no process is installed — consistent with the
// pattern `exit::mark_exited` uses when called before the ring-3
// gate installs a process.

use crate::process::current_process_mut;

/// Handle a `getpid()` syscall. Returns the calling process's pid as
/// a non-negative `i64`. Returns 0 if no current process is installed
/// (kernel boot before any process is spawned, or test teardown) —
/// pid 0 is never a valid user process pid on Linux, so the sentinel
/// is safe.
pub fn handle() -> i64 {
    current_process_mut(|maybe_proc| match maybe_proc {
        Some(proc) => proc.pid as i64,
        None => 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::address_space::AddressSpace;
    use crate::process::current_process_install;
    use crate::process::current_process_uninstall;
    use crate::process::process::CURRENT_PROCESS_TEST_LOCK;
    use crate::process::Process;

    /// `getpid()` returns the pid of the registered current process.
    /// Installs a Process with pid=7, calls `handle`, asserts 7.
    #[test]
    fn getpid_returns_current_process_pid() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(7, address_space);
        current_process_install(proc);
        let result = handle();
        current_process_uninstall();
        assert_eq!(result, 7);
    }

    /// `getpid()` with a different pid — ensures the handler reads
    /// from the process struct and doesn't return a hardcoded value.
    #[test]
    fn getpid_returns_different_pid_values() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(1234, address_space);
        current_process_install(proc);
        let result = handle();
        current_process_uninstall();
        assert_eq!(result, 1234);
    }

    /// `getpid()` when no current process is installed returns 0
    /// (not-a-valid-pid sentinel). Mirrors the `mark_exited` no-op
    /// pattern from `exit.rs`.
    #[test]
    fn getpid_returns_zero_when_no_current_process() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        current_process_uninstall();
        let result = handle();
        assert_eq!(result, 0);
    }
}
