use std::f64::consts::{PI, TAU};

use gyroid::field::{Fill, Tpms};
use gyroid::grid::Grid;
use gyroid::{Job, Origin, Region, generate, mesh, simplify, stl};

fn job(kind: Tpms, fill: Fill, cell: f64, wall: f64, h: f64) -> Job {
    Job {
        kind,
        fill,
        cell,
        wall,
        resolution: Some(h),
        origin: Origin::BodyCorner,
        world_origin: [0.0; 3],
        shell: 0.0,
        axis: 2,
        control_points: vec![],
        around: None,
        max_samples: 50_000_000,
        tolerance: Some(0.0),
        shape: None,
    }
}

/// A closed cylinder along Z: radius `r`, from z0 to z1, `n` sides.
fn cylinder(r: f64, z0: f64, z1: f64, n: usize) -> stl::Triangles {
    let p = |i: usize, z: f64| {
        let a = TAU * i as f64 / n as f64;
        [r * a.cos(), r * a.sin(), z]
    };
    let mut t = Vec::new();
    for i in 0..n {
        let j = (i + 1) % n;
        t.push([p(i, z0), p(j, z0), p(j, z1)]);
        t.push([p(i, z0), p(j, z1), p(i, z1)]);
        t.push([[0.0, 0.0, z0], p(j, z0), p(i, z0)]);
        t.push([[0.0, 0.0, z1], p(i, z1), p(j, z1)]);
    }
    t
}

fn closed(m: &mesh::Mesh, what: &str) {
    assert!(!m.tris.is_empty(), "{what}: empty");
    assert_eq!(m.open_edges(), 0, "{what}: not closed and consistently oriented");
    assert!(m.volume() > 0.0, "{what}: triangles face inwards");
}

