//! Time Series nugget (`spss.ts!ts_applier`, catalog `time_series`): score the model saved
//! in the stream. Training is statistical parity (optimizer): `dms_algo::time_series`.
//! Simple/Holt and explicit ARIMA(1,1,1) are estimated from input observations.
//! Expert supports refitting its recorded log ARIMA(0,1,0) specification only;
//! automatic transformation/order selection remains GAP (Guide p.394 gives no thresholds).
//! The four streams store inline extension properties (no Parameter attachment).
//! Builder PMML is used only as Expert selection evidence, never for coefficients.
//!
//! The applier's PMML container (`contentContainers/.../container[@name=PMML]/content`,
//! `ts_arima.str` `data/0003.dat`, and the same layout in `ts_simple.str` / `ts_holt.str`)
//! is a length-prefixed ZIP. Entries `0.xml`, `1.xml` are one PMML 4.3 `TimeSeriesModel`
//! per target, in `datamodel.xml` metric order.
//!
//! Output (`oracle/models_next/ts_arima/data/*.csv`, `ts_expert`,
//! `oracle/models_next2/ts_simple`, `ts_holt`): `$FutureFlag` after the period field, then
//! per target `$TS-`, `$TSLCI-`, `$TSUCI-`. Future records (`$FutureFlag` 1) copy the
//! period (`t` 37..42 on the ARIMA series, `t` 49..54 on the smoothing series) and leave
//! the targets empty.
//!
//! ARIMA (`ts_arima`, `ts_expert`): record 1 has no prediction. Exponential smoothing
//! (`ts_simple` simple, `ts_holt` Holt linear trend): record 1 is the forecast from the
//! saved initial level (and trend). Seasonal, damped, Winters, Brown, and transformations
//! other than `none` are `GAP:` (`parse_smooth`).
//!
//! Settings honoured (saved on the nugget, scripting names in the Python guide):
//! `_forecastSpan` (`forecastperiods`, 6), `_doForecast` (`extend_records_into_future`),
//! `_calculate_conf` (`conf_limits`), `_confidenceLevelForScoring` (`conf_limit_pct`, 95).
//! Other values that would change scores are `OperatorError::Settings` with `GAP:`.

use super::field_util::field_list;
use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::output::NodeOutput;
use crate::settings;
use dms_algo::arima::{self, ArimaSpec, Forecast, Transform};
use dms_algo::smooth::{self, SmoothSpec};
use dms_algo::time_series::{self as estimation, FittedSpec as SavedModel, Method};
use dms_pmml::Pmml;
use dms_stream::StreamNode;
use dms_strfmt::{ArchiveLimits, read_length_prefixed_zip};
use dms_types::{DataModel, Field, Measurement, Storage, Table, Value};
use std::sync::Arc;

const MAX_TARGETS: usize = 64;

