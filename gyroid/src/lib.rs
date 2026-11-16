//! TPMS lattice generator: fills a body (or a box) with a gyroid or another triply periodic
//! minimal surface, sheet or solid, with optional variable density along an axis, and returns a
//! closed, outward-facing triangle mesh ready for SolidCraft's Insert Mesh.

pub mod body;
pub mod dual;
pub mod field;
pub mod grid;
pub mod mesh;
pub mod par;
pub mod simplify;
pub mod stl;

use field::{Fill, Lattice, Profile, Tpms};
use grid::Grid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    BodyCenter,
    BodyCorner,
    World,
}

/// Everything one run needs.
#[derive(Clone, Debug)]
pub struct Job {
    pub kind: Tpms,
    pub fill: Fill,
    pub cell: f64,
    pub wall: f64,
    /// Sample spacing; `None` picks one from the wall and cell size.
    pub resolution: Option<f64>,
    pub origin: Origin,
    pub world_origin: [f64; 3],
    /// Keep a solid skin this thick (mm) on the body's surface; 0 = open lattice.
    pub shell: f64,
    /// Variable density: axis and (position from the axis start, cell size, wall) points.
    pub axis: usize,
    pub control_points: Vec<(f64, f64, f64)>,
    /// Radial gyroid: cells around; `None` = about one cell size at the body's half radius.
    pub around: Option<f64>,
    /// Refuse grids bigger than this many samples.
    pub max_samples: usize,
    /// Simplify the mesh while it stays within this RMS distance (mm) of the exact surface.
    /// `None`: 20 % of the sample spacing. 0: no simplification.
    pub tolerance: Option<f64>,
    /// Struts or honeycomb instead of the TPMS `kind` (`wall` is then the strut diameter or
    /// the honeycomb wall).
    pub shape: Option<field::Shape>,
}

/// The region to fill.
pub enum Region {
    Body(stl::Triangles),
    Box([f64; 3], [f64; 3]),
}

pub struct Output {
    pub mesh: mesh::Mesh,
    pub grid: Grid,
    pub region_lo: [f64; 3],
    pub region_hi: [f64; 3],
}

impl Job {
    /// The sample spacing: fine enough for 2.5 samples across the thinnest wall and 12 per
    /// smallest cell.
    pub fn spacing(&self) -> f64 {
        if let Some(r) = self.resolution {
            return r;
        }
        let (cmin, wmin) = if self.control_points.is_empty() {
            (self.cell, self.wall)
        } else {
            (
                self.control_points.iter().map(|c| c.1).fold(f64::MAX, f64::min),
                self.control_points.iter().map(|c| c.2).fold(f64::MAX, f64::min),
            )
        };
        let w = if self.fill == Fill::Sheet || self.shape.is_some() { wmin / 2.5 } else { f64::MAX };
        w.min(cmin / 12.0)
    }

    pub fn check(&self) -> Result<(), String> {
        let pos = |v: f64, what: &str| if v > 0.0 && v.is_finite() { Ok(()) } else { Err(format!("{what} must be above 0")) };
        pos(self.cell, "cell size")?;
        pos(self.wall, "wall thickness")?;
        if let Some(r) = self.resolution {
            pos(r, "resolution")?;
        }
        if self.shell < 0.0 {
            return Err("shell must be 0 or more".into());
        }
        if !self.control_points.is_empty() {
            if !(2..=10).contains(&self.control_points.len()) {
                return Err("variable density needs 2 to 10 control points".into());
            }
            if self.control_points.windows(2).any(|w| w[1].0 <= w[0].0) {
                return Err("control point positions must increase".into());
            }
            for c in &self.control_points {
                pos(c.1, "control point cell size")?;
                pos(c.2, "control point wall thickness")?;
            }
        }
        Ok(())
    }
}

/// The lattice's material field (mm, negative in the material) sampled on its grid.
pub struct Sampled {
    pub grid: Grid,
    pub field: Vec<f32>,
    pub region_lo: [f64; 3],
    pub region_hi: [f64; 3],
}

/// Sample the lattice cut to `region` (plus the skin, if any).
pub fn sample(job: &Job, region: &Region, mut progress: impl FnMut(&str)) -> Result<Sampled, String> {
    job.check()?;
    let (lo, hi) = match region {
        Region::Body(t) => body::bounds(t),
        Region::Box(lo, hi) => (*lo, *hi),
    };
    let h = job.spacing();
    let g = Grid::covering(lo, hi, h, 2);
    if g.len() > job.max_samples {
        return Err(format!(
            "{} x {} x {} = {:.1} million samples at {h:.3} mm, more than the limit of {:.1} million; use a coarser resolution",
            g.n[0],
            g.n[1],
            g.n[2],
            g.len() as f64 / 1e6,
            job.max_samples as f64 / 1e6
        ));
    }
    let a = job.axis;
    let length = hi[a] - lo[a];
    let (cell, wall) = if job.control_points.is_empty() {
        (Profile::constant(job.cell), Profile::constant(job.wall))
    } else {
        (Profile(job.control_points.iter().map(|c| (c.0, c.1)).collect()), Profile(job.control_points.iter().map(|c| (c.0, c.2)).collect()))
    };
    let centre = [0, 1, 2].map(|i| (lo[i] + hi[i]) / 2.0);
    let origin = match job.origin {
        Origin::BodyCenter => centre,
        Origin::BodyCorner => lo,
        Origin::World => job.world_origin,
    };
    let (u, v) = match a {
        0 => (1, 2),
        1 => (2, 0),
        _ => (0, 1),
    };
    let around = job.around.unwrap_or_else(|| {
        let r = ((hi[u] - lo[u]).max(hi[v] - lo[v])) / 4.0;
        (std::f64::consts::TAU * r / cell.at(length / 2.0)).round().max(1.0)
    });
    let lattice = Lattice { kind: job.kind, fill: job.fill, cell, wall, axis: a, axis_start: lo[a], origin, around, shape: job.shape.clone() };

    progress(&format!("grid {} x {} x {} at {h:.3} mm", g.n[0], g.n[1], g.n[2]));
    let mut field = lattice.sample(&g);
    progress("lattice field done");
    let shell = job.shell;
    match region {
        Region::Body(tris) => {
            let band = 3.0 * h + shell;
            let b = body::sdf(tris, &g, band);
            progress("body distance done");
            field.iter_mut().zip(&b).for_each(|(f, b)| *f = combine(*f, *b, shell));
        }
        Region::Box(lo, hi) => {
            for k in 0..g.n[2] {
                for j in 0..g.n[1] {
                    for i in 0..g.n[0] {
                        let b = body::box_sdf(*lo, *hi, g.point(i, j, k)) as f32;
                        let ix = g.idx(i, j, k);
                        field[ix] = combine(field[ix], b, shell);
                    }
                }
            }
        }
    }
    Ok(Sampled { grid: g, field, region_lo: lo, region_hi: hi })
}

