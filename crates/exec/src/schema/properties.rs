use super::*;
use dms_strfmt::fastinfoset::{Element, Node, QualifiedName};
use std::sync::Arc;

pub(super) fn check_tree(root: &Element) -> Result<(), String> {
    let mut stack = vec![(root, 1usize)];
    let mut count = 0usize;
    while let Some((element, depth)) = stack.pop() {
        count = count
            .saturating_add(element.children.len())
            .saturating_add(element.attributes.len())
            .saturating_add(element.namespace_declarations.len());
        if depth > MAX_DEPTH || count > MAX_ITEMS {
            return Err("Settings exceed depth 512 or 100000 items".into());
        }
        stack.extend(
            element
                .child_elements()
                .map(|e| (e, depth.saturating_add(1))),
        );
    }
    Ok(())
}

pub(super) fn empty(name: &str) -> Element {
    Element {
        name: QualifiedName {
            local: name.into(),
            prefix: None,
            namespace: None,
        },
        namespace_declarations: Vec::new(),
        attributes: Vec::new(),
        children: Vec::new(),
    }
}

// Clone with an explicit stack: callers can supply trees at the XML nesting limit.
fn copy_tree(root: &Element) -> Element {
    fn shell(e: &Element) -> Element {
        Element {
            name: e.name.clone(),
            namespace_declarations: e.namespace_declarations.clone(),
            attributes: e.attributes.clone(),
            children: Vec::new(),
        }
    }
    let mut stack = vec![(root.children.iter(), shell(root))];
    loop {
        match stack.last_mut().and_then(|(iter, _)| iter.next()) {
            Some(Node::Element(e)) => stack.push((e.children.iter(), shell(e))),
            Some(n) => {
                if let Some((_, parent)) = stack.last_mut() {
                    parent.children.push(n.clone());
                }
            }
            None => {
                if let Some((_, done)) = stack.pop() {
                    if let Some((_, parent)) = stack.last_mut() {
                        parent.children.push(Node::Element(done));
                    } else {
                        return done;
                    }
                }
            }
        }
    }
}

fn bounded_text(e: &Element) -> Result<String, String> {
    let mut result = String::new();
    for child in &e.children {
        if let Node::Text(text) = child {
            if result.len().saturating_add(text.len()) > MAX_TEXT {
                return Err("Parameter text exceeds 1 MiB".into());
            }
            result.push_str(text);
        }
    }
    Ok(result)
}

fn path_parts(path: &str) -> Result<Vec<&str>, String> {
    if path.len() > 4096 {
        return Err("Parameter path is too long".into());
    }
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() > 32 || parts.iter().any(|p| p.is_empty()) {
        return Err("Invalid parameter path".into());
    }
    Ok(parts)
}

fn target<'a>(root: &'a Element, path: &str) -> Result<Option<&'a Element>, String> {
    let parts = path_parts(path)?;
    Ok(parts.into_iter().try_fold(root, |e, name| e.child(name)))
}

