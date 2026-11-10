//! Schema-driven node settings. Draft text belongs to the UI; only validated values reach
//! the engine, through one `node.set_param` command per change.
use crate::i18n::{tr, tr_fmt};
use dms_engine::session::{Arg, FieldInfo, Session};
use dms_exec::schema::{self, ParamKind, ParamSource, ParamSpec, ParamValue};
use egui::{Key, RichText, TextEdit, Ui};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

const MAX_TEXT: usize = 1 << 20;
const MAX_ROWS: usize = 10_000;

#[derive(Default)]
struct Draft {
    text: String,
    dirty: bool,
    error: Option<String>,
}

pub(crate) struct FileRequest {
    uid: String,
    key: &'static str,
    revision: u64,
    pub extensions: &'static [&'static str],
}

#[derive(Default)]
pub struct Properties {
    uid: Option<String>,
    revision: u64,
    drafts: HashMap<String, Draft>,
    errors: HashMap<&'static str, String>,
    file_request: Option<FileRequest>,
}

impl Properties {
    /// Returns true when the host should open a native data-file picker.
    pub fn show(&mut self, ui: &mut Ui, session: &mut Session, uid: &str) -> bool {
        if self.uid.as_deref() != Some(uid) || self.revision != session.revision() {
            *self = Self {
                uid: Some(uid.into()),
                revision: session.revision(),
                ..Self::default()
            };
        }
        let Some((_, specs)) = session.node_schema(uid) else {
            ui.label(tr("Settings for this node type cannot be edited yet."));
            self.show_xml(ui, session, uid);
            return false;
        };
        let fields = session.input_fields(uid);
        let mut choose = false;
        egui::ScrollArea::vertical()
            .id_salt(("parameter-form", uid))
            .show(ui, |ui| {
                // Gather groups in schema order, including schemas with interleaved groups.
                let mut groups = Vec::new();
                for spec in specs {
                    if !groups.contains(&spec.group) {
                        groups.push(spec.group);
                    }
                }
                for group in groups {
                    let visible: Vec<_> = specs
                        .iter()
                        .filter(|s| s.group == group && visible(session, uid, s))
                        .collect();
                    if visible.is_empty() {
                        continue;
                    }
                    ui.strong(tr(group));
                    for spec in visible {
                        ui.push_id(spec.key, |ui| {
                            let read_only = session.param_read_only(uid, spec.key);
                            let value = session
                                .param_value(uid, spec.key)
                                .unwrap_or(ParamValue::Unset);
                            let mut edit = None;
                            ui.add_enabled_ui(read_only.is_none(), |ui| match &spec.kind {
                                ParamKind::Table { columns, .. } => {
                                    ui.label(tr(spec.label)).on_hover_text(tr(spec.help));
                                    edit = self.table(ui, spec, &value, columns, fields.as_deref());
                                }
                                ParamKind::FilePath { extensions } => {
                                    edit = self.text(
                                        ui,
                                        spec.key,
                                        tr(spec.label),
                                        &spec.kind,
                                        &value,
                                        false,
                                    );
                                    if ui
                                        .button(tr("Choose..."))
                                        .on_hover_text(tr(spec.help))
                                        .clicked()
                                    {
                                        self.file_request = Some(FileRequest {
                                            uid: uid.into(),
                                            key: spec.key,
                                            revision: session.revision(),
                                            extensions,
                                        });
                                        choose = true;
                                    }
                                }
                                ParamKind::Field { measures } | ParamKind::Fields { measures }
                                    if fields.is_some() =>
                                {
                                    edit = field_picker(
                                        ui,
                                        spec,
                                        &value,
                                        fields.as_deref().unwrap_or_default(),
                                        measures,
                                    );
                                }
                                ParamKind::Bool => {
                                    let mut checked = matches!(value, ParamValue::Bool(true));
                                    let response = ui
                                        .checkbox(&mut checked, tr(spec.label))
                                        .on_hover_text(tr(spec.help));
                                    if response.changed() {
                                        edit = Some(ParamValue::Bool(checked));
                                    }
                                }
                                ParamKind::Choice(choices) => {
                                    let mut text = value_text(&value);
                                    if choice(ui, tr(spec.label), &mut text, choices, false) {
                                        edit = Some(ParamValue::Text(text));
                                    }
                                }
                                _ => {
                                    edit = self.text(
                                        ui,
                                        spec.key,
                                        tr(spec.label),
                                        &spec.kind,
                                        &value,
                                        false,
                                    );
                                    if matches!(
                                        spec.kind,
                                        ParamKind::Field { .. } | ParamKind::Fields { .. }
                                    ) {
                                        ui.weak(tr(
                                            "Run the stream to list the fields reaching this node.",
                                        ));
                                        if matches!(spec.kind, ParamKind::Fields { .. }) {
                                            ui.weak(tr("Enter one field name per line."));
                                        }
                                    }
                                }
                            })
                            .response
                            .on_hover_text(tr(spec.help));
                            if let Some(reason) = read_only {
                                ui.weak(reason);
                            }
                            if let Some(value) = edit {
                                self.submit(session, uid, spec, value);
                            }
                            if let Some(error) = self.errors.get(spec.key) {
                                ui.colored_label(ui.visuals().error_fg_color, error);
                            }
                            ui.add_space(6.0);
                        });
                    }
                    ui.separator();
                }
                if specs.is_empty() {
                    ui.weak(tr("This node has no editable execution options."));
                }
                unknown_elements(ui, session, uid, specs);
            });
        // Changes made by this form must not discard drafts in other controls. A revision
        // from undo, redo or another command resets all drafts at the next frame instead.
        self.revision = session.revision();
        if let Some(request) = &mut self.file_request {
            // Losing text focus can commit a typed path in the same frame as Choose.
            request.revision = self.revision;
        }
        choose
    }