impl Sampled {
    /// The default simplification tolerance for this grid.
    pub fn default_tolerance(&self) -> f64 {
        0.2 * self.grid.h
    }

    /// The closed part of the lattice between sample layers `s0` and `s1` (inclusive) along
    /// `axis`, capped flat about half a sample beyond each. Give neighbouring slabs one shared
    /// layer (`..=c` and `c..`): each then reaches at least to that layer, so together they
    /// cover the whole lattice, overlapping by under a sample, never leaving a gap.
    pub fn slab(&self, axis: usize, s0: usize, s1: usize) -> mesh::Mesh {
        let g = &self.grid;
        let n = g.n[axis];
        let (a0, a1) = (s0.saturating_sub(1), (s1 + 1).min(n - 1));
        let mut sub = g.clone();
        sub.n[axis] = a1 - a0 + 1;
        sub.lo[axis] = g.coord(axis, a0);
        let mut f = vec![0f32; sub.len()];
        for k in 0..sub.n[2] {
            for j in 0..sub.n[1] {
                for i in 0..sub.n[0] {
                    let mut src = [i, j, k];
                    src[axis] += a0;
                    let s = src[axis] as f64;
                    let cap = ((s0 as f64 - 0.5 - s).max(s - s1 as f64 - 0.5) * g.h) as f32;
                    let v = g_val(g, &self.field, src);
                    f[sub.idx(i, j, k)] = if s0 == 0 && src[axis] < s0 || s1 == n - 1 && src[axis] > s1 { v } else { v.max(cap) };
                }
            }
        }
        dual::extract(&sub, &f)
    }
}

fn g_val(g: &Grid, f: &[f32], p: [usize; 3]) -> f32 {
    f[g.idx(p[0], p[1], p[2])]
}

/// Fill `region` with the lattice and mesh it as one closed mesh.
pub fn generate(job: &Job, region: &Region, mut progress: impl FnMut(&str)) -> Result<Output, String> {
    let sampled = sample(job, region, &mut progress)?;
    let mut mesh = dual::extract(&sampled.grid, &sampled.field);
    progress(&format!("mesh done: {} triangles", mesh.tris.len()));
    let tol = job.tolerance.unwrap_or(sampled.default_tolerance());
    if tol > 0.0 {
        simplify::simplify(&mut mesh, tol);
        progress(&format!("simplified to {} triangles (within {tol:.3} mm)", mesh.tris.len()));
    }
    Ok(Output { mesh, grid: sampled.grid, region_lo: sampled.region_lo, region_hi: sampled.region_hi })
}

/// Lattice cut to the body, plus a skin `shell` thick inside the body's surface.
#[inline]
fn combine(lattice: f32, body: f32, shell: f64) -> f32 {
    let inside = lattice.max(body);
    if shell > 0.0 { inside.min(body.max(-body - shell as f32)) } else { inside }
}

pub fn parse_list(s: &str) -> Result<Vec<f64>, String> {
    s.split([',', ';', ' ']).filter(|t| !t.is_empty()).map(|t| t.parse::<f64>().map_err(|_| format!("`{t}` is not a number"))).collect()
}

/// Control points from comma lists, as in the Fusion add-in: cell sizes and walls (one per
/// point; a missing list repeats `cell` / `wall`), positions evenly spread over `length`
/// unless given.
pub fn control_points(
    cells: &[f64],
    walls: &[f64],
    positions: &[f64],
    cell: f64,
    wall: f64,
    length: f64,
) -> Result<Vec<(f64, f64, f64)>, String> {
    let n = cells.len().max(walls.len()).max(positions.len());
    if n == 0 {
        return Ok(Vec::new());
    }
    if !(2..=10).contains(&n) {
        return Err(format!("{n} control points; give 2 to 10"));
    }
    let fill = |v: &[f64], d: f64, what: &str| match v.len() {
        0 => Ok(vec![d; n]),
        l if l == n => Ok(v.to_vec()),
        l => Err(format!("{what} has {l} values but there are {n} control points")),
    };
    let cells = fill(cells, cell, "--cell-sizes")?;
    let walls = fill(walls, wall, "--walls")?;
    let positions = if positions.is_empty() {
        (0..n).map(|i| length * i as f64 / (n - 1) as f64).collect()
    } else {
        fill(positions, 0.0, "--positions")?
    };
    Ok((0..n).map(|i| (positions[i], cells[i], walls[i])).collect())
}
