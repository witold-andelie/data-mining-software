//! Feature Selection build/settings and filtering nugget. Statistical rules and evidence:
//! `dms_algo::feature_selection`. Saved classic nuggets contain structured `model/inputs`,
//! `model/ranked`, `model/screened` and two independent selection lists, not usable PMML.
//!
//! The existing executor transports models as `Pmml`; a private, empty MiningModel envelope
//! (`algorithm = dms:feature-selection`) carries the modeled inputs and their selected usage.
//! This is an internal transport only, not a claim that Feature Selection exports standard PMML.
//! As for other nuggets, a freshly trained envelope replaces the saved model/automatic selection.
//! A disconnected nugget preserves its saved manual choices. `fs_manual_fields` distinguishes
//! these: the initial Table export keeps weak/constant, but the later data export (after running
//! the builder) keeps cats8. This follows the oracle runner's terminal execution order.
//! Missing/new scoring columns pass through; known unselected inputs are dropped in input order.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::NodeOutput;
use crate::settings;
use dms_algo::Dataset;
use dms_algo::feature_selection::{self as algo, Criterion, Selection, Settings};
use dms_pmml::{MiningFunction, MiningModel, Model, ModelHeader, Pmml, Usage};
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::{DataModel, Role, Table, Value, ValueSpec};
use std::collections::BTreeSet;
use std::sync::Arc;

const ENVELOPE: &str = "dms:feature-selection";

pub(super) struct Build {
    pub(super) settings: Settings,
    target: Option<String>,
    inputs: Option<Vec<String>>,
    partition: bool,
    name: Option<String>,
}

fn error(e: impl std::fmt::Display) -> OperatorError {
    OperatorError::Data(format!("Feature Selection: {e}"))
}

pub(super) fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    Ok(Box::new(build_spec(node)?))
}

pub(super) fn build_spec(node: &StreamNode) -> Result<Build, OperatorError> {
    let s = node.settings.as_ref();
    let defaults = Settings::default();
    let criterion = match settings::text(s, "criteria")
        .as_deref()
        .map(str::trim)
        .unwrap_or("pearson")
    {
        "pearson" => Criterion::Pearson,
        "likelihood" => Criterion::Likelihood,
        "cramersV" => Criterion::CramersV,
        "lambda" => Criterion::Lambda,
        other => {
            return Err(OperatorError::Settings(format!(
                "unsupported Feature Selection criterion {other:?}"
            )));
        }
    };
    let selection = match settings::text(s, "fieldSelectionMode")
        .as_deref()
        .map(str::trim)
        .unwrap_or("importanceLevel")
    {
        "importanceLevel" => Selection::Levels {
            important: settings::flag(s, "selectImportantFields")?.unwrap_or(true),
            marginal: settings::flag(s, "selectMarginalFields")?.unwrap_or(false),
            unimportant: settings::flag(s, "selectUnimportantFields")?.unwrap_or(false),
        },
        "topN" => {
            let n = settings::integer(s, "topNFields")?.unwrap_or(10);
            Selection::TopN(usize::try_from(n).map_err(|_| {
                OperatorError::Settings("Feature Selection top N must be nonnegative".into())
            })?)
        }
        "importanceValue" => {
            Selection::Value(settings::real(s, "importanceValue")?.unwrap_or(0.95))
        }
        other => {
            return Err(OperatorError::Settings(format!(
                "unsupported Feature Selection selection mode {other:?}"
            )));
        }
    };
    let optional = |flag: &str, value: &str, default: f64| -> Result<Option<f64>, OperatorError> {
        let threshold = settings::real(s, value)?.unwrap_or(default);
        Ok(settings::flag(s, flag)?
            .unwrap_or(true)
            .then_some(threshold))
    };
    let parameters = Settings {
        max_missing_percent: optional("screenMissingValues", "maxMissingValues", 70.0)?,
        max_single_percent: optional("screenMaxSingleCategory", "maxSingleCategory", 90.0)?,
        max_categories_percent: optional(
            "screenMaxCategoriesAsCases",
            "maxCategoriesAsCases",
            95.0,
        )?,
        min_sd: optional("screenStdDev", "minStdDev", 0.0)?,
        min_cv: optional("screenVariability", "minVariability", 0.1)?,
        criterion,
        low: settings::real(s, "importanceLowThreshold")?.unwrap_or(defaults.low),
        high: settings::real(s, "importanceMediumThreshold")?.unwrap_or(defaults.high),
        labels: [
            settings::text(s, "importanceHighLabel").unwrap_or_else(|| "Important".into()),
            settings::text(s, "importanceMediumLabel").unwrap_or_else(|| "Marginal".into()),
            settings::text(s, "importanceLowLabel").unwrap_or_else(|| "Unimportant".into()),
        ],
        selection,
    };
    parameters
        .validate()
        .map_err(|e| OperatorError::Settings(e.to_string()))?;
    if settings::flag(s, "useEvents")?.unwrap_or(false) {
        return Err(OperatorError::Settings(
            "unsupported Feature Selection events/trials".into(),
        ));
    }
    let custom = !settings::flag(s, "autoModelFieldSelection")?.unwrap_or(true);
    let inputs = if custom {
        let fields = names(s, "modelInputFieldSelection", "field")?;
        if fields.is_empty() {
            return Err(OperatorError::Settings(
                "Feature Selection custom inputs are empty".into(),
            ));
        }
        Some(fields)
    } else {
        None
    };
    let target = if custom {
        Some(
            settings::text(s, "modelTargetFieldSelection")
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    OperatorError::Settings("Feature Selection custom target is empty".into())
                })?,
        )
    } else {
        None
    };
    Ok(Build {
        settings: parameters,
        target,
        inputs,
        partition: settings::flag(s, "usePartitionedData")?.unwrap_or(true),
        name: if settings::flag(s, "autoModelName")?.unwrap_or(true) {
            None
        } else {
            settings::text(s, "modelName")
        },
    })
}

