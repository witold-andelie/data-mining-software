//! Settings checks shared by RFM Aggregate and RFM Analysis.

use crate::error::OperatorError;
use crate::settings;
use dms_strfmt::fastinfoset::Element;

pub(crate) fn required_text(s: Option<&Element>, key: &str) -> Result<String, OperatorError> {
    settings::text(s, key)
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| OperatorError::Settings(format!("RFM: missing {key}")))
}

pub(crate) fn reject_unknown(
    s: Option<&Element>,
    prefix: &str,
    known: &[&str],
) -> Result<(), OperatorError> {
    for child in s.into_iter().flat_map(Element::child_elements) {
        let name = child.name.local.as_ref();
        let metadata = [
            "annotation",
            "keywords",
            "lastSavedAs",
            "enabled",
            "fieldMappingEssentialFields",
            "repositoryPath",
            "repositoryVersionMarker",
            "doReadValues",
            "toolTipText",
            "lastSavedBy",
            "lastSavedDate",
            "label",
            "creationDate",
            "repositoryLastModified",
            "cacheEnabled",
            "scriptExecutionModeChange",
            "autoLabel",
            "createdBy",
            "name",
            "position",
        ];
        if metadata.contains(&name)
            || name
                .strip_prefix(prefix)
                .is_some_and(|key| known.contains(&key))
        {
            continue;
        }
        return Err(OperatorError::Settings(format!(
            "RFM: unsupported setting {name}"
        )));
    }
    Ok(())
}
