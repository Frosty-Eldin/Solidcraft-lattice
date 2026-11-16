//! Manifold dual contouring ("dual marching cubes"): one vertex per connected piece of the
//! surface inside each grid cube, and one quad around every grid edge the surface crosses.
//!
//! Compared with marching tetrahedra it makes 4-5x fewer, better-shaped triangles straight from
//! the grid, which matters inside a SolidCraft plugin (single-threaded, 30 s). It stays closed
//! and 2-manifold: a cube's crossed edges are grouped into pieces by its faces, and a face whose
//! four corners alternate in sign is split the same way by both cubes that share it (asymptotic
//! decider on the face's own four values), so neighbours always agree.

use crate::grid::Grid;
use crate::mesh::Mesh;

/// Corner `c` of a cube: bit 0 = x, bit 1 = y, bit 2 = z.
fn corner(c: usize) -> [usize; 3] {
    [c & 1, (c >> 1) & 1, (c >> 2) & 1]
}

/// The other two axes of `a`, right-handed: (u, v) with u x v = a.
const UV: [(usize, usize); 3] = [(1, 2), (2, 0), (0, 1)];

/// Local edge id: axis * 4 + (bit of the lower other axis) + 2 * (bit of the higher one).
fn edge_id(a: usize, c: [usize; 3]) -> usize {
    let (lo, hi) = match a {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    a * 4 + c[lo] + 2 * c[hi]
}

/// The edge between two corners that differ in one bit.
fn edge_between(p: usize, q: usize) -> usize {
    let d = p ^ q;
    let a = d.trailing_zeros() as usize;
    edge_id(a, corner(p.min(q)))
}

/// Its two corners.
fn edge_corners(e: usize) -> (usize, usize) {
    let a = e / 4;
    let (lo, hi) = match a {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    let r = e % 4;
    let base = ((r & 1) << lo) | (((r >> 1) & 1) << hi);
    (base, base | (1 << a))
}

struct Uf([u8; 12]);
impl Uf {
    fn find(&mut self, x: usize) -> usize {
        let mut x = x;
        while self.0[x] as usize != x {
            self.0[x] = self.0[self.0[x] as usize];
            x = self.0[x] as usize;
        }
        x
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.0[ra] = rb as u8;
        }
    }
}

/// Extract the closed surface where `field` crosses 0 (material negative), facing outwards.
///
/// A cube can join two pieces that one of its alternating faces split apart (a "tunnel"
/// through the cube): its single vertex would then be shared by too many quads. Such faces get
/// their split flipped, for both cubes that share them, and the cube is meshed again.
pub fn extract(g: &Grid, field: &[f32]) -> Mesh {
    let mut flipped = std::collections::HashSet::new();
    for _ in 0..8 {
        let (mesh, tunnels) = extract_with(g, field, &flipped);
        // A cube that joins both halves of a split face is harmless on its own; only a face
        // joined that way from both sides would share one mesh edge between four quads.
        let mut seen = std::collections::HashSet::new();
        let mut both = Vec::new();
        for f in tunnels {
            if !seen.insert(f) {
                both.push(f);
            }
        }
        if both.is_empty() {
            return mesh;
        }
        for f in both {
            if !flipped.remove(&f) {
                flipped.insert(f);
            }
        }
    }
    extract_with(g, field, &flipped).0
}

/// A face's id: its axis and its lowest corner's grid index.
fn face_key(g: &Grid, a: usize, c: [usize; 3]) -> u64 {
    a as u64 * g.len() as u64 + g.idx(c[0], c[1], c[2]) as u64
}

fn extract_with(g: &Grid, field: &[f32], flipped: &std::collections::HashSet<u64>) -> (Mesh, Vec<u64>) {
    let mut tunnels = Vec::new();
    let [nx, ny, nz] = g.n;
    // The outer layer counts as empty, so every crossed edge has all four cubes around it.
    let val = |i: usize, j: usize, k: usize| -> f32 {
        if i == 0 || j == 0 || k == 0 || i + 1 >= nx || j + 1 >= ny || k + 1 >= nz {
            return 1.0;
        }
        let v = field[g.idx(i, j, k)];
        if v == 0.0 { 1e-7 } else { v }
    };
    let (cx, cy, cz) = (nx - 1, ny - 1, nz - 1);
    let cube_ix = |i: usize, j: usize, k: usize| i + cx * (j + cy * k);
    // Per cube: the vertex of each local edge's piece, for cubes the surface passes through.
    let mut slot: Vec<u32> = vec![u32::MAX; cx * cy * cz];
    let mut edge_vertex: Vec<[u32; 12]> = Vec::new();
    let mut verts: Vec<[f32; 3]> = Vec::new();

    // Faces as corner cycles: axis a, side s; corners in the order (0,0),(1,0),(1,1),(0,1) of (u,v).
    let faces: Vec<(usize, usize, [usize; 4])> = (0..3)
        .flat_map(|a| {
            let (u, v) = UV[a];
            (0..2).map(move |s| {
                let c = |bu: usize, bv: usize| (s << a) | (bu << u) | (bv << v);
                (a, s, [c(0, 0), c(1, 0), c(1, 1), c(0, 1)])
            })
        })
        .collect();

    for k in 0..cz {
        for j in 0..cy {
            for i in 0..cx {
                let vals: [f32; 8] = std::array::from_fn(|c| {
                    let o = corner(c);
                    val(i + o[0], j + o[1], k + o[2])
                });
                let inside = vals.map(|v| v < 0.0);
                if inside.iter().all(|x| *x) || inside.iter().all(|x| !*x) {
                    continue;
                }
                let crossed: [bool; 12] = std::array::from_fn(|e| {
                    let (p, q) = edge_corners(e);
                    inside[p] != inside[q]
                });
                let mut uf = Uf(std::array::from_fn(|e| e as u8));
                // Split faces: their key and the two edge pairs they made.
                let mut splits: Vec<(u64, [usize; 2])> = Vec::new();
                for (fa, side, f) in &faces {
                    let f = *f;
                    let es: [usize; 4] = std::array::from_fn(|t| edge_between(f[t], f[(t + 1) % 4]));
                    let n = es.iter().filter(|e| crossed[**e]).count();
                    if n == 2 {
                        let x: Vec<usize> = es.iter().copied().filter(|e| crossed[*e]).collect();
                        uf.union(x[0], x[1]);
                    } else if n == 4 {
                        // Alternating face: asymptotic decider on its bilinear saddle.
                        let [a, b, c, d] = f.map(|c| vals[c] as f64);
                        let den = a + c - b - d;
                        let saddle = if den.abs() < 1e-30 { 0.0 } else { (a * c - b * d) / den };
                        // Corners 0 and 2 share a sign. If that sign matches the saddle's, they
                        // connect across the face, so corners 1 and 3 are cut off.
                        let mut lo = [i, j, k];
                        lo[*fa] += side;
                        let key = face_key(g, *fa, lo);
                        let joined02 = ((saddle < 0.0) == inside[f[0]]) != flipped.contains(&key);
                        splits.push((key, if joined02 { [es[0], es[2]] } else { [es[3], es[1]] }));
                        if joined02 {
                            uf.union(es[0], es[1]); // around corner 1
                            uf.union(es[2], es[3]); // around corner 3
                        } else {
                            uf.union(es[3], es[0]); // around corner 0
                            uf.union(es[1], es[2]); // around corner 2
                        }
                    }
                }
                // A tunnel: a face split two pieces apart that the cube joins anyway.
                for (key, [x, y]) in &splits {
                    if uf.find(*x) == uf.find(*y) {
                        tunnels.push(*key);
                    }
                }
                // One vertex per piece: the mean of its edge crossings.
                let mut ev = [u32::MAX; 12];
                let mut sums: [([f64; 3], u32); 12] = [([0.0; 3], 0); 12];
                for e in 0..12 {
                    if !crossed[e] {
                        continue;
                    }
                    let (p, q) = edge_corners(e);
                    let t = (vals[p] / (vals[p] - vals[q])) as f64;
                    let (op, oq) = (corner(p), corner(q));
                    let r = uf.find(e);
                    for c in 0..3 {
                        sums[r].0[c] += op[c] as f64 + (oq[c] as f64 - op[c] as f64) * t;
                    }
                    sums[r].1 += 1;
                }
                for e in 0..12 {
                    if !crossed[e] {
                        continue;
                    }
                    let r = uf.find(e);
                    if ev[r] == u32::MAX {
                        let (s, n) = sums[r];
                        let local = s.map(|x| x / n as f64);
                        verts.push([0, 1, 2].map(|c| (g.lo[c] + ([i, j, k][c] as f64 + local[c]) * g.h) as f32));
                        ev[r] = (verts.len() - 1) as u32;
                    }
                    ev[e] = ev[r];
                }
                slot[cube_ix(i, j, k)] = edge_vertex.len() as u32;
                edge_vertex.push(ev);
            }
        }
    }

    // A quad around every crossed grid edge, from the four cubes that share it.
    let mut tris = Vec::new();
    for k in 0..nz {
        for j in 0..ny {
            for i in 0..nx {
                let p = [i, j, k];
                let v0 = val(i, j, k);
                for a in 0..3 {
                    let mut q = p;
                    q[a] += 1;
                    if q[a] >= g.n[a] {
                        continue;
                    }
                    let v1 = val(q[0], q[1], q[2]);
                    if (v0 < 0.0) == (v1 < 0.0) {
                        continue;
                    }
                    let (u, v) = UV[a];
                    if p[u] == 0 || p[v] == 0 {
                        continue;
                    }
                    // Cubes around the edge, counter-clockwise seen from +a: (u,v) offsets.
                    let mut quad = [0u32; 4];
                    let mut ok = true;
                    for (t, (du, dv)) in [(1, 1), (0, 1), (0, 0), (1, 0)].into_iter().enumerate() {
                        let mut c = p;
                        c[u] -= du;
                        c[v] -= dv;
                        let s = slot[cube_ix(c[0], c[1], c[2])];
                        if s == u32::MAX {
                            ok = false;
                            break;
                        }
                        // The grid edge inside this cube: corner bits along u, v.
                        let mut bits = [0usize; 3];
                        bits[u] = du;
                        bits[v] = dv;
                        quad[t] = edge_vertex[s as usize][edge_id(a, bits)];
                    }
                    if !ok {
                        continue;
                    }
                    // Normal along +a when the edge leaves the material.
                    if v0 >= 0.0 {
                        quad.reverse();
                    }
                    tris.push([quad[0], quad[1], quad[2]]);
                    tris.push([quad[0], quad[2], quad[3]]);
                }
            }
        }
    }
    (Mesh { verts, tris }, tunnels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_tables_agree() {
        for e in 0..12 {
            let (p, q) = edge_corners(e);
            assert_eq!(edge_between(p, q), e);
            assert_eq!((p ^ q).count_ones(), 1);
        }
    }
}
