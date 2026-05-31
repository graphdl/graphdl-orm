// crates/arest-kernel/src/syscall/arch_prctl.rs
//
// Linux x86_64 syscall 158: `arch_prctl(int code, unsigned long addr)`.
// Per #501 (process-identity + TLS setup track). This is the
// high-value syscall in the slice: musl's `__init_tp` calls
// `arch_prctl(ARCH_SET_FS, tp)` in `_start`'s first instructions —
// without it every TLS access (errno, `pthread_self`, stack-guard
// canary) either reads through FS:0 (the old UEFI identity-mapped
// value, whatever that happened to be) or generates a #GP if the CPU's
// FS.base is not set. Landing this handler unblocks the entire musl
// dynamic-TLS path.
//
// Linux x86_64 number: `__NR_arch_prctl = 158`
// (`linux/arch/x86/include/uapi/asm/unistd_64.h`).
//
// Subcodes (from `<sys/prctl.h>` / `<asm/prctl.h>`):
//   ARCH_SET_FS = 0x1002  — set FS.base to `addr`
//   ARCH_GET_FS = 0x1003  — write current FS.base to *`addr`
//
// IA32_FS_BASE MSR = 0xC0000100 (Intel SDM Vol 4, Table 2-2).
//
// Implementation split
// --------------------
// The handler has two concerns:
//
//   1. **Kernel-side storage**: store / read `Process::fs_base`.
//      This is pure Rust and testable on any host.
//
//   2. **Platform MSR write**: on the real x86_64-UEFI target, a
//      `WRMSR IA32_FS_BASE` programs the CPU's FS.base so that the
//      very next `MOV rax, FS:0` in userspace reads through the right
//      thread-pointer. This is gated behind
//      `#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]`
//      so the unit tests compile + run on the host (Windows / Linux /
//      macOS) without needing real MSR access.
//
//      The platform path is in `write_fs_base_msr` — a `#[cfg]`-gated
//      helper that either issues `WRMSR` (real target) or is a no-op
//      (host test / other arches).
//
// ARCH_GET_FS stores the current `fs_base` to the userspace pointer
// `addr`. Tier-1 has the same identity-mapping caveat as `write.rs` +
// `openat.rs`: no real page tables yet (#527), so we treat `addr` as
// a kernel pointer and write directly. A null `addr` returns -EFAULT.
//
// Unknown subcode → -EINVAL (per the Linux man-page for arch_prctl).
//
// errno values:
//   EFAULT = 14  — null or bad-address `addr` on ARCH_GET_FS
//   EINVAL = 22  — unrecognised subcode

use crate::process::current_process_mut;
use crate::syscall::dispatch::{EFAULT, EINVAL};

/// `arch_prctl(ARCH_SET_FS, addr)` subcode — set FS base.
/// Value from `<asm/prctl.h>` / Linux uapi.
pub const ARCH_SET_FS: u64 = 0x1002;

/// `arch_prctl(ARCH_GET_FS, &addr)` subcode — get FS base.
/// Value from `<asm/prctl.h>` / Linux uapi.
pub const ARCH_GET_FS: u64 = 0x1003;

/// IA32_FS_BASE MSR address (Intel SDM Vol 4 Table 2-2). Documented
/// here so the constant reads as the spec value; the actual write goes
/// through the `x86_64` crate's `FsBase` register wrapper (which
/// targets this MSR) rather than a raw MSR id, matching the convention
/// `arch::uefi::x86_64::syscall_msr` already uses for IA32_LSTAR /
/// IA32_STAR / IA32_FMASK.
#[allow(dead_code)]
pub const IA32_FS_BASE: u32 = 0xC000_0100;

