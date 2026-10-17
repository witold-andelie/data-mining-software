//! XML parser for PMML 4.x documents (`roxmltree`, namespace-aware, bounded recursion).

use crate::{
    BooleanOperator, Cluster, ClusteringField, ClusteringModel, CompareFunction, ComparisonMeasure,
    DataField, DataType, DefineFunction, DerivedField, Expr, GeneralModelType,
    GeneralRegressionModel, InvalidTreatment, MeasureKind, Metric, MiningField, MiningFunction,
    MiningModel, MiningSegment, MissingValueStrategy, Model, ModelClass, ModelHeader,
    NoTrueChildStrategy, Node, OpType, Operator, Outliers, OutputField, PCell, PPCell, Pmml,
    PmmlError, Predicate, RegressionModel, RegressionTable, ScoreDistribution, TreeModel, Usage,
    Value,
};
use roxmltree::Document;

/// Maximum nesting depth for recursive PMML structures (`Node`, `CompoundPredicate`, `Apply`).
const MAX_DEPTH: usize = 64;

const MODEL_ELEMENTS: &[&str] = &[
    "TreeModel",
    "ClusteringModel",
    "GeneralRegressionModel",
    "RegressionModel",
    "NeuralNetwork",
    "NaiveBayesModel",
    "RuleSetModel",
    "SupportVectorMachineModel",
    "NearestNeighborModel",
    "MiningModel",
    "AssociationModel",
    "BaselineModel",
    "BayesianNetworkModel",
    "CoxRegressionModel",
    "GaussianProcessModel",
    "Scorecard",
    "SequenceModel",
    "TextModel",
    "TimeSeriesModel",
    "AnomalyDetectionModel",
];

pub(crate) fn parse(xml: &str) -> Result<Pmml, PmmlError> {
    if xml.len() > 64 * 1024 * 1024 {
        return Err(PmmlError::Parse("PMML document size limit exceeded".into()));
    }
    let doc = Document::parse(xml).map_err(|e| PmmlError::Parse(e.to_string()))?;
    let root = doc.root_element();
    if root.tag_name().name() != "PMML" {
        return Err(PmmlError::Parse(format!(
            "root element must be PMML, got {:?}",
            root.tag_name().name()
        )));
    }
    if let Some(ns) = root.tag_name().namespace()
        && !ns.starts_with("http://www.dmg.org/PMML-")
    {
        return Err(PmmlError::Parse(format!("unsupported namespace {ns:?}")));
    }
    let version = req_attr(root, "version")?.to_string();
    if !version.starts_with("4.") {
        return Err(PmmlError::Parse(format!(
            "expected PMML version 4.x, got {version:?}"
        )));
    }

    let mut data_dictionary: Option<Vec<DataField>> = None;
    let mut transformations = Vec::new();
    let mut functions = Vec::new();
    let mut models = Vec::new();

    for child in root.children().filter(roxmltree::Node::is_element) {
        let name = child.tag_name().name();
        if is_skipped_element(name) || name == "Header" || name == "MiningBuildTask" {
            continue;
        }
        match name {
            "DataDictionary" => {
                if data_dictionary.is_some() {
                    return Err(PmmlError::Parse("duplicate DataDictionary".into()));
                }
                data_dictionary = Some(parse_data_dictionary(child)?);
            }
            "TransformationDictionary" => {
                parse_transformation_dictionary(child, &mut transformations, &mut functions)?;
            }
            tag if MODEL_ELEMENTS.contains(&tag) => {
                models.push((
                    parse_model(child, &transformations)?,
                    child.children().any(|n| {
                        n.has_tag_name("Extension") && n.attribute("name") == Some("boost")
                    }),
                ));
            }
            other => {
                return Err(PmmlError::Parse(format!(
                    "unexpected element {other:?} under PMML"
                )));
            }
        }
    }

    let data_dictionary =
        data_dictionary.ok_or_else(|| PmmlError::Parse("missing DataDictionary".into()))?;
    let model = if models.len() == 1 {
        models.remove(0).0
    } else if models.len() > 1
        && models
            .iter()
            .all(|(m, boost)| *boost && matches!(m, Model::Tree(_) | Model::RuleSet(_)))
    {
        // Modeler C5.0 exports adjacent models, each carrying the spss boost extension.
        // Algorithms Guide p.51: confidence voting, NOT the extension's rounded value.
        let mut header = crate::score::model_header(
            &models
                .first()
                .ok_or_else(|| PmmlError::Parse("missing model".into()))?
                .0,
        )
        .ok_or_else(|| PmmlError::Parse("missing model header".into()))?
        .clone();
        header.algorithm = Some("C5".into());
        header.local_transformations.clear();
        header.mining_schema.retain(|f| f.usage != Usage::Active);
        let segments = models
            .into_iter()
            .map(|(m, _)| MiningSegment {
                predicate: Predicate::True,
                model: Box::new(m),
            })
            .collect();
        Model::Mining(MiningModel {
            header,
            method: "c50ConfidenceVote".into(),
            segments,
        })
    } else if models.len() > 1
        && models
            .iter()
            .all(|(m, boost)| !*boost && matches!(m, Model::GeneralRegression(_)))
    {
        // Modeler PCA/Factor (`oracle/models_next/factor_*`): one regression per factor.
        let header = crate::score::model_header(
            &models
                .first()
                .ok_or_else(|| PmmlError::Parse("missing model".into()))?
                .0,
        )
        .ok_or_else(|| PmmlError::Parse("missing model header".into()))?
        .clone();
        let segments = models
            .into_iter()
            .map(|(m, _)| MiningSegment {
                predicate: Predicate::True,
                model: Box::new(m),
            })
            .collect();
        Model::Mining(MiningModel {
            header,
            method: crate::FACTOR_SCORES.into(),
            segments,
        })
    } else {
        return Err(PmmlError::Parse(
            "missing model or ambiguous multiple model elements in PMML".into(),
        ));
    };

    Ok(Pmml {
        version,
        data_dictionary,
        transformations,
        functions,
        model,
    })
}

