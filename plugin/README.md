# plugin/

The SolidCraft plugin: a Rust crate compiled to a `wasm32-unknown-unknown` WebAssembly component
(`craft:plugin@0.1.0`, capability `commands@0.1`). See the [top-level README](../README.md) for
what it does and how to install it.

- `src/lib.rs`: the five button entries and the routing between them
- `src/tpms.rs`: reads the body (`model.mesh`), runs the `gyroid` engine and adds the result as
  `mesh_import` features, split into slabs that each fit in one 1 MiB plugin call
- `src/layout.rs`, `src/lattice.rs`: dialog parsing, control points and unit-cell strut sets
- `src/plan.rs`: an older B-rep strut planner, still used by the legacy `generate` entry
- `gen_manifest.py`: writes `plugin.json` (the dialogs)
- `examples/bench.rs`: times a run natively (`cargo run --release --example bench 25 25 35`)

Build: `build.sh` (Linux/macOS) or `build.cmd` (Windows) writes `dist/lattice.zip`.
The `gyroid` folder must sit next to this one.
