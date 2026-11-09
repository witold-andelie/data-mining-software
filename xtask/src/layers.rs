//! Dependency layering: a workspace crate may depend only on crates in strictly lower layers,
//! and nothing below the UI layer may use a GUI toolkit.

use crate::{Result, cargo_program, workspace_root};
use serde_json::Value;
use std::process::Command;

/// Layer of every workspace package. A new crate must be registered here.
const LAYERS: &[(&str, u8)] = &[
    ("dms-strfmt", 0),
    ("dms-sav", 0),
    ("dms-types", 0),
    ("dms-pmml", 0),
    ("dms-stream", 1),
    ("dms-clem", 1),
    ("dms-catalog", 2),
    ("dms-io", 2),
    ("dms-algo", 2),
    ("dms-exec", 3),
    ("dms-engine", 4),
    ("dms-ui", UI_LAYER),
    ("dms-macos-open", UI_LAYER),
    ("dms-cli", APPS),
    ("dms", APPS),
    ("xtask", TOOLS),
];

const UI_LAYER: u8 = 6;
const APPS: u8 = 10;
const TOOLS: u8 = 100;
const GUI_CRATES: &[&str] = &[
    "egui",
    "eframe",
    "egui_extras",
    "egui_plot",
    "egui_kittest",
    "egui-wgpu",
    "egui-winit",
    "egui_glow",
    "egui_tiles",
    "egui_dock",
    "egui-snarl",
    "winit",
    "rfd",
    "muda",
];

pub fn check() -> Result<()> {
    let output = Command::new(cargo_program())
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(workspace_root()?)
        .output()
        .map_err(|e| format!("cannot run cargo metadata: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let metadata: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("cannot parse cargo metadata: {e}"))?;
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("cargo metadata has no packages array")?;

    let mut violations = Vec::new();
    for package in packages {
        let name = package.get("name").and_then(Value::as_str).unwrap_or("?");
        let Some(layer) = layer_of(name) else {
            violations.push(format!("{name}: not registered in xtask/src/layers.rs"));
            continue;
        };
        let dependencies = package
            .get("dependencies")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for dependency in dependencies {
            if dependency.get("kind").and_then(Value::as_str) == Some("dev") {
                continue;
            }
            let dep = dependency
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("?");
            if let Some(dep_layer) = layer_of(dep)
                && dep_layer >= layer
            {
                violations.push(format!(
                    "{name} (L{layer}) depends on {dep} (L{dep_layer}); only lower layers are allowed"
                ));
            }
            if layer < UI_LAYER && GUI_CRATES.contains(&dep) {
                violations.push(format!(
                    "{name} (L{layer}) depends on GUI crate {dep}; GUI belongs in L{UI_LAYER}"
                ));
            }
        }
    }
    if violations.is_empty() {
        println!("xtask layers: {} packages ok", packages.len());
        Ok(())
    } else {
        Err(format!(
            "layering violations:\n  {}",
            violations.join("\n  ")
        ))
    }
}

fn layer_of(package: &str) -> Option<u8> {
    LAYERS
        .iter()
        .find(|(name, _)| *name == package)
        .map(|(_, layer)| *layer)
}
