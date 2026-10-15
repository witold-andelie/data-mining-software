//! An in-memory data set: a data model and its records, stored row by row.
//!
//! Row storage keeps the reference executor simple and obviously correct; a columnar executor
//! can be checked against it.

use crate::field::{DataModel, ModelError};
use crate::value::Value;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Table {
    model: DataModel,
    rows: Vec<Vec<Value>>,
}

impl Table {
    /// Every row must have exactly one value per field.
    pub fn new(model: DataModel, rows: Vec<Vec<Value>>) -> Result<Table, ModelError> {
        if let Some((row, values)) = rows
            .iter()
            .enumerate()
            .find(|(_, values)| values.len() != model.len())
        {
            return Err(ModelError::RowWidth {
                row,
                expected: model.len(),
                found: values.len(),
            });
        }
        Ok(Table { model, rows })
    }

    pub fn model(&self) -> &DataModel {
        &self.model
    }

    pub fn rows(&self) -> &[Vec<Value>] {
        &self.rows
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// The values of one field, in record order.
    pub fn column(&self, name: &str) -> Result<impl Iterator<Item = &Value>, ModelError> {
        let position = self
            .model
            .position(name)
            .ok_or_else(|| ModelError::UnknownField(name.to_string()))?;
        Ok(self.rows.iter().filter_map(move |row| row.get(position)))
    }

    pub fn into_parts(self) -> (DataModel, Vec<Vec<Value>>) {
        (self.model, self.rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::{Field, Storage};

    #[test]
    fn rows_must_match_the_model() {
        let model = DataModel::new(vec![
            Field::new("x", Storage::Integer),
            Field::new("y", Storage::String),
        ])
        .unwrap();
        let table = Table::new(
            model.clone(),
            vec![vec![Value::Integer(1), Value::string("a")]],
        )
        .unwrap();
        assert_eq!(
            table.column("x").unwrap().cloned().collect::<Vec<_>>(),
            vec![Value::Integer(1)]
        );
        assert!(table.column("z").is_err());
        assert_eq!(
            Table::new(model, vec![vec![Value::Integer(1)]]).unwrap_err(),
            ModelError::RowWidth {
                row: 0,
                expected: 2,
                found: 1
            }
        );
    }
}
