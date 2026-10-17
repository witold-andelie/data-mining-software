//! Binding input schemas (`Pmml::bind`), applying `MiningSchema` treatments, computing
//! derived fields, and dispatching to model scorers (`Scorer::score`).

use crate::expr::{self, value_matches_str};
use crate::{
    DataField, DataType, DefineFunction, DerivedField, Expr, InvalidTreatment, MiningField,
    MiningModel, Model, ModelHeader, Node, OpType, Outliers, Pmml, PmmlError, Predicate,
    Prediction, Usage, Value,
};
use std::collections::{HashMap, HashSet};

/// A bound model ready to score input records whose values follow the bound `inputs` order.
#[derive(Debug, Clone)]
pub struct Scorer<'a> {
    pmml: &'a Pmml,
    /// `(field_name, input_slot, data_field, mining_field)` for each needed input field.
    bound_inputs: Vec<BoundInput<'a>>,
    /// Derived fields to compute in topological / document order.
    derived_fields: Vec<&'a DerivedField>,
}

#[derive(Debug, Clone)]
struct BoundInput<'a> {
    name: &'a str,
    slot: usize,
    data_field: Option<&'a DataField>,
    mining_field: Option<&'a MiningField>,
}

/// Field environment during the scoring of a single record.
#[derive(Debug, Default, Clone)]
pub(crate) struct Env<'a> {
    values: HashMap<&'a str, Value>,
}

impl<'a> Env<'a> {
    pub(crate) fn insert(&mut self, name: &'a str, value: Value) {
        self.values.insert(name, value);
    }

    pub(crate) fn get(&self, name: &str) -> Option<&Value> {
        self.values.get(name)
    }
}

pub(crate) fn bind<'a>(pmml: &'a Pmml, inputs: &[&str]) -> Result<Scorer<'a>, PmmlError> {
    let header = model_header(&pmml.model);
    let mut all_derived: Vec<&'a DerivedField> = Vec::new();
    for df in &pmml.transformations {
        all_derived.push(df);
    }
    if let Some(h) = header {
        for df in &h.local_transformations {
            all_derived.push(df);
        }
    }

    let mut needed: HashSet<&'a str> = HashSet::new();
    if let Some(h) = header {
        for mf in &h.mining_schema {
            if mf.usage == Usage::Active {
                needed.insert(mf.name.as_str());
            }
        }
    }
    collect_model_fields(&pmml.model, &mut needed);

    // Propagate dependencies backward through derived fields so chains `d2 -> d1 -> input` resolve.
    for df in all_derived.iter().rev() {
        if needed.contains(df.name.as_str()) {
            collect_expr_fields(&df.expr, &pmml.functions, &[], &mut needed, 0)?;
        }
    }

    // Derived fields defined up to the point of use.
    let mut defined_derived: HashSet<&'a str> = HashSet::new();
    let mut derived_fields: Vec<&'a DerivedField> = Vec::new();
    for df in &all_derived {
        if needed.contains(df.name.as_str()) {
            let mut df_deps = HashSet::new();
            collect_expr_fields(&df.expr, &pmml.functions, &[], &mut df_deps, 0)?;
            for dep in df_deps {
                if !defined_derived.contains(dep) && !inputs.contains(&dep) {
                    return Err(PmmlError::UnknownField(dep.to_string()));
                }
            }
            defined_derived.insert(df.name.as_str());
            derived_fields.push(df);
        }
    }

    let mut bound_inputs = Vec::new();
    let mut seen_inputs: HashSet<&'a str> = HashSet::new();
    for &field_name in &needed {
        if defined_derived.contains(field_name) {
            continue;
        }
        let Some(slot) = inputs.iter().position(|&inp| inp == field_name) else {
            return Err(PmmlError::UnknownField(field_name.to_string()));
        };
        if seen_inputs.insert(field_name) {
            let data_field = pmml.data_dictionary.iter().find(|df| df.name == field_name);
            let mining_field =
                header.and_then(|h| h.mining_schema.iter().find(|mf| mf.name == field_name));
            bound_inputs.push(BoundInput {
                name: field_name,
                slot,
                data_field,
                mining_field,
            });
        }
    }

    Ok(Scorer {
        pmml,
        bound_inputs,
        derived_fields,
    })
}

