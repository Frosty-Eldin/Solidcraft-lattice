//! Iso-surface extraction by marching tetrahedra.
//!
//! Every grid cube is split into the same 6 tetrahedra around its main diagonal, so the split
//! of each shared face matches its neighbour's and the surface has no cracks. Vertices are
//! shared through the grid edge they sit on, so the output is an indexed, closed mesh around
//! every region where the field is negative (the field is forced positive on the grid's outer
//! layer, so nothing is left open at the border).

use std::collections::HashMap;

use crate::grid::Grid;

/// The cube's corner offsets, bit 0 = x, bit 1 = y, bit 2 = z.
const CORNER: [[usize; 3]; 8] = [[0, 0, 0], [1, 0, 0], [0, 1, 0], [1, 1, 0], [0, 0, 1], [1, 0, 1], [0, 1, 1], [1, 1, 1]];
/// 6 tetrahedra sharing the diagonal 0-7 (Kuhn's split), each with positive orientation.
const TETS: [[usize; 4]; 6] = [[0, 1, 3, 7], [0, 3, 2, 7], [0, 2, 6, 7], [0, 6, 4, 7], [0, 4, 5, 7], [0, 5, 1, 7]];

/// One slab's output: vertices keyed by grid edge, and triangles by edge key.
type Slab = (Vec<(u64, [f32; 3])>, Vec<[u64; 3]>);

pub struct Mesh {
    pub verts: Vec<[f32; 3]>,
    pub tris: Vec<[u32; 3]>,
}

fn orient(t: &[[usize; 3]; 4]) -> f64 {
    let s = |a: [usize; 3], b: [usize; 3]| [b[0] as f64 - a[0] as f64, b[1] as f64 - a[1] as f64, b[2] as f64 - a[2] as f64];
    let (u, v, w) = (s(t[0], t[1]), s(t[0], t[2]), s(t[0], t[3]));
    u[0] * (v[1] * w[2] - v[2] * w[1]) - u[1] * (v[0] * w[2] - v[2] * w[0]) + u[2] * (v[0] * w[1] - v[1] * w[0])
}

/// Extract the closed surface where `field` crosses 0; material is where it is negative.
/// Triangles face outwards (out of the material).
pub fn extract(g: &Grid, field: &[f32]) -> Mesh {
    let [nx, ny, nz] = g.n;
    // A value of exactly 0 would put a vertex on a grid point, shared by several edges: nudge it.
    let val = |i: usize, j: usize, k: usize| -> f32 {
        if i == 0 || j == 0 || k == 0 || i == nx - 1 || j == ny - 1 || k == nz - 1 {
            return 1.0;
        }
        let v = field[g.idx(i, j, k)];
        if v == 0.0 { 1e-7 } else { v }
    };
    let tets: Vec<[usize; 4]> = TETS
        .iter()
        .map(|t| {
            // Make every tetrahedron positively oriented, so triangle winding is consistent.
            let c = t.map(|c| CORNER[c]);
            if orient(&c) > 0.0 { *t } else { [t[0], t[2], t[1], t[3]] }
        })
        .collect();

    // Each slab of cubes (one z layer) is meshed on its own, its vertices keyed by grid edge.
    // Edge key: the two grid point indices, low first.
    let slabs: Vec<Slab> = crate::par::map(nz - 1, |k| {
        let mut verts: Vec<(u64, [f32; 3])> = Vec::new();
        let mut seen: HashMap<u64, ()> = HashMap::new();
        let mut tris: Vec<[u64; 3]> = Vec::new();
        for j in 0..ny - 1 {
            for i in 0..nx - 1 {
                let pos: [[usize; 3]; 8] = CORNER.map(|c| [i + c[0], j + c[1], k + c[2]]);
                let v: [f32; 8] = pos.map(|p| val(p[0], p[1], p[2]));
                if v.iter().all(|x| *x > 0.0) || v.iter().all(|x| *x < 0.0) {
                    continue;
                }
                let id: [u64; 8] = pos.map(|p| g.idx(p[0], p[1], p[2]) as u64);
                let mut edge = |a: usize, b: usize| -> u64 {
                    let (ia, ib) = (id[a], id[b]);
                    let key = if ia < ib { ia * (g.len() as u64) + ib } else { ib * (g.len() as u64) + ia };
                    if seen.insert(key, ()).is_none() {
                        let t = v[a] / (v[a] - v[b]);
                        let pa = g.point(pos[a][0], pos[a][1], pos[a][2]);
                        let pb = g.point(pos[b][0], pos[b][1], pos[b][2]);
                        let t = t as f64;
                        verts.push((key, [0, 1, 2].map(|c| (pa[c] + (pb[c] - pa[c]) * t) as f32)));
                    }
                    key
                };
                for t in &tets {
                    let inside: Vec<usize> = t.iter().copied().filter(|&c| v[c] < 0.0).collect();
                    let outside: Vec<usize> = t.iter().copied().filter(|&c| v[c] >= 0.0).collect();
                    match inside.len() {
                        0 | 4 => {}
                        1 | 3 => {
                            // One corner on its own: one triangle around it.
                            let (lone, others) = if inside.len() == 1 { (inside[0], &outside) } else { (outside[0], &inside) };
                            let mut tri = [edge(lone, others[0]), edge(lone, others[1]), edge(lone, others[2])];
                            if wants_flip(t, lone, others, inside.len() == 1) {
                                tri.swap(1, 2);
                            }
                            tris.push(tri);
                        }
                        _ => {
                            // Two and two: a quad.
                            let (a, b, c, d) = (inside[0], inside[1], outside[0], outside[1]);
                            let q = [edge(a, c), edge(a, d), edge(b, d), edge(b, c)];
                            let mut t1 = [q[0], q[1], q[2]];
                            let mut t2 = [q[0], q[2], q[3]];
                            if quad_flip(t, a, b, c, d) {
                                t1.swap(1, 2);
                                t2.swap(1, 2);
                            }
                            tris.push(t1);
                            tris.push(t2);
                        }
                    }
                }
            }
        }
        (verts, tris)
    });

    // Merge: global vertex numbers by edge key.
    let mut index: HashMap<u64, u32> = HashMap::new();
    let mut verts = Vec::new();
    for (vs, _) in &slabs {
        for (key, p) in vs {
            index.entry(*key).or_insert_with(|| {
                verts.push(*p);
                (verts.len() - 1) as u32
            });
        }
    }
    let tris = slabs.iter().flat_map(|(_, ts)| ts.iter().map(|t| t.map(|k| index[&k]))).collect();
    Mesh { verts, tris }
}

