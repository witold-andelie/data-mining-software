//! Ensemble (`EnsembleNode`): combines model-nugget score fields for one flag target.
//!
//! The methods below are the ones `corpus/models_next2/ens_*.str` saves. Any other
//! `ensembleFlagsMethod` is [`OperatorError::Settings`]. `ensembleFlagTiesMethod` is read for
//! every method. `random`, `rawPropensity` and `adjustedPropensity` are accepted at parse time
//! and reported as `GAP:` only when a voting-method tie actually happens.
//! `highestConfidence` and the two average methods ignore that setting (`ens_highest` and
//! `ens_avg_raw` store `random`).
//!
//! Rules. Every oracle row has agreeing labels, so a disagreeing-label result is the algorithms
//! guide's posterior sum (locked by `nodes_ensemble`, not by a corpus row):
//!
//! - Voting (`ens_voting`, `voting`, tie `highestConfidence`): plurality of hard predictions.
//!   `$XFC-<target>` is the winner's share of the votes cast. Unanimous rows are 1, including
//!   train id 1 where both confidences are 0.987804878048780.
//! - Confidence-weighted (`ens_confidence`, `confidenceWeightedVoting`): a model that predicts
//!   a flag value with confidence `c` adds `c` to that value and `1 - c` to the other.
//!   `$XFC` is the winning sum divided by the number of models that voted. Agreeing labels
//!   reduce to the mean of the confidences (train id 196: 0.989537686240140).
//! - Raw-propensity-weighted (`ens_raw_vote`, `rawPropensityWeightedVoting`): weight true by the
//!   raw propensity and false by `1 - propensity`. The output is `$XF` and `$XFC`, with no
//!   ensemble propensity field. On `ens_raw_vote` the propensities match the confidences, so
//!   `$XFC` matches `ens_confidence`.
//! - Highest confidence (`ens_highest`, `highestConfidence`): the prediction of the model with
//!   the greatest confidence; `$XFC` is that confidence (train id 196: 0.996932515337423).
//!   Equal confidence with the same label keeps that label. Equal confidence with different
//!   labels is `GAP:` (`ens_highest` has no such row).
//! - Average raw propensity (`ens_avg_raw`, `averageRawPropensity`): `$XFRP` is the mean raw
//!   propensity and `$XF` is the true value only when that mean is greater than 0.5 (train id
//!   196: 0.010462313759860, prediction N). There is no `$XFC`. Exactly 0.5 is the false value;
//!   no oracle row is exactly 0.5.
//! - Adjusted-propensity-weighted and average adjusted propensity (`ens_adjusted_vote`,
//!   `ens_avg_adjusted`): the same arithmetic on adjusted-propensity fields. Those streams have
//!   none, and the node reports `No valid models predicting the target field 'respond'
//!   available for ensemble`.
//!
//! Component fields are recognized by name, in input order: C5.0 `$C` / `$CC` / `$CRP`, tree
//! `$R` / `$RC` / `$RRP`, logistic `$L` / `$LP` / `$LRP`, regression `$E` / `$EC` / `$ERP`,
//! neural `$N` / `$NC`. Adjusted names `$CAP` / `$RAP` / `$LAP` / `$EAP` follow that pattern;
//! no oracle file contains them (`GAP`). Neural `$RP` / `$AP` and other model families are not
//! recognized (`GAP`). A numeric suffix (`$C1-`) is a later model of the same family; `$C0-`
//! is not. `$XF` / `$XFC` / `$XFRP` / `$XFAP` are this node's own fields. The first free suffix
//! is used when those names already exist.
//!
//! A model is used only when it has the column the method needs. A null, blank, non-matching
//! or non-finite required value abstains for that record. If every model abstains, the ensemble
//! cells are null. `filterEnsembledFields` false (scripting `filter_individual_model_output`,
//! every `ens_*` stream) keeps the input fields. True drops recognized component fields for
//! this target; unrecognized fields such as a rule id stay (`GAP`). `calculateStandardError`
//! is true on every `ens_*` stream and adds no field for a flag target.
//!
//! `GAP:` continuous targets (mean, median, standard error), set targets (nominal, ordinal,
//! categorical), a flag tie method other than `highestConfidence` when a tie occurs, a
//! highest-confidence collision, and a highest-confidence tie-break that is itself tied.

