//! `cargo xtask bundle-mac`: the macOS application bundle and disk image.
//!
//! Builds `dms` and `dms-cli` in release mode and writes, under `target/dist/`:
//! - `Data Mining.app`: Info.plist (document types `.str`), the icon made from
//!   `apps/dms/assets/icon.png` (our own drawing, see `crates/ui/tests/icon.rs`), ad-hoc signed
//!   (`codesign -s -`; not notarized: macOS blocks a downloaded copy the first time, see the
//!   read-me below for the steps);
//! - `DataMining-<version>.dmg`: the app, a link to /Applications and a short read-me.
//!
//! Our own repository `LICENSE` is copied into `Data Mining.app/Contents/Resources/LICENSE`
//! and to the disk image's root directory. The Rust libraries in the programs and their licence
//! texts are listed in `Contents/Resources/Licenses/THIRD-PARTY-RUST-LICENSES.txt`.
//!
//! Homebrew libraries the app links (unixODBC's `libodbc` and libtool's `libltdl`, both LGPL) are
//! copied into `Contents/Frameworks` as separate dynamic libraries, so the app starts on a Mac
//! without Homebrew and the libraries stay replaceable; their licence texts go to
//! `Contents/Resources/Licenses`. This is a recorded exception to the permissive-licence rule.
//! Only the formulas in [`BUNDLED_FORMULAS`] may be bundled: any other
//! Homebrew library fails the build.
//!
//! Uses the system tools `sips`, `iconutil`, `otool`, `install_name_tool`, `codesign` and
//! `hdiutil` (macOS only).

use crate::{Result, cargo_release, notices, workspace_root};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const APP_NAME: &str = "Data Mining";
const BUNDLE_ID: &str = "io.github.witold-andelie.dms";
/// Our exported native stream type.
const DMS_TYPE_ID: &str = "io.github.witold-andelie.dms.stream";
/// The `.str` file type, imported (another application defines the format). Apple asks importers
/// to use the defining application's identifier, and Macs with Modeler installed type `.str`
/// files as this one; with an identifier of our own, the open panel greyed those files out.
/// It is an interoperability identifier, like the extension, not IBM content.
const STR_TYPE_ID: &str = "com.ibm.spssmac.str";
const ICON_SIZES: &[u32] = &[16, 32, 128, 256, 512];

/// Homebrew formulas whose libraries may ship inside the app, with the licence file to include
/// (relative to the formula's prefix) and its licence. Approved by the owner 2026-11-15.
const BUNDLED_FORMULAS: &[(&str, &str, &str)] = &[
    ("unixodbc", "COPYING", "LGPL-2.1-or-later"),
    (
        "libtool",
        "share/libtool/COPYING.LIB",
        "LGPL-2.0-or-later (libltdl)",
    ),
];
/// Where linked libraries are rewritten to point.
const FRAMEWORKS_REF: &str = "@executable_path/../Frameworks";
/// Bound on the library graph walked from the executable.
const MAX_LIBRARIES: usize = 32;

/// The disk image's shortcut to /Applications. Symbolic links are created on Unix only; the
/// bundle is built on macOS, and `run` refuses every other system before it gets here.
#[cfg(unix)]
fn link_applications(link: &Path) -> Result<()> {
    std::os::unix::fs::symlink("/Applications", link)
        .map_err(|e| format!("cannot link /Applications: {e}"))
}

#[cfg(not(unix))]
fn link_applications(_link: &Path) -> Result<()> {
    Err("bundle-mac runs on macOS only".into())
}

/// Homebrew installs libraries read-only; `install_name_tool` must rewrite the copy.
#[cfg(unix)]
fn make_writable(path: &Path) -> Result<()> {
    let mut permissions = fs::metadata(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?
        .permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o644);
    fs::set_permissions(path, permissions)
        .map_err(|e| format!("cannot make {} writable: {e}", path.display()))
}

#[cfg(not(unix))]
fn make_writable(_path: &Path) -> Result<()> {
    Err("bundle-mac runs on macOS only".into())
}

