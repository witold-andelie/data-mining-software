# Implementing a node

> For contributors and agents. Read `AGENTS.md` first (never-crash, clean room, layering).

## Where things go

| What | Where |
|---|---|
| Node implementation | `crates/exec/src/nodes/<catalog_id>.rs` (catalog ids: `crates/catalog/src/lib.rs`) |
| Registration | one `pub mod <catalog_id>;` line in `crates/exec/src/nodes/mod.rs` and one `("<catalog_id>", nodes::<catalog_id>::build)` entry in `BUILT_IN` in `crates/exec/src/registry.rs`. Parity counts this list, so register a node only when it works and is tested. |
| File formats (readers, writers) | `crates/io` (`dms-io`, L2) — nodes call it, they do not parse file formats themselves |
| Data model | `dms_types`: `Value`, `Field`, `DataModel`, `Table`, `Storage`, `Measurement`, `Role`, `ValueSpec`, `MissingSpec`, `calendar` |
| Expressions | `dms_clem`: `parse(text)`, `compile(&expr, &Bindings { model, current_field })`, `compiled.eval(row, index)` → `Datum` (`Bool` or `Value`) |
| Settings of a node | `node.settings` (the saved `properties` element). Element names and encodings: `docs/formats/nodes/<scripting type>.md` and `docs/formats/node-settings.tsv`. Read them with `dms_exec::settings::{text, flag, integer, real, element, children}`. |
| Stream-wide settings | `context.stream_properties` (date/time formats, decimal symbol, baselines): `docs/formats/stream-properties.md` |
| Data files a node references | `context.files.resolve(path)` (relocates paths from other machines by file name) |

A node module provides:

```rust
pub fn build(node: &StreamNode) -> Result<Box<dyn Operator>, OperatorError>; // parse settings once
impl Operator for X {
    fn arity(&self) -> Arity;                                                     // inputs accepted
    fn run(&self, inputs: &[Arc<Table>], context: &ExecContext) -> Result<Table, OperatorError>;
}
```

Start each module with a doc comment: what Modeler's node does, which settings are implemented, and which are not. A setting that would change results but is not implemented must make `build` return `OperatorError::Settings("<setting> not supported yet")` — never silently ignore it. Settings that only affect display may be ignored (say so in the doc comment).

## Semantics come from evidence

- Modeler's behaviour is decided by the oracle, not by guessing: `cargo xtask compare <label> <path>` runs a corpus stream with `dms-cli run` and compares, node by node, the data reaching each terminal node with what Modeler produced (`oracle/<label>/<stream>/`). `cargo xtask oracle` produces those exports (Windows only, needs Modeler; only one agent drives Modeler at a time).
- CLEM gaps (integer division, boolean derive output, select with null, ...) are being settled in `docs/clem/oracle-results.md`; follow it when present, otherwise note the assumption in the code with `GAP:`.
- Public IBM documentation (scripting guide, user guide) may be read; Modeler's program files must never be opened or decompiled.

## Quality gate for a node

- Unit tests with small synthetic tables in `crates/exec/tests/nodes_<group>.rs` covering every implemented setting, nulls/blanks, empty input, and wrong settings (error, not panic).
- At least one corpus stream where the node's output reaches a terminal node matches Modeler in `cargo xtask compare`, or a written reason why no corpus stream exercises it.
- `cargo xtask ci` passes. Never `unwrap`/`expect`/`panic`/index out of bounds in non-test code; integration test files may start with `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "test helpers fail loudly")]`.
