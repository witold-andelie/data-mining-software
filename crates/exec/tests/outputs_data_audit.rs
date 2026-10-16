//! Oracle test for the Data Audit output node: its two tables (audit statistics, quality) against
//! every Data Audit export of the probe and team streams.

#![cfg(feature = "corpus")]

mod support;

#[test]
fn data_audit_matches_modeler_oracles() {
    support::check_oracle_cases(&["dataaudit"], |_, text| support::parse_csv_document(text));
}
