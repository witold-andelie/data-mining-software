# C5.0 global-pruning contrasts, phase 6 proposal, Modeler 19 / Jython 2.7.
# Coordinator runs this later. No implementation source or private API is used.
# Public API pattern: c50_prune_streams.py; IBM Modeler 19 Scripting and
# Automation Guide, C5.0 node properties (pp. 226-227), Var. File / Type properties.
# Predictions and sources: target/agy-c50/PREDICTIONS2.md. No oracle is claimed here.
import json


def experiment_specs():
    # Read only the existing witnesses. No new synthetic records or label changes.
    import csv
    import hashlib
    root = "D:/data-mining-software/corpus/"
    wn_schema = [("id", "Integer", "Range", None),
                 ("a", "String", "Set", ["a1", "a2", "a3"]),
                 ("b", "Real", "Range", None),
                 ("c1", "String", "Flag", ["F", "T"]),
                 ("c2", "String", "Flag", ["F", "T"]),
                 ("c3", "String", "Flag", ["F", "T"]),
                 ("c4", "String", "Flag", ["F", "T"]),
                 ("d", "Real", "Range", None), ("y", "String", "Set", ["P", "Q"])]
    auto_schema = [("id", "Integer", "Range", None),
                   ("age", "Real", "Range", None),
                   ("income", "Real", "Range", None),
                   ("region", "String", "Set", ["east", "north", "south", "west"]),
                   ("member", "String", "Flag", ["N", "Y"]),
                   ("visits", "Real", "Range", None),
                   ("buy", "String", "Flag", ["N", "Y"])]

    def read(relative, schema, expected):
        handle = open(root + relative, "rb")
        try:
            raw = handle.read()
        finally:
            handle.close()
        if hashlib.sha256(raw).hexdigest() != expected:
            raise ValueError("witness SHA-256 changed: " + relative)
        lines = list(csv.reader(raw.splitlines()))
        if lines[0] != [s[0] for s in schema]:
            raise ValueError("witness header changed: " + relative)
        rows = []
        for row in lines[1:]:
            if len(row) != len(schema):
                raise ValueError("wrong witness width: " + relative)
            rows.append([float(v) if s[1] == "Real" else (int(v) if s[1] == "Integer" else v)
                         for v, s in zip(row, schema)])
        return rows

    wn100_file = "c50_presets/wn100.csv"
    wn400_file = "c50_presets/wn.csv"
    auto_file = "auto_train/ac_train.csv"
    wn100 = read(wn100_file, wn_schema,
                 "ac9f15bf51f33e66ae426a69cffaacaa80736188bd2081438106404b4e54a5b5")
    wn400 = read(wn400_file, wn_schema,
                 "36b7658443f2a86dd9152593896749db2e5ab9f88a5dcc6c833761ab9cdb39ff")
    auto = read(auto_file, auto_schema,
                "66669959c7247495686d816b032a2433b3a963193adb5c04dbaa0d05626d0731")
    if len(wn100) != 100 or len(wn400) != 400 or len(auto) != 360:
        raise ValueError("witness row count changed")

    specs = [
        ("wn100_sev65", wn_schema, wn100, 100, wn100_file, 2, 65, True,
         "Full wn100 at severity 65 (CF 0.35), global pruning on"),
        ("wn100_sev70", wn_schema, wn100, 100, wn100_file, 2, 70, True,
         "Full wn100 at severity 70 (CF 0.30), global pruning on"),
        ("wn100_sev80", wn_schema, wn100, 100, wn100_file, 2, 80, True,
         "Full wn100 at severity 80 (CF 0.20), global pruning on"),
        ("wn100_sev85", wn_schema, wn100, 100, wn100_file, 2, 85, True,
         "Full wn100 at severity 85 (CF 0.15), global pruning on"),
        ("wn100_sev85_noglobal", wn_schema, wn100, 100, wn100_file, 2, 85, False,
         "Full wn100 at severity 85 (CF 0.15), local-pruning control with global pruning off"),
        ("wn100_mc3", wn_schema, wn100, 100, wn100_file, 3, 75, True,
         "Full wn100 with min_child_records 3 (E0=12, limit=15.25), global pruning on"),
        ("wn100_mc4", wn_schema, wn100, 100, wn100_file, 4, 75, True,
         "Full wn100 with min_child_records 4 (C already a leaf, B untied), global pruning on"),
        ("wn100_mc5", wn_schema, wn100, 100, wn100_file, 5, 75, True,
         "Full wn100 with min_child_records 5 (E0=15, limit=18.57), global pruning on"),
        ("wn51_c3T", wn_schema, [r for r in wn100 if r[5] == "T"], 51, wn100_file, 2, 75, True,
         "Isolate c3=T branch (51 rows containing A and B; E0=3, limit=4.68), global pruning on"),
        ("wn49_c3F", wn_schema, [r for r in wn100 if r[5] == "F"], 49, wn100_file, 2, 75, True,
         "Isolate c3=F branch (49 rows containing C; E0=10, limit=12.82), global pruning on"),
        ("wn25_c1F", wn_schema,
         [r for r in wn100 if r[5] == "T" and r[2] > 2.1 and r[3] == "F"], 25, wn100_file,
         2, 75, True,
         "Isolate c3=T / b>2.1 / c1=F (25 rows containing A and B; E0=2, limit=3.36), global on"),
        ("wn77_noC", wn_schema,
         [r for r in wn100 if not (r[5] == "F" and r[4] == "T")], 77, wn100_file,
         2, 75, True,
         "wn100 without c3=F/c2=T (77 rows; C absent, B untied; E0=7, limit=9.52), global on"),
        ("wn89_noC4F", wn_schema,
         [r for r in wn100 if not (r[5] == "F" and r[4] == "T" and r[6] == "F")], 89, wn100_file,
         2, 75, True,
         "wn100 without c3=F/c2=T/c4=F (89 rows; C is a leaf, B untied; E0=12), global on"),
        ("wn120_prefix", wn_schema, wn400[:120], 120, wn400_file, 2, 75, True,
         "First 120 rows of wn.csv (E0=13, limit=16.40; 3 tied +2 subtrees), global pruning on"),
        ("wn150_prefix", wn_schema, wn400[:150], 150, wn400_file, 2, 75, True,
         "First 150 rows of wn.csv (E0=14, limit=17.56; 3 tied +2 subtrees), global pruning on"),
        ("auto_sev80", auto_schema, auto, 360, auto_file, 2, 80, True,
         "Auto Classifier training data at severity 80 (E0=38, limit=43.83), global pruning on"),
    ]
    cases = []
    for name, schema, rows, expected, source, mc, sev, gp, purpose in specs:
        if len(rows) != expected:
            raise ValueError("subset count changed: " + name)
        cases.append({"name": name, "schema": schema, "names": [s[0] for s in schema[1:-1]],
                      "target": schema[-1][0], "rows": rows, "purpose": purpose,
                      "source_file": source, "source_ids": [r[0] for r in rows],
                      "compare_to": None, "min_child": mc, "severity": sev,
                      "global_pruning": gp})
    return cases


