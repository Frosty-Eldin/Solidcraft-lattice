//! What the user asked for (`Spec`) and where it lands in the model (`Layout`): the cell grid,
//! the layer boundaries of a variable-density axis, and the wall-thickness profile.
//!
//! Variable density follows the Fusion Lattice Generator add-in: 2 to 10 control points along
//! one axis, each with a position, a cell size and a wall (strut) thickness. Along that axis the
//! cells become layers whose heights follow the cell-size profile; across it the cells keep the
//! main cell size, so every strut still meets its neighbours.

use serde_json::Value;

use crate::lattice::{Kind, P, Q};

/// A 2.5D honeycomb's cell shape (SolidCraft's Cell Grid cuts them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Honeycomb {
    Hexagon,
    Square,
    Triangle,
}
impl Honeycomb {
    pub fn shape(self) -> &'static str {
        match self {
            Self::Hexagon => "hexagon",
            Self::Square => "square",
            Self::Triangle => "triangle",
        }
    }
    /// One cell's area at pitch `c` (centre to centre).
    pub fn cell_area(self, c: f64) -> f64 {
        match self {
            Self::Hexagon => 3f64.sqrt() / 2.0 * c * c,
            Self::Square => c * c,
            Self::Triangle => 3f64.sqrt() / 4.0 * c * c,
        }
    }
}

/// Strut lattices are built from cylinders; honeycombs are cut with Cell Grid; TPMS lattices
/// (gyroid and friends) come in as a mesh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lattice {
    Struts(Kind),
    Honeycomb(Honeycomb),
    Tpms(gyroid::field::Tpms, gyroid::field::Fill),
}

pub const HONEYCOMBS: [&str; 3] = ["honeycomb_hex", "honeycomb_square", "honeycomb_triangle"];
pub const TPMS: [&str; 6] = gyroid::field::Tpms::NAMES;

pub const MAX_CELLS_PER_AXIS: i64 = 50;
pub const MAX_BANDS: usize = 16;
pub const MAX_CONTROL_POINTS: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OriginMode {
    /// The cell grid is centred on the body's bounding box.
    BodyCenter,
    /// The cell grid starts at the body's bounding-box minimum corner.
    BodyCorner,
    /// The cell grid lines up with `origin_x/y/z` (world coordinates).
    World,
}

#[derive(Clone, Debug)]
pub struct Spec {
    pub lattice: Lattice,
    pub cell: f64,
    pub strut_d: f64,
    pub node_d: f64,
    pub counts: [i64; 3],
    pub origin: [f64; 3],
    pub origin_mode: OriginMode,
    /// Fill this body (by name): the lattice covers its bounding box and is cut to its shape.
    pub body: Option<String>,
    /// With no body name: fill the body selected in the viewport, if any.
    pub use_selection: bool,
    pub complete_cells: bool,
    pub keep_body: bool,
    pub clip: bool,
    pub variable: bool,
    pub axis: usize,
    pub even_spacing: bool,
    pub cp_positions: Vec<f64>,
    pub cp_cells: Vec<f64>,
    pub cp_walls: Vec<f64>,
    pub bands: usize,
    pub body_name: String,
    pub max_struts: usize,
    /// TPMS: sample spacing (0 = automatic), solid skin on the body, and the sample budget.
    pub resolution: f64,
    pub shell: f64,
    pub max_samples_millions: f64,
}

fn num(p: &Value, key: &str, default: f64) -> Result<f64, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_f64().filter(|x| x.is_finite()).ok_or_else(|| format!("`{key}` must be a number")),
    }
}

fn int(p: &Value, key: &str, default: i64) -> Result<i64, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_i64()
            .or_else(|| v.as_f64().filter(|x| x.fract() == 0.0 && x.abs() < 1e9).map(|x| x as i64))
            .ok_or_else(|| format!("`{key}` must be a whole number")),
    }
}

fn flag(p: &Value, key: &str) -> Result<bool, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(v) => v.as_bool().ok_or_else(|| format!("`{key}` must be true or false")),
    }
}

fn text<'a>(p: &'a Value, key: &str, default: &'a str) -> Result<&'a str, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_str().map(str::trim).ok_or_else(|| format!("`{key}` must be text")),
    }
}

