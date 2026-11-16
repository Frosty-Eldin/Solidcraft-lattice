//! Signed distance (mm, negative inside) from a closed triangle mesh, sampled on a grid.
//!
//! Inside/outside comes from ray parity along Z through each grid column (triangles are binned
//! by their XY footprint, so each column only meets the triangles above it). Near the surface
//! the exact distance to the nearest triangle is used, so the trimmed lattice follows the body
//! smoothly; farther away the distance is clamped, which the iso-surface never needs.

use crate::grid::Grid;
use crate::stl::{Triangles, V3};

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn add_s(a: V3, b: V3, s: f64) -> V3 {
    [a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s]
}

/// Squared distance from `p` to triangle `t` (Ericson, Real-Time Collision Detection 5.1.5).
pub fn dist2_tri(p: V3, t: &[V3; 3]) -> f64 {
    let (a, b, c) = (t[0], t[1], t[2]);
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    let q = if d1 <= 0.0 && d2 <= 0.0 {
        a
    } else {
        let bp = sub(p, b);
        let (d3, d4) = (dot(ab, bp), dot(ac, bp));
        if d3 >= 0.0 && d4 <= d3 {
            b
        } else {
            let vc = d1 * d4 - d3 * d2;
            if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
                add_s(a, ab, d1 / (d1 - d3))
            } else {
                let cp = sub(p, c);
                let (d5, d6) = (dot(ab, cp), dot(ac, cp));
                if d6 >= 0.0 && d5 <= d6 {
                    c
                } else {
                    let vb = d5 * d2 - d1 * d6;
                    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
                        add_s(a, ac, d2 / (d2 - d6))
                    } else {
                        let va = d3 * d6 - d5 * d4;
                        if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
                            add_s(b, sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6)))
                        } else {
                            let denom = 1.0 / (va + vb + vc);
                            let (v, w) = (vb * denom, vc * denom);
                            add_s(add_s(a, ab, v), ac, w)
                        }
                    }
                }
            }
        }
    };
    let d = sub(p, q);
    dot(d, d)
}

pub fn bounds(tris: &Triangles) -> ([f64; 3], [f64; 3]) {
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for t in tris {
        for v in t {
            for a in 0..3 {
                lo[a] = lo[a].min(v[a]);
                hi[a] = hi[a].max(v[a]);
            }
        }
    }
    (lo, hi)
}

