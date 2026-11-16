//! Turntable frames of plugin-style lattices (same meshing as the SolidCraft plugin), as PPM files.
//! usage: turntable <sphere|vd|diamond> <out_dir> [frames]
use gyroid::*;
use std::f64::consts::{PI, TAU};
use std::io::Write;

fn sphere(r: f64, n: usize) -> stl::Triangles {
    let p = |i: usize, j: usize| {
        let th = PI * i as f64 / n as f64;
        let ph = TAU * j as f64 / (2 * n) as f64;
        [r * th.sin() * ph.cos(), r * th.sin() * ph.sin(), r * th.cos()]
    };
    let mut t = Vec::new();
    for i in 0..n {
        for j in 0..2 * n {
            let (a, b, c, d) = (p(i, j), p(i + 1, j), p(i + 1, j + 1), p(i, j + 1));
            if i > 0 { t.push([a, b, d]); }
            if i + 1 < n { t.push([b, c, d]); }
        }
    }
    t
}

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

fn job(kind: &str, cell: f64, wall: f64) -> Job {
    Job {
        kind: field::Tpms::parse(kind).unwrap(),
        fill: field::Fill::Sheet,
        cell,
        wall,
        resolution: None,
        origin: Origin::BodyCenter,
        world_origin: [0.0; 3],
        shell: 0.0,
        axis: 2,
        control_points: vec![],
        around: None,
        max_samples: 60_000_000,
        tolerance: None,
        shape: None,
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] { [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]] }
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn norm(a: [f64; 3]) -> [f64; 3] { let l = dot(a, a).sqrt().max(1e-12); [a[0] / l, a[1] / l, a[2] / l] }

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (scene, out) = (a[1].as_str(), a[2].as_str());
    let frames: usize = a.get(3).map_or(96, |s| s.parse().unwrap());
    let (j, region) = match scene {
        "sphere" => (job("gyroid", 6.0, 0.7), Region::Body(sphere(15.0, 48))),
        "vd" => {
            let mut j = job("gyroid", 6.0, 0.8);
            j.control_points = control_points(&[10.0, 6.0, 3.0], &[1.0, 0.8, 0.6], &[], 6.0, 0.8, 50.0).unwrap();
            (j, Region::Body(cylinder(12.0, 0.0, 50.0, 96)))
        }
        _ => (job("diamond", 10.0, 0.8), Region::Box([0.0; 3], [30.0; 3])),
    };
    let o = generate(&j, &region, |m| eprintln!("{m}")).unwrap();
    let m = o.mesh;
    eprintln!("{} triangles", m.tris.len());
    let v: Vec<[f64; 3]> = m.verts.iter().map(|p| p.map(f64::from)).collect();
    let (lo, hi) = (o.region_lo, o.region_hi);
    let c = [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0);
    let rad = (0..3).map(|k| (hi[k] - lo[k]) / 2.0).map(|x| x * x).sum::<f64>().sqrt();
    // Smooth vertex normals.
    let mut vn = vec![[0.0; 3]; v.len()];
    for t in &m.tris {
        let n = cross(sub(v[t[1] as usize], v[t[0] as usize]), sub(v[t[2] as usize], v[t[0] as usize]));
        for &i in t { for k in 0..3 { vn[i as usize][k] += n[k]; } }
    }
    let vn: Vec<[f64; 3]> = vn.into_iter().map(norm).collect();
    let (w, h) = (1600usize, 1200usize);
    let bg_top = [0x2e, 0x34, 0x40].map(|x: u8| x as f64);
    let bg_bot = [0x1c, 0x20, 0x28].map(|x: u8| x as f64);
    std::fs::create_dir_all(out).unwrap();
    let elev: f64 = 22f64.to_radians();
    let dist = rad * match scene { "sphere" => 2.7, "vd" => 3.9, _ => 3.6 };
    let f = (h as f64 / 2.0) / (16f64.to_radians()).tan();
    for fr in 0..frames {
        let az = TAU * fr as f64 / frames as f64 + 0.6;
        let eye = [c[0] + dist * elev.cos() * az.cos(), c[1] + dist * elev.cos() * az.sin(), c[2] + dist * elev.sin()];
        let fwd = norm(sub(c, eye));
        let right = norm(cross(fwd, [0.0, 0.0, 1.0]));
        let up = cross(right, fwd);
        let light = norm([-0.4 * right[0] + 0.7 * up[0] - 0.6 * fwd[0], -0.4 * right[1] + 0.7 * up[1] - 0.6 * fwd[1], -0.4 * right[2] + 0.7 * up[2] - 0.6 * fwd[2]]);
        let proj: Vec<(f64, f64, f64)> = v
            .iter()
            .map(|p| {
                let d = sub(*p, eye);
                let z = dot(d, fwd);
                (w as f64 / 2.0 + f * dot(d, right) / z, h as f64 / 2.0 - f * dot(d, up) / z, z)
            })
            .collect();
        let mut zb = vec![f64::MAX; w * h];
        let mut col = vec![[0f64; 3]; w * h];
        for y in 0..h {
            let t = y as f64 / h as f64;
            for x in 0..w { col[y * w + x] = [0, 1, 2].map(|k| bg_top[k] * (1.0 - t) + bg_bot[k] * t); }
        }
        for t in &m.tris {
            let [a, b, cc] = t.map(|i| proj[i as usize]);
            let area = (b.0 - a.0) * (cc.1 - a.1) - (b.1 - a.1) * (cc.0 - a.0);
            if area.abs() < 1e-12 { continue; }
            let (x0, x1) = (a.0.min(b.0).min(cc.0).floor().max(0.0) as i64, a.0.max(b.0).max(cc.0).ceil().min(w as f64 - 1.0) as i64);
            let (y0, y1) = (a.1.min(b.1).min(cc.1).floor().max(0.0) as i64, a.1.max(b.1).max(cc.1).ceil().min(h as f64 - 1.0) as i64);
            let n = t.map(|i| vn[i as usize]);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                    let w0 = ((b.0 - px) * (cc.1 - py) - (b.1 - py) * (cc.0 - px)) / area;
                    let w1 = ((cc.0 - px) * (a.1 - py) - (cc.1 - py) * (a.0 - px)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 { continue; }
                    let z = w0 * a.2 + w1 * b.2 + w2 * cc.2;
                    let ix = y as usize * w + x as usize;
                    if z >= zb[ix] { continue; }
                    zb[ix] = z;
                    let mut nn = norm([0, 1, 2].map(|k| w0 * n[0][k] + w1 * n[1][k] + w2 * n[2][k]));
                    // Two-sided: face the camera.
                    let view = norm(sub(eye, [0, 1, 2].map(|k| 0.0 + k as f64 * 0.0)));
                    let _ = view;
                    if dot(nn, fwd) > 0.0 { nn = nn.map(|x| -x); }
                    let diff = dot(nn, light).max(0.0);
                    let fill = dot(nn, norm([right[0] * 0.8 - fwd[0] * 0.2, right[1] * 0.8 - fwd[1] * 0.2, right[2] * 0.8 - fwd[2] * 0.2])).max(0.0);
                    let rim = (1.0 + dot(nn, fwd)).powi(3) * 0.25;
                    let hvec = norm(sub(light, fwd));
                    let spec = dot(nn, hvec).max(0.0).powi(40) * 0.35;
                    // Variable-density tint: warm at the bottom, cool at the top.
                    let base = [0.86, 0.88, 0.92];
                    let s = 0.22 + 0.62 * diff + 0.18 * fill + rim;
                    col[ix] = [0, 1, 2].map(|k| ((base[k] * s + spec) * 255.0).min(255.0));
                }
            }
        }
        // Edge darkening from depth discontinuities (ambient-occlusion-ish outline).
        let mut img = col.clone();
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let z = zb[y * w + x];
                if z == f64::MAX { continue; }
                let mut dmax: f64 = 0.0;
                for (dx, dy) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
                    let zz = zb[(y as i64 + dy) as usize * w + (x as i64 + dx) as usize];
                    let d = if zz == f64::MAX { 0.0 } else { zz - z };
                    dmax = dmax.max(d);
                }
                if dmax > rad * 0.02 {
                    let k = (1.0 - (dmax / (rad * 0.3)).min(1.0) * 0.55).max(0.45);
                    img[y * w + x] = col[y * w + x].map(|c| c * k);
                }
            }
        }
        let mut fh = std::io::BufWriter::new(std::fs::File::create(format!("{out}/f{fr:04}.ppm")).unwrap());
        write!(fh, "P6\n{w} {h}\n255\n").unwrap();
        for p in &img { fh.write_all(&p.map(|x| x as u8)).unwrap(); }
        eprintln!("frame {fr}");
    }
}
