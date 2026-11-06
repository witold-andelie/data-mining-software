//! Clementine 11 inline `RuleTreeModel` (no PMML container).
//!
//! Observed on `ibm-demos/telco_dataaudit.str`: the supernode's C&R Tree nuggets store the tree
//! as `properties/model@class=...RuleTreeModel`. Mapped onto [`TreeModel`] so scoring is the
//! PMML path. CART PMML writes each child predicate as a `surrogate` compound ending in `True`
//! (default / larger child) or `False` (the other); `biggestChild` is that 0-based index.

use crate::{
    BooleanOperator, DataField, DataType, InvalidTreatment, MiningField, MiningFunction,
    MissingValueStrategy, Model, ModelHeader, NoTrueChildStrategy, Node, OpType, Operator,
    Outliers, Pmml, PmmlError, Predicate, TreeModel, Usage,
};
use roxmltree::Document;

const MAX_DEPTH: usize = 64;
const MAX_NODES: usize = 100_000;
const MAX_SET_VALUES: usize = 10_000;
const MAX_XML: usize = 64 * 1024 * 1024;

pub(crate) fn parse(xml: &str) -> Result<Pmml, PmmlError> {
    if xml.len() > MAX_XML {
        return Err(PmmlError::Parse(
            "RuleTreeModel document size limit exceeded".into(),
        ));
    }
    let doc = Document::parse(xml).map_err(|e| PmmlError::Parse(e.to_string()))?;
    let root = doc.root_element();
    if class_suffix(root) != "RuleTreeModel" {
        return Err(PmmlError::Parse(format!(
            "RuleTreeModel root must be model@class=RuleTreeModel, got {:?}",
            class_suffix(root)
        )));
    }
    parse_model(root)
}

fn parse_model(root: roxmltree::Node<'_, '_>) -> Result<Pmml, PmmlError> {
    let name = child_text(root, "name").filter(|n| !n.is_empty());
    let algorithm = child_text(root, "algorithmName").filter(|n| !n.is_empty());
    let mut data_dictionary = Vec::new();
    let mut mining_schema = Vec::new();
    let mut target = None;
    let mut target_storage = DataType::String;
    let mut target_optype = OpType::Categorical;
    let mut tree_root = None;

    for child in root.children().filter(roxmltree::Node::is_element) {
        match child.tag_name().name() {
            "inputs" => {
                for input in child.children().filter(roxmltree::Node::is_element) {
                    if input.tag_name().name() != "input" {
                        continue;
                    }
                    let field = parse_typed_field(input)?;
                    mining_schema.push(mining_field(field.name.clone(), Usage::Active));
                    data_dictionary.push(field);
                }
            }
            "outputs" => {
                for output in child.children().filter(roxmltree::Node::is_element) {
                    if output.tag_name().name() != "output" {
                        continue;
                    }
                    let info = named_child(output, "modelFieldInfo");
                    let role = info.and_then(|n| n.attribute("role")).unwrap_or("");
                    if role != "predictedValue" {
                        continue;
                    }
                    let field = parse_typed_field(output)?;
                    target_storage = field.data_type.clone();
                    target_optype = field.optype;
                    target = info
                        .and_then(|n| n.attribute("targetField"))
                        .map(str::to_string)
                        .filter(|n| !n.is_empty())
                        .or(Some(field.name));
                }
            }
            "root" => {
                if tree_root.is_some() {
                    return Err(PmmlError::Parse("RuleTreeModel has two root nodes".into()));
                }
                let mut parser = Parser { nodes: 0 };
                tree_root = Some(parser.parse_node(child, 0)?);
            }
            _ => {}
        }
    }

    let target = target
        .ok_or_else(|| PmmlError::Parse("RuleTreeModel has no predictedValue output".into()))?;
    if !data_dictionary.iter().any(|f| f.name == target) {
        data_dictionary.push(DataField {
            name: target.clone(),
            optype: target_optype,
            data_type: target_storage.clone(),
            values: Vec::new(),
            missing_values: Vec::new(),
            flag_true_value: None,
        });
    }
    mining_schema.push(mining_field(target, Usage::Target));

    let function = if target_optype == OpType::Continuous {
        MiningFunction::Regression
    } else {
        MiningFunction::Classification
    };

    let tree_root =
        tree_root.ok_or_else(|| PmmlError::Parse("RuleTreeModel is missing root".into()))?;

    Ok(Pmml {
        version: "clementine-11".into(),
        data_dictionary,
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::Tree(TreeModel {
            header: ModelHeader {
                name,
                function,
                algorithm,
                mining_schema,
                output: Vec::new(),
                local_transformations: Vec::new(),
            },
            missing_value_strategy: MissingValueStrategy::None,
            no_true_child_strategy: NoTrueChildStrategy::ReturnNullPrediction,
            root: tree_root,
        }),
    })
}

