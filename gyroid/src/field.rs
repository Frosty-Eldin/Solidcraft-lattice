//! Triply periodic minimal surfaces (TPMS) and the lattice field built from them.
//!
//! Each surface is an implicit function f(X, Y, Z) of phase coordinates (2 pi per cell). The
//! field is turned into an approximate distance in millimetres, d = f / |grad f| (gradient taken
//! on the grid, so it already includes cell size and any variable-density warp), which makes
//! the wall thickness come out in millimetres:
//! - sheet: material where |d| <= wall / 2 (a wall of `wall` mm around the surface);
//! - solid (network): material on one side, where d <= wall / 2 - cell / 4 (wall = cell / 2
//!   gives the classic 50 % network).

use std::f64::consts::TAU;

use crate::grid::Grid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tpms {
    Gyroid,
    RadialGyroid,
    SchwarzDiamond,
    SchwarzPrimitive,
    SchoenIwp,
    Neovius,
}

impl Tpms {
    pub const NAMES: [&'static str; 6] = ["gyroid", "radial_gyroid", "schwarz_diamond", "schwarz_primitive", "schoen_iwp", "neovius"];
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "gyroid" => Self::Gyroid,
            "radial_gyroid" => Self::RadialGyroid,
            "schwarz_diamond" | "diamond" => Self::SchwarzDiamond,
            "schwarz_primitive" | "primitive" => Self::SchwarzPrimitive,
            "schoen_iwp" | "iwp" => Self::SchoenIwp,
            "neovius" => Self::Neovius,
            _ => return None,
        })
    }
    /// The implicit function of phase coordinates.
    #[inline]
    pub fn f(self, x: f64, y: f64, z: f64) -> f64 {
        let (sx, cx, sy, cy, sz, cz) = (x.sin(), x.cos(), y.sin(), y.cos(), z.sin(), z.cos());
        match self {
            Self::Gyroid | Self::RadialGyroid => sx * cy + sy * cz + sz * cx,
            Self::SchwarzDiamond => sx * sy * sz + sx * cy * cz + cx * sy * cz + cx * cy * sz,
            Self::SchwarzPrimitive => cx + cy + cz,
            Self::SchoenIwp => 2.0 * (cx * cy + cy * cz + cz * cx) - ((2.0 * x).cos() + (2.0 * y).cos() + (2.0 * z).cos()),
            Self::Neovius => 3.0 * (cx + cy + cz) + 4.0 * cx * cy * cz,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fill {
    Sheet,
    Solid,
}

/// Piecewise-linear profile through (position, value) points sorted by position.
#[derive(Clone, Debug)]
pub struct Profile(pub Vec<(f64, f64)>);
impl Profile {
    pub fn constant(v: f64) -> Self {
        Self(vec![(0.0, v)])
    }
    pub fn at(&self, s: f64) -> f64 {
        let p = &self.0;
        if p.len() == 1 || s <= p[0].0 {
            return p[0].1;
        }
        for w in p.windows(2) {
            if s <= w[1].0 {
                return w[0].1 + (w[1].1 - w[0].1) * (s - w[0].0) / (w[1].0 - w[0].0);
            }
        }
        p[p.len() - 1].1
    }
    pub fn min(&self) -> f64 {
        self.0.iter().map(|p| p.1).fold(f64::MAX, f64::min)
    }
    pub fn max(&self) -> f64 {
        self.0.iter().map(|p| p.1).fold(f64::MIN, f64::max)
    }
}

/// Lattices that are not TPMS, meshed the same way from their distance field.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// Cylindrical struts between points of the unit cell (cell units, 0..1), diameter = the
    /// wall profile; `node` > 0 adds a sphere of that diameter at every strut end.
    /// `across`: a fixed cell size across the axis (the profile then sets it only along).
    Struts { segments: Vec<([f64; 3], [f64; 3])>, node: f64, across: Option<f64> },
    /// Walls of a 2D tiling across the axis, extruded along it; cell = centre-to-centre pitch
    /// (hexagon, square) or side (triangle).
    Honeycomb(Tiling),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tiling {
    Hexagon,
    Square,
    Triangle,
}

#[inline]
fn seg_dist(p: [f64; 3], a: [f64; 3], b: [f64; 3]) -> f64 {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ap = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
    let l2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
    let t = if l2 > 0.0 { ((ap[0] * ab[0] + ap[1] * ab[1] + ap[2] * ab[2]) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let d = [ap[0] - ab[0] * t, ap[1] - ab[1] * t, ap[2] - ab[2] * t];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

#[inline]
fn pt_dist(p: [f64; 3], a: [f64; 3]) -> f64 {
    ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2) + (p[2] - a[2]).powi(2)).sqrt()
}

/// Distance (pitch units) from (x, y) to the nearest wall of the tiling.
fn tiling_dist(t: Tiling, x: f64, y: f64) -> f64 {
    let s3 = 3f64.sqrt();
    match t {
        Tiling::Square => {
            let f = |v: f64| (v - v.round()).abs();
            f(x).min(f(y))
        }
        Tiling::Triangle => {
            // Three families of lines, spacing sqrt(3)/2 (side 1).
            let h = s3 / 2.0;
            let f = |v: f64| {
                let q = v / h;
                (q - q.round()).abs() * h
            };
            f(y).min(f(s3 / 2.0 * x - 0.5 * y)).min(f(-s3 / 2.0 * x - 0.5 * y))
        }
        Tiling::Hexagon => {
            // Voronoi cells of a triangular lattice with spacing 1: distance to the bisector
            // between the nearest centre and each neighbour.
            let (b2y, b2x) = (s3 / 2.0, 0.5);
            let j = y / b2y;
            let i = x - b2x * j;
            let (i0, j0) = (i.floor() as i64, j.floor() as i64);
            let mut c: Vec<([f64; 2], f64)> = Vec::with_capacity(16);
            for dj in -1..=2 {
                for di in -1..=2 {
                    let (ii, jj) = ((i0 + di) as f64, (j0 + dj) as f64);
                    let p = [ii + b2x * jj, b2y * jj];
                    c.push((p, (x - p[0]).powi(2) + (y - p[1]).powi(2)));
                }
            }
            let k = (0..c.len()).min_by(|a, b| c[*a].1.total_cmp(&c[*b].1)).unwrap_or(0);
            let (p1, d1) = c[k];
            c.iter()
                .enumerate()
                .filter(|(n, _)| *n != k)
                .map(|(_, (p, d))| (d - d1) / (2.0 * ((p[0] - p1[0]).powi(2) + (p[1] - p1[1]).powi(2)).sqrt()))
                .fold(f64::MAX, f64::min)
        }
    }
}

#[derive(Clone, Debug)]
pub struct Lattice {
    pub kind: Tpms,
    pub fill: Fill,
    /// Cell size and wall thickness along `axis`, positions in mm from `axis_start`.
    pub cell: Profile,
    pub wall: Profile,
    pub axis: usize,
    pub axis_start: f64,
    /// Where phase 0 sits (a cell corner): body centre, body corner or the world origin.
    pub origin: [f64; 3],
    /// Radial gyroid: cells around the circumference.
    pub around: f64,
    /// Struts or honeycomb instead of the TPMS.
    pub shape: Option<Shape>,
}

impl Lattice {
    /// Cells counted from the origin along the axis: the integral of 1 / cell size, so cells
    /// follow the profile without tearing.
    fn axis_phase_table(&self, g: &Grid) -> Vec<f64> {
        let a = self.axis;
        let n = g.n[a];
        let mut out = vec![0.0; n];
        // Integrate from the origin plane with fine sub-steps.
        let u = |s: f64| -> f64 {
            let steps = 64;
            let (s0, s1) = (self.origin[a], s);
            let ds = (s1 - s0) / steps as f64;
            (0..steps).map(|i| ds / self.cell.at(s0 + (i as f64 + 0.5) * ds - self.axis_start)).sum()
        };
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = u(g.coord(a, i));
        }
        out
    }

    /// The material field (mm, negative in the material) on the grid.
    pub fn sample(&self, g: &Grid) -> Vec<f32> {
        if let Some(shape) = &self.shape {
            return self.sample_shape(shape, g);
        }
        let [nx, ny, nz] = g.n;
        let a = self.axis;
        let (u, v) = match a {
            0 => (1, 2),
            1 => (2, 0),
            _ => (0, 1),
        };
        let phase_a = self.axis_phase_table(g);
        let kind = self.kind;
        let f: Vec<f32> = crate::par::flat_map(nz, |k| {
            let mut out = Vec::with_capacity(nx * ny);
            for j in 0..ny {
                for i in 0..nx {
                    let ijk = [i, j, k];
                    let p = g.point(i, j, k);
                    let s = p[a] - self.axis_start;
                    let c = self.cell.at(s);
                    let pa = TAU * phase_a[ijk[a]];
                    let (pu, pv) = (TAU * (p[u] - self.origin[u]) / c, TAU * (p[v] - self.origin[v]) / c);
                    let val = if kind == Tpms::RadialGyroid {
                        let (du, dv) = (p[u] - self.origin[u], p[v] - self.origin[v]);
                        let r = (du * du + dv * dv).sqrt();
                        kind.f(TAU * r / c, self.around * dv.atan2(du), pa)
                    } else {
                        let mut q = [0.0; 3];
                        q[u] = pu;
                        q[v] = pv;
                        q[a] = pa;
                        kind.f(q[0], q[1], q[2])
                    };
                    out.push(val as f32);
                }
            }
            out
        });
        // d = f / |grad f| with central differences on the grid.
        let h2 = 2.0 * g.h;
        crate::par::flat_map(nz, |k| {
            let f = &f;
            let mut out = Vec::with_capacity(nx * ny);
            for j in 0..ny {
                for i in 0..nx {
                    let at = |i: usize, j: usize, k: usize| f[g.idx(i, j, k)] as f64;
                    let diff = |lo: f64, hi: f64, span: f64| (hi - lo) / span;
                    let gx =
                        diff(at(i.saturating_sub(1), j, k), at((i + 1).min(nx - 1), j, k), if i == 0 || i == nx - 1 { g.h } else { h2 });
                    let gy =
                        diff(at(i, j.saturating_sub(1), k), at(i, (j + 1).min(ny - 1), k), if j == 0 || j == ny - 1 { g.h } else { h2 });
                    let gz =
                        diff(at(i, j, k.saturating_sub(1)), at(i, j, (k + 1).min(nz - 1)), if k == 0 || k == nz - 1 { g.h } else { h2 });
                    let grad = (gx * gx + gy * gy + gz * gz).sqrt().max(1e-9);
                    let d = at(i, j, k) / grad;
                    let p = g.point(i, j, k);
                    let s = p[a] - self.axis_start;
                    let w = self.wall.at(s);
                    let m = match self.fill {
                        Fill::Sheet => d.abs() - w / 2.0,
                        Fill::Solid => d - (w / 2.0 - self.cell.at(s) / 4.0),
                    };
                    out.push(m as f32);
                }
            }
            out
        })
    }

    /// Struts and honeycombs: their distance, in cell units through the same warp as the TPMS
    /// (cells counted along the axis), scaled back to mm by the local cell size.
    fn sample_shape(&self, shape: &Shape, g: &Grid) -> Vec<f32> {
        let [nx, ny, nz] = g.n;
        let a = self.axis;
        let (u, v) = match a {
            0 => (1, 2),
            1 => (2, 0),
            _ => (0, 1),
        };
        let phase_a = self.axis_phase_table(g);
        let wmax = self.wall.max();
        crate::par::flat_map(nz, |k| {
            let mut out = Vec::with_capacity(nx * ny);
            for j in 0..ny {
                for i in 0..nx {
                    let ijk = [i, j, k];
                    let p = g.point(i, j, k);
                    let s = p[a] - self.axis_start;
                    let c = self.cell.at(s);
                    let r = self.wall.at(s) / 2.0;
                    let cx = match shape {
                        Shape::Struts { across: Some(x), .. } => *x,
                        _ => c,
                    };
                    let mut sc = [cx; 3];
                    sc[a] = c;
                    let mut q = [0.0; 3];
                    q[u] = (p[u] - self.origin[u]) / cx;
                    q[v] = (p[v] - self.origin[v]) / cx;
                    q[a] = phase_a[ijk[a]];
                    let d = match shape {
                        Shape::Honeycomb(t) => c * tiling_dist(*t, q[u], q[v]) - r,
                        Shape::Struts { segments, node, .. } => {
                            let f = q.map(|x| x - x.floor());
                            // Struts of the neighbouring cells reach in near the faces.
                            let m = (wmax.max(*node) / 2.0 + 2.0 * g.h) / c.min(cx);
                            let range = |x: f64| -> (i32, i32) { (if x < m { -1 } else { 0 }, if x > 1.0 - m { 1 } else { 0 }) };
                            let (rx, ry, rz) = (range(f[0]), range(f[1]), range(f[2]));
                            let mut best = f64::MAX;
                            let mut best_node = f64::MAX;
                            for ox in rx.0..=rx.1 {
                                for oy in ry.0..=ry.1 {
                                    for oz in rz.0..=rz.1 {
                                        // In mm, cell by cell along each axis.
                                        let pp = [(f[0] - ox as f64) * sc[0], (f[1] - oy as f64) * sc[1], (f[2] - oz as f64) * sc[2]];
                                        for (sa, sb) in segments {
                                            let (sa, sb) = ([0, 1, 2].map(|k| sa[k] * sc[k]), [0, 1, 2].map(|k| sb[k] * sc[k]));
                                            best = best.min(seg_dist(pp, sa, sb));
                                            if *node > 0.0 {
                                                best_node = best_node.min(pt_dist(pp, sa)).min(pt_dist(pp, sb));
                                            }
                                        }
                                    }
                                }
                            }
                            let ds = best - r;
                            if *node > 0.0 { ds.min(best_node - node / 2.0) } else { ds }
                        }
                    };
                    out.push(d as f32);
                }
            }
            out
        })
    }
}
