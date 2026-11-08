//! Time Series builder/nugget: statistical parity (optimizer), Algorithms Guide
//! (1994–2016), Time Series pp.379–394; evidence `models_next/ts_*`,
//! `models_next2/ts_*`, and the 30 `ts_more` streams. Phase B accepts 26/30 at
//! rule 3 (worst accepted max point/bound/residual error 0.008378 target SD).
//! See `exec/tests/time_series_more.rs` for both objectives and coefficients.
//!
//! Output projects the observation field first, then targets and candidate predictors in
//! source order, whether or not the model kept them (`ts_expert_tf`: `x` stays when the
//! Expert drops it; null on future rows); other Type fields are discarded, irrespective of
//! storage type.
//! `$FutureFlag` follows the period/date; then per-target $TS/$TSLCI/$TSUCI.
//! Monthly dates advance by calendar month, with consecutive month-start input
//! required. Residuals are target minus original-scale prediction; future null.
//! USER_SPECIFY predictor values extend records, including a detached grid whose
//! generic forecast flag is off (`ts_tf_*`). Saved `.es.xml` models supply predictor
//! futures in older model collections; explicit user values take precedence.
//!
//! PMML is read from the nugget's length-prefixed ZIP, not the empty exported XML.
//! Replacement-linked nuggets receive the new builder model; detached nuggets
//! keep their saved model. Explicit builders use persisted properties. In the
//! four ARIMA transform probes the keyed transform is absent from the saved
//! builder, so rerunning it gives untransformed scores while detached copies
//! retain log/sqrt. Tests independently refit the recorded transformed model
//! and remove attachments when scoring it, preventing a saved-parameter pass.
//!
//! Expert selects Exponential Smoothing (`_expertModelTypeList == "Exsmooth"`) by
//! minimum NBIC and refits recorded ARIMA/AllModels structure/transform/omitted lags.
//! Without a recorded specification, ARIMA-only Expert selects with the Algorithms Guide
//! appendix "Time Series Algorithm: Expert Modeler" (`arima::expert_arima`); AllModels also
//! fits Expert smoothing and keeps the smaller original-scale NBIC
//! (`time_series::normalized_bic`).
//! `ts_expert/pr_wn_ar` omits the zero-order NonseasonalComponent; `pr_i2_ar`
//! scores d=2 with a constant through the existing integrated ARIMA recursion.
//! Transfer scoring supports stationary ARMA noise, delay, log/sqrt predictors,
//! ordinary/seasonal differences and multiplicative seasonal factors. PMML 4.2
//! polynomial signs and collection field IDs are normalized on import. Fitting
//! retains its nonseasonal AR(1) scope (start-up rule in `dms_algo::transfer`);
//! persisted TF columns p/q/d mean numerator/difference/denominator, evidenced
//! by the ts_tf_num/den/delay saved models and complete scores.
//! Outside rule 3: ts_sarlog_noconst (0.116627 SD) and ts_tf_static (0.016416 SD), where
//! Modeler's optimizer stops short of the likelihood optimum we reach (evidence in
//! `dms_algo::time_series`). Other GAPs: irregular/aggregated observations, missing data,
//! automatic outliers/Expert, other intervals/orders and long-horizon Winters.

use super::field_util::field_list;
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::NodeOutput;
use crate::settings;
use dms_algo::arima::{self, ArimaSpec, Forecast, Transform};
use dms_algo::smooth::{self, SmoothSpec};
use dms_algo::time_series::{self as estimation, FittedSpec as SavedModel, Method};
use dms_algo::transfer::{self, Effect, TransferSpec};
use dms_pmml::Pmml;
use dms_stream::StreamNode;
use dms_strfmt::{ArchiveLimits, read_length_prefixed_zip};
use dms_types::{DataModel, Field, Measurement, Storage, Table, Value};
use std::sync::Arc;

const MAX_TARGETS: usize = 64;

fn build_setting(s: Option<&dms_strfmt::fastinfoset::Element>, key: &str) -> Option<String> {
    settings::children(s, "buildSettings", "propertySetting")
        .into_iter()
        .find(|p| settings::text(Some(p), "name").as_deref() == Some(key))
        .and_then(|p| settings::text(Some(p), "value"))
}
fn setting_or_build(s: Option<&dms_strfmt::fastinfoset::Element>, key: &str) -> Option<String> {
    settings::text(s, key).or_else(|| build_setting(s, key))
}

fn observation_field(
    s: Option<&dms_strfmt::fastinfoset::Element>,
) -> Result<(String, bool), OperatorError> {
    let monthly = !settings::flag(s, "_usePeriod")?.unwrap_or(true);
    if monthly && setting_or_build(s, "_inputInterval").as_deref() != Some("Month") {
        return Err(OperatorError::Settings(
            "GAP: date observations require Month interval".into(),
        ));
    }
    let name = settings::text(
        s,
        if monthly {
            "_dateTimeField"
        } else {
            "_periodField"
        },
    )
    .filter(|s| !s.trim().is_empty())
    .ok_or_else(|| OperatorError::Settings("GAP: missing observation field".into()))?;
    Ok((name, monthly))
}

fn validate_period(input: &Table, at: usize, monthly: bool) -> Result<(), OperatorError> {
    let mut previous = None;
    for row in input.rows() {
        let cell = row
            .get(at)
            .ok_or_else(|| OperatorError::Data("missing observation field".into()))?;
        let value = if monthly {
            let Value::Date(days) = cell else {
                return Err(OperatorError::Settings(
                    "GAP: monthly observations require dates".into(),
                ));
            };
            let (y, m, d) = dms_types::calendar::civil_from_days(*days)
                .ok_or_else(|| OperatorError::Data("invalid monthly date".into()))?;
            if d != 1 {
                return Err(OperatorError::Settings(
                    "GAP: monthly aggregation requires month-start dates".into(),
                ));
            }
            (y * 12 + i64::from(m)) as f64
        } else {
            cell.as_f64()
                .filter(|v| v.is_finite())
                .ok_or_else(|| OperatorError::Data("period must be finite numeric".into()))?
        };
        if previous.is_some_and(|p| value - p != 1.0) {
            return Err(OperatorError::Settings(
                "GAP: observations must be consecutive".into(),
            ));
        }
        previous = Some(value);
    }
    Ok(())
}

fn add_residuals(table: Table, targets: &[String]) -> Result<Table, OperatorError> {
    let mut fields = table.model().fields().to_vec();
    let mut rows = table.rows().to_vec();
    for name in targets {
        let y = table
            .model()
            .position(name)
            .ok_or_else(|| OperatorError::Data("residual target missing".into()))?;
        let p = table
            .model()
            .position(&format!("$TS-{name}"))
            .ok_or_else(|| OperatorError::Data("residual prediction missing".into()))?;
        fields.push(score_field(&format!("$TSResidual-{name}")));
        for row in &mut rows {
            row.push(
                match (
                    row.get(y).and_then(Value::as_f64),
                    row.get(p).and_then(Value::as_f64),
                ) {
                    (Some(y), Some(p)) => Value::real(y - p),
                    _ => Value::Null,
                },
            );
        }
    }
    Table::new(
        DataModel::new(fields).map_err(|e| OperatorError::Data(e.to_string()))?,
        rows,
    )
    .map_err(|e| OperatorError::Data(e.to_string()))
}

