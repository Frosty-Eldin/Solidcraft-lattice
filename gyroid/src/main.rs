use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::time::Instant;

use clap::{Parser, ValueEnum};
use gyroid::field::{Fill, Tpms};
use gyroid::{Job, Origin, Region};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum TypeArg {
    Gyroid,
    RadialGyroid,
    SchwarzDiamond,
    SchwarzPrimitive,
    SchoenIwp,
    Neovius,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum FillArg {
    Sheet,
    Solid,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum OriginArg {
    BodyCenter,
    BodyCorner,
    World,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum AxisArg {
    X,
    Y,
    Z,
}

/// Fill a body with a gyroid (or another TPMS) lattice and write a watertight STL for
/// SolidCraft's Insert Mesh.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Body to fill: an STL exported from SolidCraft (File > Export, STL, the body alone).
    #[arg(long)]
    body: Option<PathBuf>,
    /// No body: fill a box this size (mm), X Y Z, from the origin.
    #[arg(long, num_args = 3, value_names = ["X", "Y", "Z"])]
    size: Option<Vec<f64>>,
    /// Lattice type.
    #[arg(long = "type", value_enum, default_value = "gyroid")]
    kind: TypeArg,
    /// TPMS type: a thin sheet around the surface, or one side filled solid.
    #[arg(long, value_enum, default_value = "sheet")]
    fill: FillArg,
    /// Cell size (mm).
    #[arg(long, default_value_t = 5.0)]
    cell: f64,
    /// Wall thickness (mm). For --fill solid, cell/2 gives 50 % solid.
    #[arg(long, default_value_t = 0.5)]
    wall: f64,
    /// Sample spacing (mm). Default: wall/2.5 or cell/12, whichever is finer.
    #[arg(long)]
    resolution: Option<f64>,
    /// Where the cell grid starts.
    #[arg(long, value_enum, default_value = "body-center")]
    origin: OriginArg,
    /// With --origin world: the point a cell corner sits on.
    #[arg(long, num_args = 3, value_names = ["X", "Y", "Z"], allow_negative_numbers = true)]
    origin_point: Option<Vec<f64>>,
    /// Keep a solid skin this thick (mm) inside the body's surface. 0 = open lattice.
    #[arg(long, default_value_t = 0.0)]
    shell: f64,
    /// Variable-density axis (and the radial gyroid's axis).
    #[arg(long, value_enum, default_value = "z")]
    axis: AxisArg,
    /// Variable density: cell size at each control point, e.g. "5,3,5".
    #[arg(long)]
    cell_sizes: Option<String>,
    /// Variable density: wall thickness at each control point, e.g. "0.5,0.8,0.5".
    #[arg(long)]
    walls: Option<String>,
    /// Variable density: control point positions in mm from the start of the axis. Default:
    /// evenly spaced over the body.
    #[arg(long)]
    positions: Option<String>,
    /// Radial gyroid: cells around the circumference.
    #[arg(long)]
    around: Option<f64>,
    /// Refuse grids above this many million samples (about 12 MB of memory per million).
    #[arg(long, default_value_t = 400.0)]
    max_samples: f64,
    /// Simplify while staying within this distance (mm) of the exact surface. Default: 20 % of
    /// the resolution. 0 = keep every triangle.
    #[arg(long)]
    tolerance: Option<f64>,
    /// Output STL.
    #[arg(short, long, default_value = "gyroid.stl")]
    out: PathBuf,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let a = Args::parse();
    let t0 = Instant::now();
    let region = match (&a.body, &a.size) {
        (Some(p), _) => {
            let bytes = std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
            Region::Body(gyroid::stl::read(&bytes).map_err(|e| format!("{}: {e}", p.display()))?)
        }
        (None, Some(s)) => Region::Box([0.0; 3], [s[0], s[1], s[2]]),
        (None, None) => return Err("give --body FILE.stl or --size X Y Z".into()),
    };
    let axis = match a.axis {
        AxisArg::X => 0,
        AxisArg::Y => 1,
        AxisArg::Z => 2,
    };
    let (lo, hi) = match &region {
        Region::Body(t) => gyroid::body::bounds(t),
        Region::Box(lo, hi) => (*lo, *hi),
    };
    let list = |s: &Option<String>| s.as_deref().map(gyroid::parse_list).transpose().map(Option::unwrap_or_default);
    let control_points =
        gyroid::control_points(&list(&a.cell_sizes)?, &list(&a.walls)?, &list(&a.positions)?, a.cell, a.wall, hi[axis] - lo[axis])?;
    let job = Job {
        kind: match a.kind {
            TypeArg::Gyroid => Tpms::Gyroid,
            TypeArg::RadialGyroid => Tpms::RadialGyroid,
            TypeArg::SchwarzDiamond => Tpms::SchwarzDiamond,
            TypeArg::SchwarzPrimitive => Tpms::SchwarzPrimitive,
            TypeArg::SchoenIwp => Tpms::SchoenIwp,
            TypeArg::Neovius => Tpms::Neovius,
        },
        fill: match a.fill {
            FillArg::Sheet => Fill::Sheet,
            FillArg::Solid => Fill::Solid,
        },
        cell: a.cell,
        wall: a.wall,
        resolution: a.resolution,
        origin: match a.origin {
            OriginArg::BodyCenter => Origin::BodyCenter,
            OriginArg::BodyCorner => Origin::BodyCorner,
            OriginArg::World => Origin::World,
        },
        world_origin: a.origin_point.map(|p| [p[0], p[1], p[2]]).unwrap_or([0.0; 3]),
        shell: a.shell,
        axis,
        control_points,
        around: a.around,
        max_samples: (a.max_samples * 1e6) as usize,
        tolerance: a.tolerance,
        shape: None,
    };
    println!("{:?} {:?}, cell {} mm, wall {} mm, region {:.2?} to {:.2?}", job.kind, job.fill, job.cell, job.wall, lo, hi);
    let out = gyroid::generate(&job, &region, |m| println!("  {m} ({:.1} s)", t0.elapsed().as_secs_f64()))?;
    let m = &out.mesh;
    if m.tris.is_empty() {
        return Err("the lattice came out empty: is the body closed, and the wall thick enough for the resolution?".into());
    }
    let open = m.open_edges();
    if open > 0 {
        println!("warning: {open} edges are not shared by exactly two triangles; the slicer may need to repair the mesh");
    } else {
        println!("  checked: closed, watertight, outward-facing");
    }
    let f = File::create(&a.out).map_err(|e| format!("{}: {e}", a.out.display()))?;
    gyroid::stl::write(&mut BufWriter::new(f), &m.verts, &m.tris, "gyroid lattice").map_err(|e| e.to_string())?;
    println!(
        "wrote {} ({} triangles, {:.1} MB, {:.1} mm3 solid) in {:.1} s",
        a.out.display(),
        m.tris.len(),
        (84 + 50 * m.tris.len()) as f64 / 1e6,
        m.volume(),
        t0.elapsed().as_secs_f64()
    );
    if m.tris.len() > 20_000_000 {
        println!("warning: SolidCraft inserts at most 20 million triangles; use a coarser --resolution");
    }
    Ok(())
}