/// Program the CPU's FS.base to `value` on x86_64 UEFI. This is the
/// instruction-level side-effect that makes FS-relative accesses
/// (errno, `pthread_self`, the stack canary at `FS:0x28`) resolve
/// through the right thread pointer. Routes through the `x86_64`
/// crate's `FsBase::write`, which emits a `WRMSR` to IA32_FS_BASE
/// (0xC0000100) — the same `model_specific` register family
/// `syscall_msr::install` uses for the SYSCALL MSRs. On non-x86_64
/// targets and in host unit tests this is a no-op: the unit tests
/// only verify that `Process::fs_base` is stored; they don't (and
/// cannot) verify a real MSR.
///
/// SAFETY: `WRMSR` is privileged (CPL 0 only). The kernel runs in
/// ring 0 both before the ring-3 trampoline (#552) flips and, after
/// it flips, inside the SYSCALL entry path (#552's MSR gate), so the
/// write is always issued from CPL 0. Writing a wild value to
/// IA32_FS_BASE would make all subsequent userspace TLS reads return
/// garbage — the handler validates nothing about the address
/// (mirroring Linux: any u64 is a valid FS.base, even 0).
#[cfg(all(target_os = "uefi", target_arch = "x86_64"))]
fn write_fs_base_msr(value: u64) {
    // `FsBase::write` wraps the IA32_FS_BASE MSR write; it takes a
    // `VirtAddr`. The x86_64 crate's API is safe (no `unsafe` block)
    // because writing FS.base is not memory-unsafe on its own.
    x86_64::registers::model_specific::FsBase::write(x86_64::VirtAddr::new(value));
}

/// No-op shim for host tests and non-x86_64 UEFI targets. The unit
/// tests only assert that `Process::fs_base` is stored correctly;
/// the MSR-write side-effect is not exercisable without real hardware.
#[cfg(not(all(target_os = "uefi", target_arch = "x86_64")))]
#[allow(dead_code)]
fn write_fs_base_msr(_value: u64) {
    // No real MSR on this target — storage-only path in unit tests.
}