/// Statistical parity (optimizer). Unrecorded Expert selection is explicitly GAP.
pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let s = node.settings.as_ref();
    let gap = |what: &str| OperatorError::Settings(format!("GAP: Time Series {what}"));
    if node.properties_attachment.is_some() {
        return Err(gap(
            "external training parameters have no oracle; these four streams use inline extension properties",
        ));
    }
    for key in [
        "autoModelFieldSelection",
        "useFrequency",
        "useEvents",
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
    for key in [
        "_candidateInputs",
        "_inputFieldList",
        "_eventFields",
        "eventFields",
        "_splitFields",
        "modelSplitFieldSelection",
        "_dimFieldList",
    ] {
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
        "_dateTimeField",
    ] {
        if settings::text(s, key).is_some_and(|v| !v.trim().is_empty()) {
            return Err(gap(key));
        }
    }
    for key in [
        "_cyclic_periods",
        "_arimaTransformType",
        "_arimaTfOrders",
        "_periodEstimation",
        "_aggAndDistSetting",
        "_metricDefinitions",
        "_metricDefinitionsForScore",
        "_extendMetricVal",
    ] {
        if !settings::children(s, &format!("{key}/rows"), "row").is_empty() {
            return Err(gap(key));
        }
    }
    for (key, expected) in [
        ("_dataStructure", "single"),
        ("_esTransformType", "None"),
        ("_futureValueTypeList", "compute"),
    ] {
        if settings::text(s, key).is_some_and(|v| v != expected) {
            return Err(gap(key));
        }
    }
    for key in ["_observationsExclude", "_arimaTfDelay"] {
        if settings::integer(s, key)?.unwrap_or(0) != 0 {
            return Err(gap(key));
        }
    }
    if !settings::flag(s, "_usePeriod")?.unwrap_or(true)
        || !settings::flag(s, "_isSameInterval")?.unwrap_or(true)
    {
        return Err(gap("period preparation without a regular period field"));
    }
    let period = settings::text(s, "_periodField")
        .filter(|v| !v.is_empty())
        .ok_or_else(|| gap("missing period field"))?;
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
        Some("Exsmooth") => Some(match settings::text(s, "_esModelType").as_deref() {
            Some("Simple") => Method::Simple,
            Some("HoltsLinearTrend") => Method::Holt,
            _ => return Err(gap("smoothing model outside Simple/Holt")),
        }),
        Some("Arima") => {
            if !settings::flag(s, "_arimaIncludeConst")?.unwrap_or(true) {
                return Err(gap("ARIMA without constant"));
            }
            let order = |name: &str| -> Result<usize, OperatorError> {
                let v = settings::integer(s, &format!("_arimaSeasonOrder/struct/{name}"))?
                    .ok_or_else(|| gap("missing ARIMA order"))?;
                usize::try_from(v).map_err(|_| gap("negative ARIMA order"))
            };
            let (p, d, q) = (order("p")?, order("d")?, order("q")?);
            if (p, d, q) != (1, 1, 1) {
                return Err(gap("explicit ARIMA orders outside (1,1,1)"));
            }
            // No cyclic periods means seasonLength=1. The oracle retains dormant
            // seasonal controls sp=0,sd=1,sq=1; they are not additional differences.
            if (order("sp")?, order("sd")?, order("sq")?) != (0, 1, 1) {
                return Err(gap("unrecorded seasonal controls"));
            }
            Some(Method::Arima {
                p,
                d,
                q,
                transform: Transform::None,
            })
        }
        Some("ExpertModeler") => {
            if settings::text(s, "_expertModelTypeList").as_deref() != Some("AllModels") {
                return Err(gap("Expert model family"));
            }
            None
        }
        _ => return Err(gap("estimation method")),
    };
    let selected_member = node
        .containers
        .iter()
        .find(|c| c.name.as_deref() == Some("PMML"))
        .map(|c| c.member.clone());
    if method.is_none() && selected_member.is_none() {
        return Err(gap(
            "Expert automatic transformation/order selection; no recorded specification to refit",
        ));
    }
    Ok(Box::new(TimeSeriesBuild {
        method,
        targets,
        period,
        selected_member,
    }))
}

