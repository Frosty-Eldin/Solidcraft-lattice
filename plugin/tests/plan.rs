//! Replays the commands the plugin sends through a small model of SolidCraft's feature
//! patterns, then checks the lattice they build is exactly the de-duplicated tiling, with every
//! strut at its graded thickness.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use solidcraft_lattice::layout::{Lattice, Layout, Spec};
use solidcraft_lattice::{Failure, lattice, plan, run_entry};

type Seg = (lattice::P, lattice::P);
/// A mesh `timeline.add` received: positions, triangles, the call's size in bytes.
type ReceivedMesh = (Vec<[f64; 3]>, Vec<[u32; 3]>, usize);

/// What a feature puts in the model: cylinders (base, end, radius) and spheres (centre, radius).
#[derive(Clone, Default)]
struct Items {
    cyl: Vec<([f64; 3], [f64; 3], f64)>,
    sph: Vec<([f64; 3], f64)>,
}
impl Items {
    fn moved(&self, t: [f64; 3]) -> Items {
        let m = |p: [f64; 3]| [p[0] + t[0], p[1] + t[1], p[2] + t[2]];
        Items {
            cyl: self.cyl.iter().map(|(a, b, r)| (m(*a), m(*b), *r)).collect(),
            sph: self.sph.iter().map(|(c, r)| (m(*c), *r)).collect(),
        }
    }
    fn extend(&mut self, o: Items) {
        self.cyl.extend(o.cyl);
        self.sph.extend(o.sph);
    }
}

/// A model of the app: primitives make items; a rectangular feature pattern makes copies of its
/// sources at every non-identity offset, a source that is itself a pattern standing for the
/// copies it made (SolidCraft #395); `inspect.measure` knows the bodies in `boxes`.
#[derive(Default)]
struct Mock {
    calls: Vec<(String, Value)>,
    features: BTreeMap<String, Items>,
    order: Vec<String>,
    bodies: BTreeSet<String>,
    boxes: BTreeMap<String, ([f64; 3], [f64; 3])>,
    clip: Option<([f64; 3], [f64; 3])>,
    taken_body: Option<String>,
    /// What `selection.get` returns as its items.
    selected: Vec<Value>,
    /// The last Cell Grid's arguments, and the sketch rectangles drawn.
    cell_grid: Option<Value>,
    sketches: Vec<(Value, Vec<Value>)>,
    /// `model.mesh` refuses tolerances below this (as a too-detailed body would be refused).
    mesh_min_tolerance: f64,
    /// The meshes `timeline.add` received: (positions, triangles), with each call's size.
    meshes: Vec<ReceivedMesh>,
    inner_copies: BTreeMap<String, usize>,
    next_inner: Option<usize>,
}

fn v3(v: &Value) -> [f64; 3] {
    let a = v.as_array().expect("array");
    [0, 1, 2].map(|i| a[i].as_f64().expect("number"))
}

