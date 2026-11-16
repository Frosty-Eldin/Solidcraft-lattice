//! Gyroid and the other TPMS lattices, as a mesh body, the way the Fusion Lattice Generator
//! add-in makes them.
//!
//! The body's triangles come from `model.mesh`; the lattice is a distance field cut to the body
//! (the `gyroid` crate, the same code as gyroid.exe), meshed by marching tetrahedra and
//! simplified. SolidCraft takes a mesh body with its triangles embedded (`timeline.add` of a
//! `mesh_import` feature, what Insert Mesh makes), but a plugin call carries at most 1 MiB, so
//! a large lattice arrives as several closed slabs, each a mesh body, overlapping by a hair so
//! they print as one part.

use serde_json::{Value, json};

use crate::layout::{OriginMode, Spec};
use crate::plan::Engine;
use gyroid::field::{Fill, Shape, Tpms, Tiling};
use crate::layout::{Honeycomb, Lattice};
use gyroid::{Job, Origin, Region};

/// What one `timeline.add` may carry, with room for the rest of the call.
pub const MAX_CALL_BYTES: usize = 1_000_000;
/// Calls kept back for reading the selection and the body.
const RESERVED_CALLS: usize = 16;
pub const MAX_SLABS: usize = 256 - RESERVED_CALLS;

/// The body's triangles from `model.mesh`, coarsening the chord tolerance until they fit.
pub fn body_triangles(engine: &mut dyn Engine, body: &str) -> Result<gyroid::stl::Triangles, String> {
    let mut last = String::new();
    for tol in [0.02, 0.05, 0.1, 0.2, 0.5, 1.0] {
        match engine.run("model.mesh", &json!({"body": body, "tolerance": tol})) {
            Ok(r) => {
                let pos = r.get("positions").and_then(Value::as_array).ok_or("model.mesh returned no positions")?;
                let p3 = |v: &Value| -> Option<[f64; 3]> {
                    let a = v.as_array()?;
                    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?])
                };
                let verts = pos.iter().map(p3).collect::<Option<Vec<_>>>().ok_or("model.mesh returned a bad position")?;
                let tris = r.get("triangles").and_then(Value::as_array).ok_or("model.mesh returned no triangles")?;
                let mut out = Vec::with_capacity(tris.len());
                for t in tris {
                    let ix = t.as_array().filter(|a| a.len() == 3).ok_or("model.mesh returned a bad triangle")?;
                    let v = |k: usize| ix[k].as_u64().and_then(|i| verts.get(i as usize).copied());
                    out.push([v(0).ok_or("bad index")?, v(1).ok_or("bad index")?, v(2).ok_or("bad index")?]);
                }
                if out.is_empty() {
                    return Err(format!("`{body}` has no triangles"));
                }
                return Ok(out);
            }
            Err(e) => last = e,
        }
    }
    Err(format!("could not read `{body}` as a mesh: {last}"))
}

/// The gyroid crate's job for this dialog.
pub fn job(spec: &Spec, kind: Tpms, fill: Fill, shape: Option<Shape>, length: f64) -> Result<Job, String> {
    let control_points = if spec.variable { spec.control_points(length)? } else { Vec::new() };
    let job = Job {
        kind,
        fill,
        cell: spec.cell,
        wall: spec.strut_d,
        resolution: (spec.resolution > 0.0).then_some(spec.resolution),
        origin: match spec.origin_mode {
            OriginMode::BodyCenter => Origin::BodyCenter,
            OriginMode::BodyCorner => Origin::BodyCorner,
            OriginMode::World => Origin::World,
        },
        world_origin: spec.origin,
        shell: spec.shell,
        axis: spec.axis,
        control_points,
        around: None,
        max_samples: (spec.max_samples_millions * 1e6) as usize,
        tolerance: None,
        shape,
    };
    job.check()?;
    Ok(job)
}

