// crates/arest-kernel/src/icons.rs
//
// AREST kernel — vendored Lucide icon set (#434).
//
// Track YY's design system (`readings/ui/design.md`, #432 c52e38f) names
// 52 IconToken instances grouped by use site (file-browser, repl,
// hateoas, common, status, auth, theme). The Slint kernel UI (#436)
// rasterises these on demand; the kernel runs without network access at
// boot, so the SVG bytes have to ship inside the kernel `.efi`.
//
// Each icon is exposed as a `pub static &'static [u8]` containing the
// raw SVG XML. The `by_name` lookup resolves both the design.md token
// names and (where the upstream Lucide registry has since renamed an
// icon) the upstream Lucide names too — that way #436 can hand either
// spelling to the renderer.
//
// Provenance:
//   - All 52 SVGs vendored from `lucide-icons/lucide` `main` branch
//     (raw.githubusercontent.com/lucide-icons/lucide/main/icons/<name>.svg).
//   - License: ISC, reproduced under `assets/icons/lucide/LICENSE-ISC.txt`.
//
// Naming drift: nine icons were renamed upstream after design.md was
// authored. We keep the design.md spelling on disk (so the file paths
// match the IconToken names verbatim) and let `by_name` accept the
// upstream Lucide name as an alias. Renames are documented inline next
// to each affected static below and consolidated in the alias arm of
// `by_name`.

#![allow(dead_code)] // Wired into Slint at #436; until then statics
                     // are only reachable via the public surface.

// =====================================================================
// File browser (Icon Role 'file-browser') — 8 icons
// =====================================================================

pub static FILE: &[u8]         = include_bytes!("../assets/icons/lucide/file.svg");
pub static FILE_TEXT: &[u8]    = include_bytes!("../assets/icons/lucide/file-text.svg");
pub static FILE_CODE: &[u8]    = include_bytes!("../assets/icons/lucide/file-code.svg");
pub static FOLDER: &[u8]       = include_bytes!("../assets/icons/lucide/folder.svg");
pub static FOLDER_OPEN: &[u8]  = include_bytes!("../assets/icons/lucide/folder-open.svg");
pub static FOLDER_PLUS: &[u8]  = include_bytes!("../assets/icons/lucide/folder-plus.svg");
pub static UPLOAD: &[u8]       = include_bytes!("../assets/icons/lucide/upload.svg");
pub static DOWNLOAD: &[u8]     = include_bytes!("../assets/icons/lucide/download.svg");

// =====================================================================
// REPL (Icon Role 'repl') — 5 icons
// =====================================================================

pub static TERMINAL: &[u8]     = include_bytes!("../assets/icons/lucide/terminal.svg");
pub static PLAY: &[u8]         = include_bytes!("../assets/icons/lucide/play.svg");
pub static SQUARE: &[u8]       = include_bytes!("../assets/icons/lucide/square.svg");
pub static ROTATE_CCW: &[u8]   = include_bytes!("../assets/icons/lucide/rotate-ccw.svg");
pub static COPY: &[u8]         = include_bytes!("../assets/icons/lucide/copy.svg");

// =====================================================================
// HATEOAS browser (Icon Role 'hateoas') — 6 icons
// =====================================================================

pub static LINK: &[u8]          = include_bytes!("../assets/icons/lucide/link.svg");
pub static EXTERNAL_LINK: &[u8] = include_bytes!("../assets/icons/lucide/external-link.svg");
pub static ARROW_LEFT: &[u8]    = include_bytes!("../assets/icons/lucide/arrow-left.svg");
pub static ARROW_RIGHT: &[u8]   = include_bytes!("../assets/icons/lucide/arrow-right.svg");
/// `home` in design.md; renamed to `house` in upstream Lucide. Vendored
/// under the design.md spelling; alias in `by_name`.
pub static HOME: &[u8]          = include_bytes!("../assets/icons/lucide/home.svg");
pub static GLOBE: &[u8]         = include_bytes!("../assets/icons/lucide/globe.svg");

