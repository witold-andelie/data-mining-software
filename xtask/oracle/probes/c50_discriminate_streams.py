# C5.0 discriminating experiments, phase 1 design, Modeler 19 / Jython 2.7.
# Coordinator runs this later. No implementation source or private API is used.
# Public API pattern: train_next_streams.py; IBM Modeler 19 Scripting and
# Automation Guide, C5.0 node properties (pp. 226-227), Var. File / Type properties.
# Predictions and sources: target/c50d/PREDICTIONS.md. No oracle is claimed here.
import json


def experiment_specs():
    # Pure deterministic definitions: also extracted with ast by the offline audit.
    # Columns exclude id; y is last. cat/flag values 0,1 serialize as v0,v1.
    cases = []

    def add(name, names, types, rows, purpose, compare=None, minimum=2, severity=1):
        cases.append({"name": name, "names": names, "types": types, "rows": rows,
                      "purpose": purpose, "compare_to": compare,
                      "min_child": minimum, "severity": severity})

    base = []
    for a, b, p, q in [(0, 1, 4, 0), (1, 0, 15, 5), (1, 1, 1, 15)]:
        for y, count in [("P", p), ("Q", q)]:
            for unused in range(count):
                for z in [0, 1]:
                    base.append([a, b, z, y])
    add("g01_nominal", ["a", "b"], ["cat", "cat"],
        [r[:2] + [r[-1]] for r in base], "Does the average gate apply to nominal tests?")
    add("g02_zero", ["a", "b", "z"], ["cat", "cat", "num"], base,
        "Add a nonconstant zero-gain test, exactly balanced in every profile", "g01_nominal")
    add("g03_constant", ["a", "b", "z"], ["cat", "cat", "num"],
        [r[:2] + [0, r[-1]] for r in base],
        "Change only z to a constant: declared field versus defined ratio denominator", "g02_zero")
    weak = []
    seen = {"P": 0, "Q": 0}
    for r in base:
        y = r[-1]
        i = seen[y]
        z = 0 if i < 10 else (1 if i < (28 if y == "P" else 32) else 2)
        seen[y] += 1
        weak.append(r[:2] + [z, y])
    add("g04_negative", ["a", "b", "z"], ["cat", "cat", "num"], weak,
        "Change only z: positive raw gain below one bit, negative corrected gain", "g02_zero")

    six = [[0, 0, 0, "P"], [1, 0, 2, "P"], [3, 1, 4, "P"],
           [2, 0, 1, "Q"], [4, 1, 3, "Q"], [5, 1, 5, "Q"]]
    add("g05_six_numeric", ["a", "b", "c"], ["num", "num", "num"], six,
        "Six-row gate witness; all candidate tests numeric")
    add("g06_six_flag", ["a", "b", "c"], ["num", "flag", "num"], six,
        "Change only b measurement to Flag; avoid plain-nominal small-N eligibility cutoff", "g05_six_numeric")
    add("g07_six_no_c", ["a", "b"], ["num", "num"],
        [r[:2] + [r[-1]] for r in six],
        "Drop only c: compare local-gain versus denominator switches", "g05_six_numeric")

    penalty_cells = [(0, 0, 0, 0), (0, 1, 0, 1), (1, 0, 2, 1), (1, 1, 3, 0),
                     (2, 0, 2, 0), (2, 1, 3, 2), (20, 0, 1, 3), (20, 1, 1, 1),
                     (21, 0, 2, 0), (21, 1, 2, 3), (22, 0, 1, 2), (22, 1, 0, 0)]

    def expand(cells):
        return [[a, b, y] for a, b, p, q in cells
                for y, count in [("P", p), ("Q", q)] for unused in range(count)]

    for minimum in [2, 4]:
        name = "g08_penalty_m2" if minimum == 2 else "g09_penalty_m4"
        add(name, ["a", "b"], ["num", "num"], expand(penalty_cells),
            "Only min_child changes; best partition fixed, eligible gaps four versus three",
            None if minimum == 2 else "g08_penalty_m2", minimum=minimum)

    # Each fixture targets one raising decision, keeping the gate and CF fixed.
    raising = [
        ("r01_relabel", [(0, 0, 8, 2), (0, 1, 7, 7), (0, 2, 2, 5), (0, 3, 0, 2), (0, 4, 2, 1),
                         (1, 0, 6, 6), (1, 1, 0, 0), (1, 2, 0, 2), (1, 3, 0, 7), (1, 4, 3, 8)],
         "Frozen node scores versus majority re-estimation; raised root changes P to Q"),
        ("r02_regrow", [(0, 0, 2, 0), (0, 1, 1, 5), (0, 2, 5, 2), (0, 3, 3, 8), (0, 4, 1, 4),
                        (1, 0, 4, 4), (1, 1, 0, 1), (1, 2, 1, 5), (1, 3, 3, 0), (1, 4, 3, 3)],
         "Frozen descendants versus descendant regrowth after promotion"),
        ("r04_selection", [(0, 0, 0, 3), (0, 1, 2, 7), (0, 2, 7, 5), (0, 3, 3, 6), (0, 4, 0, 2),
                           (1, 0, 3, 7), (1, 1, 2, 1), (1, 2, 0, 8), (1, 3, 1, 2), (1, 4, 8, 5)],
         "Estimate before versus after regrowth: only the latter selects a raised tree"),
        ("r05_reprune", [(0, 0, 6, 2), (0, 1, 7, 6), (0, 2, 5, 3), (0, 3, 3, 2), (0, 4, 4, 2),
                         (1, 0, 3, 0), (1, 1, 1, 2), (1, 2, 5, 2), (1, 3, 0, 4), (1, 4, 6, 2)],
         "Locally prune redistributed candidate before comparing its estimate"),
        ("r06_unpruned", [(0, 0, 8, 1), (0, 1, 1, 2), (0, 2, 1, 0), (0, 3, 6, 1), (0, 4, 7, 8),
                          (1, 0, 6, 7), (1, 1, 3, 1), (1, 2, 1, 1), (1, 3, 0, 1), (1, 4, 1, 2)],
         "Use original versus locally pruned largest branch as promotion candidate")]
    for name, cells, purpose in raising:
        rows = expand(cells)
        add(name, ["a", "b"], ["cat", "num"], rows, purpose, severity=75)
        if name == "r02_regrow":
            moved = [[r[0], 2.5 if r[0] == 1 and r[1] == 2 else r[1], r[-1]] for r in rows]
            add("r03_move_value", ["a", "b"], ["cat", "num"], moved,
                "Only off-branch b=2 moves to 2.5; regrown cut 2.5 should move to 2.75",
                "r02_regrow", severity=75)
    return cases


