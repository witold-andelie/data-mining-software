//! The parameters of an extension node (`Parameter` entries of the node's properties
//! attachment, `node@properties`, e.g. `data/0001.dat`), shared by the JSON source and export.

use crate::context::ExecContext;
use crate::error::OperatorError;
use std::collections::HashMap;

pub(crate) enum Param {
    Value(String),
    List(Vec<String>),
    /// A list of `StructuredValue`s, each a list of named `Attribute`s (the XML source's
    /// scanned `xmlReader.xpaths` and `xmlReader.storages`).
    Structured(Vec<Vec<(String, String)>>),
}

pub(crate) fn load_parameters(
    attachment: Option<&str>,
    context: &ExecContext,
    label: &str,
) -> Result<HashMap<String, Param>, OperatorError> {
    let member = attachment.ok_or_else(|| {
        OperatorError::Settings(format!("{label}: the node has no parameters attachment"))
    })?;
    let bytes = context.attachments.get(member).ok_or_else(|| {
        OperatorError::Io(format!(
            "{label}: the stream file has no attachment {member}"
        ))
    })?;
    let text = std::str::from_utf8(bytes)
        .map_err(|_| OperatorError::Data(format!("{label}: {member} is not UTF-8 text")))?;
    parse_parameters(text)
        .map_err(|error| OperatorError::Data(format!("{label}: {member}: {error}")))
}

fn parse_parameters(text: &str) -> Result<HashMap<String, Param>, String> {
    let document = roxmltree::Document::parse(text).map_err(|error| error.to_string())?;
    let mut values = HashMap::new();
    for element in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "Parameter")
    {
        let Some(name) = element.attribute("name") else {
            continue;
        };
        let structured: Vec<_> = element
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "StructuredValue")
            .collect();
        let parameter = match element.attribute("value") {
            Some(value) => Param::Value(value.to_string()),
            None if !structured.is_empty() => Param::Structured(
                structured
                    .iter()
                    .map(|item| {
                        item.children()
                            .filter(|node| {
                                node.is_element() && node.tag_name().name() == "Attribute"
                            })
                            .map(|node| {
                                (
                                    node.attribute("name").unwrap_or_default().to_string(),
                                    node.attribute("value").unwrap_or_default().to_string(),
                                )
                            })
                            .collect()
                    })
                    .collect(),
            ),
            None => Param::List(
                element
                    .descendants()
                    .filter(|node| node.is_element() && node.tag_name().name() == "Value")
                    .filter_map(|node| node.attribute("value"))
                    .map(str::to_string)
                    .collect(),
            ),
        };
        values.insert(name.to_string(), parameter);
    }
    Ok(values)
}

pub(crate) fn text(params: &HashMap<String, Param>, name: &str) -> Option<String> {
    match params.get(name) {
        Some(Param::Value(value)) => Some(value.clone()),
        _ => None,
    }
}

pub(crate) fn flag(
    params: &HashMap<String, Param>,
    name: &str,
) -> Result<Option<bool>, OperatorError> {
    match text(params, name).as_deref().map(str::trim) {
        None => Ok(None),
        Some("true") => Ok(Some(true)),
        Some("false") => Ok(Some(false)),
        Some(other) => Err(OperatorError::Settings(format!(
            "parameter {name}: {other:?} is not true or false"
        ))),
    }
}

/// A list parameter would be a setting the node does not implement.
pub(crate) fn reject_lists(
    params: &HashMap<String, Param>,
    label: &str,
) -> Result<(), OperatorError> {
    for (name, param) in params {
        let listed = match param {
            Param::Value(_) => false,
            Param::List(values) => !values.is_empty(),
            Param::Structured(items) => !items.is_empty(),
        };
        if listed {
            return Err(OperatorError::Settings(format!(
                "{label}: parameter {name} is a list, which is not implemented"
            )));
        }
    }
    Ok(())
}

pub(crate) fn reject_unknown(
    params: &HashMap<String, Param>,
    known: &[&str],
    label: &str,
) -> Result<(), OperatorError> {
    for name in params.keys() {
        if name.starts_with("pasw.annotation_") || known.contains(&name.as_str()) {
            continue;
        }
        return Err(OperatorError::Settings(format!(
            "{label}: unknown parameter {name}"
        )));
    }
    Ok(())
}