impl plan::Engine for Mock {
    fn run(&mut self, command: &str, args: &Value) -> Result<Value, String> {
        self.calls.push((command.into(), args.clone()));
        match command {
            "selection.get" => return Ok(json!({"count": self.selected.len(), "items": self.selected})),
            "sketch.create" => {
                self.sketches.push((args["plane"].clone(), vec![]));
                return Ok(json!({"sketch": 40 + self.sketches.len(), "name": format!("Sketch{}", self.sketches.len())}));
            }
            "sketch.rectangle.two_point" => {
                self.sketches.last_mut().expect("rectangle outside a sketch").1.push(args.clone());
                return Ok(json!({"sides": {}, "corners": {}}));
            }
            "sketch.finish" => return Ok(json!({"sketch": 40 + self.sketches.len(), "profiles": 1})),
            "model.mesh" => {
                let name = args["body"].as_str().unwrap();
                if args["tolerance"].as_f64().unwrap() < self.mesh_min_tolerance {
                    return Err("model.mesh failed: more than 8192 vertices".into());
                }
                let (lo, hi) = self.boxes.get(name).ok_or("model.mesh failed: no such body")?;
                let c = |i: usize| [0, 1, 2].map(|a| if i >> a & 1 == 1 { hi[a] } else { lo[a] });
                let positions: Vec<[f64; 3]> = (0..8).map(c).collect();
                // Outward-facing box: -z, +z, -y, +y, -x, +x.
                let triangles = [
                    [0, 2, 3],
                    [0, 3, 1],
                    [4, 5, 7],
                    [4, 7, 6],
                    [0, 1, 5],
                    [0, 5, 4],
                    [2, 6, 7],
                    [2, 7, 3],
                    [0, 4, 6],
                    [0, 6, 2],
                    [1, 3, 7],
                    [1, 7, 5],
                ];
                return Ok(
                    json!({"body": name, "component": 0, "frame": "component-local-mm", "positions": positions, "triangles": triangles}),
                );
            }
            "timeline.add" => {
                let size = args.to_string().len();
                let f = &args["feature"];
                assert_eq!(f["type"], "mesh_import");
                let m = &f["meshes"][0];
                let p: Vec<f64> = m["positions"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
                let t: Vec<u32> = m["triangles"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect();
                self.meshes.push((
                    p.chunks(3).map(|c| [c[0], c[1], c[2]]).collect(),
                    t.chunks(3).map(|c| [c[0], c[1], c[2]]).collect(),
                    size,
                ));
                return Ok(json!({"feature": 100 + self.meshes.len(), "name": f["name"], "error": null, "warning": null}));
            }
            _ => {}
        }
        if command == "inspect.measure" {
            let name = args["bodies"][0].as_str().unwrap();
            let (lo, hi) = self.boxes.get(name).ok_or("inspect.measure failed: no such body")?;
            return Ok(json!({"body_count": 1, "bodies": [{"name": name, "closed": true, "bbox": {"min": lo, "max": hi}}]}));
        }
        let mut created = Vec::new();
        if let Some(t) = args.get("targets") {
            let t = t[0].as_str().unwrap();
            assert!(self.bodies.contains(t) || self.boxes.contains_key(t), "{command} targets missing body {t}");
        }
        let items = match command {
            "solid.cylinder" => {
                let (b, a, h) = (v3(&args["base"]), v3(&args["axis"]), args["height"].as_f64().unwrap());
                assert!(((a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt() - 1.0).abs() < 1e-6);
                assert_eq!(args["operation"], "join");
                Items { cyl: vec![(b, [b[0] + a[0] * h, b[1] + a[1] * h, b[2] + a[2] * h], args["radius"].as_f64().unwrap())], sph: vec![] }
            }
            "solid.sphere" => {
                if args["operation"] == "new" {
                    let mut name = args["body_name"].as_str().unwrap().to_string();
                    if self.taken_body.as_deref() == Some(&name) {
                        name.push('\'');
                    }
                    self.bodies.insert(name.clone());
                    created.push(name);
                    Items::default() // the buried seed adds no shape of its own
                } else {
                    Items { cyl: vec![], sph: vec![(v3(&args["center"]), args["radius"].as_f64().unwrap())] }
                }
            }
            "solid.pattern.rectangular" => {
                let mut dirs = vec![(v3(&args["dir1"]), args["count1"].as_i64().unwrap(), args["spacing1"].as_f64().unwrap())];
                if args.get("dir2").is_some() {
                    dirs.push((v3(&args["dir2"]), args["count2"].as_i64().unwrap(), args["spacing2"].as_f64().unwrap()));
                }
                assert!(dirs.iter().map(|d| d.1).product::<i64>() <= 1000);
                let n2 = dirs.get(1).map_or(1, |d| d.1);
                let mut offsets = Vec::new();
                for i in 0..dirs[0].1 {
                    for j in 0..n2 {
                        if i == 0 && j == 0 {
                            continue;
                        }
                        let mut t = [0.0; 3];
                        for (a, slot) in t.iter_mut().enumerate() {
                            *slot += dirs[0].0[a] * dirs[0].2 * i as f64;
                            if let Some(d) = dirs.get(1) {
                                *slot += d.0[a] * d.2 * j as f64;
                            }
                        }
                        offsets.push(t);
                    }
                }
                let mut out = Items::default();
                for name in args["features"].as_array().unwrap() {
                    // A nested source's copies are copied again: SolidCraft caps those at 1000.
                    if let Some(inner) = self.inner_copies.get(name.as_str().unwrap()) {
                        assert!(offsets.len() * inner <= 1000, "nested pattern too big: {} x {inner}", offsets.len());
                    }
                }
                self.next_inner = Some(offsets.len());
                for name in args["features"].as_array().unwrap() {
                    let src = self.features.get(name.as_str().unwrap()).cloned().ok_or("no such feature")?;
                    for t in &offsets {
                        out.extend(src.moved(*t));
                    }
                }
                out
            }
            "solid.box" if args["operation"] == "new" => {
                let name = args["body_name"].as_str().unwrap().to_string();
                self.bodies.insert(name.clone());
                created.push(name);
                Items::default()
            }
            "solid.cell_grid" => {
                assert_eq!(args["sketch"], json!(40 + self.sketches.len()), "cuts the sketch just drawn");
                let t = args["targets"][0].as_str().unwrap();
                assert!(self.bodies.contains(t) || self.boxes.contains_key(t), "cell grid target {t}");
                self.cell_grid = Some(args.clone());
                Items::default()
            }
            "solid.box" => {
                assert_eq!(args["operation"], "intersect");
                self.clip = Some((
                    v3(&args["corner"]),
                    [args["length"].as_f64().unwrap(), args["width"].as_f64().unwrap(), args["height"].as_f64().unwrap()],
                ));
                Items::default()
            }
            "solid.combine" => {
                assert_eq!(args["operation"], "intersect");
                assert!(self.bodies.contains(args["target"].as_str().unwrap()));
                assert!(self.boxes.contains_key(args["tools"][0].as_str().unwrap()));
                Items::default()
            }
            other => return Err(format!("unexpected command {other}")),
        };
        let name = format!("F{}", self.order.len() + 1);
        if let Some(n) = self.next_inner.take() {
            self.inner_copies.insert(name.clone(), n);
        }
        self.features.insert(name.clone(), items);
        self.order.push(name.clone());
        Ok(
            json!({"feature": self.order.len(), "name": name, "created_bodies": created, "bodies": [], "changed_bodies": [], "removed_bodies": []}),
        )
    }
}

fn part() -> Mock {
    let mut m = Mock::default();
    m.boxes.insert("Part".into(), ([-7.3, 2.0, 0.0], [23.1, 18.0, 31.0]));
    m
}

struct Built {
    mock: Mock,
    out: Value,
    layout: Layout,
    /// Every one-cell strut, quarter units, with its built radius.
    struts: BTreeMap<Seg, f64>,
    nodes: Vec<(lattice::P, f64)>,
}

/// Builds through `mock`, then maps every cylinder back to quarter units (splitting long axis
/// chains into one-cell struts). Panics if any strut is built twice.
fn build(params: Value, mut mock: Mock) -> Built {
    let out = run_entry("generate", &params, &mut mock).unwrap();
    let spec = Spec::parse(&params).unwrap();
    let bbox = spec.body.as_ref().map(|b| mock.boxes[b]);
    let layout = Layout::resolve(&spec, bbox).unwrap();
    let mut all = Items::default();
    for n in &mock.order {
        all.extend(mock.features[n].clone());
    }
    let q = |p: [f64; 3]| {
        [0, 1, 2].map(|b| {
            let v = layout.to_q(b, p[b]);
            assert!((v - v.round()).abs() < 1e-6, "{p:?} is off the lattice on axis {b}: {v}");
            v.round() as i64
        })
    };
    let mut struts = BTreeMap::new();
    for (a, b, r) in &all.cyl {
        let (a, b) = (q(*a), q(*b));
        let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let axis = (0..3).filter(|&i| d[i] != 0).collect::<Vec<_>>();
        let pieces = if axis.len() == 1 && d[axis[0]] % lattice::Q == 0 { d[axis[0]] / lattice::Q } else { 1 };
        let step = d.map(|c| c / pieces);
        for k in 0..pieces {
            let s = [a[0] + step[0] * k, a[1] + step[1] * k, a[2] + step[2] * k];
            let e = [s[0] + step[0], s[1] + step[1], s[2] + step[2]];
            assert!(struts.insert(if s <= e { (s, e) } else { (e, s) }, *r).is_none(), "strut built twice: {s:?}-{e:?}");
        }
    }
    let nodes = all.sph.iter().map(|(c, r)| (q(*c), *r)).collect();
    Built { mock, out, layout, struts, nodes }
}

/// Every strut once, at its exact graded wall thickness rounded to the nearest level; every
/// node `ratio` x the thickest strut it joins, once.
fn check(params: Value, mock: Mock) -> Built {
    let b = build(params.clone(), mock);
    let spec = Spec::parse(&params).unwrap();
    let Lattice::Struts(kind) = spec.lattice else { panic!("not a strut lattice") };
    let want = lattice::tile(kind, b.layout.counts);
    assert_eq!(b.struts.keys().copied().collect::<BTreeSet<_>>(), want, "{params}");
    let n = b.out["thickness_levels"].as_u64().unwrap() as f64;
    let (lo, hi) = b.layout.wall_range();
    let step = if n > 1.0 { (hi - lo) / (n - 1.0) } else { 0.0 };
    let mut at: BTreeMap<lattice::P, f64> = BTreeMap::new();
    for ((s, e), r) in &b.struts {
        let (pa, pb) = (b.layout.pos(*s), b.layout.pos(*e));
        let exact = b.layout.wall_at([0, 1, 2].map(|i| (pa[i] + pb[i]) / 2.0));
        assert!((2.0 * r - exact).abs() <= step / 2.0 + 1e-6, "{params}: strut {s:?}-{e:?} is {} mm, exact {exact}", 2.0 * r);
        for p in [*s, *e] {
            let v = at.entry(p).or_insert(0.0);
            *v = v.max(2.0 * r);
        }
    }
    let mut seen = BTreeSet::new();
    for (c, r) in &b.nodes {
        assert!(seen.insert(*c), "{params}: node {c:?} twice");
        assert!((2.0 * r - spec.node_ratio() * at[c]).abs() < 1e-6, "{params}: node {c:?}");
    }
    if spec.node_d > 0.0 {
        assert_eq!(seen, lattice::nodes(&want), "{params}: nodes");
    } else {
        assert!(seen.is_empty());
    }
    assert!(b.mock.calls.iter().filter(|(c, _)| c != "inspect.measure" && c != "selection.get").count() <= plan::MAX_STEPS);
    assert_eq!(b.out["struts"], want.len());
    b
}

#[test]
fn every_lattice_builds_exactly_its_tiling_once() {
    for kind in lattice::Kind::ALL {
        for counts in [[1, 1, 1], [2, 1, 1], [1, 3, 2], [3, 3, 3], [4, 2, 5]] {
            let params = json!({"lattice": kind, "cell_size": 8, "count_x": counts[0], "count_y": counts[1], "count_z": counts[2],
                                "strut_diameter": 0.8, "node_diameter": 1.2, "origin_x": -3.5, "origin_y": 10, "origin_z": 0.25});
            let b = check(params, Mock::default());
            assert_eq!(b.out["features"], b.mock.calls.len() - 1, "one selection.get, then the features");
        }
    }
}

#[test]
fn feature_count_stays_small_and_independent_of_size() {
    let count = |kind: &str, n: i64, nodes: f64| {
        let p = json!({"lattice": kind, "cell_size": 10, "count_x": n, "count_y": n, "count_z": n, "strut_diameter": 1, "node_diameter": nodes, "max_struts": 100000});
        run_entry("estimate", &p, &mut Mock::default()).unwrap()["features"].as_u64().unwrap()
    };
    for kind in lattice::Kind::ALL {
        assert!(count(kind, 10, 1.5) <= plan::MAX_STEPS as u64, "{kind}");
        assert_eq!(count(kind, 3, 0.0), count(kind, 9, 0.0), "{kind}");
    }
    // Cubic: 1 seed + 3 axis-chain cylinders, each with one pattern over the other two axes.
    assert_eq!(count("cubic", 5, 0.0), 7);
}

#[test]
fn eleven_nodes_per_axis_use_the_one_axis_at_a_time_chain() {
    let params = json!({"lattice": "cubic", "cell_size": 4, "count_x": 10, "count_y": 10, "count_z": 10, "strut_diameter": 0.5, "node_diameter": 1, "max_struts": 100000});
    let b = check(params, Mock::default());
    let patterns = b.mock.calls.iter().filter(|(c, _)| c == "solid.pattern.rectangular").count();
    assert_eq!(patterns, 3 + 3, "3 strut families x 1 pattern + 1 node family x 3 chained patterns");
}

#[test]
fn clip_box_matches_the_envelope_and_comes_last() {
    let b = build(
        json!({"lattice": "octet", "cell_size": 5, "count_x": 2, "count_y": 3, "count_z": 4, "strut_diameter": 0.8, "clip_to_box": true, "origin_x": 1}),
        Mock::default(),
    );
    let (corner, size) = b.mock.clip.unwrap();
    assert_eq!(corner, [1.0, 0.0, 0.0]);
    assert_eq!(size, [10.0, 15.0, 20.0]);
    assert_eq!(b.mock.calls.last().unwrap().0, "solid.box");
}

#[test]
fn joins_follow_a_renamed_body() {
    let mut mock = Mock { taken_body: Some("Lattice".into()), ..Mock::default() };
    let out = run_entry("generate", &json!({"lattice": "bcc", "count_x": 2, "count_y": 2, "count_z": 2}), &mut mock).unwrap();
    assert_eq!(out["body"], "Lattice'");
    assert!(mock.calls.iter().skip(1).filter(|(_, a)| a.get("targets").is_some()).all(|(_, a)| a["targets"][0] == "Lattice'"));
}

#[test]
fn seed_sphere_is_buried_inside_the_first_strut() {
    let mut mock = Mock::default();
    run_entry(
        "generate",
        &json!({"lattice": "diamond", "count_x": 1, "count_y": 1, "count_z": 1, "cell_size": 10, "strut_diameter": 2}),
        &mut mock,
    )
    .unwrap();
    let (c0, a0) = (&mock.calls[1], &mock.calls[2]);
    assert_eq!(c0.0, "solid.sphere");
    let (c, r) = (v3(&c0.1["center"]), c0.1["radius"].as_f64().unwrap());
    let (b, ax, h, rr) = (v3(&a0.1["base"]), v3(&a0.1["axis"]), a0.1["height"].as_f64().unwrap(), a0.1["radius"].as_f64().unwrap());
    let t = (0..3).map(|i| (c[i] - b[i]) * ax[i]).sum::<f64>();
    let off = (0..3).map(|i| (c[i] - b[i] - ax[i] * t).powi(2)).sum::<f64>().sqrt();
    assert!(off < 1e-6 && t - r > 0.0 && t + r < h && r < rr);
}

#[test]
fn estimate_only_measures() {
    let mut mock = part();
    let out = run_entry("estimate", &json!({"lattice": "bcc", "body": "Part", "cell_size": 8, "strut_diameter": 1}), &mut mock).unwrap();
    assert!(mock.calls.iter().all(|(c, _)| c == "inspect.measure" || c == "selection.get"));
    assert!(out["struts"].as_u64().unwrap() > 0);
}

#[test]
fn filling_a_body_centres_the_grid_and_trims_to_the_body() {
    // Part: 30.4 x 16 x 31 mm from (-7.3, 2, 0).
    let p = json!({"lattice": "bcc", "body": "Part", "cell_size": 5, "strut_diameter": 1, "max_struts": 5000});
    let b = check(p, part());
    assert_eq!(b.layout.counts, [7, 4, 7]);
    let (lo, hi) = b.layout.envelope();
    let mid = [(-7.3 + 23.1) / 2.0, 10.0, 15.5];
    for i in 0..3 {
        assert!(((lo[i] + hi[i]) / 2.0 - mid[i]).abs() < 1e-9);
    }
    let last = b.mock.calls.last().unwrap();
    assert_eq!(last.0, "solid.combine");
    assert_eq!(last.1, json!({"target": "Lattice", "tools": ["Part"], "operation": "intersect", "keep_tools": false}));

    // Complete cells only: whole cells inside the box, nothing trimmed.
    let b = check(
        json!({"lattice": "bcc", "body": "Part", "cell_size": 5, "strut_diameter": 1, "complete_cells_only": true, "max_struts": 5000}),
        part(),
    );
    assert_eq!(b.layout.counts, [6, 3, 6]);
    assert!(b.mock.calls.iter().all(|(c, _)| c != "solid.combine"));

    // Corner origin, keeping the body.
    let b = check(
        json!({"lattice": "cubic", "body": "Part", "cell_size": 5, "strut_diameter": 1, "lattice_origin": "body_corner", "keep_body": true}),
        part(),
    );
    assert_eq!(b.layout.envelope().0, [-7.3, 2.0, 0.0]);
    assert_eq!(b.mock.calls.last().unwrap().1["keep_tools"], true);

    // World origin: grid lines at multiples of 5 from (0, 0, 0).
    let b = check(json!({"lattice": "cubic", "body": "Part", "cell_size": 5, "strut_diameter": 1, "lattice_origin": "world"}), part());
    assert_eq!(b.layout.envelope().0, [-10.0, 0.0, 0.0]);
    assert_eq!(b.layout.counts, [7, 4, 7]);
}

#[test]
fn variable_density_follows_the_control_points() {
    // Along Z of the 31 mm part: cells 4 -> 8 mm, walls 0.6 -> 1.6 mm.
    let p = json!({"lattice": "bcc", "body": "Part", "cell_size": 5, "strut_diameter": 1, "variable_density": true, "lattice_axis": "z",
                   "cp_cell_sizes": "4, 8", "cp_wall_thicknesses": "0.6, 1.6", "node_diameter": 1.2, "bands": 6});
    let b = check(p, part());
    let (_, bounds) = b.layout.layers.clone().unwrap();
    assert!((bounds[0] - 0.0).abs() < 1e-9 && (bounds[bounds.len() - 1] - 31.0).abs() < 1e-9, "{bounds:?}");
    let h: Vec<f64> = bounds.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(h.windows(2).all(|w| w[1] > w[0]), "layers grow with the cell size: {h:?}");
    assert!(h[0] > 3.0 && h[h.len() - 1] < 9.0, "{h:?}");
    // Walls thicken with z.
    let mut by_z: BTreeMap<i64, f64> = BTreeMap::new();
    for ((s, e), r) in &b.struts {
        let z = s[2] + e[2];
        let v = by_z.entry(z).or_insert(*r);
        *v = v.max(*r);
    }
    assert!(by_z.values().zip(by_z.values().skip(1)).all(|(a, b)| a <= b), "{by_z:?}");
    assert_eq!(b.out["variable_axis"], "z");
}

#[test]
fn variable_density_three_points_uneven_spacing_and_every_lattice() {
    for kind in lattice::Kind::ALL {
        let p = json!({"lattice": kind, "cell_size": 6, "count_x": 3, "count_y": 2, "count_z": 5, "strut_diameter": 1, "variable_density": true,
                       "lattice_axis": "x", "even_spacing": false, "cp_positions": "0, 5, 18", "cp_cell_sizes": "4, 6, 9", "cp_wall_thicknesses": "0.6, 0.35, 0.5"});
        let b = check(p, Mock::default());
        assert_eq!(b.layout.layers.as_ref().unwrap().0, 0, "{kind}");
    }
}

#[test]
fn uniform_lattices_ignore_control_points() {
    let a = run_entry("estimate", &json!({"lattice": "octet", "cp_wall_thicknesses": "1, 2", "bands": 9}), &mut Mock::default()).unwrap();
    let b = run_entry("estimate", &json!({"lattice": "octet"}), &mut Mock::default()).unwrap();
    assert_eq!(a, b);
    assert_eq!(a["thickness_levels"], 1);
    assert_eq!(a["wall_thicknesses_mm"], json!([1.5]));
}

#[test]
fn too_many_layers_reduce_thickness_levels_or_are_refused() {
    let out = run_entry(
        "estimate",
        &json!({"lattice": "octet", "cell_size": 6, "count_x": 3, "count_y": 3, "count_z": 5, "strut_diameter": 1, "variable_density": true, "cp_wall_thicknesses": "0.8, 2"}),
        &mut Mock::default(),
    )
    .unwrap();
    assert!(out["features"].as_u64().unwrap() as usize <= plan::MAX_STEPS);
    let err = run_entry(
        "estimate",
        &json!({"lattice": "octet", "cell_size": 6, "count_x": 3, "count_y": 3, "count_z": 30, "strut_diameter": 1, "variable_density": true, "cp_cell_sizes": "3, 3", "max_struts": 100000}),
        &mut Mock::default(),
    )
    .unwrap_err();
    assert!(matches!(&err, Failure::Input(m) if m.contains("features") || m.contains("layers")), "{err:?}");
}

#[test]
fn bad_inputs_are_refused_before_any_edit() {
    for p in [
        json!({"lattice": "spiral"}),
        json!({"count_x": 0}),
        json!({"count_y": 51}),
        json!({"cell_size": 10, "strut_diameter": 6}),
        json!({"strut_diameter": 1, "node_diameter": 1.02}),
        json!({"cell_size": -1}),
        json!({"count_x": 10, "count_y": 10, "count_z": 10, "lattice": "octet"}),
        json!({"body_name": ""}),
        json!({"clip_to_box": "yes"}),
        json!({"body": "Nope"}),
        json!({"body": "Part", "cell_size": 40, "complete_cells_only": true}),
        json!({"lattice_origin": "middle"}),
        json!({"variable_density": true, "lattice_axis": "w"}),
        json!({"variable_density": true, "cp_cell_sizes": "4, 5, 6", "cp_wall_thicknesses": "1, 1"}),
        json!({"variable_density": true, "cp_cell_sizes": "4, x"}),
        json!({"variable_density": true, "cp_cell_sizes": "4, 0"}),
        json!({"variable_density": true, "even_spacing": false, "cp_positions": "0, 5, 3"}),
        json!({"variable_density": true, "cp_cell_sizes": "1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1"}),
        json!({"lattice": "cubic", "variable_density": true, "cell_size": 10, "cp_cell_sizes": "10, 1", "strut_diameter": 1.5}),
        json!({"bands": 1}),
    ] {
        let mut mock = part();
        let err = run_entry("generate", &p, &mut mock).unwrap_err();
        assert!(matches!(err, Failure::Input(_)), "{p}: {err:?}");
        assert!(mock.calls.iter().all(|(c, _)| c == "inspect.measure" || c == "selection.get"), "{p}");
    }
}

#[test]
fn a_refused_command_stops_the_run() {
    struct Refuse(usize);
    impl plan::Engine for Refuse {
        fn run(&mut self, c: &str, _: &Value) -> Result<Value, String> {
            self.0 += 1;
            if self.0 == 3 {
                Err(format!("{c} failed: boolean"))
            } else {
                Ok(json!({"name": format!("F{}", self.0), "created_bodies": ["Lattice"]}))
            }
        }
    }
    let mut r = Refuse(0);
    assert!(matches!(run_entry("generate", &json!({}), &mut r), Err(Failure::Command(_))));
    assert_eq!(r.0, 3);
}

#[test]
fn a_selected_body_or_face_is_filled() {
    for item in
        [json!({"kind": "body", "type": "body", "name": "Part"}), json!({"kind": "face", "type": "face", "body": "Part", "index": 3})]
    {
        let mut mock = part();
        mock.selected = vec![json!({"kind": "sketch_point", "type": "sketch_point"}), item.clone()];
        let out = run_entry("generate", &json!({"lattice": "kelvin", "cell_size": 8, "strut_diameter": 1}), &mut mock).unwrap();
        assert_eq!(mock.calls[0].0, "selection.get");
        assert_eq!(mock.calls[1], ("inspect.measure".into(), json!({"bodies": ["Part"]})), "{item}");
        assert_eq!(mock.calls.last().unwrap().1["tools"], json!(["Part"]));
        assert_eq!(out["body"], "Lattice");
    }
    // A typed name wins over the selection; turning the selection off builds a free block.
    let mut mock = part();
    mock.boxes.insert("Other".into(), ([0.0; 3], [10.0; 3]));
    mock.selected = vec![json!({"kind": "body", "type": "body", "name": "Part"})];
    run_entry("generate", &json!({"body": "Other", "cell_size": 5, "strut_diameter": 1}), &mut mock).unwrap();
    assert!(mock.calls.iter().all(|(c, a)| c != "inspect.measure" || a["bodies"][0] == "Other"));
    let mut mock = part();
    mock.selected = vec![json!({"kind": "body", "type": "body", "name": "Part"})];
    run_entry("generate", &json!({"use_selection": false}), &mut mock).unwrap();
    assert!(mock.calls.iter().all(|(c, _)| c != "inspect.measure" && c != "solid.combine"));
}

#[test]
fn every_new_strut_type_fills_a_body_with_variable_density() {
    for kind in ["bccz", "fccz", "octahedron", "kelvin", "rhombic_dodecahedron", "reentrant_3d", "double_arrowhead"] {
        let p = json!({"lattice": kind, "body": "Part", "cell_size": 8, "strut_diameter": 0.5, "variable_density": true,
                       "cp_cell_sizes": "7, 10", "cp_wall_thicknesses": "0.4, 0.7", "node_diameter": 0.6});
        check(p, part());
    }
}

#[test]
fn honeycombs_cut_the_selected_body_with_cell_grid() {
    let mut mock = part();
    mock.selected = vec![json!({"kind": "body", "type": "body", "name": "Part"})];
    let out =
        run_entry("generate", &json!({"lattice": "honeycomb_hex", "cell_size": 4, "strut_diameter": 0.6, "lattice_axis": "z"}), &mut mock)
            .unwrap();
    let cmds: Vec<&str> = mock.calls.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(
        cmds,
        ["selection.get", "inspect.measure", "sketch.create", "sketch.rectangle.two_point", "sketch.finish", "solid.cell_grid"]
    );
    // Sketch on the body's bottom face, rectangle over its whole footprint.
    assert_eq!(mock.sketches[0].0, json!({"origin": [-7.3, 2.0, 0.0], "x_dir": [1.0, 0.0, 0.0], "y_dir": [0.0, 1.0, 0.0]}));
    assert_eq!(mock.sketches[0].1[0], json!({"p0": [0.0, 0.0], "p1": [30.4, 16.0]}));
    let g = mock.cell_grid.unwrap();
    assert_eq!((g["shape"].as_str(), g["size"].as_f64(), g["web"].as_f64()), (Some("hexagon"), Some(3.4), Some(0.6)));
    assert_eq!(g["anchor"], json!([7.9, 10.0, 15.5]));
    assert_eq!(g["targets"], json!(["Part"]));
    assert_eq!(g["clip"], "free");
    assert_eq!(out["body"], "Part");
    assert!(out["honeycomb_cells"].as_u64().unwrap() > 20);

    // Along X, complete cells only, square cells.
    let mut mock = part();
    run_entry("generate", &json!({"lattice": "honeycomb_square", "body": "Part", "cell_size": 4, "strut_diameter": 1, "lattice_axis": "x", "complete_cells_only": true}), &mut mock).unwrap();
    assert_eq!(mock.sketches[0].0["x_dir"], json!([0.0, 1.0, 0.0]));
    assert_eq!(mock.sketches[0].0["y_dir"], json!([0.0, 0.0, 1.0]));
    assert_eq!(mock.sketches[0].1[0]["p1"], json!([16.0, 31.0]));
    assert_eq!(mock.cell_grid.unwrap()["clip"], "whole");
}

#[test]
fn a_honeycomb_without_a_body_cuts_a_new_block() {
    let mut mock = Mock::default();
    let out = run_entry(
        "generate",
        &json!({"lattice": "honeycomb_triangle", "cell_size": 6, "strut_diameter": 0.8, "count_x": 4, "count_y": 3, "count_z": 2}),
        &mut mock,
    )
    .unwrap();
    assert_eq!(
        mock.calls[1],
        (
            "solid.box".into(),
            json!({"corner": [0.0, 0.0, 0.0], "length": 24.0, "width": 18.0, "height": 12.0, "operation": "new", "body_name": "Lattice"})
        )
    );
    assert_eq!(mock.cell_grid.unwrap()["targets"], json!(["Lattice"]));
    assert_eq!(out["body"], "Lattice");
}

#[test]
fn honeycomb_limits_are_refused_before_any_edit() {
    for p in [
        json!({"lattice": "honeycomb_hex", "body": "Part", "cell_size": 1, "strut_diameter": 0.2}),
        json!({"lattice": "honeycomb_hex", "body": "Part", "cell_size": 4, "strut_diameter": 0.5, "variable_density": true}),
        json!({"lattice": "honeycomb_hex", "body": "Part", "cell_size": 4, "strut_diameter": 0.5, "keep_body": true}),
        json!({"lattice": "honeycomb_hex", "body": "Part", "cell_size": 4, "strut_diameter": 4}),
    ] {
        let mut mock = part();
        let err = run_entry("generate", &p, &mut mock).unwrap_err();
        assert!(matches!(err, Failure::Input(_)), "{p}: {err:?}");
        assert!(mock.calls.iter().all(|(c, _)| c == "inspect.measure" || c == "selection.get"), "{p}");
    }
}

/// Each mesh closed and consistently oriented (every directed edge once, its reverse once),
/// and its signed volume.
fn closed_volume(p: &[[f64; 3]], t: &[[u32; 3]]) -> f64 {
    let mut edges = std::collections::HashMap::new();
    for tri in t {
        for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            *edges.entry((a, b)).or_insert(0) += 1;
        }
    }
    assert!(edges.iter().all(|((a, b), n)| *n == 1 && edges.get(&(*b, *a)) == Some(&1)), "mesh not closed");
    t.iter()
        .map(|tri| {
            let [a, b, c] = tri.map(|i| p[i as usize]);
            (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0])) / 6.0
        })
        .sum()
}

