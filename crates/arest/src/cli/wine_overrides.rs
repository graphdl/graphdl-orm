// DLL override + registry-key writers for Wine prefix bootstrap (#504).
//
// Two kinds of fact-driven prefix mutation are handled here:
//
//   * DLL overrides (`Wine App requires DLL Override of DLL Name 'X'
//     with DLL Behavior 'Y'`) — written to the prefix's `system.reg`
//     under `[Software\\Wine\\DllOverrides]`.
//
//   * Registry keys (`Wine App requires Registry Key at Registry Path
//     'P' with Registry Value 'V'`) — split by root key (HKCU vs HKLM)
//     and written to `user.reg` / `system.reg` respectively.
//
// Both fact types are FORML 2 ternaries; the third role (Behavior /
// Value) is currently dropped by the stage-1 parser (see
// `_reports/wine_dump_cells` introspection — only the first two roles
// land in cells; the `with X 'Y'` tail is lost). The bootstrap layer
// therefore reads the raw wine.md reading text alongside the parsed
// state to recover the full ternary tuples for the `requires` ↔
// `with` pairs, then emits the matching .reg fragments.
//
// Idempotent: the writers replace the entire `[Software\\Wine\\
// DllOverrides]` section block on every run rather than appending,
// so re-running with the same fact set produces a byte-identical
// file. Registry-key writes use `regedit /S`-style content; the
// parser-format `.reg` file Wine reads at boot is the same shape
// `wine reg add` produces, so a second run overwrites the same key
// path and value.
//
// No `wine` execution required from this module — the writers touch
// only the prefix filesystem. winetricks invocation lives in the
// sibling `winetricks` module.

use std::collections::BTreeMap;
use std::path::Path;

/// Apply every DLL override declared for `app_id` in `wine_md_text` to
/// the prefix at `prefix_path`. Returns the number of override entries
/// applied (zero if the app has none, which is the common case for
/// platinum-rated apps).
///
/// The prefix's `system.reg` is rewritten with a fresh
/// `[Software\\Wine\\DllOverrides]` section assembled from the
/// declared facts. Existing keys outside that section are preserved
/// verbatim. If `system.reg` doesn't exist yet (fresh prefix),
/// a minimal one is created with just the overrides section.
pub fn apply_dll_overrides(
    wine_md_text: &str,
    app_id: &str,
    prefix_path: &Path,
) -> std::io::Result<usize> {
    let overrides = parse_dll_overrides(wine_md_text, app_id);
    if overrides.is_empty() {
        return Ok(0);
    }
    let reg_path = prefix_path.join("system.reg");
    let existing = std::fs::read_to_string(&reg_path).unwrap_or_else(|_| reg_header());
    let new_content = rewrite_dll_overrides_section(&existing, &overrides);
    std::fs::create_dir_all(prefix_path)?;
    std::fs::write(&reg_path, new_content)?;
    Ok(overrides.len())
}

/// Apply every registry key declared for `app_id` in `wine_md_text` to
/// the prefix at `prefix_path`. Returns the number of registry keys
/// applied. HKCU keys go to `user.reg`; HKLM / HKCR / HKU keys go to
/// `system.reg`. Other roots are skipped with a logged warning.
pub fn apply_registry_keys(
    wine_md_text: &str,
    app_id: &str,
    prefix_path: &Path,
) -> std::io::Result<usize> {
    let keys = parse_registry_keys(wine_md_text, app_id);
    if keys.is_empty() {
        return Ok(0);
    }
    let mut user_keys: Vec<&RegistryKey> = Vec::new();
    let mut system_keys: Vec<&RegistryKey> = Vec::new();
    for k in &keys {
        match k.root.as_str() {
            "HKCU" | "HKEY_CURRENT_USER" => user_keys.push(k),
            "HKLM" | "HKEY_LOCAL_MACHINE" | "HKCR" | "HKEY_CLASSES_ROOT"
            | "HKU" | "HKEY_USERS" => system_keys.push(k),
            other => {
                eprintln!("[wine_overrides] skipping unknown registry root '{}' for app '{}'",
                          other, app_id);
            }
        }
    }
    std::fs::create_dir_all(prefix_path)?;
    if !user_keys.is_empty() {
        let path = prefix_path.join("user.reg");
        let existing = std::fs::read_to_string(&path).unwrap_or_else(|_| reg_header());
        let new_content = rewrite_registry_keys(&existing, &user_keys);
        std::fs::write(&path, new_content)?;
    }
    if !system_keys.is_empty() {
        let path = prefix_path.join("system.reg");
        let existing = std::fs::read_to_string(&path).unwrap_or_else(|_| reg_header());
        let new_content = rewrite_registry_keys(&existing, &system_keys);
        std::fs::write(&path, new_content)?;
    }
    Ok(keys.len())
}