/// Handle an `arch_prctl(code, addr)` syscall.
///
/// * `ARCH_SET_FS` (0x1002): store `addr` in `Process::fs_base`,
///   program IA32_FS_BASE MSR on x86_64-UEFI, return 0.
/// * `ARCH_GET_FS` (0x1003): write `Process::fs_base` to `*addr`,
///   return 0. Returns `-EFAULT` if `addr` is null.
/// * Anything else: return `-EINVAL`.
///
/// Returns `-EINVAL` for unknown subcodes regardless of whether a
/// current process is installed, mirroring the Linux kernel's
/// `arch_prctl` prototype which validates the subcode before
/// touching process state.
pub fn handle(code: u64, addr: u64) -> i64 {
    match code {
        ARCH_SET_FS => {
            // Store in the kernel-side process struct.
            current_process_mut(|maybe_proc| {
                if let Some(proc) = maybe_proc {
                    proc.fs_base = addr;
                }
            });
            // Program the MSR on the real target. The no-op shim runs
            // in tests. The CPL 0 invariant is documented on
            // `write_fs_base_msr`; `FsBase::write` itself is a safe API.
            write_fs_base_msr(addr);
            0
        }
        ARCH_GET_FS => {
            if addr == 0 {
                return -EFAULT;
            }
            // Read stored fs_base from the process struct and write it
            // to the userspace-provided pointer. Tier-1 identity-mapping
            // note: same as `write.rs` and `openat.rs` — treat the
            // userspace pointer as a kernel pointer (#527 lands real
            // page tables; #561 lands `copy_to_user`).
            let fs_base = current_process_mut(|maybe_proc| match maybe_proc {
                Some(proc) => proc.fs_base,
                None => 0,
            });
            // SAFETY: addr is non-null (checked above); under the tier-1
            // identity mapping it's a valid kernel-space pointer.
            unsafe { core::ptr::write(addr as *mut u64, fs_base) };
            0
        }
        _ => -EINVAL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::address_space::AddressSpace;
    use crate::process::current_process_install;
    use crate::process::current_process_uninstall;
    use crate::process::process::CURRENT_PROCESS_TEST_LOCK;
    use crate::process::Process;

    /// `arch_prctl(ARCH_SET_FS, X)` stores X in `Process::fs_base`
    /// and returns 0. This is the critical path for musl's `__init_tp`
    /// call in `_start`.
    #[test]
    fn arch_set_fs_stores_value_and_returns_zero() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(42, address_space);
        current_process_install(proc);

        let tp: u64 = 0x7fff_dead_beef_0000;
        let result = handle(ARCH_SET_FS, tp);

        // Return value must be 0 (success).
        assert_eq!(result, 0);

        // fs_base must be stored in the process struct.
        crate::process::current_process_mut(|maybe_proc| {
            let proc = maybe_proc.expect("current process must be installed");
            assert_eq!(proc.fs_base, tp);
        });

        current_process_uninstall();
    }

    /// `arch_prctl(ARCH_SET_FS, 0)` stores 0 and returns 0. The
    /// address 0 is valid as an FS base (musl initialises to 0 before
    /// the real `tp` is ready in some paths); we must not interpret it
    /// as a null-pointer error.
    #[test]
    fn arch_set_fs_zero_addr_is_valid() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(3, address_space);
        current_process_install(proc);

        let result = handle(ARCH_SET_FS, 0);
        assert_eq!(result, 0);
        crate::process::current_process_mut(|maybe_proc| {
            let proc = maybe_proc.expect("current process must be installed");
            assert_eq!(proc.fs_base, 0);
        });

        current_process_uninstall();
    }

    /// `arch_prctl(ARCH_GET_FS, &buf)` writes the stored `fs_base`
    /// into `buf` and returns 0.
    #[test]
    fn arch_get_fs_reads_back_stored_value() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(5, address_space);
        current_process_install(proc);

        let tp: u64 = 0x0000_abcd_ef01_2345;
        // Set first.
        let set_result = handle(ARCH_SET_FS, tp);
        assert_eq!(set_result, 0);

        // Get via a stack buffer — safe because we own `buf` and
        // the identity mapping makes a stack address a valid kernel
        // pointer.
        let mut buf: u64 = 0xdead_cafe_babe_dead;
        let get_result = handle(ARCH_GET_FS, &mut buf as *mut u64 as u64);
        assert_eq!(get_result, 0);
        assert_eq!(buf, tp);

        current_process_uninstall();
    }

    /// `arch_prctl(ARCH_GET_FS, 0)` — null destination pointer —
    /// returns `-EFAULT`.
    #[test]
    fn arch_get_fs_null_addr_returns_efault() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        let address_space = AddressSpace::new(0x40_1000);
        let proc = Process::new(6, address_space);
        current_process_install(proc);

        let result = handle(ARCH_GET_FS, 0);
        assert_eq!(result, -EFAULT);

        current_process_uninstall();
    }

    /// `arch_prctl` with an unknown subcode returns `-EINVAL`.
    /// Per Linux man-page: "EINVAL: code is not a valid subcommand."
    #[test]
    fn arch_prctl_unknown_code_returns_einval() {
        let _guard = CURRENT_PROCESS_TEST_LOCK.lock();
        // No process needed — the subcode check fires before any
        // process-struct access.
        current_process_uninstall();

        assert_eq!(handle(0x0000, 0), -EINVAL);
        assert_eq!(handle(0x1001, 0), -EINVAL); // ARCH_SET_GS (not implemented)
        assert_eq!(handle(0x1004, 0), -EINVAL); // ARCH_GET_GS (not implemented)
        assert_eq!(handle(0xffff, 0), -EINVAL);
    }

    /// Verify the ARCH_SET_FS / ARCH_GET_FS constant values against the
    /// Linux uapi header `<asm/prctl.h>`.
    #[test]
    fn arch_prctl_constants_match_linux_uapi() {
        assert_eq!(ARCH_SET_FS, 0x1002);
        assert_eq!(ARCH_GET_FS, 0x1003);
    }
}
