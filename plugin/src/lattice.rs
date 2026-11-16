//! Lattice topology: unit cells, tiling, and the grouping of struts and nodes into
//! translation families that SolidCraft can build with one seed feature plus patterns.
//!
//! Everything here is pure integer math in quarter-cell units (a unit cell is 4 x 4 x 4),
//! so diamond's 1/4 offsets are exact and de-duplication never depends on float noise.

use std::collections::{BTreeMap, BTreeSet};

/// Quarter-cell units per cell.
pub const Q: i64 = 4;

pub type P = [i64; 3];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Simple cubic: struts on the cell edges.
    Cubic,
    /// Body-centred cubic, pure: centre to the 8 corners.
    Bcc,
    /// BCC plus the cubic edges.
    BccCubic,
    /// BCC plus the 4 vertical edges (stiff along Z, prints well upright).
    BccZ,
    /// Face-centred cubic: an X on every face.
    Fcc,
    /// FCC plus the 4 vertical edges.
    FccZ,
    /// Octet truss: FCC faces plus the octahedron joining the face centres.
    Octet,
    /// The octahedron joining the face centres.
    Octahedron,
    /// Kelvin cell: the edges of a truncated octahedron, square faces on the cell faces.
    Kelvin,
    /// Diamond cubic: tetrahedral nodes, 4 struts per node.
    Diamond,
    /// The edges of a rhombic dodecahedron.
    RhombicDodecahedron,
    /// Auxetic: square top and bottom, vertical sides bent in towards the cell centre.
    Reentrant3d,
    /// Auxetic: a dart (double arrowhead) in the XZ and YZ mid-planes, joined tip to tip in Z.
    DoubleArrowhead,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Self> {
        Kind::ALL.iter().position(|k| *k == s).map(|i| Self::LIST[i])
    }
    const LIST: [Self; 13] = [
        Self::Cubic,
        Self::Bcc,
        Self::BccCubic,
        Self::BccZ,
        Self::Fcc,
        Self::FccZ,
        Self::Octet,
        Self::Octahedron,
        Self::Kelvin,
        Self::Diamond,
        Self::RhombicDodecahedron,
        Self::Reentrant3d,
        Self::DoubleArrowhead,
    ];
    pub const ALL: [&'static str; 13] = [
        "cubic",
        "bcc",
        "bcc_cubic",
        "bccz",
        "fcc",
        "fccz",
        "octet",
        "octahedron",
        "kelvin",
        "diamond",
        "rhombic_dodecahedron",
        "reentrant_3d",
        "double_arrowhead",
    ];
}

fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: P, k: i64) -> P {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn len2(a: P) -> i64 {
    a[0] * a[0] + a[1] * a[1] + a[2] * a[2]
}

fn cube_edges() -> Vec<(P, P)> {
    let mut out = Vec::new();
    for axis in 0..3 {
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        for a in [0, Q] {
            for b in [0, Q] {
                let mut p = [0; 3];
                p[u] = a;
                p[v] = b;
                let mut q = p;
                q[axis] = Q;
                out.push((p, q));
            }
        }
    }
    out
}

fn corners() -> Vec<P> {
    let mut out = Vec::new();
    for x in [0, Q] {
        for y in [0, Q] {
            for z in [0, Q] {
                out.push([x, y, z]);
            }
        }
    }
    out
}

fn face_centres() -> Vec<P> {
    let h = Q / 2;
    vec![[h, h, 0], [h, h, Q], [h, 0, h], [h, Q, h], [0, h, h], [Q, h, h]]
}

fn bcc() -> Vec<(P, P)> {
    let c = [Q / 2; 3];
    corners().into_iter().map(|k| (c, k)).collect()
}

fn fcc() -> Vec<(P, P)> {
    let mut out = Vec::new();
    for f in face_centres() {
        let axis = (0..3).find(|&i| f[i] == 0 || f[i] == Q).unwrap_or(0);
        for k in corners() {
            if k[axis] == f[axis] {
                out.push((f, k));
            }
        }
    }
    out
}

fn octahedron() -> Vec<(P, P)> {
    let fc = face_centres();
    let mut out = Vec::new();
    for (i, a) in fc.iter().enumerate() {
        for b in fc.iter().skip(i + 1) {
            if len2(sub(*a, *b)) == 2 * (Q / 2) * (Q / 2) {
                out.push((*a, *b));
            }
        }
    }
    out
}

fn diamond() -> Vec<(P, P)> {
    // Atoms at FCC sites; each inner atom at site + (1,1,1)/4 bonds to its 4 tetrahedral neighbours.
    let inner: [(P, [P; 4]); 4] = [
        ([1, 1, 1], [[0, 0, 0], [2, 2, 0], [2, 0, 2], [0, 2, 2]]),
        ([3, 3, 1], [[2, 2, 0], [4, 4, 0], [4, 2, 2], [2, 4, 2]]),
        ([3, 1, 3], [[2, 0, 2], [4, 0, 4], [4, 2, 2], [2, 2, 4]]),
        ([1, 3, 3], [[0, 2, 2], [2, 4, 2], [2, 2, 4], [0, 4, 4]]),
    ];
    inner.iter().flat_map(|(a, ns)| ns.iter().map(move |n| (*a, *n))).collect()
}

fn vertical_edges() -> Vec<(P, P)> {
    cube_edges().into_iter().filter(|(a, b)| a[2] != b[2]).collect()
}

/// Every pair of `points` exactly `d2` apart (squared, quarter units).
fn edges_of(points: &[P], d2: i64) -> Vec<(P, P)> {
    let mut out = Vec::new();
    for (i, a) in points.iter().enumerate() {
        for b in points.iter().skip(i + 1) {
            if len2(sub(*a, *b)) == d2 {
                out.push((*a, *b));
            }
        }
    }
    out
}

fn kelvin() -> Vec<(P, P)> {
    // Truncated octahedron centred in the cell: one coordinate at the centre, one 1 off it, one
    // 2 off it (so its square faces lie on the cell faces). Edges are sqrt(2) long.
    let mut v = Vec::new();
    for (b, c) in [(1, 2), (2, 1), (0, 2), (2, 0), (0, 1), (1, 0)] {
        for s1 in [-1, 1] {
            for s2 in [-1, 1] {
                let mut p = [2; 3];
                p[b] += s1;
                p[c] += 2 * s2;
                v.push(p);
            }
        }
    }
    edges_of(&v, 2)
}

fn rhombic_dodecahedron() -> Vec<(P, P)> {
    // Cube-type vertices at the centre +-1, octahedral vertices at the face centres; each cube
    // vertex joins its 3 nearest face centres.
    let mut v: Vec<P> = Vec::new();
    for x in [1, 3] {
        for y in [1, 3] {
            for z in [1, 3] {
                v.push([x, y, z]);
            }
        }
    }
    let cube = v.clone();
    v.extend(face_centres());
    edges_of(&v, 3).into_iter().filter(|(a, b)| cube.contains(a) != cube.contains(b)).collect()
}

fn reentrant_3d() -> Vec<(P, P)> {
    let mut out: Vec<(P, P)> = cube_edges().into_iter().filter(|(a, b)| a[2] == b[2]).collect();
    // Each vertical edge bends in towards the cell centre at mid-height.
    for x in [0, Q] {
        for y in [0, Q] {
            let mid = [if x == 0 { 1 } else { 3 }, if y == 0 { 1 } else { 3 }, 2];
            out.push(([x, y, 0], mid));
            out.push((mid, [x, y, Q]));
        }
    }
    out
}

fn double_arrowhead() -> Vec<(P, P)> {
    // A dart (points (0,0), (2,3), (4,0), (2,1) in the plane) in the XZ and YZ mid-planes, with
    // a centre column joining each dart's tip to the next cell's.
    let (lo, hi) = ([2, 2, 1], [2, 2, 3]);
    let mut out = vec![([2, 2, 0], lo), (hi, [2, 2, Q])];
    for side in [[0, 2, 0], [Q, 2, 0], [2, 0, 0], [2, Q, 0]] {
        out.push((side, hi));
        out.push((side, lo));
    }
    out
}

/// The struts of one unit cell, in quarter units inside [0, 4]^3.
pub fn unit_struts(kind: Kind) -> Vec<(P, P)> {
    match kind {
        Kind::Cubic => cube_edges(),
        Kind::Bcc => bcc(),
        Kind::BccCubic => [bcc(), cube_edges()].concat(),
        Kind::BccZ => [bcc(), vertical_edges()].concat(),
        Kind::Fcc => fcc(),
        Kind::FccZ => [fcc(), vertical_edges()].concat(),
        Kind::Octet => [fcc(), octahedron()].concat(),
        Kind::Octahedron => octahedron(),
        Kind::Kelvin => kelvin(),
        Kind::Diamond => diamond(),
        Kind::RhombicDodecahedron => rhombic_dodecahedron(),
        Kind::Reentrant3d => reentrant_3d(),
        Kind::DoubleArrowhead => double_arrowhead(),
    }
}

fn ordered(a: P, b: P) -> (P, P) {
    if a <= b { (a, b) } else { (b, a) }
}

/// Every strut of an `n[0] x n[1] x n[2]` block of cells, de-duplicated (shared faces and
/// edges appear once), in quarter units.
pub fn tile(kind: Kind, n: [i64; 3]) -> BTreeSet<(P, P)> {
    let unit = unit_struts(kind);
    let mut out = BTreeSet::new();
    for i in 0..n[0] {
        for j in 0..n[1] {
            for k in 0..n[2] {
                let o = scale([i, j, k], Q);
                for (a, b) in &unit {
                    out.insert(ordered(add(*a, o), add(*b, o)));
                }
            }
        }
    }
    out
}

/// Every node (strut end) of a set of struts.
pub fn nodes(struts: &BTreeSet<(P, P)>) -> BTreeSet<P> {
    struts.iter().flat_map(|(a, b)| [*a, *b]).collect()
}

/// A set of identical items at `start + Q * (i, j, k)` for `i < counts[0]`, `j < counts[1]`,
/// `k < counts[2]`: one seed feature plus up to two pattern features.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Family {
    /// The seed's start point, quarter units.
    pub start: P,
    /// Strut vector (start to end), quarter units. `[0, 0, 0]` for a node.
    pub vector: P,
    /// Copies along X, Y, Z, each step one cell. 1 = no pattern on that axis.
    pub counts: [i64; 3],
    /// What else the members share (a thickness level). Only same-tag items share a family.
    pub tag: u32,
}