/// `"5, 3.5, 2"` (commas, semicolons or spaces) → `[5.0, 3.5, 2.0]`; empty → `[]`.
fn list(p: &Value, key: &str) -> Result<Vec<f64>, String> {
    let s = text(p, key, "")?;
    let values = s
        .split([',', ';', ' '])
        .filter(|t| !t.is_empty())
        .map(|t| t.parse::<f64>().ok().filter(|x| x.is_finite()))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| format!("`{key}` must be numbers separated by commas, like \"5, 4, 3\"; got \"{s}\""))?;
    if values.len() > MAX_CONTROL_POINTS {
        return Err(format!("`{key}` has {} values; at most {MAX_CONTROL_POINTS} control points", values.len()));
    }
    Ok(values)
}

impl Spec {
    pub fn parse(p: &Value) -> Result<Self, String> {
        if !p.is_object() {
            return Err("parameters must be an object".into());
        }
        let fill = match text(p, "tpms_type", "sheet")? {
            "sheet" => gyroid::field::Fill::Sheet,
            "solid" => gyroid::field::Fill::Solid,
            f => return Err(format!("`tpms_type` must be sheet or solid; got `{f}`")),
        };
        let lattice = match text(p, "lattice", "bcc")? {
            name if TPMS.contains(&name) => Lattice::Tpms(gyroid::field::Tpms::parse(name).ok_or("internal: TPMS name")?, fill),
            "honeycomb_hex" => Lattice::Honeycomb(Honeycomb::Hexagon),
            "honeycomb_square" => Lattice::Honeycomb(Honeycomb::Square),
            "honeycomb_triangle" => Lattice::Honeycomb(Honeycomb::Triangle),
            name => Lattice::Struts(
                Kind::parse(name)
                    .ok_or_else(|| format!("`lattice` must be one of {}, {}; got `{name}`", Kind::ALL.join(", "), HONEYCOMBS.join(", ")))?,
            ),
        };
        let cell = num(p, "cell_size", 10.0)?;
        let strut_d = num(p, "strut_diameter", 1.5)?;
        let node_d = num(p, "node_diameter", 0.0)?;
        let counts = [int(p, "count_x", 3)?, int(p, "count_y", 3)?, int(p, "count_z", 3)?];
        let origin = [num(p, "origin_x", 0.0)?, num(p, "origin_y", 0.0)?, num(p, "origin_z", 0.0)?];
        let origin_mode = match text(p, "lattice_origin", "body_center")? {
            "body_center" => OriginMode::BodyCenter,
            "body_corner" => OriginMode::BodyCorner,
            "world" => OriginMode::World,
            o => return Err(format!("`lattice_origin` must be body_center, body_corner or world; got `{o}`")),
        };
        let body = Some(text(p, "body", "")?.to_string()).filter(|b| !b.is_empty());
        let axis = match text(p, "lattice_axis", "z")? {
            "x" | "X" => 0,
            "y" | "Y" => 1,
            "z" | "Z" => 2,
            a => return Err(format!("`lattice_axis` must be x, y or z; got `{a}`")),
        };
        let even_spacing = match p.get("even_spacing") {
            None | Some(Value::Null) => true,
            Some(v) => v.as_bool().ok_or("`even_spacing` must be true or false")?,
        };
        let body_name = match p.get("body_name") {
            None | Some(Value::Null) => "Lattice".to_string(),
            Some(v) => v
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty() && s.chars().count() <= 80)
                .ok_or("`body_name` must be a short name")?
                .to_string(),
        };
        let use_selection = match p.get("use_selection") {
            None | Some(Value::Null) => true,
            Some(v) => v.as_bool().ok_or("`use_selection` must be true or false")?,
        };
        let spec = Self {
            lattice,
            cell,
            strut_d,
            node_d,
            counts,
            origin,
            origin_mode,
            body,
            use_selection,
            complete_cells: flag(p, "complete_cells_only")?,
            keep_body: flag(p, "keep_body")?,
            clip: flag(p, "clip_to_box")?,
            variable: flag(p, "variable_density")?,
            axis,
            even_spacing,
            cp_positions: list(p, "cp_positions")?,
            cp_cells: list(p, "cp_cell_sizes")?,
            cp_walls: list(p, "cp_wall_thicknesses")?,
            bands: int(p, "bands", 6)? as usize,
            body_name,
            max_struts: int(p, "max_struts", 1500)?.clamp(0, 1_000_000) as usize,
            resolution: num(p, "resolution", 0.0)?,
            shell: num(p, "shell", 0.0)?,
            max_samples_millions: num(p, "max_samples_millions", 1.5)?,
        };
        spec.check()?;
        Ok(spec)
    }

    fn check(&self) -> Result<(), String> {
        if !(0.1..=10_000.0).contains(&self.cell) {
            return Err("`cell_size` must be 0.1 to 10000 mm".into());
        }
        if self.strut_d <= 0.0 {
            return Err("`strut_diameter` (wall thickness) must be above 0".into());
        }
        if self.node_d != 0.0 && self.node_d < self.strut_d * 1.05 {
            return Err("`node_diameter` must be 0 (no nodes) or at least 1.05 x `strut_diameter`".into());
        }
        for (axis, c) in ["count_x", "count_y", "count_z"].iter().zip(self.counts) {
            if !(1..=MAX_CELLS_PER_AXIS).contains(&c) {
                return Err(format!("`{axis}` must be 1 to {MAX_CELLS_PER_AXIS}"));
            }
        }
        if !(2..=MAX_BANDS).contains(&self.bands) {
            return Err(format!("`bands` must be 2 to {MAX_BANDS}"));
        }
        if !(1..=100_000).contains(&self.max_struts) {
            return Err("`max_struts` must be 1 to 100000".into());
        }
        if self.cp_cells.iter().chain(&self.cp_walls).any(|v| *v <= 0.0) {
            return Err("control point cell sizes and wall thicknesses must be above 0".into());
        }
        if self.resolution < 0.0 || self.shell < 0.0 {
            return Err("`resolution` and `shell` must be 0 or more".into());
        }
        if !(0.01..=1000.0).contains(&self.max_samples_millions) {
            return Err("`max_samples_millions` must be 0.01 to 1000".into());
        }
        if let Lattice::Honeycomb(_) = self.lattice {
            if self.variable {
                return Err("Variable Density works with strut lattices; honeycombs have one cell size and wall".into());
            }
            if self.keep_body {
                return Err("a honeycomb is cut into the body itself, so Keep body isn't available; copy the body first".into());
            }
            if self.strut_d >= self.cell {
                return Err("for a honeycomb the wall thickness must be less than the cell size".into());
            }
        }
        Ok(())
    }

    /// Node sphere diameter / strut diameter; 0 = no nodes.
    pub fn node_ratio(&self) -> f64 {
        self.node_d / self.strut_d
    }

    /// The control points as (position from the axis start, cell size, wall thickness).
    pub fn control_points(&self, length: f64) -> Result<Vec<(f64, f64, f64)>, String> {
        let n = self.cp_cells.len().max(self.cp_walls.len()).max(if self.even_spacing { 0 } else { self.cp_positions.len() }).max(2);
        let fill = |v: &[f64], default: f64, what: &str| -> Result<Vec<f64>, String> {
            match v.len() {
                0 => Ok(vec![default; n]),
                l if l == n => Ok(v.to_vec()),
                l => Err(format!("{what} has {l} values but there are {n} control points; give one per point")),
            }
        };
        let cells = fill(&self.cp_cells, self.cell, "`cp_cell_sizes`")?;
        let walls = fill(&self.cp_walls, self.strut_d, "`cp_wall_thicknesses`")?;
        // Positions only when Even Spacing is off and some were given; otherwise evenly spaced.
        let positions = if self.even_spacing || self.cp_positions.is_empty() {
            (0..n).map(|i| length * i as f64 / (n - 1) as f64).collect()
        } else {
            let v = fill(&self.cp_positions, 0.0, "`cp_positions`")?;
            if v.windows(2).any(|w| w[1] <= w[0]) {
                return Err("`cp_positions` must increase from point to point".into());
            }
            v
        };
        Ok((0..n).map(|i| (positions[i], cells[i], walls[i])).collect())
    }
}