    fn submit(&mut self, session: &mut Session, uid: &str, spec: &ParamSpec, value: ParamValue) {
        let result = schema::validate(spec, &value).and_then(|()| {
            session
                .execute(
                    "node.set_param",
                    &[Arg::Node(uid.into()), Arg::Param(spec.key.into(), value)],
                )
                .map(|_| ())
                .map_err(|e| e.to_string())
        });
        match result {
            Ok(()) => {
                self.errors.remove(spec.key);
            }
            Err(error) => {
                self.errors.insert(spec.key, error);
            }
        }
    }

    pub(crate) fn take_file_request(&mut self) -> Option<FileRequest> {
        self.file_request.take()
    }

    pub(crate) fn picked_file(
        &mut self,
        session: &mut Session,
        request: FileRequest,
        path: PathBuf,
    ) {
        if self.uid.as_deref() != Some(&request.uid) || session.revision() != request.revision {
            return;
        }
        if let Some((_, specs)) = session.node_schema(&request.uid)
            && let Some(spec) = specs.iter().find(|s| s.key == request.key)
        {
            self.submit(
                session,
                &request.uid,
                spec,
                ParamValue::Text(path.to_string_lossy().into_owned()),
            );
            self.drafts.remove(spec.key);
            self.revision = session.revision();
        }
    }

    fn text(
        &mut self,
        ui: &mut Ui,
        id: &str,
        label: &str,
        kind: &ParamKind,
        value: &ParamValue,
        immediate: bool,
    ) -> Option<ParamValue> {
        let label_response = (!immediate).then(|| ui.label(label));
        let draft = self.drafts.entry(id.into()).or_insert_with(|| Draft {
            text: value_text(value),
            ..Draft::default()
        });
        let expression = matches!(kind, ParamKind::Expr);
        let multiple = expression || matches!(kind, ParamKind::Fields { .. });
        let numeric = matches!(kind, ParamKind::Int { .. } | ParamKind::Real { .. });
        if numeric {
            draft.error = parse_value(kind, &draft.text).err();
        }
        let invalid = draft.error.is_some();
        let response = egui::Frame::new()
            .stroke(if invalid {
                egui::Stroke::new(1.0, ui.visuals().error_fg_color)
            } else {
                egui::Stroke::NONE
            })
            .show(ui, |ui| {
                let edit = if multiple {
                    TextEdit::multiline(&mut draft.text).desired_rows(3)
                } else {
                    TextEdit::singleline(&mut draft.text)
                };
                let edit = edit
                    .id_salt(id)
                    .desired_width(ui.available_width().min(420.0))
                    .char_limit(MAX_TEXT);
                let edit = if expression {
                    edit.font(egui::TextStyle::Monospace)
                } else {
                    edit
                };
                ui.add(edit)
            })
            .inner;
        let response = if let Some(label) = label_response {
            response.labelled_by(label.id)
        } else {
            response.accessible_name(label)
        };
        if response.changed() {
            draft.dirty = true;
            draft.error = None;
            if numeric {
                draft.error = parse_value(kind, &draft.text).err();
            }
        }
        let enter = response.has_focus()
            && ui.input(|i| i.key_pressed(Key::Enter) && (!multiple || i.modifiers.command));
        let commit =
            draft.dirty && (response.lost_focus() || enter || (immediate && response.changed()));
        let mut changed = None;
        if commit {
            match parse_value(kind, &draft.text) {
                Ok(value) => {
                    draft.error = None;
                    draft.dirty = false;
                    changed = Some(value);
                }
                Err(error) => {
                    draft.error = Some(error);
                }
            }
        }
        if let Some(error) = &draft.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        changed
    }