/// Signed distance to the body at every grid point, clamped to +-`band` away from the surface.
pub fn sdf(tris: &Triangles, g: &Grid, band: f64) -> Vec<f32> {
    let [nx, ny, nz] = g.n;
    // A tiny irrational nudge keeps rays off shared edges and vertices.
    let (ox, oy) = (g.h * 1.234_567e-4, g.h * 2.718_281e-4);
    // Columns each triangle covers.
    let mut cols: Vec<Vec<u32>> = vec![Vec::new(); nx * ny];
    for (ti, t) in tris.iter().enumerate() {
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for v in t {
            for a in 0..2 {
                lo[a] = lo[a].min(v[a]);
                hi[a] = hi[a].max(v[a]);
            }
        }
        let i0 = (((lo[0] - g.lo[0] - ox) / g.h).ceil().max(0.0)) as usize;
        let i1 = (((hi[0] - g.lo[0] - ox) / g.h).floor()).min(nx as f64 - 1.0);
        let j0 = (((lo[1] - g.lo[1] - oy) / g.h).ceil().max(0.0)) as usize;
        let j1 = (((hi[1] - g.lo[1] - oy) / g.h).floor()).min(ny as f64 - 1.0);
        if i1 < 0.0 || j1 < 0.0 {
            continue;
        }
        for j in j0..=j1 as usize {
            for i in i0..=i1 as usize {
                cols[i + nx * j].push(ti as u32);
            }
        }
    }
    // Inside flags from ray parity, column by column.
    let inside: Vec<Vec<bool>> = crate::par::map(nx * ny, |c| {
        let (i, j) = (c % nx, c / nx);
        let (x, y) = (g.coord(0, i) + ox, g.coord(1, j) + oy);
        let mut zs: Vec<f64> = Vec::new();
        for &ti in &cols[c] {
            let t = &tris[ti as usize];
            // Barycentric test in XY.
            let (a, b, cc) = (t[0], t[1], t[2]);
            let d = (b[1] - cc[1]) * (a[0] - cc[0]) + (cc[0] - b[0]) * (a[1] - cc[1]);
            if d.abs() < 1e-18 {
                continue;
            }
            let l1 = ((b[1] - cc[1]) * (x - cc[0]) + (cc[0] - b[0]) * (y - cc[1])) / d;
            let l2 = ((cc[1] - a[1]) * (x - cc[0]) + (a[0] - cc[0]) * (y - cc[1])) / d;
            let l3 = 1.0 - l1 - l2;
            if l1 >= 0.0 && l2 >= 0.0 && l3 >= 0.0 {
                zs.push(l1 * a[2] + l2 * b[2] + l3 * cc[2]);
            }
        }
        zs.sort_by(f64::total_cmp);
        let mut out = vec![false; nz];
        let mut n = 0;
        for (k, slot) in out.iter_mut().enumerate() {
            let z = g.coord(2, k);
            while n < zs.len() && zs[n] < z {
                n += 1;
            }
            *slot = n % 2 == 1;
        }
        out
    });
    let is_in = |i: usize, j: usize, k: usize| inside[i + nx * j][k];

    // Triangles bucketed for the near-surface distance.
    let bs = band.max(g.h) * 2.0;
    let bn = [0, 1, 2].map(|a| ((g.n[a] as f64 * g.h) / bs).ceil() as usize + 1);
    let bucket = |p: V3| -> [usize; 3] { [0, 1, 2].map(|a| (((p[a] - g.lo[a]) / bs).floor().max(0.0) as usize).min(bn[a] - 1)) };
    let mut buckets: Vec<Vec<u32>> = vec![Vec::new(); bn[0] * bn[1] * bn[2]];
    for (ti, t) in tris.iter().enumerate() {
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for v in t {
            for a in 0..3 {
                lo[a] = lo[a].min(v[a] - band);
                hi[a] = hi[a].max(v[a] + band);
            }
        }
        let (b0, b1) = (bucket(lo), bucket(hi));
        for z in b0[2]..=b1[2] {
            for y in b0[1]..=b1[1] {
                for x in b0[0]..=b1[0] {
                    buckets[x + bn[0] * (y + bn[1] * z)].push(ti as u32);
                }
            }
        }
    }

    let band2 = band * band;
    crate::par::flat_map(nz, |k| {
        let mut out = Vec::with_capacity(nx * ny);
        for j in 0..ny {
            for i in 0..nx {
                let inn = is_in(i, j, k);
                let s = if inn { -1.0 } else { 1.0 };
                let p = g.point(i, j, k);
                let b = bucket(p);
                let mut best = band2;
                for &ti in &buckets[b[0] + bn[0] * (b[1] + bn[1] * b[2])] {
                    best = best.min(dist2_tri(p, &tris[ti as usize]));
                }
                out.push((s * best.sqrt()) as f32);
            }
        }
        out
    })
}

/// Signed distance to an axis-aligned box (exact).
pub fn box_sdf(lo: V3, hi: V3, p: V3) -> f64 {
    let c = [0, 1, 2].map(|a| (lo[a] + hi[a]) / 2.0);
    let e = [0, 1, 2].map(|a| (hi[a] - lo[a]) / 2.0);
    let q = [0, 1, 2].map(|a| (p[a] - c[a]).abs() - e[a]);
    let outside = q.iter().map(|v| v.max(0.0).powi(2)).sum::<f64>().sqrt();
    let inside = q[0].max(q[1]).max(q[2]).min(0.0);
    outside + inside
}