use crate::context::ExecContext;
use crate::error::OperatorError;
use crate::operator::{Arity, Operator};
use crate::settings;
use dms_stream::StreamNode;
use dms_strfmt::fastinfoset::Element;
use dms_types::{DataModel, Field, Measurement, Role, Storage, Table, Value, ValueSpec};
use std::cmp::Ordering;
use std::sync::Arc;

const KNOWN_SETTINGS: &[&str] = &[
    "annotation",
    "autoLabel",
    "cacheEnabled",
    "calculateStandardError",
    "createdBy",
    "creationDate",
    "doReadValues",
    "enabled",
    "ensembleFlagTiesMethod",
    "ensembleFlagsMethod",
    "ensembleSetTiesMethod",
    "ensembleSetsMethod",
    "ensembleTargetField",
    "fieldMappingEssentialFields",
    "filterEnsembledFields",
    "keywords",
    "label",
    "lastSavedAs",
    "lastSavedBy",
    "lastSavedDate",
    "name",
    "position",
    "repositoryLastModified",
    "repositoryPath",
    "repositoryVersionMarker",
    "scriptExecutionModeChange",
    "toolTipText",
];

/// Longest prefix first. `$RP-` / `$AP-` (neural propensity) are absent on purpose.
const PREFIXES: &[(&str, Family, Slot)] = &[
    ("CRP", Family::C50, Slot::Raw),
    ("CAP", Family::C50, Slot::Adjusted),
    ("RRP", Family::Tree, Slot::Raw),
    ("RAP", Family::Tree, Slot::Adjusted),
    ("LRP", Family::Logistic, Slot::Raw),
    ("LAP", Family::Logistic, Slot::Adjusted),
    ("ERP", Family::Regression, Slot::Raw),
    ("EAP", Family::Regression, Slot::Adjusted),
    ("CC", Family::C50, Slot::Confidence),
    ("RC", Family::Tree, Slot::Confidence),
    ("LP", Family::Logistic, Slot::Confidence),
    ("EC", Family::Regression, Slot::Confidence),
    ("NC", Family::Neural, Slot::Confidence),
    ("C", Family::C50, Slot::Prediction),
    ("R", Family::Tree, Slot::Prediction),
    ("L", Family::Logistic, Slot::Prediction),
    ("N", Family::Neural, Slot::Prediction),
    ("E", Family::Regression, Slot::Prediction),
];

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Family {
    C50,
    Tree,
    Logistic,
    Regression,
    Neural,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Prediction,
    Confidence,
    Raw,
    Adjusted,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Method {
    Voting,
    ConfidenceWeighted,
    RawWeighted,
    AdjustedWeighted,
    Highest,
    AverageRaw,
    AverageAdjusted,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tie {
    Random,
    HighestConfidence,
    RawPropensity,
    AdjustedPropensity,
}

#[derive(Clone)]
struct Component {
    family: Family,
    index: u32,
    order: usize,
    prediction: Option<usize>,
    confidence: Option<usize>,
    raw: Option<usize>,
    adjusted: Option<usize>,
}

struct Ensemble {
    target: String,
    method: Method,
    tie: Tie,
    filter_individual: bool,
}

struct ScoreCtx<'a> {
    model: &'a DataModel,
    true_value: &'a Value,
    false_value: &'a Value,
    target: &'a str,
    tie: Tie,
}

struct Side {
    votes: u32,
    confidence: Option<f64>,
    value: Value,
}

pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError> {
    let settings = node.settings.as_ref();
    reject_unknown(settings)?;
    // Read so a malformed flag fails. A flag target has no standard-error field (ens_voting).
    let _standard_error = settings::flag(settings, "calculateStandardError")?;
    let filter_individual = settings::flag(settings, "filterEnsembledFields")?.unwrap_or(false);
    require_enum(
        settings,
        "ensembleSetsMethod",
        &["voting", "confidenceWeightedVoting", "highestConfidence"],
    )?;
    require_enum(
        settings,
        "ensembleSetTiesMethod",
        &["random", "highestConfidence"],
    )?;
    let target = trimmed(settings, "ensembleTargetField")
        .ok_or_else(|| settings_error("ensembleTargetField is required"))?;
    let method = parse_method(
        &trimmed(settings, "ensembleFlagsMethod")
            .ok_or_else(|| settings_error("ensembleFlagsMethod is required"))?,
    )?;
    let tie = parse_tie(settings)?;
    Ok(Box::new(Ensemble {
        target,
        method,
        tie,
        filter_individual,
    }))
}

fn settings_error(message: impl Into<String>) -> OperatorError {
    OperatorError::Settings(format!("Ensemble: {}", message.into()))
}

fn reject_unknown(settings: Option<&Element>) -> Result<(), OperatorError> {
    let Some(settings) = settings else {
        return Err(settings_error("no settings"));
    };
    for child in settings.child_elements() {
        let name = child.name.local.as_ref();
        if !KNOWN_SETTINGS.contains(&name) {
            return Err(settings_error(format!("setting {name:?} is not supported")));
        }
    }
    Ok(())
}

fn trimmed(settings: Option<&Element>, path: &str) -> Option<String> {
    settings::text(settings, path)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn require_enum(
    settings: Option<&Element>,
    path: &str,
    allowed: &[&str],
) -> Result<(), OperatorError> {
    let Some(value) = trimmed(settings, path) else {
        return Ok(());
    };
    if allowed.contains(&value.as_str()) {
        Ok(())
    } else {
        Err(settings_error(format!(
            "{path}: expected {}, found {value:?}",
            allowed.join(", ")
        )))
    }
}

fn parse_method(value: &str) -> Result<Method, OperatorError> {
    Ok(match value {
        "voting" => Method::Voting,
        "confidenceWeightedVoting" => Method::ConfidenceWeighted,
        "rawPropensityWeightedVoting" => Method::RawWeighted,
        "adjustedPropensityWeightedVoting" => Method::AdjustedWeighted,
        "highestConfidence" => Method::Highest,
        "averageRawPropensity" => Method::AverageRaw,
        "averageAdjustedPropensity" => Method::AverageAdjusted,
        other => {
            return Err(settings_error(format!(
                "ensembleFlagsMethod: expected voting, confidenceWeightedVoting, \
                 rawPropensityWeightedVoting, adjustedPropensityWeightedVoting, \
                 highestConfidence, averageRawPropensity or averageAdjustedPropensity, \
                 found {other:?}"
            )));
        }
    })
}

fn parse_tie(settings: Option<&Element>) -> Result<Tie, OperatorError> {
    Ok(
        match trimmed(settings, "ensembleFlagTiesMethod").as_deref() {
            None | Some("random") => Tie::Random,
            Some("highestConfidence") => Tie::HighestConfidence,
            Some("rawPropensity") => Tie::RawPropensity,
            Some("adjustedPropensity") => Tie::AdjustedPropensity,
            Some(other) => {
                return Err(settings_error(format!(
                    "ensembleFlagTiesMethod: expected random, highestConfidence, rawPropensity \
                     or adjustedPropensity, found {other:?}"
                )));
            }
        },
    )
}

fn flag_pair(target: &Field) -> Result<(Value, Value), OperatorError> {
    match target.measurement {
        Measurement::Flag => match &target.values {
            ValueSpec::Flag {
                true_value,
                false_value,
            } => Ok((true_value.clone(), false_value.clone())),
            _ => Err(OperatorError::Data(format!(
                "Ensemble: flag target {:?} has no true/false values",
                target.name
            ))),
        },
        Measurement::Continuous => Err(settings_error(
            "GAP: continuous ensemble (mean, median, standard error) is not implemented; no \
             ens_* case has a continuous target",
        )),
        Measurement::Nominal | Measurement::Ordinal | Measurement::Categorical => {
            Err(settings_error(
                "GAP: set-target ensemble (voting, confidence-weighted voting, highest \
                 confidence) is not implemented; no ens_* case has a set target",
            ))
        }
        other => Err(settings_error(format!(
            "GAP: ensemble target {:?} measurement {other:?} is not a flag",
            target.name
        ))),
    }
}

fn no_models(target: &str) -> OperatorError {
    OperatorError::Data(format!(
        "No valid models predicting the target field '{target}' available for ensemble"
    ))
}

fn short_row() -> OperatorError {
    OperatorError::Data("Ensemble: a record is shorter than its data model".into())
}

fn too_many() -> OperatorError {
    OperatorError::Data("Ensemble: too many models on one record".into())
}

/// Bare `$C-` is index 0. `$C1-` is index 1. `$C0-` and `$C01-` are not score fields.
fn numeric_suffix(rest: &str) -> Option<u32> {
    if rest.is_empty() {
        return Some(0);
    }
    let bytes = rest.as_bytes();
    if bytes.len() > 9 || bytes.first().is_some_and(|byte| *byte == b'0') {
        return None;
    }
    if !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    rest.parse().ok()
}

fn score_head<'a>(name: &'a str, target: &str) -> Option<&'a str> {
    let marker = format!("-{target}");
    let stem = name.strip_prefix('$')?.strip_suffix(marker.as_str())?;
    if stem.is_empty() { None } else { Some(stem) }
}