fn small_part() -> Mock {
    let mut m = Mock::default();
    m.boxes.insert("Part".into(), ([-5.0, 2.0, 0.0], [15.0, 22.0, 30.0]));
    m.selected = vec![json!({"kind": "body", "type": "body", "name": "Part"})];
    m
}

#[test]
fn a_gyroid_fills_the_selected_body_as_closed_mesh_slabs() {
    let mut mock = small_part();
    let out =
        run_entry("generate", &json!({"lattice": "gyroid", "cell_size": 6, "strut_diameter": 0.8, "resolution": 0.3}), &mut mock).unwrap();
    let cmds: Vec<&str> = mock.calls.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(&cmds[..2], ["selection.get", "model.mesh"]);
    assert!(cmds[2..].iter().all(|c| *c == "timeline.add"));
    assert!(mock.meshes.len() >= 2, "a 20 x 20 x 30 mm gyroid is more than one call: {}", mock.meshes.len());
    let mut vol = 0.0;
    for (p, t, size) in &mock.meshes {
        assert!(*size <= 1_048_576, "call of {size} bytes");
        vol += closed_volume(p, t);
        let bad: Vec<_> = p
            .iter()
            .filter(|v| !((-5.07..=15.07).contains(&v[0]) && (1.93..=22.07).contains(&v[1]) && (-0.07..=30.07).contains(&v[2])))
            .take(5)
            .collect();
        assert!(bad.is_empty(), "{bad:?}");
    }
    // A sheet fills about 3.09 * wall / cell of the body.
    let frac = vol / (20.0 * 20.0 * 30.0);
    let theory = 3.0919 * 0.8 / 6.0;
    assert!((frac / theory - 1.0).abs() < 0.12, "{frac} vs {theory}");
    assert_eq!(out["mesh_bodies"], mock.meshes.len());
    assert_eq!(out["feature_names"][0], "Lattice 1");
}