/// Statistical parity (optimizer).
pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let gap = |what: &str| OperatorError::Settings(format!("GAP: Time Series {what}"));
    let expert_requested = settings::text(s, "_method").as_deref() == Some("ExpertModeler")
        && matches!(
            settings::text(s, "_expertModelTypeList").as_deref(),
            Some("Arima" | "AllModels")
        );
    let mut expert_events = field_list(s, "_eventFields");
    for name in field_list(s, "eventFields") {
        if !expert_events.contains(&name) {
            expert_events.push(name);
        }
    }
    if !expert_requested
        && (!expert_events.is_empty() || settings::flag(s, "useEvents")?.unwrap_or(false))
    {
        return Err(gap("events require Expert ARIMA/AllModels"));
    }
    if node.properties_attachment.is_some() {
        return Err(gap(
            "external training parameters have no oracle; these four streams use inline extension properties",
        ));
    }
    for key in [
        "autoModelFieldSelection",
        "useFrequency",
        "_useFrequency",
        "_useWeight",
        "_specifyEstimationPeriod",
        "_continueEstimation",
        "_buildScoreModelOnly",
        "_arimaDetectOutliersAutomatically",
        "_esDetectOutliersAutomatically",
        "_expertOutlierDetectMethod",
        "_isConsiderNewESModels",
    ] {
        if settings::flag(s, key)?.unwrap_or(false) {
            return Err(gap(key));
        }
    }
    for key in ["_splitFields", "modelSplitFieldSelection", "_dimFieldList"] {
        if !field_list(s, key).is_empty() {
            return Err(gap(key));
        }
    }
    for key in [
        "weightField",
        "frequencyField",
        "partitionField",
        "_regressionWeightField",
        "_frequencyWeightField",
    ] {
        if settings::text(s, key).is_some_and(|v| !v.trim().is_empty()) {
            return Err(gap(key));
        }
    }
    for key in [
        "_cyclic_periods",
        "_arimaTransformType",
        "_periodEstimation",
        "_aggAndDistSetting",
        "_metricDefinitions",
        "_metricDefinitionsForScore",
    ] {
        if !settings::children(s, &format!("{key}/rows"), "row").is_empty() {
            return Err(gap(key));
        }
    }
    for (key, expected) in [
        ("_dataStructure", "single"),
        ("_futureValueTypeList", "compute"),
    ] {
        if settings::text(s, key)
            .is_some_and(|v| v != expected && !(key == "_futureValueTypeList" && v == "specify"))
        {
            return Err(gap(key));
        }
    }
    for key in ["_observationsExclude", "_arimaTfDelay"] {
        if settings::integer(s, key)?.unwrap_or(0) != 0 {
            return Err(gap(key));
        }
    }
    if !settings::flag(s, "_isSameInterval")?.unwrap_or(true) {
        return Err(gap("period preparation without a regular period field"));
    }
    let (period, monthly) = observation_field(s)?;
    let targets = field_list(s, "_targetField");
    if targets.is_empty() || targets.len() > MAX_TARGETS {
        return Err(gap("requires 1..=64 explicit target fields"));
    }
    if targets.iter().any(|name| name.len() > 4096) {
        return Err(gap("target name exceeds 4096 bytes"));
    }
    if targets
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len()
        != targets.len()
    {
        return Err(gap("duplicate targets"));
    }
    let method = match settings::text(s, "_method").as_deref() {
        Some("Exsmooth") => {
            let kind = match settings::text(s, "_esModelType").as_deref() {
                Some("Simple") => smooth::Kind::Simple,
                Some("HoltsLinearTrend") => smooth::Kind::Holt,
                Some("BrownsLinearTrend") => smooth::Kind::Brown,
                Some("DampedTrend") => smooth::Kind::Damped,
                Some("SimpleSeasonal") => smooth::Kind::Seasonal,
                Some("WintersAdditive") => smooth::Kind::WintersAdd,
                Some("WintersMultiplicative") => smooth::Kind::WintersMul,
                _ => return Err(gap("smoothing model")),
            };
            let transform = parse_transform(
                &settings::text(s, "_esTransformType").unwrap_or_else(|| "None".into()),
            )?;
            Some(
                if transform == Transform::None && kind == smooth::Kind::Simple {
                    Method::Simple
                } else if transform == Transform::None && kind == smooth::Kind::Holt {
                    Method::Holt
                } else {
                    Method::Smooth {
                        kind,
                        transform,
                        period: if monthly { 12 } else { 1 },
                    }
                },
            )
        }
        Some("Arima") => {
            let constant = settings::flag(s, "_arimaIncludeConst")?.unwrap_or(true);
            let order = |name: &str| -> Result<usize, OperatorError> {
                let v = settings::integer(s, &format!("_arimaSeasonOrder/struct/{name}"))?
                    .ok_or_else(|| gap("missing ARIMA order"))?;
                usize::try_from(v).map_err(|_| gap("negative ARIMA order"))
            };
            let (p, d, q) = (order("p")?, order("d")?, order("q")?);
            if p > 1 || d > 1 || q > 1 {
                return Err(gap("explicit ARIMA orders outside (1,1,1)"));
            }
            // No cyclic periods means seasonLength=1. The oracle retains dormant
            // seasonal controls sp=0,sd=1,sq=1; they are not additional differences.
            Some(if monthly || !constant {
                Method::SeasonalArima(estimation::ArimaOrder {
                    p,
                    d,
                    q,
                    sp: if monthly { order("sp")? } else { 0 },
                    sd: if monthly { order("sd")? } else { 0 },
                    sq: if monthly { order("sq")? } else { 0 },
                    period: if monthly { 12 } else { 1 },
                    constant,
                    transform: Transform::None,
                })
            } else {
                Method::Arima {
                    p,
                    d,
                    q,
                    transform: Transform::None,
                }
            })
        }
        Some("ExpertModeler") => match settings::text(s, "_expertModelTypeList").as_deref() {
            Some("Exsmooth") => {
                let consider_seasonal =
                    settings::flag(s, "_isConsiderSeasonalModels")?.unwrap_or(true);
                Some(Method::ExpertSmooth {
                    period: if monthly && consider_seasonal { 12 } else { 1 },
                })
            }
            Some("AllModels" | "Arima") => None,
            _ => return Err(gap("Expert model family")),
        },
        _ => return Err(gap("estimation method")),
    };
    let selected_member = node
        .containers
        .iter()
        .find(|c| c.name.as_deref() == Some("PMML"))
        .map(|c| c.member.clone());
    // Expert ARIMA/AllModels without a recorded specification selects with the Guide appendix
    // procedure (`arima::expert_arima`); AllModels also fits Expert smoothing and keeps the
    // smaller original-scale NBIC. The season length follows Expert smoothing.
    let expert_family = settings::text(s, "_expertModelTypeList");
    let expert_all_models = expert_family.as_deref() == Some("AllModels");
    let expert_period = if method.is_none()
        && (selected_member.is_none()
            || !field_list(s, "_candidateInputs").is_empty()
            || !expert_events.is_empty())
        && settings::text(s, "_method").as_deref() == Some("ExpertModeler")
        && matches!(expert_family.as_deref(), Some("Arima" | "AllModels"))
    {
        let consider_seasonal = settings::flag(s, "_isConsiderSeasonalModels")?.unwrap_or(true);
        Some(if monthly && consider_seasonal { 12 } else { 1 })
    } else {
        None
    };
    if method.is_none() && selected_member.is_none() && expert_period.is_none() {
        return Err(gap(
            "Expert automatic transformation/order selection; no recorded specification to refit",
        ));
    }
    let mut predictors = if matches!(method, Some(Method::ExpertSmooth { .. })) {
        Vec::new()
    } else {
        field_list(s, "_candidateInputs")
    };
    if expert_requested {
        for name in &expert_events {
            if !predictors.contains(name) {
                predictors.push(name.clone());
            }
        }
    }
    let mut effects = Vec::new();
    if predictors.len() > 16 {
        return Err(gap("too many transfer predictors"));
    }
    for field in predictors.iter().filter(|_| expert_period.is_none()) {
        let row = settings::children(s, "_arimaTfOrders/rows", "row")
            .into_iter()
            .find(|r| settings::text(Some(r), "key").as_deref() == Some(field))
            .ok_or_else(|| gap("missing transfer order"))?;
        let order = |name: &str| -> Result<usize, OperatorError> {
            let v = settings::integer(Some(row), &format!("struct/{name}"))?.unwrap_or(0);
            usize::try_from(v).map_err(|_| gap("negative transfer order"))
        };
        // Persisted column order is numerator, difference, denominator (p,q,d).
        // ts_tf_num/den/delay PMML and scored prefixes establish this mapping.
        let (numerator, difference, denominator, delay) =
            (order("p")?, order("q")?, order("d")?, order("delay")?);
        if numerator > 4
            || denominator > 4
            || difference > 1
            || delay > 64
            || order("sp")? != 0
            || order("sq")? != 0
            || order("sd")? != 0
            || settings::text(Some(row), "struct/transformation_type").as_deref() != Some("None")
        {
            return Err(gap("unsupported transfer orders/transform"));
        }
        effects.push(Effect {
            field: field.clone(),
            transform: Transform::None,
            seasonal_difference: 0,
            period: 1,
            delay,
            difference,
            numerator: vec![0.0; numerator + 1],
            denominator: vec![0.0; denominator],
        });
    }
    if !effects.is_empty()
        && !matches!(
            method,
            Some(Method::Arima {
                p: 1,
                d: 0,
                q: 0,
                transform: Transform::None
            })
        )
    {
        return Err(gap("transfer requires constant untransformed AR(1) noise"));
    }
    Ok(Box::new(TimeSeriesBuild {
        expert_events,
        expert_inputs: if expert_period.is_some() {
            predictors
        } else {
            Vec::new()
        },
        effects,
        method,
        targets,
        period,
        selected_member,
        expert_period,
        expert_all_models,
        monthly,
    }))
}

struct TimeSeriesBuild {
    expert_events: Vec<String>,
    expert_inputs: Vec<String>,
    effects: Vec<Effect>,
    method: Option<Method>,
    targets: Vec<String>,
    period: String,
    selected_member: Option<String>,
    /// Season length for automatic Expert ARIMA/AllModels selection (no recorded
    /// specification).
    expert_period: Option<usize>,
    /// AllModels: compare Expert smoothing with Expert ARIMA by original-scale NBIC.
    expert_all_models: bool,
    monthly: bool,
}

impl Operator for TimeSeriesBuild {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }
    fn run(&self, _inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        Err(OperatorError::Inputs(
            "Time Series builder produces a model".into(),
        ))
    }
    fn produce(
        &self,
        inputs: &[Arc<Table>],
        _model: Option<Arc<Pmml>>,
        context: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("Time Series needs one training input".into()))?;
        if !(5..=512).contains(&input.row_count()) {
            return Err(OperatorError::Data(
                "Time Series estimation requires 5..=512 records".into(),
            ));
        }
        if input.model().fields().iter().any(|f| {
            matches!(
                f.role,
                dms_types::Role::Split | dms_types::Role::Partition | dms_types::Role::Frequency
            )
        }) {
            return Err(OperatorError::Settings(
                "GAP: Time Series partition/split/frequency role".into(),
            ));
        }
        let period = input
            .model()
            .position(&self.period)
            .ok_or_else(|| OperatorError::Data("Time Series period absent".into()))?;
        validate_period(input, period, self.monthly)?;
        let selected = if self.method.is_none() && self.expert_period.is_none() {
            let member = self
                .selected_member
                .as_deref()
                .ok_or_else(|| OperatorError::Settings("GAP: Expert automatic selection".into()))?;
            expert_specifications(member, context)?
        } else {
            Vec::new()
        };
        let mut segments = String::new();
        for target in &self.targets {
            // Explicit builders use their persisted settings. Keyed transforms absent
            // from these streams are not recovered from a pre-rebuild nugget.
            let at = input.model().position(target).ok_or_else(|| {
                OperatorError::Data(format!("Time Series target {target:?} absent"))
            })?;
            let field =
                input.model().fields().get(at).ok_or_else(|| {
                    OperatorError::Data("Time Series target metadata absent".into())
                })?;
            let y: Result<Vec<_>, _> = input
                .rows()
                .iter()
                .map(|row| {
                    row.get(at)
                        .filter(|v| !field.is_missing(v))
                        .and_then(Value::as_f64)
                        .filter(|v| v.is_finite())
                        .ok_or_else(|| {
                            OperatorError::Data(format!(
                                "GAP: Time Series missing/nonfinite target {target}"
                            ))
                        })
                })
                .collect();
            let y = y?;
            let fit = match (self.method, self.expert_period) {
                (None, Some(period)) if !self.expert_inputs.is_empty() => {
                    let mut predictors = Vec::new();
                    for name in self.expert_inputs.iter().filter(|name| *name != target) {
                        let at = input.model().position(name).ok_or_else(|| {
                            OperatorError::Data(format!("Time Series predictor {name:?} absent"))
                        })?;
                        let field = input.model().fields().get(at).ok_or_else(|| {
                            OperatorError::Data("Time Series predictor metadata absent".into())
                        })?;
                        let values = input
                            .rows()
                            .iter()
                            .map(|row| {
                                row.get(at)
                                    .filter(|v| !field.is_missing(v))
                                    .and_then(Value::as_f64)
                                    .unwrap_or(f64::NAN)
                            })
                            .collect();
                        predictors.push(dms_algo::expert_tf::Predictor {
                            field: name.clone(),
                            values,
                            forced: false,
                            event: self.expert_events.contains(name),
                        });
                    }
                    dms_algo::expert_tf::select(&y, &predictors, period, self.expert_all_models)
                        .map_err(score_message)?
                        .fit
                }
                (None, Some(period)) => {
                    let selected = arima::expert_arima(&y, period)
                        .map_err(|e| OperatorError::Data(format!("Expert ARIMA selection: {e}")))?;
                    let arima_fit = estimation::fit(&y, selected_arima_method(&selected)?)
                        .map_err(score_message)?;
                    if self.expert_all_models {
                        // AllModels keeps the family with the smaller original-scale NBIC.
                        let smooth_fit = estimation::fit(&y, Method::ExpertSmooth { period })
                            .map_err(score_message)?;
                        let nbic = |fit: &estimation::Fit| {
                            estimation::normalized_bic(&fit.spec, &y).map_err(score_message)
                        };
                        if nbic(&arima_fit)? < nbic(&smooth_fit)? {
                            arima_fit
                        } else {
                            smooth_fit
                        }
                    } else {
                        arima_fit
                    }
                }
                (method, _) => {
                    let method = match method {
                        Some(method) => method,
                        None => selected
                            .iter()
                            .find(|(name, _)| name == target)
                            .map(|(_, method)| *method)
                            .ok_or_else(|| {
                                OperatorError::Settings(format!(
                                    "GAP: Expert has no selected specification for {target}"
                                ))
                            })?,
                    };
                    if self.effects.is_empty() {
                        estimation::fit(&y, method)
                    } else {
                        let x = self
                            .effects
                            .iter()
                            .map(|e| numeric_column(input, &e.field))
                            .collect::<Result<Vec<_>, _>>()?;
                        let template = TransferSpec {
                            noise: ArimaSpec {
                                seasonal_d: 0,
                                period: 1,
                                seasonal_ar: vec![],
                                seasonal_ma: vec![],
                                transform: Transform::None,
                                constant: 0.0,
                                d: 0,
                                ar: vec![0.0],
                                ma: vec![],
                                sigma2: 1.0,
                                df: 1.0,
                            },
                            effects: self.effects.clone(),
                        };
                        transfer::fit(&y, &x, &template)
                    }
                    .map_err(score_message)?
                }
            };
            segments.push_str(&format!(
                "<Segment><True/>{}</Segment>",
                fit.to_xml(target, input.row_count())
            ));
        }
        let xml = format!(
            "<PMML version=\"4.3\"><DataDictionary/><MiningModel functionName=\"timeSeries\"><MiningSchema/><Segmentation multipleModelMethod=\"selectAll\">{segments}</Segmentation></MiningModel></PMML>"
        );
        let mut pmml = Pmml::parse(&xml).map_err(|e| OperatorError::Data(e.to_string()))?;
        let mut dictionary_names = std::collections::HashSet::new();
        pmml.data_dictionary = std::iter::once(&self.period)
            .chain(&self.targets)
            .chain(&self.expert_inputs)
            .filter(|name| dictionary_names.insert(*name))
            .filter_map(|name| {
                input
                    .model()
                    .position(name)
                    .and_then(|at| dms_algo::data_field(input, at))
            })
            .collect();
        if let dms_pmml::Model::Mining(model) = &mut pmml.model {
            model.header.mining_schema =
                self.targets
                    .iter()
                    .filter_map(|name| {
                        input.model().position(name).and_then(|at| {
                            dms_algo::mining_field(input, at, dms_pmml::Usage::Target)
                        })
                    })
                    .collect();
        }
        Ok(NodeOutput::Model(Arc::new(pmml)))
    }
}