pub fn run(_args: &[String]) -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Err("bundle-mac runs on macOS only".into());
    }
    let root = workspace_root()?;
    let license = root.join("LICENSE");
    if !license.is_file() {
        return Err(format!(
            "repository licence file {} is missing",
            license.display()
        ));
    }
    let version = env!("CARGO_PKG_VERSION");
    cargo_release(&["build", "--release", "-p", "dms", "-p", "dms-cli"], &[])?;

    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let dist = target.join("dist");
    let app = dist.join(format!("{APP_NAME}.app"));
    if app.exists() {
        fs::remove_dir_all(&app).map_err(|e| format!("cannot clear {}: {e}", app.display()))?;
    }
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    for dir in [&macos, &resources] {
        fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    copy(&license, &resources.join("LICENSE"))?;
    // The command-line tool ships inside the app and shares its libraries (see the read-me for
    // putting it on the PATH).
    let executables = [macos.join("dms"), macos.join("dms-cli")];
    copy(&target.join("release").join("dms"), &executables[0])?;
    copy(&target.join("release").join("dms-cli"), &executables[1])?;
    bundle_libraries(&executables, &contents)?;
    let licenses = resources.join("Licenses");
    fs::create_dir_all(&licenses)
        .map_err(|e| format!("cannot create {}: {e}", licenses.display()))?;
    write(
        &licenses.join("THIRD-PARTY-RUST-LICENSES.txt"),
        &notices::rust_dependencies(&["dms", "dms-cli"], "aarch64-apple-darwin")?,
    )?;
    write(&contents.join("Info.plist"), &info_plist(version))?;
    write(&contents.join("PkgInfo"), "APPL????")?;
    make_icon(&root.join("apps/dms/assets/icon.png"), &dist, &resources)?;
    tool(
        "codesign",
        &["--force", "--deep", "--sign", "-", &path_text(&app)?],
    )?;
    tool("codesign", &["--verify", "--verbose=2", &path_text(&app)?])?;

    let staging = dist.join("dmg-root");
    if staging.exists() {
        fs::remove_dir_all(&staging)
            .map_err(|e| format!("cannot clear {}: {e}", staging.display()))?;
    }
    fs::create_dir_all(&staging)
        .map_err(|e| format!("cannot create {}: {e}", staging.display()))?;
    tool(
        "ditto",
        &[
            &path_text(&app)?,
            &path_text(&staging.join(format!("{APP_NAME}.app")))?,
        ],
    )?;
    link_applications(&staging.join("Applications"))?;
    copy(&license, &staging.join("LICENSE"))?;
    write(&staging.join("Read Me.txt"), READ_ME)?;
    let dmg = dist.join(format!("DataMining-{version}.dmg"));
    tool(
        "hdiutil",
        &[
            "create",
            "-volname",
            APP_NAME,
            "-srcfolder",
            &path_text(&staging)?,
            "-ov",
            "-format",
            "UDZO",
            &path_text(&dmg)?,
        ],
    )?;
    fs::remove_dir_all(&staging)
        .map_err(|e| format!("cannot remove {}: {e}", staging.display()))?;
    println!("xtask bundle-mac: {}", app.display());
    println!("xtask bundle-mac: {}", dmg.display());
    Ok(())
}

const READ_ME: &str = "Data Mining\n\
\n\
Drag \"Data Mining\" to Applications.\n\
\n\
The app is signed ad hoc and is not notarized, so macOS blocks it the first time. Try to open it\n\
once, then go to System Settings > Privacy & Security, scroll down and click Open Anyway next to\n\
the message about Data Mining, and confirm. After that it opens normally, and double-clicking a\n\
.str stream opens it in the app. (Verified on macOS 26.5.1; right-click > Open no longer\n\
bypasses the check there.)\n\
\n\
The command-line tool dms-cli is inside the app. To use it from Terminal, link it once:\n\
  sudo ln -sf \"/Applications/Data Mining.app/Contents/MacOS/dms-cli\" /usr/local/bin/dms-cli\n\
\n\
Our own licence is in LICENSE on this disk image and in\n\
Data Mining.app/Contents/Resources/LICENSE.\n\
\n\
The Rust libraries in the programs and their licences are listed in\n\
Data Mining.app/Contents/Resources/Licenses/THIRD-PARTY-RUST-LICENSES.txt. The app also includes\n\
the unixODBC and libltdl libraries (LGPL); their licences and sources are in the same folder.\n";