fn is_skipped_element(name: &str) -> bool {
    matches!(
        name,
        "Extension"
            | "ModelStats"
            | "ModelExplanation"
            | "ModelVerification"
            | "Targets"
            | "Taxonomy"
    ) || name.starts_with("X-")
}

pub(crate) fn req_attr<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    attr: &str,
) -> Result<&'a str, PmmlError> {
    node.attribute(attr).ok_or_else(|| {
        PmmlError::Parse(format!(
            "element <{}> is missing attribute {attr:?}",
            node.tag_name().name()
        ))
    })
}

pub(crate) fn parse_f64(text: &str, what: &str) -> Result<f64, PmmlError> {
    let val = text
        .trim()
        .parse::<f64>()
        .map_err(|_| PmmlError::Parse(format!("invalid number {text:?} for {what}")))?;
    if !val.is_finite() {
        return Err(PmmlError::Parse(format!(
            "non-finite number {text:?} for {what}"
        )));
    }
    Ok(val)
}

pub(crate) fn opt_f64_attr(
    node: roxmltree::Node<'_, '_>,
    attr: &str,
) -> Result<Option<f64>, PmmlError> {
    node.attribute(attr).map(|s| parse_f64(s, attr)).transpose()
}

fn req_f64_attr(node: roxmltree::Node<'_, '_>, attr: &str) -> Result<f64, PmmlError> {
    parse_f64(req_attr(node, attr)?, attr)
}

fn parse_optype(text: &str) -> Result<OpType, PmmlError> {
    match text {
        "categorical" => Ok(OpType::Categorical),
        "ordinal" => Ok(OpType::Ordinal),
        "continuous" => Ok(OpType::Continuous),
        other => Err(PmmlError::Parse(format!("invalid optype {other:?}"))),
    }
}

fn parse_data_type(text: &str) -> DataType {
    match text {
        "string" => DataType::String,
        "integer" => DataType::Integer,
        "float" => DataType::Float,
        "double" => DataType::Double,
        "boolean" => DataType::Boolean,
        "date" => DataType::Date,
        "time" => DataType::Time,
        "dateTime" => DataType::DateTime,
        other => DataType::Other(other.to_string()),
    }
}

fn parse_data_dictionary(node: roxmltree::Node<'_, '_>) -> Result<Vec<DataField>, PmmlError> {
    let mut fields = Vec::new();
    for child in node.children().filter(roxmltree::Node::is_element) {
        let tag = child.tag_name().name();
        if is_skipped_element(tag) {
            continue;
        }
        if tag != "DataField" {
            return Err(PmmlError::Parse(format!(
                "unexpected element {tag:?} in DataDictionary"
            )));
        }
        let name = req_attr(child, "name")?.to_string();
        let optype = parse_optype(req_attr(child, "optype")?)?;
        let data_type = parse_data_type(req_attr(child, "dataType")?);
        let mut values = Vec::new();
        let mut missing_values = Vec::new();
        let mut flag_true_value = None;
        for val_node in child.children().filter(roxmltree::Node::is_element) {
            if val_node.tag_name().name() == "Value" {
                let prop = val_node.attribute("property").unwrap_or("valid");
                let value = req_attr(val_node, "value")?.to_string();
                match prop {
                    "valid" => values.push(value),
                    "missing" => missing_values.push(value),
                    _ => {}
                }
            } else if val_node.tag_name().name() == "Extension"
                && val_node.attribute("name") == Some("flagTypeTrueValue")
            {
                if flag_true_value.is_some() {
                    return Err(PmmlError::Parse(format!(
                        "DataField {name:?} has two flagTypeTrueValue extensions"
                    )));
                }
                flag_true_value = Some(req_attr(val_node, "value")?.to_string());
            }
        }
        fields.push(DataField {
            name,
            optype,
            data_type,
            values,
            missing_values,
            flag_true_value,
        });
    }
    Ok(fields)
}

fn parse_transformation_dictionary(
    node: roxmltree::Node<'_, '_>,
    transformations: &mut Vec<DerivedField>,
    functions: &mut Vec<DefineFunction>,
) -> Result<(), PmmlError> {
    for child in node.children().filter(roxmltree::Node::is_element) {
        let tag = child.tag_name().name();
        if is_skipped_element(tag) {
            continue;
        }
        match tag {
            "DerivedField" => transformations.push(parse_derived_field(child)?),
            "DefineFunction" => functions.push(parse_define_function(child)?),
            other => {
                return Err(PmmlError::Parse(format!(
                    "unexpected element {other:?} in TransformationDictionary"
                )));
            }
        }
    }
    Ok(())
}

fn parse_derived_field(node: roxmltree::Node<'_, '_>) -> Result<DerivedField, PmmlError> {
    let name = req_attr(node, "name")?.to_string();
    let data_type = parse_data_type(req_attr(node, "dataType")?);
    let optype = parse_optype(req_attr(node, "optype")?)?;
    let mut expr = None;
    for child in node.children().filter(roxmltree::Node::is_element) {
        let tag = child.tag_name().name();
        if is_skipped_element(tag) || tag == "Value" || tag == "Interval" {
            continue;
        }
        if expr.is_some() {
            return Err(PmmlError::Parse(format!(
                "DerivedField {name:?} has multiple expressions"
            )));
        }
        expr = Some(parse_expr(child, 0)?);
    }
    let expr = expr.ok_or_else(|| {
        PmmlError::Parse(format!("DerivedField {name:?} is missing an expression"))
    })?;
    Ok(DerivedField {
        name,
        data_type,
        optype,
        expr,
    })
}

fn parse_define_function(node: roxmltree::Node<'_, '_>) -> Result<DefineFunction, PmmlError> {
    let name = req_attr(node, "name")?.to_string();
    let mut parameters = Vec::new();
    let mut body = None;
    for child in node.children().filter(roxmltree::Node::is_element) {
        let tag = child.tag_name().name();
        if is_skipped_element(tag) {
            continue;
        }
        if tag == "ParameterField" {
            parameters.push(req_attr(child, "name")?.to_string());
        } else {
            if body.is_some() {
                return Err(PmmlError::Parse(format!(
                    "DefineFunction {name:?} has multiple body expressions"
                )));
            }
            body = Some(parse_expr(child, 0)?);
        }
    }
    let body =
        body.ok_or_else(|| PmmlError::Parse(format!("DefineFunction {name:?} is missing a body")))?;
    Ok(DefineFunction {
        name,
        parameters,
        body,
    })
}