impl Scorer<'_> {
    /// Scores one record whose values are ordered according to the `inputs` passed to [`Pmml::bind`].
    pub fn score(&self, record: &[Value]) -> Result<Prediction, PmmlError> {
        let mut env = Env::default();

        for bi in &self.bound_inputs {
            let raw = record.get(bi.slot).cloned().unwrap_or(Value::Missing);
            let treated = apply_treatments(raw, bi.name, bi.data_field, bi.mining_field)?;
            env.insert(bi.name, treated);
        }

        let header = model_header(&self.pmml.model);
        for df in &self.derived_fields {
            let val = expr::eval(&df.expr, &env, &self.pmml.functions, &[], 0)?;
            let coerced = coerce_derived_value(val, df);
            let mf = header.and_then(|h| h.mining_schema.iter().find(|m| m.name == df.name));
            let treated = apply_treatments(coerced, &df.name, None, mf)?;
            env.insert(df.name.as_str(), treated);
        }

        match &self.pmml.model {
            Model::Tree(tree) => crate::tree::score(self.pmml, tree, &env),
            Model::Clustering(clustering) => crate::clustering::score(self.pmml, clustering, &env),
            Model::GeneralRegression(reg) => crate::regression::score_general(self.pmml, reg, &env),
            Model::Regression(reg) => crate::regression::score_regression(self.pmml, reg, &env),
            Model::Mining(mining) => score_mining(self.pmml, mining, &env),
            Model::Unsupported(name) => Err(PmmlError::Unsupported(name.clone())),
        }
    }
}

fn coerce_derived_value(val: Value, df: &DerivedField) -> Value {
    match (&val, &df.data_type) {
        (Value::Text(s), DataType::Integer | DataType::Float | DataType::Double) => {
            match s.as_str() {
                "true" => Value::Number(1.0),
                "false" => Value::Number(0.0),
                other => other
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|n| n.is_finite())
                    .map_or(val, Value::Number),
            }
        }
        _ => val,
    }
}

fn apply_treatments(
    mut val: Value,
    field_name: &str,
    data_field: Option<&DataField>,
    mining_field: Option<&MiningField>,
) -> Result<Value, PmmlError> {
    let invalid_treatment =
        mining_field.map_or(InvalidTreatment::ReturnInvalid, |mf| mf.invalid_treatment);

    // Check validity for non-missing values.
    if val != Value::Missing {
        let mut is_invalid = false;
        if let Value::Number(n) = val
            && !n.is_finite()
        {
            is_invalid = true;
        }
        if let Some(df) = data_field {
            if df.optype == OpType::Continuous
                || matches!(
                    df.data_type,
                    DataType::Integer | DataType::Float | DataType::Double
                )
            {
                if let Value::Text(s) = &val {
                    match s.trim().parse::<f64>() {
                        Ok(n) if n.is_finite() => val = Value::Number(n),
                        _ => is_invalid = true,
                    }
                }
            } else if !df.values.is_empty()
                && !df
                    .values
                    .iter()
                    .any(|allowed| value_matches_str(&val, allowed))
            {
                is_invalid = true;
            }
        }

        if is_invalid {
            match invalid_treatment {
                InvalidTreatment::ReturnInvalid => {
                    return Err(PmmlError::Score(format!(
                        "invalid value for field {field_name:?}"
                    )));
                }
                InvalidTreatment::AsMissing => {
                    val = Value::Missing;
                }
                InvalidTreatment::AsIs => {
                    if let Value::Number(n) = val
                        && !n.is_finite()
                    {
                        return Err(PmmlError::Score(format!(
                            "non-finite number for field {field_name:?}"
                        )));
                    }
                }
            }
        }
    }

    // Outlier treatment on non-missing numeric values.
    if let Some(mf) = mining_field
        && val != Value::Missing
        && mf.outliers != Outliers::AsIs
    {
        let num = match &val {
            Value::Number(n) => Some(*n),
            Value::Text(s) => s.trim().parse::<f64>().ok().filter(|n| n.is_finite()),
            Value::Missing => None,
        };
        if let Some(x) = num {
            match mf.outliers {
                Outliers::AsIs => {}
                Outliers::AsExtremeValues => {
                    let mut clamped = x;
                    if let Some(low) = mf.low_value
                        && clamped < low
                    {
                        clamped = low;
                    }
                    if let Some(high) = mf.high_value
                        && clamped > high
                    {
                        clamped = high;
                    }
                    val = Value::Number(clamped);
                }
                Outliers::AsMissingValues => {
                    let below = mf.low_value.is_some_and(|low| x < low);
                    let above = mf.high_value.is_some_and(|high| x > high);
                    if below || above {
                        val = Value::Missing;
                    }
                }
            }
        }
    }

    // Missing value replacement.
    if val == Value::Missing
        && let Some(mf) = mining_field
        && let Some(rep) = &mf.missing_replacement
    {
        val = parse_replacement(rep, data_field);
    }

    Ok(val)
}

