//! `dms-cli`: inspect IBM SPSS Modeler streams from the command line.

use dms_engine::catalog::{self, Resolution, Scope};
use dms_stream::{Diagram, NodeRef, Stream, StreamDoc, StreamNode};
use dms_strfmt::{ArchiveLimits, AttachmentKind, StreamArchive, read_archive};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage: dms-cli <command>

commands:
  xml <file.str>                  print the decoded stream document as XML
  inspect <file.str>              print nodes, links and attachments as JSON
  coverage [--json] <path>...     decode every .str under the paths and report which catalog
                                  nodes they use
  convert <in.str> <out.dms>      import a stream and atomically save the native document
  run <file.str|file.dms> --out <dir> [--data-dir <dir>]...
                                  run the stream and write the data reaching each terminal node
                                  to <dir>/<uid>.csv, with a report in <dir>/run.json
                                  [--terminal-order <uid,...>] overrides saved terminal run order";

/// Directory recursion bound for `coverage`.
const MAX_DIRECTORY_DEPTH: usize = 32;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let outcome = match args.as_slice() {
        [command, path] if command == "xml" => xml(Path::new(path)),
        [command, path] if command == "inspect" => inspect(Path::new(path)),
        [command, rest @ ..] if command == "coverage" && !rest.is_empty() => coverage(rest),
        [command, input, output] if command == "convert" => {
            convert(Path::new(input), Path::new(output))
        }
        [command, path, rest @ ..] if command == "run" => run(Path::new(path), rest),
        _ => Err(USAGE.to_string()),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("dms-cli: {message}");
            ExitCode::FAILURE
        }
    }
}

fn open(path: &Path) -> Result<(StreamArchive, Stream), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let archive = read_archive(&bytes, &ArchiveLimits::default())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let stream =
        Stream::from_document(&archive.document).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((archive, stream))
}

fn read_document(path: &Path) -> Result<StreamDoc, String> {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("dms") => {
            dms_stream::native::load(path).map_err(|e| format!("{}: {e}", path.display()))
        }
        Some("str") => {
            use std::io::Read;
            let file = std::fs::File::open(path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            const LIMIT: u64 = 1 << 30;
            if file.metadata().map_err(|e| e.to_string())?.len() > LIMIT {
                return Err("input exceeds 1 GiB".into());
            }
            let mut bytes = Vec::new();
            file.take(LIMIT + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or("input file name must be UTF-8")?;
            StreamDoc::import_str(&bytes, name).map_err(|e| format!("{}: {e}", path.display()))
        }
        _ => Err("expected a .str or .dms file".into()),
    }
}
fn convert(input: &Path, output: &Path) -> Result<(), String> {
    if !input
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("str"))
        || !output
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("dms"))
    {
        return Err("usage: dms-cli convert <in.str> <out.dms>".into());
    }
    let doc = read_document(input)?;
    dms_stream::native::save_atomic(&doc, output).map_err(|e| e.to_string())
}

fn xml(path: &Path) -> Result<(), String> {
    let (archive, _) = open(path)?;
    println!("{}", archive.document.to_xml());
    Ok(())
}

fn inspect(path: &Path) -> Result<(), String> {
    let (archive, stream) = open(path)?;
    let attachments: Vec<Value> = archive
        .attachments
        .iter()
        .map(|attachment| {
            json!({
                "member": attachment.name,
                "bytes": attachment.bytes.len(),
                "kind": attachment_kind(&attachment.kind),
            })
        })
        .collect();
    let report = json!({
        "file": path.display().to_string(),
        "document_member": archive.document_member,
        "file_version": stream.file_version,
        "save_version": stream.save_version,
        "diagram": diagram_json(&stream.diagram),
        "attachments": attachments,
    });
    let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    println!("{text}");
    Ok(())
}

