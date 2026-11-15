//! After a run, models trained in Python are saved in the stream: each build node's model goes
//! into an attachment and its nugget (added next to the build node, fed from the same input, on
//! the first run) scores with it. One undoable step, "Generate Model".

use crate::edit_session::random_word;
use crate::run::RunReport;
use crate::session::Session;
use dms_exec::schema::{ParamKind, ParamSource, ParamSpec, ParamValue};
use dms_exec::{NodeOutput, PythonModel};
use dms_stream::{Edit, Link, NodeRef, StreamNode, new_uid};
use std::collections::HashSet;
use std::sync::Arc;

/// The nugget setting naming its model attachment (`dms_exec::nodes::python_model`).
const MODEL: ParamSpec = ParamSpec {
    key: "python_model",
    source: ParamSource::Properties,
    path: dms_exec::nodes::python_model::MODEL_SETTING,
    label: "Saved model",
    help: "The stream attachment holding the trained model.",
    group: "Model",
    kind: ParamKind::Text,
    default: None,
    visible_if: None,
};

/// Vertical distance from a build node to the nugget placed for it.
const BELOW: i64 = 120;

/// Python models in a run's results, by build node.
pub(crate) fn trained(report: &RunReport) -> Vec<(String, Arc<PythonModel>)> {
    let mut models: Vec<_> = report
        .results
        .iter()
        .filter_map(|(uid, result)| match result {
            Ok(NodeOutput::PythonModel(model)) => Some((uid.clone(), Arc::clone(model))),
            _ => None,
        })
        .collect();
    models.sort_by(|a, b| a.0.cmp(&b.0));
    models
}

impl Session {
    /// Saves `models` into the open document. Returns how many were saved.
    pub(crate) fn save_python_models(
        &mut self,
        models: &[(String, Arc<PythonModel>)],
    ) -> Result<usize, String> {
        let Some(state) = self.document_state.as_mut() else {
            return Ok(0);
        };
        let diagram = &state.doc.stream.diagram;
        let mut used: HashSet<String> = state
            .doc
            .stream
            .all_nodes()
            .iter()
            .map(|n| n.uid.clone())
            .collect();
        let mut edits = Vec::new();
        let mut added_nodes = 0;
        let mut added_links = 0;
        let mut saved = 0;
        for (uid, model) in models {
            // The build node may have been deleted while it trained.
            let Some(builder) = diagram
                .nodes
                .iter()
                .find(|n| &n.uid == uid && n.model_builder.is_none())
            else {
                continue;
            };
            let attachment = format!("python/{uid}.pkl");
            edits.push(Edit::PutAttachment {
                name: attachment.clone(),
                bytes: Some(Arc::clone(&model.bytes)),
            });
            let settings =
                dms_exec::schema::write(None, &MODEL, &ParamValue::Text(attachment.clone()))?;
            let nugget = diagram.nodes.iter().find(|n| {
                n.model_builder.as_deref() == Some(uid.as_str())
                    && n.type_identifier() == builder.type_identifier()
            });
            match nugget {
                Some(nugget) => {
                    let current = dms_exec::settings::text(
                        nugget.settings.as_ref(),
                        dms_exec::nodes::python_model::MODEL_SETTING,
                    );
                    if current.as_deref() != Some(attachment.as_str()) {
                        let updated = dms_exec::schema::write(
                            nugget.settings.as_ref(),
                            &MODEL,
                            &ParamValue::Text(attachment),
                        )?;
                        edits.push(Edit::SetSettings {
                            uid: nugget.uid.clone(),
                            settings: Some(updated),
                        });
                    }
                }
                None => {
                    let new = new_uid(&|s| used.contains(s), &mut random_word);
                    used.insert(new.clone());
                    let node = StreamNode {
                        uid: new.clone(),
                        class: builder.class.clone(),
                        extension: builder.extension.clone(),
                        model_builder: Some(uid.clone()),
                        name: None,
                        label: builder.label.clone().or_else(|| builder.name.clone()),
                        position: builder.position.map(|(x, y)| (x, y.saturating_add(BELOW))),
                        properties_attachment: None,
                        containers: Vec::new(),
                        settings: Some(settings),
                        binning_output: None,
                        result_items: Vec::new(),
                        subdiagram: None,
                    };
                    edits.push(Edit::AddNode {
                        node,
                        index: diagram.nodes.len().saturating_add(added_nodes),
                    });
                    added_nodes += 1;
                    // Fed from the same node as the build node.
                    if let Some(input) = diagram
                        .links
                        .iter()
                        .find(|l| matches!(&l.to, NodeRef::Uid(to) if to == uid))
                    {
                        edits.push(Edit::Connect {
                            link: Link {
                                from: input.from.clone(),
                                to: NodeRef::Uid(new),
                            },
                            index: diagram.links.len().saturating_add(added_links),
                        });
                        added_links += 1;
                    }
                }
            }
            saved += 1;
        }
        if edits.is_empty() {
            return Ok(0);
        }
        let selection = state.selection.clone();
        state.commit("Generate Model", edits, selection, None)?;
        self.sync_document();
        Ok(saved)
    }
}