/// With the resolution left on auto, coarsen it until the grid fits the sample limit (a
/// plugin run has 30 s), as long as thin walls still get about one sample across.
pub fn fit_resolution(job: &mut Job, lo: [f64; 3], hi: [f64; 3]) -> Result<(), String> {
    if job.resolution.is_some() {
        return Ok(());
    }
    let mut h = job.spacing();
    for _ in 0..40 {
        let g = gyroid::grid::Grid::covering(lo, hi, h, 2);
        if g.len() <= job.max_samples {
            break;
        }
        h *= (g.len() as f64 / job.max_samples as f64).cbrt().max(1.02);
    }
    let thinnest = if job.control_points.is_empty() {
        job.wall
    } else {
        job.control_points.iter().map(|c| c.2).fold(f64::MAX, f64::min)
    };
    if job.fill == Fill::Sheet && h > 0.9 * thinnest {
        return Err(format!(
            "this part is too big for {thinnest} mm walls inside SolidCraft (it would need {h:.2} mm samples); use thicker walls, or gyroid.exe for parts this size"
        ));
    }
    job.resolution = Some(h);
    Ok(())
}

/// The lattice as the gyroid crate meshes it: a TPMS, or struts / a honeycomb as a shape.
pub fn shape(spec: &Spec) -> (Tpms, Fill, Option<Shape>) {
    match spec.lattice {
        Lattice::Tpms(kind, fill) => (kind, fill, None),
        Lattice::Struts(k) => {
            let q = crate::lattice::Q as f64;
            let segments = crate::lattice::unit_struts(k).into_iter().map(|(a, b)| (a.map(|x| x as f64 / q), b.map(|x| x as f64 / q))).collect();
            (Tpms::Gyroid, Fill::Sheet, Some(Shape::Struts { segments, node: spec.node_d, across: spec.variable.then_some(spec.cell) }))
        }
        Lattice::Honeycomb(h) => {
            let t = match h {
                Honeycomb::Hexagon => Tiling::Hexagon,
                Honeycomb::Square => Tiling::Square,
                Honeycomb::Triangle => Tiling::Triangle,
            };
            (Tpms::Gyroid, Fill::Sheet, Some(Shape::Honeycomb(t)))
        }
    }
}

/// The region: the body, or a block of `count` cells from the origin.
pub fn region(spec: &Spec, body: Option<gyroid::stl::Triangles>) -> Region {
    match body {
        Some(t) => Region::Body(t),
        None => {
            let lo = spec.origin;
            Region::Box(lo, [0, 1, 2].map(|a| lo[a] + spec.cell * spec.counts[a] as f64))
        }
    }
}

fn region_bounds(r: &Region) -> ([f64; 3], [f64; 3]) {
    match r {
        Region::Body(t) => gyroid::body::bounds(t),
        Region::Box(lo, hi) => (*lo, *hi),
    }
}

/// What `estimate` reports: the grid, without meshing.
pub fn estimate(spec: &Spec, job: &Job, region: &Region) -> Value {
    let (lo, hi) = region_bounds(region);
    let h = job.spacing();
    let g = gyroid::grid::Grid::covering(lo, hi, h, 2);
    json!({
        "lattice": "tpms",
        "resolution_mm": (h * 1e4).round() / 1e4,
        "samples_millions": (g.len() as f64 / 1e4).round() / 100.0,
        "sample_limit_millions": spec.max_samples_millions,
        "fits": g.len() <= job.max_samples,
        "region_min_mm": lo,
        "region_max_mm": hi,
    })
}

fn round3(x: f32) -> f64 {
    let r = (x as f64 * 1000.0).round() / 1000.0;
    if r == 0.0 { 0.0 } else { r }
}

/// The `timeline.add` arguments for one slab.
fn feature(name: &str, m: &gyroid::mesh::Mesh) -> Value {
    let positions: Vec<f64> = m.verts.iter().flat_map(|v| v.map(round3)).collect();
    let triangles: Vec<u32> = m.tris.iter().flatten().copied().collect();
    json!({"feature": {"type": "mesh_import", "name": name, "file": "Lattice Generator (TPMS)",
                       "meshes": [{"name": name, "positions": positions, "triangles": triangles}]}})
}

