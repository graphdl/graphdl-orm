# AREST UI: Render — Display, Surface, Frame

This reading reifies the rendering substrate as FORML 2 fact types and
instances. Today the kernel renders into a firmware-mapped framebuffer
(UEFI `GraphicsOutputProtocol`) or a virtio-gpu DMA-backed surface via
plain Rust types in `crates/arest-kernel/src/framebuffer.rs`. After
this reading lands, the rendering substrate is a graph of cells the
same way every other AREST concept is — a Display, Surfaces drawn into
that Display, and Frames presented onto those Surfaces. The kernel's
existing triple-buffer driver becomes one population of these facts,
and future targets (a phone with HDMI + DSI, a workstation with three
monitors, a headless build agent with a virtual surface) populate the
same entity types without inventing new ground-truth.

This reading is additive over `readings/ui/ui.md` (the platform-
agnostic view hierarchy), `readings/ui/monoview.md` (the per-app
render surface), and `readings/ui/components.md` (the toolkit-side
widget registry). The composition direction is the obvious
containment chain: a Frame belongs to a Surface, a Surface belongs to
a Display, and the MonoView regions defined in #457 project content
into Surfaces without restating any of the geometry here. The
component selection rules in #485 already score against MonoView
constraints; once a binding is selected, its rendered output lands in
some Surface as a sequence of Frames.

## Entity Types

Display(.Name) is an entity type.
  <!-- The physical or virtual output the kernel drives. The .Name
       reference mode is a stable slug — 'gop' for the firmware-
       mapped UEFI surface, 'virtio-gpu-0' for the virtio-gpu scanout
       wired by `install_virtio_gpu`, 'hdmi-0' / 'dsi-0' for future
       multi-output hardware. One Display per kernel boot today (the
       single GOP surface); the model admits many. -->

Surface(.Name) is an entity type.
  <!-- A logical drawable region within a Display. Slint windows, the
       Doom canvas, MonoView panes, and the boot paint smoke are all
       Surfaces. The .Name reference mode is a stable slug; the
       Origin / Width / Height fact types pin the rectangle in the
       parent Display's coordinate space. -->

Frame(.Name) is an entity type.
  <!-- A snapshot in time of a Surface's pixel content. The .Name
       reference mode is a per-Surface monotonic slug (typically the
       Surface slug suffixed with the Frame Index). One Frame per
       presented buffer; the kernel's triple-buffer driver advances
       Frame Index every time `framebuffer::present()` finds a non-
       empty dirty rect. -->

Pixel Format is an entity type.
  <!-- Closed catalog of the pixel layouts the kernel actually
       populates from `GraphicsOutputProtocol`'s `PixelFormat`
       enumeration (UEFI section 12.9). The reference scheme is the
       canonical slug of the variant. The Rust-side `PixelFormat`
       enum in framebuffer.rs is the ground truth; this entity type
       reifies the variant set so derivation rules can join through
       it without a free-form string. -->

## Value Types

Display Slug is a value type.
  <!-- Free-form identifier for a Display instance, e.g. 'gop',
       'virtio-gpu-0'. Opaque to the readings checker. -->

Surface Slug is a value type.
  <!-- Free-form identifier for a Surface instance, e.g.
       'kernel.boot-paint', 'doom.canvas', 'slint.hateoas'. -->

Frame Slug is a value type.
  <!-- Free-form identifier for a Frame instance, typically derived
       as '<surface>.<frame-index>' but the readings layer treats it
       as opaque. -->

Pixel Format Slug is a value type.
  The possible values of Pixel Format Slug are
    'rgb', 'bgr', 'u8', 'unknown'.
  <!-- One slug per `PixelFormat` variant in framebuffer.rs. 'rgb'
       and 'bgr' are the two byte-orderings GOP and virtio-gpu
       actually report; 'u8' is the greyscale fallback the draw
       helpers honour; 'unknown' covers GOP's `Bitmask` and `BltOnly`
       variants — surfaces flagged 'unknown' silently no-op writes
       rather than corrupting the display. -->

Pixel Width is a value type.
  <!-- Horizontal extent in pixels. Non-negative integer. -->

Pixel Height is a value type.
  <!-- Vertical extent in pixels. Non-negative integer. -->