fn classify(head: &str) -> Option<(Family, u32, Slot)> {
    for &(prefix, family, slot) in PREFIXES {
        if let Some(rest) = head.strip_prefix(prefix)
            && let Some(index) = numeric_suffix(rest)
        {
            return Some((family, index, slot));
        }
    }
    None
}

fn assign(component: &mut Component, slot: Slot, position: usize) {
    let cell = match slot {
        Slot::Prediction => &mut component.prediction,
        Slot::Confidence => &mut component.confidence,
        Slot::Raw => &mut component.raw,
        Slot::Adjusted => &mut component.adjusted,
    };
    if cell.is_none() {
        *cell = Some(position);
    }
}

fn discover(model: &DataModel, target: &str) -> Vec<Component> {
    let mut components: Vec<Component> = Vec::new();
    for (position, field) in model.fields().iter().enumerate() {
        let Some(head) = score_head(&field.name, target) else {
            continue;
        };
        let Some((family, index, slot)) = classify(head) else {
            continue;
        };
        if let Some(existing) = components
            .iter_mut()
            .find(|component| component.family == family && component.index == index)
        {
            assign(existing, slot, position);
        } else {
            let mut created = Component {
                family,
                index,
                order: position,
                prediction: None,
                confidence: None,
                raw: None,
                adjusted: None,
            };
            assign(&mut created, slot, position);
            components.push(created);
        }
    }
    components.sort_by(|left, right| {
        (left.order, left.family, left.index).cmp(&(right.order, right.family, right.index))
    });
    components
}

