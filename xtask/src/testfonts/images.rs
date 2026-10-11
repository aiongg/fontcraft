//! Small images drawn in code for the colour-font checks: PNG (stored, uncompressed deflate),
//! baseline uncompressed TIFF, and an SVG that embeds a PNG.

/// An RGBA picture: a disc of `rgb` on a transparent background, with a square in the corner
/// so that orientation shows.
pub fn picture(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    let (cx, cy) = (f64::from(w) / 2.0, f64::from(h) / 2.0);
    let r = cx.min(cy) * 0.8;
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (f64::from(x) + 0.5 - cx, f64::from(y) + 0.5 - cy);
            let corner = x < w / 6 && y < h / 6;
            if corner {
                px.extend_from_slice(&[0, 0, 0, 255]);
            } else if dx * dx + dy * dy <= r * r {
                px.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            } else {
                px.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    px
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// A PNG file of an RGBA picture (`w` × `h`, rows top to bottom).
pub fn png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::new();
    for row in rgba.chunks((w * 4) as usize) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    // A zlib stream of stored blocks.
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (i, b) in blocks.iter().enumerate() {
        z.push(u8::from(i + 1 == blocks.len()));
        let len = b.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(b);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// A little-endian, uncompressed RGBA TIFF file.
pub fn tiff(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let entries: u16 = 11;
    let ifd = 8u32;
    let after_ifd = ifd + 2 + u32::from(entries) * 12 + 4;
    let bits_at = after_ifd;
    let pixels_at = bits_at + 8;
    let mut out = b"II*\0".to_vec();
    out.extend_from_slice(&ifd.to_le_bytes());
    out.extend_from_slice(&entries.to_le_bytes());
    let mut entry = |tag: u16, kind: u16, count: u32, value: u32| {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&value.to_le_bytes());
    };
    let (short, long) = (3u16, 4u16);
    entry(256, long, 1, w);
    entry(257, long, 1, h);
    entry(258, short, 4, bits_at);
    entry(259, short, 1, 1);
    entry(262, short, 1, 2);
    entry(273, long, 1, pixels_at);
    entry(277, short, 1, 4);
    entry(278, long, 1, h);
    entry(279, long, 1, w * h * 4);
    entry(284, short, 1, 1);
    // Extra sample: unassociated alpha.
    entry(338, short, 1, 2);
    out.extend_from_slice(&0u32.to_le_bytes());
    for _ in 0..4 {
        out.extend_from_slice(&8u16.to_le_bytes());
    }
    out.extend_from_slice(rgba);
    out
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                s.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// An SVG document with a 100 × 100 viewBox: a gradient-filled rectangle and an embedded PNG.
pub fn svg_with_png() -> Vec<u8> {
    let p = png(24, 24, &picture(24, 24, [255, 160, 0]));
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" viewBox=\"0 0 100 100\" width=\"100\" height=\"100\">\n\
         <defs><linearGradient id=\"g\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"1\"><stop offset=\"0\" stop-color=\"#ff0000\"/><stop offset=\"1\" stop-color=\"#0000ff\"/></linearGradient></defs>\n\
         <rect x=\"10\" y=\"10\" width=\"80\" height=\"80\" fill=\"url(#g)\"/>\n\
         <image x=\"38\" y=\"38\" width=\"24\" height=\"24\" xlink:href=\"data:image/png;base64,{}\"/>\n\
         </svg>\n",
        base64(&p)
    )
    .into_bytes()
}
