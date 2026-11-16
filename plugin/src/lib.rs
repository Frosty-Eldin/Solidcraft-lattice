//! SolidCraft lattice generator: a `craft:plugin@0.1.0` command plugin.
//!
//! Builds strut lattices (cubic, BCC, BCC+cubic, FCC, octet, diamond) as ordinary native
//! features: one seed cylinder (or node sphere) per translation family, copied by
//! rectangular feature patterns, all joined into one body. It can fill a body, and vary cell
//! size and wall thickness along an axis through 2 to 10 control points (as the Fusion
//! Lattice Generator add-in does). The design stays editable and reopens without the plugin.

pub mod lattice;
pub mod layout;
pub mod plan;
pub mod tpms;

use serde_json::Value;

// The bindings only link as a Wasm component (export names hold `:`); `cargo check` on the
// host still type-checks them with `--cfg lattice_check_guest`.
#[cfg(any(target_arch = "wasm32", lattice_check_guest))]
mod guest {
    wit_bindgen::generate!({
        path: "wit",
        world: "command-plugin",
    });

    use super::*;
    use craft::plugin::types::PluginError;
    use craft::plugin::{commands, control, log};

    struct Host;

    impl plan::Engine for Host {
        fn run(&mut self, command: &str, args: &Value) -> Result<Value, String> {
            let reply = commands::run(command, &args.to_string()).map_err(|e| describe(&e))?;
            serde_json::from_str(&reply).map_err(|e| format!("{command} returned bad JSON: {e}"))
        }
        fn progress(&mut self, fraction: f32, message: &str) {
            control::progress(fraction, message);
        }
        fn cancelled(&mut self) -> bool {
            control::cancelled()
        }
    }

    fn describe(e: &PluginError) -> String {
        match e {
            PluginError::PermissionDenied(s) => format!("permission denied: {s}"),
            PluginError::CommandFailed(f) => format!("{} failed: {}", f.command, f.reason),
            PluginError::LimitExceeded(s) => format!("limit exceeded: {s}"),
            PluginError::InvalidManifest(f) | PluginError::InvalidInput(f) => {
                format!("{}: {}", f.field, f.reason)
            }
            PluginError::Trap(s) => format!("trap: {s}"),
            PluginError::Cancelled => "cancelled".into(),
        }
    }

    struct Plugin;

    impl exports::craft::plugin::entry::Guest for Plugin {
        fn execute(entry: String, params: String) -> Result<String, PluginError> {
            let bad = |reason: String| PluginError::InvalidInput(craft::plugin::types::FieldFailure { field: "params".into(), reason });
            let params: Value = serde_json::from_str(&params).map_err(|e| bad(e.to_string()))?;
            let result = super::run_entry(&entry, &params, &mut Host).map_err(|reason| match reason {
                Failure::Input(r) => bad(r),
                Failure::Command(r) if r == "cancelled" => PluginError::Cancelled,
                Failure::Command(r) => {
                    PluginError::CommandFailed(craft::plugin::types::CommandFailure { command: entry.clone(), reason: r })
                }
            })?;
            log::log(log::Level::Info, &format!("lattice {entry}: {result}"));
            Ok(result.to_string())
        }
    }

    export!(Plugin);
}

/// Why an entry failed: bad parameters, or a command the app refused.
#[derive(Debug, PartialEq)]
pub enum Failure {
    Input(String),
    Command(String),
}

/// Dispatch one manifest entry. Shared by the Wasm guest and the native tests.
/// The dialog buttons, like the Fusion add-in's tabs: each shows only its own settings.
/// `(entry, lattice family, variable density, default lattice)`.
const BUTTONS: [(&str, Family, bool, &str); 5] = [
    ("gyroid", Family::Tpms, false, "gyroid"),
    ("gyroid-variable", Family::Tpms, true, "gyroid"),
    ("struts", Family::Struts, false, "bcc_cubic"),
    ("struts-variable", Family::Struts, true, "bcc_cubic"),
    ("honeycomb", Family::Honeycomb, false, "honeycomb_hex"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    Struts,
    Honeycomb,
    Tpms,
}

fn family(l: layout::Lattice) -> Family {
    match l {
        layout::Lattice::Struts(_) => Family::Struts,
        layout::Lattice::Honeycomb(_) => Family::Honeycomb,
        layout::Lattice::Tpms(..) => Family::Tpms,
    }
}

pub fn run_entry(entry: &str, params: &Value, engine: &mut dyn plan::Engine) -> Result<Value, Failure> {
    let button = BUTTONS.iter().find(|b| b.0 == entry);
    if button.is_none() && !matches!(entry, "estimate" | "generate") {
        return Err(Failure::Input(format!("unknown entry `{entry}`")));
    }
    // A button fixes its family and whether density varies, and always fills a body.
    let mut params = params.clone();
    if let Some((_, _, variable, default)) = button {
        let p = params.as_object_mut().ok_or_else(|| Failure::Input("parameters must be an object".into()))?;
        p.insert("variable_density".into(), (*variable).into());
        p.entry("lattice").or_insert_with(|| (*default).into());
    }
    let mut spec = layout::Spec::parse(&params).map_err(Failure::Input)?;
    if let Some((name, fam, ..)) = button
        && family(spec.lattice) != *fam
    {
        return Err(Failure::Input(format!("this lattice type isn't one the `{name}` button builds")));
    }
    if spec.body.is_none() && spec.use_selection {
        spec.body = plan::selected_body(engine).map_err(Failure::Command)?;
    }
    if button.is_some() && spec.body.is_none() {
        return Err(Failure::Input(
            "select a body (or a face of it) in the viewport first, then run this again".into(),
        ));
    }
    // Every button (and any TPMS) is meshed: struts and honeycombs too, from their distance
    // field, which is fast and always closed; the old B-rep strut plan stays for `generate`.
    if button.is_some() || matches!(spec.lattice, layout::Lattice::Tpms(..)) {
        let (kind, fill, shape) = tpms::shape(&spec);
        let tris = match &spec.body {
            Some(b) => Some(tpms::body_triangles(engine, b).map_err(Failure::Input)?),
            None => None,
        };
        let region = tpms::region(&spec, tris);
        let (lo, hi) = match &region {
            gyroid::Region::Body(t) => gyroid::body::bounds(t),
            gyroid::Region::Box(lo, hi) => (*lo, *hi),
        };
        let mut job = tpms::job(&spec, kind, fill, shape, hi[spec.axis] - lo[spec.axis]).map_err(Failure::Input)?;
        tpms::fit_resolution(&mut job, lo, hi).map_err(Failure::Input)?;
        if entry == "estimate" {
            return Ok(tpms::estimate(&spec, &job, &region));
        }
        return tpms::generate(&spec, &job, &region, engine).map_err(|e| {
            if e.contains("samples") || e.contains("resolution") || e.contains("pieces") || e.contains("empty") {
                Failure::Input(e)
            } else {
                Failure::Command(e)
            }
        });
    }
    let bbox = match &spec.body {
        Some(b) => Some(plan::measure_body(engine, b).map_err(|e| Failure::Input(format!("body `{b}`: {e}")))?),
        None => None,
    };
    let layout = layout::Layout::resolve(&spec, bbox).map_err(Failure::Input)?;
    let built = plan::build(&spec, &layout).map_err(Failure::Input)?;
    if entry == "estimate" {
        return Ok(built.summary(&spec, &layout));
    }
    plan::execute(&built, &spec, &layout, engine).map_err(Failure::Command)
}