fn diagram_json(diagram: &Diagram) -> Value {
    let nodes: Vec<Value> = diagram.nodes.iter().map(node_json).collect();
    let links: Vec<Value> = diagram
        .links
        .iter()
        .map(|link| json!({ "from": node_ref(&link.from), "to": node_ref(&link.to) }))
        .collect();
    let terminals: Vec<Value> = diagram.terminal_order.iter().map(node_ref).collect();
    json!({ "nodes": nodes, "links": links, "terminal_order": terminals })
}

fn node_json(node: &StreamNode) -> Value {
    let (catalog_id, scope) = match catalog::resolve(node) {
        Resolution::Node(spec) => (json!(spec.id), json!(spec.scope.label())),
        Resolution::Structural => (Value::Null, json!("structural")),
        Resolution::Unmapped => (Value::Null, json!("unmapped")),
    };
    let containers: Vec<Value> = node
        .containers
        .iter()
        .map(|container| json!({ "name": container.name, "member": container.member }))
        .collect();
    json!({
        "uid": node.uid,
        "type": node.type_identifier(),
        "catalog_id": catalog_id,
        "scope": scope,
        "model_builder": node.model_builder,
        "name": node.name,
        "label": node.label,
        "position": node.position.map(|(x, y)| json!([x, y])),
        "properties_attachment": node.properties_attachment,
        "containers": containers,
        "subdiagram": node.subdiagram.as_ref().map(diagram_json),
    })
}

fn node_ref(reference: &NodeRef) -> Value {
    match reference {
        NodeRef::Uid(uid) => json!({ "uid": uid }),
        NodeRef::Unresolved(id) => json!({ "unresolved_id": id }),
    }
}

fn attachment_kind(kind: &AttachmentKind) -> Value {
    match kind {
        AttachmentKind::LengthPrefixed {
            declared_length,
            length_matches,
            payload,
        } => json!({
            "length_prefixed": format!("{payload:?}"),
            "declared_length": declared_length,
            "length_matches": length_matches,
        }),
        AttachmentKind::Unrecognized(head) => json!({ "unrecognized": format!("{head:02X?}") }),
        other => json!(format!("{other:?}")),
    }
}

#[derive(Default)]
struct Usage {
    streams: usize,
    instances: usize,
    catalog_id: Option<&'static str>,
    scope: &'static str,
}

