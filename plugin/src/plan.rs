//! Turns a resolved layout into an ordered list of SolidCraft feature steps, and runs them
//! through anything that can execute app commands (the real host, or a test recorder).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::lattice::{self, P};
use crate::layout::{Honeycomb, Lattice, Layout, OriginMode, Spec, Trim};

/// Host calls the runtime allows per invocation is 256; keep headroom for the body measure.
pub const MAX_STEPS: usize = 240;
/// SolidCraft refuses a pattern with more than 1000 instances, nested copies included.
pub const MAX_PATTERN_INSTANCES: i64 = 1000;

/// Round away float noise so the timeline shows 12.5, not 12.500000000000002.
pub fn tidy(x: f64) -> f64 {
    let r = (x * 1e9).round() / 1e9;
    if r == 0.0 { 0.0 } else { r }
}

/// Wall thicknesses quantized to `n` evenly spaced levels between the thinnest and thickest.
#[derive(Clone, Copy, Debug)]
pub struct Levels {
    min: f64,
    max: f64,
    pub n: usize,
}
impl Levels {
    fn new(layout: &Layout, n: usize) -> Self {
        let (min, max) = layout.wall_range();
        let n = if max - min < 1e-9 { 1 } else { n };
        Self { min, max, n }
    }
    pub fn level(&self, d: f64) -> u32 {
        if self.n == 1 { 0 } else { ((d - self.min) / (self.max - self.min) * (self.n - 1) as f64).round() as u32 }
    }
    pub fn diameter(&self, level: u32) -> f64 {
        if self.n == 1 { tidy(self.min) } else { tidy(self.min + level as f64 * (self.max - self.min) / (self.n - 1) as f64) }
    }
}

/// One feature to add. Patterns refer to earlier steps by index.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// The body everything joins: a small sphere buried in the middle of the first strut.
    Seed {
        center: [f64; 3],
        radius: f64,
    },
    Strut {
        base: [f64; 3],
        axis: [f64; 3],
        height: f64,
        radius: f64,
    },
    Node {
        center: [f64; 3],
        radius: f64,
    },
    /// Rectangular feature pattern of `sources`, one or two directions `(axis, count, spacing)`.
    Pattern {
        sources: Vec<usize>,
        dirs: Vec<(usize, i64, f64)>,
    },
    /// Intersect with the lattice's envelope.
    Clip {
        corner: [f64; 3],
        size: [f64; 3],
    },
    /// Intersect with a body.
    Trim {
        body: String,
        keep: bool,
    },
    /// The honeycomb's block when there is no body: a new box.
    Block {
        corner: [f64; 3],
        size: [f64; 3],
    },
    /// A sketch on the plane through `origin` spanned by `x_dir`, `y_dir`.
    SketchCreate {
        origin: [f64; 3],
        x_dir: [f64; 3],
        y_dir: [f64; 3],
    },
    /// A rectangle in that sketch, corners in sketch coordinates.
    Rect {
        p0: [f64; 2],
        p1: [f64; 2],
    },
    SketchFinish,
    /// Cut honeycomb cells through the body, the field being the last sketch.
    CellGrid {
        shape: &'static str,
        size: f64,
        web: f64,
        anchor: [f64; 3],
        clip: &'static str,
    },
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub steps: Vec<Step>,
    pub struts: usize,
    pub nodes: usize,
    /// Thickness levels actually built, thinnest first.
    pub diameters: Vec<f64>,
    /// How many levels the thickness range was split into (1 when uniform).
    pub bands: usize,
    /// For a honeycomb: about how many cells it cuts.
    pub honeycomb_cells: Option<usize>,
}