#[test]
fn every_tpms_type_and_fill_comes_in_closed() {
    for kind in ["gyroid", "radial_gyroid", "schwarz_diamond", "schwarz_primitive", "schoen_iwp", "neovius"] {
        for (fill, wall) in [("sheet", 0.8), ("solid", 3.0)] {
            let mut mock = small_part();
            mock.boxes.insert("Part".into(), ([0.0; 3], [12.0, 12.0, 12.0]));
            run_entry(
                "generate",
                &json!({"lattice": kind, "tpms_type": fill, "cell_size": 6, "strut_diameter": wall, "resolution": 0.4}),
                &mut mock,
            )
            .unwrap();
            assert!(!mock.meshes.is_empty(), "{kind} {fill}");
            for (p, t, _) in &mock.meshes {
                assert!(closed_volume(p, t) > 0.0, "{kind} {fill}");
            }
        }
    }
}

#[test]
fn gyroid_variable_density_shell_and_no_body() {
    let mut mock = small_part();
    run_entry(
        "generate",
        &json!({"lattice": "gyroid", "cell_size": 6, "strut_diameter": 0.8, "resolution": 0.35, "variable_density": true,
                "cp_cell_sizes": "4, 8", "cp_wall_thicknesses": "0.6, 1.2", "shell": 0.8}),
        &mut mock,
    )
    .unwrap();
    for (p, t, _) in &mock.meshes {
        assert!(closed_volume(p, t) > 0.0);
    }
    // No body: a block of cells from the origin.
    let mut mock = Mock::default();
    run_entry(
        "generate",
        &json!({"lattice": "gyroid", "cell_size": 5, "strut_diameter": 0.8, "count_x": 2, "count_y": 2, "count_z": 2, "origin_x": 10}),
        &mut mock,
    )
    .unwrap();
    assert!(mock.calls.iter().all(|(c, _)| c != "model.mesh"));
    let v: Vec<[f64; 3]> = mock.meshes.iter().flat_map(|m| m.0.clone()).collect();
    let bad: Vec<_> = v.iter().filter(|p| !((9.93..=20.07).contains(&p[0]) && (-0.07..=10.07).contains(&p[1]))).take(5).collect();
    assert!(bad.is_empty(), "inside the 10 mm block: {bad:?}");
}