impl Family {
    pub fn instances(&self) -> i64 {
        self.counts.iter().product()
    }
    /// Every item this family stands for, as quarter-unit (start, end).
    pub fn expand(&self) -> Vec<(P, P)> {
        let mut out = Vec::new();
        for i in 0..self.counts[0] {
            for j in 0..self.counts[1] {
                for k in 0..self.counts[2] {
                    let s = add(self.start, scale([i, j, k], Q));
                    out.push((s, add(s, self.vector)));
                }
            }
        }
        out
    }
}

/// Group items `(start, vector, tag)` into translation families. Each class (same vector, same
/// position within the cell) becomes one family when its cell indices fill a box, else a few
/// boxes that cover it exactly.
fn families(items: impl IntoIterator<Item = (P, P, u32)>) -> Vec<Family> {
    let mut classes: BTreeMap<(u32, P, P), BTreeSet<P>> = BTreeMap::new();
    for (start, vector, tag) in items {
        let r = [start[0].rem_euclid(Q), start[1].rem_euclid(Q), start[2].rem_euclid(Q)];
        let cell = [(start[0] - r[0]) / Q, (start[1] - r[1]) / Q, (start[2] - r[2]) / Q];
        classes.entry((tag, r, vector)).or_default().insert(cell);
    }
    let mut out = Vec::new();
    for ((tag, r, vector), cells) in classes {
        let lo = [0, 1, 2].map(|a| cells.iter().map(|c| c[a]).min().unwrap_or(0));
        let hi = [0, 1, 2].map(|a| cells.iter().map(|c| c[a]).max().unwrap_or(0));
        let counts = [0, 1, 2].map(|a| hi[a] - lo[a] + 1);
        if counts.iter().product::<i64>() == cells.len() as i64 {
            out.push(Family { start: add(r, scale(lo, Q)), vector, counts, tag });
            continue;
        }
        // Not a box (lattice edges where a node or strut has no neighbour): split greedily
        // into boxes, each grown along X, then Y, then Z while every cell is present.
        let mut left = cells;
        while let Some(&c) = left.iter().next() {
            let mut size = [1i64; 3];
            for axis in 0..3 {
                loop {
                    let mut next = size;
                    next[axis] += 1;
                    let mut full = true;
                    'scan: for i in 0..next[0] {
                        for j in 0..next[1] {
                            for k in 0..next[2] {
                                let p = [c[0] + i, c[1] + j, c[2] + k];
                                if [i, j, k][axis] == next[axis] - 1 && !left.contains(&p) {
                                    full = false;
                                    break 'scan;
                                }
                            }
                        }
                    }
                    if !full {
                        break;
                    }
                    size = next;
                }
            }
            for i in 0..size[0] {
                for j in 0..size[1] {
                    for k in 0..size[2] {
                        left.remove(&[c[0] + i, c[1] + j, c[2] + k]);
                    }
                }
            }
            out.push(Family { start: add(r, scale(c, Q)), vector, counts: size, tag });
        }
    }
    out
}