/// Orientation of the triangle cut around `lone`: with tetrahedron (p0, p1, p2, p3) positively
/// oriented, the triangle on edges (lone,o0), (lone,o1), (lone,o2) faces away from `lone` when
/// (lone, o0, o1, o2) is an even permutation of the tetrahedron's order. Faces must point out of
/// the material: away from `lone` when it is the material corner.
fn wants_flip(t: &[usize; 4], lone: usize, others: &[usize], lone_inside: bool) -> bool {
    let order = [lone, others[0], others[1], others[2]].map(|c| t.iter().position(|x| *x == c).unwrap_or(0));
    let even = parity(order);
    // Even permutation: (o0, o1, o2) seen from `lone` turn clockwise, so the normal points away
    // from `lone`.
    let away = even;
    // Inside corner alone: normal should point away from it (outwards). Outside corner alone:
    // normal should point towards it.
    away != lone_inside
}

/// Orientation of the quad between material corners (a, b) and empty corners (c, d).
fn quad_flip(t: &[usize; 4], a: usize, b: usize, c: usize, d: usize) -> bool {
    let order = [a, b, c, d].map(|x| t.iter().position(|y| *y == x).unwrap_or(0));
    !parity(order)
}

/// True for an even permutation of 0..4.
fn parity(p: [usize; 4]) -> bool {
    let mut inv = 0;
    for i in 0..4 {
        for j in i + 1..4 {
            if p[i] > p[j] {
                inv += 1;
            }
        }
    }
    inv % 2 == 0
}

impl Mesh {
    /// Signed volume (positive when triangles face outwards).
    pub fn volume(&self) -> f64 {
        self.tris
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| self.verts[i as usize].map(f64::from));
                (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0])) / 6.0
            })
            .sum()
    }
    /// Every edge used by exactly two triangles, once in each direction: a closed, consistently
    /// oriented surface. Returns the number of edges that break that.
    pub fn open_edges(&self) -> usize {
        let mut count: HashMap<(u32, u32), i32> = HashMap::new();
        for t in &self.tris {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                *count.entry((a, b)).or_insert(0) += 1;
            }
        }
        count.iter().filter(|((a, b), n)| **n != 1 || count.get(&(*b, *a)) != Some(&1)).count()
    }
}