// =====================================================================
// Common controls (Icon Role 'common') — 18 icons
// =====================================================================

pub static SEARCH: &[u8]          = include_bytes!("../assets/icons/lucide/search.svg");
pub static X: &[u8]               = include_bytes!("../assets/icons/lucide/x.svg");
pub static CHECK: &[u8]           = include_bytes!("../assets/icons/lucide/check.svg");
pub static PLUS: &[u8]            = include_bytes!("../assets/icons/lucide/plus.svg");
pub static MINUS: &[u8]           = include_bytes!("../assets/icons/lucide/minus.svg");
pub static TRASH: &[u8]           = include_bytes!("../assets/icons/lucide/trash.svg");
pub static PENCIL: &[u8]          = include_bytes!("../assets/icons/lucide/pencil.svg");
pub static SAVE: &[u8]            = include_bytes!("../assets/icons/lucide/save.svg");
pub static SETTINGS: &[u8]        = include_bytes!("../assets/icons/lucide/settings.svg");
pub static MENU: &[u8]            = include_bytes!("../assets/icons/lucide/menu.svg");
/// `more-horizontal` in design.md; renamed to `ellipsis` upstream.
pub static MORE_HORIZONTAL: &[u8] = include_bytes!("../assets/icons/lucide/more-horizontal.svg");
/// `more-vertical` in design.md; renamed to `ellipsis-vertical` upstream.
pub static MORE_VERTICAL: &[u8]   = include_bytes!("../assets/icons/lucide/more-vertical.svg");
pub static CHEVRON_RIGHT: &[u8]   = include_bytes!("../assets/icons/lucide/chevron-right.svg");
pub static CHEVRON_LEFT: &[u8]    = include_bytes!("../assets/icons/lucide/chevron-left.svg");
pub static CHEVRON_DOWN: &[u8]    = include_bytes!("../assets/icons/lucide/chevron-down.svg");
pub static CHEVRON_UP: &[u8]      = include_bytes!("../assets/icons/lucide/chevron-up.svg");
/// `filter` in design.md; renamed to `funnel` upstream.
pub static FILTER: &[u8]          = include_bytes!("../assets/icons/lucide/filter.svg");
/// IconToken 'sort-asc' has Lucide Name 'arrow-up-narrow-wide'.
/// design.md uses the role-bearing token name `sort-asc`; the upstream
/// SVG file is `arrow-up-narrow-wide.svg`. Vendored under the upstream
/// (Lucide) name to keep the file→upstream mapping 1:1, since the
/// design.md fact already records the Lucide Name explicitly.
pub static SORT_ASC: &[u8]        = include_bytes!("../assets/icons/lucide/arrow-up-narrow-wide.svg");
/// IconToken 'sort-desc' has Lucide Name 'arrow-down-narrow-wide'. Same
/// rationale as `SORT_ASC`.
pub static SORT_DESC: &[u8]       = include_bytes!("../assets/icons/lucide/arrow-down-narrow-wide.svg");

// =====================================================================
// Status / semantic (Icon Role 'status') — 6 icons
// =====================================================================

pub static INFO: &[u8]          = include_bytes!("../assets/icons/lucide/info.svg");
/// `alert-triangle` in design.md; renamed to `triangle-alert` upstream.
pub static ALERT_TRIANGLE: &[u8] = include_bytes!("../assets/icons/lucide/alert-triangle.svg");
/// `alert-circle` in design.md; renamed to `circle-alert` upstream.
pub static ALERT_CIRCLE: &[u8]   = include_bytes!("../assets/icons/lucide/alert-circle.svg");
/// `check-circle` in design.md; renamed to `circle-check` upstream.
pub static CHECK_CIRCLE: &[u8]   = include_bytes!("../assets/icons/lucide/check-circle.svg");
/// `x-circle` in design.md; renamed to `circle-x` upstream.
pub static X_CIRCLE: &[u8]       = include_bytes!("../assets/icons/lucide/x-circle.svg");
pub static LOADER: &[u8]        = include_bytes!("../assets/icons/lucide/loader.svg");

