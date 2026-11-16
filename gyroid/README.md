# Gyroid lattice generator (for SolidCraft)

Fills a body with a gyroid, or another TPMS lattice, and writes a watertight STL. You bring it
into SolidCraft with **Insert Mesh**, and it lands exactly where the body is.

The SolidCraft **Lattice Generator** plugin uses this same code (single-threaded) to make TPMS
lattices inside SolidCraft. Use gyroid.exe for parts too big for a 30 s plugin run.

A plugin run is single-threaded and stops after 30 s. This tool has no time limit and uses
every core.

## Workflow

1. In SolidCraft, select the body and choose **Export STL...** to save it as `body.stl`.
   If the export includes other bodies, copy the design and delete them first.
2. Run the generator (examples below).
3. In SolidCraft, choose **Insert Mesh...** and pick `gyroid.stl`. It keeps the body's
   coordinates. It arrives as a mesh body in its own component, ready to print or export.
   You can hide or delete the original body.

## Options (as in the Fusion add-in)

| Option | Default | |
|---|---|---|
| `--body FILE.stl` / `--size X Y Z` | | Body to fill, or a box from the origin |
| `--type` | `gyroid` | `gyroid`, `radial-gyroid`, `schwarz-diamond`, `schwarz-primitive`, `schoen-iwp`, `neovius` |
| `--fill` | `sheet` | TPMS type: `sheet` (a wall around the surface) or `solid` (one side filled; wall = cell/2 is 50 % solid) |
| `--cell` | 5 mm | Cell size |
| `--wall` | 0.5 mm | Wall thickness; comes out true to within a few % |
| `--resolution` | wall/2.5 or cell/12 | Sample spacing |
| `--origin` | `body-center` | `body-center`, `body-corner`, `world` (`--origin-point X Y Z`) |
| `--axis` | `z` | Variable-density axis, and the radial gyroid's axis |
| `--cell-sizes`, `--walls`, `--positions` | | Variable density: 2 to 10 control points, e.g. `--cell-sizes 4,8 --walls 0.6,1.2`. Positions in mm from the axis start; default evenly spaced |
| `--shell` | 0 | Keep a solid skin this thick on the body's surface (ends included) |
| `--tolerance` | 20 % of resolution | Mesh simplification error; 0 = keep every triangle |
| `--around` | auto | Radial gyroid: cells around the circumference |
| `-o` | `gyroid.stl` | Output |

The tool checks every mesh: closed, watertight and facing outward. It prints a warning if not.

## Size and speed

A 25 x 130 mm can with 5 mm cells and 0.5 mm walls takes 17.5 M triangles before
simplification and 2.0 M after (a 98 MB STL). That ran in 103 s on a 2-core machine, and will
be faster on more cores. Thin walls in large parts are millions of triangles whatever tool
makes them. To trade detail for size, raise `--resolution` or `--tolerance`.

## Build (Windows)

```bat
cd gyroid
cargo build --release
```

The program is `target\release\gyroid.exe`. Run `cargo test --release` for the checks.
