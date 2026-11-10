//! What every node can see while running: stream-wide settings and file resolution.

use dms_strfmt::fastinfoset::Element;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Directory recursion bound when searching data directories.
const MAX_SEARCH_DEPTH: usize = 4;

#[derive(Debug, Clone, Default)]
pub struct ExecContext {
    /// The same cooperative flag used by the owning worker job.
    pub cancel: dms_types::CancelToken,
    pub files: FileResolver,
    /// The stream's saved properties (`stream/properties`: date and time formats, decimal symbol,
    /// date baselines, ...); see docs/formats/stream-properties.md.
    pub stream_properties: Option<Arc<Element>>,
    /// The stream file's attachments by ZIP member (`data/0001.dat`, ...): model nuggets keep
    /// their PMML there.
    pub attachments: Arc<HashMap<String, Arc<[u8]>>>,
    /// Overrides the stream's `realDecimalPlaces` (the oracle exports with 15).
    pub decimal_places: Option<usize>,
    /// Where export nodes write: their file name in this directory instead of the path they
    /// were saved with (the oracle redirects exports the same way, into `exports/`).
    pub export_dir: Option<PathBuf>,
}

impl ExecContext {
    /// The file an export node writes for its saved path `configured`.
    pub fn export_path(&self, configured: &str) -> PathBuf {
        match &self.export_dir {
            Some(dir) => dir.join(file_name(configured).unwrap_or_else(|| "export".to_string())),
            None => PathBuf::from(configured),
        }
    }

    /// Decimal places for reals in documents: the override, else the stream's
    /// `realDecimalPlaces`, else Modeler's default 3.
    pub fn decimal_places(&self) -> usize {
        self.decimal_places
            .or_else(|| {
                crate::settings::text(self.stream_properties.as_deref(), "realDecimalPlaces")
                    .and_then(|text| text.trim().parse().ok())
            })
            .unwrap_or(3)
    }

    /// How exports print Date values when the field sets no format of its own: the stream's
    /// `dateFormat` `format27` prints `Jan 1999` (`ibm-demos/broadband_create_models`). Other
    /// stream date formats keep the plain rendering.
    pub fn date_display(&self) -> Option<dms_types::DisplayFormat> {
        match crate::settings::text(self.stream_properties.as_deref(), "dateFormat")
            .as_deref()
            .map(str::trim)
        {
            Some("format27" | "MON YYYY") => Some(dms_types::DisplayFormat::MonthYear),
            _ => None,
        }
    }

    /// The calendar day (days since 1970-01-01) of an instant saved as epoch milliseconds, in
    /// the stream's `timeZone`: `server` (the default) is this machine's zone, as Modeler uses
    /// its server's; `UTC`/`GMT` and IANA names are themselves. Date settings saved from
    /// Modeler's date pickers are such instants (`oracle/misc/misc_ra_*`: 1585605600000 is
    /// 2020-03-31 in the oracle machine's UTC+2).
    pub fn day_of_instant(&self, millis: i64) -> Result<i64, crate::OperatorError> {
        use jiff::tz::TimeZone;
        let bad = crate::OperatorError::Settings;
        let zone = match crate::settings::text(self.stream_properties.as_deref(), "timeZone")
            .as_deref()
            .map(str::trim)
        {
            None | Some("") | Some("server") => TimeZone::system(),
            Some("UTC" | "GMT") => TimeZone::UTC,
            Some(name) => {
                TimeZone::get(name).map_err(|e| bad(format!("stream timeZone {name:?}: {e}")))?
            }
        };
        let instant = jiff::Timestamp::from_millisecond(millis)
            .map_err(|e| bad(format!("saved instant {millis}: {e}")))?;
        let date = instant.to_zoned(zone).date();
        dms_types::calendar::days_from_civil(
            i64::from(date.year()),
            u32::try_from(date.month()).unwrap_or(0),
            u32::try_from(date.day()).unwrap_or(0),
        )
        .ok_or_else(|| bad(format!("saved instant {millis} is out of range")))
    }
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

/// What a stream path names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    File,
    Directory,
}

impl FileResolver {
    pub fn resolve(&self, requested: &str) -> Option<Resolved> {
        self.resolve_entry(requested, EntryKind::File)
    }

    /// A directory a stream references (the XML source reading every file of a directory),
    /// found the same way as a file.
    pub fn resolve_dir(&self, requested: &str) -> Option<Resolved> {
        self.resolve_entry(requested, EntryKind::Directory)
    }

    fn resolve_entry(&self, requested: &str, kind: EntryKind) -> Option<Resolved> {
        let requested_path = requested.trim_end_matches(['/', '\\']);
        let direct = PathBuf::from(requested_path);
        let exists = match kind {
            EntryKind::File => direct.is_file(),
            EntryKind::Directory => direct.is_dir(),
        };
        if !requested.starts_with('$') && exists {
            return Some(Resolved::AsIs(direct));
        }
        let name = file_name(requested_path)?;
        self.data_dirs.iter().find_map(|dir| {
            find_by_name(dir, &name, kind, 0).map(|found| Resolved::Relocated {
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

fn find_by_name(dir: &Path, name: &str, wanted: EntryKind, depth: usize) -> Option<PathBuf> {
    if depth > MAX_SEARCH_DEPTH {
        return None;
    }
    let mut subdirectories = Vec::new();
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let matches = entry
            .file_name()
            .to_str()
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name));
        if kind.is_dir() {
            if matches && wanted == EntryKind::Directory {
                return Some(path);
            }
            subdirectories.push(path);
        } else if kind.is_file() && matches && wanted == EntryKind::File {
            return Some(path);
        }
    }
    subdirectories.sort();
    subdirectories
        .iter()
        .find_map(|sub| find_by_name(sub, name, wanted, depth + 1))
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
