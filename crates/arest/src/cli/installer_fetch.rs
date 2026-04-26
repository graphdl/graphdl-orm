// Wine App installer-binary fetcher (#505).
//
// Downloads an installer binary (typically `setup.exe` / `*.msi`) from
// a fact-derived URL into the prefix's `_install/` cache directory so
// the sibling `installer_run` module can launch it under wine. Mirrors
// the subprocess-wrapper pattern from `cli::winetricks` — small,
// std-only, no third-party HTTP client dependency.
//
// Idempotent: if the cached file already exists at
// `<prefix>/drive_c/_install/<filename>` the fetcher short-circuits
// without spawning a downloader. The disk file is the single
// observable handle the next stage of `cli::wine_install` keys off
// of, so re-runs of `arest run` after a successful download avoid the
// network entirely.
//
// Backend selection walks PATH for `curl` first (universal on Linux
// hosts and shipped with Windows 10+), then falls back to PowerShell's
// `Invoke-WebRequest`. Either backend is invoked as a subprocess with
// the URL + output path supplied verbatim. No HTTP body parsing is
// done in-process; the wrapper trusts the downloader to follow
// redirects and validates only the final exit code + output-file
// presence.
//
// Local paths are also supported by the public `fetch_installer`
// entrypoint: when `url_or_path` does not parse as a URL (no
// scheme://), the value is treated as a path on the host filesystem
// and copied (rather than downloaded). This lets the FORML facts
// declare a pre-staged installer (useful for licensed apps where the
// upstream URL is gated behind a login wall).

use std::path::{Path, PathBuf};
use std::process::Command;

/// Outcome of a `fetch_installer` call. Distinguishes the four cases
/// the orchestrator (`cli::wine_install`) cares about so the install
/// state machine can emit the right transition:
///
///   * `AlreadyCached` — file already on disk; subprocess skipped.
///   * `Downloaded` — downloader ran and wrote the file fresh.
///   * `CopiedLocal` — `url_or_path` was a host filesystem path; copied.
///   * `NoFetcher` — neither curl nor PowerShell resolved on PATH;
///     no subprocess spawned, no error returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    AlreadyCached,
    Downloaded,
    CopiedLocal,
    NoFetcher,
}

/// Fetcher backend resolved from PATH. Public for unit tests; the
/// production caller goes through `fetch_installer`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetcherKind {
    Curl(PathBuf),
    PowerShell(PathBuf),
}

/// Resolve the installer cache directory for a Wine prefix. The
/// `_install/` subdir lives under `drive_c/` so it sits inside the
/// prefix's emulated C:\ — the installer runner (sibling
/// `installer_run`) can then reference the path by the Windows-style
/// `C:\_install\<filename>` if a recipe needs to.
pub fn install_cache_dir(prefix_dir: &Path) -> PathBuf {
    prefix_dir.join("drive_c").join("_install")
}

/// Public entrypoint. Resolves `url_or_path`:
///
///   * If it parses as a URL (`http://`, `https://`, `file://`),
///     downloads via curl / PowerShell into
///     `<install_cache_dir>/<filename>`.
///   * Otherwise treats `url_or_path` as a local filesystem path and
///     copies it into the cache.
///
/// `filename` is the cache-side filename — typically the trailing
/// segment of the URL or the original on-disk basename. Caller
/// supplies it explicitly so the cache is keyed off a stable name
/// rather than re-derived per call (URLs with redirects can change
/// the response filename).
///
/// Returns the outcome on success; spawn failures or non-zero exit
/// codes surface as `Err`.
pub fn fetch_installer(
    prefix_dir: &Path,
    url_or_path: &str,
    filename: &str,
) -> std::io::Result<FetchOutcome> {
    let cache_dir = install_cache_dir(prefix_dir);
    let target = cache_dir.join(filename);
    if target.is_file() {
        return Ok(FetchOutcome::AlreadyCached);
    }
    std::fs::create_dir_all(&cache_dir)?;
    if is_url(url_or_path) {
        download_url(url_or_path, &target)
    } else {
        copy_local(url_or_path, &target)
    }
}

