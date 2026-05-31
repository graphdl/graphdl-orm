// crates/arest-kernel/src/syscall/ioctl.rs
//
// Linux x86_64 syscall 16: `ioctl(int fd, unsigned long request, ...)`.
// Per #502 (terminal stubs track). Implements the two terminal-query
// requests that a static musl/glibc binary is likely to issue during
// early startup or when probing for TTY capabilities:
//
//   TIOCGWINSZ (0x5413) — query terminal window size (`struct winsize`)
//   TCGETS     (0x5401) — get terminal attributes (`struct termios`)
//
// Linux x86_64 number: `__NR_ioctl = 16`
// (`linux/arch/x86/include/uapi/asm/unistd_64.h`).
//
// Struct layouts (from `<asm/termios.h>` / `<sys/ioctl.h>`)
// ----------------------------------------------------------
// `struct winsize` (from `<asm/termios.h>`, 8 bytes):
//   __u16 ws_row;     // rows, in characters
//   __u16 ws_col;     // columns, in characters
//   __u16 ws_xpixel;  // horizontal size, in pixels
//   __u16 ws_ypixel;  // vertical size, in pixels
//
// `struct termios` (from `<asm/termbits.h>`, 36 bytes on x86_64):
//   tcflag_t c_iflag;    // input mode flags  (4 bytes)
//   tcflag_t c_oflag;    // output mode flags (4 bytes)
//   tcflag_t c_cflag;    // control mode flags(4 bytes)
//   tcflag_t c_lflag;    // local mode flags  (4 bytes)
//   cc_t     c_line;     // line discipline   (1 byte)
//   cc_t     c_cc[NCCS]; // control chars     (19 bytes)
//   speed_t  c_ispeed;   // input speed       (4 bytes) — on Linux these
//   speed_t  c_ospeed;   // output speed      (4 bytes)   trail c_cc[19]
//
// The audit specifies "zeroed-struct is acceptable" for TCGETS.  We use
// exactly that: all fields zero, which presents a terminal with no flags
// set and speed 0 — enough for a library call to succeed without error
// and proceed past the tty-check.
//
// Tier-1 identity-mapped pointer model
// -------------------------------------
// There are no real page tables in tier-1 (#527 lands those). Following
// the precedent set by `arch_prctl::handle` (ARCH_GET_FS) and other
// handlers that write through user pointers, we treat the `arg` pointer
// (rdx) as a kernel-space pointer and write directly via
// `core::ptr::write`. A null `arg` (rdx == 0) returns -EFAULT.
//
// errno values used
// -----------------
//   EFAULT = 14  — null destination pointer
//   ENOTTY = 25  — unknown/unsupported ioctl request
//                  (matches `<asm-generic/errno-base.h>:ENOTTY`)

use crate::syscall::dispatch::EFAULT;

/// Linux ioctl request code for `TIOCGWINSZ`: get terminal window size.
/// Value from `<asm/ioctls.h>` (same for all Linux architectures on
/// the x86_64 ioctl-code table).
pub const TIOCGWINSZ: u64 = 0x5413;

/// Linux ioctl request code for `TCGETS`: get terminal attributes
/// (struct termios). Value from `<asm/ioctls.h>`.
pub const TCGETS: u64 = 0x5401;

/// Linux errno value for "Not a typewriter" (not a TTY / unsupported
/// ioctl request).  Value: 25.  Source: `<asm-generic/errno-base.h>`.
/// Returned for any ioctl request code not in the above set.
pub const ENOTTY: i64 = 25;

/// Terminal window-size struct matching `struct winsize` from
/// `<asm/termios.h>`. Packed to exactly 8 bytes: four `u16` fields
/// in row / col / xpixel / ypixel order.
///
/// `repr(C)` ensures the Rust layout matches the Linux ABI layout —
/// no padding between the fields (all `u16`, naturally 2-byte aligned,
/// placed at offsets 0 / 2 / 4 / 6).
#[repr(C)]
pub struct WinSize {
    /// Number of terminal rows (characters).
    pub ws_row: u16,
    /// Number of terminal columns (characters).
    pub ws_col: u16,
    /// Horizontal pixel size (informational; 0 when unknown).
    pub ws_xpixel: u16,
    /// Vertical pixel size (informational; 0 when unknown).
    pub ws_ypixel: u16,
}