pub(crate) fn parse_value_str(text: &str) -> Value {
    if let Ok(n) = text.trim().parse::<f64>()
        && n.is_finite()
    {
        Value::Number(n)
    } else {
        Value::Text(text.to_string())
    }
}

fn parse_outliers(text: &str) -> Result<Outliers, PmmlError> {
    match text {
        "asIs" => Ok(Outliers::AsIs),
        "asMissingValues" => Ok(Outliers::AsMissingValues),
        "asExtremeValues" => Ok(Outliers::AsExtremeValues),
        other => Err(PmmlError::Parse(format!(
            "invalid outliers treatment {other:?}"
        ))),
    }
}

pub(crate) fn parse_expr(node: roxmltree::Node<'_, '_>, depth: usize) -> Result<Expr, PmmlError> {
    if depth > MAX_DEPTH {
        return Err(PmmlError::Parse(
            "expression nesting depth limit exceeded".into(),
        ));
    }
    match node.tag_name().name() {
        "Constant" => {
            if node.attribute("missing") == Some("true") {
                return Ok(Expr::Constant(Value::Missing));
            }
            let text = node.text().unwrap_or("");
            let val = match node.attribute("dataType") {
                Some("string") => Value::Text(text.to_string()),
                Some("integer" | "float" | "double") => Value::Number(parse_f64(text, "Constant")?),
                _ => parse_value_str(text),
            };
            Ok(Expr::Constant(val))
        }
        "FieldRef" => {
            let field = req_attr(node, "field")?.to_string();
            let map_missing_to = node.attribute("mapMissingTo").map(parse_value_str);
            Ok(Expr::FieldRef {
                field,
                map_missing_to,
            })
        }
        "NormContinuous" => {
            let field = req_attr(node, "field")?.to_string();
            let outliers = node
                .attribute("outliers")
                .map_or(Ok(Outliers::AsIs), parse_outliers)?;
            let map_missing_to = opt_f64_attr(node, "mapMissingTo")?;
            let mut points = Vec::new();
            for child in node.children().filter(roxmltree::Node::is_element) {
                let tag = child.tag_name().name();
                if is_skipped_element(tag) {
                    continue;
                }
                if tag != "LinearNorm" {
                    return Err(PmmlError::Parse(format!(
                        "unexpected element {tag:?} in NormContinuous"
                    )));
                }
                let orig = req_f64_attr(child, "orig")?;
                let norm = req_f64_attr(child, "norm")?;
                points.push((orig, norm));
            }
            if points.len() < 2 {
                return Err(PmmlError::Parse(
                    "NormContinuous requires at least two LinearNorm points".into(),
                ));
            }
            Ok(Expr::NormContinuous {
                field,
                points,
                outliers,
                map_missing_to,
            })
        }
        "NormDiscrete" => {
            let field = req_attr(node, "field")?.to_string();
            let value = req_attr(node, "value")?.to_string();
            let map_missing_to = opt_f64_attr(node, "mapMissingTo")?;
            Ok(Expr::NormDiscrete {
                field,
                value,
                map_missing_to,
            })
        }
        "Apply" => {
            let function = req_attr(node, "function")?.to_string();
            let map_missing_to = node
                .attribute("mapMissingTo")
                .or_else(|| node.attribute("defaultValue"))
                .map(parse_value_str);
            let mut args = Vec::new();
            for child in node.children().filter(roxmltree::Node::is_element) {
                if is_skipped_element(child.tag_name().name()) {
                    continue;
                }
                args.push(parse_expr(child, depth + 1)?);
            }
            Ok(Expr::Apply {
                function,
                args,
                map_missing_to,
            })
        }
        "MapValues" => {
            let output = req_attr(node, "outputColumn")?.to_string();
            let default = node.attribute("defaultValue").map(parse_value_str);
            let map_missing_to = node.attribute("mapMissingTo").map(parse_value_str);
            let mut pairs = Vec::new();
            let mut rows = Vec::new();
            for child in node.children().filter(roxmltree::Node::is_element) {
                match child.tag_name().name() {
                    "FieldColumnPair" => pairs.push((
                        req_attr(child, "field")?.to_string(),
                        req_attr(child, "column")?.to_string(),
                    )),
                    "InlineTable" => {
                        for row in child
                            .children()
                            .filter(|n| n.is_element() && n.tag_name().name() == "row")
                        {
                            rows.push(
                                row.children()
                                    .filter(roxmltree::Node::is_element)
                                    .map(|cell| {
                                        (
                                            cell.tag_name().name().to_string(),
                                            cell.text().unwrap_or("").to_string(),
                                        )
                                    })
                                    .collect(),
                            );
                        }
                    }
                    tag if is_skipped_element(tag) => {}
                    tag => {
                        return Err(PmmlError::Parse(format!(
                            "unsupported element {tag:?} in MapValues"
                        )));
                    }
                }
            }
            if pairs.is_empty() {
                return Err(PmmlError::Parse("MapValues without FieldColumnPair".into()));
            }
            Ok(Expr::MapValues {
                pairs,
                output,
                rows,
                default,
                map_missing_to,
            })
        }
        other => Err(PmmlError::Parse(format!(
            "unsupported expression element {other:?}"
        ))),
    }
}

fn parse_model(
    node: roxmltree::Node<'_, '_>,
    dictionary: &[crate::DerivedField],
) -> Result<Model, PmmlError> {
    if node
        .ancestors()
        .filter(|n| MODEL_ELEMENTS.contains(&n.tag_name().name()))
        .count()
        > MAX_DEPTH
    {
        return Err(PmmlError::Parse(
            "model nesting depth limit exceeded".into(),
        ));
    }
    match node.tag_name().name() {
        "TreeModel" => Ok(Model::Tree(parse_tree_model(node)?)),
        "RuleSetModel" => Ok(Model::RuleSet(crate::rules::parse(node)?)),
        "ClusteringModel" => Ok(Model::Clustering(parse_clustering_model(node)?)),
        "GeneralRegressionModel" => Ok(Model::GeneralRegression(parse_general_regression_model(
            node,
        )?)),
        "RegressionModel" => Ok(Model::Regression(parse_regression_model(node)?)),
        "NeuralNetwork" => Ok(Model::Neural(crate::neural::parse(node, dictionary)?)),
        "MiningModel" => Ok(Model::Mining(parse_mining_model(node, dictionary)?)),
        other => Ok(Model::Unsupported(other.to_string())),
    }
}

