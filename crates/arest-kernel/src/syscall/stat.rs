// crates/arest-kernel/src/syscall/stat.rs
//
// Linux x86_64 syscalls:
//   4: `stat(const char *pathname, struct stat *statbuf)`
//   5: `fstat(int fd, struct stat *statbuf)`
//
// Per the AREST kernel tier-1 file-state surface (#500).  Both syscalls
// fill a Linux x86_64 `struct stat` at a user-supplied pointer with
// stubbed metadata.  Full path-resolved stat and file-backed writes
// require a VFS/block layer that does not yet exist; a follow-up task
// should be filed to implement real stat once the VFS is in place.
//
// Linux x86_64 numbers:
//   `__NR_stat  = 4`
//   `__NR_fstat = 5`
// (`linux/arch/x86/include/uapi/asm/unistd_64.h`).
//
// `struct stat` layout (Linux x86_64 uapi, `<asm/stat.h>`)
// ----------------------------------------------------------
// The definitive layout is from the kernel uapi header
// `linux/arch/x86/include/uapi/asm/stat.h` (the same layout musl
// uses for `bits/alltypes.h` on x86_64):
//
//   offset  0: st_dev      u64    — device ID of containing filesystem
//   offset  8: st_ino      u64    — inode number
//   offset 16: st_nlink    u64    — number of hard links
//   offset 24: st_mode     u32    — file type and mode
//   offset 28: st_uid      u32    — user ID of file owner
//   offset 32: st_gid      u32    — group ID of file owner
//   offset 36: __pad0      u32    — padding (reserved, must be zero)
//   offset 40: st_rdev     u64    — device ID (if special file)
//   offset 48: st_size     i64    — total size in bytes
//   offset 56: st_blksize  i64    — preferred block size for I/O
//   offset 64: st_blocks   i64    — number of 512B blocks allocated
//   offset 72: st_atime    i64    — time of last access (seconds)
//   offset 80: st_atime_ns i64    — time of last access (nanoseconds)
//   offset 88: st_mtime    i64    — time of last modification (seconds)
//   offset 96: st_mtime_ns i64    — time of last modification (nanoseconds)
//   offset 104: st_ctime   i64    — time of last status change (seconds)
//   offset 112: st_ctime_ns i64   — time of last status change (nanoseconds)
//   offset 120: __reserved [3]i64 — kernel reserved, must be zero
//   total: 144 bytes
//
// This matches the musl source:
//   vendor/musl/arch/x86_64/bits/stat.h — used by `struct stat` in musl.
//
// Note: the kernel ABI uses separate `st_atime` + `st_atime_nsec` names
// internally, but the struct layout is identical — two consecutive 64-bit
// fields per timestamp.
//
// Tier-1 stub values
// ------------------
// For known fds (0 = stdin, 1 = stdout, 2 = stderr) we return:
//   st_dev     = 5           (virtual device — matches Linux's devpts dev)
//   st_ino     = fd + 1      (synthetic inode, distinct per fd)
//   st_nlink   = 1           (one hard link — always)
//   st_mode    = S_IFCHR | 0o666  (character device, rw-rw-rw-)
//   st_uid     = 0           (root — tier-1 is single-user)
//   st_gid     = 0           (root gid)
//   __pad0     = 0
//   st_rdev    = 0x8800      (synthetic rdev matching Linux's /dev/tty maj:min)
//   st_size    = 0           (char device — no fixed size)
//   st_blksize = 4096        (standard block size, what glibc/musl expect)
//   st_blocks  = 0           (no allocated blocks for a char device)
//   all timestamps = 0       (epoch — no time tracking in tier-1)
//
// For `fstat` with unknown fd (anything other than 0/1/2): returns
// `-EBADF` (-9).  For `stat` with any path: returns a stub that looks
// like a character device (same as fd 1) — a real path-resolved stat
// needs the VFS layer; returning the stub lets programs that stat
// stdin/stdout-equivalent paths (/dev/tty, etc.) proceed. Unknown-path
// stat that shouldn't exist can return `-ENOENT` (-2) only once a
// synthetic fs table exists; until then a stub is the correct tier-1
// response.
//
// Tier-1 identity-mapped pointer model
// -------------------------------------
// Follows the precedent of `ioctl::handle` (TIOCGWINSZ / TCGETS):
// user pointer `statbuf` is treated as a kernel-space pointer under
// the tier-1 identity mapping (no real page tables until #527). A null
// `statbuf` returns `-EFAULT` (-14). Once #527 lands real page tables
// the write must route through `copy_to_user`.
//
// errno values used
// -----------------
//   EFAULT = 14  — null statbuf pointer
//   EBADF  =  9  — unknown fd (fstat only; tier-1 recognises 0/1/2)
//   ENOENT =  2  — path not found (stat — future use once VFS lands)