/// Number of control-character slots in `struct termios::c_cc[]`.
/// Linux x86_64 value: `NCCS = 19` from `<asm/termbits.h>`.
pub const NCCS: usize = 19;

/// Terminal-attributes struct matching `struct termios` from
/// `<asm/termbits.h>` for Linux x86_64.  Layout:
///
///   offset  0: c_iflag  (u32)
///   offset  4: c_oflag  (u32)
///   offset  8: c_cflag  (u32)
///   offset 12: c_lflag  (u32)
///   offset 16: c_line   (u8)
///   offset 17: c_cc[19] (u8 × 19)
///   offset 36: c_ispeed (u32)
///   offset 40: c_ospeed (u32)
///   total: 44 bytes
///
/// `repr(C)` preserves the ABI layout.  All fields default / zero:
/// "zeroed termios" is the tier-1 acceptable implementation per the
/// audit ("tcgetattr zeroed-struct").
#[repr(C)]
pub struct Termios {
    /// Input mode flags (`c_iflag`).
    pub c_iflag: u32,
    /// Output mode flags (`c_oflag`).
    pub c_oflag: u32,
    /// Control mode flags (`c_cflag`).
    pub c_cflag: u32,
    /// Local mode flags (`c_lflag`).
    pub c_lflag: u32,
    /// Line discipline identifier (`c_line`).
    pub c_line: u8,
    /// Control characters array (`c_cc[NCCS]`).
    pub c_cc: [u8; NCCS],
    /// Input baud rate (`c_ispeed`).
    pub c_ispeed: u32,
    /// Output baud rate (`c_ospeed`).
    pub c_ospeed: u32,
}

