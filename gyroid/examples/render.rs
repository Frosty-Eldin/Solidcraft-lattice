//! Preview: render an STL to a PPM with a z-buffer and simple lighting.
//! cargo run --release --example render -- in.stl out.ppm [yaw_deg] [pitch_deg]
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let tris = gyroid::stl::read(&std::fs::read(&a[1]).unwrap()).unwrap();
    let yaw = a.get(3).and_then(|s| s.parse::<f64>().ok()).unwrap_or(35.0).to_radians();
    let pitch = a.get(4).and_then(|s| s.parse::<f64>().ok()).unwrap_or(30.0).to_radians();
    let (w, h) = (1000usize, 1000usize);
    let rot = |p: [f64; 3]| {
        let (x, y) = (p[0] * yaw.cos() - p[1] * yaw.sin(), p[0] * yaw.sin() + p[1] * yaw.cos());
        let (y2, z2) = (y * pitch.cos() - p[2] * pitch.sin(), y * pitch.sin() + p[2] * pitch.cos());
        [x, z2, y2] // screen x, screen y (up), depth (into screen = +)
    };
    let pts: Vec<[[f64; 3]; 3]> = tris.iter().map(|t| t.map(rot)).collect();
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for t in &pts {
        for p in t {
            for i in 0..2 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
    }
    let s = 0.92 * (w as f64) / (hi[0] - lo[0]).max(hi[1] - lo[1]);
    let cx = (lo[0] + hi[0]) / 2.0;
    let cy = (lo[1] + hi[1]) / 2.0;
    let mut zb = vec![f64::MAX; w * h];
    let mut img = vec![[245u8, 245, 242]; w * h];
    let light = {
        let l = [-0.4f64, 0.6, -0.7];
        let n = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
        l.map(|c| c / n)
    };
    for (t, o) in pts.iter().zip(&tris) {
        let sp = t.map(|p| [(p[0] - cx) * s + w as f64 / 2.0, h as f64 / 2.0 - (p[1] - cy) * s, p[2]]);
        let u = [0, 1, 2].map(|i| t[1][i] - t[0][i]);
        let v = [0, 1, 2].map(|i| t[2][i] - t[0][i]);
        let mut n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if l == 0.0 {
            continue;
        }
        n = n.map(|c| c / l);
        let _ = o;
        let shade = (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]).abs();
        let c = (0.25 + 0.75 * shade).min(1.0);
        let col = [(70.0 + 150.0 * c) as u8, (90.0 + 140.0 * c) as u8, (120.0 + 125.0 * c) as u8];
        let x0 = sp.iter().map(|p| p[0]).fold(f64::MAX, f64::min).floor().max(0.0) as usize;
        let x1 = (sp.iter().map(|p| p[0]).fold(f64::MIN, f64::max).ceil() as usize).min(w - 1);
        let y0 = sp.iter().map(|p| p[1]).fold(f64::MAX, f64::min).floor().max(0.0) as usize;
        let y1 = (sp.iter().map(|p| p[1]).fold(f64::MIN, f64::max).ceil() as usize).min(h - 1);
        let d = (sp[1][1] - sp[2][1]) * (sp[0][0] - sp[2][0]) + (sp[2][0] - sp[1][0]) * (sp[0][1] - sp[2][1]);
        if d.abs() < 1e-12 {
            continue;
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let a = ((sp[1][1] - sp[2][1]) * (px - sp[2][0]) + (sp[2][0] - sp[1][0]) * (py - sp[2][1])) / d;
                let b = ((sp[2][1] - sp[0][1]) * (px - sp[2][0]) + (sp[0][0] - sp[2][0]) * (py - sp[2][1])) / d;
                let c3 = 1.0 - a - b;
                if a < 0.0 || b < 0.0 || c3 < 0.0 {
                    continue;
                }
                let z = a * sp[0][2] + b * sp[1][2] + c3 * sp[2][2];
                let i = x + w * y;
                if z < zb[i] {
                    zb[i] = z;
                    img[i] = col;
                }
            }
        }
    }
    let mut out = format!("P6 {w} {h} 255\n").into_bytes();
    for p in img {
        out.extend_from_slice(&p);
    }
    std::fs::write(&a[2], out).unwrap();
}
