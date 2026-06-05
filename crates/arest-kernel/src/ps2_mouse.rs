// crates/arest-kernel/src/ps2_mouse.rs
//
// Pure PS/2 mouse packet logic — host-testable core of the UEFI mouse
// driver (#598 pointer producer).
//
// The 8042 aux device (the mouse) delivers 3-byte packets on IRQ 12.
// This module owns the *logic* — decoding a packet into a motion delta
// + button state, assembling the byte stream into packets (with
// resync), and tracking button-press edges — with zero hardware or
// `arch::uefi` dependencies, so it compiles + tests under the hosted
// `cargo test` target. The hardware half (8042 ports, the IRQ 12
// handler, the `PointerEvent` push) lives in `arch::uefi::mouse`, which
// is `target_os="uefi"`-gated and cannot host-test. Same split
// rationale as `unified_repl_regions`.

#![allow(dead_code)]

/// Logical mouse button. The UEFI driver maps these to Linux `BTN_*`
/// codes when pushing `pointer::PointerEvent::Button`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// Decoded motion + button state from one standard 3-byte PS/2 packet.
/// `dy` is already in screen space (PS/2 reports +y as up; this flips
/// it to +down).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MouseUpdate {
    pub dx: i32,
    pub dy: i32,
    pub left: bool,
    pub right: bool,
    pub middle: bool,
}

/// Decode a standard 3-byte PS/2 mouse packet.
///
/// Byte 0 bits: `[y-overflow x-overflow y-sign x-sign 1 middle right left]`.
/// Bytes 1/2 are the low 8 bits of the 9-bit two's-complement X/Y
/// deltas; the 9th (sign) bit is `x-sign` / `y-sign` in byte 0.
///
/// On an overflow bit set, that axis's motion is dropped (0) rather
/// than decoded into a wild jump — the next packet recovers cleanly.
/// `dy` is negated so the result is screen-space (+down).
pub fn parse_packet(b0: u8, b1: u8, b2: u8) -> MouseUpdate {
    let left = b0 & 0x01 != 0;
    let right = b0 & 0x02 != 0;
    let middle = b0 & 0x04 != 0;

    // 9-bit two's-complement: low 8 bits in b1/b2, sign bit in b0.
    // Drop the axis on overflow rather than decoding a wild jump.
    let dx = if b0 & 0x40 != 0 {
        0
    } else if b0 & 0x10 != 0 {
        (b1 as i32) - 0x100
    } else {
        b1 as i32
    };
    let dy_ps2 = if b0 & 0x80 != 0 {
        0
    } else if b0 & 0x20 != 0 {
        (b2 as i32) - 0x100
    } else {
        b2 as i32
    };

    // PS/2 reports +y as up; flip to screen space (+down).
    MouseUpdate { dx, dy: -dy_ps2, left, right, middle }
}

/// One emitted frame after a packet completes: motion + the button
/// transitions (edges) observed relative to the previous packet. A
/// `None` edge means "unchanged"; `Some(true)` / `Some(false)` is a
/// press / release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MouseFrame {
    pub dx: i32,
    pub dy: i32,
    pub left_edge: Option<bool>,
    pub right_edge: Option<bool>,
    pub middle_edge: Option<bool>,
}

/// 3-byte packet assembler + button-edge tracker. Single-writer (the
/// IRQ 12 handler holds it under a `Mutex` in the UEFI driver).
#[derive(Debug, Clone, Copy)]
pub struct Accumulator {
    buf: [u8; 3],
    idx: usize,
    left: bool,
    right: bool,
    middle: bool,
}

impl Default for Accumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl Accumulator {
    pub const fn new() -> Self {
        Self { buf: [0; 3], idx: 0, left: false, right: false, middle: false }
    }