/// Handle an `ioctl(fd, request, arg)` syscall.
///
/// * `TIOCGWINSZ` (0x5413): write a `winsize` struct at `*arg`
///   with ws_row=24, ws_col=80, ws_xpixel=0, ws_ypixel=0.  Returns 0.
/// * `TCGETS` (0x5401): write a zeroed `termios` struct at `*arg`.
///   Returns 0.
/// * Any other request code: returns `-ENOTTY` (25).
///
/// `fd` (rdi) is accepted but ignored — tier-1 does not track whether
/// the fd is actually a TTY; the stubs unconditionally satisfy the
/// request so that static binaries probing for terminal capabilities
/// see a plausible response and proceed.
///
/// Returns `-EFAULT` if `arg` is null for TIOCGWINSZ or TCGETS.
pub fn handle(fd: u64, request: u64, arg: u64) -> i64 {
    let _ = fd; // accepted, not inspected in tier-1

    match request {
        TIOCGWINSZ => {
            if arg == 0 {
                return -EFAULT;
            }
            let ws = WinSize {
                ws_row: 24,
                ws_col: 80,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            // SAFETY: arg is non-null (checked above); under the tier-1
            // identity mapping it is a valid kernel-space pointer.
            // The caller owns the buffer (enforced in tests via a local
            // stack variable).
            unsafe { core::ptr::write(arg as *mut WinSize, ws) };
            0
        }
        TCGETS => {
            if arg == 0 {
                return -EFAULT;
            }
            let termios = Termios {
                c_iflag: 0,
                c_oflag: 0,
                c_cflag: 0,
                c_lflag: 0,
                c_line: 0,
                c_cc: [0u8; NCCS],
                c_ispeed: 0,
                c_ospeed: 0,
            };
            // SAFETY: same as TIOCGWINSZ above.
            unsafe { core::ptr::write(arg as *mut Termios, termios) };
            0
        }
        _ => -ENOTTY,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------
    // TIOCGWINSZ tests
    // ---------------------------------------------------------------

    /// `TIOCGWINSZ` constant matches the Linux uapi value 0x5413.
    #[test]
    fn tiocgwinsz_constant_matches_linux_uapi() {
        assert_eq!(TIOCGWINSZ, 0x5413);
    }

    /// `TCGETS` constant matches the Linux uapi value 0x5401.
    #[test]
    fn tcgets_constant_matches_linux_uapi() {
        assert_eq!(TCGETS, 0x5401);
    }

    /// `ENOTTY` is 25 — matches `<asm-generic/errno-base.h>:ENOTTY`.
    #[test]
    fn enotty_value_matches_linux_uapi() {
        assert_eq!(ENOTTY, 25);
    }

    /// `WinSize` is exactly 8 bytes (four `u16`s) — matches the Linux
    /// ABI layout for `struct winsize`.
    #[test]
    fn winsize_struct_is_8_bytes() {
        assert_eq!(core::mem::size_of::<WinSize>(), 8);
    }

    /// `Termios` layout check: 44 bytes total on x86_64.
    /// 4+4+4+4+1+19+4+4 = 44 (no padding because u32 fields are 4-byte
    /// aligned and the u8 run ends on a 4-byte boundary after padding).
    #[test]
    fn termios_struct_size_matches_linux_abi() {
        // 4*4 (flags) + 1 (c_line) + 19 (c_cc) + 4 + 4 (speeds) = 44
        // with repr(C) the compiler may add 2 bytes after c_cc[19] to
        // align c_ispeed to 4 bytes: 20 bytes for c_line+c_cc → pad 3 → 23
        // but 17+19=36 (from offset 16), aligned to 4 → pad to 36 → speeds.
        // Actual: 16 (flags) + 1 + 19 = 36; then 2 pad bytes to reach 38?
        // Let's just document the real size and assert it.
        let sz = core::mem::size_of::<Termios>();
        // On x86_64 Linux, struct termios is 60 bytes in the POSIX (glibc)
        // layout but only 44 bytes in the kernel ABI layout (asm/termbits.h).
        // With repr(C) and these fields, the Rust compiler will lay it out as:
        //   0..16:  four u32 (16 bytes)
        //   16:     c_line u8 (1 byte)
        //   17..36: c_cc [u8;19] (19 bytes)  → total 36 bytes so far
        //   36..38: 2 bytes padding to align next u32 to 4
        //   38..42: actually NO — u32 has 4-byte alignment; 36 % 4 == 0, so
        //           no padding needed here.
        //   36..40: c_ispeed u32
        //   40..44: c_ospeed u32
        // Total: 44 bytes.
        assert_eq!(sz, 44, "Termios must be 44 bytes (kernel asm/termbits.h ABI)");
    }

    /// `ioctl(fd, TIOCGWINSZ, &buf)` fills ws_row=24, ws_col=80,
    /// ws_xpixel=0, ws_ypixel=0 and returns 0.
    #[test]
    fn tiocgwinsz_fills_correct_dimensions() {
        // Use an uninitialised (MaybeUninit) local as the destination
        // buffer — safe because we write before we read.
        let mut buf = core::mem::MaybeUninit::<WinSize>::uninit();
        let result = handle(1, TIOCGWINSZ, buf.as_mut_ptr() as u64);
        assert_eq!(result, 0, "TIOCGWINSZ must return 0 (success)");
        // SAFETY: handle() wrote a valid WinSize into buf.
        let ws = unsafe { buf.assume_init() };
        assert_eq!(ws.ws_row, 24, "ws_row must be 24");
        assert_eq!(ws.ws_col, 80, "ws_col must be 80");
        assert_eq!(ws.ws_xpixel, 0, "ws_xpixel must be 0");
        assert_eq!(ws.ws_ypixel, 0, "ws_ypixel must be 0");
    }

    /// `ioctl(fd, TIOCGWINSZ, 0)` — null destination — returns -EFAULT.
    #[test]
    fn tiocgwinsz_null_arg_returns_efault() {
        let result = handle(1, TIOCGWINSZ, 0);
        assert_eq!(result, -EFAULT);
    }

    // ---------------------------------------------------------------
    // TCGETS tests
    // ---------------------------------------------------------------

    /// `ioctl(fd, TCGETS, &buf)` fills a zeroed termios and returns 0.
    #[test]
    fn tcgets_fills_zeroed_termios_and_returns_zero() {
        let mut buf = core::mem::MaybeUninit::<Termios>::uninit();
        let result = handle(1, TCGETS, buf.as_mut_ptr() as u64);
        assert_eq!(result, 0, "TCGETS must return 0 (success)");
        // SAFETY: handle() wrote a valid Termios into buf.
        let t = unsafe { buf.assume_init() };
        assert_eq!(t.c_iflag, 0);
        assert_eq!(t.c_oflag, 0);
        assert_eq!(t.c_cflag, 0);
        assert_eq!(t.c_lflag, 0);
        assert_eq!(t.c_line, 0);
        assert_eq!(t.c_cc, [0u8; NCCS]);
        assert_eq!(t.c_ispeed, 0);
        assert_eq!(t.c_ospeed, 0);
    }

    /// `ioctl(fd, TCGETS, 0)` — null destination — returns -EFAULT.
    #[test]
    fn tcgets_null_arg_returns_efault() {
        let result = handle(1, TCGETS, 0);
        assert_eq!(result, -EFAULT);
    }

    // ---------------------------------------------------------------
    // Unknown request test
    // ---------------------------------------------------------------

    /// Unknown ioctl request codes return -ENOTTY (25).
    /// Per `<asm-generic/errno-base.h>`: ENOTTY = 25 "Not a typewriter".
    #[test]
    fn unknown_ioctl_request_returns_minus_enotty() {
        assert_eq!(handle(1, 0x0000, 0), -ENOTTY);
        assert_eq!(handle(1, 0x5412, 0), -ENOTTY); // one below TCGETS... wait
        // 0x5401 is TCGETS, 0x5413 is TIOCGWINSZ; anything else is unknown
        assert_eq!(handle(1, 0x5402, 0), -ENOTTY);
        assert_eq!(handle(1, 0x5414, 0), -ENOTTY);
        assert_eq!(handle(1, 0xffff_ffff, 0), -ENOTTY);
    }

    /// `fd` argument is ignored: ioctl works on fd 0, 42, or u64::MAX.
    #[test]
    fn tiocgwinsz_fd_is_ignored() {
        let mut buf = core::mem::MaybeUninit::<WinSize>::uninit();
        // fd = 0
        assert_eq!(handle(0, TIOCGWINSZ, buf.as_mut_ptr() as u64), 0);
        // fd = 42
        assert_eq!(handle(42, TIOCGWINSZ, buf.as_mut_ptr() as u64), 0);
        // fd = u64::MAX
        assert_eq!(handle(u64::MAX, TIOCGWINSZ, buf.as_mut_ptr() as u64), 0);
    }

    /// Dispatch route: `dispatch(SYS_IOCTL, fd, TIOCGWINSZ, arg, ...)` → 0.
    /// Exercises the wiring in dispatch.rs without a real user pointer —
    /// uses a stack buffer as the destination.
    #[test]
    fn dispatch_sys_ioctl_tiocgwinsz_routes_correctly() {
        use crate::syscall::dispatch::{dispatch, SYS_IOCTL};
        let mut buf = core::mem::MaybeUninit::<WinSize>::uninit();
        let result = dispatch(
            SYS_IOCTL,
            1,
            TIOCGWINSZ,
            buf.as_mut_ptr() as u64,
            0,
            0,
            0,
        );
        assert_eq!(result, 0);
        let ws = unsafe { buf.assume_init() };
        assert_eq!(ws.ws_row, 24);
        assert_eq!(ws.ws_col, 80);
    }

    /// Dispatch route: unknown ioctl → -ENOTTY via the dispatch table.
    #[test]
    fn dispatch_sys_ioctl_unknown_returns_minus_enotty() {
        use crate::syscall::dispatch::{dispatch, SYS_IOCTL};
        let result = dispatch(SYS_IOCTL, 1, 0xdead_beef, 0, 0, 0, 0);
        assert_eq!(result, -ENOTTY);
    }
}