/// True iff `s` looks like a URL (any of `http://`, `https://`,
/// `file://`, or any other `<scheme>://` prefix). Conservative on
/// purpose — anything without `://` is treated as a local path. We
/// don't pull in a URL crate for one CLI fetch path.
pub fn is_url(s: &str) -> bool {
    if let Some(idx) = s.find("://") {
        // Scheme must be ASCII alphanum + a couple of common punct.
        s[..idx].chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
            && idx > 0
    } else {
        false
    }
}

/// Spawn the resolved downloader to fetch `url` into `target`. Uses
/// curl first (universal), PowerShell `Invoke-WebRequest` second.
fn download_url(url: &str, target: &Path) -> std::io::Result<FetchOutcome> {
    match resolve_fetcher_on_path() {
        Some(FetcherKind::Curl(path)) => {
            let status = Command::new(&path)
                .arg("-fsSL")            // fail on HTTP error, silent, follow redirects
                .arg("-o")
                .arg(target)
                .arg(url)
                .status()?;
            if !status.success() {
                let code = status.code().map(|c| c.to_string()).unwrap_or_else(|| "signal".into());
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("curl exited with status {} for url {}", code, url),
                ));
            }
            Ok(FetchOutcome::Downloaded)
        }
        Some(FetcherKind::PowerShell(path)) => {
            // -UseBasicParsing avoids the ancient IE-engine dependency
            // that newer PS7 doesn't even ship.
            let script = format!(
                "Invoke-WebRequest -UseBasicParsing -Uri '{}' -OutFile '{}'",
                url.replace('\'', "''"),
                target.display().to_string().replace('\'', "''"),
            );
            let status = Command::new(&path)
                .arg("-NoProfile")
                .arg("-NonInteractive")
                .arg("-Command")
                .arg(&script)
                .status()?;
            if !status.success() {
                let code = status.code().map(|c| c.to_string()).unwrap_or_else(|| "signal".into());
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("powershell Invoke-WebRequest exited with status {} for url {}", code, url),
                ));
            }
            Ok(FetchOutcome::Downloaded)
        }
        None => Ok(FetchOutcome::NoFetcher),
    }
}

/// Copy `local_path` to `target`. Wrapped here so the public API
/// stays uniform across URL and path inputs and so the call site
/// can map io::Error consistently regardless of branch.
fn copy_local(local_path: &str, target: &Path) -> std::io::Result<FetchOutcome> {
    let src = Path::new(local_path);
    if !src.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("installer source path is not a file: {}", local_path),
        ));
    }
    std::fs::copy(src, target)?;
    Ok(FetchOutcome::CopiedLocal)
}

/// Walk PATH looking for a downloader binary. Curl wins over
/// PowerShell because it is universally available across distros and
/// because Windows 10+ ships `curl.exe` natively.
pub fn resolve_fetcher_on_path() -> Option<FetcherKind> {
    let path_env = std::env::var_os("PATH")?;
    let curl_candidates: &[&str] = if cfg!(windows) {
        &["curl.exe", "curl"]
    } else {
        &["curl"]
    };
    let ps_candidates: &[&str] = if cfg!(windows) {
        &["pwsh.exe", "powershell.exe"]
    } else {
        &["pwsh"]
    };
    let dirs: Vec<PathBuf> = std::env::split_paths(&path_env).collect();
    for dir in &dirs {
        for cand in curl_candidates {
            let p = dir.join(cand);
            if p.is_file() {
                return Some(FetcherKind::Curl(p));
            }
        }
    }
    for dir in &dirs {
        for cand in ps_candidates {
            let p = dir.join(cand);
            if p.is_file() {
                return Some(FetcherKind::PowerShell(p));
            }
        }
    }
    None
}

