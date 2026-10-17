//! Stream global statistics (Modeler 19 User's Guide, Global Functions, table 35).
//! Values are supplied by the execution session, never computed from the reader's rows.

use crate::ClemError;
use dms_types::Value;
use std::collections::BTreeMap;

/// The five statistics supported by Set Globals and the public GlobalValues API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GlobalStatistic {
    Sum,
    Mean,
    Min,
    Max,
    Sdev,
}

impl GlobalStatistic {
    pub const ALL: [Self; 5] = [Self::Sum, Self::Mean, Self::Min, Self::Max, Self::Sdev];

    pub fn function(self) -> &'static str {
        match self {
            Self::Sum => "@GLOBAL_SUM",
            Self::Mean => "@GLOBAL_MEAN",
            Self::Min => "@GLOBAL_MIN",
            Self::Max => "@GLOBAL_MAX",
            Self::Sdev => "@GLOBAL_SDEV",
        }
    }

    pub fn from_function(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|stat| stat.function() == name)
    }

    /// Names in the stream's globalsTable and the node's setGlobalsTable.
    pub fn setting(self) -> &'static str {
        match self {
            Self::Sum => "sum",
            Self::Mean => "ave",
            Self::Min => "min",
            Self::Max => "max",
            Self::Sdev => "sdev",
        }
    }
}

/// One stream execution's values, keyed by field name and statistic.
/// Undefined statistics are absent, and reading one is an error (misc_gl_all_null,
/// misc_gl_singleton). Cloning explicitly creates an independent snapshot.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GlobalValues {
    values: BTreeMap<(String, GlobalStatistic), Value>,
}

impl GlobalValues {
    pub fn get(&self, field: &str, statistic: GlobalStatistic) -> Result<&Value, ClemError> {
        self.values
            .get(&(field.to_string(), statistic))
            .ok_or_else(|| ClemError::UndefinedGlobal {
                function: statistic.function().to_string(),
                field: field.to_string(),
            })
    }

    /// Null removes the statistic. Bound the number of keys before allocating another entry.
    pub fn set(
        &mut self,
        field: &str,
        statistic: GlobalStatistic,
        value: Value,
    ) -> Result<(), ClemError> {
        let key = (field.to_string(), statistic);
        if value.is_null() {
            self.values.remove(&key);
        } else {
            if self.values.len() >= 100_000 && !self.values.contains_key(&key) {
                return Err(ClemError::Unsupported(
                    "more than 100000 global values".into(),
                ));
            }
            self.values.insert(key, value);
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        self.values.clear();
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "unit test assertions"
    )]

    use super::*;
    use crate::{Bindings, Datum, compile, compile_with_globals, function_names, parse};
    use dms_types::{DataModel, Field, Storage};

    #[test]
    fn all_five_functions_read_the_supplied_statistics_not_the_record() {
        let model = DataModel::new(vec![Field::new("x", Storage::Real)]).unwrap();
        let bindings = Bindings {
            model: &model,
            current_field: None,
        };
        let mut globals = GlobalValues::default();
        for (stat, value) in GlobalStatistic::ALL
            .into_iter()
            .zip([10.0, 2.5, 1.0, 4.0, 1.25])
        {
            globals.set("x", stat, Value::Real(value)).unwrap();
            let expr = parse(&format!("{}(x)", stat.function())).unwrap();
            let compiled = compile_with_globals(&expr, &bindings, &globals, false).unwrap();
            assert_eq!(
                compiled.eval(&[Value::Real(999.0)], 1).unwrap(),
                Datum::Value(Value::Real(value))
            );
            assert!(function_names().any(|name| name == stat.function()));
        }
    }