use crate::syscall::dispatch::{EBADF, EFAULT};

/// Linux errno value for "No such file or directory". Value: 2.
/// Source: `<asm-generic/errno-base.h>`. Returned by `stat` for paths
/// that can't be resolved (reserved for future VFS integration).
pub const ENOENT: i64 = 2;

/// File mode bit: character special file. Value: `0o020000`.
/// Source: `<linux/stat.h>:S_IFCHR`. Combined with the permission bits
/// to form `st_mode` for fd 0/1/2 (stdin/stdout/stderr), which are
/// character devices on Linux.
pub const S_IFCHR: u32 = 0o020000;

/// File mode bit: directory. Value: `0o040000`. Source:
/// `<linux/stat.h>:S_IFDIR`. Returned for paths/fds the directory
/// resolution recognizes (getdents64-file-population) — busybox ls
/// stats its operand FIRST and only opens + getdents64s it when the
/// mode says directory; the prior char-device stub made `ls /` print
/// `/` as a plain name and exit.
pub const S_IFDIR: u32 = 0o040000;

/// File mode bit: regular file. Value: `0o100000`. Source:
/// `<linux/stat.h>:S_IFREG`. Returned for exact `File_has_Name`
/// matches.
pub const S_IFREG: u32 = 0o100000;

/// Linux x86_64 syscall number for `stat(pathname, statbuf)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_stat` (= 4).
/// The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_stat`. Routes to
/// `stat::handle_stat`, which fills a stubbed `struct stat` at the
/// caller's `statbuf` pointer. Full path resolution requires the VFS
/// layer (#500 follow-up).
pub const SYS_STAT: u64 = 4;

/// Linux x86_64 syscall number for `fstat(fd, statbuf)`. Source:
/// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_fstat` (= 5).
/// The vendored musl tree confirms at
/// `vendor/musl/arch/x86_64/bits/syscall.h.in:__NR_fstat`. Routes to
/// `stat::handle_fstat`, which fills a stubbed `struct stat` for the
/// known tier-1 fds (0/1/2) and returns `-EBADF` for others.
pub const SYS_FSTAT: u64 = 5;

