"""Writes plugin.json: one dialog per button, each with only its own settings (SolidCraft's plugin
dialog has no tabs or conditional fields, so the buttons play the part of the Fusion add-in's tabs).

Field order: SolidCraft shows the `required` fields first, in that order, then the rest by name.
Every field the dialog can fill is listed in `required` (they all have defaults); the optional
text fields (Body name, control point positions) follow by name.

Run: python gen_manifest.py
"""

import json

TPMS = [("gyroid", "Gyroid"), ("radial_gyroid", "Radial Gyroid"), ("schwarz_diamond", "Schwarz Diamond"),
        ("schwarz_primitive", "Schwarz Primitive"), ("schoen_iwp", "Schoen IWP"), ("neovius", "Neovius")]
STRUTS = [("bcc_cubic", "Body Centered Cubic"), ("bcc", "Body Centered Cubic (Pure)"), ("bccz", "BCC-Z"),
          ("cubic", "Simple Cubic"), ("fcc", "Face Centered Cubic"), ("fccz", "FCC-Z"), ("octet", "Octet Truss"),
          ("octahedron", "Octahedron"), ("kelvin", "Kelvin Cell"), ("diamond", "Diamond"),
          ("rhombic_dodecahedron", "Rhombic Dodecahedron"), ("reentrant_3d", "Re-entrant 3D"),
          ("double_arrowhead", "Double Arrowhead")]
HONEY = [("honeycomb_hex", "Hexagonal"), ("honeycomb_square", "Square"), ("honeycomb_triangle", "Triangular")]


def choice(title, options, default):
    return {"type": "string", "title": title, "default": default, "oneOf": [{"const": c, "title": t} for c, t in options]}


def num(title, default, unit="mm", minimum=None, excl=None, desc=None):
    p = {"type": "number", "title": title, "default": default}
    if unit:
        p["unit"] = unit
    if minimum is not None:
        p["minimum"] = minimum
    if excl is not None:
        p["exclusiveMinimum"] = excl
    if desc:
        p["description"] = desc
    return p


def flag(title, default, desc=None):
    p = {"type": "boolean", "title": title, "default": default}
    if desc:
        p["description"] = desc
    return p


def text(title, default, desc):
    return {"type": "string", "title": title, "default": default, "description": desc}


F = {
    "tpms_type": choice("TPMS Type", [("sheet", "Sheet"), ("solid", "Solid")], "sheet"),
    "cell_size": num("Cell Size", 5, minimum=0.1),
    "wall": num("Wall Thickness", 0.5, excl=0),
    "resolution": num("Resolution", 0, minimum=0, desc="0 = automatic"),
    "lattice_origin": choice("Lattice Origin", [("body_center", "Body Center"), ("body_corner", "Body Corner")], "body_center"),
    "shell": num("Shell", 0, minimum=0, desc="Solid skin on the body's surface; 0 = none"),
    "lattice_axis": choice("Axis", [("x", "X"), ("y", "Y"), ("z", "Z")], "z"),
    "even_spacing": flag("Even Spacing", True),
    "cp_cell_sizes": text("Cell Size at each point", "5, 5, 5", "mm, start to end, 2 to 10 points"),
    "cp_wall_thicknesses": text("Wall Thickness at each point", "0.5, 0.5, 0.5", "mm, one per point"),
    "cp_positions": text("Point positions", "", "mm from the axis start, one per point; used when Even Spacing is off"),
    "complete_cells_only": flag("Complete Cells Only", False),
    "node_diameter": num("Node Diameter", 0, minimum=0, desc="Sphere at each joint; 0 = none"),
    "keep_body": flag("Keep Body", False, "Keep the original body too"),
    "body_name": text("Result name", "Lattice", "Name for the new mesh bodies"),
}


def form(lattice, fields, optional=("body_name",)):
    props = {"lattice": lattice}
    for key in fields:
        src = "wall" if key == "strut_diameter" else key
        props[key] = dict(F[src])
    for key in optional:
        props[key] = F[key]
    return {"type": "object", "required": ["lattice", *fields], "properties": props}


def entry(name, label, menu, schema):
    return {"name": name, "effect": "edit", "input_schema": schema,
            "slots": [{"kind": "ribbon", "name": label}, {"kind": "menu", "name": menu}]}


entries = [
    entry("gyroid", "Gyroid", "Gyroid / TPMS",
          form(choice("Lattice Type", TPMS, "gyroid"),
               ["tpms_type", "cell_size", "strut_diameter", "resolution", "lattice_origin", "shell"])),
    entry("gyroid-variable", "Gyroid VD", "Gyroid / TPMS, Variable Density",
          form(choice("Lattice Type", TPMS, "gyroid"),
               ["tpms_type", "lattice_axis", "even_spacing", "cp_cell_sizes", "cp_wall_thicknesses", "resolution", "lattice_origin", "shell"],
               optional=("body_name", "cp_positions"))),
    entry("struts", "Struts", "Strut Lattice",
          form(choice("Lattice Type", STRUTS, "bcc_cubic"),
               ["cell_size", "strut_diameter", "node_diameter", "lattice_origin"])),
    entry("struts-variable", "Struts VD", "Strut Lattice, Variable Density",
          form(choice("Lattice Type", STRUTS, "bcc_cubic"),
               ["lattice_axis", "even_spacing", "cp_cell_sizes", "cp_wall_thicknesses", "cell_size", "lattice_origin"],
               optional=("body_name", "cp_positions"))),
    entry("honeycomb", "Honeycomb", "Honeycomb",
          form(choice("Honeycomb Type", HONEY, "honeycomb_hex"),
               ["lattice_axis", "cell_size", "strut_diameter", "lattice_origin"])),
]
entries[2]["input_schema"]["properties"]["strut_diameter"].update(title="Strut Diameter", default=1)
# Struts VD: the cell size is across the axis (along it, the control points set it).
entries[3]["input_schema"]["properties"]["cell_size"] = num("Cell Size across the axis", 5, minimum=0.1)
entries[1]["input_schema"]["properties"]["cp_wall_thicknesses"] = F["cp_wall_thicknesses"]
entries[3]["input_schema"]["properties"]["cp_wall_thicknesses"] = text("Strut diameter at each point", "1, 1, 1", "mm, one per point")

manifest = {
    "id": "lattice",
    "name": "Lattice Generator",
    "version": "0.5.2",
    "description": "Gyroid/TPMS, strut and honeycomb lattices that fill the selected body as watertight meshes, with variable density by control points. SolidCraft 0.42+.",
    "capabilities": ["commands@0.1"],
    "runtime": {
        "module": "lattice.wasm",
        "entries": entries,
        "commands": sorted(["inspect.measure", "model.mesh", "selection.get", "sketch.create", "sketch.finish",
                            "sketch.rectangle.two_point", "solid.box", "solid.cell_grid", "solid.combine", "solid.cylinder",
                            "solid.pattern.rectangular", "solid.sphere", "timeline.add"]),
        "limits": {"fuel": 100000000000, "memory_mib": 512, "deadline_ms": 30000},
    },
}
out = json.dumps(manifest, separators=(",", ":")) + "\n"
assert len(out.encode()) <= 16 * 1024, f"manifest is {len(out.encode())} bytes, over 16 KiB"
open("plugin.json", "w").write(out)
print(f"plugin.json: {len(out.encode())} bytes, {len(entries)} buttons")