    #[test]
    fn absent_globals_are_errors_at_binding_even_without_records() {
        let model = DataModel::new(vec![Field::new("x", Storage::Real)]).unwrap();
        let bindings = Bindings {
            model: &model,
            current_field: None,
        };
        for stat in GlobalStatistic::ALL {
            let expr = parse(&format!("{}(x)", stat.function())).unwrap();
            assert!(matches!(
                compile(&expr, &bindings),
                Err(ClemError::UndefinedGlobal { .. })
            ));
            assert!(matches!(
                compile_with_globals(&expr, &bindings, &GlobalValues::default(), false),
                Err(ClemError::UndefinedGlobal { .. })
            ));
        }
    }

    #[test]
    fn globals_preserve_storage_and_support_quoted_field_names() {
        let model = DataModel::new(vec![Field::new("some field", Storage::String)]).unwrap();
        let bindings = Bindings {
            model: &model,
            current_field: Some(0),
        };
        for value in [
            Value::Integer(12),
            Value::Real(1.25),
            Value::string(""),
            Value::Date(10),
            Value::Timestamp(1_234_567),
        ] {
            let mut globals = GlobalValues::default();
            globals
                .set("some field", GlobalStatistic::Min, value.clone())
                .unwrap();
            for source in ["@GLOBAL_MIN('some field')", "@GLOBAL_MIN(@FIELD)"] {
                let compiled =
                    compile_with_globals(&parse(source).unwrap(), &bindings, &globals, false)
                        .unwrap();
                assert_eq!(
                    compiled.eval(&[Value::Null], 1).unwrap(),
                    Datum::Value(value.clone())
                );
            }
        }
    }

    #[test]
    fn arguments_must_be_one_field_name() {
        let model = DataModel::new(Vec::new()).unwrap();
        let bindings = Bindings {
            model: &model,
            current_field: None,
        };
        for source in ["@GLOBAL_SUM()", "@GLOBAL_MEAN(x, y)"] {
            assert!(matches!(
                compile(&parse(source).unwrap(), &bindings),
                Err(ClemError::Arity { .. })
            ));
        }
        for source in [
            "@GLOBAL_MIN(1)",
            "@GLOBAL_MAX(x + 1)",
            "@GLOBAL_SDEV(@FIELD)",
        ] {
            assert!(matches!(
                compile(&parse(source).unwrap(), &bindings),
                Err(ClemError::Type(_))
            ));
        }
    }

    #[test]
    fn globals_combine_with_arithmetic_and_condition_binding() {
        let model = DataModel::new(vec![Field::new("x", Storage::Real)]).unwrap();
        let bindings = Bindings {
            model: &model,
            current_field: None,
        };
        let mut globals = GlobalValues::default();
        globals
            .set("x", GlobalStatistic::Mean, Value::Real(2.0))
            .unwrap();
        globals
            .set("x", GlobalStatistic::Sdev, Value::Real(0.5))
            .unwrap();
        let compiled = compile_with_globals(
            &parse("(x - @GLOBAL_MEAN(x)) / @GLOBAL_SDEV(x)").unwrap(),
            &bindings,
            &globals,
            false,
        )
        .unwrap();
        assert_eq!(
            compiled.eval(&[Value::Real(3.0)], 1).unwrap(),
            Datum::Value(Value::Real(2.0))
        );
        let condition = compile_with_globals(
            &parse("x > @GLOBAL_MEAN(x)").unwrap(),
            &bindings,
            &globals,
            true,
        )
        .unwrap();
        assert_eq!(
            condition.eval(&[Value::Real(3.0)], 1).unwrap(),
            Datum::Bool(true)
        );
    }

    #[test]
    fn cloning_and_removal_do_not_share_state() {
        let mut globals = GlobalValues::default();
        globals
            .set("x", GlobalStatistic::Sum, Value::Integer(1))
            .unwrap();
        let mut other = globals.clone();
        other.set("x", GlobalStatistic::Sum, Value::Null).unwrap();
        assert!(other.get("x", GlobalStatistic::Sum).is_err());
        assert_eq!(
            globals.get("x", GlobalStatistic::Sum).unwrap(),
            &Value::Integer(1)
        );
        globals.clear();
        assert!(globals.get("x", GlobalStatistic::Sum).is_err());
    }
}