/// Linux x86_64 kernel ABI `struct stat`.
///
/// Layout matches `linux/arch/x86/include/uapi/asm/stat.h` and the
/// musl `vendor/musl/arch/x86_64/bits/stat.h` layout exactly.
/// Field offsets and sizes (see module-level comment for provenance):
///
///   offset   0: st_dev      u64   (8)  → total   8
///   offset   8: st_ino      u64   (8)  → total  16
///   offset  16: st_nlink    u64   (8)  → total  24
///   offset  24: st_mode     u32   (4)  → total  28
///   offset  28: st_uid      u32   (4)  → total  32
///   offset  32: st_gid      u32   (4)  → total  36
///   offset  36: __pad0      u32   (4)  → total  40
///   offset  40: st_rdev     u64   (8)  → total  48
///   offset  48: st_size     i64   (8)  → total  56
///   offset  56: st_blksize  i64   (8)  → total  64
///   offset  64: st_blocks   i64   (8)  → total  72
///   offset  72: st_atime    i64   (8)  → total  80
///   offset  80: st_atime_ns i64   (8)  → total  88
///   offset  88: st_mtime    i64   (8)  → total  96
///   offset  96: st_mtime_ns i64   (8)  → total 104
///   offset 104: st_ctime    i64   (8)  → total 112
///   offset 112: st_ctime_ns i64   (8)  → total 120
///   offset 120: __reserved  [3]i64 (24) → total 144
///
/// Total: 144 bytes.  `repr(C)` ensures the Rust compiler preserves
/// this exact layout — all fields are naturally aligned so no implicit
/// padding is inserted.
#[repr(C)]
pub struct Stat {
    /// Device ID of the containing filesystem (`st_dev`).
    pub st_dev: u64,
    /// Inode number (`st_ino`).
    pub st_ino: u64,
    /// Number of hard links (`st_nlink`).
    pub st_nlink: u64,
    /// File type and access mode (`st_mode`). Set to `S_IFCHR | 0o666`
    /// for stdin/stdout/stderr (character devices with rw-rw-rw- perms).
    pub st_mode: u32,
    /// User ID of file owner (`st_uid`). Tier-1: 0 (root).
    pub st_uid: u32,
    /// Group ID of file owner (`st_gid`). Tier-1: 0 (root).
    pub st_gid: u32,
    /// Reserved padding (`__pad0`). Must be zero.
    pub __pad0: u32,
    /// Device ID if special file (`st_rdev`). Synthetic tty device.
    pub st_rdev: u64,
    /// Total size in bytes (`st_size`). Zero for character devices.
    pub st_size: i64,
    /// Preferred I/O block size (`st_blksize`). 4096 bytes.
    pub st_blksize: i64,
    /// Number of 512B blocks allocated (`st_blocks`). Zero for char devs.
    pub st_blocks: i64,
    /// Last access time, seconds since epoch (`st_atime`).
    pub st_atime: i64,
    /// Last access time, nanoseconds part (`st_atime_ns`).
    pub st_atime_ns: i64,
    /// Last modification time, seconds since epoch (`st_mtime`).
    pub st_mtime: i64,
    /// Last modification time, nanoseconds part (`st_mtime_ns`).
    pub st_mtime_ns: i64,
    /// Last status change time, seconds since epoch (`st_ctime`).
    pub st_ctime: i64,
    /// Last status change time, nanoseconds part (`st_ctime_ns`).
    pub st_ctime_ns: i64,
    /// Kernel-reserved, must be zero (`__reserved[3]`).
    pub __reserved: [i64; 3],
}

/// Produce stub `Stat` values for a known terminal fd (0/1/2).
///
/// All three standard streams are treated as character devices
/// (S_IFCHR | 0o666). The device ID and rdev are synthetic values
/// that match what Linux returns for `/dev/tty`-class devices.
/// All timestamps are zero (epoch) — tier-1 has no time source.
///
/// `fd` is used only to generate a distinct `st_ino` (inode = fd + 1)
/// so that programs distinguishing stdin/stdout/stderr by inode see
/// three different values — the same technique Linux uses for the
/// real devpts inodes.
fn tty_stat(fd: u64) -> Stat {
    Stat {
        st_dev: 5,             // synthetic device ID (matches Linux devpts)
        st_ino: fd + 1,        // synthetic inode: 1/2/3 for fd 0/1/2
        st_nlink: 1,           // exactly one hard link
        st_mode: S_IFCHR | 0o666, // character device, rw-rw-rw-
        st_uid: 0,             // root
        st_gid: 0,             // root
        __pad0: 0,
        st_rdev: 0x8800,       // synthetic rdev (maj=136, min=0 — devpts-style)
        st_size: 0,            // char devices have no size
        st_blksize: 4096,      // standard preferred I/O block size
        st_blocks: 0,          // no allocated blocks
        st_atime: 0,
        st_atime_ns: 0,
        st_mtime: 0,
        st_mtime_ns: 0,
        st_ctime: 0,
        st_ctime_ns: 0,
        __reserved: [0i64; 3],
    }
}