impl Plan {
    pub fn summary(&self, spec: &Spec, layout: &Layout) -> Value {
        let (lo, hi) = layout.envelope();
        let mut out = json!({
            "struts": self.struts,
            "nodes": if spec.node_d > 0.0 { self.nodes } else { 0 },
            "features": self.steps.len(),
            "cells": layout.counts,
            "wall_thicknesses_mm": self.diameters,
            "thickness_levels": self.bands,
            "envelope_min_mm": lo.map(tidy),
            "envelope_max_mm": hi.map(tidy),
        });
        if let Some(n) = self.honeycomb_cells {
            out["honeycomb_cells"] = json!(n);
            out["struts"] = json!(0);
        }
        if let Some((a, bounds)) = &layout.layers {
            out["variable_axis"] = json!(["x", "y", "z"][*a]);
            out["layer_heights_mm"] = json!(bounds.windows(2).map(|w| ((w[1] - w[0]) * 1e3).round() / 1e3).collect::<Vec<_>>());
        }
        out
    }
}

/// Patterns that copy every seed in `seeds` (all with the same `counts`) over the block.
fn push_patterns(steps: &mut Vec<Step>, seeds: &[usize], counts: [i64; 3], cell: f64) -> Result<(), String> {
    let axes: Vec<(usize, i64, f64)> = (0..3).filter(|&a| counts[a] > 1).map(|a| (a, counts[a], tidy(cell))).collect();
    let flat = |dirs: &[(usize, i64, f64)]| -> Result<(), String> {
        let n: i64 = dirs.iter().map(|d| d.1).product();
        if n > MAX_PATTERN_INSTANCES {
            Err(format!("a pattern of {n} copies exceeds SolidCraft's {MAX_PATTERN_INSTANCES}; use fewer cells"))
        } else {
            Ok(())
        }
    };
    if axes.len() <= 2 {
        if !axes.is_empty() {
            flat(&axes)?;
            steps.push(Step::Pattern { sources: seeds.to_vec(), dirs: axes });
        }
        return Ok(());
    }
    // The outer pattern copies the inner one's copies again: SolidCraft caps that at
    // (outer copies) x (inner copies) <= 1000, so pattern the pair that keeps it lowest.
    let nested = |alone: usize| {
        let pair: i64 = (0..3).filter(|&a| a != alone).map(|a| axes[a].1).product();
        (axes[alone].1 - 1) * (pair - 1)
    };
    let alone = (0..3).min_by_key(|&a| nested(a)).unwrap_or(2);
    if nested(alone) <= MAX_PATTERN_INSTANCES {
        let pair: Vec<_> = (0..3).filter(|&a| a != alone).map(|a| axes[a]).collect();
        flat(&pair)?;
        steps.push(Step::Pattern { sources: seeds.to_vec(), dirs: pair });
        // A pattern of a pattern copies only that pattern's copies; listing the seeds too
        // copies the originals as well (SolidCraft #395).
        let mut sources = seeds.to_vec();
        sources.push(steps.len() - 1);
        steps.push(Step::Pattern { sources, dirs: vec![axes[alone]] });
        return Ok(());
    }
    // One axis at a time: X, then [seeds, X] along Y, then [seeds, X, XY] along Z. The deepest
    // nesting is (cx-1)(cy-1)(cz-1) copies.
    let deepest: i64 = axes.iter().map(|d| d.1 - 1).product();
    if deepest > MAX_PATTERN_INSTANCES {
        return Err(format!(
            "a block of {} x {} x {} cells needs {deepest} nested pattern copies, more than SolidCraft's {MAX_PATTERN_INSTANCES}; use fewer cells (about 11 nodes per axis at most)",
            counts[0], counts[1], counts[2]
        ));
    }
    let mut sources = seeds.to_vec();
    for d in &axes {
        steps.push(Step::Pattern { sources: sources.clone(), dirs: vec![*d] });
        sources.push(steps.len() - 1);
    }
    Ok(())
}

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Family tag: thickness level, plus the position along a variable axis (layers differ, so
/// nothing is copied along it).
fn tag(level: u32, layout: &Layout, start: &P) -> u32 {
    level * 4096 + layout.layer_axis().map_or(0, |a| start[a] as u32)
}

/// The plan at one number of thickness levels.
fn build_with(spec: &Spec, layout: &Layout, struts: &BTreeSet<(P, P)>, levels: Levels) -> Result<Plan, String> {
    let strut_level = |s: &(P, P)| {
        let (a, b) = (layout.pos(s.0), layout.pos(s.1));
        levels.level(layout.wall_at([0, 1, 2].map(|i| (a[i] + b[i]) / 2.0)))
    };
    // A strut thicker than half its length is a blob, and the seed sphere would poke out.
    let mut shortest_at: BTreeMap<P, f64> = BTreeMap::new();
    for s in struts {
        let (a, b) = (layout.pos(s.0), layout.pos(s.1));
        let (len, d) = (dist(a, b), levels.diameter(strut_level(s)));
        if d > 0.5 * len {
            let m = [0, 1, 2].map(|i| tidy((a[i] + b[i]) / 2.0));
            return Err(format!(
                "a {d} mm wall is too thick for a {len:.3} mm strut (near {m:?}); keep wall thickness at or below half the strut length: use thinner walls or bigger cells there"
            ));
        }
        for e in [s.0, s.1] {
            let v = shortest_at.entry(e).or_insert(f64::MAX);
            *v = v.min(len);
        }
    }
    let sf = lattice::strut_families(struts, |s| tag(strut_level(s), layout, &s.0));
    let nodes = lattice::nodes(struts);
    let ratio = spec.node_ratio();
    let nf = if ratio > 0.0 {
        // A node is as thick as the thickest strut it joins.
        let mut at: BTreeMap<P, u32> = BTreeMap::new();
        for s in struts {
            let l = strut_level(s);
            for end in [s.0, s.1] {
                let e = at.entry(end).or_insert(l);
                *e = (*e).max(l);
            }
        }
        for (n, l) in &at {
            let d = levels.diameter(*l) * ratio;
            if d > shortest_at[n] {
                return Err(format!(
                    "a {d:.3} mm node is longer than the {:.3} mm strut it joins; use a smaller node diameter",
                    shortest_at[n]
                ));
            }
        }
        lattice::node_families(&nodes, |n| tag(at.get(n).copied().unwrap_or(0), layout, n))
    } else {
        Vec::new()
    };
    let level_of = |t: u32| t / 4096;
    let end_of = |f: &lattice::Family| [0, 1, 2].map(|i| f.start[i] + f.vector[i]);

    let first = sf.first().ok_or("the lattice has no struts")?;
    let (a0, b0) = (layout.pos(first.start), layout.pos(end_of(first)));
    let mid = [0, 1, 2].map(|i| tidy((a0[i] + b0[i]) / 2.0));
    let mut steps = vec![Step::Seed { center: mid, radius: tidy(levels.diameter(level_of(first.tag)) / 4.0) }];

    // Seeds that share a block shape share their patterns.
    let mut groups: BTreeMap<[i64; 3], Vec<usize>> = BTreeMap::new();
    let mut used = BTreeSet::new();
    for f in &sf {
        if let Some(a) = layout.layer_axis()
            && f.counts[a] > 1
        {
            return Err("internal: a family repeats along the variable-density axis".into());
        }
        let (a, b) = (layout.pos(f.start), layout.pos(end_of(f)));
        let len = dist(a, b);
        let axis = [0, 1, 2].map(|i| tidy((b[i] - a[i]) / len));
        let d = levels.diameter(level_of(f.tag));
        used.insert(level_of(f.tag));
        steps.push(Step::Strut { base: a.map(tidy), axis, height: tidy(len), radius: tidy(d / 2.0) });
        groups.entry(f.counts).or_default().push(steps.len() - 1);
    }
    for f in &nf {
        steps.push(Step::Node { center: layout.pos(f.start).map(tidy), radius: tidy(levels.diameter(level_of(f.tag)) * ratio / 2.0) });
        groups.entry(f.counts).or_default().push(steps.len() - 1);
    }
    for (counts, seeds) in &groups {
        push_patterns(&mut steps, seeds, *counts, layout.cell)?;
    }
    match &layout.trim {
        Trim::None => {}
        Trim::Box(corner, size) => steps.push(Step::Clip { corner: corner.map(tidy), size: size.map(tidy) }),
        Trim::Body { name, keep } => steps.push(Step::Trim { body: name.clone(), keep: *keep }),
    }
    Ok(Plan {
        steps,
        struts: struts.len(),
        nodes: nodes.len(),
        diameters: used.into_iter().map(|l| levels.diameter(l)).collect(),
        bands: levels.n,
        honeycomb_cells: None,
    })
}

/// Most cells one Cell Grid cuts (SolidCraft's `MAX_CUT_CELLS`).
pub const MAX_HONEYCOMB_CELLS: f64 = 250.0;

/// A 2.5D honeycomb: walls `strut_d` thick at pitch `cell`, prisms along the lattice axis,
/// cut through the body (or a new block) with SolidCraft's Cell Grid.
fn build_honeycomb(spec: &Spec, layout: &Layout, h: Honeycomb) -> Result<Plan, String> {
    let a = spec.axis;
    let (u, v) = match a {
        0 => (1, 2),
        1 => (2, 0),
        _ => (0, 1),
    };
    let unit = |i: usize| {
        let mut d = [0.0; 3];
        d[i] = 1.0;
        d
    };
    let (lo, hi) = if spec.body.is_some() { layout.region.ok_or("internal: no body region")? } else { layout.envelope() };
    let ext = [0, 1, 2].map(|i| hi[i] - lo[i]);
    let cells = ext[u] * ext[v] / h.cell_area(spec.cell);
    // Cells along the field's edge are counted too.
    let border = 2.0 * (ext[u] + ext[v]) / spec.cell + 4.0;
    if cells + border > MAX_HONEYCOMB_CELLS {
        return Err(format!(
            "about {:.0} honeycomb cells across {:.1} x {:.1} mm; SolidCraft cuts at most {MAX_HONEYCOMB_CELLS} in one go, so use a cell size of about {:.1} mm or more",
            cells + border,
            ext[u],
            ext[v],
            spec.cell * ((cells + border) / MAX_HONEYCOMB_CELLS).sqrt()
        ));
    }
    let centre = [0, 1, 2].map(|i| (lo[i] + hi[i]) / 2.0);
    let anchor = match spec.origin_mode {
        OriginMode::BodyCenter => centre,
        OriginMode::BodyCorner => {
            let mut p = lo;
            p[u] += spec.cell / 2.0;
            p[v] += spec.cell / 2.0;
            p
        }
        OriginMode::World => spec.origin,
    };
    let mut steps = Vec::new();
    if spec.body.is_none() {
        steps.push(Step::Block { corner: lo.map(tidy), size: ext.map(tidy) });
    }
    steps.push(Step::SketchCreate { origin: lo.map(tidy), x_dir: unit(u), y_dir: unit(v) });
    steps.push(Step::Rect { p0: [0.0, 0.0], p1: [tidy(ext[u]), tidy(ext[v])] });
    steps.push(Step::SketchFinish);
    steps.push(Step::CellGrid {
        shape: h.shape(),
        size: tidy(spec.cell - spec.strut_d),
        web: tidy(spec.strut_d),
        anchor: anchor.map(tidy),
        clip: if spec.complete_cells { "whole" } else { "free" },
    });
    Ok(Plan { steps, struts: 0, nodes: 0, diameters: vec![tidy(spec.strut_d)], bands: 1, honeycomb_cells: Some(cells.round() as usize) })
}

pub fn build(spec: &Spec, layout: &Layout) -> Result<Plan, String> {
    let kind = match spec.lattice {
        Lattice::Struts(k) => k,
        Lattice::Honeycomb(h) => return build_honeycomb(spec, layout, h),
        Lattice::Tpms(..) => return Err("internal: TPMS lattices are meshed, not planned".into()),
    };
    let struts = lattice::tile(kind, layout.counts);
    if struts.len() > spec.max_struts {
        return Err(format!(
            "{} struts is more than the strut limit ({}). Large lattices can run past the 30 s plugin limit; use bigger cells or raise the limit",
            struts.len(),
            spec.max_struts
        ));
    }
    // More thickness levels mean more features: use as many as fit in one plugin call.
    let mut needed = 0;
    for n in (2..=spec.bands).rev() {
        let levels = Levels::new(layout, n);
        let plan = build_with(spec, layout, &struts, levels)?;
        if plan.steps.len() <= MAX_STEPS {
            return Ok(plan);
        }
        needed = plan.steps.len();
        if levels.n == 1 {
            break;
        }
    }
    Err(format!(
        "this lattice needs {needed} features, more than one plugin call may add ({MAX_STEPS}); use bigger cells, fewer layers along the variable axis, a simpler lattice type, or no nodes"
    ))
}

/// Anything that runs an app command and returns its JSON result.
pub trait Engine {
    fn run(&mut self, command: &str, args: &Value) -> Result<Value, String>;
    fn progress(&mut self, _fraction: f32, _message: &str) {}
    fn cancelled(&mut self) -> bool {
        false
    }
}

const AXES: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// The command and arguments for one step. `names` holds the feature names of earlier steps.
pub fn command(
    step: &Step,
    body: &str,
    names: &[String],
    body_name: &str,
    sketch: Option<&Value>,
) -> Result<(&'static str, Value), String> {
    let name_of = |i: usize| names.get(i).cloned().ok_or_else(|| format!("internal: step {i} has no feature name yet"));
    Ok(match step {
        Step::Seed { center, radius } => {
            ("solid.sphere", json!({"center": center, "radius": radius, "operation": "new", "body_name": body_name}))
        }
        Step::Strut { base, axis, height, radius } => (
            "solid.cylinder",
            json!({"base": base, "axis": axis, "height": height, "radius": radius, "operation": "join", "targets": [body]}),
        ),
        Step::Node { center, radius } => {
            ("solid.sphere", json!({"center": center, "radius": radius, "operation": "join", "targets": [body]}))
        }
        Step::Pattern { sources, dirs } => {
            let features = sources.iter().map(|&i| name_of(i)).collect::<Result<Vec<_>, _>>()?;
            let mut args = json!({"features": features, "dir1": AXES[dirs[0].0], "count1": dirs[0].1, "spacing1": dirs[0].2});
            if let Some(d) = dirs.get(1) {
                args["dir2"] = json!(AXES[d.0]);
                args["count2"] = json!(d.1);
                args["spacing2"] = json!(d.2);
            }
            ("solid.pattern.rectangular", args)
        }
        Step::Clip { corner, size } => (
            "solid.box",
            json!({"corner": corner, "length": size[0], "width": size[1], "height": size[2], "operation": "intersect", "targets": [body]}),
        ),
        Step::Trim { body: tool, keep } => {
            ("solid.combine", json!({"target": body, "tools": [tool], "operation": "intersect", "keep_tools": keep}))
        }
        Step::Block { corner, size } => (
            "solid.box",
            json!({"corner": corner, "length": size[0], "width": size[1], "height": size[2], "operation": "new", "body_name": body_name}),
        ),
        Step::SketchCreate { origin, x_dir, y_dir } => {
            ("sketch.create", json!({"plane": {"origin": origin, "x_dir": x_dir, "y_dir": y_dir}}))
        }
        Step::Rect { p0, p1 } => ("sketch.rectangle.two_point", json!({"p0": p0, "p1": p1})),
        Step::SketchFinish => ("sketch.finish", json!({})),
        Step::CellGrid { shape, size, web, anchor, clip } => (
            "solid.cell_grid",
            json!({"sketch": sketch.ok_or("internal: no sketch for the cell grid")?, "shape": shape, "size": size, "web": web,
                   "anchor": anchor, "clip": clip, "through_all": true, "direction": "positive", "targets": [body]}),
        ),
    })
}

/// Run every step. Returns the result summary with the body and feature names.
pub fn execute(plan: &Plan, spec: &Spec, layout: &Layout, engine: &mut dyn Engine) -> Result<Value, String> {
    let total = plan.steps.len();
    let mut names: Vec<String> = Vec::with_capacity(total);
    // A strut lattice is its own new body; a honeycomb is cut into the filled body.
    let mut body = match (spec.lattice, &spec.body) {
        (Lattice::Honeycomb(_), Some(b)) => b.clone(),
        _ => spec.body_name.clone(),
    };
    let mut sketch: Option<Value> = None;
    for (i, step) in plan.steps.iter().enumerate() {
        if engine.cancelled() {
            return Err("cancelled".into());
        }
        engine.progress(i as f32 / total as f32, &format!("Lattice feature {} of {total}", i + 1));
        let (cmd, args) = command(step, &body, &names, &spec.body_name, sketch.as_ref())?;
        let reply = engine.run(cmd, &args)?;
        let name = reply.get("name").and_then(Value::as_str);
        if name.is_none() && matches!(step, Step::Seed { .. } | Step::Strut { .. } | Step::Node { .. } | Step::Pattern { .. }) {
            return Err(format!("{cmd} returned no feature name: {reply}"));
        }
        names.push(name.unwrap_or_default().to_string());
        if let Step::SketchCreate { .. } = step {
            sketch = Some(reply.get("sketch").cloned().ok_or_else(|| format!("sketch.create returned no sketch: {reply}"))?);
        }
        if matches!(step, Step::Seed { .. } | Step::Block { .. }) {
            // The app may rename a body whose name is taken.
            if let Some(made) = reply.get("created_bodies").and_then(|b| b.get(0)).and_then(Value::as_str) {
                body = made.to_string();
            }
        }
    }
    engine.progress(1.0, "Lattice done");
    let mut out = plan.summary(spec, layout);
    out["body"] = json!(body);
    out["feature_names"] = json!(names.into_iter().filter(|n| !n.is_empty()).collect::<Vec<_>>());
    Ok(out)
}

/// The `(min, max)` bounding box `inspect.measure` reports for `body`.
pub fn measure_body(engine: &mut dyn Engine, body: &str) -> Result<([f64; 3], [f64; 3]), String> {
    let reply = engine.run("inspect.measure", &json!({"bodies": [body]}))?;
    let found = reply
        .get("bodies")
        .and_then(Value::as_array)
        .and_then(|b| b.iter().find(|x| x.get("name").and_then(Value::as_str) == Some(body)))
        .ok_or_else(|| format!("no body called `{body}`"))?;
    if let Some(e) = found.get("error").and_then(Value::as_str) {
        return Err(format!("could not measure `{body}`: {e}"));
    }
    let v3 = |v: Option<&Value>| -> Option<[f64; 3]> {
        let a = v?.as_array()?;
        Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?])
    };
    let bbox = found.get("bbox");
    match (v3(bbox.and_then(|b| b.get("min"))), v3(bbox.and_then(|b| b.get("max")))) {
        (Some(lo), Some(hi)) if (0..3).all(|i| hi[i] > lo[i]) => Ok((lo, hi)),
        _ => Err(format!("`{body}` has no usable bounding box")),
    }
}

/// The body selected in the viewport: a selected body, or the body of a selected face, edge or
/// vertex. `None` when nothing like that is selected.
pub fn selected_body(engine: &mut dyn Engine) -> Result<Option<String>, String> {
    let reply = engine.run("selection.get", &json!({"detail": false}))?;
    let items = reply.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(items.iter().find_map(|it| {
        let kind = it.get("kind").or_else(|| it.get("type")).and_then(Value::as_str)?;
        let key = match kind {
            "body" => "name",
            "face" | "edge" | "vertex" => "body",
            _ => return None,
        };
        it.get(key).and_then(Value::as_str).map(str::to_string)
    }))
}