    fn table(
        &mut self,
        ui: &mut Ui,
        spec: &ParamSpec,
        value: &ParamValue,
        columns: &[schema::ColumnSpec],
        fields: Option<&[FieldInfo]>,
    ) -> Option<ParamValue> {
        let mut rows = match value {
            ParamValue::Rows(rows) => rows.clone(),
            _ => Vec::new(),
        };
        let mut changed = false;
        egui::ScrollArea::both()
            .id_salt("rows")
            .max_height(260.0)
            .show(ui, |ui| {
                egui::Grid::new("cells").striped(true).show(ui, |ui| {
                    ui.strong(tr("Field"));
                    for col in columns {
                        ui.strong(tr(col.label));
                    }
                    ui.end_row();
                    for (key, values) in rows.iter_mut().take(MAX_ROWS) {
                        ui.label(key.as_str());
                        for (index, col) in columns.iter().enumerate() {
                            let Some(cell) = values.get_mut(index) else {
                                continue;
                            };
                            ui.push_id((key.as_str(), col.child), |ui| {
                                let label = format!("{}: {}", key, tr(col.label));
                                if let ParamKind::Choice(choices) = col.kind {
                                    let mut text = cell.clone().unwrap_or_default();
                                    if choice(ui, &label, &mut text, choices, true) {
                                        *cell = if text.is_empty() { None } else { Some(text) };
                                        changed = true;
                                    }
                                } else {
                                    let id = format!("{}/{}/{}", spec.key, key, col.child);
                                    let old =
                                        cell.clone().map_or(ParamValue::Unset, ParamValue::Text);
                                    if let Some(value) =
                                        self.text(ui, &id, &label, &col.kind, &old, true)
                                    {
                                        *cell = Some(value_text(&value));
                                        changed = true;
                                    }
                                }
                            });
                        }
                        ui.end_row();
                    }
                });
            });
        if rows.len() < MAX_ROWS {
            let id = format!("{}/new-row", spec.key);
            let draft = self.drafts.entry(id).or_default();
            let label = tr_fmt("New field for {label}", &[("label", tr(spec.label))]);
            let response = ui.add(
                TextEdit::singleline(&mut draft.text)
                    .hint_text(tr("Field name"))
                    .char_limit(MAX_TEXT),
            );
            response.accessible_name(label);
            if let Some(fields) = fields {
                egui::ComboBox::from_label(tr("Incoming fields")).show_ui(ui, |ui| {
                    for field in fields.iter().take(MAX_ROWS) {
                        ui.selectable_value(&mut draft.text, field.name.clone(), &field.name);
                    }
                });
            }
            let key = draft.text.trim();
            if ui
                .add_enabled(
                    !key.is_empty() && !rows.iter().any(|(k, _)| k == key),
                    egui::Button::new(tr("Add field")),
                )
                .clicked()
            {
                rows.push((key.into(), vec![None; columns.len()]));
                draft.text.clear();
                changed = true;
            }
        }
        changed.then_some(ParamValue::Rows(rows))
    }

    fn show_xml(&self, ui: &mut Ui, session: &Session, uid: &str) {
        egui::CollapsingHeader::new(tr("Settings XML")).show(ui, |ui| {
            if let Some(settings) = session.node_settings(uid) {
                // Debug formats string contents incrementally without allocating a copy.
                // Use it only to count an upper bound before calling the XML writer (which
                // returns a whole String). This also bounds large single-line settings.
                use std::fmt::Write as _;
                let mut budget = PreviewBudget(MAX_TEXT / 8);
                let mut stack = vec![(settings, 0usize)];
                let mut count = 0usize;
                let mut fits = true;
                while let Some((element, depth)) = stack.pop() {
                    count = count.saturating_add(element.children.len());
                    if depth > 512 || count > 100_000 || write!(&mut budget, "{:?}{:?}{:?}", element.name, element.attributes, element.namespace_declarations).is_err() {
                        fits = false;
                        break;
                    }
                    for child in &element.children {
                        if let Some(element) = child.as_element() {
                            stack.push((element, depth.saturating_add(1)));
                        } else if write!(&mut budget, "{child:?}").is_err() {
                            fits = false;
                            break;
                        }
                    }
                    if !fits { break; }
                }
                if !fits {
                    ui.weak(tr("Settings XML exceeds the preview size limit. The complete settings are preserved."));
                    return;
                }
                let xml = settings.to_xml();
                let mut text = String::new();
                for line in xml.lines().take(2000) {
                    for c in line.chars() {
                        if text.len().saturating_add(c.len_utf8()) > MAX_TEXT {
                            break;
                        }
                        text.push(c);
                    }
                    if text.len() >= MAX_TEXT {
                        break;
                    }
                    text.push('\n');
                }
                egui::ScrollArea::both().max_height(400.0).show(ui, |ui| {
                    ui.add(
                        TextEdit::multiline(&mut text)
                            .font(egui::TextStyle::Monospace)
                            .interactive(false)
                            .desired_width(600.0),
                    );
                });
                if xml.lines().count() > 2000 || xml.len() > MAX_TEXT {
                    ui.weak(tr("XML preview truncated; the complete settings are preserved."));
                }
            } else {
                ui.weak(tr("No saved settings."));
            }
        });
    }
}

