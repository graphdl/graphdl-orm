// crates/arest-kernel-image/tests/userbuf_validation.rs
//
// Host-side unit tests for the UserBuf::from_raw validation logic
// in crates/arest-kernel/src/syscall.rs. The arest-kernel crate is
// bin-only + no_std, so we can't `cargo test` it directly — we
// mirror the pure-logic portion here and assert on it. Keep this
// file in sync with the source module.

#[derive(Debug, PartialEq, Eq)]
enum Err { EInval, EFault }

const KERNEL_START: u64 = 0xFFFF_8000_0000_0000;

fn is_canonical(addr: u64) -> bool {
    let top = addr >> 47;
    top == 0 || top == 0x1_FFFF
}

fn from_raw(ptr: u64, len: u64) -> Result<(u64, u64), Err> {
    if len == 0 {
        return Ok((0, 0));
    }
    if len > isize::MAX as u64 {
        return Err(Err::EInval);
    }
    let end = ptr.checked_add(len).ok_or(Err::EFault)?;
    if end > KERNEL_START {
        return Err(Err::EFault);
    }
    if !is_canonical(ptr) || !is_canonical(end.saturating_sub(1)) {
        return Err(Err::EFault);
    }
    Ok((ptr, len))
}

#[test]
fn zero_len_returns_ok_null() {
    assert_eq!(from_raw(0xdead_beef, 0), Ok((0, 0)));
}

#[test]
fn len_over_isize_max_is_einval() {
    let huge = isize::MAX as u64 + 1;
    assert_eq!(from_raw(0x1000, huge), Err(Err::EInval));
}

#[test]
fn wraparound_is_efault() {
    assert_eq!(from_raw(u64::MAX - 16, 32), Err(Err::EFault));
}

#[test]
fn crossing_kernel_start_is_efault() {
    let ptr = KERNEL_START - 8;
    assert_eq!(from_raw(ptr, 16), Err(Err::EFault));
}

#[test]
fn non_canonical_is_efault() {
    // Bit 47 = 0 but bits 48+ non-zero -> non-canonical.
    let ptr = 0x0001_0000_0000_0000_u64;
    assert_eq!(from_raw(ptr, 16), Err(Err::EFault));
}

#[test]
fn low_half_valid_range_is_ok() {
    assert_eq!(from_raw(0x0000_1000, 4096), Ok((0x0000_1000, 4096)));
}