/// Handle a `fstat(fd, statbuf)` syscall (SYS_FSTAT = 5).
///
/// Fills the `struct stat` at `statbuf` with stub metadata for the
/// known tier-1 file descriptors:
///   * fd 0 (stdin), fd 1 (stdout), fd 2 (stderr): filled as a
///     character device (`S_IFCHR | 0o666`, `st_blksize = 4096`,
///     `st_size = 0`).  Returns 0 (success).
///   * Any other fd: returns `-EBADF` (-9).
///
/// Returns `-EFAULT` (-14) if `statbuf` is null.
///
/// SAFETY: `statbuf` is treated as a kernel-space pointer under the
/// tier-1 identity mapping (no real page tables until #527). The null
/// check guards against the most common mistake; a non-null unmapped
/// address would fault under real page tables.
pub fn handle_fstat(fd: u64, statbuf: u64) -> i64 {
    if statbuf == 0 {
        return -EFAULT;
    }
    // The three standard streams keep the tty stub. fds ≥ 3 consult the
    // per-process fd table (getdents64-file-population): a Directory fd
    // reports S_IFDIR (busybox ls fstats the directory it just opened —
    // an -EBADF here would abort the listing), a File fd S_IFREG, a
    // Synthetic fd the char-device stub. Unknown fds stay -EBADF.
    match fd {
        0 | 1 | 2 => {
            let s = tty_stat(fd);
            // SAFETY: statbuf is non-null (checked above); under the
            // tier-1 identity mapping it is a valid kernel-space pointer.
            unsafe { core::ptr::write(statbuf as *mut Stat, s) };
            0
        }
        _ => {
            use crate::process::current_process_fd_table;
            use crate::process::fd_table::FdEntry;
            let mode = current_process_fd_table(|maybe| {
                maybe.and_then(|table| match table.lookup(fd as i32) {
                    Some(FdEntry::Directory { .. }) => Some(S_IFDIR | 0o755),
                    Some(FdEntry::File { .. }) => Some(S_IFREG | 0o644),
                    Some(FdEntry::Synthetic { .. }) => Some(S_IFCHR | 0o666),
                    _ => None,
                })
            });
            match mode {
                Some(st_mode) => {
                    let mut s = tty_stat(fd);
                    s.st_mode = st_mode;
                    if st_mode & S_IFDIR != 0 {
                        s.st_size = 4096; // conventional directory size
                    }
                    // SAFETY: statbuf is non-null (checked above).
                    unsafe { core::ptr::write(statbuf as *mut Stat, s) };
                    0
                }
                None => -EBADF,
            }
        }
    }
}