fn accepts(component: &Component, method: Method) -> bool {
    match method {
        Method::Voting => component.prediction.is_some(),
        Method::ConfidenceWeighted | Method::Highest => {
            component.prediction.is_some() && component.confidence.is_some()
        }
        Method::RawWeighted | Method::AverageRaw => component.raw.is_some(),
        Method::AdjustedWeighted | Method::AverageAdjusted => component.adjusted.is_some(),
    }
}

fn drop_mask(width: usize, components: &[Component], filter_individual: bool) -> Vec<bool> {
    let mut drop = vec![false; width];
    if !filter_individual {
        return drop;
    }
    for component in components {
        for index in [
            component.prediction,
            component.confidence,
            component.raw,
            component.adjusted,
        ]
        .into_iter()
        .flatten()
        {
            if let Some(flag) = drop.get_mut(index) {
                *flag = true;
            }
        }
    }
    drop
}

fn second_prefix(method: Method) -> &'static str {
    match method {
        Method::AverageRaw => "$XFRP",
        Method::AverageAdjusted => "$XFAP",
        Method::Voting
        | Method::ConfidenceWeighted
        | Method::RawWeighted
        | Method::AdjustedWeighted
        | Method::Highest => "$XFC",
    }
}

fn indexed(prefix: &str, index: usize, target: &str) -> String {
    if index == 0 {
        format!("{prefix}-{target}")
    } else {
        format!("{prefix}{index}-{target}")
    }
}

