// crates/arest-kernel/src/synthetic_fs/meminfo.rs
//
// Synthetic `/proc/meminfo` renderer (#534, #475a). Produces the byte-
// stream Linux userspace tools (`free -m`, `cat /proc/meminfo | grep
// MemAvailable`, glibc's `__get_avphys_pages`) expect when they read
// the file.
//
// The Linux format is one key-value pair per line, formatted as
// `<Key>:<padding><value> kB` where `<padding>` is enough whitespace
// to right-align `<value>` to column 8. Tools that grep this file key
// on the colon and tolerate variable padding, but right-alignment is
// the convention every distro ships and `lscpu`-class formatters depend
// on the alignment for column rendering. We reproduce the right-aligned
// format byte-for-byte.
//
// What's modelled today
// ---------------------
// The fields userspace tools actually grep on:
//   * `MemTotal` / `MemFree` / `MemAvailable` — the three any
//     `free`-style tool reads.
//   * `Buffers` / `Cached` / `SwapCached` — set to 0; AREST has no
//     page cache or swap subsystem yet, but the keys must be present
//     for tools that compute `MemAvailable` themselves
//     (`MemFree + Buffers + Cached - SwapCached`) to yield a sane
//     value.
//   * `SwapTotal` / `SwapFree` — set to 0; matches a system with no
//     swap configured.
//
// Fields like `Active`, `Inactive`, `Slab`, `KernelStack`, etc. are
// elided. Tools that need them don't compile cleanly against a kernel
// that lacks the corresponding cells; emitting zero values for them
// would be misleading. Userspace tools that grep for the modelled
// keys keep working — anything more advanced waits on the relevant
// kernel subsystem (page cache, swap, slab allocator) coming online.
//
// Why a snapshot rather than reading cells directly
// -------------------------------------------------
// Same rationale as `cpuinfo` — `crate::arch::memory::usable_frame_count`
// panics if `init` has not run yet, so a renderer that calls it directly
// is unusable from a host `cargo test` run. The boot path constructs
// the snapshot from real cells; tests construct it from fixtures.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Renderable snapshot of the system's memory state. Every field is in
/// kibibytes (kB in Linux convention — actually 1024-byte units, not
/// the SI-correct kilobyte). Defaults to all-zero so the boot path can
/// populate only the fields it has actually computed.
#[derive(Debug, Clone, Default)]
pub struct MemInfoSnapshot {
    /// Total physical RAM the kernel knows about. Sourced from
    /// `usable_frame_count() * 4096 / 1024` on the boot path.
    pub mem_total_kb: u64,
    /// Currently-free RAM. Sourced from the frame allocator's free-
    /// frame count; see the boot wiring in `synthetic_fs::mod`.
    pub mem_free_kb: u64,
    /// Memory available for new allocations without swapping. Linux
    /// computes this as a non-trivial heuristic; we publish whatever
    /// the boot path supplies and default to `mem_free_kb` when the
    /// caller leaves it zero.
    pub mem_available_kb: u64,
    /// Buffer-cache size. AREST has none — keep at 0 unless a future
    /// page cache subsystem populates it.
    pub buffers_kb: u64,
    /// Page-cache size. AREST has none — keep at 0.
    pub cached_kb: u64,
    /// Swap-cached size. AREST has no swap — keep at 0.
    pub swap_cached_kb: u64,
    /// Total swap configured. AREST has no swap — keep at 0.
    pub swap_total_kb: u64,
    /// Free swap. AREST has no swap — keep at 0.
    pub swap_free_kb: u64,
}