fn value_text(value: &ParamValue) -> String {
    match value {
        ParamValue::Unset => String::new(),
        ParamValue::Bool(v) => v.to_string(),
        ParamValue::Int(v) => v.to_string(),
        ParamValue::Real(v) => v.to_string(),
        ParamValue::Text(v) => v.clone(),
        ParamValue::Fields(v) => v.join("\n"),
        ParamValue::Rows(_) => String::new(),
    }
}

fn parse_value(kind: &ParamKind, text: &str) -> Result<ParamValue, String> {
    if text.len() > MAX_TEXT {
        return Err(tr("Parameter text exceeds 1 MiB.").into());
    }
    match kind {
        ParamKind::Int { min, max } => {
            let n = text
                .trim()
                .parse::<i64>()
                .map_err(|_| tr("Enter a whole number.").to_string())?;
            if n < *min || n > *max {
                return Err(tr_fmt(
                    "Enter a value from {min} to {max}.",
                    &[("min", &min.to_string()), ("max", &max.to_string())],
                ));
            }
            Ok(ParamValue::Int(n))
        }
        ParamKind::Real { min, max } => {
            let n = text
                .trim()
                .parse::<f64>()
                .map_err(|_| tr("Enter a finite number.").to_string())?;
            if !n.is_finite() || n < *min || n > *max {
                return Err(tr_fmt(
                    "Enter a finite value from {min} to {max}.",
                    &[("min", &min.to_string()), ("max", &max.to_string())],
                ));
            }
            Ok(ParamValue::Real(n))
        }
        ParamKind::Expr => dms_clem::parse(text)
            .map(|_| ParamValue::Text(text.into()))
            .map_err(|e| e.to_string()),
        ParamKind::Fields { .. } => {
            let fields: Vec<_> = text
                .lines()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .take(MAX_ROWS.saturating_add(1))
                .map(str::to_string)
                .collect();
            if fields.len() > MAX_ROWS {
                return Err(tr("A field list can contain at most 10000 names.").into());
            }
            Ok(ParamValue::Fields(fields))
        }
        _ => Ok(ParamValue::Text(text.into())),
    }
}

fn choice(
    ui: &mut Ui,
    label: &str,
    text: &mut String,
    choices: &[(&'static str, &'static str)],
    optional: bool,
) -> bool {
    let old = text.clone();
    let selected = choices
        .iter()
        .find(|(v, _)| *v == text)
        .map(|(_, l)| tr(l))
        .unwrap_or(if text.is_empty() { tr("Not set") } else { text });
    let label_response = (!optional).then(|| ui.label(label));
    let response = egui::ComboBox::from_id_salt("choice")
        .selected_text(selected)
        .width(ui.available_width().clamp(100.0, 420.0))
        .show_ui(ui, |ui| {
            if optional {
                ui.selectable_value(text, String::new(), tr("Not set"));
            }
            for (value, label) in choices {
                ui.selectable_value(text, (*value).into(), tr(label));
            }
        })
        .response;
    if let Some(label) = label_response {
        response.labelled_by(label.id);
    } else {
        response.accessible_name(label);
    }
    *text != old
}

fn visible(session: &Session, uid: &str, spec: &ParamSpec) -> bool {
    spec.visible_if.is_none_or(|(key, values)| {
        session
            .param_value(uid, key)
            .is_ok_and(|value| values.contains(&value_text(&value).as_str()))
    })
}