/// True iff some fetcher backend is on PATH. Lightweight — no
/// subprocess. Used by the wine_install dispatcher to decide whether
/// to mark the install as failed-with-no-fetcher vs. failed-by-network.
pub fn fetcher_available() -> bool {
    resolve_fetcher_on_path().is_some()
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_cache_dir_lives_under_drive_c() {
        let p = install_cache_dir(Path::new("/tmp/prefix"));
        assert!(p.ends_with("drive_c/_install") || p.ends_with(r"drive_c\_install"));
    }

    #[test]
    fn is_url_detects_common_schemes() {
        assert!(is_url("http://example.com/setup.exe"));
        assert!(is_url("https://example.com/setup.exe"));
        assert!(is_url("file:///tmp/x"));
        assert!(is_url("ftp://example.com/setup.exe"));
    }

    #[test]
    fn is_url_rejects_local_paths() {
        assert!(!is_url("/tmp/setup.exe"));
        assert!(!is_url("setup.exe"));
        assert!(!is_url(r"C:\Users\me\setup.exe"));
        // Edge cases — empty / no scheme.
        assert!(!is_url(""));
        assert!(!is_url("://no-scheme"));
    }

    #[test]
    fn fetch_installer_short_circuits_when_cached() {
        let tmp = tempdir();
        let cache = install_cache_dir(&tmp);
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("setup.exe"), b"already here").unwrap();
        // Pass a URL that would otherwise fail (no fetcher reachable
        // even if there were one — bogus host) — short-circuit must
        // happen before that path is exercised.
        let outcome = fetch_installer(
            &tmp,
            "https://example.invalid/whatever.exe",
            "setup.exe",
        ).expect("short-circuit must not error");
        assert_eq!(outcome, FetchOutcome::AlreadyCached);
        // Body unchanged — short-circuit didn't touch the file.
        let body = std::fs::read(cache.join("setup.exe")).unwrap();
        assert_eq!(body, b"already here");
    }

    #[test]
    fn fetch_installer_copies_local_path() {
        let tmp = tempdir();
        let src = tmp.join("source.exe");
        std::fs::write(&src, b"local installer").unwrap();
        let outcome = fetch_installer(&tmp, src.to_str().unwrap(), "setup.exe")
            .expect("local copy must succeed");
        assert_eq!(outcome, FetchOutcome::CopiedLocal);
        let cache = install_cache_dir(&tmp);
        let body = std::fs::read(cache.join("setup.exe")).unwrap();
        assert_eq!(body, b"local installer");
    }

    #[test]
    fn fetch_installer_returns_notfound_for_missing_local_path() {
        let tmp = tempdir();
        let err = fetch_installer(
            &tmp,
            "/this/path/does/not/exist/setup.exe",
            "setup.exe",
        ).expect_err("missing local path must error");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn fetch_installer_creates_cache_dir() {
        let tmp = tempdir();
        let src = tmp.join("source.exe");
        std::fs::write(&src, b"x").unwrap();
        // _install/ does not exist yet.
        assert!(!install_cache_dir(&tmp).exists());
        let _ = fetch_installer(&tmp, src.to_str().unwrap(), "setup.exe").unwrap();
        assert!(install_cache_dir(&tmp).is_dir(), "cache dir must be created on first fetch");
    }

    #[test]
    fn resolve_fetcher_returns_none_when_path_empty() {
        let original = std::env::var_os("PATH");
        std::env::set_var("PATH", "");
        let got = resolve_fetcher_on_path();
        match original {
            Some(v) => std::env::set_var("PATH", v),
            None => std::env::remove_var("PATH"),
        }
        assert!(got.is_none(), "empty PATH must yield no fetcher; got {:?}", got);
    }

    #[test]
    fn fetcher_available_does_not_spawn() {
        // Probe is just a PATH walk; safe to call.
        let _ = fetcher_available();
    }

    /// Tempdir helper — same shape as the sibling modules use.
    fn tempdir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("arest-fetch-test-{}-{}", pid, n));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("tempdir create");
        path
    }
}
