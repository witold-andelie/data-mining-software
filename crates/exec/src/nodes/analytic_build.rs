//! Settings observed in `models_as/treeas_*` (Modeler 19).
//! Unverified algorithm settings fail with GAP instead of silently changing training.

use crate::{OperatorError, settings};
use dms_algo::chaid::ChaidSettings;
use dms_stream::StreamNode;

fn gap(message: impl std::fmt::Debug) -> OperatorError {
    OperatorError::Settings(format!("GAP: Analytic Engine training: {message:?}"))
}

fn defaults(
    node: &StreamNode,
    expected: &[(&str, &str)],
    variable: &[&str],
) -> Result<(), OperatorError> {
    let s = node.settings.as_ref();
    if let Some(s) = s {
        for child in s.child_elements() {
            let key = child.name.local.as_ref();
            if key.starts_with('_') && !variable.contains(&key) {
                let Some((_, value)) = expected.iter().find(|(name, _)| *name == key) else {
                    return Err(gap(format!("unknown setting {key}")));
                };
                if child.text() != *value {
                    return Err(gap(format!(
                        "{key}={:?}; supported value {value:?}",
                        child.text()
                    )));
                }
            }
        }
    }
    for key in [
        "expertMode",
        "useWeight",
        "useFrequency",
        "useEvents",
        "calculateRawPropensities",
        "calculateAdjustedPropensities",
    ] {
        if settings::flag(s, key).map_err(gap)?.unwrap_or(false) {
            return Err(gap(format!("{key}=true")));
        }
    }
    for key in [
        "weightField",
        "frequencyField",
        "partitionField",
        "eventFields",
    ] {
        if settings::text(s, key).is_some_and(|v| !v.is_empty()) {
            return Err(gap(key));
        }
    }
    if settings::element(s, "modelSplitFieldSelection")
        .is_some_and(|e| e.child_elements().next().is_some() || !e.text().is_empty())
    {
        return Err(gap("split fields"));
    }
    Ok(())
}

pub(super) fn tree(node: &StreamNode) -> Result<ChaidSettings, OperatorError> {
    defaults(
        node,
        &[
            ("_useWeight", "false"),
            ("_useFrequency", "false"),
            ("_weightField", ""),
            ("_frequencyField", ""),
            ("_method", "chaid"),
            ("_num_bins", "5"),
            ("_record_threshold", "1000000"),
            ("_split_alpha", "0.05"),
            ("_merge_alpha", "0.05"),
            ("_bonferroni_adjustment", "true"),
            ("_effect_size_threshold_cont", "0.04"),
            ("_effect_size_threshold_cat", "0.2"),
            ("_split_merged_categories", "false"),
            ("_grouping_sig_level", "0.05"),
            ("_chi_square", "pearson"),
            ("_minimum_record_use", "use_percentage"),
            ("_min_parent_records_pc", "2"),
            ("_min_child_records_pc", "1"),
            ("_min_parent_records_abs", "100"),
            ("_min_child_records_abs", "50"),
            ("_epsilon", "0.001"),
            ("_max_iterations", "100"),
            ("_use_costs", "false"),
            ("_default_cost_increase", "linear"),
            ("_target", ""),
            ("_inputs", ""),
        ],
        &[
            "_max_depth",
            "_calculate_conf",
            "_display_rule_id",
            "_costs",
        ],
    )?;
    let s = node.settings.as_ref();
    for key in ["_calculate_conf", "_display_rule_id"] {
        settings::flag(s, key).map_err(gap)?;
    }
    if !settings::flag(s, "autoModelFieldSelection")
        .map_err(gap)?
        .unwrap_or(true)
    {
        return Err(gap("Tree-AS custom fields"));
    }
    if settings::element(s, "_costs/rows")
        .and_then(|rows| rows.attribute("count"))
        .is_some_and(|count| count != "0")
        || !settings::children(s, "_costs/rows", "row").is_empty()
    {
        return Err(gap("Tree-AS costs"));
    }
    let depth = settings::integer(s, "_max_depth")
        .map_err(gap)?
        .unwrap_or(5);
    if ![3, 5].contains(&depth) {
        return Err(gap(format!("Tree-AS depth {depth}; only 3 and 5 verified")));
    }
    Ok(ChaidSettings {
        max_depth: usize::try_from(depth).map_err(gap)?,
        ..ChaidSettings::default()
    })
}