pub(crate) fn parse_model_header(node: roxmltree::Node<'_, '_>) -> Result<ModelHeader, PmmlError> {
    let name = node.attribute("modelName").map(str::to_string);
    let function = match req_attr(node, "functionName")? {
        "classification" => MiningFunction::Classification,
        "regression" => MiningFunction::Regression,
        "clustering" => MiningFunction::Clustering,
        other => MiningFunction::Other(other.to_string()),
    };
    let algorithm = node.attribute("algorithmName").map(str::to_string);

    let mut mining_schema = None;
    let mut output = Vec::new();
    let mut local_transformations = Vec::new();

    for child in node.children().filter(roxmltree::Node::is_element) {
        match child.tag_name().name() {
            "MiningSchema" => {
                if mining_schema.is_some() {
                    return Err(PmmlError::Parse("duplicate MiningSchema".into()));
                }
                mining_schema = Some(parse_mining_schema(child)?);
            }
            "Output" => {
                output = parse_output(child)?;
            }
            "LocalTransformations" => {
                for df in child.children().filter(roxmltree::Node::is_element) {
                    if is_skipped_element(df.tag_name().name()) {
                        continue;
                    }
                    if df.tag_name().name() != "DerivedField" {
                        return Err(PmmlError::Parse(format!(
                            "unexpected element {:?} in LocalTransformations",
                            df.tag_name().name()
                        )));
                    }
                    local_transformations.push(parse_derived_field(df)?);
                }
            }
            _ => {}
        }
    }

    let mining_schema =
        mining_schema.ok_or_else(|| PmmlError::Parse("model is missing MiningSchema".into()))?;

    Ok(ModelHeader {
        name,
        function,
        algorithm,
        mining_schema,
        output,
        local_transformations,
    })
}

fn parse_mining_schema(node: roxmltree::Node<'_, '_>) -> Result<Vec<MiningField>, PmmlError> {
    let mut fields = Vec::new();
    for child in node.children().filter(roxmltree::Node::is_element) {
        let tag = child.tag_name().name();
        if is_skipped_element(tag) {
            continue;
        }
        if tag != "MiningField" {
            return Err(PmmlError::Parse(format!(
                "unexpected element {tag:?} in MiningSchema"
            )));
        }
        let name = req_attr(child, "name")?.to_string();
        let usage = match child.attribute("usageType").unwrap_or("active") {
            "active" => Usage::Active,
            "target" | "predicted" => Usage::Target,
            "supplementary" => Usage::Supplementary,
            _ => Usage::Other,
        };
        let missing_replacement = child
            .attribute("missingValueReplacement")
            .map(str::to_string);
        let outliers = child
            .attribute("outliers")
            .map_or(Ok(Outliers::AsIs), parse_outliers)?;
        let low_value = opt_f64_attr(child, "lowValue")?;
        let high_value = opt_f64_attr(child, "highValue")?;
        let invalid_treatment = match child
            .attribute("invalidValueTreatment")
            .unwrap_or("returnInvalid")
        {
            "returnInvalid" => InvalidTreatment::ReturnInvalid,
            "asIs" => InvalidTreatment::AsIs,
            "asMissing" => InvalidTreatment::AsMissing,
            other => {
                return Err(PmmlError::Parse(format!(
                    "invalid invalidValueTreatment {other:?}"
                )));
            }
        };
        fields.push(MiningField {
            name,
            usage,
            missing_replacement,
            outliers,
            low_value,
            high_value,
            invalid_treatment,
        });
    }
    Ok(fields)
}

fn parse_output(node: roxmltree::Node<'_, '_>) -> Result<Vec<OutputField>, PmmlError> {
    let mut fields = Vec::new();
    for child in node.children().filter(roxmltree::Node::is_element) {
        let tag = child.tag_name().name();
        if is_skipped_element(tag) {
            continue;
        }
        if tag != "OutputField" {
            return Err(PmmlError::Parse(format!(
                "unexpected element {tag:?} in Output"
            )));
        }
        let name = req_attr(child, "name")?.to_string();
        let feature = child
            .attribute("feature")
            .unwrap_or("predictedValue")
            .to_string();
        let value = child.attribute("value").map(str::to_string);
        fields.push(OutputField {
            name,
            feature,
            value,
        });
    }
    Ok(fields)
}

fn parse_tree_model(node: roxmltree::Node<'_, '_>) -> Result<TreeModel, PmmlError> {
    let header = parse_model_header(node)?;
    let missing_value_strategy = match node.attribute("missingValueStrategy").unwrap_or("none") {
        "none" => MissingValueStrategy::None,
        "lastPrediction" => MissingValueStrategy::LastPrediction,
        "nullPrediction" => MissingValueStrategy::NullPrediction,
        "defaultChild" => MissingValueStrategy::DefaultChild,
        "weightedConfidence" => MissingValueStrategy::WeightedConfidence,
        "aggregateNodes" => MissingValueStrategy::AggregateNodes,
        other => {
            return Err(PmmlError::Parse(format!(
                "invalid missingValueStrategy {other:?}"
            )));
        }
    };
    let no_true_child_strategy = match node
        .attribute("noTrueChildStrategy")
        .unwrap_or("returnNullPrediction")
    {
        "returnNullPrediction" => NoTrueChildStrategy::ReturnNullPrediction,
        "returnLastPrediction" => NoTrueChildStrategy::ReturnLastPrediction,
        other => {
            return Err(PmmlError::Parse(format!(
                "invalid noTrueChildStrategy {other:?}"
            )));
        }
    };

    let mut root = None;
    for child in node.children().filter(roxmltree::Node::is_element) {
        if child.tag_name().name() == "Node" {
            if root.is_some() {
                return Err(PmmlError::Parse("TreeModel has multiple root Nodes".into()));
            }
            root = Some(parse_tree_node(child, 0)?);
        }
    }
    let root = root.ok_or_else(|| PmmlError::Parse("TreeModel is missing root Node".into()))?;

    Ok(TreeModel {
        header,
        missing_value_strategy,
        no_true_child_strategy,
        root,
    })
}