fn info_plist(version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleDisplayName</key>
	<string>{APP_NAME}</string>
	<key>CFBundleExecutable</key>
	<string>dms</string>
	<key>CFBundleIconFile</key>
	<string>AppIcon</string>
	<key>CFBundleIdentifier</key>
	<string>{BUNDLE_ID}</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>{APP_NAME}</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>{version}</string>
	<key>CFBundleVersion</key>
	<string>{version}</string>
	<key>LSApplicationCategoryType</key>
	<string>public.app-category.productivity</string>
	<key>LSMinimumSystemVersion</key>
	<string>11.0</string>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>NSSupportsAutomaticGraphicsSwitching</key>
	<true/>
	<key>CFBundleDocumentTypes</key>
	<array>
		<dict>
			<key>CFBundleTypeName</key>
			<string>Data Mining Stream</string>
			<key>CFBundleTypeRole</key>
			<string>Editor</string>
			<key>LSHandlerRank</key>
			<string>Owner</string>
			<key>LSItemContentTypes</key>
			<array><string>{DMS_TYPE_ID}</string></array>
		</dict>
		<dict>
			<key>CFBundleTypeName</key>
			<string>Modeler Stream</string>
			<key>CFBundleTypeRole</key>
			<string>Viewer</string>
			<key>LSHandlerRank</key>
			<string>Default</string>
			<key>LSItemContentTypes</key>
			<array>
				<string>{STR_TYPE_ID}</string>
			</array>
		</dict>
	</array>
	<key>UTExportedTypeDeclarations</key>
	<array>
		<dict>
			<key>UTTypeIdentifier</key>
			<string>{DMS_TYPE_ID}</string>
			<key>UTTypeDescription</key>
			<string>Data Mining Stream</string>
			<key>UTTypeConformsTo</key>
			<array><string>public.data</string></array>
			<key>UTTypeTagSpecification</key>
			<dict>
				<key>public.filename-extension</key>
				<array><string>dms</string></array>
			</dict>
		</dict>
	</array>
	<key>UTImportedTypeDeclarations</key>
	<array>
		<dict>
			<key>UTTypeIdentifier</key>
			<string>{STR_TYPE_ID}</string>
			<key>UTTypeDescription</key>
			<string>Modeler Stream</string>
			<key>UTTypeConformsTo</key>
			<array>
				<string>public.data</string>
			</array>
			<key>UTTypeTagSpecification</key>
			<dict>
				<key>public.filename-extension</key>
				<array>
					<string>str</string>
				</array>
			</dict>
		</dict>
	</array>
</dict>
</plist>
"#
    )
}

/// `AppIcon.icns` from the 1024-pixel master image, via an iconset of the standard sizes.
fn make_icon(master: &Path, dist: &Path, resources: &Path) -> Result<()> {
    if !master.is_file() {
        return Err(format!(
            "{} is missing: regenerate it with DMS_WRITE_ICON=1 cargo test -p dms-ui --test icon -- --ignored",
            master.display()
        ));
    }
    let iconset = dist.join("AppIcon.iconset");
    if iconset.exists() {
        fs::remove_dir_all(&iconset)
            .map_err(|e| format!("cannot clear {}: {e}", iconset.display()))?;
    }
    fs::create_dir_all(&iconset)
        .map_err(|e| format!("cannot create {}: {e}", iconset.display()))?;
    for &size in ICON_SIZES {
        for (scale, suffix) in [(1, ""), (2, "@2x")] {
            let pixels = (size * scale).to_string();
            let out = iconset.join(format!("icon_{size}x{size}{suffix}.png"));
            tool(
                "sips",
                &[
                    "-z",
                    &pixels,
                    &pixels,
                    &path_text(master)?,
                    "--out",
                    &path_text(&out)?,
                ],
            )?;
        }
    }
    tool(
        "iconutil",
        &[
            "-c",
            "icns",
            &path_text(&iconset)?,
            "-o",
            &path_text(&resources.join("AppIcon.icns"))?,
        ],
    )?;
    fs::remove_dir_all(&iconset).map_err(|e| format!("cannot remove {}: {e}", iconset.display()))
}