/// Piecewise-linear value at `x` through `(x, y)` points sorted by x; constant beyond the ends.
fn interp(points: &[(f64, f64)], x: f64) -> f64 {
    match points {
        [] => 0.0,
        [only] => only.1,
        _ => {
            if x <= points[0].0 {
                return points[0].1;
            }
            for w in points.windows(2) {
                if x <= w[1].0 {
                    let t = (x - w[0].0) / (w[1].0 - w[0].0);
                    return w[0].1 + (w[1].1 - w[0].1) * t;
                }
            }
            points[points.len() - 1].1
        }
    }
}

/// Layer boundaries from `start` over `length` whose heights follow `cell(s)`: the number of
/// layers is the rounded count of local cells that fit, and the boundaries are spaced so each
/// layer holds the same amount of "cells" (so heights track the profile and end exactly).
fn layers(start: f64, length: f64, cell: &[(f64, f64)]) -> Vec<f64> {
    const N: usize = 4096;
    let ds = length / N as f64;
    let mut u = vec![0.0; N + 1];
    for i in 0..N {
        let (a, b) = (i as f64 * ds, (i + 1) as f64 * ds);
        u[i + 1] = u[i] + ds * 0.5 * (1.0 / interp(cell, a) + 1.0 / interp(cell, b));
    }
    let total = u[N];
    let n = (total.round() as usize).max(1);
    let mut out = vec![start];
    let mut j = 0;
    for k in 1..n {
        let target = total * k as f64 / n as f64;
        while u[j + 1] < target {
            j += 1;
        }
        let t = (target - u[j]) / (u[j + 1] - u[j]);
        out.push(start + (j as f64 + t) * ds);
    }
    out.push(start + length);
    out
}