import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/c50_global"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)
BUILD_NAMES = ["custom_fields", "target", "inputs", "use_model_name", "model_name", "output_type", "mode", "group_symbolics",
               "use_boost", "boost_num_trials", "use_xval", "xval_num_folds",
               "min_child_records", "pruning_severity", "use_global_pruning",
               "use_winnowing", "use_costs", "use_weight", "weight_field",
               "use_partitioned_data", "calculate_variable_importance"]
manifest = {"label": "c50_global", "saved": [], "errors": [], "cases": [],
            "inputs": [], "design_version": 1, "oracle_run": True}
taskrunner = None


def message(exc):
    get = getattr(exc, "getMessage", None)
    return unicode(get()) if get is not None else unicode(exc)


def snapshot(value):
    if value is None or isinstance(value, (bool, int, long, float, basestring)):
        return value
    if isinstance(value, (list, tuple, java.util.List)):
        return [snapshot(item) for item in value]
    return unicode(value)


def equal_property(requested, effective):
    # Enumerated spellings returned by Modeler are sometimes upper case.
    # Numeric values, field names, paths and model names retain exact comparison.
    return requested == effective


ENUM_PROPERTIES = set(["mode", "output_type", "decimal_symbol", "quotes_2", "strip_spaces",
                       "create_model_applier_update_links", "encoding"])