/// Parsed DLL override entry: `dll = behavior` (e.g. `comdlg32 = native,builtin`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DllOverride {
    /// DLL stem without the `.dll` suffix — Wine's
    /// `[DllOverrides]` registry section keys DLLs by stem only.
    pub dll_stem: String,
    /// Behavior string in Wine's WINEDLLOVERRIDES grammar:
    /// `native`, `builtin`, `native,builtin`, `builtin,native`,
    /// `disabled` (rendered as the empty string in the registry).
    pub behavior: String,
}

/// Parse all `Wine App 'X' requires DLL Override of DLL Name 'D' with DLL Behavior 'B'`
/// instance facts for `app_id` from the raw reading text. Returns a
/// deterministic BTreeMap (DLL stem → behavior) so re-runs produce
/// byte-identical output.
pub fn parse_dll_overrides(wine_md_text: &str, app_id: &str) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    let needle_app = format!("Wine App '{}' requires DLL Override of DLL Name '", app_id);
    for line in wine_md_text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix(&needle_app) else { continue };
        // `<dll>' with DLL Behavior '<behavior>'.`
        let Some(dll_end) = rest.find('\'') else { continue };
        let dll_name = &rest[..dll_end];
        let after_dll = &rest[dll_end + 1..];
        let needle_with = " with DLL Behavior '";
        let Some(behavior_start) = after_dll.find(needle_with) else { continue };
        let after_with = &after_dll[behavior_start + needle_with.len()..];
        let Some(behavior_end) = after_with.find('\'') else { continue };
        let behavior = &after_with[..behavior_end];
        let dll_stem = dll_name.strip_suffix(".dll").unwrap_or(dll_name);
        out.insert(dll_stem.to_string(), behavior_to_wine_format(behavior));
    }
    out
}

/// Translate a FORML DLL Behavior value into the Wine-registry
/// override string. `'native-then-builtin'` ↔ `'native,builtin'`,
/// `'disabled'` ↔ `''` (empty), etc.
pub fn behavior_to_wine_format(b: &str) -> String {
    match b {
        "native" => "native".to_string(),
        "builtin" => "builtin".to_string(),
        "native-then-builtin" => "native,builtin".to_string(),
        "builtin-then-native" => "builtin,native".to_string(),
        "disabled" => "".to_string(),
        other => other.to_string(),
    }
}

/// Parsed registry-key entry: `[<root>\<path>] "<name>"="<value>"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryKey {
    /// Root key abbreviation: `HKCU`, `HKLM`, `HKCR`, `HKU`, etc.
    pub root: String,
    /// Subpath under the root (no leading backslash). Backslashes
    /// are double-escaped on the way in (the FORML reading uses
    /// `HKCU\\Software\\X`); this field stores them un-escaped
    /// (single backslash) so the .reg writer can re-escape once.
    pub subpath: String,
    /// REG_SZ value to write at the (default) value of the key.
    pub value: String,
}