fn coverage(args: &[String]) -> Result<(), String> {
    let as_json = args.iter().any(|arg| arg == "--json");
    let mut files = Vec::new();
    for arg in args.iter().filter(|arg| *arg != "--json") {
        collect_streams(Path::new(arg), 0, &mut files)?;
    }
    files.sort();
    let mut failures = Vec::new();
    let mut usage: BTreeMap<String, Usage> = BTreeMap::new();
    let mut decoded = 0usize;
    for file in &files {
        let stream = match open(file) {
            Ok((_, stream)) => stream,
            Err(error) => {
                failures.push(error);
                continue;
            }
        };
        decoded += 1;
        let mut seen_in_stream = std::collections::BTreeSet::new();
        for node in stream.all_nodes() {
            let identifier = catalog::catalog_identifier(node);
            let entry = usage.entry(identifier.clone()).or_default();
            entry.instances += 1;
            match catalog::resolve(node) {
                Resolution::Node(spec) => {
                    entry.catalog_id = Some(spec.id);
                    entry.scope = spec.scope.label();
                }
                Resolution::Structural => entry.scope = "structural",
                Resolution::Unmapped => entry.scope = "unmapped",
            }
            if seen_in_stream.insert(identifier) {
                entry.streams += 1;
            }
        }
    }

    let mut catalog_ids: BTreeMap<&str, &str> = BTreeMap::new();
    for item in usage.values() {
        if let Some(id) = item.catalog_id {
            catalog_ids.insert(id, item.scope);
        }
    }
    let used_in = |scope: Scope| {
        catalog_ids
            .values()
            .filter(|s| **s == scope.label())
            .count()
    };
    let unmapped: Vec<&String> = usage
        .iter()
        .filter(|(_, item)| item.scope == "unmapped")
        .map(|(identifier, _)| identifier)
        .collect();

    if as_json {
        let rows: Vec<Value> = usage
            .iter()
            .map(|(identifier, item)| {
                json!({
                    "identifier": identifier,
                    "catalog_id": item.catalog_id,
                    "scope": item.scope,
                    "streams": item.streams,
                    "instances": item.instances,
                })
            })
            .collect();
        let report = json!({
            "streams": files.len(),
            "decoded": decoded,
            "failures": failures,
            "catalog_nodes_used": { "mvp": used_in(Scope::Mvp), "later": used_in(Scope::Later), "out_of_scope": used_in(Scope::OutOfScope) },
            "unmapped_identifiers": unmapped,
            "identifiers": rows,
        });
        let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
        println!("{text}");
    } else {
        println!(
            "streams: {} found, {} decoded, {} failed",
            files.len(),
            decoded,
            failures.len()
        );
        for failure in &failures {
            println!("  FAILED {failure}");
        }
        println!(
            "catalog nodes used: {} MVP, {} later, {} out of scope; {} unmapped identifiers",
            used_in(Scope::Mvp),
            used_in(Scope::Later),
            used_in(Scope::OutOfScope),
            unmapped.len()
        );
        println!(
            "\n{:<44} {:<22} {:<13} {:>7} {:>9}",
            "identifier", "catalog id", "scope", "streams", "instances"
        );
        let mut rows: Vec<(&String, &Usage)> = usage.iter().collect();
        rows.sort_by(|a, b| b.1.streams.cmp(&a.1.streams).then(a.0.cmp(b.0)));
        for (identifier, item) in rows {
            println!(
                "{:<44} {:<22} {:<13} {:>7} {:>9}",
                identifier,
                item.catalog_id.unwrap_or("-"),
                item.scope,
                item.streams,
                item.instances
            );
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("{} streams failed to decode", failures.len()))
    }
}

fn collect_streams(path: &Path, depth: usize, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if depth > MAX_DIRECTORY_DEPTH {
        return Err(format!("{} is nested too deeply", path.display()));
    }
    if path.is_file() {
        out.push(path.to_path_buf());
        return Ok(());
    }
    let entries =
        std::fs::read_dir(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let child = entry.path();
        if child.is_dir() {
            collect_streams(&child, depth + 1, out)?;
        } else if child
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("str"))
        {
            out.push(child);
        }
    }
    Ok(())
}

