# C5.0 pruning confidence: which z does Modeler use per severity? Modeler 19 / Jython 2.7.
# Synthetic rows, one flag input s, global pruning off: the grown tree is one split whose
# local pruning (leaf versus subtree bound within 0.1) flips at a known z for each count table
# (computed offline with the continuity-corrected Wilson bound). Follow-up of c50_global
# wn100_mc3/mc5, where a 0.0024 margin decided raising.
import json


def experiment_specs():
    nom = [("id", "Integer", "Range", None), ("s", "String", "Flag", ["u", "v"]),
           ("y", "String", "Set", ["P", "Q"])]

    def rows(counts):
        a, b, c, d = counts
        out = []
        for group, (p, q) in enumerate([(a, b), (c, d)]):
            for label, k in [("P", p), ("Q", q)]:
                for _ in range(k):
                    out.append([len(out) + 1, ["u", "v"][group], label])
        return out

    # (severity, counts, z at which the split flips)
    specs = [
        (75, (0, 12, 8, 7), 0.6702), (75, (2, 3, 6, 1), 0.6786), (75, (1, 11, 4, 3), 0.6893),
        (75, (1, 2, 8, 3), 0.7005),
        (70, (1, 3, 9, 8), 0.5204), (70, (5, 6, 3, 2), 0.5240), (70, (4, 6, 3, 2), 0.5359),
        (70, (2, 9, 4, 3), 0.5534),
        (65, (7, 8, 9, 4), 0.3817), (65, (3, 7, 10, 9), 0.3869), (65, (8, 9, 8, 3), 0.3944),
        (65, (3, 9, 8, 7), 0.4066),
        (85, (0, 11, 4, 3), 1.0354), (85, (0, 8, 3, 2), 1.0383), (85, (1, 2, 12, 1), 1.0517),
        (80, (1, 2, 2, 1), 0.8379), (80, (2, 3, 6, 0), 0.8484),
    ]
    cases = []
    for severity, counts, zstar in specs:
        name = "s%d_%s" % (severity, "_".join(str(c) for c in counts))
        data = rows(counts)
        cases.append({"name": name, "schema": nom, "names": ["s"], "target": "y", "rows": data,
                      "purpose": "severity %d, counts %s, flips at z %.4f" % (severity, counts, zstar),
                      "source_file": "synthetic", "source_ids": [r[0] for r in data],
                      "source_sha256": None, "compare_to": None, "min_child": 2,
                      "severity": severity, "global_pruning": False})
    return cases


import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/c50_conf"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)
BUILD_NAMES = ["custom_fields", "target", "inputs", "use_model_name", "model_name", "output_type", "mode", "group_symbolics",
               "use_boost", "boost_num_trials", "use_xval", "xval_num_folds",
               "min_child_records", "pruning_severity", "use_global_pruning",
               "use_winnowing", "use_costs", "use_weight", "weight_field",
               "use_partitioned_data", "calculate_variable_importance"]
manifest = {"label": "c50_conf", "saved": [], "errors": [], "cases": [],
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
             "source_file": case["source_file"], "source_ids": case["source_ids"],
             "source_sha256": case["source_sha256"]}
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
                 ("use_model_name", True), ("model_name", "c50_conf"),
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
