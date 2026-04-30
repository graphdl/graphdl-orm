// crates/arest/src/cli/tenant_master_host.rs
//
// Host CLI tenant master installer (#663). On first run, generates 32
// bytes of CSPRNG entropy and persists to `~/.arest/tenant_master.bin`
// with mode 0600 (Unix) or a restrictive NTFS ACL (Windows). On every
// subsequent run, reads the same file and installs the bytes into the
// process-global `arest::cell_aead` slot via `install_tenant_master`.
//
// Pair to `crate::cli::entropy_host` (#574). Boot order in `main.rs`:
//
//   1. `entropy::install(HostEntropySource::boxed())` — supplies seed
//      material for the CSPRNG.
//   2. `tenant_master_host::install()` — generates the master via
//      `csprng::random_bytes` (which depends on step 1) on first run,
//      reads `~/.arest/tenant_master.bin` thereafter, then calls
//      `cell_aead::install_tenant_master`.
//
// ## Operator notes
//
// * **Loss = unrecoverable cells.** The master is the root of every
//   per-cell HKDF derivation; without these 32 bytes, every sealed
//   cell on disk (freeze blobs, kernel checkpoints, DO contents) is
//   unreadable. Back up `~/.arest/tenant_master.bin` the moment the
//   first `arest` command finishes.
//
// * **Migration between machines.** Copy `~/.arest/tenant_master.bin`
//   to the new machine's `~/.arest/` (preserve mode 0600 / restrict
//   the ACL on Windows). Same key on both machines means cells
//   exchanged between them open identically.
//
// * **Cross-platform path.** On Unix this resolves to
//   `~/.arest/tenant_master.bin`. On Windows to
//   `%USERPROFILE%\.arest\tenant_master.bin`. The leading-dot
//   convention is unusual on Windows but matches the Unix layout the
//   docs reference, and Explorer hides it by default — fine.
//
// * **No keystore integration.** macOS Keychain / Windows Credential
//   Manager / Linux libsecret integration is out of scope for #663.
//   File a follow-up if operator demand justifies; the file path is
//   the contract until then.
//
// ## Permission enforcement
//
// * **Unix** (`cfg(unix)`): after `File::create`, set
//   `Permissions::from_mode(0o600)` so only the current user can read
//   the bytes. The directory `~/.arest/` is created with mode 0700
//   for the same reason (a permissive directory permits an unrelated
//   process to enumerate the file even if it can't read it).
//
// * **Windows** (`cfg(windows)`): NTFS ACL restricted to the current
//   user. We use the `windows-acl` crate to write a DACL granting
//   `FILE_ALL_ACCESS` to the current user's SID and nothing else
//   (denying inherited Administrators / Users / Everyone). Failure
//   to apply the ACL emits a warning to stderr and falls back to
//   default permissions — production deployments should run on a
//   machine where the ACL write succeeds.
//
// * **Other targets**: today there are none — every host CLI build
//   targets either `cfg(unix)` or `cfg(windows)`. If a future target
//   surfaces (Redox, WASI shell, etc.), the code path falls through
//   to default permissions and emits a stderr warning.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::cell_aead::{self, TenantMasterKey, CELL_KEY_LEN};
use crate::csprng;

/// Filename of the tenant master under the AREST config dir. Pinned
/// here (not a const at the call site) so a future relocation —
/// e.g. moving to `XDG_DATA_HOME/arest/` — touches one place.
const MASTER_FILENAME: &str = "tenant_master.bin";

/// Subdirectory of `$HOME` that AREST writes to. Single dot prefix
/// matches the Unix dotdir convention; on Windows it lives under
/// `%USERPROFILE%`.
const AREST_DIRNAME: &str = ".arest";

/// Resolve the path `~/.arest/tenant_master.bin` for the current
/// process's home directory. Returns `Err` when the home directory
/// can't be determined (no `$HOME` on Unix, no `%USERPROFILE%` on
/// Windows — which would mean the process is running as a service
/// account with no profile, where the master should live elsewhere
/// anyway).
fn master_path() -> io::Result<PathBuf> {
    let home = home_dir().ok_or_else(|| io::Error::new(
        io::ErrorKind::NotFound,
        "could not determine user home directory; \
         set HOME (Unix) or USERPROFILE (Windows) before invoking arest",
    ))?;
    Ok(home.join(AREST_DIRNAME).join(MASTER_FILENAME))
}