#[derive(Clone, Debug, PartialEq)]
pub enum Trim {
    None,
    /// Intersect with this box (corner, size).
    Box([f64; 3], [f64; 3]),
    /// Intersect with this body; `keep` leaves the body as it was.
    Body {
        name: String,
        keep: bool,
    },
}

/// The grid in the model.
#[derive(Clone, Debug)]
pub struct Layout {
    /// The grid's minimum corner.
    pub origin: [f64; 3],
    /// Cell size on the uniform axes.
    pub cell: f64,
    pub counts: [i64; 3],
    /// The variable-density axis and its layer boundaries (absolute, `counts[axis] + 1` of them).
    pub layers: Option<(usize, Vec<f64>)>,
    /// Wall thickness along the variable axis: (absolute position, diameter). One point when
    /// uniform.
    pub walls: Vec<(f64, f64)>,
    pub trim: Trim,
    /// The filled body's bounding box.
    pub region: Option<([f64; 3], [f64; 3])>,
}

impl Layout {
    pub fn resolve(spec: &Spec, bbox: Option<([f64; 3], [f64; 3])>) -> Result<Self, String> {
        let c = spec.cell;
        let eps = 1e-9;
        let mut origin = spec.origin;
        let mut counts = spec.counts;
        let names = ["X", "Y", "Z"];
        if let Some((lo, hi)) = bbox {
            for b in 0..3 {
                let e = hi[b] - lo[b];
                let (start, n) = match spec.origin_mode {
                    OriginMode::BodyCorner | OriginMode::BodyCenter => {
                        let n = if spec.complete_cells { (e / c + eps).floor() } else { (e / c - eps).ceil().max(1.0) } as i64;
                        let start =
                            if spec.origin_mode == OriginMode::BodyCorner { lo[b] } else { (lo[b] + hi[b]) / 2.0 - n as f64 * c / 2.0 };
                        (start, n)
                    }
                    OriginMode::World => {
                        let a0 = spec.origin[b];
                        let (k0, k1) = if spec.complete_cells {
                            (((lo[b] - a0) / c - eps).ceil(), ((hi[b] - a0) / c + eps).floor())
                        } else {
                            (((lo[b] - a0) / c + eps).floor(), ((hi[b] - a0) / c - eps).ceil())
                        };
                        (a0 + k0 * c, (k1 - k0) as i64)
                    }
                };
                if n < 1 {
                    return Err(format!(
                        "the body is {e:.3} mm along {}, smaller than one {c} mm cell; use a smaller cell size or turn off Complete Cells Only",
                        names[b]
                    ));
                }
                if n > MAX_CELLS_PER_AXIS {
                    return Err(format!(
                        "the body needs {n} cells along {}, more than {MAX_CELLS_PER_AXIS}; use a bigger cell size",
                        names[b]
                    ));
                }
                origin[b] = start;
                counts[b] = n;
            }
        }
        let mut walls = vec![(0.0, spec.strut_d)];
        let mut layered = None;
        if spec.variable {
            let a = spec.axis;
            let (start, length) = match bbox {
                Some((lo, hi)) => (lo[a], hi[a] - lo[a]),
                None => (spec.origin[a], spec.counts[a] as f64 * c),
            };
            if length <= 0.0 {
                return Err(format!("the variable-density axis {} has no length", names[a]));
            }
            let cps = spec.control_points(length)?;
            let cell_profile: Vec<(f64, f64)> = cps.iter().map(|p| (p.0, p.1)).collect();
            let bounds = layers(start, length, &cell_profile);
            let n = bounds.len() as i64 - 1;
            if n > MAX_CELLS_PER_AXIS {
                return Err(format!("the control points make {n} layers along {}, more than {MAX_CELLS_PER_AXIS}", names[a]));
            }
            origin[a] = start;
            counts[a] = n;
            walls = cps.iter().map(|p| (start + p.0, p.2)).collect();
            layered = Some((a, bounds));
        }
        let trim = match (&spec.body, spec.complete_cells) {
            (Some(name), false) => Trim::Body { name: name.clone(), keep: spec.keep_body },
            (Some(_), true) => Trim::None,
            (None, _) if spec.clip => {
                let size = [0, 1, 2].map(|b| match &layered {
                    Some((a, bounds)) if *a == b => bounds[bounds.len() - 1] - bounds[0],
                    _ => counts[b] as f64 * c,
                });
                Trim::Box(origin, size)
            }
            (None, _) => Trim::None,
        };
        Ok(Self { origin, cell: c, counts, layers: layered, walls, trim, region: bbox })
    }