/// Collinear struts one cell long along an axis chain end to end: build them as one long
/// cylinder per line instead of a pattern along that axis.
fn merge_axis_chains(mut f: Family) -> Family {
    for axis in 0..3 {
        let mut unit = [0; 3];
        unit[axis] = Q;
        if f.vector == unit && f.counts[axis] > 1 {
            f.vector[axis] = Q * f.counts[axis];
            f.counts[axis] = 1;
        }
    }
    f
}

/// The strut families of a tiled lattice, each strut tagged (its thickness level), collinear
/// axis chains merged.
pub fn strut_families(struts: &BTreeSet<(P, P)>, tag: impl Fn(&(P, P)) -> u32) -> Vec<Family> {
    families(struts.iter().map(|s| (s.0, sub(s.1, s.0), tag(s)))).into_iter().map(merge_axis_chains).collect()
}

/// The node families of a tiled lattice, each node tagged.
pub fn node_families(nodes: &BTreeSet<P>, tag: impl Fn(&P) -> u32) -> Vec<Family> {
    families(nodes.iter().map(|p| (*p, [0, 0, 0], tag(p))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connected(kind: Kind, n: [i64; 3]) -> bool {
        let struts = tile(kind, n);
        let nodes: Vec<P> = nodes(&struts).into_iter().collect();
        let mut seen = BTreeSet::from([nodes[0]]);
        let mut stack = vec![nodes[0]];
        while let Some(p) = stack.pop() {
            for (a, b) in &struts {
                for (x, y) in [(a, b), (b, a)] {
                    if *x == p && seen.insert(*y) {
                        stack.push(*y);
                    }
                }
            }
        }
        seen.len() == nodes.len()
    }

    #[test]
    fn unit_cells_have_the_expected_strut_counts_and_stay_in_the_cell() {
        let want = [12, 8, 20, 12, 24, 28, 36, 12, 36, 16, 24, 16, 10];
        for (name, n) in Kind::ALL.iter().zip(want) {
            let k = Kind::parse(name).unwrap();
            let u = unit_struts(k);
            assert_eq!(u.len(), n, "{name}");
            assert!(u.iter().all(|(a, b)| a != b && [a, b].iter().all(|p| p.iter().all(|c| (0..=Q).contains(c)))), "{name}");
            let set: BTreeSet<_> = u.iter().map(|(a, b)| ordered(*a, *b)).collect();
            assert_eq!(set.len(), u.len(), "{name}: duplicate strut");
        }
    }

    #[test]
    fn every_lattice_is_one_connected_piece() {
        for name in Kind::ALL {
            let k = Kind::parse(name).unwrap();
            for n in [[1, 1, 1], [2, 2, 2], [3, 1, 2]] {
                assert!(connected(k, n), "{name} {n:?}");
            }
        }
    }
}