fn parse_tree_node(node: roxmltree::Node<'_, '_>, depth: usize) -> Result<Node, PmmlError> {
    if depth > MAX_DEPTH {
        return Err(PmmlError::Parse(
            "TreeModel node depth limit exceeded".into(),
        ));
    }
    let id = node.attribute("id").map(str::to_string);
    let score = node.attribute("score").map(str::to_string);
    let record_count = opt_f64_attr(node, "recordCount")?;
    let default_child = node.attribute("defaultChild").map(str::to_string);

    let mut predicate = None;
    let mut distribution = Vec::new();
    let mut children = Vec::new();

    for child in node.children().filter(roxmltree::Node::is_element) {
        let tag = child.tag_name().name();
        if is_skipped_element(tag) || tag == "Partition" || tag == "EmbeddedModel" {
            continue;
        }
        match tag {
            "True" | "False" | "SimplePredicate" | "SimpleSetPredicate" | "CompoundPredicate" => {
                if predicate.is_some() {
                    return Err(PmmlError::Parse("Node has multiple predicates".into()));
                }
                predicate = Some(parse_predicate(child, 0)?);
            }
            "ScoreDistribution" => {
                let value = req_attr(child, "value")?.to_string();
                let record_count = req_f64_attr(child, "recordCount")?;
                let probability = opt_f64_attr(child, "probability")?;
                let confidence = opt_f64_attr(child, "confidence")?;
                distribution.push(ScoreDistribution {
                    value,
                    record_count,
                    probability,
                    confidence,
                });
            }
            "Node" => {
                children.push(parse_tree_node(child, depth + 1)?);
            }
            other => {
                return Err(PmmlError::Parse(format!(
                    "unexpected element {other:?} in Node"
                )));
            }
        }
    }

    let predicate =
        predicate.ok_or_else(|| PmmlError::Parse("Node is missing a predicate".into()))?;

    Ok(Node {
        id,
        score,
        record_count,
        default_child,
        predicate,
        distribution,
        children,
    })
}

pub(crate) fn parse_predicate(
    node: roxmltree::Node<'_, '_>,
    depth: usize,
) -> Result<Predicate, PmmlError> {
    if depth > MAX_DEPTH {
        return Err(PmmlError::Parse(
            "predicate nesting depth limit exceeded".into(),
        ));
    }
    match node.tag_name().name() {
        "True" => Ok(Predicate::True),
        "False" => Ok(Predicate::False),
        "SimplePredicate" => {
            let field = req_attr(node, "field")?.to_string();
            let operator = match req_attr(node, "operator")? {
                "equal" => Operator::Equal,
                "notEqual" => Operator::NotEqual,
                "lessThan" => Operator::LessThan,
                "lessOrEqual" => Operator::LessOrEqual,
                "greaterThan" => Operator::GreaterThan,
                "greaterOrEqual" => Operator::GreaterOrEqual,
                "isMissing" => Operator::IsMissing,
                "isNotMissing" => Operator::IsNotMissing,
                other => {
                    return Err(PmmlError::Parse(format!(
                        "invalid SimplePredicate operator {other:?}"
                    )));
                }
            };
            let value = node.attribute("value").map(str::to_string);
            if !matches!(operator, Operator::IsMissing | Operator::IsNotMissing) && value.is_none()
            {
                return Err(PmmlError::Parse(format!(
                    "SimplePredicate with operator {operator:?} requires a value attribute"
                )));
            }
            Ok(Predicate::Simple {
                field,
                operator,
                value,
            })
        }
        "SimpleSetPredicate" => {
            let field = req_attr(node, "field")?.to_string();
            let is_in = match req_attr(node, "booleanOperator")? {
                "isIn" => true,
                "isNotIn" => false,
                other => {
                    return Err(PmmlError::Parse(format!(
                        "invalid SimpleSetPredicate booleanOperator {other:?}"
                    )));
                }
            };
            let array_node = node
                .children()
                .find(|c| c.is_element() && c.tag_name().name() == "Array")
                .ok_or_else(|| {
                    PmmlError::Parse("SimpleSetPredicate is missing an Array child".into())
                })?;
            let values = parse_array_strings(array_node)?;
            Ok(Predicate::SimpleSet {
                field,
                is_in,
                values,
            })
        }
        "CompoundPredicate" => {
            let operator = match req_attr(node, "booleanOperator")? {
                "and" => BooleanOperator::And,
                "or" => BooleanOperator::Or,
                "xor" => BooleanOperator::Xor,
                "surrogate" => BooleanOperator::Surrogate,
                other => {
                    return Err(PmmlError::Parse(format!(
                        "invalid CompoundPredicate booleanOperator {other:?}"
                    )));
                }
            };
            let mut predicates = Vec::new();
            for child in node.children().filter(roxmltree::Node::is_element) {
                if is_skipped_element(child.tag_name().name()) {
                    continue;
                }
                predicates.push(parse_predicate(child, depth + 1)?);
            }
            if predicates.len() < 2 {
                return Err(PmmlError::Parse(
                    "CompoundPredicate requires at least two child predicates".into(),
                ));
            }
            Ok(Predicate::Compound {
                operator,
                predicates,
            })
        }
        other => Err(PmmlError::Parse(format!("unexpected predicate {other:?}"))),
    }
}