#[test]
fn a_sphere_field_meshes_closed_with_the_right_volume() {
    let g = Grid::covering([-6.0; 3], [6.0; 3], 0.25, 1);
    let mut f = vec![0f32; g.len()];
    for k in 0..g.n[2] {
        for j in 0..g.n[1] {
            for i in 0..g.n[0] {
                let p = g.point(i, j, k);
                f[g.idx(i, j, k)] = ((p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt() - 5.0) as f32;
            }
        }
    }
    let mut m = mesh::extract(&g, &f);
    closed(&m, "sphere");
    let exact = 4.0 / 3.0 * PI * 125.0;
    assert!((m.volume() / exact - 1.0).abs() < 0.01, "{} vs {exact}", m.volume());
    let before = m.tris.len();
    simplify::simplify(&mut m, 0.02);
    closed(&m, "simplified sphere");
    assert!(m.tris.len() < before / 3, "{before} -> {}", m.tris.len());
    assert!((m.volume() / exact - 1.0).abs() < 0.01);
    // Every vertex still on the sphere.
    for v in &m.verts {
        let r = (v[0] as f64).hypot(v[1] as f64).hypot(v[2] as f64);
        assert!((r - 5.0).abs() < 0.05, "{r}");
    }
}

#[test]
fn every_tpms_sheet_and_solid_is_closed() {
    for name in Tpms::NAMES {
        for fill in [Fill::Sheet, Fill::Solid] {
            let kind = Tpms::parse(name).unwrap();
            let j = Job { tolerance: Some(0.05), ..job(kind, fill, 8.0, if fill == Fill::Sheet { 1.0 } else { 4.0 }, 0.35) };
            let out = generate(&j, &Region::Box([0.0; 3], [16.0, 16.0, 16.0]), |_| {}).unwrap();
            closed(&out.mesh, &format!("{name} {fill:?}"));
        }
    }
}

/// A gyroid has about 3.09 / cell of surface per unit volume, so a thin sheet `w` thick fills
/// about 3.09 w / cell of the space. Measure it in the middle of a big block (the edges cut
/// sheets off).
#[test]
fn sheet_wall_thickness_comes_out_in_millimetres() {
    for (cell, wall) in [(10.0, 0.8), (6.0, 0.5)] {
        let size = 3.0 * cell;
        let j = job(Tpms::Gyroid, Fill::Sheet, cell, wall, wall / 4.0);
        let out = generate(&j, &Region::Box([0.0; 3], [size; 3]), |_| {}).unwrap();
        let frac = out.mesh.volume() / size.powi(3);
        let theory = 3.0919 * wall / cell;
        assert!((frac / theory - 1.0).abs() < 0.08, "cell {cell} wall {wall}: {frac:.4} vs {theory:.4}");
    }
}

#[test]
fn solid_gyroid_at_half_cell_wall_is_half_solid() {
    let j = job(Tpms::Gyroid, Fill::Solid, 8.0, 4.0, 0.25);
    let out = generate(&j, &Region::Box([0.0; 3], [16.0; 3]), |_| {}).unwrap();
    let frac = out.mesh.volume() / 16f64.powi(3);
    assert!((frac - 0.5).abs() < 0.03, "{frac}");
}

#[test]
fn a_body_is_filled_and_nothing_leaves_it() {
    let body = cylinder(12.0, 0.0, 40.0, 96);
    let j = Job { tolerance: None, ..job(Tpms::Gyroid, Fill::Sheet, 5.0, 0.6, 0.2) };
    let out = generate(&j, &Region::Body(body.clone()), |_| {}).unwrap();
    let m = &out.mesh;
    closed(m, "cylinder fill");
    // The 96-gon's inscribed radius is a hair under 12.
    let rin = 12.0 * (PI / 96.0).cos();
    for v in &m.verts {
        let (x, y, z) = (v[0] as f64, v[1] as f64, v[2] as f64);
        assert!(x.hypot(y) <= rin + 0.05 && (-0.05..=40.05).contains(&z), "{v:?} outside the body");
    }
    // About as much solid as the sheet density says.
    let vol = PI * rin * rin * 40.0;
    let frac = m.volume() / vol;
    let theory = 3.0919 * 0.6 / 5.0;
    assert!((frac / theory - 1.0).abs() < 0.12, "{frac} vs {theory}");
}

#[test]
fn a_shell_keeps_a_solid_skin() {
    let body = cylinder(10.0, 0.0, 20.0, 96);
    let open = generate(&job(Tpms::Gyroid, Fill::Sheet, 5.0, 0.6, 0.25), &Region::Body(body.clone()), |_| {}).unwrap();
    let skin = generate(&Job { shell: 1.0, ..job(Tpms::Gyroid, Fill::Sheet, 5.0, 0.6, 0.25) }, &Region::Body(body), |_| {}).unwrap();
    closed(&skin.mesh, "shelled cylinder");
    // A 1 mm skin on a r10 x 20 cylinder is pi*(100-81)*18 + 2*pi*81*1 = 1583 mm3, less the
    // ~37 % the lattice already filled there: about 1000-1200 mm3 more.
    let gained = skin.mesh.volume() - open.mesh.volume();
    assert!((900.0..1400.0).contains(&gained), "{gained}");
}

#[test]
fn variable_density_grows_cells_and_walls_along_the_axis() {
    let mut j = job(Tpms::Gyroid, Fill::Sheet, 5.0, 0.5, 0.2);
    j.control_points = gyroid::control_points(&[4.0, 8.0], &[0.5, 1.2], &[], 5.0, 0.5, 40.0).unwrap();
    let out = generate(&j, &Region::Box([0.0; 3], [20.0, 20.0, 40.0]), |_| {}).unwrap();
    closed(&out.mesh, "graded");
    // Volume in the bottom and top quarters: dense+thin vs sparse+thick. Solid fraction goes
    // as 3.09 w / c: 3.09*0.6/4.5 = 0.41 at the bottom, 3.09*1.1/7.5 = 0.45 at the top; the
    // surface area density drops instead: count triangles' area in each band.
    let area_in = |z0: f64, z1: f64| -> f64 {
        out.mesh
            .tris
            .iter()
            .map(|t| t.map(|i| out.mesh.verts[i as usize].map(f64::from)))
            .filter(|p| p.iter().all(|v| v[2] >= z0 && v[2] <= z1))
            .map(|p| {
                let (u, v) = ([0, 1, 2].map(|i| p[1][i] - p[0][i]), [0, 1, 2].map(|i| p[2][i] - p[0][i]));
                let c = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
                (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt() / 2.0
            })
            .sum()
    };
    let (bottom, top) = (area_in(2.0, 10.0), area_in(30.0, 38.0));
    assert!(bottom > 1.4 * top, "smaller cells have more surface: {bottom} vs {top}");
}

#[test]
fn stl_round_trip() {
    let out = generate(&job(Tpms::SchwarzPrimitive, Fill::Sheet, 6.0, 1.0, 0.4), &Region::Box([0.0; 3], [12.0; 3]), |_| {}).unwrap();
    let mut bytes = Vec::new();
    stl::write(&mut bytes, &out.mesh.verts, &out.mesh.tris, "t").unwrap();
    let back = stl::read(&bytes).unwrap();
    assert_eq!(back.len(), out.mesh.tris.len());
    let ascii = "solid x\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid x\n";
    assert_eq!(stl::read(ascii.as_bytes()).unwrap(), vec![[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]]);
}

#[test]
fn bad_jobs_are_refused() {
    let r = Region::Box([0.0; 3], [10.0; 3]);
    for j in [
        Job { cell: 0.0, ..job(Tpms::Gyroid, Fill::Sheet, 5.0, 0.5, 0.2) },
        Job { wall: -1.0, ..job(Tpms::Gyroid, Fill::Sheet, 5.0, 0.5, 0.2) },
        Job { max_samples: 1000, ..job(Tpms::Gyroid, Fill::Sheet, 5.0, 0.5, 0.2) },
        Job { control_points: vec![(0.0, 5.0, 0.5)], ..job(Tpms::Gyroid, Fill::Sheet, 5.0, 0.5, 0.2) },
        Job { control_points: vec![(5.0, 5.0, 0.5), (1.0, 5.0, 0.5)], ..job(Tpms::Gyroid, Fill::Sheet, 5.0, 0.5, 0.2) },
    ] {
        assert!(generate(&j, &r, |_| {}).is_err());
    }
    assert!(gyroid::control_points(&[4.0, 5.0, 6.0], &[0.5, 0.6], &[], 5.0, 0.5, 10.0).is_err());
}

#[test]
fn slabs_are_closed_and_add_up_to_the_whole() {
    let body = cylinder(10.0, 0.0, 24.0, 64);
    let j = job(Tpms::Gyroid, Fill::Sheet, 5.0, 0.7, 0.3);
    let s = gyroid::sample(&j, &Region::Body(body), |_| {}).unwrap();
    let whole = mesh::extract(&s.grid, &s.field).volume();
    let n = s.grid.n[2];
    let cuts = [0, n / 3, n / 2, n - 1];
    let mut sum = 0.0;
    for w in cuts.windows(2) {
        let (s0, s1) = (w[0], w[1]);
        let mut m = s.slab(2, s0, s1);
        closed(&m, &format!("slab {s0}..{s1}"));
        simplify::simplify(&mut m, s.default_tolerance());
        closed(&m, &format!("simplified slab {s0}..{s1}"));
        sum += m.volume();
    }
    // Neighbours overlap by under a sample: never less than the whole, barely more.
    assert!(sum >= whole * 0.999 && sum <= whole * 1.03, "{sum} vs {whole}");
}

#[test]
fn dual_contouring_is_closed_with_fewer_triangles() {
    // Sphere.
    let g = Grid::covering([-6.0; 3], [6.0; 3], 0.25, 2);
    let mut f = vec![0f32; g.len()];
    for k in 0..g.n[2] {
        for j in 0..g.n[1] {
            for i in 0..g.n[0] {
                let p = g.point(i, j, k);
                f[g.idx(i, j, k)] = ((p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt() - 5.0) as f32;
            }
        }
    }
    let m = gyroid::dual::extract(&g, &f);
    closed(&m, "dual sphere");
    let exact = 4.0 / 3.0 * PI * 125.0;
    assert!((m.volume() / exact - 1.0).abs() < 0.02, "{} vs {exact}", m.volume());
    // Every TPMS, sheet and solid, cut to a cylinder.
    for name in Tpms::NAMES {
        for fill in [Fill::Sheet, Fill::Solid] {
            let j = job(Tpms::parse(name).unwrap(), fill, 6.0, if fill == Fill::Sheet { 0.8 } else { 3.0 }, 0.3);
            let s = gyroid::sample(&j, &Region::Body(cylinder(8.0, 0.0, 14.0, 48)), |_| {}).unwrap();
            let mt = mesh::extract(&s.grid, &s.field);
            let dc = gyroid::dual::extract(&s.grid, &s.field);
            closed(&dc, &format!("dual {name} {fill:?}"));
            assert!(dc.tris.len() * 5 < mt.tris.len() * 2, "{name}: {} vs {}", dc.tris.len(), mt.tris.len());
            assert!((dc.volume() / mt.volume() - 1.0).abs() < 0.04, "{name} {fill:?}: {} vs {}", dc.volume(), mt.volume());
        }
    }
}

#[test]
fn struts_and_honeycombs_are_closed_with_the_expected_volume() {
    use gyroid::field::{Shape, Tiling};
    let cube: Vec<([f64; 3], [f64; 3])> = vec![([0.0; 3], [1.0, 0.0, 0.0]), ([0.0; 3], [0.0, 1.0, 0.0]), ([0.0; 3], [0.0, 0.0, 1.0])];
    let run = |shape: Shape, cell: f64, wall: f64, hi: [f64; 3]| {
        let j = Job { shape: Some(shape), origin: gyroid::Origin::BodyCorner, ..job(Tpms::Gyroid, Fill::Sheet, cell, wall, 0.15) };
        let o = generate(&j, &Region::Box([0.0; 3], hi), |_| {}).unwrap();
        closed(&o.mesh, "shape");
        o.mesh.volume()
    };
    // Simple cubic: 3 struts per cell, less the overlap at the joints.
    let v = run(Shape::Struts { segments: cube.clone(), node: 0.0, across: None }, 5.0, 1.0, [20.0; 3]);
    let struts = 64.0 * 3.0 * PI * 0.25 * 5.0;
    assert!(v < struts && v > 0.8 * struts, "{v} vs {struts}");
    let with_nodes = run(Shape::Struts { segments: cube, node: 2.0, across: None }, 5.0, 1.0, [20.0; 3]);
    assert!(with_nodes > v * 1.05, "{with_nodes} vs {v}");
    for (t, frac, tol) in [(Tiling::Square, 2.0 * 0.12 - 0.0144, 0.08), (Tiling::Hexagon, 0.24, 0.1), (Tiling::Triangle, 3.0 * 0.6 / (5.0 * 3f64.sqrt() / 2.0), 0.15)] {
        let v = run(Shape::Honeycomb(t), 5.0, 0.6, [20.0, 20.0, 10.0]);
        let want = frac * 4000.0;
        assert!((v / want - 1.0).abs() < tol, "{t:?}: {v} vs {want}");
    }
}