Pixel Origin is a value type.
  <!-- A coordinate component (X or Y) in the parent Display's pixel
       space. Non-negative integer; the origin is the top-left of the
       Surface rectangle. -->

Refresh Rate is a value type.
  <!-- Nominal vertical refresh in Hz. Free-form integer; absent on
       Displays whose firmware does not report a refresh rate (the
       firmware-mapped GOP surface typically does not). -->

Display Backend is a value type.
  The possible values of Display Backend are
    'gop', 'virtio-gpu', 'headless'.
  <!-- The transport that carries Frame bytes to the Display.
       'gop' = firmware-mapped MMIO surface (UEFI
       `GraphicsOutputProtocol`); the GPU reads it on its own vsync.
       'virtio-gpu' = DMA-backed 2D resource attached to a scanout;
       `present()` issues a `RESOURCE_FLUSH` so the host blits to the
       display (virtio-gpu spec section 5.7.6.7).
       'headless' = no firmware surface; rendering goes through the
       hash-only smoke path (`front_fnv1a` checksum without an
       attached display). -->

Frame Index is a value type.
  <!-- Monotonic non-negative integer. Increments every time
       `framebuffer::present()` finds a non-empty dirty rect on the
       owning Surface. The kernel's `presents` counter is the
       runtime peer. -->

Wall Time is a value type.
  <!-- Free-form ISO-8601 timestamp string. Opaque to the readings
       checker. Optional on every Frame; populated when the kernel
       has a wall clock and chooses to record it (smoke-test traces
       always do, hot rendering paths usually do not). -->

Frame Bytes is a value type.
  <!-- Opaque byte payload carrying the Frame's pixel content. The
       readings layer does not interpret the bytes; consumers read
       the parent Surface's Pixel Format to decode. The kernel-side
       BackBuffer's byte slice is the runtime peer. -->

Frame Hash is a value type.
  <!-- FNV-1a checksum of a Frame's bytes. Used by the boot paint
       smoke (`front_fnv1a` / `back_fnv1a`) and by the eventual
       diff-presentation path to detect "did anything change since
       the last Frame on this Surface". Opaque hex-string at the
       readings layer. -->

## Fact Types

### Display

Display has Display Slug.
  Each Display has exactly one Display Slug.

Display has Pixel Width.
  Each Display has exactly one Pixel Width.

Display has Pixel Height.
  Each Display has exactly one Pixel Height.

Display has Pixel Format.
  Each Display has exactly one Pixel Format.

Display has Refresh Rate.
  Each Display has at most one Refresh Rate.

Display has Display Backend.
  Each Display has exactly one Display Backend.

Display has display- Title.
  Each Display has at most one display- Title.

### Pixel Format

Pixel Format has Pixel Format Slug.
  Each Pixel Format has exactly one Pixel Format Slug.

Pixel Format has display- Title.
  Each Pixel Format has at most one display- Title.

### Surface

Surface belongs to Display.
  Each Surface belongs to exactly one Display.
  <!-- Containment is single-parent: a Surface lives in exactly one
       Display. Cross-Display surfaces (a window dragged between
       monitors) are modelled as a new Surface on the destination
       Display, not as a Surface with two parents — matching how the
       kernel's `framebuffer::install` singleton behaves today. -->

Surface has Surface Slug.
  Each Surface has exactly one Surface Slug.

Surface has Pixel Origin as origin- X.
  Each Surface has exactly one origin- X Pixel Origin.

Surface has Pixel Origin as origin- Y.
  Each Surface has exactly one origin- Y Pixel Origin.

Surface has Pixel Width.
  Each Surface has exactly one Pixel Width.

Surface has Pixel Height.
  Each Surface has exactly one Pixel Height.

Surface has display- Title.
  Each Surface has at most one display- Title.

Surface has Description.
  Each Surface has at most one Description.

### Frame

Frame belongs to Surface.
  Each Frame belongs to exactly one Surface.
  <!-- Containment chain: Frame belongs-to Surface belongs-to
       Display. The kernel's triple-buffer driver produces one Frame
       per `present()` call that finds a non-empty dirty rect; the
       owning Surface is the one whose BackBuffer was active. -->

Frame has Frame Slug.
  Each Frame has exactly one Frame Slug.

Frame has Frame Index.
  Each Frame has exactly one Frame Index.