impl Build {
    pub(super) fn fit(
        &self,
        input: &Table,
    ) -> Result<(Table, usize, algo::FeatureSelectionModel), OperatorError> {
        if input.row_count() > 1_000_000
            || input.row_count().saturating_mul(input.model().len()) > 16_000_000
        {
            return Err(error(
                "training table exceeds 1000000 rows / 16000000 cells",
            ));
        }
        if input.model().fields().iter().any(|f| f.role == Role::Split) {
            return Err(OperatorError::Settings(
                "unsupported Feature Selection split models".into(),
            ));
        }
        let partition = input
            .model()
            .fields()
            .iter()
            .enumerate()
            .find(|(_, f)| f.role == Role::Partition);
        let table = if let Some((position, field)) = partition.filter(|_| self.partition) {
            let training = match &field.values {
                ValueSpec::Categories(values) => values.first(),
                _ => None,
            }
            .ok_or_else(|| error("partition field must declare its training value"))?;
            Table::new(
                input.model().clone(),
                input
                    .rows()
                    .iter()
                    .filter(|r| r.get(position) == Some(training))
                    .cloned()
                    .collect(),
            )
            .map_err(error)?
        } else {
            input.clone()
        };
        let (target, inputs) = super::build::feature_selection_fields(
            &table,
            self.target.as_deref(),
            self.inputs.as_deref(),
        )?;
        let fitted = algo::train(
            &Dataset {
                table: &table,
                target,
                inputs: &inputs,
                frequency: None,
            },
            &self.settings,
        )
        .map_err(error)?;
        Ok((table, target, fitted))
    }
}