/// Map Expert structure to a fit method, omitting lags whose coefficients are zero.
/// Seasonal orders are read only for a sparse fit: recorded dense selections have always
/// ignored that metadata, including invalid seasonal attributes. Recorded selections also
/// pass `constant: true`; automatic selections pass the selected constant flag.
fn expert_arima_method(
    (p, d, q): (usize, usize, usize),
    period: usize,
    seasonal_orders: impl FnOnce() -> Result<(usize, usize, usize), OperatorError>,
    transform: Transform,
    constant: bool,
    ar: &[f64],
    ma: &[f64],
) -> Result<Method, OperatorError> {
    let zero_mask = |coefficients: &[f64]| -> u64 {
        coefficients
            .iter()
            .take(64)
            .enumerate()
            .fold(
                0,
                |mask, (i, v)| if *v == 0.0 { mask | (1u64 << i) } else { mask },
            )
    };
    let (zero_ar, zero_ma) = (zero_mask(ar), zero_mask(ma));
    let period = period.max(1);
    Ok(
        if period == 1 && constant && zero_ar == 0 && zero_ma == 0 && p <= 1 && q <= 1 && d <= 1 {
            Method::Arima { p, d, q, transform }
        } else {
            let (sp, sd, sq) = seasonal_orders()?;
            Method::SparseArima {
                order: estimation::ArimaOrder {
                    p,
                    d,
                    q,
                    sp,
                    sd,
                    sq,
                    period,
                    constant,
                    transform,
                },
                zero_ar,
                zero_ma,
            }
        },
    )
}

fn selected_arima_method(spec: &ArimaSpec) -> Result<Method, OperatorError> {
    expert_arima_method(
        (spec.ar.len(), spec.d, spec.ma.len()),
        spec.period,
        || {
            Ok((
                spec.seasonal_ar.len(),
                spec.seasonal_d,
                spec.seasonal_ma.len(),
            ))
        },
        spec.transform,
        spec.constant != 0.0,
        &spec.ar,
        &spec.ma,
    )
}

/// Read only selected structure, not parameters, variance or initial states.
fn expert_specifications(
    member: &str,
    context: &ExecContext,
) -> Result<Vec<(String, Method)>, OperatorError> {
    let bytes = context
        .attachments
        .get(member)
        .ok_or_else(|| OperatorError::Io(format!("missing {member}")))?;
    let entries = read_length_prefixed_zip(bytes, &ArchiveLimits::default())
        .map_err(|e| OperatorError::Data(e.to_string()))?;
    let names = datamodel_targets(&entries)?;
    let mut selected = Vec::new();
    if names.is_empty() || names.len() > MAX_TARGETS {
        return Err(OperatorError::Settings(
            "GAP: Expert requires 1..=64 recorded series specifications".into(),
        ));
    }
    for (index, name) in names.iter().enumerate() {
        let entry = entries
            .iter()
            .find(|e| e.name == format!("{index}.xml"))
            .ok_or_else(|| OperatorError::Data("Expert PMML missing series".into()))?;
        let text =
            std::str::from_utf8(&entry.bytes).map_err(|e| OperatorError::Data(e.to_string()))?;
        if text.len() > 16 * 1024 * 1024 {
            return Err(OperatorError::Data("Expert PMML exceeds 16 MiB".into()));
        }
        let doc =
            roxmltree::Document::parse(text).map_err(|e| OperatorError::Data(e.to_string()))?;
        let method = if let Some(es) = doc
            .descendants()
            .find(|n| n.has_tag_name("ExponentialSmoothing"))
        {
            let tr = es
                .children()
                .find(|n| n.has_tag_name("Trend_ExpoSmooth"))
                .and_then(|n| n.attribute("trend"));
            let season = es
                .children()
                .find(|n| n.has_tag_name("Seasonality_ExpoSmooth"));
            let kind = match (tr, season.and_then(|n| n.attribute("type"))) {
                (None, None) => smooth::Kind::Simple,
                (Some("additive"), None) => smooth::Kind::Holt,
                (None, Some("additive")) => smooth::Kind::Seasonal,
                (Some("additive"), Some("additive")) => smooth::Kind::WintersAdd,
                (Some("additive"), Some("multiplicative")) => smooth::Kind::WintersMul,
                _ => {
                    return Err(OperatorError::Settings(
                        "GAP: Expert selected smoothing structure".into(),
                    ));
                }
            };
            Method::Smooth {
                kind,
                transform: parse_transform(es.attribute("transformation").unwrap_or("none"))?,
                period: season
                    .map(|n| attr_usize(n, "period"))
                    .transpose()?
                    .unwrap_or(1),
            }
        } else {
            let arima = doc
                .descendants()
                .find(|n| n.has_tag_name("ARIMA"))
                .ok_or_else(|| OperatorError::Settings("GAP: Expert model family".into()))?;
            let ns = arima
                .descendants()
                .find(|n| n.has_tag_name("NonseasonalComponent"))
                .ok_or_else(|| OperatorError::Data("Expert order missing".into()))?;
            let season = arima
                .descendants()
                .find(|n| n.has_tag_name("SeasonalComponent"));
            let p = attr_usize(ns, "p")?;
            let d = attr_usize(ns, "d")?;
            let q = attr_usize(ns, "q")?;
            let period = attr_usize(arima, "seasonLength")?.max(1);
            let transform = parse_transform(arima.attribute("transformation").unwrap_or("none"))?;
            expert_arima_method(
                (p, d, q),
                period,
                || {
                    Ok((
                        season
                            .map(|n| attr_usize(n, "seasonalP"))
                            .transpose()?
                            .unwrap_or(0),
                        season
                            .map(|n| attr_usize(n, "seasonalD"))
                            .transpose()?
                            .unwrap_or(0),
                        season
                            .map(|n| attr_usize(n, "seasonalQ"))
                            .transpose()?
                            .unwrap_or(0),
                    ))
                },
                transform,
                true,
                &parse_coef_array(ns, "AR", p)?,
                &parse_coef_array(ns, "MA", q)?,
            )?
        };
        selected.push((name.clone(), method));
    }
    Ok(selected)
}

fn trained_models(pmml: &Pmml) -> Result<Vec<(String, SavedModel)>, OperatorError> {
    let models: Vec<&dms_pmml::Model> = match &pmml.model {
        dms_pmml::Model::TimeSeries { .. } => vec![&pmml.model],
        dms_pmml::Model::Mining(m)
            if m.method == "selectAll" && m.segments.len() <= MAX_TARGETS =>
        {
            m.segments.iter().map(|s| s.model.as_ref()).collect()
        }
        _ => {
            return Err(OperatorError::Data(
                "Time Series nugget received a different model type".into(),
            ));
        }
    };
    models
        .iter()
        .map(|model| match model {
            dms_pmml::Model::TimeSeries { header, xml } => {
                let target = header
                    .mining_schema
                    .iter()
                    .find(|f| f.usage == dms_pmml::Usage::Target)
                    .ok_or_else(|| OperatorError::Data("TimeSeriesModel has no target".into()))?;
                Ok((target.name.clone(), parse_saved_model(xml)?))
            }
            _ => Err(OperatorError::Data(
                "Time Series segment has a different model type".into(),
            )),
        })
        .collect()
}

pub fn apply(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    // `_calculatePI` and `_isOutputOthers` do not change the scored fields: the IBM demo
    // `catalog_forecast` sets both on its nugget and its forecasts, intervals and residuals
    // match Modeler exactly without them (they shape the model viewer's tables).
    if settings::flag(s, "_outputInterval")?.unwrap_or(false) {
        return Err(OperatorError::Settings(
            "GAP: Time Series _outputInterval is not implemented".into(),
        ));
    }
    let inputs = field_list(s, "_candidateInputs");
    if inputs.len() > 16 {
        return Err(OperatorError::Settings(
            "too many transfer predictors".into(),
        ));
    }
    let future_inputs = parse_future_inputs(s)?;
    let splits = field_list(s, "_splitFields");
    if !splits.is_empty() {
        return Err(OperatorError::Settings(
            "GAP: Time Series split fields have no oracle".into(),
        ));
    }
    match settings::text(s, "_dataStructure")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("single") => {}
        Some(other) => {
            return Err(OperatorError::Settings(format!(
                "GAP: Time Series data structure {other:?} is not implemented"
            )));
        }
    }
    let forecast_span = match settings::integer(s, "_forecastSpan")? {
        None => 0,
        Some(span) if span < 0 => {
            return Err(OperatorError::Settings(format!(
                "_forecastSpan: expected a non-negative integer, found {span}"
            )));
        }
        Some(span) => usize::try_from(span)
            .map_err(|_| OperatorError::Settings(format!("_forecastSpan {span} does not fit")))?,
    };
    let linked = settings::text(s, "refreshLinkState").as_deref() == Some("TurnOnReplaceLink");
    let effective_flag = |key: &str, default: bool| -> Result<bool, OperatorError> {
        if linked && let Some(v) = build_setting(s, key) {
            return match v.as_str() {
                "true" => Ok(true),
                "false" => Ok(false),
                _ => Err(OperatorError::Settings(format!("invalid {key}"))),
            };
        }
        Ok(settings::flag(s, key)?.unwrap_or(default))
    };
    let do_forecast = effective_flag("_doForecast", false)?;
    let calculate_conf = effective_flag("_calculate_conf", true)?;
    let conf_percent = settings::real(s, "_confidenceLevelForScoring")?.unwrap_or(95.0);
    if !(conf_percent > 0.0 && conf_percent < 100.0) {
        return Err(OperatorError::Settings(format!(
            "GAP: Time Series confidence level {conf_percent} is not in (0, 100)"
        )));
    }
    let (period_field, monthly) = observation_field(s)?;
    let include = field_list(s, "_includeTargets");
    let candidates = field_list(s, "_candidateTargets");
    let targets = if !include.is_empty() {
        include
    } else {
        candidates
    };
    let pmml_member = node
        .containers
        .iter()
        .find(|container| container.name.as_deref() == Some("PMML"))
        .map(|container| container.member.clone())
        .ok_or_else(|| {
            OperatorError::Settings("the Time Series nugget has no PMML container".into())
        })?;
    Ok(Box::new(TimeSeriesApply {
        pmml_member,
        replace_link: linked,
        recorded_expert: build_setting(s, "_method").as_deref() == Some("ExpertModeler"),
        predictors: inputs,
        future_inputs,
        period_field,
        targets,
        forecast_span: if do_forecast { forecast_span } else { 0 },
        calculate_conf,
        conf_percent,
        monthly,
        residuals: effective_flag("_calculate_residuals", false)?,
    }))
}

