//! STL reading (binary or ASCII) and binary STL writing.

use std::io::{self, Write};

pub type V3 = [f64; 3];

/// A triangle soup.
pub type Triangles = Vec<[V3; 3]>;

pub fn read(bytes: &[u8]) -> Result<Triangles, String> {
    // Binary when the size matches the triangle count in the header; some binary files start
    // with "solid", so the size test comes first.
    if bytes.len() >= 84 {
        let n = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
        if 84 + n * 50 == bytes.len() {
            let mut out = Vec::with_capacity(n);
            for t in 0..n {
                let base = 84 + t * 50 + 12;
                let f = |i: usize| {
                    f32::from_le_bytes([bytes[base + i * 4], bytes[base + i * 4 + 1], bytes[base + i * 4 + 2], bytes[base + i * 4 + 3]])
                        as f64
                };
                out.push([[f(0), f(1), f(2)], [f(3), f(4), f(5)], [f(6), f(7), f(8)]]);
            }
            return Ok(out);
        }
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "not a binary STL (size does not match) and not ASCII text".to_string())?;
    let mut out = Vec::new();
    let mut pts: Vec<V3> = Vec::new();
    for line in text.lines() {
        let mut w = line.split_whitespace();
        if w.next() == Some("vertex") {
            let v: Vec<f64> = w.filter_map(|t| t.parse().ok()).collect();
            if v.len() != 3 {
                return Err(format!("bad vertex line: {line}"));
            }
            pts.push([v[0], v[1], v[2]]);
            if pts.len() == 3 {
                out.push([pts[0], pts[1], pts[2]]);
                pts.clear();
            }
        }
    }
    if out.is_empty() {
        return Err("no triangles found".into());
    }
    Ok(out)
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// Write an indexed mesh as binary STL.
pub fn write(w: &mut impl Write, verts: &[[f32; 3]], tris: &[[u32; 3]], header: &str) -> io::Result<()> {
    let mut h = [0u8; 80];
    for (d, s) in h.iter_mut().zip(header.bytes()) {
        *d = s;
    }
    w.write_all(&h)?;
    w.write_all(&(tris.len() as u32).to_le_bytes())?;
    let mut buf = Vec::with_capacity(50 * 4096);
    for t in tris {
        let p = t.map(|i| verts[i as usize].map(f64::from));
        let n = cross(sub(p[1], p[0]), sub(p[2], p[0]));
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-30);
        for c in n {
            buf.extend_from_slice(&((c / l) as f32).to_le_bytes());
        }
        for v in p {
            for c in v {
                buf.extend_from_slice(&(c as f32).to_le_bytes());
            }
        }
        buf.extend_from_slice(&[0, 0]);
        if buf.len() >= 50 * 4096 {
            w.write_all(&buf)?;
            buf.clear();
        }
    }
    w.write_all(&buf)
}
