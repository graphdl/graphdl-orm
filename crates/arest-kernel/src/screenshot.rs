//! Framebuffer -> PNG screenshot encoding for the headless see-and-drive
//! surface (task `kernel-see-drive-surface`).
//!
//! Pure `no_std` + `alloc` logic: turns a raw RGB pixel buffer into a
//! valid PNG byte stream the agent can `Read` over `GET /screen`. Uses
//! DEFLATE *stored* (uncompressed) blocks so there is no compression
//! dependency in the kernel -- just PNG framing, a CRC-32 over each
//! chunk, and an Adler-32 over the zlib stream. Host-tested by round-
//! tripping through the `png` crate (see tests); run with
//! `cargo test --lib --target x86_64-pc-windows-msvc` from
//! `crates/arest-kernel`, the same harness `ps2_mouse` uses.

use alloc::vec::Vec;

/// Encode a tightly-packed RGB8 buffer (`width * height * 3` bytes,
/// row-major, R,G,B order) as a PNG byte stream (color type 2, 8-bit).
///
/// Returns an empty `Vec` for a degenerate input (zero area, or a buffer
/// whose length is not exactly `width * height * 3`); every well-formed
/// input yields a PNG that a conforming decoder accepts.
pub fn encode_png_rgb(rgb: &[u8], width: usize, height: usize) -> Vec<u8> {
    // Degenerate input: zero area, or a buffer that isn't exactly
    // width*height*3. Caller gets an empty Vec (the /screen handler
    // treats that as "no frame" rather than serving a corrupt PNG).
    if width == 0 || height == 0 || rgb.len() != width * height * 3 {
        return Vec::new();
    }

    let mut out = Vec::new();
    // PNG signature -- the 8 magic bytes every PNG opens with.
    out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);

    // IHDR: dimensions + 8-bit truecolour RGB (color type 2), DEFLATE
    // compression, no interlace.
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(2); // color type: truecolour (R,G,B)
    ihdr.push(0); // compression method: DEFLATE
    ihdr.push(0); // filter method: adaptive (only "None" used below)
    ihdr.push(0); // interlace: none
    write_chunk(&mut out, b"IHDR", &ihdr);

    // Filtered scanlines: each row is prefixed with a filter-type byte
    // (0 = None) per the PNG spec, then the row's RGB bytes verbatim.
    let row_bytes = width * 3;
    let mut raw = Vec::with_capacity((row_bytes + 1) * height);
    for y in 0..height {
        raw.push(0); // filter type: None
        let start = y * row_bytes;
        raw.extend_from_slice(&rgb[start..start + row_bytes]);
    }

    // IDAT: a zlib stream wrapping the filtered data in DEFLATE *stored*
    // (uncompressed) blocks -- no compressor in the kernel, just framing.
    let mut zlib = Vec::new();
    zlib.push(0x78); // CMF: DEFLATE, 32K window
    zlib.push(0x01); // FLG: no preset dict, fastest (0x7801 % 31 == 0)
    let mut i = 0;
    while i < raw.len() {
        let block = (raw.len() - i).min(0xFFFF);
        let is_final = i + block >= raw.len();
        zlib.push(if is_final { 1 } else { 0 }); // BFINAL bit + BTYPE 00
        let len = block as u16;
        zlib.extend_from_slice(&len.to_le_bytes()); // LEN
        zlib.extend_from_slice(&(!len).to_le_bytes()); // NLEN (~LEN)
        zlib.extend_from_slice(&raw[i..i + block]);
        i += block;
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes()); // zlib trailer

    write_chunk(&mut out, b"IDAT", &zlib);
    write_chunk(&mut out, b"IEND", &[]);
    out
}

/// Append one PNG chunk: `len(4) | type(4) | data | CRC-32(4)`, all
/// big-endian; the CRC covers the type tag + the data.
fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(kind, data).to_be_bytes());
}

/// PNG CRC-32 (reflected, polynomial 0xEDB88320) over the chunk type tag
/// followed by its data. Table-free bitwise form -- keeps the kernel
/// image free of a 1 KiB lookup table at the cost of 8 shifts per byte.
fn crc32(kind: &[u8; 4], data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in kind.iter().chain(data.iter()) {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    crc ^ 0xFFFF_FFFF
}

/// Adler-32 over the uncompressed (filtered) data -- the zlib stream's
/// trailing checksum.
fn adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// Decode a native-format framebuffer byte buffer into tightly-packed
/// RGB8 (`width*height*3`, row-major, R,G,B) -- the bridge from a
/// surface's `BackBuffer.bytes` to `encode_png_rgb` for `/screen`.
/// Drops stride padding and the X/alpha byte, and honours the surface's
/// `PixelFormat` channel order (the classic BGR-vs-RGB swap lives here,
/// so it has a direct test). Channel reads are bounds-checked, so a
/// short or mis-described buffer yields black pixels rather than a panic.
pub fn framebuffer_to_rgb(
    bytes: &[u8],
    width: usize,
    height: usize,
    stride: usize,
    bytes_per_pixel: usize,
    fmt: crate::framebuffer::PixelFormat,
) -> Vec<u8> {
    use crate::framebuffer::PixelFormat;
    let mut out = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        for x in 0..width {
            let off = y * stride * bytes_per_pixel + x * bytes_per_pixel;
            let px = bytes.get(off..off + bytes_per_pixel).unwrap_or(&[]);
            let ch = |i: usize| px.get(i).copied().unwrap_or(0);
            let (r, g, b) = match fmt {
                PixelFormat::Rgb => (ch(0), ch(1), ch(2)),
                PixelFormat::Bgr => (ch(2), ch(1), ch(0)),
                PixelFormat::U8 => (ch(0), ch(0), ch(0)),
                PixelFormat::Unknown => (0, 0, 0),
            };
            out.push(r);
            out.push(g);
            out.push(b);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_png_rgb_round_trips_2x2() {
        // 2x2 RGB: red, green / blue, white.
        let src: [u8; 12] = [
            255, 0, 0, 0, 255, 0, //
            0, 0, 255, 255, 255, 255,
        ];
        let png_bytes = encode_png_rgb(&src, 2, 2);

        let decoder = png::Decoder::new(png_bytes.as_slice());
        let mut reader = decoder
            .read_info()
            .expect("encoder must emit a decodable PNG header");
        let mut buf = alloc::vec![0u8; reader.output_buffer_size()];
        let info = reader
            .next_frame(&mut buf)
            .expect("encoder must emit a decodable PNG frame");

        assert_eq!(info.width, 2);
        assert_eq!(info.height, 2);
        assert_eq!(info.color_type, png::ColorType::Rgb);
        assert_eq!(&buf[..info.buffer_size()], &src[..]);
    }

    #[test]
    fn framebuffer_to_rgb_decodes_bgrx_dropping_stride_and_x() {
        use crate::framebuffer::PixelFormat;
        // 2x2 logical, BGRX (bpp=4), stride=3 px (one padding column).
        // Row 0: blue, green, <pad>. Row 1: red, white, <pad>.
        let bytes = alloc::vec![
            255u8, 0, 0, 0, 0, 255, 0, 0, 9, 9, 9, 9, // row0: blue, green, pad
            0, 0, 255, 0, 255, 255, 255, 0, 8, 8, 8, 8, // row1: red, white, pad
        ];
        let rgb = framebuffer_to_rgb(&bytes, 2, 2, 3, 4, PixelFormat::Bgr);
        assert_eq!(
            rgb,
            alloc::vec![
                0, 0, 255, 0, 255, 0, // blue, green
                255, 0, 0, 255, 255, 255, // red, white
            ]
        );
    }
}