#[test]
fn a_detailed_body_is_read_at_a_coarser_tolerance() {
    let mut mock = small_part();
    mock.mesh_min_tolerance = 0.2;
    run_entry("generate", &json!({"lattice": "gyroid", "cell_size": 8, "strut_diameter": 1, "resolution": 0.5}), &mut mock).unwrap();
    let tols: Vec<f64> = mock.calls.iter().filter(|(c, _)| c == "model.mesh").map(|(_, a)| a["tolerance"].as_f64().unwrap()).collect();
    assert_eq!(tols, [0.02, 0.05, 0.1, 0.2]);
}

#[test]
fn gyroid_estimate_meshes_nothing_and_limits_are_refused_before_any_edit() {
    let mut mock = small_part();
    let out = run_entry("estimate", &json!({"lattice": "gyroid", "cell_size": 5, "strut_diameter": 0.5}), &mut mock).unwrap();
    assert!(mock.calls.iter().all(|(c, _)| c != "timeline.add"));
    // Auto resolution: 0.2 mm would be over the 1.5 million sample limit, so a bit coarser.
    let r = out["resolution_mm"].as_f64().unwrap();
    assert!(r > 0.2 && r < 0.25, "{r}");
    assert!(out["samples_millions"].as_f64().unwrap() <= 1.5);
    assert_eq!(out["fits"], true);
    for p in [
        json!({"lattice": "gyroid", "cell_size": 5, "strut_diameter": 0.5, "max_samples_millions": 0.05}),
        json!({"lattice": "gyroid", "tpms_type": "foam"}),
        json!({"lattice": "gyroid", "resolution": -1}),
        json!({"lattice": "gyroid", "shell": -1}),
    ] {
        let mut mock = small_part();
        let err = run_entry("generate", &p, &mut mock).unwrap_err();
        assert!(matches!(err, Failure::Input(_)), "{p}: {err:?}");
        assert!(mock.calls.iter().all(|(c, _)| c != "timeline.add"), "{p}");
    }
}