/// Parses a PMML `<Array>` into strings, respecting quotes (`"..."`) and backslash-escaped quotes
/// (`"`), and validating `@n` when present.
fn parse_array_strings(node: roxmltree::Node<'_, '_>) -> Result<Vec<String>, PmmlError> {
    let text = node.text().unwrap_or("");
    let mut items = Vec::new();
    let mut chars = text.chars().peekable();

    while let Some(&ch) = chars.peek() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }
        if ch == '"' {
            chars.next();
            let mut token = String::new();
            let mut closed = false;
            let backslash = char::from(92u8);
            while let Some(c) = chars.next() {
                if c == backslash {
                    if let Some(&next) = chars.peek()
                        && (next == '"' || next == backslash)
                    {
                        token.push(next);
                        chars.next();
                    } else {
                        token.push(c);
                    }
                } else if c == '"' {
                    closed = true;
                    break;
                } else {
                    token.push(c);
                }
            }
            if !closed {
                return Err(PmmlError::Parse("unterminated quote in Array".into()));
            }
            items.push(token);
        } else {
            let mut token = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                token.push(c);
                chars.next();
            }
            items.push(token);
        }
    }

    if let Some(n_str) = node.attribute("n") {
        let expected = n_str
            .trim()
            .parse::<usize>()
            .map_err(|_| PmmlError::Parse(format!("invalid Array n={n_str:?}")))?;
        if items.len() != expected {
            return Err(PmmlError::Parse(format!(
                "Array n={expected} does not match element count {}",
                items.len()
            )));
        }
    }

    Ok(items)
}

fn parse_array_f64(node: roxmltree::Node<'_, '_>) -> Result<Vec<f64>, PmmlError> {
    let strings = parse_array_strings(node)?;
    strings
        .iter()
        .map(|s| parse_f64(s, "Array"))
        .collect::<Result<Vec<_>, _>>()
}

fn parse_compare_function(name: &str, scale: Option<f64>) -> Result<CompareFunction, PmmlError> {
    match name {
        "absDiff" => Ok(CompareFunction::AbsDiff),
        "gaussSim" => {
            let s = scale.ok_or_else(|| {
                PmmlError::Parse("gaussSim compareFunction requires similarityScale".into())
            })?;
            if s <= 0.0 {
                return Err(PmmlError::Parse("similarityScale must be positive".into()));
            }
            Ok(CompareFunction::GaussSim(s))
        }
        "delta" => Ok(CompareFunction::Delta),
        "equal" => Ok(CompareFunction::Equal),
        "table" => Ok(CompareFunction::Table),
        other => Err(PmmlError::Parse(format!(
            "invalid compareFunction {other:?}"
        ))),
    }
}

fn parse_clustering_model(node: roxmltree::Node<'_, '_>) -> Result<ClusteringModel, PmmlError> {
    let header = parse_model_header(node)?;
    let model_class = match req_attr(node, "modelClass")? {
        "centerBased" => ModelClass::CenterBased,
        "distributionBased" => ModelClass::DistributionBased,
        other => {
            return Err(PmmlError::Parse(format!(
                "invalid ClusteringModel modelClass {other:?}"
            )));
        }
    };

    let mut measure = None;
    let mut fields = Vec::new();
    let mut clusters = Vec::new();
    let mut missing_weights = Vec::new();

    for child in node.children().filter(roxmltree::Node::is_element) {
        let tag = child.tag_name().name();
        if is_skipped_element(tag)
            || matches!(tag, "MiningSchema" | "Output" | "LocalTransformations")
        {
            continue;
        }
        match tag {
            "ComparisonMeasure" => {
                measure = Some(parse_comparison_measure(child)?);
            }
            "ClusteringField" => {
                let field = req_attr(child, "field")?.to_string();
                let weight = opt_f64_attr(child, "fieldWeight")?.unwrap_or(1.0);
                let scale = opt_f64_attr(child, "similarityScale")?;
                let compare = child
                    .attribute("compareFunction")
                    .map(|cf| parse_compare_function(cf, scale))
                    .transpose()?;
                let is_center_field = match child.attribute("isCenterField").unwrap_or("true") {
                    "true" => true,
                    "false" => false,
                    other => {
                        return Err(PmmlError::Parse(format!("invalid isCenterField {other:?}")));
                    }
                };
                fields.push(ClusteringField {
                    field,
                    weight,
                    compare,
                    is_center_field,
                });
            }
            "Cluster" => {
                let id = child.attribute("id").map(str::to_string);
                let name = child.attribute("name").map(str::to_string);
                let size = opt_f64_attr(child, "size")?;
                let mut center = None;
                let mut variances = Vec::new();
                let mut category_counts = Vec::new();
                let mut mean_distance = None;
                for sub in child.children().filter(roxmltree::Node::is_element) {
                    match sub.tag_name().name() {
                        "Extension" if sub.attribute("name") == Some("meanClusterDistance") => {
                            mean_distance = opt_f64_attr(sub, "value")?;
                        }
                        "Array" if center.is_none() => center = Some(parse_array_f64(sub)?),
                        "Partition" => category_counts = parse_partition_counts(sub)?,
                        "Covariances" => variances = parse_diagonal(sub)?,
                        _ => {}
                    }
                }
                clusters.push(Cluster {
                    id,
                    name,
                    size,
                    center: center.unwrap_or_default(),
                    variances,
                    category_counts,
                    mean_distance,
                });
            }
            "MissingValueWeights" => {
                if let Some(arr) = child
                    .children()
                    .find(|c| c.is_element() && c.tag_name().name() == "Array")
                {
                    missing_weights = parse_array_f64(arr)?;
                }
            }
            other => {
                return Err(PmmlError::Parse(format!(
                    "unexpected element {other:?} in ClusteringModel"
                )));
            }
        }
    }

    let measure = measure
        .ok_or_else(|| PmmlError::Parse("ClusteringModel is missing ComparisonMeasure".into()))?;

    Ok(ClusteringModel {
        header,
        model_class,
        measure,
        fields,
        clusters,
        missing_weights,
        anomaly_cutoff: None,
    })
}