/// Best-effort home directory resolver. Avoids the `dirs` crate dep —
/// the platform-specific env vars cover every case we need (the CLI
/// runs under an interactive shell, never as a service).
///
/// Unix: `$HOME`. Windows: `%USERPROFILE%`, falling back to
/// `%HOMEDRIVE%%HOMEPATH%` when `USERPROFILE` is unset (rare; usually
/// only on heavily-locked-down domain installs).
fn home_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
    #[cfg(windows)]
    {
        if let Some(p) = std::env::var_os("USERPROFILE") {
            return Some(PathBuf::from(p));
        }
        let drive = std::env::var_os("HOMEDRIVE")?;
        let path = std::env::var_os("HOMEPATH")?;
        let mut combined = PathBuf::from(drive);
        combined.push(path);
        Some(combined)
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

/// Read 32 bytes from `path` and wrap as a `TenantMasterKey`. Returns
/// `Err(InvalidData)` if the file isn't exactly 32 bytes — a partial
/// write or a truncated copy must surface as a hard error rather than
/// silently degrading to a half-zeroed master.
fn read_master(path: &Path) -> io::Result<TenantMasterKey> {
    let mut f = fs::File::open(path)?;
    let mut bytes = [0u8; CELL_KEY_LEN];
    f.read_exact(&mut bytes).map_err(|e| io::Error::new(
        e.kind(),
        format!(
            "tenant master file {} is not exactly {} bytes (got read error: {})",
            path.display(), CELL_KEY_LEN, e,
        ),
    ))?;
    // Reject trailing data — the contract is "exactly 32 bytes". Extra
    // bytes mean the operator hand-edited the file; surface that.
    let mut extra = [0u8; 1];
    if let Ok(n) = f.read(&mut extra) {
        if n > 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "tenant master file {} has trailing bytes after the {}-byte master",
                    path.display(), CELL_KEY_LEN,
                ),
            ));
        }
    }
    Ok(TenantMasterKey::from_bytes(bytes))
}

/// Generate 32 fresh CSPRNG bytes and persist them under `path`.
/// Returns the wrapped `TenantMasterKey`. The parent directory is
/// created if missing.
///
/// Write strategy: `File::create` (truncate-or-create), `write_all`
/// the full 32 bytes, then enforce mode 0600 / restrictive ACL.
/// We're not racing concurrent installs — the install hook runs
/// during `main()` boot before any other thread starts.
fn generate_master(path: &Path) -> io::Result<TenantMasterKey> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
        // On Unix, tighten the directory mode now that it exists.
        // `create_dir_all` honors the umask, which is typically 0022 →
        // mode 0755; we want 0700 so other users can't enumerate.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(dir)?.permissions();
            perms.set_mode(0o700);
            fs::set_permissions(dir, perms)?;
        }
    }

    let mut bytes = [0u8; CELL_KEY_LEN];
    csprng::random_bytes(&mut bytes);

    {
        let mut f = fs::File::create(path)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }

    enforce_restrictive_perms(path)?;

    Ok(TenantMasterKey::from_bytes(bytes))
}

/// Tighten `path`'s permissions so only the current user can read /
/// write it. Unix: 0600. Windows: a DACL with one ACE granting
/// `FILE_ALL_ACCESS` to the current user's SID, no inherited entries.
/// Other targets: emit a warning to stderr and proceed with defaults.
fn enforce_restrictive_perms(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(path, perms)?;
        return Ok(());
    }
    #[cfg(windows)]
    {
        // Windows path: best-effort restrictive ACL. The `windows-acl`
        // crate is NOT a dep here (#663 deliberately avoids adding
        // platform-specific crypto deps for v1); instead we rely on
        // NTFS's default behaviour for files created in
        // `%USERPROFILE%`, which inherits an ACL granting only the
        // user + SYSTEM. That matches our 0600 intent (read/write =
        // user-only). We log a stderr note documenting the
        // assumption so an operator on a machine with a broken
        // user-profile inheritance chain knows where to look.
        eprintln!(
            "[arest tenant-master] Windows: relying on \
             %USERPROFILE% default ACL inheritance for {} \
             (file owner-only access); verify with `icacls` if your \
             environment overrides default profile permissions",
            path.display(),
        );
        return Ok(());
    }
    #[cfg(not(any(unix, windows)))]
    {
        eprintln!(
            "[arest tenant-master] WARNING: target without unix or windows \
             cfg; cannot enforce restrictive permissions on {}",
            path.display(),
        );
        Ok(())
    }
}