#[test]
fn even_spacing_off_without_positions_spaces_points_evenly() {
    let a = run_entry(
        "estimate",
        &json!({"lattice": "bcc", "variable_density": true, "even_spacing": false, "cp_cell_sizes": "8, 12", "cell_size": 10}),
        &mut Mock::default(),
    )
    .unwrap();
    let b = run_entry(
        "estimate",
        &json!({"lattice": "bcc", "variable_density": true, "cp_cell_sizes": "8, 12", "cell_size": 10}),
        &mut Mock::default(),
    )
    .unwrap();
    assert_eq!(a, b);
}

/// What SolidCraft 0.43's dialog sends for each button with its defaults (checked against its
/// form code), run against a selected 20 x 20 x 30 mm body.
#[test]
fn every_button_runs_with_its_dialog_defaults() {
    let sends = [
        (
            "gyroid",
            json!({"cell_size":5.0,"lattice":"gyroid","lattice_origin":"body_center","resolution":0.4,"shell":0.0,"strut_diameter":0.5,"tpms_type":"sheet"}),
        ),
        (
            "gyroid-variable",
            json!({"cp_cell_sizes":"5, 5, 5","cp_wall_thicknesses":"0.5, 0.5, 0.5","even_spacing":true,"lattice":"gyroid","lattice_axis":"z","lattice_origin":"body_center","resolution":0.4,"shell":0.0,"tpms_type":"sheet"}),
        ),
        (
            "struts",
            json!({"cell_size":5.0,"complete_cells_only":false,"keep_body":false,"lattice":"bcc_cubic","lattice_origin":"body_center","node_diameter":0.0,"strut_diameter":0.5}),
        ),
        (
            "struts-variable",
            json!({"cell_size":5.0,"cp_cell_sizes":"5, 5, 5","cp_wall_thicknesses":"1, 1, 1","even_spacing":true,"keep_body":false,"lattice":"bcc_cubic","lattice_axis":"z","lattice_origin":"body_center"}),
        ),
        (
            "honeycomb",
            json!({"cell_size":5.0,"complete_cells_only":false,"lattice":"honeycomb_hex","lattice_axis":"z","lattice_origin":"body_center","strut_diameter":0.5}),
        ),
    ];
    for (entry, p) in sends {
        let mut mock = small_part();
        let out = run_entry(entry, &p, &mut mock).unwrap_or_else(|e| panic!("{entry}: {e:?}"));
        assert!(mock.calls.len() > 2, "{entry}: {out}");
        // Every button now adds closed mesh bodies.
        assert!(out["mesh_bodies"].as_u64().unwrap_or(0) >= 1, "{entry}: {out}");
        let vol: f64 = mock.meshes.iter().map(|(p, t, _)| closed_volume(p, t)).sum();
        assert!(vol > 100.0 && vol < 0.6 * 12000.0, "{entry}: {vol}");
    }
}

#[test]
fn buttons_need_a_body_and_their_own_lattice_family() {
    let err = run_entry("gyroid", &json!({}), &mut Mock::default()).unwrap_err();
    assert!(matches!(&err, Failure::Input(m) if m.contains("select a body")), "{err:?}");
    let err = run_entry("struts", &json!({"lattice": "gyroid"}), &mut small_part()).unwrap_err();
    assert!(matches!(&err, Failure::Input(m) if m.contains("isn't one")), "{err:?}");
    // Variable density follows the button, whatever the params say.
    let fixed = run_entry("struts", &json!({"variable_density": true, "cp_cell_sizes": "4, 8"}), &mut small_part()).unwrap();
    let vd = run_entry("struts-variable", &json!({"cp_cell_sizes": "4, 8", "cp_wall_thicknesses": "0.8, 1.2"}), &mut small_part()).unwrap();
    assert_ne!(fixed["triangles"], vd["triangles"]);
}