struct Parser {
    nodes: usize,
}

impl Parser {
    fn bump(&mut self) -> Result<(), PmmlError> {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > MAX_NODES {
            return Err(PmmlError::Parse(
                "RuleTreeModel node count limit exceeded".into(),
            ));
        }
        Ok(())
    }

    fn parse_node(
        &mut self,
        node: roxmltree::Node<'_, '_>,
        depth: usize,
    ) -> Result<Node, PmmlError> {
        if depth > MAX_DEPTH {
            return Err(PmmlError::Parse(
                "RuleTreeModel node depth limit exceeded".into(),
            ));
        }
        self.bump()?;
        let mut children = Vec::new();
        if let Some(container) = node.children().find(|c| c.has_tag_name("children")) {
            for child in container.children().filter(roxmltree::Node::is_element) {
                if child.tag_name().name() != "child" {
                    continue;
                }
                match class_suffix(child) {
                    "DecisionRuleNode" | "Rule" => {
                        children.push(self.parse_node(child, depth.saturating_add(1))?);
                    }
                    other => {
                        return Err(PmmlError::Parse(format!(
                            "unexpected RuleTreeModel child class {other:?}"
                        )));
                    }
                }
            }
        }

        let biggest = child_text(node, "biggestChild")
            .and_then(|text| text.trim().parse::<i64>().ok())
            .unwrap_or(-1);
        if biggest >= 0 && children.len() >= 2 {
            let default = usize::try_from(biggest).unwrap_or(usize::MAX);
            if default < children.len() {
                for (index, child) in children.iter_mut().enumerate() {
                    child.predicate = with_default_surrogate(
                        std::mem::replace(&mut child.predicate, Predicate::True),
                        index == default,
                    );
                }
            }
        }

        let predicate = node
            .children()
            .find(|c| c.has_tag_name("condition"))
            .map(|c| parse_condition(c, 0))
            .transpose()?
            .unwrap_or(Predicate::True);

        let score = child_text(node, "result")
            .filter(|s| !s.is_empty())
            .or_else(|| child_text(node, "mean").filter(|s| !s.is_empty()));
        let record_count =
            child_text(node, "recordCount").and_then(|text| parse_finite(text.trim()));
        let id = child_text(node, "id").filter(|s| !s.is_empty());
        let default_child = if biggest >= 0 {
            usize::try_from(biggest)
                .ok()
                .and_then(|index| children.get(index))
                .and_then(|child| child.id.clone())
        } else {
            None
        };

        Ok(Node {
            id,
            score,
            record_count,
            default_child,
            predicate,
            distribution: Vec::new(),
            children,
        })
    }
}

fn with_default_surrogate(predicate: Predicate, is_default: bool) -> Predicate {
    let tail = if is_default {
        Predicate::True
    } else {
        Predicate::False
    };
    match predicate {
        Predicate::Compound {
            operator: BooleanOperator::Surrogate,
            mut predicates,
        } => {
            predicates.push(tail);
            Predicate::Compound {
                operator: BooleanOperator::Surrogate,
                predicates,
            }
        }
        other => Predicate::Compound {
            operator: BooleanOperator::Surrogate,
            predicates: vec![other, tail],
        },
    }
}

fn parse_condition(node: roxmltree::Node<'_, '_>, depth: usize) -> Result<Predicate, PmmlError> {
    if depth > MAX_DEPTH {
        return Err(PmmlError::Parse(
            "RuleTreeModel condition depth limit exceeded".into(),
        ));
    }
    match class_suffix(node) {
        "TrueCondition" => Ok(Predicate::True),
        "FalseCondition" => Ok(Predicate::False),
        "SimpleCondition" => parse_simple_condition(node),
        "SimpleSetCondition" => parse_set_condition(node),
        "LogicalCondition" => parse_logical_condition(node, depth),
        other => Err(PmmlError::Parse(format!(
            "unsupported RuleTreeModel condition {other:?}"
        ))),
    }
}

fn parse_simple_condition(node: roxmltree::Node<'_, '_>) -> Result<Predicate, PmmlError> {
    let field = child_text(node, "field")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| PmmlError::Parse("SimpleCondition is missing a field".into()))?;
    let op = child_text(node, "op").unwrap_or_default();
    let operator = match op.trim() {
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
                "SimpleCondition operator {other:?} is not supported"
            )));
        }
    };
    let value = child_text(node, "value");
    if !matches!(operator, Operator::IsMissing | Operator::IsNotMissing) && value.is_none() {
        return Err(PmmlError::Parse(
            "SimpleCondition is missing a value".into(),
        ));
    }
    Ok(Predicate::Simple {
        field,
        operator,
        value,
    })
}