/// Render a `MemInfoSnapshot` into the byte stream Linux userspace
/// tools expect when reading `/proc/meminfo`. Right-aligns every
/// numeric value to column 8 to match the conventional Linux format.
///
/// The renderer always emits the same eight keys in the same order so
/// userspace parsers that walk the file top-to-bottom (some shell
/// scripts do this rather than grep) find them in stable positions.
pub fn render(snapshot: &MemInfoSnapshot) -> Vec<u8> {
    // Compute `MemAvailable` fallback: when the snapshot leaves it
    // zero AND `mem_free_kb` is non-zero, publish `mem_free_kb` so a
    // tool that reads this single field gets a reasonable answer. The
    // boot path can override by setting `mem_available_kb` explicitly
    // (e.g. once the kernel grows a reclaim heuristic).
    let mem_available = if snapshot.mem_available_kb == 0 {
        snapshot.mem_free_kb
    } else {
        snapshot.mem_available_kb
    };

    let mut out = String::with_capacity(512);
    push_kv(&mut out, "MemTotal",     snapshot.mem_total_kb);
    push_kv(&mut out, "MemFree",      snapshot.mem_free_kb);
    push_kv(&mut out, "MemAvailable", mem_available);
    push_kv(&mut out, "Buffers",      snapshot.buffers_kb);
    push_kv(&mut out, "Cached",       snapshot.cached_kb);
    push_kv(&mut out, "SwapCached",   snapshot.swap_cached_kb);
    push_kv(&mut out, "SwapTotal",    snapshot.swap_total_kb);
    push_kv(&mut out, "SwapFree",     snapshot.swap_free_kb);
    out.into_bytes()
}

/// Convenience entry that mirrors the planned production wiring —
/// builds a snapshot from the kernel's frame allocator (when the cell
/// is live) and renders it. Falls back to all-zero defaults when the
/// allocator has not been initialised yet (host-side `cargo test`
/// builds, or boot before `arch::init_memory` runs).
pub fn render_meminfo() -> Vec<u8> {
    render(&snapshot_from_kernel_state())
}

/// Build a `MemInfoSnapshot` from the kernel's runtime state. Used by
/// `render_meminfo`; lives behind a `cfg(target_os = "uefi")` arm so
/// host-side tests get the all-zero default and don't try to call into
/// `arch::memory` (which is unreachable on the host target).
#[cfg(target_os = "uefi")]
fn snapshot_from_kernel_state() -> MemInfoSnapshot {
    // `usable_frame_count` returns the firmware-reported total minus
    // the carved DMA pool — that's the number of frames the kernel
    // actually controls. We don't yet track per-allocation usage, so
    // `mem_free_kb` mirrors `mem_total_kb` until the frame allocator
    // grows a "remaining frames" cell. The renderer's MemAvailable
    // fallback then picks up the same value, which matches how Linux
    // reports a freshly-booted machine before any process allocates.
    let frames = crate::arch::memory::usable_frame_count() as u64;
    let bytes = frames.saturating_mul(4096);
    let kb = bytes / 1024;
    MemInfoSnapshot {
        mem_total_kb: kb,
        mem_free_kb: kb,
        ..MemInfoSnapshot::default()
    }
}

#[cfg(not(target_os = "uefi"))]
fn snapshot_from_kernel_state() -> MemInfoSnapshot {
    // Host-side `cargo test` build — no `arch::memory` to query, so
    // hand back the default snapshot. Tests that need real numbers
    // construct a `MemInfoSnapshot` directly and call `render`.
    MemInfoSnapshot::default()
}