/// Build the lattice and add it as mesh bodies.
pub fn generate(spec: &Spec, job: &Job, region: &Region, engine: &mut dyn Engine) -> Result<Value, String> {
    engine.progress(0.02, "Sampling the lattice");
    let sampled = gyroid::sample(job, region, |m| {
        let _ = m;
    })?;
    if engine.cancelled() {
        return Err("cancelled".into());
    }
    let g = &sampled.grid;
    // Slab along the longest axis.
    let axis = (0..3).max_by_key(|&a| g.n[a]).unwrap_or(2);
    let n = g.n[axis];
    let tol = sampled.default_tolerance();
    let mut slabs: Vec<(usize, usize, Value)> = Vec::new();
    // Start thin and grow: re-meshing an oversized slab wastes more than a few extra calls.
    let (mut s0, mut step) = (0usize, (n / 12).max(2));
    let mut triangles = 0usize;
    // Bytes per raw (unsimplified) triangle, learnt from the slabs so far: checks a slab's size
    // before the costly simplification, since density can change a lot along the axis.
    let mut per_raw: Option<f64> = None;
    while s0 < n - 1 {
        let s1 = (s0 + step).min(n - 1);
        engine.progress(0.05 + 0.85 * s0 as f32 / n as f32, &format!("Meshing slab {} ({s0}..{s1} of {n})", slabs.len() + 1));
        if engine.cancelled() {
            return Err("cancelled".into());
        }
        let mut m = sampled.slab(axis, s0, s1);
        if m.tris.is_empty() {
            s0 = s1;
            continue;
        }
        if let Some(r) = per_raw {
            let guess = r * m.tris.len() as f64;
            if guess > 0.95 * MAX_CALL_BYTES as f64 && s1 - s0 > 1 {
                step = ((step as f64 * 0.8 * MAX_CALL_BYTES as f64 / guess) as usize).clamp(1, step - 1);
                continue;
            }
        }
        let raw = m.tris.len();
        gyroid::simplify::simplify(&mut m, tol);
        let name = format!("{} {}", spec.body_name, slabs.len() + 1);
        let args = feature(&name, &m);
        let bytes = args.to_string().len();
        per_raw = Some(bytes as f64 / raw as f64);
        if bytes > MAX_CALL_BYTES && s1 - s0 > 1 {
            // Too big for one call: a thinner slab.
            step = ((step as f64 * 0.8 * MAX_CALL_BYTES as f64 / bytes as f64) as usize).clamp(1, step - 1);
            continue;
        }
        if bytes > MAX_CALL_BYTES {
            return Err(format!(
                "one sample layer of the lattice is {bytes} bytes, more than a plugin call carries; use a coarser resolution"
            ));
        }
        triangles += m.tris.len();
        slabs.push((s0, s1, args));
        if slabs.len() > MAX_SLABS {
            return Err(format!(
                "the lattice needs more than {MAX_SLABS} mesh pieces; use a coarser resolution or bigger cells, or gyroid.exe for parts this size"
            ));
        }
        // Aim the next slab at about 85 % of a call.
        step = ((step as f64 * 0.85 * MAX_CALL_BYTES as f64 / bytes as f64) as usize).clamp(1, n);
        s0 = s1;
    }
    if slabs.is_empty() {
        return Err("the lattice came out empty: is the wall thick enough for the resolution, and the body closed?".into());
    }
    let mut names = Vec::new();
    let total = slabs.len();
    for (i, (_, _, args)) in slabs.into_iter().enumerate() {
        engine.progress(0.9 + 0.1 * i as f32 / total as f32, &format!("Adding mesh {} of {total}", i + 1));
        let reply = engine.run("timeline.add", &args)?;
        if let Some(e) = reply.get("error").and_then(Value::as_str) {
            return Err(format!("the lattice mesh was refused: {e}"));
        }
        names.push(reply.get("name").and_then(Value::as_str).unwrap_or_default().to_string());
    }
    engine.progress(1.0, "Lattice done");
    Ok(json!({
        "lattice": "tpms",
        "mesh_bodies": names.len(),
        "feature_names": names,
        "triangles": triangles,
        "resolution_mm": (g.h * 1e4).round() / 1e4,
        "region_min_mm": sampled.region_lo,
        "region_max_mm": sampled.region_hi,
        "note": "The lattice is a mesh, as in the Fusion add-in. Large lattices arrive as several slabs that overlap by a hair and print as one part. The original body is unchanged: hide it before exporting.",
    }))
}
