//! `cargo xtask bundle-mac`: the macOS application bundle and disk image.
//!
//! Builds `dms` in release mode and writes, under `target/dist/`:
//! - `Data Mining.app`: Info.plist (document types `.str`), the icon made from
//!   `apps/dms/assets/icon.png` (our own drawing, see `crates/ui/tests/icon.rs`), ad-hoc signed
//!   (`codesign -s -`; not notarized: on first launch, right-click the app and choose Open);
//! - `DataMining-<version>.dmg`: the app, a link to /Applications and a short read-me.
//!
//! Uses the system tools `sips`, `iconutil`, `codesign` and `hdiutil` (macOS only).

use crate::{Result, cargo, workspace_root};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const APP_NAME: &str = "Data Mining";
const BUNDLE_ID: &str = "io.github.witold-andelie.dms";
/// Our identifier for the `.str` file type (imported: another application defines the format).
const STR_TYPE_ID: &str = "io.github.witold-andelie.dms.modeler-stream";
const ICON_SIZES: &[u32] = &[16, 32, 128, 256, 512];

pub fn run(_args: &[String]) -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Err("bundle-mac runs on macOS only".into());
    }
    let root = workspace_root()?;
    let version = env!("CARGO_PKG_VERSION");
    cargo(&["build", "--release", "-p", "dms"])?;

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
    copy(&target.join("release").join("dms"), &macos.join("dms"))?;
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
    std::os::unix::fs::symlink("/Applications", staging.join("Applications"))
        .map_err(|e| format!("cannot link /Applications: {e}"))?;
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
The app is signed for this team only (ad-hoc), not notarized by Apple. The first time, open it\n\
with a right-click (or Control-click) on the app and choose Open, then Open again in the\n\
dialog. After that it opens normally, and double-clicking a .str stream opens it in the app.\n";

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_plist_declares_the_str_document_type() {
        let plist = info_plist("1.2.3");
        assert!(plist.contains("<string>str</string>"));
        assert!(plist.contains(STR_TYPE_ID));
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
}
