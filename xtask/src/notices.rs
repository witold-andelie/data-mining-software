//! The third-party notice of a binary distribution: every Rust dependency that ends up in the
//! shipped programs, with its licence expression and the licence texts its package carries.
//!
//! The dependency set is Cargo's resolve for one target platform, followed along normal
//! dependency edges only from the named workspace packages, so build scripts, proc macros'
//! build-time helpers and dev-dependencies are not listed. Licence files are read from the
//! packages as downloaded (`LICENSE*`, `LICENCE*`, `COPYING*`, `NOTICE*`, `UNLICENSE*`); a text
//! shared by several packages is printed once. A package that ships no licence file is listed
//! with the licence its manifest declares.

use crate::{Result, cargo_program, workspace_root};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::process::Command;

/// Larger licence files are not texts.
const MAX_LICENSE_BYTES: u64 = 256 * 1024;

struct Library {
    name: String,
    version: String,
    license: String,
    repository: String,
    texts: Vec<String>,
}

/// The notice text for the dependencies of `packages` on `platform` (a target triple).
pub fn rust_dependencies(packages: &[&str], platform: &str) -> Result<String> {
    let output = Command::new(cargo_program())
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--filter-platform",
            platform,
        ])
        .current_dir(workspace_root()?)
        .output()
        .map_err(|e| format!("cannot run cargo metadata: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let metadata: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("cargo metadata is not JSON: {e}"))?;
    let libraries = libraries(&metadata, packages)?;
    Ok(render(&libraries, packages, platform))
}

fn libraries(metadata: &Value, packages: &[&str]) -> Result<Vec<Library>> {
    let list = metadata["packages"]
        .as_array()
        .ok_or("cargo metadata lists no packages")?;
    let by_id: HashMap<&str, &Value> = list
        .iter()
        .filter_map(|package| Some((package["id"].as_str()?, package)))
        .collect();
    let nodes = metadata["resolve"]["nodes"]
        .as_array()
        .ok_or("cargo metadata has no resolve graph")?;
    let deps: HashMap<&str, &Vec<Value>> = nodes
        .iter()
        .filter_map(|node| Some((node["id"].as_str()?, node["deps"].as_array()?)))
        .collect();

    let mut pending: Vec<&str> = list
        .iter()
        .filter(|package| {
            package["source"].is_null()
                && package["name"]
                    .as_str()
                    .is_some_and(|name| packages.contains(&name))
        })
        .filter_map(|package| package["id"].as_str())
        .collect();
    if pending.len() != packages.len() {
        return Err(format!(
            "expected the workspace packages {packages:?}, found {}",
            pending.len()
        ));
    }
    let mut seen: BTreeSet<&str> = pending.iter().copied().collect();
    while let Some(id) = pending.pop() {
        for dep in deps.get(id).map(|d| d.as_slice()).unwrap_or_default() {
            let Some(target) = dep["pkg"].as_str() else {
                continue;
            };
            // a normal edge has a dependency kind of null; build and dev edges are skipped
            let normal = dep["dep_kinds"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind["kind"].is_null()));
            if normal && seen.insert(target) {
                pending.push(target);
            }
        }
    }

    let mut libraries = Vec::new();
    for id in seen {
        let Some(package) = by_id.get(id) else {
            continue;
        };
        if package["source"].is_null() {
            continue; // our own crates
        }
        let text = |key: &str| package[key].as_str().unwrap_or_default().to_string();
        let manifest = text("manifest_path");
        let folder = Path::new(&manifest)
            .parent()
            .ok_or_else(|| format!("{manifest} has no folder"))?;
        libraries.push(Library {
            name: text("name"),
            version: text("version"),
            license: package["license"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| "see the licence file".to_string()),
            repository: text("repository"),
            texts: license_texts(folder),
        });
    }
    libraries.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    Ok(libraries)
}

/// Sub-folders of a package that hold licence texts of bundled material (fonts, for example);
/// every `.txt` file in them is a licence text.
const LICENSE_FOLDERS: &[&str] = &["licenses", "license", "licences", "fonts", "third-party"];