impl Operator for Build {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(
            "Feature Selection builder produces a model".into(),
        ))
    }

    fn produce(
        &self,
        inputs: &[Arc<Table>],
        _model: Option<Arc<Pmml>>,
        _context: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Feature Selection needs one input".into()))?;
        let (table, target, fitted) = self.fit(input)?;
        let selected: BTreeSet<_> = fitted.selected.iter().copied().collect();
        let schema = std::iter::once(dms_algo::mining_field(&table, target, Usage::Target))
            .chain(fitted.inputs.iter().map(|p| {
                dms_algo::mining_field(
                    &table,
                    *p,
                    if selected.contains(p) {
                        Usage::Active
                    } else {
                        Usage::Supplementary
                    },
                )
            }))
            .flatten()
            .collect();
        let dictionary = std::iter::once(target)
            .chain(fitted.inputs)
            .filter_map(|p| dms_algo::data_field(&table, p))
            .collect();
        Ok(NodeOutput::Model(Arc::new(Pmml {
            version: "4.3".into(),
            data_dictionary: dictionary,
            transformations: Vec::new(),
            functions: Vec::new(),
            model: Model::Mining(MiningModel {
                header: ModelHeader {
                    name: self.name.clone(),
                    function: MiningFunction::Other("featureSelection".into()),
                    algorithm: Some(ENVELOPE.into()),
                    mining_schema: schema,
                    output: Vec::new(),
                    local_transformations: Vec::new(),
                },
                method: ENVELOPE.into(),
                segments: Vec::new(),
            }),
        })))
    }
}

struct Apply {
    /// None means an unsaved nugget; Some(empty) is an explicit empty selection.
    saved: Option<(BTreeSet<String>, BTreeSet<String>)>,
}

fn names(s: Option<&Element>, path: &str, tag: &str) -> Result<Vec<String>, OperatorError> {
    let mut names = Vec::new();
    for el in settings::element(s, path)
        .into_iter()
        .flat_map(Element::child_elements)
    {
        if el.name.local.as_ref() != tag {
            return Err(OperatorError::Settings(format!(
                "unsupported Feature Selection {path} entry {}",
                el.name.local
            )));
        }
        let name = el.text();
        if name.is_empty() || name.len() > 65536 || names.len() >= 4096 {
            return Err(OperatorError::Settings(format!(
                "invalid Feature Selection {path}: empty/oversized field list"
            )));
        }
        names.push(name);
    }
    Ok(names)
}

pub(super) fn apply(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let model = settings::element(s, "model");
    let saved = if let Some(model) = model {
        let mut modeled = BTreeSet::new();
        for input in settings::children(Some(model), "inputs", "input") {
            let name = settings::text(Some(input), "name")
                .filter(|n| !n.is_empty())
                .ok_or_else(|| error("saved model input has no name"))?;
            if modeled.len() >= 4096 {
                return Err(error("saved model exceeds 4096 inputs"));
            }
            modeled.insert(name);
        }
        let selected = names(s, "rankedFieldSelection", "value")?
            .into_iter()
            .chain(names(s, "screenedFieldSelection", "value")?)
            .collect();
        Some((modeled, selected))
    } else if settings::element(s, "rankedFieldSelection").is_some()
        || settings::element(s, "screenedFieldSelection").is_some()
    {
        return Err(error("saved selection has no model inputs"));
    } else {
        None
    };
    Ok(Box::new(Apply { saved }))
}

impl Apply {
    fn filter(&self, inputs: &[Arc<Table>], model: Option<&Pmml>) -> Result<Table, OperatorError> {
        let input = inputs.first().ok_or_else(|| {
            OperatorError::Inputs("Feature Selection nugget needs one input".into())
        })?;
        let learned;
        let (modeled, selected) = if let Some(model) = model {
            let Pmml {
                model: Model::Mining(m),
                ..
            } = model
            else {
                return Err(error("nugget received a different model type"));
            };
            if m.header.algorithm.as_deref() != Some(ENVELOPE) {
                return Err(error("nugget received a different model type"));
            }
            learned = (
                m.header
                    .mining_schema
                    .iter()
                    .filter(|f| f.usage != Usage::Target)
                    .map(|f| f.name.clone())
                    .collect::<BTreeSet<_>>(),
                m.header
                    .mining_schema
                    .iter()
                    .filter(|f| f.usage == Usage::Active)
                    .map(|f| f.name.clone())
                    .collect::<BTreeSet<_>>(),
            );
            &learned
        } else if let Some(saved) = &self.saved {
            saved
        } else {
            return Err(error("nugget has no saved selection or trained model"));
        };
        let kept: Vec<_> = input
            .model()
            .fields()
            .iter()
            .enumerate()
            .filter(|(_, f)| !modeled.contains(&f.name) || selected.contains(&f.name))
            .collect();
        let schema =
            DataModel::new(kept.iter().map(|(_, f)| (*f).clone()).collect()).map_err(error)?;
        let rows = input
            .rows()
            .iter()
            .map(|row| {
                kept.iter()
                    .map(|(p, _)| row.get(*p).cloned().unwrap_or(Value::Null))
                    .collect()
            })
            .collect();
        Table::new(schema, rows).map_err(error)
    }
}