/// Runs the stream's top-level diagram and exports the input of every terminal node, which is
/// what `cargo xtask oracle` exports from Modeler (`data/NN_<type>_<uid>.csv`).
fn run(path: &Path, args: &[String]) -> Result<(), String> {
    let mut out = None;
    let mut terminal_order = None;
    let mut data_dirs = Vec::new();
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        let value = rest.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--out" => out = Some(PathBuf::from(value)),
            "--data-dir" => data_dirs.push(PathBuf::from(value)),
            "--terminal-order" => {
                terminal_order = Some(value.split(',').map(str::to_string).collect::<Vec<_>>())
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    let out = out.ok_or("run needs --out <dir>")?;
    let StreamDoc {
        stream,
        attachments,
        ..
    } = read_document(path)?;
    let plan = dms_exec::Plan::new(&stream);
    let terminals = plan.terminals();
    let run_order = match &terminal_order {
        Some(order) => {
            let uids: Vec<&str> = order.iter().map(String::as_str).collect();
            let unique: std::collections::HashSet<&str> = uids.iter().copied().collect();
            if unique.len() != terminals.len()
                || unique.len() != uids.len()
                || terminals.iter().any(|uid| !unique.contains(uid))
            {
                return Err("--terminal-order must list every terminal uid exactly once, separated by commas".into());
            }
            uids
        }
        None => plan.terminals_in_run_order(),
    };
    let mut exported: Vec<&str> = Vec::new();
    for terminal in plan.terminals() {
        for input in plan.inputs_of(terminal) {
            if !exported.contains(&input) {
                exported.push(input);
            }
        }
    }
    let context = dms_exec::ExecContext {
        cancel: Default::default(),
        files: dms_exec::FileResolver { data_dirs },
        stream_properties: stream.properties.clone().map(std::sync::Arc::new),
        attachments: std::sync::Arc::new(attachments.into_iter().collect()),
        decimal_places: None,
        // Export nodes write under the output directory, never over the stream's own paths.
        export_dir: Some(out.join("exports")),
    };
    let mut execution = plan
        .execution()
        .map_err(|e| format!("cannot initialize stream globals: {e:?}"))?;
    let mut runs = Vec::new();
    // Terminals run one by one only when one of them can change the globals later runs and
    // the exports read; otherwise one evaluation computes each node once.
    let sequential = terminal_order.is_some()
        || plan
            .nodes()
            .iter()
            .any(|node| node.catalog_id == Some("set_globals"));
    let run_order = if sequential { run_order } else { Vec::new() };
    for terminal in run_order {
        let result = execution.evaluate(&[terminal], &context);
        runs.push(match result.get(terminal) {
            Some(Ok(_)) => json!({ "uid": terminal, "ok": true }),
            Some(Err(error)) => json!({ "uid": terminal, "ok": false, "error": error.to_string() }),
            None => json!({ "uid": terminal, "ok": false, "error": "not evaluated" }),
        });
    }
    // The oracle exports terminal inputs only after all terminals have run. Re-evaluate
    // readers here, with the final globals, including branches that failed in an earlier run.
    let results = execution.evaluate(&exported, &context);
    std::fs::create_dir_all(&out).map_err(|e| format!("cannot create {}: {e}", out.display()))?;
    let mut report_ok = Vec::new();
    let mut report_failed = Vec::new();
    for uid in &exported {
        match results.get(*uid) {
            Some(Ok(dms_exec::NodeOutput::Data(table))) => {
                let file = out.join(format!("{uid}.csv"));
                let mut writer = std::io::BufWriter::new(
                    std::fs::File::create(&file)
                        .map_err(|e| format!("cannot create {}: {e}", file.display()))?,
                );
                dms_io::csv::write_table(table, &mut writer, context.date_display())
                    .and_then(|()| std::io::Write::flush(&mut writer))
                    .map_err(|e| format!("cannot write {}: {e}", file.display()))?;
                report_ok.push(
                    json!({ "uid": uid, "rows": table.row_count(), "fields": table.model().len() }),
                );
            }
            Some(Ok(_)) => report_failed.push(json!({ "uid": uid, "error": "produces no data" })),
            Some(Err(error)) => {
                report_failed.push(json!({ "uid": uid, "error": error.to_string() }))
            }
            None => report_failed.push(json!({ "uid": uid, "error": "not evaluated" })),
        }
    }
    // Root causes: failures that are not just "an upstream node failed".
    let root_causes: Vec<Value> = results
        .iter()
        .filter_map(|(uid, result)| match result {
            Err(error) if !matches!(error.kind, dms_exec::ErrorKind::Upstream(_)) => {
                Some(json!({ "uid": uid, "error": error.to_string() }))
            }
            _ => None,
        })
        .collect();
    let report = json!({
        "stream": path.display().to_string(),
        "exported": report_ok,
        "failed": report_failed,
        "root_causes": root_causes,
        "runs": runs,
    });
    let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    std::fs::write(out.join("run.json"), text + "\n")
        .map_err(|e| format!("cannot write run.json: {e}"))?;
    println!(
        "dms-cli run: {} exported, {} failed ({} root causes) -> {}",
        report_ok.len(),
        report_failed.len(),
        root_causes.len(),
        out.display()
    );
    Ok(())
}