struct TimeSeriesApply {
    /// Retain the recorded Expert artifact when scoring with saved attachments.
    /// A refit is scoreable through the attachment-free API.
    recorded_expert: bool,
    predictors: Vec<String>,
    future_inputs: Vec<(String, Vec<f64>)>,
    replace_link: bool,
    pmml_member: String,
    period_field: String,
    targets: Vec<String>,
    forecast_span: usize,
    calculate_conf: bool,
    conf_percent: f64,
    monthly: bool,
    residuals: bool,
}

impl Operator for TimeSeriesApply {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        self.score(inputs, None, context)
    }

    fn produce(
        &self,
        inputs: &[Arc<Table>],
        model: Option<Arc<Pmml>>,
        context: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        self.score(inputs, model.as_deref(), context)
            .map(|table| NodeOutput::Data(Arc::new(table)))
    }
}

impl TimeSeriesApply {
    fn score(
        &self,
        inputs: &[Arc<Table>],
        trained: Option<&Pmml>,
        context: &ExecContext,
    ) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("a Time Series nugget needs one input".into()))?;
        // Detached nuggets retain their saved model when the builder is rerun.
        // Supplying PMML without attachments remains the explicit scoring API.
        let trained = trained.filter(|_| {
            (self.replace_link && !self.recorded_expert)
                || !context.attachments.contains_key(&self.pmml_member)
        });
        let models = match trained {
            Some(pmml) => trained_models(pmml)?,
            None => load_saved_models(&self.pmml_member, context)?,
        };
        let targets = resolve_targets(&self.targets, &models)?;
        let models = if targets
            .iter()
            .all(|target| models.iter().any(|(name, _)| name == target))
        {
            targets
                .iter()
                .filter_map(|target| models.iter().find(|(name, _)| name == target).cloned())
                .collect()
        } else {
            models
        };
        if targets.len() != models.len() {
            return Err(OperatorError::Data(format!(
                "Time Series lists {} targets and {} saved models",
                targets.len(),
                models.len()
            )));
        }
        if trained.is_some()
            && targets
                .iter()
                .zip(&models)
                .any(|(target, (name, _))| target != name)
        {
            return Err(OperatorError::Settings(
                "GAP: Time Series trained targets differ from the nugget's target order".into(),
            ));
        }
        let period_at = input.model().position(&self.period_field).ok_or_else(|| {
            OperatorError::Data(format!(
                "Time Series period field {:?} is not in the input",
                self.period_field
            ))
        })?;
        let mut series: Vec<Vec<f64>> = Vec::with_capacity(targets.len());
        for name in &targets {
            let at = input.model().position(name).ok_or_else(|| {
                OperatorError::Data(format!("Time Series target {name:?} is not in the input"))
            })?;
            let mut values = Vec::with_capacity(input.row_count());
            for (index, row) in input.rows().iter().enumerate() {
                let cell = row.get(at).ok_or_else(|| {
                    OperatorError::Data(format!("Time Series row {index} is short"))
                })?;
                let value = cell.as_f64().ok_or_else(|| {
                    OperatorError::Data(format!(
                        "GAP: Time Series target {name:?} is missing or non-numeric at record {}",
                        index.saturating_add(1)
                    ))
                })?;
                values.push(value);
            }
            series.push(values);
        }
        let mut fitted: Vec<Vec<Option<Forecast>>> = Vec::with_capacity(models.len());
        let mut forecasts: Vec<Vec<Forecast>> = Vec::with_capacity(models.len());
        let horizon = self.forecast_span.max(
            self.future_inputs
                .iter()
                .map(|(_, v)| v.len())
                .max()
                .unwrap_or(0),
        );
        let mut future_inputs = self.future_inputs.clone();
        if horizon > 0
            && models
                .iter()
                .any(|(_, s)| matches!(s, SavedModel::Transfer(_)))
        {
            // Saved collections carry independent smoothing models for future
            // predictor values, including predictors which are also targets.
            for (name, predictor) in load_predictor_models(&self.pmml_member, context)? {
                if future_inputs.iter().any(|(field, _)| field == &name) {
                    continue;
                }
                let values = numeric_column(input, &name)?;
                let (_, ahead) = score_saved(&predictor, &values, horizon, self.conf_percent)?;
                future_inputs.push((name, ahead.into_iter().map(|f| f.point).collect()));
            }
        }
        for ((_name, spec), y) in models.iter().zip(&series) {
            let (history, ahead) = if let SavedModel::Transfer(spec) = spec {
                let x = spec
                    .effects
                    .iter()
                    .map(|e| {
                        let name = if input.model().position(&e.field).is_some() {
                            e.field.as_str()
                        } else {
                            e.field
                                .parse::<usize>()
                                .ok()
                                .and_then(|i| i.checked_sub(1))
                                .and_then(|i| self.predictors.get(i))
                                .map(String::as_str)
                                .ok_or_else(|| {
                                    OperatorError::Data("unknown transfer field".into())
                                })?
                        };
                        let mut values = numeric_column(input, name)?;
                        if horizon > 0 {
                            let future = future_inputs
                                .iter()
                                .find(|(n, _)| n == name)
                                .map(|(_, v)| v)
                                .ok_or_else(|| {
                                    OperatorError::Settings(
                                        "GAP: missing user-specified predictor future".into(),
                                    )
                                })?;
                            if future.len() != horizon {
                                return Err(OperatorError::Data(
                                    "predictor horizon mismatch".into(),
                                ));
                            }
                            values.extend(future);
                        }
                        Ok(values)
                    })
                    .collect::<Result<Vec<_>, OperatorError>>()?;
                let scored = spec
                    .score(y, &x, horizon, self.conf_percent)
                    .map_err(score_message)?;
                (scored.fitted, scored.forecasts)
            } else {
                score_saved(spec, y, horizon, self.conf_percent)?
            };
            fitted.push(history);
            forecasts.push(ahead);
        }
        validate_period(input, period_at, self.monthly)?;
        let result = assemble(
            input,
            period_at,
            &targets,
            &fitted,
            &forecasts,
            self.calculate_conf,
            Predictors {
                future: &self.future_inputs,
                candidates: &self.predictors,
            },
        )?;
        if self.residuals {
            add_residuals(result, &targets)
        } else {
            Ok(result)
        }
    }
}

fn score_saved(
    model: &SavedModel,
    y: &[f64],
    horizon: usize,
    conf_percent: f64,
) -> Result<(Vec<Option<Forecast>>, Vec<Forecast>), OperatorError> {
    match model {
        SavedModel::Transfer(_) => Err(OperatorError::Data(
            "transfer scoring requires predictors".into(),
        )),
        SavedModel::Arima(spec) => {
            let scored = arima::score(spec, y, horizon, conf_percent).map_err(score_message)?;
            Ok((scored.fitted, scored.forecasts))
        }
        SavedModel::Smooth(spec) => {
            let scored = smooth::score(spec, y, horizon, conf_percent).map_err(score_message)?;
            let history = scored
                .fitted
                .into_iter()
                .map(|forecast| Some(smooth_forecast(forecast)))
                .collect();
            let ahead = scored.forecasts.into_iter().map(smooth_forecast).collect();
            Ok((history, ahead))
        }
    }
}

fn score_message(error: impl std::fmt::Display) -> OperatorError {
    OperatorError::Data(error.to_string())
}

fn smooth_forecast(forecast: smooth::Forecast) -> Forecast {
    Forecast {
        point: forecast.point,
        lower: forecast.lower,
        upper: forecast.upper,
    }
}

fn resolve_targets(
    listed: &[String],
    models: &[(String, SavedModel)],
) -> Result<Vec<String>, OperatorError> {
    if listed.len() > MAX_TARGETS || models.len() > MAX_TARGETS {
        return Err(OperatorError::Data(format!(
            "Time Series has more than {MAX_TARGETS} targets"
        )));
    }
    if listed.is_empty() {
        return Ok(models.iter().map(|(name, _)| name.clone()).collect());
    }
    Ok(listed.to_vec())
}

fn load_saved_models(
    member: &str,
    context: &ExecContext,
) -> Result<Vec<(String, SavedModel)>, OperatorError> {
    let bytes = context
        .attachments
        .get(member)
        .ok_or_else(|| OperatorError::Io(format!("the stream file has no attachment {member}")))?;
    let entries = read_length_prefixed_zip(bytes, &ArchiveLimits::default())
        .map_err(|error| OperatorError::Data(format!("{member}: {error}")))?;
    let names = datamodel_targets(&entries)?;
    let mut indexed: Vec<(usize, &str)> = Vec::new();
    for entry in &entries {
        if let Some(index) = numeric_xml_name(&entry.name) {
            let text = std::str::from_utf8(&entry.bytes).map_err(|_| {
                OperatorError::Data(format!("{member}: {} is not UTF-8 PMML", entry.name))
            })?;
            indexed.push((index, text));
        }
    }
    indexed.sort_by_key(|(index, _)| *index);
    if indexed.is_empty() {
        return Err(OperatorError::Data(format!(
            "{member}: the Time Series archive has no 0.xml PMML"
        )));
    }
    if !names.is_empty() && names.len() < indexed.len() {
        return Err(OperatorError::Data(format!(
            "{member}: datamodel lists {} series and the archive has {} PMML files",
            names.len(),
            indexed.len()
        )));
    }
    let mut models = Vec::with_capacity(indexed.len());
    for (position, (_, xml)) in indexed.iter().enumerate() {
        let mut spec = parse_saved_model(xml)?;
        if let SavedModel::Transfer(transfer) = &mut spec {
            let document =
                roxmltree::Document::parse(xml).map_err(|e| OperatorError::Data(e.to_string()))?;
            if document
                .descendants()
                .any(|n| n.has_tag_name("TimeSeriesModel") && legacy_polynomial_signs(n))
            {
                for effect in &mut transfer.effects {
                    if let Ok(index) = effect.field.parse::<usize>() {
                        effect.field = names.get(index).cloned().ok_or_else(|| {
                            OperatorError::Data("unknown legacy transfer field index".into())
                        })?;
                    }
                }
            }
        }
        let name = names
            .get(position)
            .cloned()
            .unwrap_or_else(|| format!("{position}"));
        models.push((name, spec));
    }
    Ok(models)
}

fn numeric_xml_name(name: &str) -> Option<usize> {
    let stem = name.strip_suffix(".xml")?;
    if stem.bytes().all(|b| b.is_ascii_digit()) {
        stem.parse().ok()
    } else {
        None
    }
}

fn load_predictor_models(
    member: &str,
    context: &ExecContext,
) -> Result<Vec<(String, SavedModel)>, OperatorError> {
    let Some(bytes) = context.attachments.get(member) else {
        return Ok(Vec::new());
    };
    let entries = read_length_prefixed_zip(bytes, &ArchiveLimits::default())
        .map_err(|e| OperatorError::Data(e.to_string()))?;
    let names = datamodel_targets(&entries)?;
    let mut models = Vec::new();
    for entry in &entries {
        let Some(index) = entry
            .name
            .strip_suffix(".es.xml")
            .and_then(|s| s.parse::<usize>().ok())
        else {
            continue;
        };
        let name = names
            .get(index)
            .ok_or_else(|| OperatorError::Data("unknown predictor model index".into()))?;
        let xml =
            std::str::from_utf8(&entry.bytes).map_err(|e| OperatorError::Data(e.to_string()))?;
        models.push((name.clone(), parse_saved_model(xml)?));
        if models.len() > 16 {
            return Err(OperatorError::Data("too many predictor models".into()));
        }
    }
    Ok(models)
}

