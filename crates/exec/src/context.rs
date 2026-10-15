//! What every node can see while running: stream-wide settings and file resolution.

use dms_strfmt::fastinfoset::Element;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Directory recursion bound when searching data directories.
const MAX_SEARCH_DEPTH: usize = 4;

#[derive(Debug, Clone, Default)]
pub struct ExecContext {
    pub files: FileResolver,
    /// The stream's saved properties (`stream/properties`: date and time formats, decimal symbol,
    /// date baselines, ...); see docs/formats/stream-properties.md.
    pub stream_properties: Option<Arc<Element>>,
}

/// Finds data files a stream references. A path is used as-is when it exists; otherwise (for
/// example an absolute path from another machine, or `$CLEO_DEMOS/...`) the file name is looked
/// up, case-insensitively, under the configured data directories. This mirrors how the Modeler
/// oracle relocates data, so both sides read the same files.
#[derive(Debug, Clone, Default)]
pub struct FileResolver {
    pub data_dirs: Vec<PathBuf>,
}

/// How a referenced file was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    AsIs(PathBuf),
    /// Found by file name under a data directory.
    Relocated {
        requested: String,
        found: PathBuf,
    },
}

impl Resolved {
    pub fn path(&self) -> &Path {
        match self {
            Resolved::AsIs(path) => path,
            Resolved::Relocated { found, .. } => found,
        }
    }
}

impl FileResolver {
    pub fn resolve(&self, requested: &str) -> Option<Resolved> {
        let direct = PathBuf::from(requested);
        if !requested.starts_with('$') && direct.is_file() {
            return Some(Resolved::AsIs(direct));
        }
        let name = file_name(requested)?;
        self.data_dirs.iter().find_map(|dir| {
            find_by_name(dir, &name, 0).map(|found| Resolved::Relocated {
                requested: requested.to_string(),
                found,
            })
        })
    }
}

/// The last component of a Windows or POSIX path.
fn file_name(path: &str) -> Option<String> {
    path.rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

fn find_by_name(dir: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    if depth > MAX_SEARCH_DEPTH {
        return None;
    }
    let mut subdirectories = Vec::new();
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            subdirectories.push(path);
        } else if kind.is_file()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
        {
            return Some(path);
        }
    }
    subdirectories.sort();
    subdirectories
        .iter()
        .find_map(|sub| find_by_name(sub, name, depth + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_file_names_from_foreign_paths() {
        assert_eq!(
            file_name(r"F:\PACAKOVA\data\HEART.csv").as_deref(),
            Some("HEART.csv")
        );
        assert_eq!(file_name("$CLEO_DEMOS/DRUG1n").as_deref(), Some("DRUG1n"));
        assert_eq!(file_name("dir/"), None);
    }
}
