#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]
use dms_io::odbc::{select_type, validate_dsn};
use dms_types::Storage;

#[test]
fn access_create_types_and_other_dbms_gap() {
    for (storage, sql) in [
        (Storage::Integer, "CURRENCY"),
        (Storage::String, "VARCHAR(255)"),
        (Storage::Real, "DOUBLE"),
        (Storage::Date, "DATETIME"),
        (Storage::Timestamp, "DATETIME"),
    ] {
        assert_eq!(select_type("ACCESS", storage).unwrap().0, sql);
    }
    assert!(select_type("ACCESS", Storage::Time).is_err());
    assert!(select_type("ACCESS", Storage::List).is_err());
    let other = select_type("Microsoft SQL Server", Storage::Real).unwrap_err();
    assert!(other.0.starts_with("GAP:"), "{other}");
}

#[test]
fn dsn_must_be_a_plain_name() {
    for dsn in ["", "dms_access;PWD=secret", "x\0y", "{driver}"] {
        assert!(validate_dsn(dsn).is_err());
    }
    assert!(validate_dsn("dms_access").is_ok());
}