fn tool(program: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("cannot run {program}: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{program} {} failed ({}): {}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn copy(from: &Path, to: &Path) -> Result<()> {
    fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| format!("cannot copy {} to {}: {e}", from.display(), to.display()))
}

fn write(path: &Path, text: &str) -> Result<()> {
    fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn path_text(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| format!("{} is not valid UTF-8", path.display()))
}

/// Libraries `binary` links that live outside the system (Homebrew), as written in its load
/// commands.
fn foreign_libraries(binary: &Path) -> Result<Vec<String>> {
    let output = Command::new("otool")
        .arg("-L")
        .arg(binary)
        .output()
        .map_err(|e| format!("cannot run otool: {e}"))?;
    if !output.status.success() {
        return Err(format!("otool -L {} failed", binary.display()));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text
        .lines()
        .skip(1)
        .filter_map(|line| line.trim().split(" (compatibility").next())
        .map(str::to_string)
        .filter(|path| path.starts_with("/opt/homebrew/") || path.starts_with("/usr/local/"))
        .collect())
}

/// The Homebrew formula a library path belongs to (`/opt/homebrew/opt/<formula>/lib/...`).
fn formula_of(path: &str) -> Option<&str> {
    let rest = path
        .strip_prefix("/opt/homebrew/opt/")
        .or_else(|| path.strip_prefix("/usr/local/opt/"))?;
    rest.split('/').next()
}