/// The licence files of one package folder, in file-name order.
fn license_texts(folder: &Path) -> Vec<String> {
    let mut files = license_files(folder, false);
    if let Ok(entries) = std::fs::read_dir(folder) {
        let mut folders: Vec<_> = entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry.file_type().is_ok_and(|kind| kind.is_dir())
                    && LICENSE_FOLDERS.contains(
                        &entry
                            .file_name()
                            .to_string_lossy()
                            .to_ascii_lowercase()
                            .as_str(),
                    )
            })
            .map(|entry| entry.path())
            .collect();
        folders.sort();
        for folder in folders {
            files.extend(license_files(&folder, true));
        }
    }
    files
        .iter()
        .filter_map(|path| std::fs::read(path).ok())
        .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_string())
        .filter(|text| !text.is_empty())
        .collect()
}

/// The files of one folder that are licence texts: by name, or every `.txt` when `any_text`.
fn license_files(folder: &Path, any_text: bool) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut files: Vec<_> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            let by_name = ["license", "licence", "copying", "notice", "unlicense"]
                .iter()
                .any(|prefix| name.starts_with(prefix));
            (by_name || (any_text && name.ends_with(".txt")))
                && entry
                    .metadata()
                    .is_ok_and(|m| m.is_file() && m.len() <= MAX_LICENSE_BYTES)
        })
        .map(|entry| entry.path())
        .collect();
    files.sort();
    files
}

fn render(libraries: &[Library], packages: &[&str], platform: &str) -> String {
    let system = if platform.contains("windows") {
        "On Windows the programs also use the system libraries of the operating system, which are\n\
         not part of this distribution."
    } else {
        "The programs also use the system libraries of the operating system, which are not part of\n\
         this distribution. The unixODBC and libltdl libraries bundled with the app are covered by\n\
         THIRD-PARTY-NOTICES.txt in this folder."
    };
    let mut out = format!(
        "Third-party libraries in {}\n\
         \n\
         These programs are built from our own source code (see LICENSE) and from the Rust\n\
         libraries listed below, each under the licence shown. The licence texts follow the list.\n\
         {system}\n\
         \n\
         == Libraries ({}) ==\n\n",
        packages.join(" and "),
        libraries.len()
    );
    for library in libraries {
        out.push_str(&format!(
            "{} {}  [{}]\n",
            library.name, library.version, library.license
        ));
        if !library.repository.is_empty() {
            out.push_str(&format!("    {}\n", library.repository));
        }
    }
    out.push_str("\n== Licence texts ==\n");
    let mut shared: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut without: Vec<String> = Vec::new();
    for library in libraries {
        if library.texts.is_empty() {
            without.push(format!(
                "{} {}  [{}]",
                library.name, library.version, library.license
            ));
        }
        for text in &library.texts {
            shared
                .entry(text)
                .or_default()
                .push(format!("{} {}", library.name, library.version));
        }
    }
    for (text, users) in &shared {
        out.push_str(&format!(
            "\n--- applies to: {} ---\n\n{text}\n",
            users.join(", ")
        ));
    }
    if !without.is_empty() {
        out.push_str(
            "\n--- packages that ship no licence file; the licence is the one their manifest declares ---\n\n",
        );
        for line in &without {
            out.push_str(&format!("{line}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The notice follows the platform: the macOS build links Objective-C bindings the Windows
    /// build does not, and both list the licence texts of what they ship.
    #[test]
    fn the_notice_lists_the_libraries_of_the_platform_with_their_licence_texts() {
        let windows = rust_dependencies(&["dms", "dms-cli"], "x86_64-pc-windows-msvc").unwrap();
        let mac = rust_dependencies(&["dms", "dms-cli"], "aarch64-apple-darwin").unwrap();
        for notice in [&windows, &mac] {
            assert!(notice.contains("== Libraries ("), "{notice:.200}");
            assert!(notice.contains("serde "), "serde is a dependency of both");
            assert!(notice.contains("Permission is hereby granted"), "MIT text");
            assert!(notice.contains("Apache License"), "Apache text");
            assert!(
                !notice.contains("dms-types"),
                "our own crates are not listed"
            );
        }
        assert!(mac.contains("objc2 "), "the macOS build uses objc2");
        assert!(!windows.contains("objc2 "), "the Windows build does not");
        assert!(
            windows.contains("windows-sys "),
            "the Windows build uses windows-sys"
        );
        assert!(windows.contains("Windows") && !mac.contains("On Windows the programs"));
    }
}