import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/c50_discriminate"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)
BUILD_NAMES = ["custom_fields", "target", "inputs", "use_model_name", "model_name", "output_type", "mode", "group_symbolics",
               "use_boost", "boost_num_trials", "use_xval", "xval_num_folds",
               "min_child_records", "pruning_severity", "use_global_pruning",
               "use_winnowing", "use_costs", "use_weight", "weight_field",
               "use_partitioned_data", "calculate_variable_importance"]
manifest = {"label": "c50_discriminate", "saved": [], "errors": [], "cases": [],
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
    schema = [("id", "Integer", "Range", None)]
    for name, typ in zip(case["names"], case["types"]):
        schema.append((name, "Real" if typ == "num" else "String",
                       "Range" if typ == "num" else ("Flag" if typ == "flag" else "Set"),
                       None if typ == "num" else ["v0", "v1"]))
    return schema + [("y", "String", "Set", ["P", "Q"])]


def csv_rows(case, rows, start=1):
    result = []
    for i, row in enumerate(rows):
        result.append([start + i] + [v if typ == "num" else "v%d" % v
                                    for v, typ in zip(row[:-1], case["types"])] + [row[-1]])
    return result


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


def score_rows(case):
    # All observed predictor profiles plus boundary/interior probes. Target P is
    # a placeholder; this source is linked only to a saved scoring nugget.
    profiles = set(tuple(r[:-1]) for r in case["rows"])
    if case["name"].startswith("r"):
        profiles.update((a, b) for a in [0, 1] for b in [-1, 0.5, 1.25, 2.125, 2.5, 2.75, 3.5, 5])
    elif case["name"].startswith(("g08", "g09")):
        profiles.update((a, b) for a in [-1, 1.5, 11, 20.5, 21.5, 23] for b in [0, 1])
    return [list(p) + ["P"] for p in sorted(profiles)]


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
        props = [("type", measure), ("direction", "Target" if field == "y" else
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
             "row_count": len(case["rows"]), "inputs": case["names"], "target": "y"}
    manifest["cases"].append(entry)
    stream = None
    stage = "write_inputs"
    try:
        schema = schema_for(case)
        train_file = case["name"] + "_train.csv"
        grid_file = case["name"] + "_grid.csv"
        write_csv(train_file, schema, csv_rows(case, case["rows"]))
        write_csv(grid_file, schema, csv_rows(case, score_rows(case), 10001))
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
        props = [("custom_fields", True), ("target", "y"), ("inputs", case["names"]),
                 ("use_model_name", True), ("model_name", "c50_discriminate"),
                 ("output_type", "DecisionTree"), ("mode", "Expert"),
                 ("group_symbolics", False), ("use_boost", False), ("use_xval", False),
                 ("min_child_records", case["min_child"]), ("pruning_severity", case["severity"]),
                 ("use_global_pruning", False), ("use_winnowing", False),
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
        grid = typed_source(stream, grid_file, schema, case, 420, entry)
        tables = []
        entry["terminals"] = []
        for source, label, y_pos in [(training, "Train scores", 260), (grid, "Grid scores", 420)]:
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
