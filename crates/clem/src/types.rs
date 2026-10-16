//! Static types of CLEM expressions. Modeler types CLEM when it compiles an expression; we
//! only need the types where they change values (`if` branches widen integers to reals,
//! oracle rule 38), reject an expression, or answer a type test (`is_integer` of a null integer
//! field is true).

use dms_types::{Storage, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ty {
    Integer,
    Real,
    String,
    Date,
    Time,
    Timestamp,
    Bool,
    /// `undef`: fits any type.
    Null,
    Unknown,
}

impl Ty {
    pub fn of_value(value: &Value) -> Ty {
        match value {
            Value::Null => Ty::Null,
            Value::Integer(_) => Ty::Integer,
            Value::Real(_) => Ty::Real,
            Value::String(_) => Ty::String,
            Value::Date(_) => Ty::Date,
            Value::Time(_) => Ty::Time,
            Value::Timestamp(_) => Ty::Timestamp,
            Value::List(_) => Ty::Unknown,
        }
    }

    pub fn of_storage(storage: Storage) -> Ty {
        match storage {
            Storage::Integer => Ty::Integer,
            Storage::Real => Ty::Real,
            Storage::String => Ty::String,
            Storage::Date => Ty::Date,
            Storage::Time => Ty::Time,
            Storage::Timestamp => Ty::Timestamp,
            _ => Ty::Unknown,
        }
    }

    /// Whether the type is known well enough to answer a type test.
    pub fn is_known(self) -> bool {
        !matches!(self, Ty::Null | Ty::Unknown)
    }
}

/// The common type of two values that must share one (`if` branches, `min`, list items);
/// `None` when they cannot.
pub fn unify(a: Ty, b: Ty) -> Option<Ty> {
    match (a, b) {
        _ if a == b => Some(a),
        (Ty::Null, other) | (other, Ty::Null) => Some(other),
        (Ty::Unknown, _) | (_, Ty::Unknown) => Some(Ty::Unknown),
        (Ty::Integer, Ty::Real) | (Ty::Real, Ty::Integer) => Some(Ty::Real),
        _ => None,
    }
}
