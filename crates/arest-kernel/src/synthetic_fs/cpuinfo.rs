// crates/arest-kernel/src/synthetic_fs/cpuinfo.rs
//
// Synthetic `/proc/cpuinfo` renderer (#534, #475a). Produces the byte-
// stream Linux userspace tools (`grep ^processor /proc/cpuinfo`, the
// `lscpu` libc reader, doomgeneric's `SDL_GetCPUCount` shim) expect
// when they read the file.
//
// The Linux format is a strict per-CPU stanza: one stanza per logical
// CPU, blank line between stanzas, key-value lines inside each stanza
// formatted as `<key>\t: <value>` (the literal tab is part of the
// alignment convention — userspace parsers like the busybox `lscpu`
// implementation key on the tab + colon to split). This module reproduces
// that format byte-for-byte against a `CpuInfoSnapshot` describing the
// observed cells.
//
// Why a snapshot rather than reading cells directly
// -------------------------------------------------
// `crate::arch::memory::usable_frame_count()` panics if the boot-time
// memory init has not run yet. The same constraint will apply once a
// proper `arch::cpu` module lands (#475 epic — boot-time CPUID + ACPI
// MADT walk). Taking the snapshot as a parameter means the same render
// function exercises end-to-end in a host `cargo test` run (Windows /
// Linux / Darwin) without needing the kernel singletons live, AND
// matches the "render fixture state → compare output to expected Linux
// format" verification recipe in the task spec.
//
// What's modelled today vs. left as defaults
// -----------------------------------------
// Real Linux `/proc/cpuinfo` exposes ~30 fields per CPU; the tier-1
// surface we need to keep `grep ^processor` / `nproc` working is a
// proper subset (about 15 fields). The defaults we emit for fields we
// haven't yet detected (microcode = 0x0, cache size = 0 KB, flags =
// "fpu") are the same shape Linux emits on the simplest QEMU TCG guest
// without -cpu host, so userspace tools that pattern-match the format
// see a valid, if minimal, line. Once #475's per-arch CPU detection
// lands, the snapshot constructor populates more of these from real
// hardware reads; the renderer below stays unchanged.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// One renderable stanza in `/proc/cpuinfo`. Mirrors the per-CPU shape
/// Linux emits. Every field has a sane default the renderer falls back
/// on so callers can populate only what they've actually detected (the
/// current arch::memory init does not yet expose CPUID-style cell
/// reads, so the boot path supplies defaults).
#[derive(Debug, Clone)]
pub struct CpuStanza {
    /// `processor` field — the logical CPU index (0..N).
    pub processor: u32,
    /// `vendor_id` field — typically `GenuineIntel`, `AuthenticAMD`,
    /// or `ARM` on aarch64 / armv7. Twelve ASCII characters from CPUID
    /// leaf 0 on x86; an arch-prefix string on ARM.
    pub vendor_id: String,
    /// `cpu family` field — CPUID-derived family number on x86; 0 on
    /// ARM where the field is conventionally elided but kept in the
    /// snapshot so the renderer's format string stays uniform.
    pub cpu_family: u32,
    /// `model` field — CPUID-derived model number on x86; 0 on ARM.
    pub model: u32,
    /// `model name` field — human-readable processor name. Linux pulls
    /// this from CPUID extended leaves on x86 and from `/proc/device-
    /// tree/model` on ARM. Defaults to a generic "AREST CPU" string
    /// when no detection has run.
    pub model_name: String,
    /// `stepping` field — CPUID-derived stepping number on x86; 0 on
    /// ARM.
    pub stepping: u32,
    /// `microcode` field — microcode revision. Renderer formats it as
    /// `0x{:x}`. Defaults to 0 when no detection has run.
    pub microcode: u32,
    /// `cpu MHz` field — current frequency in MHz. Renderer formats
    /// with three decimal places (Linux convention). Defaults to 0.0.
    pub cpu_mhz: u32,
    /// `cache size` field — last-level cache size in KB. Renderer
    /// formats as `{} KB`. Defaults to 0.
    pub cache_size_kb: u32,
    /// `physical id` field — package id (socket index for SMP, 0 for
    /// uniprocessor). Defaults to 0.
    pub physical_id: u32,
    /// `siblings` field — total logical CPUs in this package.
    pub siblings: u32,
    /// `core id` field — physical core index within the package.
    pub core_id: u32,
    /// `cpu cores` field — physical cores in this package.
    pub cpu_cores: u32,
    /// `flags` field — space-separated CPU feature names (e.g. `fpu vme
    /// de pse tsc msr pae`). Renderer emits them verbatim. Defaults to
    /// `"fpu"` so a tool that requires at least one flag (gnu binutils
    /// `as` does this) keeps working.
    pub flags: String,
}