fn free_names(
    taken: &[String],
    method: Method,
    target: &str,
    limit: usize,
) -> Result<(String, String), OperatorError> {
    let second = second_prefix(method);
    for index in 0..=limit {
        let prediction = indexed("$XF", index, target);
        let companion = indexed(second, index, target);
        let collision = taken
            .iter()
            .any(|name| name == &prediction || name == &companion);
        if !collision {
            return Ok((prediction, companion));
        }
    }
    Err(OperatorError::Data(
        "Ensemble: no free score field name".into(),
    ))
}

fn prediction_field(
    name: String,
    storage: Storage,
    true_value: &Value,
    false_value: &Value,
) -> Field {
    let mut field = Field::new(name, storage);
    field.measurement = Measurement::Flag;
    field.role = Role::None;
    field.values = ValueSpec::Flag {
        true_value: true_value.clone(),
        false_value: false_value.clone(),
    };
    field
}

fn continuous_field(name: String) -> Field {
    let mut field = Field::new(name, Storage::Real);
    field.measurement = Measurement::Continuous;
    field.role = Role::None;
    field
}

fn finite_at(model: &DataModel, row: &[Value], index: usize) -> Result<Option<f64>, OperatorError> {
    let Some(cell) = row.get(index) else {
        return Err(short_row());
    };
    if model
        .fields()
        .get(index)
        .is_some_and(|field| field.is_missing(cell))
    {
        return Ok(None);
    }
    Ok(cell.as_f64().filter(|value| value.is_finite()))
}

fn side_of(ctx: &ScoreCtx<'_>, row: &[Value], index: usize) -> Result<Option<bool>, OperatorError> {
    let Some(cell) = row.get(index) else {
        return Err(short_row());
    };
    if ctx
        .model
        .fields()
        .get(index)
        .is_some_and(|field| field.is_missing(cell))
    {
        return Ok(None);
    }
    if cell.total_cmp(ctx.true_value) == Ordering::Equal {
        Ok(Some(true))
    } else if cell.total_cmp(ctx.false_value) == Ordering::Equal {
        Ok(Some(false))
    } else {
        Ok(None)
    }
}

fn prefer(best: &mut Option<f64>, confidence: f64) {
    let replace = match *best {
        Some(current) => confidence.total_cmp(&current) == Ordering::Greater,
        None => true,
    };
    if replace {
        *best = Some(confidence);
    }
}

fn cmp_conf(left: Option<f64>, right: Option<f64>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left.total_cmp(&right),
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => Ordering::Equal,
    }
}

fn cmp_number(left: f64, right: f64) -> Ordering {
    if left == right {
        Ordering::Equal
    } else if left > right {
        Ordering::Greater
    } else {
        Ordering::Less
    }
}

fn break_flag_tie(
    true_conf: Option<f64>,
    false_conf: Option<f64>,
    ctx: &ScoreCtx<'_>,
) -> Result<bool, OperatorError> {
    match ctx.tie {
        Tie::Random => Err(settings_error(format!(
            "GAP: flag_voting_tie_selection random is not implemented (a tie occurred on '{}')",
            ctx.target
        ))),
        Tie::RawPropensity => Err(settings_error(format!(
            "GAP: flag_voting_tie_selection rawPropensity is not implemented (a tie occurred on \
             '{}'); no ens_* row ties",
            ctx.target
        ))),
        Tie::AdjustedPropensity => Err(settings_error(format!(
            "GAP: flag_voting_tie_selection adjustedPropensity is not implemented (a tie \
             occurred on '{}'); no ens_* row ties",
            ctx.target
        ))),
        Tie::HighestConfidence => match cmp_conf(true_conf, false_conf) {
            Ordering::Greater => Ok(true),
            Ordering::Less => Ok(false),
            Ordering::Equal => Err(settings_error(format!(
                "GAP: voting tie on '{}' remains after highest confidence; random tie-breaking \
                 is not implemented",
                ctx.target
            ))),
        },
    }
}