impl Operator for Apply {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }
    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        self.filter(inputs, None)
    }
    fn produce(
        &self,
        inputs: &[Arc<Table>],
        model: Option<Arc<Pmml>>,
        _context: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        self.filter(inputs, model.as_deref())
            .map(|t| NodeOutput::Data(Arc::new(t)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dms_strfmt::fastinfoset::{Node, QualifiedName};

    fn element(name: &str, children: Vec<Node>) -> Element {
        Element {
            name: QualifiedName {
                local: Arc::from(name),
                prefix: None,
                namespace: None,
            },
            namespace_declarations: Vec::new(),
            attributes: Vec::new(),
            children,
        }
    }
    fn property(name: &str, value: &str) -> Node {
        Node::Element(element(name, vec![Node::Text(Arc::from(value))]))
    }
    fn node(class: &str, properties: Vec<Node>) -> StreamNode {
        StreamNode {
            uid: "fs".into(),
            class: class.into(),
            extension: None,
            model_builder: None,
            name: None,
            label: None,
            position: None,
            properties_attachment: None,
            containers: Vec::new(),
            settings: Some(element("properties", properties)),
            binning_output: None,
            subdiagram: None,
        }
    }

    #[test]
    fn bad_settings_are_errors_and_model_node_dispatches_as_nugget() {
        for (name, value) in [
            ("criteria", "unknown"),
            ("fieldSelectionMode", "unknown"),
            ("maxMissingValues", "NaN"),
            ("minStdDev", "-0.1"),
            ("screenVariability", "yes"),
            ("useEvents", "true"),
        ] {
            assert!(
                build(&node(
                    "BuildFeatureSelectionNode",
                    vec![property(name, value)]
                ))
                .is_err(),
                "{name}"
            );
        }
        for n in ["-1", "1001", "9223372036854775807"] {
            assert!(
                build(&node(
                    "BuildFeatureSelectionNode",
                    vec![
                        property("fieldSelectionMode", "topN"),
                        property("topNFields", n)
                    ]
                ))
                .is_err()
            );
        }
        let factory = crate::registry::factory("feature_selection").unwrap();
        let n = node(
            "com.spss.clementine.component.node.FeatureSelectionModelNode",
            Vec::new(),
        );
        let op = factory(&n).unwrap();
        let input = Arc::new(Table::new(DataModel::default(), Vec::new()).unwrap());
        let message = format!(
            "{:?}",
            op.run(&[input], &ExecContext::default()).unwrap_err()
        );
        assert!(
            message.contains("nugget has no saved selection"),
            "{message}"
        );
    }

    #[test]
    #[cfg(feature = "corpus")]
    fn learned_statistics_and_screens_match_saved_model_details() {
        use algo::{Level, ScreenReason};
        use std::path::Path;
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let mut cases: Vec<_> = std::fs::read_dir(root.join("oracle/models_stats"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("fs_"))
            })
            .collect();
        cases.sort();
        assert_eq!(cases.len(), 15);
        for case in cases {
            let name = case.file_name().unwrap().to_string_lossy();
            let archive = dms_strfmt::container::read_archive(
                &std::fs::read(root.join(format!("corpus/models_stats/{name}.str"))).unwrap(),
                &dms_strfmt::ArchiveLimits::default(),
            )
            .unwrap();
            let stream = dms_stream::Stream::from_document(&archive.document).unwrap();
            let context = ExecContext {
                files: crate::FileResolver {
                    data_dirs: vec![root.join("corpus/models_stats")],
                },
                attachments: Arc::new(
                    archive
                        .attachments
                        .into_iter()
                        .map(|a| (a.name, Arc::from(a.bytes)))
                        .collect(),
                ),
                stream_properties: stream.properties.clone().map(Arc::new),
                ..ExecContext::default()
            };
            let type_node = stream
                .diagram
                .nodes
                .iter()
                .find(|n| n.type_identifier() == "TypeNode")
                .unwrap();
            let results = crate::Plan::new(&stream).evaluate(&[type_node.uid.as_str()], &context);
            let table = results
                .get(&type_node.uid)
                .unwrap()
                .as_ref()
                .unwrap()
                .data()
                .unwrap();
            let builder = stream
                .diagram
                .nodes
                .iter()
                .find(|n| n.type_identifier() == "BuildFeatureSelectionNode")
                .unwrap();
            let spec = build_spec(builder).unwrap();
            let (_, _, fitted) = spec.fit(table).unwrap();
            let nugget = stream
                .diagram
                .nodes
                .iter()
                .find(|n| n.type_identifier() == "ApplyFeatureSelectionNode")
                .unwrap();
            let saved = settings::element(nugget.settings.as_ref(), "model").unwrap();
            let ranked = settings::children(Some(saved), "ranked", "rankedField");
            assert_eq!(ranked.len(), fitted.ranked.len(), "{name}: ranked count");
            for expected in &ranked {
                let field_name = settings::text(Some(expected), "field/name").unwrap();
                let actual = fitted.ranked.iter().find(|f| f.name == field_name).unwrap();
                let p = settings::text(Some(expected), "pvalue")
                    .unwrap()
                    .parse::<f64>()
                    .unwrap();
                let importance = settings::text(Some(expected), "ivalue")
                    .unwrap()
                    .parse::<f64>()
                    .unwrap();
                assert!(
                    actual.importance == importance
                        || (actual.importance - importance).abs() < 2e-10,
                    "{name}/{field_name}: importance {} != {importance}",
                    actual.importance
                );
                // Saved extreme correlation tails lose precision via CDF subtraction. Absolute
                // tolerance verifies ordinary p-values while the unit tests verify the tails.
                assert!(
                    actual.p_value == p || (actual.p_value - p).abs() < 2e-10,
                    "{name}/{field_name}: p {} != {p}",
                    actual.p_value
                );
                let level = match settings::integer(Some(expected), "level").unwrap().unwrap() {
                    0 => Level::Important,
                    1 => Level::Marginal,
                    2 => Level::Unimportant,
                    _ => panic!("unexpected saved level"),
                };
                assert_eq!(actual.level, level, "{name}/{field_name}: band");
                let label = match level {
                    Level::Important => "importantLabel",
                    Level::Marginal => "marginalLabel",
                    Level::Unimportant => "unimportantLabel",
                };
                assert_eq!(
                    Some(actual.label.as_str()),
                    saved.attribute(label),
                    "{name}/{field_name}: label"
                );
            }
            // Numeric rescalings expose floating-point ordering differences documented as GAPs.
            if name != "fs_continuous_target" && name != "fs_screen_cv" && name != "fs_screen_sd" {
                assert_eq!(
                    fitted
                        .ranked
                        .iter()
                        .map(|f| f.name.clone())
                        .collect::<Vec<_>>(),
                    ranked
                        .iter()
                        .map(|e| settings::text(Some(e), "field/name").unwrap())
                        .collect::<Vec<_>>(),
                    "{name}: rank order"
                );
            }
            let screened = settings::children(Some(saved), "screened", "screenedField");
            assert_eq!(
                screened.len(),
                fitted.screened.len(),
                "{name}: screened count"
            );
            for expected in screened {
                let field_name = settings::text(Some(expected), "field/name").unwrap();
                let actual = fitted
                    .screened
                    .iter()
                    .find(|f| f.name == field_name)
                    .unwrap();
                let reason = match expected.attribute("reason").unwrap_or("") {
                    "manyCategs" => ScreenReason::Categories,
                    "manyConst" => ScreenReason::Dominance,
                    "manyMissing" => ScreenReason::Missing,
                    "smallVar" => ScreenReason::Variation,
                    "smallStdDev" => ScreenReason::StandardDeviation,
                    "" => ScreenReason::Identifier,
                    _ => panic!("unexpected screen reason"),
                };
                assert_eq!(actual.reason, reason, "{name}/{field_name}");
            }
        }
    }
}
