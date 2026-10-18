//! Tree-AS, Modeler 19 Algorithms Guide, "Tree AS Algorithm", section 4 (five-bin
//! tiling) and CHAID growth. Oracle `models_as/treeas_*` verifies the lower-closed
//! bin edges. Rounded n*k/5 positions, deduplicated and excluding the minimum,
//! reproduce every observed edge; the general tiling rule remains an inference.
//! This differs from classic CHAID's ten upper-closed bins. No sampling or missing
//! training values are reproduced; callers must reject those paths.

use crate::{AlgoError, Dataset, chaid};
use dms_pmml::{DataType, DerivedField, DiscretizeBin, Expr, OpType, Pmml};
use dms_types::{DataModel, Measurement, Table, Value, ValueSpec};

pub fn train(dataset: &Dataset<'_>, settings: &chaid::ChaidSettings) -> Result<Pmml, AlgoError> {
    if *settings
        != (chaid::ChaidSettings {
            max_depth: settings.max_depth,
            ..chaid::ChaidSettings::default()
        })
        || ![3, 5].contains(&settings.max_depth)
    {
        return Err(AlgoError::Unsupported(
            "GAP: Tree-AS only the default CHAID settings and depth 3 or 5 are verified".into(),
        ));
    }
    if dataset.frequency.is_some()
        || dataset.inputs.len() > 256
        || dataset.table.row_count() >= 1_000_000
        || dataset
            .table
            .row_count()
            .saturating_mul(dataset.table.model().len())
            > 16_000_000
        || settings.max_depth > 32
    {
        return Err(AlgoError::Unsupported(
            "GAP: Tree-AS weights or limits (256 inputs, 1000000 rows, 16000000 cells, depth 32)"
                .into(),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for &position in dataset.inputs {
        if position == dataset.target || !seen.insert(position) {
            return Err(AlgoError::Settings(
                "Tree-AS inputs must be distinct and exclude the target".into(),
            ));
        }
    }
    let mut fields = dataset.table.model().fields().to_vec();
    let mut rows = dataset.table.rows().to_vec();
    let mut transformations = Vec::new();
    for field in &mut fields {
        if let ValueSpec::Flag {
            true_value,
            false_value,
        } = &field.values
        {
            field.values = ValueSpec::Categories(vec![true_value.clone(), false_value.clone()]);
        }
    }
    for &position in std::iter::once(&dataset.target).chain(dataset.inputs) {
        let field = fields
            .get(position)
            .ok_or_else(|| AlgoError::Settings("Tree-AS field out of range".into()))?;
        if rows
            .iter()
            .any(|r| r.get(position).is_none_or(|v| field.is_missing(v)))
        {
            return Err(AlgoError::Unsupported(
                "GAP: Tree-AS missing training values".into(),
            ));
        }
        if field.measurement != Measurement::Continuous {
            if crate::linear::bounded_categories(dataset.table, position, 512)?.len() > 512 {
                return Err(AlgoError::Data("Tree-AS limit: 512 categories".into()));
            }
            continue;
        }
        let mut values = rows
            .iter()
            .map(|r| {
                r.get(position)
                    .and_then(Value::as_f64)
                    .filter(|x| x.is_finite())
                    .ok_or_else(|| AlgoError::Data("Tree-AS needs finite numbers".into()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if position == dataset.target {
            if values.iter().any(|x| x.abs() > 1e150) {
                return Err(AlgoError::Data(
                    "Tree-AS target moments would overflow".into(),
                ));
            }
            continue;
        }
        values.sort_by(f64::total_cmp);
        let mut cuts = Vec::new();
        for k in 1usize..5 {
            // Integer rounding, without converting a potentially malicious float to usize.
            let index = values.len().saturating_mul(k).saturating_add(2) / 5;
            if let Some(&cut) = values.get(index)
                && values.first().is_some_and(|&min| cut > min)
                && cuts.last() != Some(&cut)
            {
                cuts.push(cut);
            }
        }
        let name = format!("{}_BINNED", field.name);
        if fields.iter().any(|f| f.name == name) {
            return Err(AlgoError::Settings(format!(
                "Tree-AS derived field {name} already exists"
            )));
        }
        let bins = (0..=cuts.len())
            .map(|i| DiscretizeBin {
                value: dms_pmml::Value::Number((i + 1) as f64),
                left: i.checked_sub(1).and_then(|j| cuts.get(j)).copied(),
                right: cuts.get(i).copied(),
                left_closed: true,
                right_closed: i == cuts.len(),
            })
            .collect();
        transformations.push(DerivedField {
            name: name.clone(),
            data_type: DataType::Integer,
            optype: OpType::Ordinal,
            expr: Expr::Discretize {
                field: field.name.clone(),
                bins,
                default: None,
                map_missing_to: None,
            },
        });
        for row in &mut rows {
            let value = row
                .get_mut(position)
                .ok_or_else(|| AlgoError::Data("Tree-AS short row".into()))?;
            let number = value
                .as_f64()
                .ok_or_else(|| AlgoError::Data("Tree-AS nonnumeric input".into()))?;
            *value = Value::Integer(
                i64::try_from(cuts.partition_point(|&cut| cut <= number).saturating_add(1))
                    .map_err(|_| AlgoError::Data("Tree-AS bin overflow".into()))?,
            );
        }
        let field = fields
            .get_mut(position)
            .ok_or_else(|| AlgoError::Settings("Tree-AS field out of range".into()))?;
        field.name = name;
        field.measurement = Measurement::Ordinal;
        field.storage = dms_types::Storage::Integer;
        field.values = ValueSpec::Unknown;
    }
    let table = Table::new(
        DataModel::new(fields).map_err(|e| AlgoError::Data(e.to_string()))?,
        rows,
    )
    .map_err(|e| AlgoError::Data(e.to_string()))?;
    let mut pmml = chaid::train_analytic(
        &Dataset {
            table: &table,
            ..*dataset
        },
        settings,
    )?;
    pmml.data_dictionary = crate::data_dictionary(dataset);
    for field in &mut pmml.data_dictionary {
        if let Some(true_value) = &field.flag_true_value
            && let Some(position) = field.values.iter().position(|v| v == true_value)
        {
            field.values.swap(0, position);
        }
    }
    pmml.transformations = transformations;
    Ok(pmml)
}
