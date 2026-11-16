//! Quadric-error edge-collapse simplification (Garland and Heckbert) that keeps a closed mesh
//! closed: an edge collapses only when the link condition holds (the surface stays a 2-manifold),
//! no triangle flips or turns to a sliver, and the collapsed vertex stays within `tolerance`
//! (RMS distance) of the planes of the triangles it replaces.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use crate::mesh::Mesh;

type V = [f64; 3];
/// A simplified piece: vertices, the global vertex each came from, triangles.
type Piece = (Vec<V>, Vec<u32>, Vec<[u32; 3]>);

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: V, b: V) -> V {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm(a: V) -> f64 {
    dot(a, a).sqrt()
}

/// Symmetric 4x4 quadric, upper triangle: a2 ab ac ad b2 bc bd c2 cd d2.
#[derive(Clone, Copy, Default)]
struct Q([f64; 10]);
impl Q {
    fn plane(n: V, d: f64, w: f64) -> Self {
        let [a, b, c] = n;
        Q([a * a * w, a * b * w, a * c * w, a * d * w, b * b * w, b * c * w, b * d * w, c * c * w, c * d * w, d * d * w])
    }
    fn add(&self, o: &Q) -> Q {
        let mut r = self.0;
        for (x, y) in r.iter_mut().zip(o.0) {
            *x += y;
        }
        Q(r)
    }
    fn eval(&self, p: V) -> f64 {
        let q = &self.0;
        let [x, y, z] = p;
        q[0] * x * x
            + 2.0 * q[1] * x * y
            + 2.0 * q[2] * x * z
            + 2.0 * q[3] * x
            + q[4] * y * y
            + 2.0 * q[5] * y * z
            + 2.0 * q[6] * y
            + q[7] * z * z
            + 2.0 * q[8] * z
            + q[9]
    }
    /// The point minimizing the quadric, if well conditioned.
    fn optimum(&self) -> Option<V> {
        let q = &self.0;
        let m = [[q[0], q[1], q[2]], [q[1], q[4], q[5]], [q[2], q[5], q[7]]];
        let rhs = [-q[3], -q[6], -q[8]];
        let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
        let scale = (q[0] + q[4] + q[7]).powi(3).max(1e-300);
        if det.abs() < 1e-9 * scale {
            return None;
        }
        let solve = |c: usize| {
            let mut mm = m;
            for r in 0..3 {
                mm[r][c] = rhs[r];
            }
            (mm[0][0] * (mm[1][1] * mm[2][2] - mm[1][2] * mm[2][1]) - mm[0][1] * (mm[1][0] * mm[2][2] - mm[1][2] * mm[2][0])
                + mm[0][2] * (mm[1][0] * mm[2][1] - mm[1][1] * mm[2][0]))
                / det
        };
        Some([solve(0), solve(1), solve(2)])
    }
}

#[derive(PartialEq)]
struct Cand {
    cost: f64,
    a: u32,
    b: u32,
    stamp: (u32, u32),
}
impl Eq for Cand {}
impl PartialOrd for Cand {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Cand {
    fn cmp(&self, o: &Self) -> Ordering {
        o.cost.total_cmp(&self.cost)
    }
}

/// The vertices sharing a live triangle with `v`.
fn neighbours(v: u32, out: &mut Vec<u32>, tris: &[[u32; 3]], alive: &[bool], vfaces: &[Vec<u32>]) {
    out.clear();
    for &f in &vfaces[v as usize] {
        if alive[f as usize] {
            for &w in &tris[f as usize] {
                if w != v && !out.contains(&w) {
                    out.push(w);
                }
            }
        }
    }
}

/// Simplify in place, in parallel: the mesh is cut into slabs along its longest axis, each
/// slab simplified on its own with the vertices it shares with other slabs held fixed, then a
/// second pass with the slabs shifted by half a slab cleans up the seams. Returns how many
/// triangles were removed.
pub fn simplify(mesh: &mut Mesh, tolerance: f64) -> usize {
    let before = mesh.tris.len();
    if before == 0 {
        return 0;
    }
    let blocks = (crate::par::threads() * 4).max(1);
    for pass in 0..2 {
        let verts: Vec<V> = mesh.verts.iter().map(|v| v.map(f64::from)).collect();
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for v in &verts {
            for a in 0..3 {
                lo[a] = lo[a].min(v[a]);
                hi[a] = hi[a].max(v[a]);
            }
        }
        let axis = (0..3).max_by(|a, b| (hi[*a] - lo[*a]).total_cmp(&(hi[*b] - lo[*b]))).unwrap_or(2);
        let size = (hi[axis] - lo[axis]).max(1e-9) / blocks as f64;
        let shift = if pass == 1 { size / 2.0 } else { 0.0 };
        let block_of = |t: &[u32; 3]| -> usize {
            let c = t.iter().map(|i| verts[*i as usize][axis]).sum::<f64>() / 3.0;
            (((c - lo[axis] + shift) / size).floor().max(0.0) as usize).min(blocks)
        };
        let owner: Vec<usize> = mesh.tris.iter().map(block_of).collect();
        // A vertex used by two blocks is held fixed.
        let mut first = vec![usize::MAX; verts.len()];
        let mut locked = vec![false; verts.len()];
        for (t, &b) in mesh.tris.iter().zip(&owner) {
            for &v in t {
                let f = &mut first[v as usize];
                if *f == usize::MAX {
                    *f = b;
                } else if *f != b {
                    locked[v as usize] = true;
                }
            }
        }
        let mut per_block: Vec<Vec<usize>> = vec![Vec::new(); blocks + 1];
        for (f, &b) in owner.iter().enumerate() {
            per_block[b].push(f);
        }
        let results: Vec<Piece> = crate::par::map(per_block.len(), |b| {
            let faces = &per_block[b];
            let mut local = std::collections::HashMap::new();
            let mut lv: Vec<V> = Vec::new();
            let mut global: Vec<u32> = Vec::new();
            let mut lt = Vec::with_capacity(faces.len());
            for &f in faces {
                lt.push(mesh.tris[f].map(|g| {
                    *local.entry(g).or_insert_with(|| {
                        lv.push(verts[g as usize]);
                        global.push(g);
                        (lv.len() - 1) as u32
                    })
                }));
            }
            let lock: Vec<bool> = global.iter().map(|g| locked[*g as usize]).collect();
            let (pv, src, pt) = core(lv, lt, &lock, tolerance);
            (pv, src.into_iter().map(|i| global[i]).collect(), pt)
        });
        // Merge, keeping each fixed vertex once.
        let mut shared = std::collections::HashMap::new();
        let mut verts_out: Vec<[f32; 3]> = Vec::new();
        let mut tris_out = Vec::new();
        for (pv, src, pt) in results {
            let ids: Vec<u32> = pv
                .iter()
                .zip(&src)
                .map(|(p, g)| {
                    let mut push = || {
                        verts_out.push(p.map(|c| c as f32));
                        (verts_out.len() - 1) as u32
                    };
                    if locked[*g as usize] { *shared.entry(*g).or_insert_with(push) } else { push() }
                })
                .collect();
            tris_out.extend(pt.into_iter().map(|t| t.map(|i| ids[i as usize])));
        }
        mesh.verts = verts_out;
        mesh.tris = tris_out;
    }
    before - mesh.tris.len()
}

/// Simplify one piece. `locked` vertices never move or go. Returns the surviving vertices,
/// the input index each came from, and the triangles.
fn core(mut pos: Vec<V>, mut tris: Vec<[u32; 3]>, locked: &[bool], tolerance: f64) -> (Vec<V>, Vec<usize>, Vec<[u32; 3]>) {
    let nv = pos.len();
    let mut alive = vec![true; tris.len()];
    let mut vfaces: Vec<Vec<u32>> = vec![Vec::new(); nv];
    for (f, t) in tris.iter().enumerate() {
        for &v in t {
            vfaces[v as usize].push(f as u32);
        }
    }
    let mut quad = vec![Q::default(); nv];
    for t in &tris {
        let p = t.map(|i| pos[i as usize]);
        let n = cross(sub(p[1], p[0]), sub(p[2], p[0]));
        let area2 = norm(n);
        if area2 < 1e-30 {
            continue;
        }
        let n = n.map(|c| c / area2);
        let q = Q::plane(n, -dot(n, p[0]), area2 / 2.0);
        for &v in t {
            quad[v as usize] = quad[v as usize].add(&q);
        }
    }
    // Area weights make the quadric an area-weighted sum of squared distances; compare it with
    // the area around the edge to get an RMS distance.
    let mut area = vec![0.0; nv];
    for t in &tris {
        let p = t.map(|i| pos[i as usize]);
        let a = norm(cross(sub(p[1], p[0]), sub(p[2], p[0]))) / 2.0;
        for &v in t {
            area[v as usize] += a / 3.0;
        }
    }
    let mut stamp = vec![0u32; nv];
    let tol2 = tolerance * tolerance;

    let target = |a: u32, b: u32, quad: &[Q], pos: &[V]| -> (V, f64) {
        let q = quad[a as usize].add(&quad[b as usize]);
        let (pa, pb) = (pos[a as usize], pos[b as usize]);
        let mid = [0, 1, 2].map(|i| (pa[i] + pb[i]) / 2.0);
        let mut best = (mid, q.eval(mid));
        if let Some(o) = q.optimum() {
            // Stay near the edge: an optimum far away is a sign of a nearly flat quadric.
            let len = norm(sub(pa, pb));
            if norm(sub(o, mid)) <= len {
                let e = q.eval(o);
                if e < best.1 {
                    best = (o, e);
                }
            }
        }
        for p in [pa, pb] {
            let e = q.eval(p);
            if e < best.1 {
                best = (p, e);
            }
        }
        best
    };
    let rms = |e: f64, a: u32, b: u32, area: &[f64]| e.max(0.0) / (area[a as usize] + area[b as usize]).max(1e-30);

    let mut heap = BinaryHeap::new();
    for t in &tris {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            if a < b {
                let (_, e) = target(a, b, &quad, &pos);
                let cost = rms(e, a, b, &area);
                if cost <= tol2 {
                    heap.push(Cand { cost, a, b, stamp: (0, 0) });
                }
            }
        }
    }

    let mut removed = 0;
    let mut ring_a: Vec<u32> = Vec::new();
    let mut ring_b: Vec<u32> = Vec::new();
    while let Some(c) = heap.pop() {
        let (a, b) = (c.a, c.b);
        if c.stamp != (stamp[a as usize], stamp[b as usize]) || locked[a as usize] || locked[b as usize] {
            continue;
        }
        // The two triangles on the edge.
        let shared: Vec<u32> =
            vfaces[a as usize].iter().copied().filter(|f| alive[*f as usize] && tris[*f as usize].contains(&b)).collect();
        if shared.len() != 2 {
            continue;
        }
        // Link condition: a and b share exactly the two opposite vertices.
        neighbours(a, &mut ring_a, &tris, &alive, &vfaces);
        neighbours(b, &mut ring_b, &tris, &alive, &vfaces);
        let common = ring_a.iter().filter(|w| ring_b.contains(w)).count();
        if common != 2 {
            continue;
        }
        let (p, e) = target(a, b, &quad, &pos);
        if rms(e, a, b, &area) > tol2 {
            continue;
        }
        // No flipped or degenerate triangles around a or b.
        let mut ok = true;
        for &v in &[a, b] {
            for &f in &vfaces[v as usize] {
                let f = f as usize;
                if !alive[f] || shared.contains(&(f as u32)) {
                    continue;
                }
                let t = tris[f];
                let old = t.map(|i| pos[i as usize]);
                let new = t.map(|i| if i == a || i == b { p } else { pos[i as usize] });
                let (n0, n1) = (cross(sub(old[1], old[0]), sub(old[2], old[0])), cross(sub(new[1], new[0]), sub(new[2], new[0])));
                let (l0, l1) = (norm(n0), norm(n1));
                // Flip, or a sliver: area small against its longest edge.
                let longest = (0..3).map(|k| norm(sub(new[(k + 1) % 3], new[k]))).fold(0.0, f64::max);
                if l1 < 1e-30 || dot(n0, n1) < 0.3 * l0 * l1 || l1 < 0.05 * longest * longest {
                    ok = false;
                    break;
                }
            }
            if !ok {
                break;
            }
        }
        if !ok {
            continue;
        }
        // Collapse b into a.
        for f in &shared {
            alive[*f as usize] = false;
            removed += 1;
        }
        let bf = std::mem::take(&mut vfaces[b as usize]);
        for f in bf {
            if !alive[f as usize] {
                continue;
            }
            for v in tris[f as usize].iter_mut() {
                if *v == b {
                    *v = a;
                }
            }
            vfaces[a as usize].push(f);
        }
        vfaces[a as usize].retain(|f| alive[*f as usize]);
        pos[a as usize] = p;
        quad[a as usize] = quad[a as usize].add(&quad[b as usize]);
        area[a as usize] += area[b as usize];
        stamp[a as usize] += 1;
        stamp[b as usize] += 1;
        // Requeue the edges around a.
        neighbours(a, &mut ring_a, &tris, &alive, &vfaces);
        for &w in &ring_a {
            let (x, y) = if a < w { (a, w) } else { (w, a) };
            let (_, e) = target(x, y, &quad, &pos);
            let cost = rms(e, x, y, &area);
            if cost <= tol2 {
                heap.push(Cand { cost, a: x, b: y, stamp: (stamp[x as usize], stamp[y as usize]) });
            }
        }
    }

    // Compact.
    let mut remap = vec![u32::MAX; nv];
    let mut verts = Vec::new();
    let mut src = Vec::new();
    let mut out = Vec::with_capacity(tris.len() - removed);
    for (f, t) in tris.iter().enumerate() {
        if !alive[f] {
            continue;
        }
        out.push(t.map(|v| {
            if remap[v as usize] == u32::MAX {
                remap[v as usize] = verts.len() as u32;
                verts.push(pos[v as usize]);
                src.push(v as usize);
            }
            remap[v as usize]
        }));
    }
    (verts, src, out)
}