fn parse_replacement(rep: &str, data_field: Option<&DataField>) -> Value {
    let want_number = data_field.is_none_or(|df| {
        df.optype == OpType::Continuous
            || matches!(
                df.data_type,
                DataType::Integer | DataType::Float | DataType::Double
            )
    });
    if want_number
        && let Ok(n) = rep.trim().parse::<f64>()
        && n.is_finite()
    {
        Value::Number(n)
    } else {
        Value::Text(rep.to_string())
    }
}

fn score_mining(pmml: &Pmml, mining: &MiningModel, env: &Env<'_>) -> Result<Prediction, PmmlError> {
    match mining.method.as_str() {
        "" | "selectFirst" => {}
        other => {
            return Err(PmmlError::Unsupported(format!(
                "MiningModel multipleModelMethod {other:?}"
            )));
        }
    }
    for segment in &mining.segments {
        if !segment_matches(&segment.predicate, env)? {
            continue;
        }
        let mut inner_env = env.clone();
        if let Some(header) = model_header(&segment.model) {
            for mf in &header.mining_schema {
                if mf.usage != Usage::Active {
                    continue;
                }
                let Some(current) = inner_env.get(mf.name.as_str()).cloned() else {
                    continue;
                };
                let df = pmml.data_dictionary.iter().find(|d| d.name == mf.name);
                let treated = apply_treatments(current, &mf.name, df, Some(mf))?;
                inner_env.insert(mf.name.as_str(), treated);
            }
        }
        return match segment.model.as_ref() {
            Model::GeneralRegression(reg) => {
                crate::regression::score_general(pmml, reg, &inner_env)
            }
            Model::Regression(reg) => crate::regression::score_regression(pmml, reg, &inner_env),
            Model::Unsupported(name) => Err(PmmlError::Unsupported(name.clone())),
            Model::Tree(_) | Model::Clustering(_) | Model::Mining(_) => Err(
                PmmlError::Unsupported("nested MiningModel segment type".into()),
            ),
        };
    }
    Ok(empty_prediction())
}

fn segment_matches(pred: &Predicate, env: &Env<'_>) -> Result<bool, PmmlError> {
    match pred {
        Predicate::True => Ok(true),
        Predicate::False => Ok(false),
        Predicate::Simple {
            field,
            operator,
            value,
        } => {
            let Some(val) = env.get(field) else {
                return Err(PmmlError::UnknownField(field.clone()));
            };
            if *val == Value::Missing {
                return Ok(false);
            }
            match operator {
                crate::Operator::Equal => Ok(value
                    .as_deref()
                    .is_some_and(|rhs| value_matches_str(val, rhs))),
                crate::Operator::NotEqual => Ok(!value
                    .as_deref()
                    .is_some_and(|rhs| value_matches_str(val, rhs))),
                _ => Err(PmmlError::Unsupported(
                    "MiningModel segment predicate".into(),
                )),
            }
        }
        _ => Err(PmmlError::Unsupported(
            "MiningModel segment predicate".into(),
        )),
    }
}