fn note_vote(side: &mut Side, confidence: Option<f64>, value: &Value) -> Result<(), OperatorError> {
    if side.votes == 0 {
        side.value = value.clone();
    }
    side.votes = side.votes.checked_add(1).ok_or_else(too_many)?;
    if let Some(confidence) = confidence {
        prefer(&mut side.confidence, confidence);
    }
    Ok(())
}

fn vote(
    ctx: &ScoreCtx<'_>,
    row: &[Value],
    components: &[Component],
) -> Result<Option<(Value, Value)>, OperatorError> {
    let mut true_side = Side {
        votes: 0,
        confidence: None,
        value: ctx.true_value.clone(),
    };
    let mut false_side = Side {
        votes: 0,
        confidence: None,
        value: ctx.false_value.clone(),
    };
    for component in components {
        let Some(prediction) = component.prediction else {
            continue;
        };
        let Some(is_true) = side_of(ctx, row, prediction)? else {
            continue;
        };
        let confidence = match component.confidence {
            Some(index) => finite_at(ctx.model, row, index)?,
            None => None,
        };
        let Some(value) = row.get(prediction) else {
            return Err(short_row());
        };
        if is_true {
            note_vote(&mut true_side, confidence, value)?;
        } else {
            note_vote(&mut false_side, confidence, value)?;
        }
    }
    let Some(votes) = true_side.votes.checked_add(false_side.votes) else {
        return Err(too_many());
    };
    if votes == 0 {
        return Ok(None);
    }
    let winner_true = if true_side.votes > false_side.votes {
        true
    } else if false_side.votes > true_side.votes {
        false
    } else {
        break_flag_tie(true_side.confidence, false_side.confidence, ctx)?
    };
    let side = if winner_true { &true_side } else { &false_side };
    let confidence = f64::from(side.votes) / f64::from(votes);
    Ok(finite_pair(side.value.clone(), confidence))
}

fn confidence_weighted(
    ctx: &ScoreCtx<'_>,
    row: &[Value],
    components: &[Component],
) -> Result<Option<(Value, Value)>, OperatorError> {
    let mut sum_true = 0.0;
    let mut sum_false = 0.0;
    let mut count: u32 = 0;
    let mut best_true = None;
    let mut best_false = None;
    for component in components {
        let (Some(prediction), Some(confidence_index)) =
            (component.prediction, component.confidence)
        else {
            continue;
        };
        let Some(is_true) = side_of(ctx, row, prediction)? else {
            continue;
        };
        let Some(confidence) = finite_at(ctx.model, row, confidence_index)? else {
            continue;
        };
        count = count.checked_add(1).ok_or_else(too_many)?;
        // Posterior of the predicted category is the confidence; the other flag value gets the rest.
        if is_true {
            sum_true += confidence;
            sum_false += 1.0 - confidence;
            prefer(&mut best_true, confidence);
        } else {
            sum_false += confidence;
            sum_true += 1.0 - confidence;
            prefer(&mut best_false, confidence);
        }
    }
    finish_weighted(ctx, sum_true, sum_false, count, best_true, best_false)
}

fn propensity_weighted(
    ctx: &ScoreCtx<'_>,
    row: &[Value],
    components: &[Component],
    adjusted: bool,
) -> Result<Option<(Value, Value)>, OperatorError> {
    let mut sum_true = 0.0;
    let mut sum_false = 0.0;
    let mut count: u32 = 0;
    let mut best_true = None;
    let mut best_false = None;
    for component in components {
        let propensity_index = if adjusted {
            component.adjusted
        } else {
            component.raw
        };
        let Some(propensity_index) = propensity_index else {
            continue;
        };
        let Some(propensity) = finite_at(ctx.model, row, propensity_index)? else {
            continue;
        };
        count = count.checked_add(1).ok_or_else(too_many)?;
        sum_true += propensity;
        sum_false += 1.0 - propensity;
        note_propensity_tie(ctx, row, component, &mut best_true, &mut best_false)?;
    }
    finish_weighted(ctx, sum_true, sum_false, count, best_true, best_false)
}