// =====================================================================
// Auth / user (Icon Role 'auth') — 5 icons
// =====================================================================

pub static USER: &[u8]    = include_bytes!("../assets/icons/lucide/user.svg");
pub static LOG_IN: &[u8]  = include_bytes!("../assets/icons/lucide/log-in.svg");
pub static LOG_OUT: &[u8] = include_bytes!("../assets/icons/lucide/log-out.svg");
pub static LOCK: &[u8]    = include_bytes!("../assets/icons/lucide/lock.svg");
/// `unlock` in design.md; renamed to `lock-open` upstream.
pub static UNLOCK: &[u8]  = include_bytes!("../assets/icons/lucide/unlock.svg");

// =====================================================================
// Theme switcher (Icon Role 'theme') — 3 icons
// =====================================================================

pub static SUN: &[u8]     = include_bytes!("../assets/icons/lucide/sun.svg");
pub static MOON: &[u8]    = include_bytes!("../assets/icons/lucide/moon.svg");
pub static PALETTE: &[u8] = include_bytes!("../assets/icons/lucide/palette.svg");

// =====================================================================
// Lookup
// =====================================================================

/// Resolve an icon name to its raw SVG bytes.
///
/// Accepted spellings (in priority order):
///
/// 1. The design.md `IconToken` name (kebab-case). For
///    `IconToken 'sort-asc'` this is `"sort-asc"`.
/// 2. The design.md `Lucide Name` value (kebab-case). For
///    `IconToken 'sort-asc' has Lucide Name 'arrow-up-narrow-wide'`
///    this is `"arrow-up-narrow-wide"` — i.e. the same SVG can also
///    be reached under its upstream filename.
/// 3. The current-upstream Lucide name where it differs from the
///    design.md spelling (the registry renamed nine icons after
///    design.md was authored — see the per-static comments above).
///    For example, `"house"` resolves to `HOME`.
///
/// Unknown names return `None`.
pub fn by_name(name: &str) -> Option<&'static [u8]> {
    match name {
        // ---- File browser
        "file"            => Some(FILE),
        "file-text"       => Some(FILE_TEXT),
        "file-code"       => Some(FILE_CODE),
        "folder"          => Some(FOLDER),
        "folder-open"     => Some(FOLDER_OPEN),
        "folder-plus"     => Some(FOLDER_PLUS),
        "upload"          => Some(UPLOAD),
        "download"        => Some(DOWNLOAD),

        // ---- REPL
        "terminal"        => Some(TERMINAL),
        "play"            => Some(PLAY),
        "square"          => Some(SQUARE),
        "rotate-ccw"      => Some(ROTATE_CCW),
        "copy"            => Some(COPY),

        // ---- HATEOAS
        "link"            => Some(LINK),
        "external-link"   => Some(EXTERNAL_LINK),
        "arrow-left"      => Some(ARROW_LEFT),
        "arrow-right"     => Some(ARROW_RIGHT),
        "home"            => Some(HOME),
        "house"           => Some(HOME), // upstream rename alias
        "globe"           => Some(GLOBE),

        // ---- Common
        "search"          => Some(SEARCH),
        "x"               => Some(X),
        "check"           => Some(CHECK),
        "plus"            => Some(PLUS),
        "minus"           => Some(MINUS),
        "trash"           => Some(TRASH),
        "pencil"          => Some(PENCIL),
        "save"            => Some(SAVE),
        "settings"        => Some(SETTINGS),
        "menu"            => Some(MENU),
        "more-horizontal" => Some(MORE_HORIZONTAL),
        "ellipsis"        => Some(MORE_HORIZONTAL), // upstream rename alias
        "more-vertical"   => Some(MORE_VERTICAL),
        "ellipsis-vertical" => Some(MORE_VERTICAL), // upstream rename alias
        "chevron-right"   => Some(CHEVRON_RIGHT),
        "chevron-left"    => Some(CHEVRON_LEFT),
        "chevron-down"    => Some(CHEVRON_DOWN),
        "chevron-up"      => Some(CHEVRON_UP),
        "filter"          => Some(FILTER),
        "funnel"          => Some(FILTER), // upstream rename alias
        "sort-asc"        => Some(SORT_ASC),
        "arrow-up-narrow-wide"   => Some(SORT_ASC),   // Lucide Name from design.md
        "sort-desc"       => Some(SORT_DESC),
        "arrow-down-narrow-wide" => Some(SORT_DESC),  // Lucide Name from design.md

        // ---- Status
        "info"            => Some(INFO),
        "alert-triangle"  => Some(ALERT_TRIANGLE),
        "triangle-alert"  => Some(ALERT_TRIANGLE), // upstream rename alias
        "alert-circle"    => Some(ALERT_CIRCLE),
        "circle-alert"    => Some(ALERT_CIRCLE), // upstream rename alias
        "check-circle"    => Some(CHECK_CIRCLE),
        "circle-check"    => Some(CHECK_CIRCLE), // upstream rename alias
        "x-circle"        => Some(X_CIRCLE),
        "circle-x"        => Some(X_CIRCLE), // upstream rename alias
        "loader"          => Some(LOADER),

        // ---- Auth
        "user"            => Some(USER),
        "log-in"          => Some(LOG_IN),
        "log-out"         => Some(LOG_OUT),
        "lock"            => Some(LOCK),
        "unlock"          => Some(UNLOCK),
        "lock-open"       => Some(UNLOCK), // upstream rename alias

        // ---- Theme
        "sun"             => Some(SUN),
        "moon"            => Some(MOON),
        "palette"         => Some(PALETTE),

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spot-check both spellings of the renamed icons resolve to the
    /// same bytes — `by_name` is the only place the rename map is
    /// expressed in code.
    #[test]
    fn rename_aliases_match() {
        assert_eq!(by_name("home"),            by_name("house"));
        assert_eq!(by_name("more-horizontal"), by_name("ellipsis"));
        assert_eq!(by_name("more-vertical"),   by_name("ellipsis-vertical"));
        assert_eq!(by_name("filter"),          by_name("funnel"));
        assert_eq!(by_name("alert-triangle"),  by_name("triangle-alert"));
        assert_eq!(by_name("alert-circle"),    by_name("circle-alert"));
        assert_eq!(by_name("check-circle"),    by_name("circle-check"));
        assert_eq!(by_name("x-circle"),        by_name("circle-x"));
        assert_eq!(by_name("unlock"),          by_name("lock-open"));
        assert_eq!(by_name("sort-asc"),        by_name("arrow-up-narrow-wide"));
        assert_eq!(by_name("sort-desc"),       by_name("arrow-down-narrow-wide"));
    }

    #[test]
    fn unknown_name_returns_none() {
        assert!(by_name("not-a-real-icon").is_none());
        assert!(by_name("").is_none());
    }

    #[test]
    fn every_design_md_token_resolves() {
        // The 52 IconToken names from design.md.
        let tokens = [
            "file", "file-text", "file-code", "folder", "folder-open",
            "folder-plus", "upload", "download",
            "terminal", "play", "square", "rotate-ccw", "copy",
            "link", "external-link", "arrow-left", "arrow-right", "home", "globe",
            "search", "x", "check", "plus", "minus", "trash", "pencil",
            "save", "settings", "menu", "more-horizontal", "more-vertical",
            "chevron-right", "chevron-left", "chevron-down", "chevron-up",
            "filter", "sort-asc", "sort-desc",
            "info", "alert-triangle", "alert-circle", "check-circle",
            "x-circle", "loader",
            "user", "log-in", "log-out", "lock", "unlock",
            "sun", "moon", "palette",
        ];
        for token in tokens {
            assert!(by_name(token).is_some(), "token {} did not resolve", token);
        }
    }
}