pub(crate) fn model_header(model: &Model) -> Option<&ModelHeader> {
    match model {
        Model::Tree(m) => Some(&m.header),
        Model::Clustering(m) => Some(&m.header),
        Model::GeneralRegression(m) => Some(&m.header),
        Model::Regression(m) => Some(&m.header),
        Model::Mining(m) => Some(&m.header),
        Model::Unsupported(_) => None,
    }
}

pub(crate) fn target_categories<'a>(pmml: &'a Pmml, header: &'a ModelHeader) -> Vec<&'a str> {
    if let Some(target_mf) = header
        .mining_schema
        .iter()
        .find(|mf| mf.usage == Usage::Target)
        && let Some(df) = pmml
            .data_dictionary
            .iter()
            .find(|df| df.name == target_mf.name)
        && !df.values.is_empty()
    {
        return df.values.iter().map(String::as_str).collect();
    }
    Vec::new()
}

pub(crate) fn empty_prediction() -> Prediction {
    Prediction {
        value: Value::Missing,
        confidence: None,
        probabilities: Vec::new(),
        entity_id: None,
        affinity: None,
    }
}

fn collect_model_fields<'a>(model: &'a Model, out: &mut HashSet<&'a str>) {
    match model {
        Model::Tree(tree) => collect_node_fields(&tree.root, out),
        Model::Clustering(clustering) => {
            for f in &clustering.fields {
                out.insert(f.field.as_str());
            }
        }
        Model::GeneralRegression(reg) => {
            for f in &reg.factors {
                out.insert(f.as_str());
            }
            for c in &reg.covariates {
                out.insert(c.as_str());
            }
            for cell in &reg.pp_matrix {
                out.insert(cell.predictor.as_str());
            }
        }
        Model::Regression(reg) => {
            for table in &reg.tables {
                for (field, _, _) in &table.numeric {
                    out.insert(field.as_str());
                }
                for (field, _, _) in &table.categorical {
                    out.insert(field.as_str());
                }
            }
        }
        Model::Mining(mining) => {
            for segment in &mining.segments {
                collect_predicate_fields(&segment.predicate, out);
                collect_model_fields(&segment.model, out);
            }
        }
        Model::Unsupported(_) => {}
    }
}

fn collect_node_fields<'a>(node: &'a Node, out: &mut HashSet<&'a str>) {
    collect_predicate_fields(&node.predicate, out);
    for child in &node.children {
        collect_node_fields(child, out);
    }
}

fn collect_predicate_fields<'a>(pred: &'a Predicate, out: &mut HashSet<&'a str>) {
    match pred {
        Predicate::True | Predicate::False => {}
        Predicate::Simple { field, .. } | Predicate::SimpleSet { field, .. } => {
            out.insert(field.as_str());
        }
        Predicate::Compound { predicates, .. } => {
            for p in predicates {
                collect_predicate_fields(p, out);
            }
        }
    }
}

fn collect_expr_fields<'a>(
    expr: &'a Expr,
    functions: &'a [DefineFunction],
    params: &[&str],
    out: &mut HashSet<&'a str>,
    depth: usize,
) -> Result<(), PmmlError> {
    if depth > 64 {
        return Err(PmmlError::Parse(
            "expression nesting depth limit exceeded".into(),
        ));
    }
    match expr {
        Expr::Constant(_) => {}
        Expr::FieldRef { field, .. }
        | Expr::NormContinuous { field, .. }
        | Expr::NormDiscrete { field, .. } => {
            if !params.contains(&field.as_str()) {
                out.insert(field.as_str());
            }
        }
        Expr::MapValues { pairs, .. } => {
            for (field, _) in pairs {
                if !params.contains(&field.as_str()) {
                    out.insert(field.as_str());
                }
            }
        }
        Expr::Apply { function, args, .. } => {
            if let Some(def) = functions.iter().find(|f| f.name == *function) {
                let def_params: Vec<&str> = def.parameters.iter().map(String::as_str).collect();
                collect_expr_fields(&def.body, functions, &def_params, out, depth + 1)?;
            }
            for arg in args {
                collect_expr_fields(arg, functions, params, out, depth + 1)?;
            }
        }
    }
    Ok(())
}