    /// A lattice point (quarter-cell units) in mm.
    pub fn pos(&self, q: P) -> [f64; 3] {
        [0, 1, 2].map(|b| match &self.layers {
            Some((a, bounds)) if *a == b => {
                let k = q[b].div_euclid(Q) as usize;
                let f = q[b].rem_euclid(Q) as f64 / Q as f64;
                match (bounds.get(k), bounds.get(k + 1)) {
                    (Some(s0), Some(s1)) => s0 + f * (s1 - s0),
                    (Some(s0), None) => *s0,
                    _ => f64::NAN,
                }
            }
            _ => self.origin[b] + q[b] as f64 * self.cell / Q as f64,
        })
    }

    /// The inverse of [`Self::pos`] on one axis (for checks): mm → quarter-cell units.
    pub fn to_q(&self, b: usize, x: f64) -> f64 {
        match &self.layers {
            Some((a, bounds)) if *a == b => {
                let k = bounds.windows(2).position(|w| x <= w[1] + 1e-9).unwrap_or(bounds.len() - 2);
                Q as f64 * (k as f64 + (x - bounds[k]) / (bounds[k + 1] - bounds[k]))
            }
            _ => (x - self.origin[b]) / self.cell * Q as f64,
        }
    }

    /// The wall thickness (strut diameter) at `m`, before banding.
    pub fn wall_at(&self, m: [f64; 3]) -> f64 {
        match &self.layers {
            Some((a, _)) => interp(&self.walls, m[*a]),
            None => self.walls[0].1,
        }
    }

    pub fn wall_range(&self) -> (f64, f64) {
        self.walls.iter().fold((f64::MAX, f64::MIN), |(lo, hi), w| (lo.min(w.1), hi.max(w.1)))
    }

    /// The variable axis, if any.
    pub fn layer_axis(&self) -> Option<usize> {
        self.layers.as_ref().map(|l| l.0)
    }

    /// The grid's extent: (min corner, max corner).
    pub fn envelope(&self) -> ([f64; 3], [f64; 3]) {
        let hi = self.pos(self.counts.map(|n| n * Q));
        (self.origin, hi)
    }
}
