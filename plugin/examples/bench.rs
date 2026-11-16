//! Times the plugin's TPMS path natively (same single-threaded code as the Wasm build).
use serde_json::{Value, json};
use solidcraft_lattice::{plan::Engine, run_entry};
use std::time::Instant;

struct Fake { t0: Instant, lo: [f64; 3], hi: [f64; 3], adds: usize, bytes: usize }
impl Engine for Fake {
    fn run(&mut self, c: &str, a: &Value) -> Result<Value, String> {
        match c {
            "selection.get" => Ok(json!({"items": [{"kind": "body", "name": "Part"}]})),
            "model.mesh" => {
                let (lo, hi) = (self.lo, self.hi);
                let p: Vec<[f64; 3]> = (0..8).map(|i| [0, 1, 2].map(|k| if i >> k & 1 == 1 { hi[k] } else { lo[k] })).collect();
                Ok(json!({"positions": p, "triangles": [[0,2,3],[0,3,1],[4,5,7],[4,7,6],[0,1,5],[0,5,4],[2,6,7],[2,7,3],[0,4,6],[0,6,2],[1,3,7],[1,7,5]]}))
            }
            "timeline.add" => { self.adds += 1; self.bytes += a.to_string().len(); Ok(json!({"name": format!("L{}", self.adds)})) }
            _ => Err(c.into()),
        }
    }
    fn progress(&mut self, f: f32, m: &str) { eprintln!("{:6.2}s {:4.0}% {m}", self.t0.elapsed().as_secs_f64(), f * 100.0); }
}

fn main() {
    let a: Vec<f64> = std::env::args().skip(1).map(|s| s.parse().unwrap()).collect();
    let (x, y, z) = (a[0], a[1], a[2]);
    let mut e = Fake { t0: Instant::now(), lo: [0.0; 3], hi: [x, y, z], adds: 0, bytes: 0 };
    let p = json!({"cp_cell_sizes": std::env::var("CELLS").unwrap_or("10, 3".into()), "cp_wall_thicknesses": std::env::var("WALLS").unwrap_or("1, 1".into()), "even_spacing": false, "lattice": "gyroid", "lattice_axis": "z", "lattice_origin": "body_center", "resolution": std::env::var("RES").ok().and_then(|r| r.parse::<f64>().ok()).unwrap_or(0.0), "shell": 0.0, "tpms_type": "sheet"});
    let entry = std::env::var("ENTRY").unwrap_or("gyroid-variable".into());
    let p: serde_json::Value = std::env::var("P").ok().map(|s| serde_json::from_str(&s).unwrap()).unwrap_or(p);
    let out = run_entry(&entry, &p, &mut e);
    println!("{:.2}s total, {} meshes, {} bytes: {:?}", e.t0.elapsed().as_secs_f64(), e.adds, e.bytes, out.map(|o| (o["triangles"].clone(), o["resolution_mm"].clone())));
}