fn datamodel_targets(entries: &[dms_strfmt::ZipMember]) -> Result<Vec<String>, OperatorError> {
    let Some(entry) = entries.iter().find(|entry| entry.name == "datamodel.xml") else {
        return Ok(Vec::new());
    };
    let text = std::str::from_utf8(&entry.bytes)
        .map_err(|_| OperatorError::Data("Time Series datamodel.xml is not UTF-8".into()))?;
    let document = roxmltree::Document::parse(text)
        .map_err(|error| OperatorError::Data(format!("Time Series datamodel.xml: {error}")))?;
    let mut names = Vec::new();
    for field in document.descendants().filter(|n| n.has_tag_name("Field")) {
        if field.attribute("name") != Some("metricField") {
            continue;
        }
        for value in field.descendants().filter(|n| n.has_tag_name("Value")) {
            if let Some(name) = value.attribute("value") {
                names.push(name.to_string());
            }
        }
    }
    Ok(names)
}

pub fn parse_saved_model(xml: &str) -> Result<SavedModel, OperatorError> {
    if xml.len() > 16 * 1024 * 1024 {
        return Err(OperatorError::Data(
            "Time Series PMML exceeds 16 MiB".into(),
        ));
    }
    let document = roxmltree::Document::parse(xml)
        .map_err(|error| OperatorError::Data(format!("Time Series PMML: {error}")))?;
    let model = document
        .descendants()
        .find(|n| n.has_tag_name("TimeSeriesModel"))
        .ok_or_else(|| OperatorError::Data("the nugget is not a TimeSeriesModel".into()))?;
    let has_smooth = model
        .descendants()
        .any(|n| n.has_tag_name("ExponentialSmoothing"));
    let has_arima = model.descendants().any(|n| n.has_tag_name("ARIMA"));
    if has_smooth && has_arima {
        return Err(OperatorError::Settings(
            "GAP: Time Series PMML mixes ARIMA and ExponentialSmoothing".into(),
        ));
    }
    if has_smooth {
        return parse_smooth(model).map(SavedModel::Smooth);
    }
    let algorithm = model.attribute("algorithmName").unwrap_or("");
    let best = model.attribute("bestFit").unwrap_or("");
    if algorithm == "exponential smoothing" || best == "ExponentialSmoothing" {
        return Err(OperatorError::Data(
            "Time Series PMML names exponential smoothing but has no ExponentialSmoothing element"
                .into(),
        ));
    }
    let noise = parse_arima_model(model)?;
    let mut effects = Vec::new();
    for e in model
        .descendants()
        .filter(|n| n.has_tag_name("PredicterEffect"))
    {
        if effects.len() >= 16 {
            return Err(OperatorError::Data("too many transfer effects".into()));
        }
        let factor = |tag: &str| e.children().find(|n| n.has_tag_name(tag));
        let numerator = factor("Numerator")
            .ok_or_else(|| OperatorError::Data("missing transfer numerator".into()))?;
        let den = factor("Denominator");
        let legacy = legacy_polynomial_signs(model);
        let coefficients = parse_transfer_polynomial(numerator, true, legacy, noise.period)?;
        let denominator = den
            .map(|den| parse_transfer_polynomial(den, false, legacy, noise.period))
            .transpose()?
            .unwrap_or_default();
        effects.push(Effect {
            field: e.attribute("field").unwrap_or("").into(),
            transform: parse_transform(e.attribute("transformation").unwrap_or("none"))?,
            seasonal_difference: attr_usize(e, "seasonalDiff")?,
            period: noise.period,
            difference: attr_usize(e, "nonSeasonalDiff")?,
            delay: attr_usize(e, "delay")?,
            numerator: coefficients,
            denominator,
        });
    }
    if effects.is_empty() {
        Ok(SavedModel::Arima(noise))
    } else {
        Ok(SavedModel::Transfer(TransferSpec { noise, effects }))
    }
}

/// PMML 4.2 saved by the older Time Series engine stores polynomial
/// coefficients, whereas positiveParameters=1 stores AR/denominator parameters.
/// Keep this encoding distinction at the boundary, never in the filter.
fn legacy_polynomial_signs(model: roxmltree::Node<'_, '_>) -> bool {
    model
        .ancestors()
        .any(|n| n.has_tag_name("PMML") && n.attribute("version") == Some("4.2"))
        && !model.children().any(|n| {
            n.attribute("name") == Some("positiveParameters") && n.attribute("value") == Some("1")
        })
}

/// Normalize both multiplicative factors into a dense lag polynomial. A
/// numerator includes its gain at lag zero; seasonal factors have implicit 1.
/// Denominators are returned as additive recurrence coefficients, without 1.
fn parse_transfer_polynomial(
    parent: roxmltree::Node<'_, '_>,
    numerator: bool,
    legacy: bool,
    period: usize,
) -> Result<Vec<f64>, OperatorError> {
    let mut nonseasonal = parse_coef_array(parent, "NonseasonFactor", 0)?;
    let seasonal = parse_coef_array(parent, "SeasonFactor", 0)?;
    let sign = if legacy { 1.0 } else { -1.0 };
    for value in nonseasonal.iter_mut().skip(usize::from(numerator)) {
        *value *= sign;
    }
    if !numerator {
        nonseasonal.insert(0, 1.0);
    }
    if nonseasonal.is_empty() || !(1..=64).contains(&period) {
        return Err(OperatorError::Data("invalid transfer factor/period".into()));
    }
    let degree = nonseasonal
        .len()
        .saturating_sub(1)
        .saturating_add(seasonal.len().saturating_mul(period));
    if degree > 64 {
        return Err(OperatorError::Data(
            "transfer polynomial degree exceeds 64".into(),
        ));
    }
    let mut polynomial = vec![0.0; degree + 1];
    for (i, &value) in nonseasonal.iter().enumerate() {
        if let Some(slot) = polynomial.get_mut(i) {
            *slot += value;
        }
        for (j, &coefficient) in seasonal.iter().enumerate() {
            if let Some(slot) = polynomial.get_mut(i + (j + 1) * period) {
                *slot += value * sign * coefficient;
            }
        }
    }
    if polynomial.iter().any(|v| !v.is_finite()) {
        return Err(OperatorError::Data("nonfinite transfer polynomial".into()));
    }
    if numerator {
        Ok(polynomial)
    } else {
        Ok(polynomial.into_iter().skip(1).map(|v| -v).collect())
    }
}

fn parse_arima_model(model: roxmltree::Node<'_, '_>) -> Result<ArimaSpec, OperatorError> {
    let algorithm = model.attribute("algorithmName").unwrap_or("");
    let best = model.attribute("bestFit").unwrap_or("");
    if algorithm != "ARIMA" && best != "ARIMA" {
        return Err(OperatorError::Settings(format!(
            "GAP: Time Series algorithm {algorithm:?} / bestFit {best:?} is not ARIMA"
        )));
    }
    let mut arima_el = None;
    let mut sigma2 = None;
    let mut df = None;
    for ext in model.children().filter(|n| n.has_tag_name("Extension")) {
        if let Some(child) = ext.children().find(|n| n.has_tag_name("ARIMA")) {
            arima_el = Some(child);
        }
        match ext.attribute("name") {
            Some("errVariance") => {
                sigma2 = parse_f64_attr(ext, "value", "errVariance")?;
            }
            Some("degreesOfFreedom") => {
                df = parse_f64_attr(ext, "value", "degreesOfFreedom")?;
            }
            Some("positiveParameters") => {
                let value = ext.attribute("value").unwrap_or("");
                if value != "1" && !value.is_empty() {
                    return Err(OperatorError::Settings(format!(
                        "GAP: Time Series positiveParameters {value:?} is not implemented"
                    )));
                }
            }
            _ => {}
        }
    }
    let arima = arima_el
        .ok_or_else(|| OperatorError::Data("Time Series PMML has no ARIMA extension".into()))?;
    let transform = match arima.attribute("transformation").unwrap_or("none") {
        "none" | "" => Transform::None,
        "logarithmic" => Transform::Log,
        "squareroot" | "squareRoot" => Transform::Sqrt,
        other => {
            return Err(OperatorError::Settings(format!(
                "GAP: Time Series transformation {other:?} has no oracle"
            )));
        }
    };
    let constant = parse_required_f64(arima, "constantTerm")?;
    let period = attr_usize(arima, "seasonLength")?.max(1);
    let seasonal = arima
        .descendants()
        .find(|n| n.has_tag_name("SeasonalComponent"));
    let sp = seasonal
        .map(|n| attr_usize(n, "seasonalP"))
        .transpose()?
        .unwrap_or(0);
    let sd = seasonal
        .map(|n| attr_usize(n, "seasonalD"))
        .transpose()?
        .unwrap_or(0);
    let sq = seasonal
        .map(|n| attr_usize(n, "seasonalQ"))
        .transpose()?
        .unwrap_or(0);
    let sign = if legacy_polynomial_signs(model) {
        -1.0
    } else {
        1.0
    };
    let seasonal_ar = seasonal
        .map(|n| parse_coef_array(n, "AR", sp))
        .transpose()?
        .unwrap_or_default()
        .into_iter()
        .map(|v| v * sign)
        .collect();
    let seasonal_ma = seasonal
        .map(|n| parse_coef_array(n, "MA", sq))
        .transpose()?
        .unwrap_or_default()
        .into_iter()
        .map(|v| -v * sign)
        .collect();
    let nonseasonal = arima
        .descendants()
        .find(|n| n.has_tag_name("NonseasonalComponent"));
    // Modeler omits this component for white noise (p=d=q=0).
    let order = |name| {
        nonseasonal
            .map(|n| attr_usize(n, name))
            .transpose()
            .map(|v| v.unwrap_or(0))
    };
    let p = order("p")?;
    let d = order("d")?;
    let q = order("q")?;
    if d > 2 {
        return Err(OperatorError::Settings(format!(
            "GAP: ARIMA differencing order {d} exceeds the supported maximum 2"
        )));
    }
    if let Some(stat) = arima
        .descendants()
        .find(|n| n.has_tag_name("MaximumLikelihoodStat"))
    {
        match stat.attribute("method").unwrap_or("thetaRecursion") {
            "thetaRecursion" | "kalman" | "" => {}
            other => {
                return Err(OperatorError::Settings(format!(
                    "GAP: Time Series MaximumLikelihoodStat method {other:?} is not implemented"
                )));
            }
        }
    }
    let ar = nonseasonal
        .map(|n| parse_coef_array(n, "AR", p))
        .transpose()?
        .unwrap_or_default()
        .into_iter()
        .map(|v| v * sign)
        .collect();
    let ma_pmml = nonseasonal
        .map(|n| parse_coef_array(n, "MA", q))
        .transpose()?
        .unwrap_or_default();
    let ma: Vec<f64> = ma_pmml.into_iter().map(|theta| -theta * sign).collect();
    let sigma2 =
        sigma2.ok_or_else(|| OperatorError::Data("Time Series PMML has no errVariance".into()))?;
    let df =
        df.ok_or_else(|| OperatorError::Data("Time Series PMML has no degreesOfFreedom".into()))?;
    Ok(ArimaSpec {
        seasonal_d: sd,
        period,
        seasonal_ar,
        seasonal_ma,
        transform,
        constant,
        d,
        ar,
        ma,
        sigma2,
        df,
    })
}

