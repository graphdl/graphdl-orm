# 24. UI Toolkit Decision — egui vs Slint (#425)

## Why this commit

Epic #424 lifts the boot-time framebuffer demo (#270/#271 Doom blit) into
a real on-device UI surface for the kernel. The driver is already in place
(`crates/arest-kernel/src/framebuffer.rs` — triple-buffered linear
framebuffer with damage tracking, `with_back(|back| ...)` + `present()`).
What's missing is a widget toolkit on top: layout, text, theming, input
routing.

Two crates dominate the no-GPU Rust UI space at the level we need:

- **egui** (`emilk/egui`) — immediate-mode, retained-mode-free, no DSL,
  triangle-list output via `epaint`. Caller writes plain Rust closures
  per frame; toolkit emits tessellated triangles + texture atlases that
  the caller rasterises onto whatever surface they own.
- **Slint** (`slint-ui/slint`) — declarative `.slint` DSL compiled at
  build time, retained-mode tree, optional `software_renderer` that
  paints pixels directly into a surface the caller provides via the
  `LineBufferProvider` trait.

This doc records the choice and the reasoning. NO Cargo.toml change in
this commit — wiring lands under #426.

## Comparison

Ranked criteria (most → least decisive for AREST kernel):

| # | Criterion                       | egui                                                     | Slint                                                    | Edge     |
|---|---------------------------------|----------------------------------------------------------|----------------------------------------------------------|----------|
| 1 | `no_std` support                | Not officially supported; community fork only ([discussion #1251][egui-1251], unmerged since 2022) | First-class; documented MCU target with `default-features = false` + `compat-1-2`, `unsafe-single-threaded`, `libm`, `renderer-software` ([slint MCU docs][slint-mcu]) | **Slint** |
| 2 | Framebuffer backend             | `epaint` emits `Vec<ClippedPrimitive>` of triangle meshes + texture atlases — caller writes a triangle rasteriser (or vendors `egui_software_backend` v0.0.3, ~unmaintained) | `SoftwareRenderer::render_by_line` calls `LineBufferProvider::process_line(line, range, render_fn)` — render_fn fills `&mut [TargetPixel]` directly. Trivial wrapper around `framebuffer::with_back` | **Slint** |
| 3 | RAM footprint                   | Unknown for kernel target; immediate-mode rebuilds widget tree per frame, pulls hashbrown maps. Demos on desktop run 5–10 MiB heap | Documented MCU runtime < 300 KiB for the toolkit itself; `render_by_line` adds one scanline (~5 KiB at 1280×24bpp) instead of a full back buffer ([Slint MCU port][slint-port]) | **Slint** (well under 2 MiB budget) |
| 4 | Build-time complexity           | Pure Rust, no codegen, no proc-macro magic beyond what's already in the tree | Adds `slint-build` as a build-dep; `build.rs` compiles `.slint` into Rust with `EmbedResourcesKind::EmbedForSoftwareRenderer`. Nightly tolerated, no unstable features required. | **egui** (slightly), but the cost is one `build.rs` + one DSL file |
| 5 | License                         | MIT OR Apache-2.0                                        | MIT OR Apache-2.0 (also GPLv3 / commercial — we pick MIT/Apache) | tie       |
| 6 | Dep tree size                   | "egui has ten times as many dependencies as both iced and slint" — 2025 Rust GUI survey | Lean — `slint` + `i-slint-core` + `slint-build` + `libm` for the no_std path | **Slint** |
| 7 | Activity / community / kernel precedent | Active, but every embedded story ends at "use the SDL/wgpu backend"; community no_std backend is `0.0.3` and tracks egui 0.34 only | Active (1.15 shipped 2026), embedded is a first-class market — STM32H7 ports, RP2040 ports, official MCU board-support crate, kernel-class targets in CI | **Slint** |

## Decision

**Slint.**

The decision is not close. egui is the more enjoyable toolkit to write
*against* on a hosted target — its immediate-mode model maps cleanly to
Rust ownership and there's no DSL — but every criterion that matters for
an in-kernel UEFI surface points the other way:

1. **`no_std` is not optional for us** and Slint ships it as a supported
   configuration; egui requires either an unofficial fork or carrying a
   patch set forward indefinitely.
2. **The framebuffer integration shape is decisively better** — Slint's
   `LineBufferProvider` is a 3-line trait impl that hands us back a
   `&mut [TargetPixel]` per scanline. egui hands us triangles to
   rasterise, which means we either ship a software triangle rasteriser
   (one we'd own and test forever) or vendor a 41-star unmaintained
   crate.
3. **RAM budget fits with margin.** Slint's < 300 KiB runtime + one
   ~5 KiB scanline buffer leaves the AREST 16 MiB heap with room for
   wasmi, smoltcp, and the Doom WAD. egui's per-frame widget-tree
   rebuild + `epaint` mesh generation is unbenchmarked at this target
   class and trivially blows the < 2 MiB goal under even a modest UI.

The DSL cost (Slint requires `.slint` files compiled by `build.rs`) is
real but bounded — it's one extra file type and a build-dep, both of
which the kernel already tolerates (`build.rs` exists already for the
custom-target JSON file under `arest-kernel-armv7-uefi.json`).

## Integration sketch (lands in #426)

```rust
// crates/arest-kernel/src/ui/slint_backend.rs (sketch — #426 will land)
struct ArestLineBuffer;

impl slint::platform::software_renderer::LineBufferProvider for ArestLineBuffer {
    type TargetPixel = slint::platform::software_renderer::Rgb565Pixel; // or Rgb888

    fn process_line(
        &mut self,
        line: usize,
        range: core::ops::Range<usize>,
        render_fn: impl FnOnce(&mut [Self::TargetPixel]),
    ) {
        framebuffer::with_back(|back| {
            // Borrow the dest slice for this scanline from the active back
            // buffer (see framebuffer::BackBuffer::bytes); hand it to render_fn.
            // present() at end of frame flushes the dirty rect.
        });
    }
}
```

The `LineBufferProvider` impl is the only non-trivial bridge code; the
rest is `slint::platform::set_platform(...)` once at boot, then a
`MainWindow::new()?.run()?` analogue under `kernel_run`.

## What ships in this commit

This document only. Code, Cargo.toml, and the `ui/` module follow under
the #424 chain (#426 = Slint dep + scaffold; #427-#431 = widgets, input,
theming, REPL surface, settings UI).

[egui-1251]: https://github.com/emilk/egui/discussions/1251
[slint-mcu]: https://docs.rs/slint/latest/slint/docs/mcu/index.html
[slint-port]: https://slint.dev/blog/porting-slint-to-microcontrollers
