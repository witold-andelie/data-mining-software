//! Linear regression for the Linear node (`LinearRegression!spss.alm-extension`; Algorithms
//! Guide: "Linear modeling algorithms"): ordinary least squares with every input entered
//! (`linreg.VarSelectionMethod` `none`) and no automatic data preparation.
//!
//! The model is a PMML `GeneralRegressionModel` `generalLinear` shaped like Modeler's
//! (`oracle/models/linear_enter`, `oracle/team/linear_regression` `02_increase.xml`):
//! - Data dictionary: the target, then the inputs; mining schema: the inputs, then the target as
//!   predicted.
//! - Parameters `P0000001`, `P0000002`, ...: `Intercept` (with `linreg.Intercept`), then each
//!   input in input order: a covariate as one parameter labelled with its name, a categorical
//!   input as one indicator `[<field>=<category>]` per category in the Type's order, the last one
//!   the reference with beta 0 (`team/linear_regression` `01_increase.xml`: `[Class=Meat]`
//!   beta 0, df 0).
//! - Coefficients minimise the residual sum of squares, solved by Householder QR of the design
//!   matrix (records with a missing target or input are left out, `linreg.ListwiseDeletion`);
//!   an aliased column gets beta 0.
//!
//! The classic Regression node (`BuildRegressionNode`, method `enter`) fits the same least
//! squares and writes `modelType` `regression` (`oracle/models/regression_enter`): parameters
//! named `(Constant)` and the covariate names, PMML 4.2. It takes continuous inputs only.
//!
//! Not implemented (`AlgoError::Unsupported` in the build node): automatic data preparation,
//! forward stepwise and best-subsets selection (the defaults of a new Linear node, which
//! `team/linear_regression` uses), weights.

use crate::common::{data_field, field_categories, mining_field};
use crate::{AlgoError, Dataset};
use dms_pmml::{
    GeneralModelType, GeneralRegressionModel, MiningFunction, Model, ModelHeader, PCell, PPCell,
    Pmml, Usage,
};
use dms_types::{Measurement, Value};

/// Which node's model to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// The Linear node: `generalLinear`, parameters `P0000001`, ...
    Linear,
    /// The classic Regression node: `regression`, parameters named after their fields.
    Regression,
}

/// Settings of a Linear or Regression build node.
#[derive(Debug, Clone, PartialEq)]
pub struct LinearSettings {
    pub style: Style,
    /// `linreg.Intercept` / `includeConstant` (default true).
    pub intercept: bool,
    /// The model name.
    pub name: Option<String>,
}

impl Default for LinearSettings {
    fn default() -> Self {
        LinearSettings {
            style: Style::Linear,
            intercept: true,
            name: None,
        }
    }
}

/// One PMML parameter and, unless it is a reference category, its design-matrix column.
enum Parameter {
    Intercept,
    Covariate {
        position: usize,
    },
    Indicator {
        position: usize,
        category: String,
        reference: bool,
    },
}

impl Parameter {
    fn estimated(&self) -> bool {
        !matches!(
            self,
            Parameter::Indicator {
                reference: true,
                ..
            }
        )
    }