def set_properties(node, props, entry):
    entry["requested"] = [[name, snapshot(value)] for name, value in props]
    entry["readback"] = []
    for name, value in props:
        entry["setting_in_progress"] = [name, snapshot(value)]
        node.setPropertyValue(name, value)
        effective = snapshot(node.getPropertyValue(name))
        same = equal_property(value, effective)
        if name in ENUM_PROPERTIES and isinstance(value, basestring) and isinstance(effective, basestring):
            same = value.upper() == effective.upper()
        entry["readback"].append({"property": name, "requested": snapshot(value),
                                  "effective": effective, "matches": same})
        if not same:
            raise ValueError("property changed: %s requested=%s effective=%s" % (name, value, effective))
    entry.pop("setting_in_progress", None)


def read_properties(node, names):
    result = {}
    for name in names:
        try:
            result[name] = snapshot(node.getPropertyValue(name))
        except ERRORS as exc:
            result[name] = {"read_error": message(exc)}
    return result


def write_manifest():
    handle = open(OUT_DIR + "/manifest.json", "w")
    try:
        json.dump(manifest, handle, indent=2, sort_keys=True)
    finally:
        handle.close()


def ensure_dir(path):
    directory = java.io.File(path)
    if directory.exists():
        if not directory.isDirectory():
            raise IOError("not a directory: " + path)
    elif not directory.mkdirs():
        raise IOError("cannot create " + path)


def replace_stream_file(path):
    # Remove only this case's named output: reruns overwrite their own artifacts.
    # No recursive cleanup and no deletion of other corpus labels or unknown files.
    if not path.startswith(DIR + "/") or not path.endswith(".str"):
        raise ValueError("unexpected output path " + path)
    old = java.io.File(path)
    if old.exists() and (not old.isFile() or not old.delete()):
        raise IOError("cannot overwrite " + path)


def schema_for(case):
    return case["schema"]


def csv_rows(case, rows, start=1):
    return rows


def write_csv(name, schema, rows):
    def cell(value):
        if isinstance(value, basestring):
            return '"' + value.replace('"', '""') + '"'
        return "%.17g" % value if isinstance(value, float) else str(value)
    lines = [",".join(field[0] for field in schema)]
    for row in rows:
        if len(row) != len(schema):
            raise ValueError("wrong CSV width: " + name)
        lines.append(",".join(cell(value) for value in row))
    handle = open(DIR + "/" + name, "wb")
    try:
        handle.write((NL.join(lines) + NL).encode("utf-8"))
    finally:
        handle.close()
    manifest["saved"].append(name)
    manifest["inputs"].append({"file": name, "rows": len(rows)})


def instantiate(stream, node):
    table = stream.createAt("table", "Instantiate", 1000, 800)
    stream.link(node, table)
    try:
        table.run([])
    finally:
        stream.delete(table)


def typed_source(stream, name, schema, case, y_pos, entry):
    source = stream.createAt("variablefile", name, 60, y_pos)
    props = [("full_filename", DIR + "/" + name), ("read_field_names", True),
             ("delimit_comma", True), ("delimit_space", False), ("delimit_tab", False),
             ("quotes_2", "PairAndDiscard"), ("strip_spaces", "None"),
             ("decimal_symbol", "Period"), ("encoding", "UTF-8"),
             ("auto_recognize_datetime", False), ("lines_to_scan", 1000)]
    source_entry = {"file": name}
    entry.setdefault("sources", []).append(source_entry)
    set_properties(source, props, source_entry)
    instantiate(stream, source)
    for field, storage, measure, values in schema:
        source.setKeyedPropertyValue("custom_storage_type", field, storage)
    node = stream.createAt("type", "Type " + name, 260, y_pos)
    stream.link(source, node)
    instantiate(stream, node)
    type_entry = {"file": name, "requested": [], "effective": []}
    entry.setdefault("types", []).append(type_entry)
    for field, storage, measure, values in schema:
        props = [("type", measure), ("direction", "Target" if field == case["target"] else
                 ("Input" if field in case["names"] else "None")), ("check", "None")]
        if values is not None:
            props += [("values", values), ("extend_values", False)]
        if storage == "Real":
            props.append(("export_places", 15))
        for prop, value in props:
            node.setKeyedPropertyValue(prop, field, value)
        type_entry["requested"].append({"field": field, "storage": storage, "properties": props})
    instantiate(stream, node)
    for field, storage, measure, values in schema:
        actual = {"field": field}
        for prop in ["storage", "type", "direction", "values", "extend_values"]:
            actual[prop] = snapshot(node.getKeyedPropertyValue(prop, field))
        type_entry["effective"].append(actual)
    return node