    /// Feed one aux byte. Returns `Some(frame)` when the third byte of
    /// a packet lands; `None` mid-packet, or when a stray first byte
    /// (missing byte-0's always-set marker bit `0x08`) is dropped to
    /// resynchronise the stream.
    pub fn feed(&mut self, byte: u8) -> Option<MouseFrame> {
        // Resync: a valid first byte always has the marker bit (0x08)
        // set. If it's clear we're mid-stream out of frame — drop the
        // byte and stay at index 0 until a plausible header arrives.
        if self.idx == 0 && byte & 0x08 == 0 {
            return None;
        }
        self.buf[self.idx] = byte;
        self.idx += 1;
        if self.idx < 3 {
            return None;
        }
        self.idx = 0;

        let upd = parse_packet(self.buf[0], self.buf[1], self.buf[2]);
        let left_edge = (upd.left != self.left).then_some(upd.left);
        let right_edge = (upd.right != self.right).then_some(upd.right);
        let middle_edge = (upd.middle != self.middle).then_some(upd.middle);
        self.left = upd.left;
        self.right = upd.right;
        self.middle = upd.middle;

        Some(MouseFrame {
            dx: upd.dx,
            dy: upd.dy,
            left_edge,
            right_edge,
            middle_edge,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_packet ────────────────────────────────────────────────

    #[test]
    fn parse_zero_packet_is_idle() {
        assert_eq!(parse_packet(0x08, 0, 0), MouseUpdate::default());
    }

    #[test]
    fn parse_positive_x_motion() {
        let u = parse_packet(0x08, 5, 0);
        assert_eq!(u.dx, 5);
        assert_eq!(u.dy, 0);
    }

    #[test]
    fn parse_positive_y_is_negated_to_screen_space() {
        // PS/2 reports +y as UP; screen +y is DOWN.
        let u = parse_packet(0x08, 0, 5);
        assert_eq!(u.dy, -5);
        assert_eq!(u.dx, 0);
    }

    #[test]
    fn parse_negative_x_sign_extends() {
        // x-sign (0x10) set, byte1 = 0xFB (251) → 251 - 256 = -5.
        assert_eq!(parse_packet(0x18, 0xFB, 0).dx, -5);
    }

    #[test]
    fn parse_negative_y_sign_extends_and_negates() {
        // y-sign (0x20) set, byte2 = 0xFB → PS/2 dy = -5 → screen +5.
        assert_eq!(parse_packet(0x28, 0, 0xFB).dy, 5);
    }

    #[test]
    fn parse_buttons() {
        let l = parse_packet(0x09, 0, 0); // marker + left
        assert!(l.left && !l.right && !l.middle);
        let r = parse_packet(0x0A, 0, 0); // marker + right
        assert!(!r.left && r.right && !r.middle);
        let m = parse_packet(0x0C, 0, 0); // marker + middle
        assert!(!m.left && !m.right && m.middle);
        let all = parse_packet(0x0F, 0, 0); // marker + all three
        assert!(all.left && all.right && all.middle);
    }

    #[test]
    fn parse_overflow_drops_that_axis() {
        // x-overflow (0x40) set → dx dropped to 0 regardless of byte1.
        assert_eq!(parse_packet(0x48, 0xFF, 0).dx, 0);
        // y-overflow (0x80) set → dy dropped to 0.
        assert_eq!(parse_packet(0x88, 0, 0xFF).dy, 0);
    }

    // ── Accumulator ─────────────────────────────────────────────────

    #[test]
    fn feed_assembles_three_bytes() {
        let mut acc = Accumulator::new();
        assert_eq!(acc.feed(0x08), None); // byte 0 (marker)
        assert_eq!(acc.feed(5), None); // byte 1 (dx)
        let frame = acc.feed(0).expect("packet completes on 3rd byte");
        assert_eq!(frame.dx, 5);
        assert_eq!(frame.dy, 0);
    }

    #[test]
    fn feed_resyncs_on_missing_marker_bit() {
        let mut acc = Accumulator::new();
        // A first byte without 0x08 is impossible for a real header —
        // drop it and stay at index 0 rather than mis-framing.
        assert_eq!(acc.feed(0x00), None);
        // The next real packet still assembles correctly.
        assert_eq!(acc.feed(0x08), None);
        assert_eq!(acc.feed(3), None);
        assert_eq!(acc.feed(0).map(|f| f.dx), Some(3));
    }

    #[test]
    fn feed_tracks_button_edges() {
        let mut acc = Accumulator::new();
        // Press left (marker 0x08 | left 0x01), no motion.
        acc.feed(0x09);
        acc.feed(0);
        let press = acc.feed(0).expect("packet");
        assert_eq!(press.left_edge, Some(true));
        // Hold left — no new edge.
        acc.feed(0x09);
        acc.feed(0);
        let hold = acc.feed(0).expect("packet");
        assert_eq!(hold.left_edge, None);
        // Release left.
        acc.feed(0x08);
        acc.feed(0);
        let release = acc.feed(0).expect("packet");
        assert_eq!(release.left_edge, Some(false));
    }
}