fn parse_smooth(model: roxmltree::Node<'_, '_>) -> Result<SmoothSpec, OperatorError> {
    let el = model
        .descendants()
        .find(|n| n.has_tag_name("ExponentialSmoothing"))
        .ok_or_else(|| {
            OperatorError::Data("Time Series PMML has no ExponentialSmoothing element".into())
        })?;
    let transform = el.attribute("transformation").unwrap_or("none");
    let trend = el.children().find(|n| n.has_tag_name("Trend_ExpoSmooth"));
    let seasonal = el
        .children()
        .find(|n| n.has_tag_name("Seasonality_ExpoSmooth"));
    if transform != "none" && !transform.is_empty()
        || trend.is_some_and(|t| {
            matches!(
                t.attribute("trend"),
                Some("polynomial_exponential" | "damped_additive")
            )
        })
        || model
            .children()
            .any(|n| n.attribute("name") == Some("initSeason"))
    {
        return parse_general_smooth(model, el, trend, seasonal).map(SmoothSpec::General);
    }
    if transform != "none" && !transform.is_empty() {
        return Err(OperatorError::Settings(format!(
            "GAP: exponential smoothing transformation {transform:?} has no oracle"
        )));
    }
    let mut level = None;
    let mut trend = None;
    for child in el.children().filter(|n| n.is_element()) {
        match child.tag_name().name() {
            "Level" => {
                if level.is_some() {
                    return Err(OperatorError::Data(
                        "exponential smoothing has more than one Level".into(),
                    ));
                }
                if child.attribute("quadraticSmoothedValue").is_some()
                    || child.attribute("cubicSmoothedValue").is_some()
                {
                    return Err(OperatorError::Settings(
                        "GAP: Brown exponential smoothing (quadratic or cubic smoothed level) has no oracle"
                            .into(),
                    ));
                }
                level = Some(child);
            }
            "Trend_ExpoSmooth" => {
                if trend.is_some() {
                    return Err(OperatorError::Data(
                        "exponential smoothing has more than one trend".into(),
                    ));
                }
                trend = Some(child);
            }
            "Seasonality_ExpoSmooth" => {
                return Err(OperatorError::Settings(
                    "GAP: seasonal exponential smoothing, including Winters (Seasonality_ExpoSmooth), has no oracle"
                        .into(),
                ));
            }
            other => {
                return Err(OperatorError::Settings(format!(
                    "GAP: exponential smoothing element {other:?} is not implemented"
                )));
            }
        }
    }
    let level =
        level.ok_or_else(|| OperatorError::Data("exponential smoothing has no Level".into()))?;
    let alpha = required_smoothing_f64(level, "alpha")?;
    // Reject a trend this build cannot score before asking for the numeric state,
    // so a damped or seasonal file is a GAP even when errVariance is absent.
    let gamma = match trend {
        None => None,
        Some(node) => Some(additive_gamma(node)?),
    };
    let mut sigma2 = None;
    let mut df = None;
    let mut init_level = None;
    let mut init_trend = None;
    for ext in model.children().filter(|n| n.has_tag_name("Extension")) {
        match ext.attribute("name") {
            Some("errVariance") => {
                sigma2 = parse_f64_attr(ext, "value", "errVariance")?;
            }
            Some("degreesOfFreedom") => {
                df = parse_f64_attr(ext, "value", "degreesOfFreedom")?;
            }
            Some("initLevel") => {
                init_level = parse_f64_attr(ext, "value", "initLevel")?;
            }
            Some("initTrend") => {
                init_trend = parse_f64_attr(ext, "value", "initTrend")?;
            }
            Some("periodStartIndex") => {
                let value = ext.attribute("value").unwrap_or("");
                if value != "0" && !value.is_empty() {
                    return Err(OperatorError::Settings(format!(
                        "GAP: Time Series periodStartIndex {value:?} is not implemented"
                    )));
                }
            }
            Some("positiveParameters") => {
                let value = ext.attribute("value").unwrap_or("");
                if value != "1" && !value.is_empty() {
                    return Err(OperatorError::Settings(format!(
                        "GAP: Time Series positiveParameters {value:?} is not implemented"
                    )));
                }
            }
            _ => {}
        }
    }
    let sigma2 =
        sigma2.ok_or_else(|| OperatorError::Data("Time Series PMML has no errVariance".into()))?;
    let df =
        df.ok_or_else(|| OperatorError::Data("Time Series PMML has no degreesOfFreedom".into()))?;
    let init_level = init_level
        .ok_or_else(|| OperatorError::Data("exponential smoothing has no initLevel".into()))?;
    let Some(gamma) = gamma else {
        if init_trend.is_some() {
            return Err(OperatorError::Settings(
                "GAP: exponential smoothing initTrend without a trend element has no oracle".into(),
            ));
        }
        return Ok(SmoothSpec::Simple {
            alpha,
            init_level,
            sigma2,
            df,
        });
    };
    let init_trend = init_trend
        .ok_or_else(|| OperatorError::Data("Holt exponential smoothing has no initTrend".into()))?;
    Ok(SmoothSpec::Holt {
        alpha,
        gamma,
        init_level,
        init_trend,
        sigma2,
        df,
    })
}

fn parse_transform(value: &str) -> Result<Transform, OperatorError> {
    match value {
        "none" | "None" | "" => Ok(Transform::None),
        "NaturalLog" | "logarithmic" => Ok(Transform::Log),
        "SquareRoot" | "squareroot" | "squareRoot" => Ok(Transform::Sqrt),
        _ => Err(OperatorError::Settings(format!(
            "GAP: transformation {value}"
        ))),
    }
}

fn parse_general_smooth(
    model: roxmltree::Node<'_, '_>,
    el: roxmltree::Node<'_, '_>,
    trend: Option<roxmltree::Node<'_, '_>>,
    season: Option<roxmltree::Node<'_, '_>>,
) -> Result<smooth::GeneralSpec, OperatorError> {
    use smooth::Kind;
    let ext = |name: &str| {
        model
            .children()
            .find(|n| n.has_tag_name("Extension") && n.attribute("name") == Some(name))
    };
    let num = |name: &str| {
        ext(name)
            .ok_or_else(|| OperatorError::Data(format!("missing {name}")))
            .and_then(|n| parse_required_f64(n, "value"))
    };
    let kind = match (
        trend.and_then(|n| n.attribute("trend")),
        season.and_then(|n| n.attribute("type")),
    ) {
        (None, None) => Kind::Simple,
        (Some("additive"), None) => Kind::Holt,
        (Some("polynomial_exponential"), None) => Kind::Brown,
        (Some("damped_additive"), None) => Kind::Damped,
        (None, Some("additive")) => Kind::Seasonal,
        (Some("additive"), Some("additive")) => Kind::WintersAdd,
        (Some("additive"), Some("multiplicative")) => Kind::WintersMul,
        _ => {
            return Err(OperatorError::Settings(
                "GAP: smoothing component combination".into(),
            ));
        }
    };
    let level = el
        .children()
        .find(|n| n.has_tag_name("Level"))
        .ok_or_else(|| OperatorError::Data("missing Level".into()))?;
    let seasons = if let Some(season) = season {
        let period = attr_usize(season, "period")?;
        let phase = attr_usize(season, "phase")?;
        let init_season =
            ext("initSeason").ok_or_else(|| OperatorError::Data("missing initSeason".into()))?;
        let start_index = init_season
            .attribute("periodStartIndex")
            .map(|v| v.parse::<usize>())
            .transpose()
            .map_err(|_| OperatorError::Data("invalid periodStartIndex".into()))?
            .unwrap_or(0);
        if !(2..=64).contains(&period) || phase >= period || start_index != 0 {
            return Err(OperatorError::Settings("GAP: seasonal period/phase".into()));
        }
        let initial = init_season
            .children()
            .find(|n| n.has_tag_name("Array"))
            .ok_or_else(|| OperatorError::Data("missing initSeason".into()))?;
        let values = initial
            .text()
            .unwrap_or("")
            .split_whitespace()
            .take(65)
            .map(|v| parse_finite(v, "initSeason"))
            .collect::<Result<Vec<_>, _>>()?;
        if values.len() != period {
            return Err(OperatorError::Data("initSeason length mismatch".into()));
        }
        values
    } else {
        Vec::new()
    };
    Ok(smooth::GeneralSpec {
        kind,
        transform: parse_transform(el.attribute("transformation").unwrap_or("none"))?,
        alpha: parse_required_f64(level, "alpha")?,
        gamma: trend
            .filter(|_| kind != Kind::Brown)
            .map(|n| parse_required_f64(n, "gamma"))
            .transpose()?
            .unwrap_or(0.0),
        phi: trend
            .map(|n| parse_f64_attr(n, "phi", "phi"))
            .transpose()?
            .flatten()
            .unwrap_or(1.0),
        delta: season
            .map(|n| parse_required_f64(n, "delta"))
            .transpose()?
            .unwrap_or(0.0),
        initial: smooth::State {
            level: num("initLevel")?,
            trend: if kind.trend() { num("initTrend")? } else { 0.0 },
            seasons,
        },
        sigma2: num("errVariance")?,
        df: num("degreesOfFreedom")?,
    })
}

fn additive_gamma(trend: roxmltree::Node<'_, '_>) -> Result<f64, OperatorError> {
    if let Some(phi) = trend.attribute("phi").filter(|text| !text.is_empty()) {
        let phi = parse_finite(phi, "phi")?;
        if phi != 1.0 {
            return Err(OperatorError::Settings(format!(
                "GAP: damped-trend exponential smoothing (phi={phi}) has no oracle"
            )));
        }
    }
    match trend.attribute("trend").unwrap_or("") {
        "additive" => required_smoothing_f64(trend, "gamma"),
        "damped" => Err(OperatorError::Settings(
            "GAP: damped-trend exponential smoothing has no oracle".into(),
        )),
        "multiplicative" => Err(OperatorError::Settings(
            "GAP: multiplicative-trend exponential smoothing has no oracle".into(),
        )),
        "polynomial_exponential" => Err(OperatorError::Settings(
            "GAP: polynomial exponential smoothing has no oracle".into(),
        )),
        other => Err(OperatorError::Settings(format!(
            "GAP: exponential smoothing trend {other:?} has no oracle"
        ))),
    }
}

fn required_smoothing_f64(node: roxmltree::Node<'_, '_>, name: &str) -> Result<f64, OperatorError> {
    let text = node.attribute(name).ok_or_else(|| {
        OperatorError::Data(format!(
            "Time Series exponential smoothing {name} is missing"
        ))
    })?;
    parse_finite(text, name)
}

fn parse_coef_array(
    parent: roxmltree::Node<'_, '_>,
    tag: &str,
    expected: usize,
) -> Result<Vec<f64>, OperatorError> {
    let Some(el) = parent.children().find(|n| n.has_tag_name(tag)) else {
        if expected == 0 {
            return Ok(Vec::new());
        }
        return Err(OperatorError::Data(format!(
            "Time Series ARIMA {tag} is missing"
        )));
    };
    let array = el
        .children()
        .find(|n| n.has_tag_name("Array"))
        .ok_or_else(|| OperatorError::Data(format!("Time Series ARIMA {tag} has no Array")))?;
    let mut values = Vec::new();
    for token in array.text().unwrap_or("").split_whitespace() {
        let value: f64 = token.parse().map_err(|_| {
            OperatorError::Data(format!("Time Series ARIMA {tag} coefficient {token:?}"))
        })?;
        if !value.is_finite() {
            return Err(OperatorError::Data(format!(
                "Time Series ARIMA {tag} coefficient is not finite"
            )));
        }
        if values.len() >= MAX_TARGETS {
            return Err(OperatorError::Data(format!(
                "Time Series ARIMA {tag} lists too many coefficients"
            )));
        }
        values.push(value);
    }
    if expected != 0 && values.len() != expected {
        return Err(OperatorError::Data(format!(
            "Time Series ARIMA {tag} expected {expected} coefficients, found {}",
            values.len()
        )));
    }
    Ok(values)
}