def build_case(case):
    entry = {"case": case["name"], "file": case["name"] + ".str", "status": "planned",
             "purpose": case["purpose"], "compare_to": case["compare_to"],
             "row_count": len(case["rows"]), "inputs": case["names"], "target": case["target"],
             "source_file": case["source_file"], "source_ids": case["source_ids"]}
    manifest["cases"].append(entry)
    stream = None
    stage = "write_inputs"
    try:
        schema = schema_for(case)
        train_file = case["name"] + "_train.csv"
        write_csv(train_file, schema, csv_rows(case, case["rows"]))
        stage = "create_stream"
        stream = taskrunner.createStream(case["name"], False, False)
        entry["stream_settings"] = {}
        set_properties(stream, [("decimal_places", 15), ("decimal_symbol", "Period"),
                       ("enable_parallelism", False),
                       ("create_model_applier_for_new_models", False)], entry["stream_settings"])
        stage = "training_type"
        training = typed_source(stream, train_file, schema, case, 100, entry)
        builder = stream.createAt("c50", case["name"], 480, 100)
        stream.link(training, builder)
        stage = "build_properties"
        entry["builder_defaults"] = read_properties(builder, BUILD_NAMES)
        props = [("custom_fields", True), ("target", case["target"]), ("inputs", case["names"]),
                 ("use_model_name", True), ("model_name", "c50_global"),
                 ("output_type", "DecisionTree"), ("mode", "Expert"),
                 ("group_symbolics", False), ("use_boost", False), ("use_xval", False),
                 ("min_child_records", case["min_child"]), ("pruning_severity", case["severity"]),
                 ("use_global_pruning", case["global_pruning"]), ("use_winnowing", False),
                 ("use_costs", False), ("use_weight", False), ("use_partitioned_data", False)]
        entry["builder_settings"] = {}
        set_properties(builder, props, entry["builder_settings"])
        entry["builder_effective"] = read_properties(builder, BUILD_NAMES)
        stage = "train"
        results = []
        builder.run(results)
        models = [r for r in results if isinstance(r, modeler.api.ModelOutput)]
        if len(models) != 1:
            raise RuntimeError("expected one ModelOutput, got " + str(len(models)))
        entry["model_output_count"] = len(models)
        stage = "score_nuggets"
        tables = []
        entry["terminals"] = []
        for source, label, y_pos in [(training, "Train scores", 260)]:
            nugget = stream.createModelApplierAt(models[0], label, 480, y_pos)
            stream.link(source, nugget)
            table = stream.createAt("table", label, 760, y_pos)
            stream.link(nugget, table)
            tables.append(table)
            entry["terminals"].append({"label": label, "applier_type": unicode(nugget.getTypeName()),
                                       "applier_id": unicode(nugget.getID()), "table_id": unicode(table.getID())})
        stage = "save_stream"
        path = DIR + "/" + entry["file"]
        replace_stream_file(path)
        taskrunner.saveStreamToFile(stream, path)
        manifest["saved"].append(entry["file"])
        entry["status"] = "saved"
        stage = "score"
        for table in tables:
            try:
                table.run([])
            except ERRORS as exc:
                entry["status"] = "saved_with_score_errors"
                manifest["errors"].append({"case": case["name"], "stage": stage,
                                           "table": unicode(table.getLabel()), "message": message(exc)})
    except ERRORS as exc:
        entry["status"] = "failed"
        manifest["errors"].append({"case": case["name"], "stage": stage, "message": message(exc)})
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS as exc:
                manifest["errors"].append({"case": case["name"], "stage": "clear", "message": message(exc)})
        write_manifest()


def main():
    global taskrunner
    try:
        ensure_dir(OUT_DIR)
        ensure_dir(DIR)
        cases = experiment_specs()
        manifest["planned_cases"] = [case["name"] for case in cases]
        taskrunner = modeler.script.session().getTaskRunner()
        for case in cases:
            build_case(case)
    except ERRORS as exc:
        manifest["errors"].append({"case": None, "stage": "setup", "message": message(exc)})
    finally:
        write_manifest()


main()