fn parse_set_condition(node: roxmltree::Node<'_, '_>) -> Result<Predicate, PmmlError> {
    let field = child_text(node, "field")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| PmmlError::Parse("SimpleSetCondition is missing a field".into()))?;
    let is_in = match child_text(node, "op").as_deref().map(str::trim) {
        None | Some("") | Some("in") => true,
        Some("notIn") => false,
        Some(other) => {
            return Err(PmmlError::Parse(format!(
                "SimpleSetCondition operator {other:?} is not supported"
            )));
        }
    };
    let mut values = Vec::new();
    if let Some(container) = node.children().find(|c| c.has_tag_name("values")) {
        for value in container.children().filter(|c| c.has_tag_name("value")) {
            if values.len() >= MAX_SET_VALUES {
                return Err(PmmlError::Parse(
                    "SimpleSetCondition value count limit exceeded".into(),
                ));
            }
            values.push(text_of(value));
        }
    }
    Ok(Predicate::SimpleSet {
        field,
        is_in,
        values,
    })
}

fn parse_logical_condition(
    node: roxmltree::Node<'_, '_>,
    depth: usize,
) -> Result<Predicate, PmmlError> {
    let operator = match child_text(node, "op").as_deref().map(str::trim) {
        None | Some("") | Some("surrogate") => BooleanOperator::Surrogate,
        Some("and") => BooleanOperator::And,
        Some("or") => BooleanOperator::Or,
        Some("xor") => BooleanOperator::Xor,
        Some(other) => {
            return Err(PmmlError::Parse(format!(
                "LogicalCondition operator {other:?} is not supported"
            )));
        }
    };
    let mut predicates = Vec::new();
    if let Some(container) = node.children().find(|c| c.has_tag_name("children")) {
        for child in container.children().filter(roxmltree::Node::is_element) {
            predicates.push(parse_condition(child, depth.saturating_add(1))?);
        }
    }
    match predicates.len() {
        0 => Ok(Predicate::True),
        1 => predicates
            .pop()
            .ok_or_else(|| PmmlError::Parse("LogicalCondition lost its predicate".into())),
        _ => Ok(Predicate::Compound {
            operator,
            predicates,
        }),
    }
}

fn parse_typed_field(node: roxmltree::Node<'_, '_>) -> Result<DataField, PmmlError> {
    let name = child_text(node, "name")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| PmmlError::Parse("RuleTreeModel field is missing a name".into()))?;
    let type_el = named_child(node, "type");
    let storage = type_el
        .and_then(|t| child_text(t, "storage"))
        .unwrap_or_default();
    let measure = type_el
        .and_then(|t| child_text(t, "measure"))
        .unwrap_or_default();
    let data_type = match storage.trim() {
        "integer" => DataType::Integer,
        "real" => DataType::Double,
        "date" => DataType::Date,
        "time" => DataType::Time,
        "timestamp" => DataType::DateTime,
        _ => DataType::String,
    };
    let optype = match measure.trim() {
        "range" => OpType::Continuous,
        "orderedSet" => OpType::Ordinal,
        _ => OpType::Categorical,
    };
    let mut values = Vec::new();
    if !matches!(optype, OpType::Continuous)
        && let Some(type_el) = type_el
        && let Some(container) = type_el.children().find(|c| c.has_tag_name("values"))
    {
        for value in container.children().filter(|c| c.has_tag_name("value")) {
            if values.len() >= MAX_SET_VALUES {
                return Err(PmmlError::Parse(
                    "RuleTreeModel field value count limit exceeded".into(),
                ));
            }
            values.push(text_of(value));
        }
    }
    Ok(DataField {
        name,
        optype,
        data_type,
        values,
        missing_values: Vec::new(),
        flag_true_value: None,
    })
}

fn mining_field(name: String, usage: Usage) -> MiningField {
    MiningField {
        name,
        usage,
        missing_replacement: None,
        outliers: Outliers::AsIs,
        low_value: None,
        high_value: None,
        invalid_treatment: InvalidTreatment::AsIs,
        direction: None,
    }
}

fn class_suffix<'a>(node: roxmltree::Node<'a, '_>) -> &'a str {
    node.attribute("class")
        .and_then(|class| class.rsplit('.').next())
        .unwrap_or_else(|| node.tag_name().name())
}

fn named_child<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children().find(|c| c.has_tag_name(name))
}

fn child_text(node: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    named_child(node, name).map(text_of)
}

fn text_of(node: roxmltree::Node<'_, '_>) -> String {
    node.text().unwrap_or("").to_string()
}

fn parse_finite(text: &str) -> Option<f64> {
    text.parse::<f64>().ok().filter(|n| n.is_finite())
}
