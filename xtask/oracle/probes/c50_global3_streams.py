# C5.0 global3: hidden split representation, partition size and suppressed growth.
# Modeler 19 / Jython 2.7. Designed offline; coordinator runs this later.
# Twelve matched global-on/off pairs, all derived from the SHA-256 checked wn11 B witness.
# Same 11 records (c3=T / b>2.1 / c1=F / c2=F of c50_presets/wn100.csv); only the inputs change.
# Separates the number of inputs from the deeper grown structure that local pruning removes
# (c50_twig: a lone [1,5]/[4,1] split on one input is kept; c50_global2/wn11_B_global collapses).
import json


def experiment_specs():
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
    relative = "c50_presets/wn100.csv"
    expected = "ac9f15bf51f33e66ae426a69cffaacaa80736188bd2081438106404b4e54a5b5"
    handle = open(root + relative, "rb")
    try:
        raw = handle.read()
    finally:
        handle.close()
    if hashlib.sha256(raw).hexdigest() != expected:
        raise ValueError("witness SHA-256 changed: " + relative)
    # Python 3 offline validation and Jython 2.7 both accept these text lines.
    lines = list(csv.reader(raw.decode("utf-8").splitlines()))
    if lines[0] != [s[0] for s in wn_schema]:
        raise ValueError("witness header changed: " + relative)
    wn = []
    for row in lines[1:]:
        wn.append([float(v) if s[1] == "Real" else (int(v) if s[1] == "Integer" else v)
                   for v, s in zip(row, wn_schema)])
    b11 = [r for r in wn if r[5] == "T" and r[2] > 2.1 and r[3] == "F" and r[4] == "F"]
    if len(b11) != 11:
        raise ValueError("subset count changed")
    # h and z encode the same binary partition in flag and numeric form.
    # T maps to 0 so numeric <=0.5 and flag T visit the same branch first.
    # The size-two partition is exactly b<=3.6 inside the left local leaf.
    augmented = wn_schema + [("h", "String", "Flag", ["F", "T"]),
                              ("z", "Real", "Range", None)]
    variants = [
        ("base", ["b"], 3, "original", 2),
        ("flag3", ["b", "h"], 3, "original", 2),
        ("numeric3", ["b", "z"], 3, "original", 2),
        ("flag2", ["b", "h"], 2, "original", 2),
        ("numeric2", ["b", "z"], 2, "original", 2),
        ("base_mc3", ["b"], 3, "original", 3),
        ("flag3_mc3", ["b", "h"], 3, "original", 3),
        ("numeric3_mc3", ["b", "z"], 3, "original", 3),
        ("plateau_base", ["b"], 3, "plateau", 2),
        ("plateau_flag3", ["b", "h"], 3, "plateau", 2),
        ("duplicate3", ["b", "h", "z"], 3, "original", 2),
        ("rightflat_flag3", ["b", "h"], 3, "rightflat", 2),
    ]
    cases = []
    for name, inputs, size, transform, mc in variants:
        rows = []
        for original in b11:
            r = list(original)
            flag = r[6] == "T" if size == 3 else r[2] <= 3.6
            if transform == "plateau":
                r[2] = 7.2 if original[2] <= 7.3 else 7.4
            elif transform == "rightflat" and original[2] > 7.3:
                r[2] = 7.4
            rows.append(r + ["T" if flag else "F", 0.0 if flag else 1.0])
        for gp in [False, True]:
            case_name = name + ("_on" if gp else "_off")
            cases.append({"name": case_name, "schema": augmented, "names": inputs,
                          "target": "y", "rows": rows, "min_child": mc, "severity": 75,
                          "global_pruning": gp, "source_file": relative,
                          "source_ids": [r[0] for r in b11], "source_sha256": expected,
                          "compare_to": name + "_off" if gp else None,
                          "purpose": "hidden left partition size=%d; b=%s; inputs=%s; min_child=%d" %
                                     (size, transform, inputs, mc)})
    if len(cases) != 24:
        raise ValueError("global3 case budget changed")
    return cases


import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/c50_global3"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)
BUILD_NAMES = ["custom_fields", "target", "inputs", "use_model_name", "model_name", "output_type", "mode", "group_symbolics",
               "use_boost", "boost_num_trials", "use_xval", "xval_num_folds",
               "min_child_records", "pruning_severity", "use_global_pruning",
               "use_winnowing", "use_costs", "use_weight", "weight_field",
               "use_partitioned_data", "calculate_variable_importance"]
manifest = {"label": "c50_global3", "saved": [], "errors": [], "cases": [],
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
                 ("use_model_name", True), ("model_name", "c50_global3"),
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