/// Parse all `Wine App 'X' requires Registry Key at Registry Path 'P' with Registry Value 'V'`
/// instance facts for `app_id`. Returns the keys in source order so
/// the resulting `.reg` file rewrites idempotently.
pub fn parse_registry_keys(wine_md_text: &str, app_id: &str) -> Vec<RegistryKey> {
    let mut out: Vec<RegistryKey> = Vec::new();
    let needle_app = format!("Wine App '{}' requires Registry Key at Registry Path '", app_id);
    for line in wine_md_text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix(&needle_app) else { continue };
        let Some(path_end) = rest.find('\'') else { continue };
        let raw_path = &rest[..path_end];
        let after_path = &rest[path_end + 1..];
        let needle_with = " with Registry Value '";
        let Some(value_start) = after_path.find(needle_with) else { continue };
        let after_with = &after_path[value_start + needle_with.len()..];
        let Some(value_end) = after_with.find('\'') else { continue };
        let value = &after_with[..value_end];
        // `HKCU\\Software\\X` — strip the duplicate backslashes.
        let unescaped = raw_path.replace("\\\\", "\\");
        let mut parts = unescaped.splitn(2, '\\');
        let root = parts.next().unwrap_or("").to_string();
        let subpath = parts.next().unwrap_or("").to_string();
        if root.is_empty() || subpath.is_empty() {
            continue;
        }
        out.push(RegistryKey {
            root,
            subpath,
            value: value.to_string(),
        });
    }
    out
}

/// Header lines for a freshly-created `.reg` file. Wine's loader
/// requires these at the top so the file is recognised as the
/// version-2 registry format.
fn reg_header() -> String {
    "WINE REGISTRY Version 2\n;; All keys relative to \\\\\n\n".to_string()
}

/// Rewrite the `[Software\\Wine\\DllOverrides]` section of `existing`
/// with a fresh body assembled from `overrides`. Other sections are
/// preserved verbatim. If the section doesn't exist yet, it is
/// appended at the end.
pub fn rewrite_dll_overrides_section(
    existing: &str,
    overrides: &BTreeMap<String, String>,
) -> String {
    let section_header = "[Software\\\\Wine\\\\DllOverrides]";
    let mut new_section = format!("{}\n", section_header);
    for (dll, behavior) in overrides {
        new_section.push_str(&format!("\"{}\"=\"{}\"\n", dll, behavior));
    }
    splice_section(existing, section_header, &new_section)
}

/// Rewrite the keyspace defined by `keys` inside `existing`. Each key
/// becomes its own `[<root>\\<path>]\n@="<value>"\n` block; existing
/// blocks at the same key path are replaced.
pub fn rewrite_registry_keys(existing: &str, keys: &[&RegistryKey]) -> String {
    keys.iter().fold(existing.to_string(), |acc, k| {
        let header = format!("[{}\\\\{}]", k.root, k.subpath.replace('\\', "\\\\"));
        let body = format!("{}\n@=\"{}\"\n", header, k.value);
        splice_section(&acc, &header, &body)
    })
}