fn attr_usize(node: roxmltree::Node<'_, '_>, name: &str) -> Result<usize, OperatorError> {
    match node.attribute(name) {
        None | Some("") => Ok(0),
        Some(text) => text.parse().map_err(|_| {
            OperatorError::Settings(format!(
                "Time Series ARIMA {name} {text:?} is not an integer"
            ))
        }),
    }
}

fn parse_required_f64(node: roxmltree::Node<'_, '_>, name: &str) -> Result<f64, OperatorError> {
    let text = node
        .attribute(name)
        .ok_or_else(|| OperatorError::Data(format!("Time Series ARIMA {name} is missing")))?;
    parse_finite(text, name)
}

fn parse_f64_attr(
    node: roxmltree::Node<'_, '_>,
    attr: &str,
    label: &str,
) -> Result<Option<f64>, OperatorError> {
    match node.attribute(attr) {
        None | Some("") => Ok(None),
        Some(text) => parse_finite(text, label).map(Some),
    }
}

fn parse_finite(text: &str, label: &str) -> Result<f64, OperatorError> {
    let value: f64 = text.parse().map_err(|_| {
        OperatorError::Data(format!("Time Series {label} {text:?} is not a number"))
    })?;
    if !value.is_finite() {
        return Err(OperatorError::Data(format!(
            "Time Series {label} is not finite"
        )));
    }
    Ok(value)
}

/// Predictor fields of the scored output: user-specified future values, and the candidate
/// inputs that are kept in the output whether or not the model uses them.
#[derive(Clone, Copy)]
struct Predictors<'a> {
    future: &'a [(String, Vec<f64>)],
    candidates: &'a [String],
}

fn assemble(
    input: &Table,
    period_at: usize,
    targets: &[String],
    fitted: &[Vec<Option<Forecast>>],
    forecasts: &[Vec<Forecast>],
    calculate_conf: bool,
    predictors: Predictors<'_>,
) -> Result<Table, OperatorError> {
    let Predictors {
        future: future_inputs,
        candidates: candidate_inputs,
    } = predictors;
    let horizon = forecasts.first().map(Vec::len).unwrap_or(0);
    let positions: Vec<_> = std::iter::once(period_at)
        .chain(
            input
                .model()
                .fields()
                .iter()
                .enumerate()
                .filter_map(|(i, f)| {
                    (i != period_at
                        && (targets.contains(&f.name)
                            || candidate_inputs.contains(&f.name)
                            || future_inputs.iter().any(|(n, _)| n == &f.name)))
                    .then_some(i)
                }),
        )
        .collect();
    let projected = Table::new(
        DataModel::new(
            positions
                .iter()
                .filter_map(|i| input.model().fields().get(*i).cloned())
                .collect(),
        )
        .map_err(|e| OperatorError::Data(e.to_string()))?,
        input
            .rows()
            .iter()
            .map(|r| {
                positions
                    .iter()
                    .map(|i| r.get(*i).cloned().unwrap_or(Value::Null))
                    .collect()
            })
            .collect(),
    )
    .map_err(|e| OperatorError::Data(e.to_string()))?;
    let period_at = positions
        .iter()
        .position(|i| *i == period_at)
        .ok_or_else(|| OperatorError::Data("missing projected period".into()))?;
    let input = &projected;
    let orig_width = input.model().len();
    let mut fields = input.model().fields().to_vec();
    let flag_at = period_at.saturating_add(1);
    if flag_at > orig_width {
        return Err(OperatorError::Data(
            "Time Series period field index is past the input".into(),
        ));
    }
    let mut flag = Field::new("$FutureFlag", Storage::Integer);
    flag.measurement = Measurement::Continuous;
    fields.insert(flag_at, flag);
    for name in targets {
        fields.push(score_field(&format!("$TS-{name}")));
        if calculate_conf {
            fields.push(score_field(&format!("$TSLCI-{name}")));
            fields.push(score_field(&format!("$TSUCI-{name}")));
        }
    }
    let mut rows = Vec::with_capacity(input.row_count().saturating_add(horizon));
    for (index, source) in input.rows().iter().enumerate() {
        let mut row = with_future_flag(source, flag_at, Value::Integer(0));
        push_history_scores(&mut row, fitted, index, calculate_conf);
        rows.push(row);
    }
    let mut period = input
        .rows()
        .last()
        .and_then(|row| row.get(period_at))
        .cloned()
        .unwrap_or(Value::Null);
    for h in 0..horizon {
        period = next_period(&period)?;
        let mut row = vec![Value::Null; orig_width.saturating_add(1)];
        if let Some(slot) = row.get_mut(period_at) {
            *slot = period.clone();
        }
        if let Some(slot) = row.get_mut(flag_at) {
            *slot = Value::Integer(1);
        }
        for (name, values) in future_inputs {
            if let Some(i) = input.model().position(name) {
                let i = if i >= flag_at { i + 1 } else { i };
                if let Some(slot) = row.get_mut(i) {
                    *slot = values
                        .get(h)
                        .copied()
                        .map(Value::real)
                        .unwrap_or(Value::Null);
                }
            }
        }
        push_future_scores(&mut row, forecasts, h, calculate_conf);
        rows.push(row);
    }
    let model = DataModel::new(fields).map_err(|error| OperatorError::Data(error.to_string()))?;
    Table::new(model, rows).map_err(|error| OperatorError::Data(error.to_string()))
}

fn score_field(name: &str) -> Field {
    let mut field = Field::new(name, Storage::Real);
    field.measurement = Measurement::Continuous;
    field
}

fn with_future_flag(source: &[Value], flag_at: usize, flag: Value) -> Vec<Value> {
    let mut row = Vec::with_capacity(source.len().saturating_add(1));
    for (col, value) in source.iter().enumerate() {
        if col == flag_at {
            row.push(flag.clone());
        }
        row.push(value.clone());
    }
    if flag_at == source.len() {
        row.push(flag);
    }
    row
}

fn push_history_scores(
    row: &mut Vec<Value>,
    fitted: &[Vec<Option<Forecast>>],
    index: usize,
    calculate_conf: bool,
) {
    for target in fitted {
        push_forecast(
            row,
            target.get(index).and_then(Option::as_ref),
            calculate_conf,
        );
    }
}

fn push_future_scores(
    row: &mut Vec<Value>,
    forecasts: &[Vec<Forecast>],
    h: usize,
    calculate_conf: bool,
) {
    for target in forecasts {
        push_forecast(row, target.get(h), calculate_conf);
    }
}

fn push_forecast(row: &mut Vec<Value>, cell: Option<&Forecast>, calculate_conf: bool) {
    match cell {
        None => {
            row.push(Value::Null);
            if calculate_conf {
                row.push(Value::Null);
                row.push(Value::Null);
            }
        }
        Some(forecast) => {
            row.push(Value::real(forecast.point));
            if calculate_conf {
                row.push(Value::real(forecast.lower));
                row.push(Value::real(forecast.upper));
            }
        }
    }
}

fn next_period(value: &Value) -> Result<Value, OperatorError> {
    match value {
        Value::Date(days) => {
            let (y, m, d) = dms_types::calendar::civil_from_days(*days)
                .ok_or_else(|| OperatorError::Data("date outside range".into()))?;
            let (y, m) = if m == 12 {
                (y.saturating_add(1), 1)
            } else {
                (y, m + 1)
            };
            let next = dms_types::calendar::days_from_civil(y, m, d)
                .ok_or_else(|| OperatorError::Data("invalid next month".into()))?;
            Ok(Value::Date(next))
        }
        Value::Integer(i) => Ok(Value::Integer(i.saturating_add(1))),
        Value::Real(r) => Ok(Value::real(r + 1.0)),
        Value::Null => Err(OperatorError::Data(
            "Time Series period field is empty on the last history row".into(),
        )),
        _ => Err(OperatorError::Settings(
            "GAP: Time Series period field is not numeric".into(),
        )),
    }
}

fn numeric_column(input: &Table, name: &str) -> Result<Vec<f64>, OperatorError> {
    let at = input
        .model()
        .position(name)
        .ok_or_else(|| OperatorError::Data(format!("missing predictor {name}")))?;
    input
        .rows()
        .iter()
        .map(|r| {
            r.get(at)
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite())
                .ok_or_else(|| OperatorError::Data(format!("nonfinite/missing predictor {name}")))
        })
        .collect()
}

