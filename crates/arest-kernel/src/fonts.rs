// crates/arest-kernel/src/fonts.rs
//
// AREST kernel — vendored UI font binaries (#433).
//
// Track YY's design system (`readings/ui/design.md`, #432 c52e38f) names
// two FontFamily instances — `Inter` (Sans Family) and `JetBrains Mono`
// (Mono Family) — that the Slint kernel UI surface (#436) and the ui.do
// React frontend share. The Slint renderer can't reach the network at
// boot, so the bytes have to ship inside the kernel `.efi` PE32+ image.
//
// This module exposes those bytes as `pub static &'static [u8]` slices
// the renderer registers with Slint's font backend. No parsing happens
// at compile time — `include_bytes!` just splices the file in verbatim.
//
// Weight scope: only the Regular weight ships in this commit. Track YY's
// TypographyScale instances use weights 400/500/600 from a single TTF
// (Inter's TTF is hinted-only — the design system relies on the variable
// `InterVariable.ttf` shipped in the v4.0 release zip if synthetic
// weighting via Slint's font shaping isn't sufficient). Bold/Italic
// drops can land in a follow-up if Slint complains about missing
// weights at #436 wire-up time. Keeps the binary growth bounded
// (~680 KiB combined Regular vs ~3 MiB if every weight ships).
//
// Provenance:
//   - Inter-Regular.ttf:   rsms/inter v4.0 release zip, extras/ttf/.
//                          Upstream: https://github.com/rsms/inter
//                          License:  SIL Open Font License 1.1 (OFL).
//   - JetBrainsMono-Regular.ttf:
//                          JetBrains/JetBrainsMono master tree,
//                          fonts/ttf/JetBrainsMono-Regular.ttf.
//                          Upstream: https://github.com/JetBrains/JetBrainsMono
//                          License:  SIL Open Font License 1.1 (OFL).
//
// Both licenses are reproduced under `assets/fonts/LICENSE-OFL.txt`.

#![allow(dead_code)] // Wired into Slint at #436; until then statics
                     // are only reachable via `pub` re-exports.

/// Inter Regular (weight 400). Sans-serif family backing every
/// `TypographyScale` instance in `design.md` whose `FontFamily` is
/// `Inter` — i.e. display, h1, h2, h3, body, body-sm, caption, label,
/// button (9 of the 10 scales).
pub static INTER_REGULAR: &[u8] =
    include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// JetBrains Mono Regular (weight 400). Monospace family backing the
/// `code` `TypographyScale` instance in `design.md` and the REPL
/// surface (#365 / #436 follow-up).
pub static JETBRAINS_MONO_REGULAR: &[u8] =
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");

/// Logical family name as declared by `design.md`'s `FontFamily 'Inter'`.
/// Slint looks fonts up by family name; this constant keeps the wire-up
/// at #436 honest if the upstream TTF's name table ever drifts from the
/// design-system spelling.
pub const INTER_FAMILY_NAME: &str = "Inter";

/// Logical family name as declared by `design.md`'s
/// `FontFamily 'JetBrains Mono'`.
pub const JETBRAINS_MONO_FAMILY_NAME: &str = "JetBrains Mono";