struct TimeSeriesBuild {
    method: Option<Method>,
    targets: Vec<String>,
    period: String,
    selected_member: Option<String>,
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
        let mut previous = None;
        for row in input.rows() {
            let value = row
                .get(period)
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite())
                .ok_or_else(|| {
                    OperatorError::Data("Time Series period must be finite numeric".into())
                })?;
            if previous.is_some_and(|p| value - p != 1.0) {
                return Err(OperatorError::Settings(
                    "GAP: Time Series requires consecutive unit periods".into(),
                ));
            }
            previous = Some(value);
        }
        let selected = if self.method.is_none() {
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
            let method = match self.method {
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
            let fit = estimation::fit(&y?, method).map_err(score_message)?;
            segments.push_str(&format!(
                "<Segment><True/>{}</Segment>",
                fit.to_xml(target, input.row_count())
            ));
        }
        let xml = format!(
            "<PMML version=\"4.3\"><DataDictionary/><MiningModel functionName=\"timeSeries\"><MiningSchema/><Segmentation multipleModelMethod=\"selectAll\">{segments}</Segmentation></MiningModel></PMML>"
        );
        let mut pmml = Pmml::parse(&xml).map_err(|e| OperatorError::Data(e.to_string()))?;
        pmml.data_dictionary = std::iter::once(&self.period)
            .chain(&self.targets)
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
        let arima = doc
            .descendants()
            .find(|n| n.has_tag_name("ARIMA"))
            .ok_or_else(|| {
                OperatorError::Settings("GAP: Expert selected model outside ARIMA".into())
            })?;
        let order = arima
            .descendants()
            .find(|n| n.has_tag_name("NonseasonalComponent"))
            .ok_or_else(|| OperatorError::Data("Expert order missing".into()))?;
        if arima.attribute("transformation") != Some("logarithmic")
            || arima.attribute("seasonLength") != Some("1")
            || (
                attr_usize(order, "p")?,
                attr_usize(order, "d")?,
                attr_usize(order, "q")?,
            ) != (0, 1, 0)
        {
            return Err(OperatorError::Settings(
                "GAP: Expert selected specification outside log ARIMA(0,1,0)".into(),
            ));
        }
        for season in arima
            .descendants()
            .filter(|n| n.has_tag_name("SeasonalComponent"))
        {
            if (
                attr_usize(season, "p")?,
                attr_usize(season, "d")?,
                attr_usize(season, "q")?,
            ) != (0, 0, 0)
            {
                return Err(OperatorError::Settings(
                    "GAP: Expert seasonal specification".into(),
                ));
            }
        }
        selected.push((
            name.clone(),
            Method::Arima {
                p: 0,
                d: 1,
                q: 0,
                transform: Transform::Log,
            },
        ));
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
    if settings::flag(s, "_calculate_residuals")?.unwrap_or(false) {
        return Err(OperatorError::Settings(
            "GAP: Time Series residual fields (_calculate_residuals) have no oracle".into(),
        ));
    }
    if settings::flag(s, "_calculatePI")?.unwrap_or(false) {
        return Err(OperatorError::Settings(
            "GAP: Time Series _calculatePI is not implemented".into(),
        ));
    }
    if settings::flag(s, "_isOutputOthers")?.unwrap_or(false) {
        return Err(OperatorError::Settings(
            "GAP: Time Series _isOutputOthers is not implemented".into(),
        ));
    }
    if settings::flag(s, "_outputInterval")?.unwrap_or(false) {
        return Err(OperatorError::Settings(
            "GAP: Time Series _outputInterval is not implemented".into(),
        ));
    }
    let inputs = field_list(s, "_candidateInputs");
    if !inputs.is_empty() {
        return Err(OperatorError::Settings(
            "GAP: Time Series predictors (_candidateInputs) have no oracle".into(),
        ));
    }
    let events = field_list(s, "_eventFields");
    if !events.is_empty() {
        return Err(OperatorError::Settings(
            "GAP: Time Series events (_eventFields) have no oracle".into(),
        ));
    }
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
    let do_forecast = settings::flag(s, "_doForecast")?.unwrap_or(false);
    let calculate_conf = settings::flag(s, "_calculate_conf")?.unwrap_or(true);
    let conf_percent = settings::real(s, "_confidenceLevelForScoring")?.unwrap_or(95.0);
    if !(conf_percent > 0.0 && conf_percent < 100.0) {
        return Err(OperatorError::Settings(format!(
            "GAP: Time Series confidence level {conf_percent} is not in (0, 100)"
        )));
    }
    let period_field = settings::text(s, "_periodField")
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| {
            OperatorError::Settings("Time Series period field (_periodField) is missing".into())
        })?;
    if !settings::flag(s, "_usePeriod")?.unwrap_or(true) {
        return Err(OperatorError::Settings(
            "GAP: Time Series without a period field (_usePeriod false) has no oracle".into(),
        ));
    }
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
        period_field,
        targets,
        forecast_span: if do_forecast { forecast_span } else { 0 },
        calculate_conf,
        conf_percent,
    }))
}

struct TimeSeriesApply {
    pmml_member: String,
    period_field: String,
    targets: Vec<String>,
    forecast_span: usize,
    calculate_conf: bool,
    conf_percent: f64,
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
        let models = match trained {
            Some(pmml) => trained_models(pmml)?,
            None => load_saved_models(&self.pmml_member, context)?,
        };
        let targets = resolve_targets(&self.targets, &models)?;
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
        for ((_name, spec), y) in models.iter().zip(&series) {
            let (history, ahead) = score_saved(spec, y, self.forecast_span, self.conf_percent)?;
            fitted.push(history);
            forecasts.push(ahead);
        }
        assemble(
            input,
            period_at,
            &targets,
            &fitted,
            &forecasts,
            self.calculate_conf,
        )
    }
}