/// Handle a `stat(pathname, statbuf)` syscall (SYS_STAT = 4).
///
/// `pathname` (rdi) is a pointer to a null-terminated path string.
/// In tier-1 there is no VFS/path-resolution layer; we return a
/// stub character-device stat (same as fd 1) for any non-null path.
/// This lets programs that stat console-adjacent paths (`/dev/tty`,
/// `/dev/stdin`, etc.) proceed without error.
///
/// Returns `-EFAULT` (-14) if either `pathname` or `statbuf` is null.
///
/// Note: a real path-resolved `stat` requires the VFS layer. Once
/// that lands, this handler should be updated to:
///   1. Walk the synthetic fs table for `/proc/*` etc.
///   2. Fall back to the File-cell graph (#398) for real paths.
///   3. Return `-ENOENT` for paths not found in either table.
/// Track as a follow-up to #500.
///
/// SAFETY: `pathname` and `statbuf` are kernel-space pointers under
/// the tier-1 identity mapping (see module-level note on the
/// identity-mapped pointer model).
pub fn handle_stat(pathname: u64, statbuf: u64) -> i64 {
    // Guard both pointers; pathname != 0 is required even though the
    // fallback arm doesn't dereference it (it would be a logic error
    // to stat the null path).
    if pathname == 0 || statbuf == 0 {
        return -EFAULT;
    }
    // Path classification (getdents64-file-population): resolve the
    // pathname and report the REAL file type for the surfaces tier-1
    // models — directory (the openat directory predicate: synthetic-fs
    // children or File-graph prefix), regular file (exact
    // File_has_Name match), else the legacy char-device stub. The stub
    // fallback is deliberate: ash and friends stat console-adjacent
    // and $PATH entries during boot, and an -ENOENT here would change
    // working behavior — narrowing the stub to ENOENT is the #500
    // VFS follow-up's call, not this slice's.
    let mut s = tty_stat(0);
    if let Ok(path) = crate::syscall::openat::read_pathname(pathname) {
        let dir = if path.len() > 1 && path.ends_with('/') {
            alloc::string::String::from(&path[..path.len() - 1])
        } else {
            path.clone()
        };
        if crate::synthetic_fs::list_children(&dir).is_some()
            || crate::syscall::openat::path_has_file_children(&dir)
        {
            s.st_mode = S_IFDIR | 0o755;
            s.st_size = 4096; // conventional directory size
        } else if crate::syscall::openat::lookup_file_cell_id(&path).is_some() {
            s.st_mode = S_IFREG | 0o644;
        }
    }
    // SAFETY: statbuf is non-null (checked above) and points to a
    // valid buffer in the tier-1 identity-mapped address space.
    unsafe { core::ptr::write(statbuf as *mut Stat, s) };
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------
    // Constant value tests
    // -------------------------------------------------------------------

    /// `SYS_STAT` is 4 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_stat`.
    #[test]
    fn sys_stat_constant_is_4() {
        assert_eq!(SYS_STAT, 4, "SYS_STAT must be 4 per Linux x86_64 unistd_64.h");
    }

    /// `SYS_FSTAT` is 5 — matches
    /// `linux/arch/x86/include/uapi/asm/unistd_64.h:__NR_fstat`.
    #[test]
    fn sys_fstat_constant_is_5() {
        assert_eq!(SYS_FSTAT, 5, "SYS_FSTAT must be 5 per Linux x86_64 unistd_64.h");
    }

    /// `S_IFCHR` is 0o020000 — matches `<linux/stat.h>:S_IFCHR`.
    #[test]
    fn s_ifchr_constant_matches_linux_uapi() {
        assert_eq!(S_IFCHR, 0o020000);
    }

    /// `ENOENT` is 2 — matches `<asm-generic/errno-base.h>:ENOENT`.
    #[test]
    fn enoent_constant_is_2() {
        assert_eq!(ENOENT, 2);
    }

    // -------------------------------------------------------------------
    // Struct layout tests
    // -------------------------------------------------------------------

    /// `Stat` is exactly 144 bytes — matches the Linux x86_64 kernel ABI
    /// `struct stat` from `asm/stat.h`.
    ///
    /// Field breakdown:
    ///   u64 × 3 (dev/ino/nlink)  = 24
    ///   u32 × 4 (mode/uid/gid/pad) = 16
    ///   u64 (rdev)               =  8
    ///   i64 × 9 (size/blksize/blocks + 3×2 timestamps)  = 72
    ///   i64 × 3 (__reserved)     = 24
    ///   Total: 24+16+8+72+24 = 144
    #[test]
    fn stat_struct_is_144_bytes() {
        assert_eq!(
            core::mem::size_of::<Stat>(),
            144,
            "Stat must be 144 bytes (Linux x86_64 uapi asm/stat.h ABI)"
        );
    }

    /// Field offsets match the Linux uapi layout exactly. Verified
    /// against `<asm/stat.h>` field ordering and natural alignment.
    #[test]
    fn stat_field_offsets_match_linux_abi() {
        // Use a zeroed instance to compute field offsets via pointer
        // arithmetic — no macro, no unsafe offset_of hack needed: we
        // construct a zeroed Stat on the stack and take raw field pointers.
        let s = Stat {
            st_dev: 0,
            st_ino: 0,
            st_nlink: 0,
            st_mode: 0,
            st_uid: 0,
            st_gid: 0,
            __pad0: 0,
            st_rdev: 0,
            st_size: 0,
            st_blksize: 0,
            st_blocks: 0,
            st_atime: 0,
            st_atime_ns: 0,
            st_mtime: 0,
            st_mtime_ns: 0,
            st_ctime: 0,
            st_ctime_ns: 0,
            __reserved: [0; 3],
        };
        let base = &s as *const Stat as usize;
        assert_eq!(&s.st_dev     as *const _ as usize - base, 0);
        assert_eq!(&s.st_ino     as *const _ as usize - base, 8);
        assert_eq!(&s.st_nlink   as *const _ as usize - base, 16);
        assert_eq!(&s.st_mode    as *const _ as usize - base, 24);
        assert_eq!(&s.st_uid     as *const _ as usize - base, 28);
        assert_eq!(&s.st_gid     as *const _ as usize - base, 32);
        assert_eq!(&s.__pad0     as *const _ as usize - base, 36);
        assert_eq!(&s.st_rdev    as *const _ as usize - base, 40);
        assert_eq!(&s.st_size    as *const _ as usize - base, 48);
        assert_eq!(&s.st_blksize as *const _ as usize - base, 56);
        assert_eq!(&s.st_blocks  as *const _ as usize - base, 64);
        assert_eq!(&s.st_atime   as *const _ as usize - base, 72);
        assert_eq!(&s.st_atime_ns as *const _ as usize - base, 80);
        assert_eq!(&s.st_mtime   as *const _ as usize - base, 88);
        assert_eq!(&s.st_mtime_ns as *const _ as usize - base, 96);
        assert_eq!(&s.st_ctime   as *const _ as usize - base, 104);
        assert_eq!(&s.st_ctime_ns as *const _ as usize - base, 112);
        assert_eq!(&s.__reserved as *const _ as usize - base, 120);
    }

    // -------------------------------------------------------------------
    // fstat tests — known fds
    // -------------------------------------------------------------------

    /// `fstat(0, &buf)` fills a char-device stat and returns 0.
    /// Asserts the key stubbed fields: st_mode, st_size, st_blksize.
    #[test]
    fn fstat_stdin_fills_stub_stat_and_returns_zero() {
        let mut buf = core::mem::MaybeUninit::<Stat>::uninit();
        let result = handle_fstat(0, buf.as_mut_ptr() as u64);
        assert_eq!(result, 0, "fstat(fd=0) must return 0");
        let s = unsafe { buf.assume_init() };
        assert_eq!(
            s.st_mode,
            S_IFCHR | 0o666,
            "st_mode must be S_IFCHR|0o666 for stdin"
        );
        assert_eq!(s.st_size, 0, "st_size must be 0 for char device");
        assert_eq!(s.st_blksize, 4096, "st_blksize must be 4096");
        assert_eq!(s.st_blocks, 0, "st_blocks must be 0");
        assert_eq!(s.st_ino, 1, "st_ino must be fd+1 = 1 for fd 0");
    }

    /// `fstat(1, &buf)` — stdout — fills char-device stat, returns 0.
    #[test]
    fn fstat_stdout_fills_stub_stat_and_returns_zero() {
        let mut buf = core::mem::MaybeUninit::<Stat>::uninit();
        let result = handle_fstat(1, buf.as_mut_ptr() as u64);
        assert_eq!(result, 0, "fstat(fd=1) must return 0");
        let s = unsafe { buf.assume_init() };
        assert_eq!(s.st_mode, S_IFCHR | 0o666);
        assert_eq!(s.st_ino, 2, "st_ino must be fd+1 = 2 for fd 1");
        assert_eq!(s.st_blksize, 4096);
    }

    /// `fstat(2, &buf)` — stderr — fills char-device stat, returns 0.
    #[test]
    fn fstat_stderr_fills_stub_stat_and_returns_zero() {
        let mut buf = core::mem::MaybeUninit::<Stat>::uninit();
        let result = handle_fstat(2, buf.as_mut_ptr() as u64);
        assert_eq!(result, 0, "fstat(fd=2) must return 0");
        let s = unsafe { buf.assume_init() };
        assert_eq!(s.st_mode, S_IFCHR | 0o666);
        assert_eq!(s.st_ino, 3, "st_ino must be fd+1 = 3 for fd 2");
    }

    // -------------------------------------------------------------------
    // fstat tests — error paths
    // -------------------------------------------------------------------

    /// `fstat(3, &buf)` — unknown fd — returns `-EBADF` (-9).
    #[test]
    fn fstat_unknown_fd_returns_ebadf() {
        let mut buf = core::mem::MaybeUninit::<Stat>::uninit();
        let result = handle_fstat(3, buf.as_mut_ptr() as u64);
        assert_eq!(result, -EBADF, "fstat(fd=3) must return -EBADF");
    }

    /// `fstat(u64::MAX, &buf)` — arbitrary invalid fd — returns `-EBADF`.
    #[test]
    fn fstat_max_fd_returns_ebadf() {
        let mut buf = core::mem::MaybeUninit::<Stat>::uninit();
        let result = handle_fstat(u64::MAX, buf.as_mut_ptr() as u64);
        assert_eq!(result, -EBADF);
    }

    /// `fstat(1, NULL)` — null statbuf — returns `-EFAULT` (-14).
    #[test]
    fn fstat_null_statbuf_returns_efault() {
        let result = handle_fstat(1, 0);
        assert_eq!(result, -EFAULT, "fstat with null statbuf must return -EFAULT");
    }

    // -------------------------------------------------------------------
    // stat tests
    // -------------------------------------------------------------------

    /// `stat(pathname, &buf)` with a non-null pathname fills the stub
    /// char-device stat and returns 0.
    #[test]
    fn stat_non_null_path_fills_stub_and_returns_zero() {
        let path = b"/dev/tty\0";
        let mut buf = core::mem::MaybeUninit::<Stat>::uninit();
        let result = handle_stat(path.as_ptr() as u64, buf.as_mut_ptr() as u64);
        assert_eq!(result, 0, "stat with valid path must return 0");
        let s = unsafe { buf.assume_init() };
        assert_eq!(s.st_mode, S_IFCHR | 0o666);
        assert_eq!(s.st_size, 0);
        assert_eq!(s.st_blksize, 4096);
    }

    /// `stat(NULL, &buf)` — null pathname — returns `-EFAULT`.
    #[test]
    fn stat_null_pathname_returns_efault() {
        let mut buf = core::mem::MaybeUninit::<Stat>::uninit();
        let result = handle_stat(0, buf.as_mut_ptr() as u64);
        assert_eq!(result, -EFAULT, "stat with null pathname must return -EFAULT");
    }

    /// `stat(pathname, NULL)` — null statbuf — returns `-EFAULT`.
    #[test]
    fn stat_null_statbuf_returns_efault() {
        let path = b"/dev/tty\0";
        let result = handle_stat(path.as_ptr() as u64, 0);
        assert_eq!(result, -EFAULT, "stat with null statbuf must return -EFAULT");
    }

    // -------------------------------------------------------------------
    // Dispatch wiring tests
    // -------------------------------------------------------------------

    /// `dispatch(SYS_FSTAT, 1, &buf, ...)` routes to `fstat::handle`
    /// and returns 0 for stdout.
    #[test]
    fn dispatch_sys_fstat_stdout_routes_correctly() {
        use crate::syscall::dispatch::{dispatch, SYS_FSTAT};
        let mut buf = core::mem::MaybeUninit::<Stat>::uninit();
        let result = dispatch(
            SYS_FSTAT,
            1,
            buf.as_mut_ptr() as u64,
            0,
            0,
            0,
            0,
        );
        assert_eq!(result, 0);
        let s = unsafe { buf.assume_init() };
        assert_eq!(s.st_mode, S_IFCHR | 0o666);
        assert_eq!(s.st_blksize, 4096);
    }

    /// `dispatch(SYS_FSTAT, 99, &buf, ...)` — unknown fd — returns
    /// `-EBADF` via the dispatch table.
    #[test]
    fn dispatch_sys_fstat_unknown_fd_returns_ebadf() {
        use crate::syscall::dispatch::{dispatch, SYS_FSTAT};
        let mut buf = core::mem::MaybeUninit::<Stat>::uninit();
        let result = dispatch(
            SYS_FSTAT,
            99,
            buf.as_mut_ptr() as u64,
            0,
            0,
            0,
            0,
        );
        assert_eq!(result, -EBADF);
    }

    /// `dispatch(SYS_STAT, pathname, &buf, ...)` routes to `stat::handle`
    /// and returns 0 for any non-null path (tier-1 stub).
    #[test]
    fn dispatch_sys_stat_routes_correctly() {
        use crate::syscall::dispatch::{dispatch, SYS_STAT};
        let path = b"/dev/stdin\0";
        let mut buf = core::mem::MaybeUninit::<Stat>::uninit();
        let result = dispatch(
            SYS_STAT,
            path.as_ptr() as u64,
            buf.as_mut_ptr() as u64,
            0,
            0,
            0,
            0,
        );
        assert_eq!(result, 0);
        let s = unsafe { buf.assume_init() };
        assert_eq!(s.st_mode, S_IFCHR | 0o666);
    }

    /// `dispatch(SYS_STAT, 0, ...)` — null pathname — returns `-EFAULT`.
    #[test]
    fn dispatch_sys_stat_null_pathname_returns_efault() {
        use crate::syscall::dispatch::{dispatch, SYS_STAT};
        let mut buf = core::mem::MaybeUninit::<Stat>::uninit();
        let result = dispatch(
            SYS_STAT,
            0,
            buf.as_mut_ptr() as u64,
            0,
            0,
            0,
            0,
        );
        assert_eq!(result, -EFAULT);
    }

    // -------------------------------------------------------------------
    // Heap-buffer test (mirrors ioctl.rs pattern)
    // -------------------------------------------------------------------

    /// `fstat` into a heap-allocated mock Stat — mirrors the ioctl.rs
    /// `dispatch_sys_ioctl_tiocgwinsz_routes_correctly` heap-buffer
    /// pattern for completeness.
    #[test]
    fn fstat_into_heap_stat_buffer_matches_expected_fields() {
        let mut heap_stat: alloc::boxed::Box<core::mem::MaybeUninit<Stat>> =
            alloc::boxed::Box::new(core::mem::MaybeUninit::uninit());
        let result = handle_fstat(0, heap_stat.as_mut_ptr() as u64);
        assert_eq!(result, 0);
        let s = unsafe { heap_stat.assume_init_ref() };
        assert_eq!(s.st_mode, S_IFCHR | 0o666);
        assert_eq!(s.st_blksize, 4096);
        assert_eq!(s.st_ino, 1);
        assert_eq!(s.st_nlink, 1);
        assert_eq!(s.st_uid, 0);
        assert_eq!(s.st_gid, 0);
    }
}
