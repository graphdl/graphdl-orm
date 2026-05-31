// crates/arest-kernel/src/syscall/identity.rs
//
// Linux x86_64 credential syscalls:
//   getuid  (102): return the real user ID
//   getgid  (104): return the real group ID
//   geteuid (107): return the effective user ID
//   getegid (108): return the effective group ID
//
// Per #501 (process-identity + TLS setup). These four syscalls share
// the same tier-1 implementation: all return 0, representing the
// single-user "root" identity of the AREST kernel. There is no user
// model in tier-1 — the kernel runs a single static Linux ELF as the
// sole user, and reporting uid=0/gid=0 is both correct (the process
// effectively runs as root) and minimal (no /etc/passwd, no uid_t
// allocator, no namespace container ID).
//
// Linux x86_64 numbers (from `linux/arch/x86/include/uapi/asm/
// unistd_64.h`):
//   __NR_getuid  = 102
//   __NR_getgid  = 104
//   __NR_geteuid = 107
//   __NR_getegid = 108
//
// Why include geteuid/getegid here
// ---------------------------------
// They fall out trivially from the same all-zero pattern — a single
// `pub fn handle_uid() -> i64 { 0 }` function is shared by both
// getuid and geteuid (and similarly for gid). Adding them now avoids
// a follow-up slice of two one-liners.
//
// Future uid model
// ----------------
// When AREST grows a real user model (containers, namespace contexts),
// these handlers will look up `current_process`'s `Credential` field.
// The tier-1 shape (always 0) keeps the API surface consistent with
// that future extension: the handlers are already in the dispatch
// table and the test pattern is established.

/// Handle a `getuid()` or `geteuid()` syscall. Tier-1 always returns
/// 0 (root uid). The real and effective uid are identical in tier-1
/// — there is no `setuid` or credential-swap surface yet.
pub fn handle_uid() -> i64 {
    0
}

/// Handle a `getgid()` or `getegid()` syscall. Tier-1 always returns
/// 0 (root gid). The real and effective gid are identical in tier-1.
pub fn handle_gid() -> i64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `getuid()` / `geteuid()` return 0 — tier-1 single-user root.
    #[test]
    fn handle_uid_returns_zero() {
        assert_eq!(handle_uid(), 0);
    }

    /// `getgid()` / `getegid()` return 0 — tier-1 single-user root.
    #[test]
    fn handle_gid_returns_zero() {
        assert_eq!(handle_gid(), 0);
    }

    /// uid and gid are both zero — confirms the two handlers are
    /// independent (not sharing a static that might be accidentally
    /// mutated) and both hit the tier-1 constant correctly.
    #[test]
    fn uid_and_gid_are_both_zero() {
        assert_eq!(handle_uid(), handle_gid());
        assert_eq!(handle_uid(), 0);
    }
}