/// Splice `replacement` into `text` at the location of the
/// `section_header` line. If the header is present, the existing
/// block (header line up to the next blank line or next `[` header)
/// is replaced. If absent, the replacement is appended with a
/// leading blank line. Pure string surgery — no .reg parsing.
fn splice_section(text: &str, section_header: &str, replacement: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut header_idx: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        if line.trim() == section_header {
            header_idx = Some(i);
            break;
        }
    }
    match header_idx {
        Some(start) => {
            let mut end = lines.len();
            for (j, line) in lines.iter().enumerate().skip(start + 1) {
                if line.trim().is_empty() || line.trim_start().starts_with('[') {
                    end = j;
                    break;
                }
            }
            let mut out = String::new();
            for line in &lines[..start] {
                out.push_str(line);
                out.push('\n');
            }
            out.push_str(replacement);
            if !replacement.ends_with('\n') {
                out.push('\n');
            }
            for line in &lines[end..] {
                out.push_str(line);
                out.push('\n');
            }
            out
        }
        None => {
            let mut out = text.to_string();
            if !out.ends_with('\n') {
                out.push('\n');
            }
            if !out.is_empty() && !out.ends_with("\n\n") {
                out.push('\n');
            }
            out.push_str(replacement);
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out
        }
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_MD: &str = r#"
Wine App 'photoshop-cs6' requires DLL Override of DLL Name 'msvcr120.dll' with DLL Behavior 'native'.
Wine App 'office-2016-word' requires DLL Override of DLL Name 'riched20.dll' with DLL Behavior 'native'.
Wine App 'steam-windows' requires DLL Override of DLL Name 'dwrite.dll' with DLL Behavior 'disabled'.
Wine App 'steam-windows' requires DLL Override of DLL Name 'msxml3.dll' with DLL Behavior 'native'.
Wine App 'steam-windows' requires DLL Override of DLL Name 'msxml6.dll' with DLL Behavior 'native'.
Wine App 'spotify' requires Registry Key at Registry Path 'HKCU\\Software\\Spotify\\CrashReporter' with Registry Value 'disabled'.
Wine App 'fictional-app' requires Registry Key at Registry Path 'HKLM\\Software\\Fictional\\X' with Registry Value 'on'.
"#;

    #[test]
    fn parse_dll_overrides_strips_dll_suffix() {
        let out = parse_dll_overrides(SAMPLE_MD, "photoshop-cs6");
        assert_eq!(out.get("msvcr120").map(String::as_str), Some("native"));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn parse_dll_overrides_returns_multiple_entries() {
        let out = parse_dll_overrides(SAMPLE_MD, "steam-windows");
        // BTreeMap iteration is sorted; assert all three present.
        assert_eq!(out.len(), 3);
        assert_eq!(out.get("dwrite").map(String::as_str), Some(""));   // disabled → empty
        assert_eq!(out.get("msxml3").map(String::as_str), Some("native"));
        assert_eq!(out.get("msxml6").map(String::as_str), Some("native"));
    }

    #[test]
    fn parse_dll_overrides_returns_empty_for_unknown_app() {
        let out = parse_dll_overrides(SAMPLE_MD, "nonexistent");
        assert!(out.is_empty());
    }

    #[test]
    fn behavior_to_wine_format_translates_disabled() {
        assert_eq!(behavior_to_wine_format("disabled"), "");
        assert_eq!(behavior_to_wine_format("native"), "native");
        assert_eq!(behavior_to_wine_format("native-then-builtin"), "native,builtin");
        assert_eq!(behavior_to_wine_format("builtin-then-native"), "builtin,native");
    }

    #[test]
    fn parse_registry_keys_splits_root_and_subpath() {
        let out = parse_registry_keys(SAMPLE_MD, "spotify");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].root, "HKCU");
        assert_eq!(out[0].subpath, r"Software\Spotify\CrashReporter");
        assert_eq!(out[0].value, "disabled");
    }

    #[test]
    fn parse_registry_keys_returns_empty_for_unknown_app() {
        let out = parse_registry_keys(SAMPLE_MD, "nonexistent");
        assert!(out.is_empty());
    }

    #[test]
    fn rewrite_dll_overrides_section_creates_when_missing() {
        let mut overrides = BTreeMap::new();
        overrides.insert("msvcr120".to_string(), "native".to_string());
        let out = rewrite_dll_overrides_section("WINE REGISTRY Version 2\n", &overrides);
        assert!(out.contains("[Software\\\\Wine\\\\DllOverrides]"));
        assert!(out.contains("\"msvcr120\"=\"native\""));
    }

    #[test]
    fn rewrite_dll_overrides_section_replaces_existing_block() {
        let existing = "WINE REGISTRY Version 2\n\n\
                        [Software\\\\Wine\\\\DllOverrides]\n\
                        \"oldlib\"=\"native\"\n\n\
                        [Other\\\\Section]\n\
                        \"keep\"=\"me\"\n";
        let mut overrides = BTreeMap::new();
        overrides.insert("newlib".to_string(), "builtin".to_string());
        let out = rewrite_dll_overrides_section(existing, &overrides);
        assert!(out.contains("\"newlib\"=\"builtin\""));
        assert!(!out.contains("\"oldlib\"=\"native\""), "old override entry must be replaced; got: {}", out);
        assert!(out.contains("[Other\\\\Section]"), "unrelated sections must be preserved; got: {}", out);
        assert!(out.contains("\"keep\"=\"me\""));
    }

    #[test]
    fn rewrite_dll_overrides_section_idempotent() {
        let mut overrides = BTreeMap::new();
        overrides.insert("msvcr120".to_string(), "native".to_string());
        overrides.insert("dwrite".to_string(), "".to_string());
        let pass1 = rewrite_dll_overrides_section("", &overrides);
        let pass2 = rewrite_dll_overrides_section(&pass1, &overrides);
        assert_eq!(pass1, pass2, "second pass must be byte-identical to first");
    }

    #[test]
    fn rewrite_registry_keys_idempotent() {
        let key = RegistryKey {
            root: "HKCU".to_string(),
            subpath: r"Software\Test\Key".to_string(),
            value: "x".to_string(),
        };
        let pass1 = rewrite_registry_keys("", &[&key]);
        let pass2 = rewrite_registry_keys(&pass1, &[&key]);
        assert_eq!(pass1, pass2);
    }

    #[test]
    fn apply_dll_overrides_writes_system_reg() {
        let tmp = tempdir();
        let n = apply_dll_overrides(SAMPLE_MD, "photoshop-cs6", &tmp).expect("apply must succeed");
        assert_eq!(n, 1);
        let body = std::fs::read_to_string(tmp.join("system.reg")).expect("system.reg written");
        assert!(body.contains("[Software\\\\Wine\\\\DllOverrides]"));
        assert!(body.contains("\"msvcr120\"=\"native\""));
    }

    #[test]
    fn apply_dll_overrides_returns_zero_for_no_overrides() {
        let tmp = tempdir();
        let n = apply_dll_overrides(SAMPLE_MD, "notepad-plus-plus", &tmp).expect("apply must succeed");
        assert_eq!(n, 0, "platinum app with no overrides must return 0; no file write");
        // No system.reg created when there's nothing to write.
        assert!(!tmp.join("system.reg").exists());
    }

    #[test]
    fn apply_dll_overrides_idempotent_on_disk() {
        let tmp = tempdir();
        let n1 = apply_dll_overrides(SAMPLE_MD, "steam-windows", &tmp).expect("first apply");
        let body1 = std::fs::read_to_string(tmp.join("system.reg")).unwrap();
        let n2 = apply_dll_overrides(SAMPLE_MD, "steam-windows", &tmp).expect("second apply");
        let body2 = std::fs::read_to_string(tmp.join("system.reg")).unwrap();
        assert_eq!(n1, n2);
        assert_eq!(body1, body2, "second apply must produce byte-identical system.reg");
    }

    #[test]
    fn apply_registry_keys_routes_hkcu_to_user_reg() {
        let tmp = tempdir();
        let n = apply_registry_keys(SAMPLE_MD, "spotify", &tmp).expect("apply must succeed");
        assert_eq!(n, 1);
        let body = std::fs::read_to_string(tmp.join("user.reg")).expect("user.reg must exist");
        assert!(body.contains("[HKCU\\\\Software\\\\Spotify\\\\CrashReporter]"));
        assert!(body.contains("@=\"disabled\""));
        // No system.reg since this app only writes HKCU.
        assert!(!tmp.join("system.reg").exists(),
                "spotify only writes HKCU so system.reg must not be created");
    }

    #[test]
    fn apply_registry_keys_routes_hklm_to_system_reg() {
        let tmp = tempdir();
        let n = apply_registry_keys(SAMPLE_MD, "fictional-app", &tmp).expect("apply must succeed");
        assert_eq!(n, 1);
        let body = std::fs::read_to_string(tmp.join("system.reg")).expect("system.reg must exist");
        assert!(body.contains("[HKLM\\\\Software\\\\Fictional\\\\X]"));
    }

    #[test]
    fn apply_registry_keys_idempotent() {
        let tmp = tempdir();
        apply_registry_keys(SAMPLE_MD, "spotify", &tmp).expect("first apply");
        let body1 = std::fs::read_to_string(tmp.join("user.reg")).unwrap();
        apply_registry_keys(SAMPLE_MD, "spotify", &tmp).expect("second apply");
        let body2 = std::fs::read_to_string(tmp.join("user.reg")).unwrap();
        assert_eq!(body1, body2);
    }

    /// Tiny tempdir helper — std::env::temp_dir + a per-test unique
    /// subdirectory based on the test name + a counter. Good enough
    /// for the bootstrap module (no concurrent process tests).
    fn tempdir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("arest-wine-test-{}-{}", pid, n));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("tempdir create");
        path
    }
}