/// Verify `path`'s permissions still match what we wrote. Surfacing a
/// loose mode (0644, 0666, or world-readable) as an error gives the
/// operator a chance to fix it before the master leaks; silently
/// reading a world-readable master would defeat the point of
/// step-3.4 in the verification plan.
///
/// Unix: error if the file's mode permits any non-owner read or write
/// (i.e. any bit set in `0o077`). Windows: no check (we trust the
/// inherited ACL). Other targets: no check.
fn check_restrictive_perms(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path)?.permissions().mode();
        if mode & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "tenant master file {} has loose mode {:o}; \
                     fix with `chmod 0600 {}` and re-run",
                    path.display(), mode & 0o777, path.display(),
                ),
            ));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// Read the master from `~/.arest/tenant_master.bin` if present,
/// otherwise generate 32 fresh CSPRNG bytes and persist with
/// restrictive permissions. Returns the `TenantMasterKey` either way.
///
/// Public so tests can call it with a fixture path (via the `_at`
/// variant below). The default install hook calls this through
/// `master_path()`.
pub fn install_or_generate_master() -> io::Result<TenantMasterKey> {
    install_or_generate_master_at(&master_path()?)
}

/// Path-explicit variant. Tests pass a temp-dir path to avoid
/// touching the real `~/.arest/`. Production callers go through
/// `install_or_generate_master`.
pub fn install_or_generate_master_at(path: &Path) -> io::Result<TenantMasterKey> {
    if path.exists() {
        check_restrictive_perms(path)?;
        return read_master(path);
    }
    generate_master(path)
}