/// Copies the Homebrew libraries the executables need (transitively) into Frameworks, points
/// every reference at the copies, and adds the licence texts and a notice.
fn bundle_libraries(executables: &[PathBuf], contents: &Path) -> Result<()> {
    let frameworks = contents.join("Frameworks");
    let licenses = contents.join("Resources").join("Licenses");
    // (original path, bundled file name)
    let mut found: Vec<(String, String)> = Vec::new();
    let mut queue = Vec::new();
    for executable in executables {
        queue.extend(foreign_libraries(executable)?);
    }
    while let Some(library) = queue.pop() {
        if found.iter().any(|(path, _)| *path == library) {
            continue;
        }
        if found.len() >= MAX_LIBRARIES {
            return Err("too many linked libraries".into());
        }
        let formula = formula_of(&library)
            .ok_or_else(|| format!("{library} is not inside a Homebrew formula"))?;
        if !BUNDLED_FORMULAS.iter().any(|(name, _, _)| *name == formula) {
            return Err(format!(
                "the app links {library} (Homebrew formula {formula}), which is not approved \
                 for bundling; see BUNDLED_FORMULAS"
            ));
        }
        let name = Path::new(&library)
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| format!("bad library path {library}"))?
            .to_string();
        queue.extend(foreign_libraries(Path::new(&library))?);
        found.push((library, name));
    }
    if found.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(&frameworks)
        .map_err(|e| format!("cannot create {}: {e}", frameworks.display()))?;
    fs::create_dir_all(&licenses)
        .map_err(|e| format!("cannot create {}: {e}", licenses.display()))?;
    let mut binaries = executables.to_vec();
    for (library, name) in &found {
        let copied = frameworks.join(name);
        copy(Path::new(library), &copied)?;
        // Homebrew installs libraries read-only; install_name_tool must rewrite the copy.
        make_writable(&copied)?;
        tool(
            "install_name_tool",
            &[
                "-id",
                &format!("{FRAMEWORKS_REF}/{name}"),
                &path_text(&copied)?,
            ],
        )?;
        binaries.push(copied);
    }
    for binary in &binaries {
        for (library, name) in &found {
            tool(
                "install_name_tool",
                &[
                    "-change",
                    library,
                    &format!("{FRAMEWORKS_REF}/{name}"),
                    &path_text(binary)?,
                ],
            )?;
        }
    }
    // Every reference now points inside the app.
    for binary in &binaries {
        if let Some(left) = foreign_libraries(binary)?.first() {
            return Err(format!("{} still links {left}", binary.display()));
        }
    }
    let mut notice = String::from(
        "Third-party libraries bundled with Data Mining\n\n\
         These libraries are dynamically linked and kept as separate files in\n\
         Contents/Frameworks, so they can be replaced with other builds of the same\n\
         libraries. Their source code is available from the projects below.\n\n",
    );
    let mut formulas: Vec<&str> = found
        .iter()
        .filter_map(|(path, _)| formula_of(path))
        .collect();
    formulas.sort_unstable();
    formulas.dedup();
    for formula in formulas {
        let Some((_, license_file, license)) = BUNDLED_FORMULAS
            .iter()
            .find(|(name, _, _)| *name == formula)
        else {
            continue;
        };
        let prefix = Path::new("/opt/homebrew/opt").join(formula);
        let version = homebrew_version(&prefix).unwrap_or_else(|| "unknown".into());
        copy(
            &prefix.join(license_file),
            &licenses.join(format!(
                "{formula}-{}.txt",
                license.split(' ').next().unwrap_or("license")
            )),
        )?;
        let files: Vec<&str> = found
            .iter()
            .filter(|(path, _)| formula_of(path) == Some(formula))
            .map(|(_, name)| name.as_str())
            .collect();
        notice.push_str(&format!(
            "{formula} {version}: {} ({license})\n  source: {}\n\n",
            files.join(", "),
            match formula {
                "unixodbc" => "https://www.unixodbc.org/",
                "libtool" => "https://www.gnu.org/software/libtool/",
                _ => "https://formulae.brew.sh/",
            }
        ));
    }
    write(&licenses.join("THIRD-PARTY-NOTICES.txt"), &notice)
}

/// The installed version of a Homebrew formula (the Cellar directory its `opt` link names).
fn homebrew_version(prefix: &Path) -> Option<String> {
    let target = fs::read_link(prefix).ok()?;
    Some(target.file_name()?.to_str()?.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_plist_declares_the_str_document_type() {
        let plist = info_plist("1.2.3");
        assert!(plist.contains("<string>str</string>"));
        assert!(plist.contains(STR_TYPE_ID));
        // The identifier .str files carry on a Mac with Modeler; ours made them unselectable.
        assert_eq!(STR_TYPE_ID, "com.ibm.spssmac.str");
        assert!(plist.contains("<key>CFBundleShortVersionString</key>\n\t<string>1.2.3</string>"));
        assert_eq!(
            plist.matches("<dict>").count(),
            plist.matches("</dict>").count()
        );
        assert_eq!(
            plist.matches("<array>").count(),
            plist.matches("</array>").count()
        );
    }

    #[test]
    fn info_plist_exports_native_stream_as_owner_and_editor() {
        let plist = info_plist("1.2.3");
        assert!(plist.contains("<key>UTExportedTypeDeclarations</key>"));
        assert!(plist.contains("<string>io.github.witold-andelie.dms.stream</string>"));
        assert!(plist.contains("<string>dms</string>"));
        assert!(plist.contains("<string>Editor</string>"));
        assert!(plist.contains("<string>Owner</string>"));
        assert!(plist.contains("<string>public.data</string>"));
        assert!(plist.contains("<string>Viewer</string>"));
        assert!(plist.contains("<string>Default</string>"));
    }
}
