//! Imported PMML model (`ApplyImportedNode`, catalog `pmml_model`): a model brought in with
//! File > Models > Import PMML and placed in a stream (`oracle/pmml_import/*`).
//!
//! Modeler keeps the PMML text in the node's attachment (`model@pmml`) and, beside it, the
//! inputs and outputs it derived when the file was imported (`model/inputs`, `model/outputs`).
//! It scores with that saved description, not one rebuilt from the PMML: with a different PMML
//! swapped into the attachment (a QUEST classification tree under a regression template) every
//! record scored the template's `$X-y` as 0.0. The saved outputs are therefore the contract
//! here: one field per output, in order, named and stored as saved.
//!
//! Evidence: `pmml_cart_regression` (the user's dialog import of a C&R Tree regression of `y` on
//! `x`): the single output `$X-y` (`prefix $X`, role `predictedValue`, storage real) is the
//! tree's predicted value for each record.
//!
//! GAP: output roles other than `predictedValue` (a classifier's confidence or probabilities)
//! have no oracle yet.

use super::apply::{saved_model_xml, to_pmml, typed};
use super::field_util::{Column, with_columns};
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_pmml::Pmml;
use dms_stream::StreamNode;
use dms_types::{Field, Measurement, Storage, Table};
use std::sync::Arc;

const LABEL: &str = "Imported PMML model";

/// Bound on the saved outputs we allocate columns for.
const MAX_OUTPUTS: usize = 1_000;

fn gap(message: impl std::fmt::Display) -> OperatorError {
    OperatorError::Settings(format!("GAP: {LABEL}: {message}"))
}

struct Output {
    name: String,
    storage: Storage,
}

struct ImportedModel {
    member: String,
    outputs: Vec<Output>,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let model = settings::element(s, "model")
        .ok_or_else(|| OperatorError::Settings(format!("{LABEL}: the node saved no model")))?;
    if model
        .attribute("class")
        .is_some_and(|class| !class.ends_with(".ImportedModel"))
    {
        return Err(gap(format!("model class {:?}", model.attribute("class"))));
    }
    for (attribute, expected) in [
        ("isExternalModel", "false"),
        ("isCFScoringModel", "false"),
        ("useVariablelabels", "false"),
    ] {
        if model
            .attribute(attribute)
            .is_some_and(|value| value != expected)
        {
            return Err(gap(format!("{attribute} other than {expected}")));
        }
    }
    let member = model
        .attribute("pmml")
        .filter(|member| !member.is_empty())
        .ok_or_else(|| {
            OperatorError::Settings(format!("{LABEL}: the model has no PMML attachment"))
        })?
        .to_string();
    let saved = settings::children(Some(model), "outputs", "output");
    if saved.is_empty() {
        return Err(OperatorError::Settings(format!(
            "{LABEL}: the model saved no outputs"
        )));
    }
    if saved.len() > MAX_OUTPUTS {
        return Err(gap(format!("more than {MAX_OUTPUTS} outputs")));
    }
    let mut outputs = Vec::with_capacity(saved.len());
    for output in saved {
        let name = settings::text(Some(output), "name")
            .filter(|name| !name.is_empty())
            .ok_or_else(|| OperatorError::Settings(format!("{LABEL}: an output without a name")))?;
        let role = output
            .child("modelFieldInfo")
            .and_then(|info| info.attribute("role"))
            .unwrap_or_default();
        if role != "predictedValue" {
            return Err(gap(format!("output {name} with role {role:?}")));
        }
        let storage = match settings::text(Some(output), "type/storage").as_deref() {
            Some("real") => Storage::Real,
            Some("integer") => Storage::Integer,
            Some("string") => Storage::String,
            other => return Err(gap(format!("output {name} storage {other:?}"))),
        };
        outputs.push(Output { name, storage });
    }
    Ok(Box::new(ImportedModel { member, outputs }))
}

impl Operator for ImportedModel {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        let [input] = inputs else {
            return Err(OperatorError::Inputs(format!("{LABEL} needs one input")));
        };
        let xml = saved_model_xml(&self.member, context)?;
        let pmml =
            Pmml::parse(&xml).map_err(|error| OperatorError::Data(format!("{LABEL}: {error}")))?;
        let names: Vec<&str> = input
            .model()
            .fields()
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        let scorer = pmml
            .bind(&names)
            .map_err(|error| OperatorError::Data(format!("{LABEL}: {error}")))?;
        let mut columns: Vec<Column> = self
            .outputs
            .iter()
            .map(|output| {
                let mut field = Field::new(output.name.clone(), output.storage);
                field.measurement = match output.storage {
                    Storage::String => Measurement::Categorical,
                    _ => Measurement::Continuous,
                };
                Column {
                    field,
                    values: Vec::with_capacity(input.row_count()),
                }
            })
            .collect();
        for row in input.rows() {
            let record: Vec<_> = row.iter().map(to_pmml).collect();
            let prediction = scorer
                .score(&record)
                .map_err(|error| OperatorError::Data(format!("{LABEL}: {error}")))?;
            for (column, output) in columns.iter_mut().zip(&self.outputs) {
                column.values.push(typed(&prediction.value, output.storage));
            }
        }
        if columns
            .iter()
            .any(|column| input.model().field(&column.field.name).is_some())
        {
            return Err(gap("an output field name already exists in the input"));
        }
        with_columns(input, columns)
    }
}