    fn value(&self, row: &[Value]) -> f64 {
        match self {
            Parameter::Intercept => 1.0,
            Parameter::Covariate { position } => {
                row.get(*position).and_then(Value::as_f64).unwrap_or(0.0)
            }
            Parameter::Indicator {
                position, category, ..
            } => {
                if row
                    .get(*position)
                    .is_some_and(|v| v.to_string() == *category)
                {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
}

/// Trains a Linear model: least squares of the target on all inputs.
pub fn train(dataset: &Dataset<'_>, settings: &LinearSettings) -> Result<Pmml, AlgoError> {
    let table = dataset.table;
    let fields = table.model().fields();
    let field = |position: usize| {
        fields
            .get(position)
            .ok_or_else(|| AlgoError::Data(format!("no field at position {position}")))
    };
    let target = field(dataset.target)?;
    if target.measurement != Measurement::Continuous {
        return Err(AlgoError::Settings(format!(
            "the Linear target {} is not continuous",
            target.name
        )));
    }
    let present = |position: usize, value: Option<&Value>| match value {
        None | Some(Value::Null) => false,
        Some(v) => fields.get(position).is_some_and(|f| !f.is_missing(v)),
    };
    let rows: Vec<&Vec<Value>> = table
        .rows()
        .iter()
        .filter(|row| {
            std::iter::once(dataset.target)
                .chain(dataset.inputs.iter().copied())
                .all(|p| present(p, row.get(p)))
        })
        .collect();

    let mut parameters = Vec::new();
    if settings.intercept {
        parameters.push(Parameter::Intercept);
    }
    for &position in dataset.inputs {
        if field(position)?.measurement == Measurement::Continuous {
            parameters.push(Parameter::Covariate { position });
            continue;
        }
        if settings.style == Style::Regression {
            return Err(AlgoError::Unsupported(format!(
                "Regression input {} that is not continuous",
                fields[position].name
            )));
        }
        let categories = field_categories(table, position);
        let last = categories.len().saturating_sub(1);
        for (k, category) in categories.into_iter().enumerate() {
            parameters.push(Parameter::Indicator {
                position,
                category,
                reference: k == last,
            });
        }
    }
    let estimated: Vec<&Parameter> = parameters.iter().filter(|p| p.estimated()).collect();
    let design: Vec<Vec<f64>> = rows
        .iter()
        .map(|row| estimated.iter().map(|p| p.value(row)).collect())
        .collect();
    let y: Vec<f64> = rows
        .iter()
        .map(|row| {
            row.get(dataset.target)
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
        })
        .collect();
    let mut beta = least_squares(design, y, estimated.len())?.into_iter();

    let mut names = Vec::new();
    let mut pp_matrix = Vec::new();
    let mut param_matrix = Vec::new();
    for (index, parameter) in parameters.iter().enumerate() {
        let name = match (settings.style, parameter) {
            (Style::Linear, _) => format!("P{:07}", index + 1),
            (Style::Regression, Parameter::Intercept) => "(Constant)".to_string(),
            (Style::Regression, Parameter::Covariate { position }) => {
                fields[*position].name.clone()
            }
            (
                Style::Regression,
                Parameter::Indicator {
                    position, category, ..
                },
            ) => format!("[{}={category}]", fields[*position].name),
        };
        match parameter {
            Parameter::Intercept => {}
            Parameter::Covariate { position } => pp_matrix.push(PPCell {
                value: "1".into(),
                predictor: fields[*position].name.clone(),
                parameter: name.clone(),
            }),
            Parameter::Indicator {
                position, category, ..
            } => pp_matrix.push(PPCell {
                value: category.clone(),
                predictor: fields[*position].name.clone(),
                parameter: name.clone(),
            }),
        }
        let value = if parameter.estimated() {
            beta.next().unwrap_or(0.0)
        } else {
            0.0
        };
        param_matrix.push(PCell {
            target_category: None,
            parameter: name.clone(),
            beta: value,
        });
        names.push(name);
    }
    let (covariates, factors): (Vec<usize>, Vec<usize>) = dataset
        .inputs
        .iter()
        .partition(|&&p| fields[p].measurement == Measurement::Continuous);
    let name_of = |positions: Vec<usize>| -> Vec<String> {
        positions
            .into_iter()
            .map(|p| fields[p].name.clone())
            .collect()
    };
    let mut mining_schema: Vec<_> = dataset
        .inputs
        .iter()
        .filter_map(|&p| mining_field(table, p, Usage::Active))
        .collect();
    mining_schema.extend(mining_field(table, dataset.target, Usage::Target));
    let (version, algorithm, model_type) = match settings.style {
        Style::Linear => (
            "4.3",
            Some("ALM".to_string()),
            GeneralModelType::GeneralLinear,
        ),
        Style::Regression => ("4.2", None, GeneralModelType::Regression),
    };
    Ok(Pmml {
        version: version.into(),
        data_dictionary: std::iter::once(dataset.target)
            .chain(dataset.inputs.iter().copied())
            .filter_map(|p| data_field(table, p))
            .collect(),
        transformations: Vec::new(),
        functions: Vec::new(),
        model: Model::GeneralRegression(GeneralRegressionModel {
            header: ModelHeader {
                name: settings.name.clone(),
                function: MiningFunction::Regression,
                algorithm,
                mining_schema,
                output: Vec::new(),
                local_transformations: Vec::new(),
            },
            model_type,
            target_reference_category: None,
            link_function: None,
            link_parameter: None,
            parameters: names,
            factors: name_of(factors),
            covariates: name_of(covariates),
            pp_matrix,
            param_matrix,
        }),
    })
}

/// Least squares by Householder QR; an (almost) aliased column gets beta 0.
#[allow(
    clippy::needless_range_loop,
    reason = "column-wise Householder updates index rows and columns together"
)]
fn least_squares(mut a: Vec<Vec<f64>>, mut y: Vec<f64>, p: usize) -> Result<Vec<f64>, AlgoError> {
    let n = a.len();
    if n < p.max(1) {
        return Err(AlgoError::Data(format!(
            "{n} complete records for {p} parameters"
        )));
    }
    let scale: Vec<f64> = (0..p)
        .map(|j| a.iter().map(|row| row[j] * row[j]).sum::<f64>().sqrt())
        .collect();
    let mut active = vec![true; p];
    let mut pivot = 0;
    let mut order = Vec::new();
    for j in 0..p {
        let norm = (pivot..n).map(|i| a[i][j] * a[i][j]).sum::<f64>().sqrt();
        if norm <= 1e-10 * scale[j].max(f64::MIN_POSITIVE) {
            active[j] = false;
            continue;
        }
        let alpha = if a[pivot][j] > 0.0 { -norm } else { norm };
        let mut v: Vec<f64> = (pivot..n).map(|i| a[i][j]).collect();
        v[0] -= alpha;
        let vv: f64 = v.iter().map(|x| x * x).sum();
        for k in j..p {
            let dot: f64 = (pivot..n).map(|i| v[i - pivot] * a[i][k]).sum();
            let f = 2.0 * dot / vv;
            for i in pivot..n {
                a[i][k] -= f * v[i - pivot];
            }
        }
        let dot: f64 = (pivot..n).map(|i| v[i - pivot] * y[i]).sum();
        let f = 2.0 * dot / vv;
        for i in pivot..n {
            y[i] -= f * v[i - pivot];
        }
        order.push((pivot, j));
        pivot += 1;
    }
    let mut beta = vec![0.0; p];
    for &(row, j) in order.iter().rev() {
        let s: f64 = (j + 1..p)
            .filter(|&k| active[k])
            .map(|k| a[row][k] * beta[k])
            .sum();
        beta[j] = (y[row] - s) / a[row][j];
    }
    Ok(beta)
}
