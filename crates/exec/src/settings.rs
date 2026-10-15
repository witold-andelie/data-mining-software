//! Reading node settings from the saved `properties` element.
//!
//! Element names and value encodings per node are documented in `docs/formats/nodes/`. Settings
//! are text: flags are `true`/`false`, integers decimal, reals Java `Double.toString` form.
//! A missing element means "not saved" (`Ok(None)`); malformed text is an error, never a guess.

use crate::error::OperatorError;
use dms_strfmt::fastinfoset::Element;

/// The element at a `/`-separated path below `settings`.
pub fn element<'a>(settings: Option<&'a Element>, path: &str) -> Option<&'a Element> {
    path.split('/')
        .try_fold(settings?, |current, name| current.child(name))
}

/// Text content at `path`; empty elements read as `Some("")`.
pub fn text(settings: Option<&Element>, path: &str) -> Option<String> {
    element(settings, path).map(Element::text)
}

pub fn flag(settings: Option<&Element>, path: &str) -> Result<Option<bool>, OperatorError> {
    match text(settings, path).as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some("true") => Ok(Some(true)),
        Some("false") => Ok(Some(false)),
        Some(other) => Err(OperatorError::Settings(format!(
            "{path}: expected true or false, found {other:?}"
        ))),
    }
}

pub fn integer(settings: Option<&Element>, path: &str) -> Result<Option<i64>, OperatorError> {
    match text(settings, path).as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(digits) => digits.parse().map(Some).map_err(|_| {
            OperatorError::Settings(format!("{path}: expected an integer, found {digits:?}"))
        }),
    }
}

pub fn real(settings: Option<&Element>, path: &str) -> Result<Option<f64>, OperatorError> {
    match text(settings, path).as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(number) => number
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .map(Some)
            .ok_or_else(|| {
                OperatorError::Settings(format!("{path}: expected a number, found {number:?}"))
            }),
    }
}

/// Child elements named `name` of the element at `path` (for example the `rows/row` of a table
/// setting).
pub fn children<'a>(settings: Option<&'a Element>, path: &str, name: &'a str) -> Vec<&'a Element> {
    element(settings, path)
        .map(|parent| {
            parent
                .child_elements()
                .filter(|child| &*child.name.local == name)
                .collect()
        })
        .unwrap_or_default()
}