pub fn read(settings: Option<&Element>, spec: &ParamSpec) -> Result<ParamValue, String> {
    if spec.source != ParamSource::Properties {
        return Err("This parameter comes from an attachment".into());
    }
    if let Some(root) = settings {
        check_tree(root)?;
    }
    let element = match settings {
        Some(root) => target(root, spec.path)?,
        None => None,
    };
    let Some(element) = element else {
        return spec
            .default
            .map_or(Ok(ParamValue::Unset), |text| scalar(&spec.kind, text));
    };
    match &spec.kind {
        ParamKind::Fields { .. } => {
            let child_name = list_child(spec);
            let mut values = Vec::new();
            let mut size = 0usize;
            for child in element
                .child_elements()
                .filter(|c| c.name.local.as_ref() == child_name)
            {
                if values.len() >= MAX_ROWS {
                    return Err("List exceeds 10000 entries".into());
                }
                let text = bounded_text(child)?;
                size = size.saturating_add(text.len());
                if size > MAX_TEXT {
                    return Err("List text exceeds 1 MiB".into());
                }
                values.push(text);
            }
            Ok(ParamValue::Fields(values))
        }
        ParamKind::Table {
            row_path,
            key_child,
            columns,
        } => {
            let (parent_path, row_name) = row_path
                .rsplit_once('/')
                .ok_or("Table needs a row parent")?;
            let mut values = Vec::new();
            let mut size = 0usize;
            if let Some(parent) = target(element, parent_path)? {
                for row in parent
                    .child_elements()
                    .filter(|r| r.name.local.as_ref() == row_name)
                {
                    if values.len() >= MAX_ROWS {
                        return Err("Table exceeds 10000 rows".into());
                    }
                    let key = target(row, key_child)?
                        .map(bounded_text)
                        .transpose()?
                        .unwrap_or_default();
                    size = size.saturating_add(key.len());
                    let mut cells = Vec::new();
                    for column in *columns {
                        let text = target(row, column.child)?.map(bounded_text).transpose()?;
                        if let Some(text) = &text {
                            scalar(&column.kind, text)?;
                            size = size.saturating_add(text.len());
                        }
                        if size > MAX_TEXT {
                            return Err("Table text exceeds 1 MiB".into());
                        }
                        cells.push(text);
                    }
                    values.push((key, cells));
                }
            }
            Ok(ParamValue::Rows(values))
        }
        kind => scalar(kind, &bounded_text(element)?),
    }
}

// Field lists and enumeration sets have different child names in the documented format.
fn list_child(spec: &ParamSpec) -> &str {
    if matches!(spec.key, "calculate" | "user_compute") {
        "setElement"
    } else {
        "field"
    }
}

fn parent_mut<'a>(mut root: &'a mut Element, parts: &[&str]) -> Result<&'a mut Element, String> {
    for name in parts {
        let index = match root
            .children
            .iter()
            .position(|c| matches!(c, Node::Element(e) if e.name.local.as_ref() == *name))
        {
            Some(i) => i,
            None => {
                let index = root.children.len();
                root.children.push(Node::Element(empty(name)));
                index
            }
        };
        root = match root.children.get_mut(index) {
            Some(Node::Element(e)) => e,
            _ => return Err("Invalid settings path".into()),
        };
    }
    Ok(root)
}

// Changes text chunks only, leaving attributes, comments and unknown children in place.
fn set_text(root: &mut Element, path: &str, text: Option<&str>) -> Result<(), String> {
    let parts = path_parts(path)?;
    let (last, parents) = parts.split_last().ok_or("Empty parameter path")?;
    if text.is_none() && target(root, path)?.is_none() {
        return Ok(());
    }
    let parent = parent_mut(root, parents)?;
    let index = parent
        .children
        .iter()
        .position(|n| matches!(n, Node::Element(e) if e.name.local.as_ref() == *last));
    if let Some(text) = text {
        if let Some(i) = index {
            if let Some(Node::Element(e)) = parent.children.get_mut(i) {
                if bounded_text(e)? == text {
                    return Ok(());
                }
                let position = e
                    .children
                    .iter()
                    .position(|n| matches!(n, Node::Text(_)))
                    .unwrap_or(0);
                e.children.retain(|n| !matches!(n, Node::Text(_)));
                e.children
                    .insert(position.min(e.children.len()), Node::Text(Arc::from(text)));
            }
        } else {
            let mut e = empty(last);
            e.children.push(Node::Text(Arc::from(text)));
            parent.children.push(Node::Element(e));
        }
    } else if let Some(i) = index {
        parent.children.remove(i);
    }
    Ok(())
}