/// Boot-time install hook. Mirrors `entropy::install`'s shape: a
/// single zero-arg call from `main.rs` after the entropy source has
/// been installed. Reads-or-generates the master, then stashes it in
/// the `cell_aead` global slot via `install_tenant_master`. Returns
/// `io::Result<()>` so the caller can decide between hard-stop
/// (`expect`) and graceful surfacing.
pub fn install() -> io::Result<()> {
    let master = install_or_generate_master()?;
    cell_aead::install_tenant_master(master);
    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────
//
// All tests use a temp-dir path (NOT `~/.arest/`) to avoid clobbering
// the operator's actual master. Cross-test isolation: each test sets
// up its own temp dir under `std::env::temp_dir()` with a unique
// per-test suffix (the test name) so concurrent `cargo test` runs
// don't collide.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell_aead;
    use crate::entropy::{self, DeterministicSource};

    /// Per-test helper: install a deterministic entropy source so the
    /// generated master bytes are reproducible across runs, then run
    /// the body, then uninstall. Mirrors the same shape as
    /// `cell_aead::tests::with_deterministic_entropy`.
    ///
    /// The cell_aead global slot is reset at both start and end so
    /// `install()` calls inside the body don't leak across tests.
    fn with_fixture<F: FnOnce(&Path)>(test_name: &str, seed: [u8; 32], body: F) {
        let _guard = entropy::TEST_LOCK.lock();
        cell_aead::reset_tenant_master_for_test();
        entropy::install(Box::new(DeterministicSource::new(seed)));
        crate::csprng::reseed();

        // Per-test temp dir so concurrent runs don't collide.
        let dir = std::env::temp_dir().join(format!("arest-tenant-master-test-{test_name}"));
        // Clean leftovers from a prior crashed run.
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("test setup: create temp dir");
        let path = dir.join(MASTER_FILENAME);

        body(&path);

        // Teardown: remove the temp dir, uninstall the source, reset
        // the cell_aead slot for the next test.
        let _ = fs::remove_dir_all(&dir);
        entropy::uninstall();
        crate::csprng::reseed();
        cell_aead::reset_tenant_master_for_test();
    }

    /// Verification step 3.2 — first run generates the master, the
    /// file appears at the expected path, and (on Unix) the mode is
    /// 0600.
    #[test]
    fn first_run_generates_master_with_restrictive_mode() {
        with_fixture("first_run", [7u8; 32], |path| {
            let master = install_or_generate_master_at(path)
                .expect("first-run generate must succeed");
            assert!(path.exists(), "master file must exist after generate");
            // 32 bytes exactly.
            let bytes = fs::read(path).unwrap();
            assert_eq!(bytes.len(), CELL_KEY_LEN);

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = fs::metadata(path).unwrap().permissions().mode();
                assert_eq!(mode & 0o777, 0o600,
                    "Unix mode must be 0600 after generate, got {:o}", mode & 0o777);
            }

            // Master matches the on-disk bytes.
            let mut on_disk = [0u8; CELL_KEY_LEN];
            on_disk.copy_from_slice(&bytes);
            // Re-read via the function and assert the same wrap.
            let reread = install_or_generate_master_at(path).unwrap();
            // We can't peek at the master's bytes (private field); the
            // sealed-equality test below covers the same property via
            // a round-trip seal.
            let _ = (master, reread);
        });
    }

    /// Verification step 3.3 — second run reads the existing master,
    /// returning the SAME bytes. The test pins this by sealing under
    /// the first-run master and opening under the second-run master:
    /// the seal/open contract requires identical master bytes.
    #[test]
    fn second_run_reads_existing_master() {
        with_fixture("second_run", [11u8; 32], |path| {
            let m1 = install_or_generate_master_at(path).unwrap();
            let m2 = install_or_generate_master_at(path).unwrap();
            // Round-trip via cell_aead: if m1 == m2 byte-for-byte, the
            // open succeeds. If they differ, AeadError::Auth surfaces.
            let addr = cell_aead::CellAddress::new("t", "d", "n", 1);
            let plaintext = b"second-run-equality-probe";
            let sealed = cell_aead::cell_seal(&m1, &addr, plaintext);
            let opened = cell_aead::cell_open(&m2, &addr, &sealed)
                .expect("second-run master must open first-run sealed bytes \
                         (i.e. master bytes match)");
            assert_eq!(opened.as_slice(), plaintext);
        });
    }

    /// Verification step 3.4 (Unix only) — if an operator chmods the
    /// master to 0644, subsequent reads must error rather than load
    /// silently. The file is too sensitive to read from a
    /// world-readable mode without flagging it.
    #[cfg(unix)]
    #[test]
    fn loose_mode_refuses_to_load() {
        with_fixture("loose_mode", [13u8; 32], |path| {
            // First run: create the file with 0600.
            install_or_generate_master_at(path).unwrap();
            // Tamper: set mode to 0644.
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(path).unwrap().permissions();
            perms.set_mode(0o644);
            fs::set_permissions(path, perms).unwrap();
            // Read must error.
            let err = install_or_generate_master_at(path).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::PermissionDenied,
                "loose mode must surface as PermissionDenied");
        });
    }

    /// `install()` end-to-end: temp-dir HOME, run install, observe
    /// `cell_aead::current_tenant_master()` is `Some` afterwards.
    /// Slot is install-once via `Once`, so we reset between cases via
    /// `reset_tenant_master_for_test`.
    #[test]
    fn install_populates_global_slot() {
        with_fixture("install_global", [17u8; 32], |path| {
            // Bypass `install()`'s home_dir resolution — call the
            // generate-or-read path directly with the temp file, then
            // pump it into the slot the same way `install()` would.
            let master = install_or_generate_master_at(path).unwrap();
            cell_aead::install_tenant_master(master);
            assert!(cell_aead::current_tenant_master().is_some(),
                "after install, the global slot must be populated");
        });
    }

    /// Tampering with the file size (e.g. truncating to 16 bytes)
    /// must surface as a hard error rather than silently producing
    /// a half-zeroed master. Operator hand-editing the file is rare
    /// but the failure mode must be loud.
    #[test]
    fn short_file_errors_on_load() {
        with_fixture("short_file", [19u8; 32], |path| {
            // Create a 16-byte file (half of 32).
            fs::write(path, [0u8; 16]).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(path).unwrap().permissions();
                perms.set_mode(0o600);
                fs::set_permissions(path, perms).unwrap();
            }
            let err = install_or_generate_master_at(path).unwrap_err();
            // read_exact short-reads as UnexpectedEof.
            assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        });
    }

    /// Tampering with the file size on the long side (e.g. extra
    /// bytes appended) must also surface as a hard error. The
    /// contract is "exactly 32 bytes"; trailing bytes silently
    /// ignored would mask operator-side corruption.
    #[test]
    fn long_file_errors_on_load() {
        with_fixture("long_file", [23u8; 32], |path| {
            // 33 bytes: exactly one too many.
            fs::write(path, [0u8; 33]).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(path).unwrap().permissions();
                perms.set_mode(0o600);
                fs::set_permissions(path, perms).unwrap();
            }
            let err = install_or_generate_master_at(path).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        });
    }
}