fn note_propensity_tie(
    ctx: &ScoreCtx<'_>,
    row: &[Value],
    component: &Component,
    best_true: &mut Option<f64>,
    best_false: &mut Option<f64>,
) -> Result<(), OperatorError> {
    let Some(prediction) = component.prediction else {
        return Ok(());
    };
    let Some(is_true) = side_of(ctx, row, prediction)? else {
        return Ok(());
    };
    let Some(confidence_index) = component.confidence else {
        return Ok(());
    };
    let Some(confidence) = finite_at(ctx.model, row, confidence_index)? else {
        return Ok(());
    };
    if is_true {
        prefer(best_true, confidence);
    } else {
        prefer(best_false, confidence);
    }
    Ok(())
}

fn finish_weighted(
    ctx: &ScoreCtx<'_>,
    sum_true: f64,
    sum_false: f64,
    count: u32,
    best_true: Option<f64>,
    best_false: Option<f64>,
) -> Result<Option<(Value, Value)>, OperatorError> {
    if count == 0 || !sum_true.is_finite() || !sum_false.is_finite() {
        return Ok(None);
    }
    let winner_true = match cmp_number(sum_true, sum_false) {
        Ordering::Greater => true,
        Ordering::Less => false,
        Ordering::Equal => break_flag_tie(best_true, best_false, ctx)?,
    };
    let winning = if winner_true { sum_true } else { sum_false };
    let value = if winner_true {
        ctx.true_value.clone()
    } else {
        ctx.false_value.clone()
    };
    Ok(finite_pair(value, winning / f64::from(count)))
}

fn highest(
    ctx: &ScoreCtx<'_>,
    row: &[Value],
    components: &[Component],
) -> Result<Option<(Value, Value)>, OperatorError> {
    let mut best_confidence = None;
    let mut best_true = false;
    let mut best_value = Value::Null;
    let mut conflict = false;
    let mut found = false;
    for component in components {
        let (Some(prediction), Some(confidence_index)) =
            (component.prediction, component.confidence)
        else {
            continue;
        };
        let Some(is_true) = side_of(ctx, row, prediction)? else {
            continue;
        };
        let Some(confidence) = finite_at(ctx.model, row, confidence_index)? else {
            continue;
        };
        let Some(value) = row.get(prediction) else {
            return Err(short_row());
        };
        match best_confidence {
            None => {
                best_confidence = Some(confidence);
                best_true = is_true;
                best_value = value.clone();
                conflict = false;
                found = true;
            }
            Some(current) => match confidence.total_cmp(&current) {
                Ordering::Greater => {
                    best_confidence = Some(confidence);
                    best_true = is_true;
                    best_value = value.clone();
                    conflict = false;
                }
                Ordering::Equal => {
                    if is_true != best_true {
                        conflict = true;
                    }
                }
                Ordering::Less => {}
            },
        }
    }
    if !found {
        return Ok(None);
    }
    if conflict {
        return Err(settings_error(format!(
            "GAP: highest confidence is shared by models that disagree on '{}'; ens_highest has \
             no such row",
            ctx.target
        )));
    }
    let Some(confidence) = best_confidence else {
        return Ok(None);
    };
    Ok(finite_pair(best_value, confidence))
}

fn average(
    ctx: &ScoreCtx<'_>,
    row: &[Value],
    components: &[Component],
    adjusted: bool,
) -> Result<Option<(Value, Value)>, OperatorError> {
    let mut sum = 0.0;
    let mut count: u32 = 0;
    for component in components {
        let index = if adjusted {
            component.adjusted
        } else {
            component.raw
        };
        let Some(index) = index else {
            continue;
        };
        let Some(propensity) = finite_at(ctx.model, row, index)? else {
            continue;
        };
        count = count.checked_add(1).ok_or_else(too_many)?;
        sum += propensity;
    }
    if count == 0 || !sum.is_finite() {
        return Ok(None);
    }
    let mean = sum / f64::from(count);
    // The guide scores true only when the average is greater than 0.5. Exactly 0.5 is false.
    let value = if mean > 0.5 {
        ctx.true_value.clone()
    } else {
        ctx.false_value.clone()
    };
    Ok(finite_pair(value, mean))
}