Frame has Wall Time.
  Each Frame has at most one Wall Time.

Frame has Frame Bytes.
  Each Frame has at most one Frame Bytes.

Frame has Frame Hash.
  Each Frame has at most one Frame Hash.

## Constraints

No two Displays share the same Display Slug.
No two Surfaces belonging to the same Display share the same Surface Slug.
No two Frames belonging to the same Surface share the same Frame Index.
No two Pixel Formats share the same Pixel Format Slug.

## Deontic Constraints

It is obligatory that each Display has some Display Backend.
It is obligatory that each Display has some Pixel Format.
It is obligatory that each Surface belongs to some Display.
It is obligatory that each Frame belongs to some Surface.
It is obligatory that each Frame has some Frame Index.

## Derivation Rules

### Surface inherits Pixel Format

+ Surface has Pixel Format
    if Surface belongs to Display
    and Display has Pixel Format.
  <!-- A Surface's pixel layout follows its Display's by default.
       Surfaces that re-encode (e.g. a virtual headless surface
       capturing 'rgb' regardless of the parent's 'bgr') override
       the inherited Pixel Format with an explicit fact at the
       instance layer; FORML 2 derivation rules contribute facts
       additively, so the explicit value wins via the cardinality
       constraint elsewhere. -->

### Frame inherits Surface dimensions

+ Frame has Pixel Width
    if Frame belongs to Surface
    and Surface has Pixel Width.

+ Frame has Pixel Height
    if Frame belongs to Surface
    and Surface has Pixel Height.

## Instance Facts

Domain 'ui' has Description 'Render substrate — Display, Surface, and Frame as FORML 2 facts. The kernel framebuffer driver, virtio-gpu scanout, and future multi-output targets all populate the same entity types; the MonoView regions defined in #457 project content into Surfaces without restating any geometry here.'.

### Pixel Format catalog

Pixel Format 'rgb' has Pixel Format Slug 'rgb'.
Pixel Format 'rgb' has display- Title 'RGB linear'.

Pixel Format 'bgr' has Pixel Format Slug 'bgr'.
Pixel Format 'bgr' has display- Title 'BGR linear'.

Pixel Format 'u8' has Pixel Format Slug 'u8'.
Pixel Format 'u8' has display- Title '8-bit greyscale'.

Pixel Format 'unknown' has Pixel Format Slug 'unknown'.
Pixel Format 'unknown' has display- Title 'Unmapped (Bitmask / BltOnly)'.

### Display: GOP firmware-mapped framebuffer

Display 'gop' has Display Slug 'gop'.
Display 'gop' has display- Title 'UEFI GOP framebuffer'.
Display 'gop' has Pixel Width 1280.
Display 'gop' has Pixel Height 720.
Display 'gop' has Pixel Format 'bgr'.
Display 'gop' has Display Backend 'gop'.

### Surface: boot paint smoke

Surface 'kernel.boot-paint' belongs to Display 'gop'.
Surface 'kernel.boot-paint' has Surface Slug 'kernel.boot-paint'.
Surface 'kernel.boot-paint' has Pixel Origin 0 as origin- X.
Surface 'kernel.boot-paint' has Pixel Origin 0 as origin- Y.
Surface 'kernel.boot-paint' has Pixel Width 1280.
Surface 'kernel.boot-paint' has Pixel Height 720.
Surface 'kernel.boot-paint' has display- Title 'Boot paint smoke'.
Surface 'kernel.boot-paint' has Description 'Whole-screen surface the kernel paints at boot to verify the framebuffer chain. The FNV-1a checksum over its first Frame is asserted by the host harness.'.

### Surface: Doom canvas

Surface 'doom.canvas' belongs to Display 'gop'.
Surface 'doom.canvas' has Surface Slug 'doom.canvas'.
Surface 'doom.canvas' has Pixel Origin 320 as origin- X.
Surface 'doom.canvas' has Pixel Origin 160 as origin- Y.
Surface 'doom.canvas' has Pixel Width 640.
Surface 'doom.canvas' has Pixel Height 400.
Surface 'doom.canvas' has display- Title 'Doom canvas'.
Surface 'doom.canvas' has Description 'Doom-host shim renders into this centered 640x400 surface. The surrounding letterbox stays whatever colour the kernel painted before the blit.'.