fn field_picker(
    ui: &mut Ui,
    spec: &ParamSpec,
    value: &ParamValue,
    fields: &[FieldInfo],
    measures: &[&str],
) -> Option<ParamValue> {
    let label = ui.label(tr(spec.label)).on_hover_text(tr(spec.help));
    let allowed =
        |field: &&FieldInfo| measures.is_empty() || measures.contains(&field.measure.as_str());
    match &spec.kind {
        ParamKind::Field { .. } => {
            let mut selected = value_text(value);
            let old = selected.clone();
            egui::ComboBox::from_id_salt("field")
                .width(ui.available_width().min(420.0))
                .selected_text(if selected.is_empty() {
                    tr("Not set")
                } else {
                    &selected
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut selected, String::new(), tr("Not set"));
                    for field in fields.iter().filter(allowed) {
                        ui.selectable_value(&mut selected, field.name.clone(), &field.name);
                    }
                })
                .response
                .labelled_by(label.id);
            (selected != old).then_some(ParamValue::Text(selected))
        }
        _ => {
            let mut selected = match value {
                ParamValue::Fields(v) => v.clone(),
                _ => Vec::new(),
            };
            let mut changed = false;
            egui::ScrollArea::vertical()
                .id_salt("field-list")
                .max_height(140.0)
                .show(ui, |ui| {
                    // Also show saved names absent from this run, so they can be deselected.
                    let names: BTreeSet<_> = fields
                        .iter()
                        .filter(allowed)
                        .map(|f| f.name.as_str())
                        .chain(selected.iter().map(String::as_str))
                        .collect();
                    let names: Vec<_> = names.into_iter().map(str::to_string).collect();
                    for name in names {
                        let mut checked = selected.contains(&name);
                        if ui.checkbox(&mut checked, &name).changed() {
                            if checked {
                                selected.push(name);
                            } else {
                                selected.retain(|s| s != &name);
                            }
                            changed = true;
                        }
                    }
                });
            changed.then_some(ParamValue::Fields(selected))
        }
    }
}

fn unknown_elements(ui: &mut Ui, session: &Session, uid: &str, specs: &[ParamSpec]) {
    let Some(root) = session.node_settings(uid) else {
        return;
    };
    // Paths include table containers and columns. Stop at scalar leaves; their internals
    // are values, not independent parameters. Traverse iteratively with explicit limits.
    let mut known = BTreeSet::new();
    let mut leaves = BTreeSet::new();
    for spec in specs.iter().filter(|s| s.source == ParamSource::Properties) {
        known.insert(spec.path.to_string());
        match &spec.kind {
            ParamKind::Table {
                row_path,
                key_child,
                columns,
            } => {
                let row = format!("{}/{}", spec.path, row_path);
                known.insert(row.clone());
                leaves.insert(format!("{row}/{key_child}"));
                for col in *columns {
                    leaves.insert(format!("{row}/{}", col.child));
                }
            }
            ParamKind::Fields { .. } => {
                let child = if matches!(spec.key, "calculate" | "user_compute") {
                    "setElement"
                } else {
                    "field"
                };
                leaves.insert(format!("{}/{child}", spec.path));
            }
            _ => {
                leaves.insert(spec.path.to_string());
            }
        }
    }
    let mut names = BTreeSet::new();
    let mut stack = vec![(root.child_elements(), String::new(), 0usize)];
    let mut count = 0usize;
    let mut name_bytes = 0usize;
    while let Some((iter, parent, depth)) = stack.last_mut() {
        if count >= 100_000 {
            break;
        }
        let Some(element) = iter.next() else {
            stack.pop();
            continue;
        };
        count = count.saturating_add(1);
        if parent.len().saturating_add(element.name.local.len()) > 4096 {
            break;
        }
        let path = if parent.is_empty() {
            element.name.local.to_string()
        } else {
            format!("{parent}/{}", element.name.local)
        };
        if known.iter().chain(leaves.iter()).any(|p| {
            p == &path
                || p.strip_prefix(&path)
                    .is_some_and(|tail| tail.starts_with('/'))
        }) {
            if *depth < 512 {
                let next_depth = depth.saturating_add(1);
                stack.push((element.child_elements(), path, next_depth));
            }
        } else {
            name_bytes = name_bytes.saturating_add(path.len());
            if name_bytes > MAX_TEXT {
                break;
            }
            names.insert(path);
        }
    }
    if !names.is_empty() {
        ui.separator();
        ui.strong(tr("Other saved settings"));
        ui.weak(tr(
            "These settings are read-only and will be saved unchanged.",
        ));
        for name in names {
            ui.label(RichText::new(name).monospace());
        }
    }
}

// No buffer allocation while checking the size of an XML preview.
struct PreviewBudget(usize);
impl std::fmt::Write for PreviewBudget {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        self.0 = self.0.checked_sub(text.len()).ok_or(std::fmt::Error)?;
        Ok(())
    }
}