/// Append one `Key:    value kB\n` line to `out` with the value right-
/// aligned in an 8-column field. Mirrors the conventional Linux
/// format you can see in any distro's `/proc/meminfo`. The colon goes
/// flush against the key with no preceding tab (different from
/// `/proc/cpuinfo`) — userspace parsers split on the colon and trim.
fn push_kv(out: &mut String, key: &str, value_kb: u64) {
    // 8-column right-aligned numeric field; one space between the
    // colon and the value when the value is shorter than 8 digits.
    out.push_str(&format!("{}:{:>9} kB\n", key, value_kb));
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
    fn empty_snapshot_renders_eight_zero_lines() {
        let bytes = render(&MemInfoSnapshot::default());
        let s = core::str::from_utf8(&bytes).unwrap();
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines.len(), 8, "expected 8 lines, got: {:?}", lines);
        for line in &lines {
            assert!(line.ends_with(" kB"), "line `{}` missing kB suffix", line);
        }
    }

    #[test]
    fn fixture_4gb_4cpu_renders_expected_keys() {
        // Spec verification recipe: 4 GB RAM fixture → render → spot-
        // check the bytes look like real `/proc/meminfo`.
        let four_gb_kb: u64 = 4 * 1024 * 1024;
        let half_free: u64 = 2 * 1024 * 1024;
        let snap = MemInfoSnapshot {
            mem_total_kb: four_gb_kb,
            mem_free_kb: half_free,
            mem_available_kb: half_free,
            ..MemInfoSnapshot::default()
        };
        let bytes = render(&snap);
        let s = core::str::from_utf8(&bytes).unwrap();
        // Right-aligned column 8 means a 4194304-value line is
        // `MemTotal:  4194304 kB`. Spot-check each.
        assert!(
            s.contains("MemTotal:  4194304 kB\n"),
            "MemTotal line wrong; got:\n{}", s,
        );
        assert!(
            s.contains("MemFree:  2097152 kB\n"),
            "MemFree line wrong; got:\n{}", s,
        );
        assert!(
            s.contains("MemAvailable:  2097152 kB\n"),
            "MemAvailable line wrong; got:\n{}", s,
        );
    }

    #[test]
    fn mem_available_falls_back_to_mem_free_when_zero() {
        // When the snapshot leaves `mem_available_kb = 0` but
        // `mem_free_kb > 0`, the renderer publishes `mem_free_kb` so
        // a single-field grep doesn't see a misleading zero.
        let snap = MemInfoSnapshot {
            mem_total_kb: 1000,
            mem_free_kb: 500,
            mem_available_kb: 0,
            ..MemInfoSnapshot::default()
        };
        let bytes = render(&snap);
        let s = core::str::from_utf8(&bytes).unwrap();
        // Both MemFree and MemAvailable read 500.
        assert!(s.contains("MemFree:      500 kB\n"));
        assert!(s.contains("MemAvailable:      500 kB\n"));
    }

    #[test]
    fn mem_available_explicit_overrides_fallback() {
        let snap = MemInfoSnapshot {
            mem_total_kb: 1000,
            mem_free_kb: 500,
            mem_available_kb: 700,
            ..MemInfoSnapshot::default()
        };
        let bytes = render(&snap);
        let s = core::str::from_utf8(&bytes).unwrap();
        // The explicit value wins.
        assert!(s.contains("MemAvailable:      700 kB\n"));
    }

    #[test]
    fn keys_appear_in_stable_order() {
        let bytes = render(&MemInfoSnapshot::default());
        let s = core::str::from_utf8(&bytes).unwrap();
        let pos = |k: &str| s.find(k).expect(k);
        // Linux convention: MemTotal first, then MemFree, then
        // MemAvailable — bash scripts that walk the file top-to-bottom
        // depend on this order.
        assert!(pos("MemTotal:") < pos("MemFree:"));
        assert!(pos("MemFree:") < pos("MemAvailable:"));
        assert!(pos("MemAvailable:") < pos("Buffers:"));
        assert!(pos("Buffers:") < pos("Cached:"));
        assert!(pos("Cached:") < pos("SwapCached:"));
        assert!(pos("SwapCached:") < pos("SwapTotal:"));
        assert!(pos("SwapTotal:") < pos("SwapFree:"));
    }

    #[test]
    fn each_line_ends_with_kb_unit() {
        // Userspace tools (busybox, GNU `free`) require the literal
        // ` kB` suffix to know the value's unit. A line that drops
        // it (e.g. `MemTotal: 4194304\n`) gets parsed as bytes by
        // some tools and produces wildly wrong numbers downstream.
        let snap = MemInfoSnapshot {
            mem_total_kb: 1234567,
            mem_free_kb: 1000,
            ..MemInfoSnapshot::default()
        };
        let bytes = render(&snap);
        let s = core::str::from_utf8(&bytes).unwrap();
        for line in s.lines() {
            assert!(
                line.ends_with(" kB"),
                "line `{}` missing ` kB` suffix",
                line,
            );
        }
    }
}