fn finite_pair(prediction: Value, score: f64) -> Option<(Value, Value)> {
    if score.is_finite() {
        Some((prediction, Value::real(score)))
    } else {
        None
    }
}

fn score_row(
    ctx: &ScoreCtx<'_>,
    method: Method,
    row: &[Value],
    components: &[Component],
) -> Result<[Value; 2], OperatorError> {
    let scored = match method {
        Method::Voting => vote(ctx, row, components)?,
        Method::ConfidenceWeighted => confidence_weighted(ctx, row, components)?,
        Method::RawWeighted => propensity_weighted(ctx, row, components, false)?,
        Method::AdjustedWeighted => propensity_weighted(ctx, row, components, true)?,
        Method::Highest => highest(ctx, row, components)?,
        Method::AverageRaw => average(ctx, row, components, false)?,
        Method::AverageAdjusted => average(ctx, row, components, true)?,
    };
    Ok(match scored {
        Some((prediction, score)) => [prediction, score],
        None => [Value::Null, Value::Null],
    })
}

impl Operator for Ensemble {
    fn arity(&self) -> Arity {
        Arity::Exactly(1)
    }

    fn run(&self, inputs: &[Arc<Table>], _context: &ExecContext) -> Result<Table, OperatorError> {
        let input = match inputs {
            [input] => input,
            _ => return Err(OperatorError::Inputs("Ensemble needs one input".into())),
        };
        let model = input.model();
        let target = model.field(&self.target).ok_or_else(|| {
            OperatorError::Data(format!(
                "Ensemble: target field {:?} is not in the input",
                self.target
            ))
        })?;
        let storage = target.storage;
        let (true_value, false_value) = flag_pair(target)?;
        let components = discover(model, &self.target);
        let usable: Vec<Component> = components
            .iter()
            .filter(|component| accepts(component, self.method))
            .cloned()
            .collect();
        if usable.is_empty() {
            return Err(no_models(&self.target));
        }
        let dropped = drop_mask(model.len(), &components, self.filter_individual);
        let kept: Vec<Field> = model
            .fields()
            .iter()
            .enumerate()
            .filter(|(index, _)| dropped.get(*index).copied() != Some(true))
            .map(|(_, field)| field.clone())
            .collect();
        let kept_names: Vec<String> = kept.iter().map(|field| field.name.clone()).collect();
        let (prediction_name, score_name) =
            free_names(&kept_names, self.method, &self.target, model.len())?;
        let ctx = ScoreCtx {
            model,
            true_value: &true_value,
            false_value: &false_value,
            target: &self.target,
            tie: self.tie,
        };
        let mut rows = Vec::with_capacity(input.row_count());
        for (row_index, row) in input.rows().iter().enumerate() {
            if row.len() != model.len() {
                return Err(OperatorError::Data(format!(
                    "Ensemble: record {row_index} has {} values, expected {}",
                    row.len(),
                    model.len()
                )));
            }
            let scored = score_row(&ctx, self.method, row, &usable)?;
            let mut out = Vec::with_capacity(kept.len().saturating_add(2));
            for (index, cell) in row.iter().enumerate() {
                if dropped.get(index).copied() == Some(true) {
                    continue;
                }
                out.push(cell.clone());
            }
            out.push(scored[0].clone());
            out.push(scored[1].clone());
            rows.push(out);
        }
        let mut fields = kept;
        fields.push(prediction_field(
            prediction_name,
            storage,
            &true_value,
            &false_value,
        ));
        fields.push(continuous_field(score_name));
        let data_model = DataModel::new(fields)
            .map_err(|error| OperatorError::Data(format!("Ensemble: {error}")))?;
        Table::new(data_model, rows)
            .map_err(|error| OperatorError::Data(format!("Ensemble: {error}")))
    }
}
