//! Time Series nugget (`spss.ts!ts_applier`, catalog `time_series`): score the ARIMA model
//! saved in the stream. Training (`spss.ts!ts_builder`) stays a GAP.
//!
//! The applier's PMML container (`contentContainers/.../container[@name=PMML]/content`,
//! `ts_arima.str` `data/0003.dat`) is a length-prefixed ZIP. Entries `0.xml`, `1.xml` are
//! one PMML 4.3 `TimeSeriesModel` per target, in `datamodel.xml` metric order.
//!
//! Output (`oracle/models_next/ts_arima/data/*.csv`, `ts_expert`): `$FutureFlag` after the
//! period field, then per target `$TS-`, `$TSLCI-`, `$TSUCI-`. Record 1 has no prediction;
//! future records (`$FutureFlag` 1) copy the period (`t` 37..42) and leave the targets empty.
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
use dms_pmml::Pmml;
use dms_stream::StreamNode;
use dms_strfmt::{ArchiveLimits, read_length_prefixed_zip};
use dms_types::{DataModel, Field, Measurement, Storage, Table, Value};
use std::sync::Arc;

const MAX_TARGETS: usize = 64;

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
        self.score(inputs, context)
    }

    fn produce(
        &self,
        inputs: &[Arc<Table>],
        _model: Option<Arc<Pmml>>,
        context: &ExecContext,
    ) -> Result<NodeOutput, OperatorError> {
        self.score(inputs, context)
            .map(|table| NodeOutput::Data(Arc::new(table)))
    }
}

impl TimeSeriesApply {
    fn score(&self, inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError> {
        let input = inputs
            .first()
            .ok_or_else(|| OperatorError::Inputs("a Time Series nugget needs one input".into()))?;
        let models = load_saved_models(&self.pmml_member, context)?;
        let targets = resolve_targets(&self.targets, &models)?;
        if targets.len() != models.len() {
            return Err(OperatorError::Data(format!(
                "Time Series lists {} targets and {} saved models",
                targets.len(),
                models.len()
            )));
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
            let scored =
                arima::score(spec, y, self.forecast_span, self.conf_percent).map_err(|error| {
                    match error {
                        arima::ArimaError::Settings(message) | arima::ArimaError::Data(message) => {
                            OperatorError::Data(message)
                        }
                    }
                })?;
            fitted.push(scored.fitted);
            forecasts.push(scored.forecasts);
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

fn resolve_targets(
    listed: &[String],
    models: &[(String, ArimaSpec)],
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
) -> Result<Vec<(String, ArimaSpec)>, OperatorError> {
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
        let spec = parse_arima_pmml(xml)?;
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

fn parse_arima_pmml(xml: &str) -> Result<ArimaSpec, OperatorError> {
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
    let algorithm = model.attribute("algorithmName").unwrap_or("");
    let best = model.attribute("bestFit").unwrap_or("");
    if algorithm != "ARIMA" && best != "ARIMA" {
        return Err(OperatorError::Settings(format!(
            "GAP: Time Series algorithm {algorithm:?} / bestFit {best:?} is not ARIMA"
        )));
    }
    if model
        .descendants()
        .any(|n| n.has_tag_name("ExponentialSmoothing"))
    {
        return Err(OperatorError::Settings(
            "GAP: exponential smoothing Time Series models have no oracle".into(),
        ));
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