impl Default for CpuStanza {
    fn default() -> Self {
        Self {
            processor: 0,
            vendor_id: "AREST".to_string(),
            cpu_family: 0,
            model: 0,
            model_name: "AREST CPU".to_string(),
            stepping: 0,
            microcode: 0,
            cpu_mhz: 0,
            cache_size_kb: 0,
            physical_id: 0,
            siblings: 1,
            core_id: 0,
            cpu_cores: 1,
            flags: "fpu".to_string(),
        }
    }
}

/// Renderable snapshot of the system's CPU topology. One `CpuStanza`
/// per logical CPU; the renderer walks them in order and emits a stanza
/// per entry separated by blank lines.
#[derive(Debug, Clone)]
pub struct CpuInfoSnapshot {
    /// One entry per logical CPU. Empty vec → renderer emits the empty
    /// byte string (matches what Linux does on a `cpu_count = 0`
    /// system, which never actually happens but keeps the function
    /// total).
    pub stanzas: Vec<CpuStanza>,
}

impl CpuInfoSnapshot {
    /// Build a snapshot for `cpu_count` logical CPUs sharing the same
    /// vendor / model / flags. Each stanza gets a unique `processor`
    /// and `core_id`; everything else mirrors the template. Used by
    /// the boot-time wiring (#475 epic) and by tests that want a
    /// uniform N-CPU fixture.
    pub fn uniform(cpu_count: u32, template: CpuStanza) -> Self {
        let mut stanzas = Vec::with_capacity(cpu_count as usize);
        for i in 0..cpu_count {
            let mut s = template.clone();
            s.processor = i;
            s.core_id = i;
            s.siblings = cpu_count;
            s.cpu_cores = cpu_count;
            stanzas.push(s);
        }
        Self { stanzas }
    }
}

impl Default for CpuInfoSnapshot {
    fn default() -> Self {
        Self::uniform(1, CpuStanza::default())
    }
}

/// Render a `CpuInfoSnapshot` into the byte stream Linux userspace
/// tools expect when reading `/proc/cpuinfo`. One stanza per CPU,
/// blank line between stanzas, no trailing blank after the last
/// stanza (matches Linux behavior — `od -c /proc/cpuinfo` shows a
/// single `\n` after the last `flags` line).
///
/// Always succeeds — `format!` allocations panic on OOM but on the
/// tier-1 kernel that's a hard fault anyway. No `Result` wrapper.
pub fn render(snapshot: &CpuInfoSnapshot) -> Vec<u8> {
    let mut out = String::with_capacity(snapshot.stanzas.len() * 512);
    for (i, s) in snapshot.stanzas.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        push_stanza(&mut out, s);
    }
    out.into_bytes()
}

/// Convenience entry that mirrors the planned production wiring —
/// builds a default snapshot of `cpu_count` CPUs (1 by default until
/// the #475 boot-time CPUID walk lands) and renders it. Available so
/// `proc::render_proc_file` can call a one-liner without constructing
/// the snapshot inline.
pub fn render_cpuinfo() -> Vec<u8> {
    render(&CpuInfoSnapshot::default())
}

/// Append one stanza to `out` in the exact Linux key/value layout.
/// The literal `\t: ` separator (one tab, then colon, then space) is
/// what Linux emits — the tab keeps the colon column-aligned across
/// keys of varying length when viewed in a terminal. Tools like
/// `grep ^processor` only key on the `<key>\t:` prefix so we keep
/// the tab discipline strict.
fn push_stanza(out: &mut String, s: &CpuStanza) {
    push_kv(out, "processor", &format!("{}", s.processor));
    push_kv(out, "vendor_id", &s.vendor_id);
    push_kv(out, "cpu family", &format!("{}", s.cpu_family));
    push_kv(out, "model", &format!("{}", s.model));
    push_kv(out, "model name", &s.model_name);
    push_kv(out, "stepping", &format!("{}", s.stepping));
    push_kv(out, "microcode", &format!("0x{:x}", s.microcode));
    push_kv(out, "cpu MHz", &format!("{}.000", s.cpu_mhz));
    push_kv(out, "cache size", &format!("{} KB", s.cache_size_kb));
    push_kv(out, "physical id", &format!("{}", s.physical_id));
    push_kv(out, "siblings", &format!("{}", s.siblings));
    push_kv(out, "core id", &format!("{}", s.core_id));
    push_kv(out, "cpu cores", &format!("{}", s.cpu_cores));
    push_kv(out, "flags", &s.flags);
}

fn push_kv(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push('\t');
    out.push_str(": ");
    out.push_str(value);
    out.push('\n');
}