pub fn write(
    settings: Option<&Element>,
    spec: &ParamSpec,
    value: &ParamValue,
) -> Result<Element, String> {
    // Preserve untouched imports, including absent defaults and legacy values outside today's bounds.
    let previous = read(settings, spec)?;
    if same_value(&previous, value) {
        return Ok(settings
            .map(copy_tree)
            .unwrap_or_else(|| empty("properties")));
    }
    validate(spec, value)?;
    let mut root = settings
        .map(copy_tree)
        .unwrap_or_else(|| empty("properties"));
    match (&spec.kind, value) {
        (ParamKind::Fields { .. }, value @ (ParamValue::Fields(_) | ParamValue::Unset)) => {
            let fields = match value {
                ParamValue::Fields(fields) => fields.as_slice(),
                _ => &[],
            };
            let parent = parent_mut(&mut root, &path_parts(spec.path)?)?;
            let child_name = list_child(spec);
            // Reuse the old slots, preserving comments and non-list elements between them.
            let mut next = fields.iter();
            let mut children = Vec::new();
            for node in std::mem::take(&mut parent.children) {
                if let Node::Element(mut e) = node {
                    if e.name.local.as_ref() == child_name {
                        if let Some(text) = next.next() {
                            if bounded_text(&e)? != *text {
                                let at = e
                                    .children
                                    .iter()
                                    .position(|n| matches!(n, Node::Text(_)))
                                    .unwrap_or(0);
                                e.children.retain(|n| !matches!(n, Node::Text(_)));
                                e.children.insert(
                                    at.min(e.children.len()),
                                    Node::Text(Arc::from(text.as_str())),
                                );
                            }
                            children.push(Node::Element(e));
                        }
                    } else {
                        children.push(Node::Element(e));
                    }
                } else {
                    children.push(node);
                }
            }
            for text in next {
                let mut e = empty(child_name);
                e.children.push(Node::Text(Arc::from(text.as_str())));
                children.push(Node::Element(e));
            }
            parent.children = children;
        }
        (
            ParamKind::Table {
                row_path,
                key_child,
                columns,
            },
            value @ (ParamValue::Rows(_) | ParamValue::Unset),
        ) => {
            let rows = match value {
                ParamValue::Rows(rows) => rows.as_slice(),
                _ => &[],
            };
            let table = parent_mut(&mut root, &path_parts(spec.path)?)?;
            let parts = path_parts(row_path)?;
            let (row_name, parents) = parts.split_last().ok_or("Invalid row path")?;
            let parent = parent_mut(table, parents)?;
            let mut desired: std::collections::HashMap<_, _> = rows
                .iter()
                .map(|(key, cells)| (key.as_str(), cells))
                .collect();
            for node in &mut parent.children {
                let Node::Element(row) = node else {
                    continue;
                };
                if row.name.local.as_ref() != *row_name {
                    continue;
                }
                let key = target(row, key_child)?
                    .map(bounded_text)
                    .transpose()?
                    .unwrap_or_default();
                let cells = desired.remove(key.as_str());
                for (i, column) in columns.iter().enumerate() {
                    let text = cells.and_then(|c| c.get(i)).and_then(Option::as_deref);
                    let old = target(row, column.child)?.map(bounded_text).transpose()?;
                    if old.as_deref() != text {
                        let canonical = text
                            .map(|t| {
                                scalar(&column.kind, t).and_then(|v| {
                                    if matches!(v, ParamValue::Unset) {
                                        Ok(String::new())
                                    } else {
                                        file_text(&v)
                                    }
                                })
                            })
                            .transpose()?;
                        set_text(row, column.child, canonical.as_deref())?;
                    }
                }
            }
            // Retain row order and every unknown cell; append newly named rows in request order.
            for (key, cells) in rows {
                if desired.remove(key.as_str()).is_none() {
                    continue;
                }
                let mut row = empty(row_name);
                set_text(&mut row, key_child, Some(key))?;
                for (column, text) in columns.iter().zip(cells) {
                    let canonical = text
                        .as_deref()
                        .map(|t| {
                            scalar(&column.kind, t).and_then(|v| {
                                if matches!(v, ParamValue::Unset) {
                                    Ok(String::new())
                                } else {
                                    file_text(&v)
                                }
                            })
                        })
                        .transpose()?;
                    set_text(&mut row, column.child, canonical.as_deref())?;
                }
                parent.children.push(Node::Element(row));
            }
        }
        (_, ParamValue::Unset) => set_text(&mut root, spec.path, None)?,
        _ => set_text(&mut root, spec.path, Some(&file_text(value)?))?,
    }
    check_tree(&root)?;
    Ok(root)
}