fn parse_future_inputs(
    s: Option<&dms_strfmt::fastinfoset::Element>,
) -> Result<Vec<(String, Vec<f64>)>, OperatorError> {
    let Some(text) = build_setting(s, "_extendMetricVal") else {
        return Ok(Vec::new());
    };
    if text.len() > 131072 {
        return Err(OperatorError::Data(
            "future predictor settings exceed limit".into(),
        ));
    }
    let text = text
        .trim()
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(|| OperatorError::Data("malformed future predictor list".into()))?;
    let mut result = Vec::new();
    for entry in text.split('}').filter(|v| !v.trim().is_empty()) {
        let parts: Vec<_> = entry
            .trim()
            .trim_start_matches(',')
            .trim()
            .split('"')
            .collect();
        if result.len() >= 16
            || parts.len() != 5
            || parts.first().copied() != Some("{")
            || parts.get(3).copied() != Some("USER_SPECIFY")
        {
            return Err(OperatorError::Settings(
                "GAP: future predictors require USER_SPECIFY lists".into(),
            ));
        }
        let name = parts
            .get(1)
            .copied()
            .ok_or_else(|| OperatorError::Data("missing future field".into()))?;
        let array = parts
            .get(4)
            .copied()
            .ok_or_else(|| OperatorError::Data("missing future array".into()))?
            .trim();
        // Bound number count before allocating the JSON vector.
        if array.split(',').count() > 512 {
            return Err(OperatorError::Data(
                "future predictor span exceeds 512".into(),
            ));
        }
        let array = array
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .ok_or_else(|| OperatorError::Data("malformed future array".into()))?;
        let values: Vec<f64> = if array.trim().is_empty() {
            Vec::new()
        } else {
            array
                .split(',')
                .map(|v| parse_finite(v.trim(), "future predictor"))
                .collect::<Result<_, _>>()?
        };
        if name.is_empty()
            || name.len() > 4096
            || values.iter().any(|v| !v.is_finite())
            || result.iter().any(|(n, _)| n == name)
        {
            return Err(OperatorError::Data("invalid future predictor".into()));
        }
        result.push((name.to_string(), values));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arima_model(component: &str) -> String {
        format!(
            "<PMML><TimeSeriesModel algorithmName=\"ARIMA\" bestFit=\"ARIMA\"><Extension name=\"errVariance\" value=\"4\"/><Extension name=\"degreesOfFreedom\" value=\"20\"/><Extension><ARIMA constantTerm=\"2\" transformation=\"none\" seasonLength=\"1\">{component}</ARIMA></Extension></TimeSeriesModel></PMML>"
        )
    }

    fn transfer_model(transform: &str, legacy: bool) -> String {
        let sign = if legacy { -1.0 } else { 1.0 };
        let marker = if legacy {
            ""
        } else {
            "<Extension name=\"positiveParameters\" value=\"1\"/>"
        };
        format!(
            r#"<PMML version="4.2"><TimeSeriesModel algorithmName="ARIMA">{marker}
          <Extension name="errVariance" value="4"/><Extension name="degreesOfFreedom" value="20"/>
          <Extension><ARIMA constantTerm="2" seasonLength="3">
          <PredicterEffect field="x" transformation="{transform}" delay="2" nonSeasonalDiff="1" seasonalDiff="1">
          <Numerator><NonseasonFactor><Array>2 {}</Array></NonseasonFactor><SeasonFactor><Array>{}</Array></SeasonFactor></Numerator>
          <Denominator><NonseasonFactor><Array>{}</Array></NonseasonFactor><SeasonFactor><Array>{}</Array></SeasonFactor></Denominator>
          </PredicterEffect></ARIMA></Extension></TimeSeriesModel></PMML>"#,
            0.5 * sign,
            0.25 * sign,
            0.2 * sign,
            0.3 * sign
        )
    }

    #[test]
    fn transfer_seasonal_factors_transform_difference_and_delay() {
        for transform in ["logarithmic", "squareroot"] {
            let SavedModel::Transfer(spec) =
                parse_saved_model(&transfer_model(transform, false)).unwrap()
            else {
                panic!("transfer model");
            };
            let SavedModel::Transfer(legacy) =
                parse_saved_model(&transfer_model(transform, true)).unwrap()
            else {
                panic!("legacy transfer model");
            };
            assert_eq!(legacy, spec);
            assert_eq!(spec.effects[0].numerator, [2.0, -0.5, 0.0, -0.5, 0.125]);
            assert_eq!(spec.effects[0].denominator, [0.2, 0.0, 0.3, -0.06]);
            let z: Vec<f64> = (1..=40)
                .map(|i| 1.0 + 0.002 * f64::from(i).powi(3))
                .collect();
            let x: Vec<f64> = z
                .iter()
                .map(|v| {
                    if transform == "logarithmic" {
                        v.exp()
                    } else {
                        v * v
                    }
                })
                .collect();
            let y = vec![10.0; 36];
            let result = spec.score(&y, &[x], 4, 95.0).unwrap();
            // Independent cascade: differences followed by the numerator, then
            // two first-order recurrences (ordinary lag 1 and seasonal lag 3).
            let difference: Vec<f64> = z.windows(2).map(|v| v[1] - v[0]).collect();
            let w: Vec<f64> = difference
                .iter()
                .skip(3)
                .zip(&difference)
                .map(|(a, b)| a - b)
                .collect();
            let gain = 1.5 * 0.75;
            let mut ordinary = vec![gain * w[0] / 0.8; 40];
            let mut seasonal = vec![gain * w[0] / (0.8 * 0.7); 40];
            for t in 10..40 {
                let at = t - 6; // 4 differences and 2 delay observations.
                let forcing =
                    2.0 * w[at] - 0.5 * w[at - 1] - 0.25 * (2.0 * w[at - 3] - 0.5 * w[at - 4]);
                ordinary[t] = forcing + 0.2 * ordinary[t - 1];
                seasonal[t] = ordinary[t] + 0.3 * seasonal[t - 3];
            }
            assert!(result.fitted[..10].iter().all(Option::is_none));
            let scored = result
                .fitted
                .into_iter()
                .skip(10)
                .map(Option::unwrap)
                .chain(result.forecasts);
            for (f, v) in scored.zip(seasonal.iter().skip(10)) {
                assert!(
                    (f.point - (2.0 + v)).abs() < 1e-10,
                    "{transform}: {f:?}, {v}"
                );
                assert!(f.lower < f.point && f.upper > f.point);
            }
        }
    }

    #[test]
    fn transfer_rejects_invalid_domains_orders_and_short_history() {
        let xml = transfer_model("logarithmic", false);
        let SavedModel::Transfer(mut spec) = parse_saved_model(&xml).unwrap() else {
            panic!("transfer");
        };
        assert!(spec.score(&[1.0; 36], &[vec![0.0; 40]], 4, 95.0).is_err());
        assert!(spec.score(&[1.0; 4], &[vec![1.0; 40]], 36, 95.0).is_err());
        spec.effects[0].seasonal_difference = usize::MAX;
        assert!(spec.score(&[1.0; 36], &[vec![1.0; 40]], 4, 95.0).is_err());
        assert!(
            parse_saved_model(&xml.replace(
                "seasonLength=\"3\"",
                "seasonLength=\"18446744073709551615\""
            ))
            .is_err()
        );
        assert!(
            parse_saved_model(&xml.replace("<Array>2 0.5</Array>", "<Array>NaN 0.5</Array>"))
                .is_err()
        );
        assert!(parse_saved_model(&xml.replace("logarithmic", "unknown")).is_err());
    }

    #[test]
    fn scores_arima_without_nonseasonal_component() {
        let SavedModel::Arima(spec) = parse_saved_model(&arima_model("")).unwrap() else {
            panic!("expected ARIMA");
        };
        let explicit = parse_saved_model(&arima_model(
            "<NonseasonalComponent p=\"0\" d=\"0\" q=\"0\"/>",
        ))
        .unwrap();
        let SavedModel::Arima(explicit) = explicit else {
            panic!("expected ARIMA");
        };
        assert_eq!(spec, explicit);
        let scored = arima::score(&spec, &[1.0, 3.0, 2.0], 3, 95.0).unwrap();
        for f in scored.fitted.into_iter().flatten().chain(scored.forecasts) {
            assert_eq!(f.point, 2.0);
            assert!(f.lower < 2.0 && f.upper > 2.0);
        }
    }

    #[test]
    fn scores_arima_second_difference_with_constant() {
        let SavedModel::Arima(spec) = parse_saved_model(&arima_model(
            "<NonseasonalComponent p=\"0\" d=\"2\" q=\"0\"/>",
        ))
        .unwrap() else {
            panic!("expected ARIMA");
        };
        let scored = arima::score(&spec, &[1.0, 4.0, 9.0, 16.0], 3, 95.0).unwrap();
        assert_eq!(scored.fitted[..2], [None, None]);
        assert_eq!(scored.fitted[2].unwrap().point, 9.0);
        assert_eq!(scored.fitted[3].unwrap().point, 16.0);
        let width = scored.forecasts[0].upper - scored.forecasts[0].point;
        for (i, (point, variance)) in [(25.0, 1.0_f64), (36.0, 5.0), (49.0, 14.0)]
            .into_iter()
            .enumerate()
        {
            let f = scored.forecasts[i];
            assert_eq!(f.point, point);
            assert!(((f.upper - point) / width - variance.sqrt()).abs() < 1e-12);
            assert!(((point - f.lower) / width - variance.sqrt()).abs() < 1e-12);
        }
        assert!(
            parse_saved_model(&arima_model(
                "<NonseasonalComponent p=\"0\" d=\"3\" q=\"0\"/>"
            ))
            .is_err()
        );
    }

    fn model(body: &str) -> String {
        format!(
            "<PMML><TimeSeriesModel algorithmName=\"exponential smoothing\" bestFit=\"ExponentialSmoothing\">{body}</TimeSeriesModel></PMML>"
        )
    }

    fn gap(xml: &str) -> String {
        match parse_saved_model(xml).unwrap_err() {
            OperatorError::Settings(message) => message,
            other => panic!("expected a settings error, got {other:?}"),
        }
    }

    #[test]
    fn parses_simple_and_holt_parameters() {
        let simple = model(
            "<ExponentialSmoothing transformation=\"none\"><Level alpha=\"0.9999889207657683\" smoothedValue=\"43.9999944603215\"/></ExponentialSmoothing><Extension name=\"errVariance\" value=\"0.25000542184106844\"/><Extension name=\"degreesOfFreedom\" value=\"47\"/><Extension name=\"initLevel\" value=\"20.50000553967849\"/><Extension name=\"periodStartIndex\" value=\"0\"/>",
        );
        match parse_saved_model(&simple).unwrap() {
            SavedModel::Smooth(SmoothSpec::Simple {
                alpha,
                init_level,
                sigma2,
                df,
            }) => {
                assert_eq!(alpha, 0.9999889207657683);
                assert_eq!(init_level, 20.50000553967849);
                assert_eq!(sigma2, 0.25000542184106844);
                assert_eq!(df, 47.0);
            }
            other => panic!("expected simple smoothing, got {other:?}"),
        }
        let holt = model(
            "<ExponentialSmoothing RMSE=\"0.0\" transformation=\"none\"><Level alpha=\"0.8\" smoothedValue=\"44\"/><Trend_ExpoSmooth trend=\"additive\" gamma=\"0.4\" smoothedValue=\"0.5\"/></ExponentialSmoothing><Extension name=\"errVariance\" value=\"0.0\"/><Extension name=\"degreesOfFreedom\" value=\"46\"/><Extension name=\"initLevel\" value=\"20\"/><Extension name=\"initTrend\" value=\"0.5\"/>",
        );
        match parse_saved_model(&holt).unwrap() {
            SavedModel::Smooth(SmoothSpec::Holt {
                alpha,
                gamma,
                init_level,
                init_trend,
                sigma2,
                df,
            }) => {
                assert_eq!(alpha, 0.8);
                assert_eq!(gamma, 0.4);
                assert_eq!(init_level, 20.0);
                assert_eq!(init_trend, 0.5);
                assert_eq!(sigma2, 0.0);
                assert_eq!(df, 46.0);
            }
            other => panic!("expected Holt smoothing, got {other:?}"),
        }
    }

    #[test]
    fn rejects_smoothing_models_outside_the_oracle() {
        // Seasonal, Winters, damped, Brown and log/sqrt smoothing are implemented since
        // `ts_more`; an additive trend with a phi and a multiplicative trend still have no oracle.
        let phi = model(
            "<ExponentialSmoothing transformation=\"none\"><Level alpha=\"0.2\" smoothedValue=\"1\"/><Trend_ExpoSmooth trend=\"additive\" gamma=\"0.1\" phi=\"0.9\" smoothedValue=\"0.1\"/></ExponentialSmoothing>",
        );
        assert!(gap(&phi).contains("phi"));
        let multiplicative = model(
            "<ExponentialSmoothing transformation=\"none\"><Level alpha=\"0.2\" smoothedValue=\"1\"/><Trend_ExpoSmooth trend=\"multiplicative\" gamma=\"0.1\" smoothedValue=\"1\"/></ExponentialSmoothing>",
        );
        assert!(gap(&multiplicative).contains("multiplicative"));
    }

    #[test]
    fn parses_seasonal_smoothing_with_nonzero_terminal_phase() {
        let seasonal = model(
            "<ExponentialSmoothing transformation=\"none\"><Level alpha=\"0.1\" smoothedValue=\"80\"/><Seasonality_ExpoSmooth type=\"additive\" period=\"12\" phase=\"6\" delta=\"0.01\"><Array n=\"12\" type=\"real\">1 2 3 4 5 6 -1 -2 -3 -4 -5 -6</Array></Seasonality_ExpoSmooth></ExponentialSmoothing><Extension name=\"errVariance\" value=\"1\"/><Extension name=\"degreesOfFreedom\" value=\"16\"/><Extension name=\"initLevel\" value=\"79\"/><Extension name=\"initSeason\" periodStartIndex=\"0\" periodLength=\"18\"><Array n=\"12\" type=\"real\">1 2 3 4 5 6 -1 -2 -3 -4 -5 -6</Array></Extension>",
        );
        assert!(matches!(
            parse_saved_model(&seasonal).unwrap(),
            SavedModel::Smooth(SmoothSpec::General(_))
        ));
        let bad_start = seasonal.replace("periodStartIndex=\"0\"", "periodStartIndex=\"1\"");
        assert!(gap(&bad_start).contains("seasonal period/phase"));
    }
}