fn parse_comparison_measure(node: roxmltree::Node<'_, '_>) -> Result<ComparisonMeasure, PmmlError> {
    let kind = match req_attr(node, "kind")? {
        "distance" => MeasureKind::Distance,
        "similarity" => MeasureKind::Similarity,
        other => {
            return Err(PmmlError::Parse(format!(
                "invalid ComparisonMeasure kind {other:?}"
            )));
        }
    };
    let scale = opt_f64_attr(node, "similarityScale")?;
    let compare = parse_compare_function(
        node.attribute("compareFunction").unwrap_or("absDiff"),
        scale,
    )?;

    let mut metric = None;
    let mut log_likelihood = false;
    for child in node.children().filter(roxmltree::Node::is_element) {
        let tag = child.tag_name().name();
        if tag == "Extension" && child.attribute("name") == Some("x-measure") {
            log_likelihood = child.attribute("value") == Some("loglikelihood");
        }
        if is_skipped_element(tag) {
            continue;
        }
        let m = match tag {
            "euclidean" => Metric::Euclidean,
            "squaredEuclidean" => Metric::SquaredEuclidean,
            "cityBlock" => Metric::CityBlock,
            "chebychev" => Metric::Chebychev,
            "minkowski" => {
                let p = req_f64_attr(child, "p")?;
                if p <= 0.0 {
                    return Err(PmmlError::Parse("minkowski p must be positive".into()));
                }
                Metric::Minkowski(p)
            }
            other => {
                return Err(PmmlError::Parse(format!(
                    "unsupported ComparisonMeasure metric {other:?}"
                )));
            }
        };
        metric = Some(m);
    }

    let metric = metric
        .ok_or_else(|| PmmlError::Parse("ComparisonMeasure is missing metric element".into()))?;

    Ok(ComparisonMeasure {
        kind,
        metric,
        compare,
        log_likelihood,
    })
}

/// A cluster's `Partition`: the counts array of each `PartitionFieldStats`, by field.
fn parse_partition_counts(
    node: roxmltree::Node<'_, '_>,
) -> Result<Vec<(String, Vec<f64>)>, PmmlError> {
    let mut counts = Vec::new();
    for stats in node
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "PartitionFieldStats")
    {
        let field = req_attr(stats, "field")?.to_string();
        if let Some(array) = stats
            .children()
            .find(|c| c.is_element() && c.tag_name().name() == "Array")
        {
            counts.push((field, parse_array_f64(array)?));
        }
    }
    Ok(counts)
}

/// The diagonal of a cluster's `Covariances`; a full matrix is not supported.
fn parse_diagonal(node: roxmltree::Node<'_, '_>) -> Result<Vec<f64>, PmmlError> {
    let Some(matrix) = node
        .children()
        .find(|c| c.is_element() && c.tag_name().name() == "Matrix")
    else {
        return Ok(Vec::new());
    };
    if matrix.attribute("kind") != Some("diagonal") {
        return Err(PmmlError::Unsupported(
            "cluster Covariances other than a diagonal matrix".into(),
        ));
    }
    matrix
        .children()
        .find(|c| c.is_element() && c.tag_name().name() == "Array")
        .map_or(Ok(Vec::new()), parse_array_f64)
}

fn parse_general_regression_model(
    node: roxmltree::Node<'_, '_>,
) -> Result<GeneralRegressionModel, PmmlError> {
    let mut header = parse_model_header(node)?;
    if let Some(target_var) = node.attribute("targetVariableName")
        && !header
            .mining_schema
            .iter()
            .any(|mf| mf.usage == Usage::Target)
    {
        for mf in &mut header.mining_schema {
            if mf.name == target_var {
                mf.usage = Usage::Target;
            }
        }
    }

    let model_type = match req_attr(node, "modelType")? {
        "regression" => GeneralModelType::Regression,
        "generalLinear" => GeneralModelType::GeneralLinear,
        "multinomialLogistic" => GeneralModelType::MultinomialLogistic,
        "generalizedLinear" => GeneralModelType::GeneralizedLinear,
        "ordinalMultinomial" => GeneralModelType::OrdinalMultinomial,
        other => GeneralModelType::Other(other.to_string()),
    };
    let target_reference_category = node
        .attribute("targetReferenceCategory")
        .map(str::to_string);
    let link_function = node.attribute("linkFunction").map(str::to_string);
    let link_parameter = opt_f64_attr(node, "linkParameter")?;

    let mut parameters = Vec::new();
    let mut factors = Vec::new();
    let mut covariates = Vec::new();
    let mut pp_matrix = Vec::new();
    let mut param_matrix = Vec::new();
    let mut has_param_list = false;
    let mut has_param_matrix = false;

    for child in node.children().filter(roxmltree::Node::is_element) {
        let tag = child.tag_name().name();
        if is_skipped_element(tag)
            || matches!(
                tag,
                "MiningSchema"
                    | "Output"
                    | "LocalTransformations"
                    | "EventValues"
                    | "BaseCumHazardTables"
            )
        {
            continue;
        }
        match tag {
            "ParameterList" => {
                has_param_list = true;
                for p in child.children().filter(roxmltree::Node::is_element) {
                    if p.tag_name().name() == "Parameter" {
                        parameters.push(req_attr(p, "name")?.to_string());
                    }
                }
            }
            "FactorList" => {
                for p in child.children().filter(roxmltree::Node::is_element) {
                    if p.tag_name().name() == "Predictor" {
                        factors.push(req_attr(p, "name")?.to_string());
                    }
                }
            }
            "CovariateList" => {
                for p in child.children().filter(roxmltree::Node::is_element) {
                    if p.tag_name().name() == "Predictor" {
                        covariates.push(req_attr(p, "name")?.to_string());
                    }
                }
            }
            "PPMatrix" => {
                for cell in child.children().filter(roxmltree::Node::is_element) {
                    if cell.tag_name().name() == "PPCell" {
                        pp_matrix.push(PPCell {
                            value: req_attr(cell, "value")?.to_string(),
                            predictor: req_attr(cell, "predictorName")?.to_string(),
                            parameter: req_attr(cell, "parameterName")?.to_string(),
                        });
                    }
                }
            }
            "ParamMatrix" => {
                has_param_matrix = true;
                for cell in child.children().filter(roxmltree::Node::is_element) {
                    if cell.tag_name().name() == "PCell" {
                        param_matrix.push(PCell {
                            target_category: cell.attribute("targetCategory").map(str::to_string),
                            parameter: req_attr(cell, "parameterName")?.to_string(),
                            beta: req_f64_attr(cell, "beta")?,
                        });
                    }
                }
            }
            other => {
                return Err(PmmlError::Parse(format!(
                    "unexpected element {other:?} in GeneralRegressionModel"
                )));
            }
        }
    }

    if !has_param_list || !has_param_matrix {
        return Err(PmmlError::Parse(
            "GeneralRegressionModel requires ParameterList and ParamMatrix".into(),
        ));
    }

    Ok(GeneralRegressionModel {
        header,
        model_type,
        target_reference_category,
        link_function,
        link_parameter,
        parameters,
        factors,
        covariates,
        pp_matrix,
        param_matrix,
    })
}