fn score_saved(
    model: &SavedModel,
    y: &[f64],
    horizon: usize,
    conf_percent: f64,
) -> Result<(Vec<Option<Forecast>>, Vec<Forecast>), OperatorError> {
    match model {
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
    if !names.is_empty() && names.len() != indexed.len() {
        return Err(OperatorError::Data(format!(
            "{member}: datamodel lists {} series and the archive has {} PMML files",
            names.len(),
            indexed.len()
        )));
    }
    let mut models = Vec::with_capacity(indexed.len());
    for (position, (_, xml)) in indexed.iter().enumerate() {
        let spec = parse_saved_model(xml)?;
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
    parse_arima_model(model).map(SavedModel::Arima)
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
        other => {
            return Err(OperatorError::Settings(format!(
                "GAP: Time Series transformation {other:?} has no oracle"
            )));
        }
    };
    let constant = parse_required_f64(arima, "constantTerm")?;
    let season = arima.attribute("seasonLength").unwrap_or("1");
    if season != "1" {
        return Err(OperatorError::Settings(format!(
            "GAP: Time Series seasonLength {season:?} is not implemented"
        )));
    }
    if let Some(seasonal) = arima
        .descendants()
        .find(|n| n.has_tag_name("SeasonalComponent"))
    {
        let p = attr_usize(seasonal, "p")?;
        let d = attr_usize(seasonal, "d")?;
        let q = attr_usize(seasonal, "q")?;
        if p != 0 || d != 0 || q != 0 {
            return Err(OperatorError::Settings(format!(
                "GAP: seasonal ARIMA({p},{d},{q}) has no oracle"
            )));
        }
    }
    let nonseasonal = arima
        .descendants()
        .find(|n| n.has_tag_name("NonseasonalComponent"))
        .ok_or_else(|| {
            OperatorError::Data("Time Series ARIMA has no NonseasonalComponent".into())
        })?;
    let p = attr_usize(nonseasonal, "p")?;
    let d = attr_usize(nonseasonal, "d")?;
    let q = attr_usize(nonseasonal, "q")?;
    if d != 1 {
        return Err(OperatorError::Settings(format!(
            "GAP: ARIMA differencing order {d} is not implemented (oracle cases use d=1)"
        )));
    }
    if let Some(stat) = arima
        .descendants()
        .find(|n| n.has_tag_name("MaximumLikelihoodStat"))
    {
        match stat.attribute("method").unwrap_or("thetaRecursion") {
            "thetaRecursion" | "" => {}
            other => {
                return Err(OperatorError::Settings(format!(
                    "GAP: Time Series MaximumLikelihoodStat method {other:?} is not implemented"
                )));
            }
        }
    }
    let ar = parse_coef_array(nonseasonal, "AR", p)?;
    let ma_pmml = parse_coef_array(nonseasonal, "MA", q)?;
    let ma: Vec<f64> = ma_pmml.into_iter().map(|theta| -theta).collect();
    let sigma2 =
        sigma2.ok_or_else(|| OperatorError::Data("Time Series PMML has no errVariance".into()))?;
    let df =
        df.ok_or_else(|| OperatorError::Data("Time Series PMML has no degreesOfFreedom".into()))?;
    Ok(ArimaSpec {
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

fn assemble(
    input: &Table,
    period_at: usize,
    targets: &[String],
    fitted: &[Vec<Option<Forecast>>],
    forecasts: &[Vec<Forecast>],
    calculate_conf: bool,
) -> Result<Table, OperatorError> {
    let horizon = forecasts.first().map(Vec::len).unwrap_or(0);
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let seasonal = model(
            "<ExponentialSmoothing transformation=\"none\"><Level alpha=\"0.2\" smoothedValue=\"1\"/><Seasonality_ExpoSmooth type=\"additive\" period=\"12\" delta=\"0.2\"/></ExponentialSmoothing>",
        );
        assert!(gap(&seasonal).starts_with("GAP:"));
        assert!(gap(&seasonal).contains("Winters"));
        let damped = model(
            "<ExponentialSmoothing transformation=\"none\"><Level alpha=\"0.2\" smoothedValue=\"1\"/><Trend_ExpoSmooth trend=\"damped\" gamma=\"0.1\" phi=\"0.8\" smoothedValue=\"0.1\"/></ExponentialSmoothing>",
        );
        assert!(gap(&damped).contains("damped"));
        let phi = model(
            "<ExponentialSmoothing transformation=\"none\"><Level alpha=\"0.2\" smoothedValue=\"1\"/><Trend_ExpoSmooth trend=\"additive\" gamma=\"0.1\" phi=\"0.9\" smoothedValue=\"0.1\"/></ExponentialSmoothing>",
        );
        assert!(gap(&phi).contains("phi"));
        let brown = model(
            "<ExponentialSmoothing transformation=\"none\"><Level alpha=\"0.2\" smoothedValue=\"1\" quadraticSmoothedValue=\"0.4\"/></ExponentialSmoothing>",
        );
        assert!(gap(&brown).contains("Brown"));
        let logged = model(
            "<ExponentialSmoothing transformation=\"logarithmic\"><Level alpha=\"0.2\" smoothedValue=\"1\"/></ExponentialSmoothing><Extension name=\"errVariance\" value=\"1\"/><Extension name=\"degreesOfFreedom\" value=\"10\"/><Extension name=\"initLevel\" value=\"1\"/>",
        );
        assert!(gap(&logged).contains("transformation"));
        let multiplicative = model(
            "<ExponentialSmoothing transformation=\"none\"><Level alpha=\"0.2\" smoothedValue=\"1\"/><Trend_ExpoSmooth trend=\"multiplicative\" gamma=\"0.1\" smoothedValue=\"1\"/></ExponentialSmoothing>",
        );
        assert!(gap(&multiplicative).contains("multiplicative"));
    }
}