// Inline tests are gated on `cfg(target_os = "linux")` for the same
// reason `composer` / `slint_backend` / `doom` gate theirs: the
// `arest-kernel` bin sets `test = false` in Cargo.toml, so the only
// way to run these tests is via a host-target `cargo test --bin
// arest-kernel --target x86_64-unknown-linux-gnu` invocation. On a
// Windows / Darwin host the `no_std` + UEFI dep chain refuses to
// link a test binary, so the gate keeps the build cross-host clean.
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn default_snapshot_renders_one_stanza() {
        let bytes = render(&CpuInfoSnapshot::default());
        let s = core::str::from_utf8(&bytes).unwrap();
        // One stanza, terminated by exactly one newline (no blank
        // line after the last stanza — Linux convention).
        assert!(s.starts_with("processor\t: 0\n"));
        assert!(s.ends_with("flags\t: fpu\n"));
        // Exactly one stanza means no blank-line separator at all.
        assert!(!s.contains("\n\n"));
    }

    #[test]
    fn uniform_four_cores_renders_four_stanzas() {
        let snap = CpuInfoSnapshot::uniform(4, CpuStanza::default());
        let bytes = render(&snap);
        let s = core::str::from_utf8(&bytes).unwrap();
        // Four stanzas → three internal blank-line separators.
        let blank_lines = s.matches("\n\n").count();
        assert_eq!(blank_lines, 3, "expected 3 blank-line separators, got: {}", s);
        // Each stanza starts with `processor\t: N\n` for N = 0..4.
        for i in 0..4 {
            let needle = format!("processor\t: {}\n", i);
            assert!(s.contains(&needle), "missing `{}` in:\n{}", needle, s);
        }
        // siblings + cpu cores reflect the count.
        for _ in 0..4 {
            assert!(s.contains("siblings\t: 4\n"));
            assert!(s.contains("cpu cores\t: 4\n"));
        }
    }

    #[test]
    fn key_value_separator_is_tab_colon_space() {
        // Linux uses a literal tab, then `: `. Userspace parsers
        // (lscpu, busybox, glibc's get_nprocs) split on this exact
        // separator, so it must not regress.
        let bytes = render(&CpuInfoSnapshot::default());
        let s = core::str::from_utf8(&bytes).unwrap();
        for line in s.lines() {
            if line.is_empty() { continue; }
            // Each non-blank line should contain a `\t: ` exactly once.
            let count = line.matches("\t: ").count();
            assert_eq!(count, 1, "line `{}` has {} `\\t: ` separators", line, count);
        }
    }

    #[test]
    fn intel_template_round_trips_known_fields() {
        // A realistic stanza populated with Intel-style values; the
        // renderer should emit them verbatim where the format permits.
        let intel = CpuStanza {
            processor: 0,
            vendor_id: "GenuineIntel".to_string(),
            cpu_family: 6,
            model: 142,
            model_name: "Intel(R) Core(TM) i7-8650U CPU @ 1.90GHz".to_string(),
            stepping: 10,
            microcode: 0xf4,
            cpu_mhz: 1992,
            cache_size_kb: 8192,
            physical_id: 0,
            siblings: 8,
            core_id: 0,
            cpu_cores: 4,
            flags: "fpu vme de pse tsc msr pae mce cx8 apic".to_string(),
        };
        let snap = CpuInfoSnapshot { stanzas: vec![intel] };
        let bytes = render(&snap);
        let s = core::str::from_utf8(&bytes).unwrap();
        // Spot-check the format-sensitive fields.
        assert!(s.contains("vendor_id\t: GenuineIntel\n"));
        assert!(s.contains("cpu family\t: 6\n"));
        assert!(s.contains("microcode\t: 0xf4\n"), "got:\n{}", s);
        assert!(s.contains("cpu MHz\t: 1992.000\n"));
        assert!(s.contains("cache size\t: 8192 KB\n"));
        assert!(s.contains("model name\t: Intel(R) Core(TM) i7-8650U CPU @ 1.90GHz\n"));
    }

    #[test]
    fn empty_snapshot_renders_to_empty_bytes() {
        let snap = CpuInfoSnapshot { stanzas: Vec::new() };
        let bytes = render(&snap);
        assert!(bytes.is_empty());
    }

    #[test]
    fn render_cpuinfo_default_one_stanza() {
        // The convenience entry produces a single-CPU stanza by default
        // — what the boot path would emit before #475's CPUID walk
        // populates real fields.
        let bytes = render_cpuinfo();
        let s = core::str::from_utf8(&bytes).unwrap();
        assert!(s.contains("processor\t: 0\n"));
        assert!(s.contains("vendor_id\t: AREST\n"));
    }
}