fn parse_mining_model(
    node: roxmltree::Node<'_, '_>,
    dictionary: &[crate::DerivedField],
) -> Result<MiningModel, PmmlError> {
    let header = parse_model_header(node)?;
    let mut method = String::new();
    let mut segments = Vec::new();
    for child in node.children().filter(roxmltree::Node::is_element) {
        let tag = child.tag_name().name();
        if is_skipped_element(tag)
            || matches!(
                tag,
                "MiningSchema" | "Output" | "LocalTransformations" | "Targets"
            )
        {
            continue;
        }
        if tag != "Segmentation" {
            return Err(PmmlError::Parse(format!(
                "unexpected element {tag:?} in MiningModel"
            )));
        }
        method = child
            .attribute("multipleModelMethod")
            .unwrap_or("selectFirst")
            .to_string();
        for segment in child.children().filter(roxmltree::Node::is_element) {
            if is_skipped_element(segment.tag_name().name()) {
                continue;
            }
            if segment.tag_name().name() != "Segment" {
                return Err(PmmlError::Parse(format!(
                    "unexpected element {:?} in Segmentation",
                    segment.tag_name().name()
                )));
            }
            segments.push(parse_segment(segment, dictionary)?);
        }
    }
    if segments.is_empty() {
        return Err(PmmlError::Parse("MiningModel has no Segment".into()));
    }
    if method.is_empty() {
        method = "selectFirst".into();
    }
    Ok(MiningModel {
        header,
        method,
        segments,
    })
}

fn parse_segment(
    node: roxmltree::Node<'_, '_>,
    dictionary: &[crate::DerivedField],
) -> Result<MiningSegment, PmmlError> {
    let mut predicate = None;
    let mut model = None;
    for child in node.children().filter(roxmltree::Node::is_element) {
        let tag = child.tag_name().name();
        if is_skipped_element(tag) {
            continue;
        }
        match tag {
            "True" | "False" | "SimplePredicate" | "SimpleSetPredicate" | "CompoundPredicate" => {
                if predicate.is_some() {
                    return Err(PmmlError::Parse("Segment has two predicates".into()));
                }
                predicate = Some(parse_predicate(child, 0)?);
            }
            tag if MODEL_ELEMENTS.contains(&tag) => {
                if model.is_some() {
                    return Err(PmmlError::Parse("Segment has two models".into()));
                }
                model = Some(parse_model(child, dictionary)?);
            }
            other => {
                return Err(PmmlError::Parse(format!(
                    "unexpected element {other:?} in Segment"
                )));
            }
        }
    }
    Ok(MiningSegment {
        predicate: predicate.unwrap_or(Predicate::True),
        model: Box::new(model.ok_or_else(|| PmmlError::Parse("Segment has no model".into()))?),
    })
}

fn parse_regression_model(node: roxmltree::Node<'_, '_>) -> Result<RegressionModel, PmmlError> {
    let mut header = parse_model_header(node)?;
    if let Some(target_var) = node.attribute("targetFieldName")
        && !header
            .mining_schema
            .iter()
            .any(|mf| mf.usage == Usage::Target)
    {
        for mf in &mut header.mining_schema {
            if mf.name == target_var {
                mf.usage = Usage::Target;
            }
        }
    }

    let mut normalization = node
        .attribute("normalizationMethod")
        .unwrap_or("none")
        .to_string();
    let mut tables = Vec::new();

    for child in node.children().filter(roxmltree::Node::is_element) {
        let tag = child.tag_name().name();
        if is_skipped_element(tag)
            || matches!(tag, "MiningSchema" | "Output" | "LocalTransformations")
        {
            continue;
        }
        if tag != "RegressionTable" {
            return Err(PmmlError::Parse(format!(
                "unexpected element {tag:?} in RegressionModel"
            )));
        }
        if normalization == "none"
            && let Some(table_norm) = child.attribute("normalizationMethod")
            && table_norm != "none"
        {
            normalization = table_norm.to_string();
        }
        let intercept = req_f64_attr(child, "intercept")?;
        let target_category = child.attribute("targetCategory").map(str::to_string);
        let mut numeric = Vec::new();
        let mut categorical = Vec::new();

        for pred in child.children().filter(roxmltree::Node::is_element) {
            let ptag = pred.tag_name().name();
            if is_skipped_element(ptag) {
                continue;
            }
            match ptag {
                "NumericPredictor" => {
                    let field = req_attr(pred, "name")?.to_string();
                    let exponent = match pred.attribute("exponent") {
                        Some(s) => s.trim().parse::<i32>().map_err(|_| {
                            PmmlError::Parse(format!("invalid NumericPredictor exponent {s:?}"))
                        })?,
                        None => 1,
                    };
                    let coefficient = req_f64_attr(pred, "coefficient")?;
                    numeric.push((field, exponent, coefficient));
                }
                "CategoricalPredictor" => {
                    let field = req_attr(pred, "name")?.to_string();
                    let value = req_attr(pred, "value")?.to_string();
                    let coefficient = req_f64_attr(pred, "coefficient")?;
                    categorical.push((field, value, coefficient));
                }
                other => {
                    return Err(PmmlError::Parse(format!(
                        "unexpected predictor {other:?} in RegressionTable"
                    )));
                }
            }
        }
        tables.push(RegressionTable {
            intercept,
            target_category,
            numeric,
            categorical,
        });
    }

    if tables.is_empty() {
        return Err(PmmlError::Parse(
            "RegressionModel requires at least one RegressionTable".into(),
        ));
    }

    Ok(RegressionModel {
        header,
        normalization,
        tables,
    })
}
