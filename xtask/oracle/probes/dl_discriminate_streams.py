# Decision List policy discrimination, phase 1. Coordinator runs this in Modeler.
# Jython 2.7, public Modeler API only; no packages or random-number generator.
# Source: Modeler 19.0 Python Scripting and Automation Guide, Table 124
# (decisionlistnode, pp.237-238), common modeling properties, and source/Type
# conventions from train_next_streams.py. Algorithm hypotheses: target/dl2/PREDICTIONS.md.
# @OUT_DIR@ is replaced by xtask probe before Jython parses this template.
import hashlib
import itertools
import json
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/dl_discriminate"
MODEL_NAME = "dl_discriminate"
NL = chr(10)
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
BUILD_PROPERTIES = ["search_direction", "target_value", "max_rules", "min_group_size",
                    "min_group_size_pct", "confidence_level", "max_segments_per_rule",
                    "mode", "bin_method", "bin_count", "max_models_per_cycle",
                    "max_rules_per_cycle", "segment_growth", "include_missing",
                    "final_results_only", "reuse_fields", "max_alternatives"]

# BEGIN GENERATED CASES -- frozen cells, not runtime random draws.
CASES = [{'cells': [[['v0', 'v0', 'v0'], 120, 84],
            [['v0', 'v0', 'v1'], 80, 0],
            [['v0', 'v1', 'v0'], 100, 5],
            [['v0', 'v1', 'v1'], 60, 60],
            [['v1', 'v0', 'v0'], 120, 36],
            [['v1', 'v0', 'v1'], 60, 0],
            [['v1', 'v1', 'v0'], 60, 24],
            [['v1', 'v1', 'v1'], 60, 12],
            [['v2', 'v0', 'v0'], 60, 30],
            [['v2', 'v0', 'v1'], 100, 20],
            [['v2', 'v1', 'v0'], 80, 56],
            [['v2', 'v1', 'v1'], 40, 16]],
  'fields': ['a', 'b', 'c'],
  'name': 'dl01_duplicates',
  'params': {'confidence': 0.8,
             'list_depth': 3,
             'list_width': 4,
             'minimum': 1,
             'rule_depth': 3,
             'rule_width': 2},
  'seed': 268},
 {'cells': [[['v0', 'v0', 'v0'], 100, 95],
            [['v0', 'v0', 'v1'], 120, 24],
            [['v0', 'v1', 'v0'], 40, 20],
            [['v0', 'v1', 'v1'], 60, 12],
            [['v1', 'v0', 'v0'], 40, 4],
            [['v1', 'v0', 'v1'], 40, 12],
            [['v1', 'v1', 'v0'], 80, 56],
            [['v1', 'v1', 'v1'], 60, 24]],
  'fields': ['a', 'b', 'c'],
  'name': 'dl02_final_prefix',
  'params': {'confidence': 0.8,
             'list_depth': 4,
             'list_width': 5,
             'minimum': 1,
             'rule_depth': 3,
             'rule_width': 3},
  'seed': 1347},
 {'cells': [[['v0', 'v0', 'v0'], 80, 32],
            [['v0', 'v0', 'v1'], 120, 84],
            [['v0', 'v1', 'v0'], 60, 48],
            [['v0', 'v1', 'v1'], 40, 28],
            [['v1', 'v0', 'v0'], 80, 8],
            [['v1', 'v0', 'v1'], 100, 30],
            [['v1', 'v1', 'v0'], 60, 12],
            [['v1', 'v1', 'v1'], 40, 28]],
  'fields': ['a', 'b', 'c'],
  'name': 'dl03_canonical',
  'params': {'confidence': 0.8,
             'list_depth': 2,
             'list_width': 2,
             'minimum': 40,
             'rule_depth': 2,
             'rule_width': 4},
  'seed': 1965},
 {'cells': [[['v0', 'v0', 'v0'], 40, 8],
            [['v0', 'v0', 'v1'], 40, 4],
            [['v0', 'v1', 'v0'], 120, 120],
            [['v0', 'v1', 'v1'], 80, 0],
            [['v1', 'v0', 'v0'], 100, 95],
            [['v1', 'v0', 'v1'], 60, 57],
            [['v1', 'v1', 'v0'], 40, 24],
            [['v1', 'v1', 'v1'], 80, 48],
            [['v2', 'v0', 'v0'], 100, 20],
            [['v2', 'v0', 'v1'], 60, 54],
            [['v2', 'v1', 'v0'], 40, 12],
            [['v2', 'v1', 'v1'], 100, 40]],
  'fields': ['a', 'b', 'c'],
  'name': 'dl04_coverage',
  'params': {'confidence': 0.8,
             'list_depth': 4,
             'list_width': 3,
             'minimum': 80,
             'rule_depth': 3,
             'rule_width': 5},
  'seed': 200},
 {'cells': [[['v0', 'v0', 'v0'], 60, 42],
            [['v0', 'v0', 'v1'], 40, 36],
            [['v0', 'v1', 'v0'], 100, 0],
            [['v0', 'v1', 'v1'], 100, 0],
            [['v1', 'v0', 'v0'], 80, 76],
            [['v1', 'v0', 'v1'], 100, 10],
            [['v1', 'v1', 'v0'], 80, 40],
            [['v1', 'v1', 'v1'], 100, 80]],
  'fields': ['a', 'b', 'c'],
  'name': 'dl05_carry',
  'params': {'confidence': 0.95,
             'list_depth': 4,
             'list_width': 4,
             'minimum': 1,
             'rule_depth': 2,
             'rule_width': 4},
  'seed': 2458},
 {'cells': [[['v0', 'v0', 'v0'], 40, 40],
            [['v0', 'v0', 'v1'], 60, 3],
            [['v0', 'v1', 'v0'], 120, 60],
            [['v0', 'v1', 'v1'], 60, 30],
            [['v1', 'v0', 'v0'], 80, 64],
            [['v1', 'v0', 'v1'], 100, 100],
            [['v1', 'v1', 'v0'], 120, 24],
            [['v1', 'v1', 'v1'], 120, 0]],
  'fields': ['a', 'b', 'c'],
  'name': 'dl06_preprune',
  'params': {'confidence': 0.95,
             'list_depth': 4,
             'list_width': 2,
             'minimum': 1,
             'rule_depth': 2,
             'rule_width': 3},
  'seed': 1001},
 {'cells': [[['v0', 'v0', 'v0'], 60, 24],
            [['v0', 'v0', 'v1'], 60, 30],
            [['v0', 'v1', 'v0'], 80, 48],
            [['v0', 'v1', 'v1'], 100, 10],
            [['v1', 'v0', 'v0'], 100, 80],
            [['v1', 'v0', 'v1'], 80, 76],
            [['v1', 'v1', 'v0'], 80, 76],
            [['v1', 'v1', 'v1'], 40, 36]],
  'fields': ['a', 'b', 'c'],
  'name': 'dl07_interaction',
  'params': {'confidence': 0.8,
             'list_depth': 3,
             'list_width': 2,
             'minimum': 80,
             'rule_depth': 2,
             'rule_width': 3},
  'seed': 3209},
 {'cells': [[['v0', 'v0', 'v0'], 120, 12],
            [['v0', 'v0', 'v1'], 60, 18],
            [['v0', 'v1', 'v0'], 120, 48],
            [['v0', 'v1', 'v1'], 40, 28],
            [['v1', 'v0', 'v0'], 60, 0],
            [['v1', 'v0', 'v1'], 80, 8],
            [['v1', 'v1', 'v0'], 80, 16],
            [['v1', 'v1', 'v1'], 40, 28]],
  'fields': ['a', 'b', 'c'],
  'name': 'dl08_completed',
  'params': {'confidence': 0.8,
             'list_depth': 5,
             'list_width': 8,
             'minimum': 80,
             'rule_depth': 2,
             'rule_width': 5},
  'seed': 3305},
 {'cells': [[['v0', 'v0', 'v0'], 100, 0],
            [['v0', 'v0', 'v1'], 120, 0],
            [['v0', 'v1', 'v0'], 60, 12],
            [['v0', 'v1', 'v1'], 40, 28],
            [['v1', 'v0', 'v0'], 40, 4],
            [['v1', 'v0', 'v1'], 120, 120],
            [['v1', 'v1', 'v0'], 100, 0],
            [['v1', 'v1', 'v1'], 120, 72]],
  'fields': ['a', 'b', 'c'],
  'name': 'dl09_prefix_canonical',
  'params': {'confidence': 0.95,
             'list_depth': 4,
             'list_width': 3,
             'minimum': 1,
             'rule_depth': 3,
             'rule_width': 3},
  'seed': 3287},
 {'cells': [[['v1', 'v0', 'v0'], 60, 60],
            [['v0', 'v1', 'v0'], 80, 80],
            [['v0', 'v0', 'v1'], 100, 100],
            [['v0', 'v0', 'v0'], 400, 0]],
  'fields': ['z', 'a', 'm'],
  'name': 'dl10_tie_sizes',
  'params': {'confidence': 0.95,
             'list_depth': 2,
             'list_width': 6,
             'minimum': 1,
             'rule_depth': 1,
             'rule_width': 3}},
 {'cells': [[['v1', 'v0', 'v0'], 80, 80],
            [['v0', 'v1', 'v0'], 80, 80],
            [['v0', 'v0', 'v1'], 80, 80],
            [['v0', 'v0', 'v0'], 400, 0]],
  'fields': ['z', 'a', 'm'],
  'name': 'dl11_tie_equal',
  'params': {'confidence': 0.95,
             'list_depth': 2,
             'list_width': 6,
             'minimum': 1,
             'rule_depth': 1,
             'rule_width': 3}},
 {'cells': [[['v0', 'v0', 'v1'], 80, 80],
            [['v0', 'v1', 'v0'], 80, 80],
            [['v1', 'v0', 'v0'], 80, 80],
            [['v0', 'v0', 'v0'], 400, 0]],
  'fields': ['m', 'a', 'z'],
  'name': 'dl12_tie_reorder',
  'params': {'confidence': 0.95,
             'list_depth': 2,
             'list_width': 6,
             'minimum': 1,
             'rule_depth': 1,
             'rule_width': 3}}]
# END GENERATED CASES

manifest = {"label": "dl_discriminate", "saved": [], "errors": [], "inputs": [],
            "cases": CASES, "guide": "ModelerScriptingAutomation-19.0.txt Table 124",
            "design": {"response": "Y", "search_direction": "Up", "expert": True,
                       "profile_format": "[predictor values, total records, Y records]",
                       "row_order": "cells in literal order, Y copies then N copies",
                       "roles": "id None; listed predictors Input; respond Target",
                       "min_percent": 5.0, "segment_growth": 2.0,
                       "bins": "EqualCount / 10 (all predictors categorical)",
                       "prediction_source": "target/dl2/PREDICTIONS.md"}}
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


def write_manifest():
    handle = open(OUT_DIR + "/manifest.json", "w")
    try:
        json.dump(manifest, handle, indent=2, sort_keys=True)
    finally:
        handle.close()


def prepare_directory():
    # Preserve existing shared data. A second run requires coordinator cleanup.
    directory = java.io.File(DIR)
    if not directory.exists() and not directory.mkdirs():
        raise IOError("cannot create " + DIR)
    for entry in CASES:
        for suffix in [".csv", "_grid.csv", ".str"]:
            path = DIR + "/" + entry["name"] + suffix
            if java.io.File(path).exists():
                raise IOError("refusing to overwrite existing artifact " + path)


def schema_for(entry):
    schema = [("id", "Integer", "Range", None)]
    for index, field in enumerate(entry["fields"]):
        values = sorted(set(cell[0][index] for cell in entry["cells"]))
        schema.append((field, "String", "Set", values))
    schema.append(("respond", "String", "Flag", ["N", "Y"]))
    return schema


def csv_cell(value):
    if isinstance(value, basestring):
        return '"' + value.replace('"', '""') + '"'
    return str(value)


def write_csv(name, schema, rows, kind):
    lines = [",".join(field[0] for field in schema)]
    positives = 0
    for row in rows:
        if len(row) != len(schema):
            raise ValueError("wrong CSV width in " + name)
        lines.append(",".join(csv_cell(value) for value in row))
        if row[-1] == "Y":
            positives += 1
    data = (NL.join(lines) + NL).encode("utf-8")
    handle = open(DIR + "/" + name, "wb")
    try:
        handle.write(data)
    finally:
        handle.close()
    manifest["saved"].append(name)
    manifest["inputs"].append({"file": name, "rows": len(rows), "Y": positives,
                               "kind": kind, "sha256": hashlib.sha256(data).hexdigest()})


def write_inputs(entry):
    schema = schema_for(entry)
    rows = []
    for values, count, positives in entry["cells"]:
        if count <= 0 or positives < 0 or positives > count:
            raise ValueError("invalid profile count")
        for copy in range(count):
            rows.append([len(rows) + 1] + list(values) + ["Y" if copy < positives else "N"])
    write_csv(entry["name"] + ".csv", schema, rows, "training")
    domains = [field[3] for field in schema[1:-1]]
    grid = []
    for values in itertools.product(*domains):
        grid.append([10001 + len(grid)] + list(values) + ["N"])
    write_csv(entry["name"] + "_grid.csv", schema, grid, "full predictor grid; target placeholder N")


def set_properties(node, props, entry):
    for name, value in props:
        entry["setting_in_progress"] = [name, snapshot(value)]
        node.setPropertyValue(name, value)
        entry.setdefault("readback", []).append([name, snapshot(node.getPropertyValue(name))])
    entry.pop("setting_in_progress", None)


def read_properties(node):
    result = {}
    for name in BUILD_PROPERTIES:
        try:
            result[name] = snapshot(node.getPropertyValue(name))
        except ERRORS as exc:
            result[name] = {"read_error": message(exc)}
            manifest["errors"].append({"stage": "read_property", "property": name,
                                       "message": message(exc)})
    return result


def instantiate_node(stream, node):
    table = stream.createAt("table", "Instantiate", 1000, 800)
    stream.link(node, table)
    try:
        table.run([])
    finally:
        stream.delete(table)


def typed_source(stream, entry, filename, y_position):
    schema = schema_for(entry)
    source = stream.createAt("variablefile", filename, 60, y_position)
    info = {"file": filename, "fields": []}
    entry.setdefault("sources", []).append(info)
    set_properties(source, [("full_filename", DIR + "/" + filename),
                           ("read_field_names", True), ("delimit_comma", True),
                           ("delimit_space", False), ("delimit_tab", False),
                           ("quotes_2", "PairAndDiscard"), ("strip_spaces", "None"),
                           ("decimal_symbol", "Period"), ("encoding", "UTF-8"),
                           ("auto_recognize_datetime", False), ("lines_to_scan", 1000)], info)
    instantiate_node(stream, source)
    for field, storage, measure, values in schema:
        source.setKeyedPropertyValue("custom_storage_type", field, storage)
    node = stream.createAt("type", "Type " + filename, 260, y_position)
    stream.link(source, node)
    instantiate_node(stream, node)
    for field, storage, measure, values in schema:
        node.setKeyedPropertyValue("type", field, measure)
        if values is not None:
            node.setKeyedPropertyValue("values", field, values)
            node.setKeyedPropertyValue("extend_values", field, False)
        direction = "None"
        if field == "respond":
            direction = "Target"
        elif field in entry["fields"]:
            direction = "Input"
        node.setKeyedPropertyValue("direction", field, direction)
        node.setKeyedPropertyValue("check", field, "None")
    instantiate_node(stream, node)
    for field, storage, measure, values in schema:
        actual = {"name": field, "storage_requested": storage}
        for prop in ["storage", "type", "direction", "values"]:
            actual[prop] = snapshot(node.getKeyedPropertyValue(prop, field))
        info["fields"].append(actual)
    return node


def appliers(stream):
    found = []
    iterator = stream.iterator()
    while iterator.hasNext():
        node = iterator.next()
        if unicode(node.getTypeName()).startswith("apply"):
            found.append(node)
    return found


def build_case(entry):
    stream = None
    stage = "write_inputs"
    entry["status"] = "planned"
    try:
        write_inputs(entry)
        stage = "create_stream"
        stream = taskrunner.createStream(entry["name"], False, False)
        set_properties(stream, [("decimal_places", 15), ("decimal_symbol", "Period"),
                                ("enable_parallelism", False),
                                ("create_model_applier_for_new_models", True),
                                ("create_model_applier_update_links", "createEnabled")], entry)
        stage = "training_type"
        training = typed_source(stream, entry, entry["name"] + ".csv", 100)
        builder = stream.createAt("decisionlist", entry["name"], 480, 100)
        stream.link(training, builder)
        entry["builder_defaults"] = read_properties(builder)
        p = entry["params"]
        props = [("use_model_name", True), ("model_name", MODEL_NAME),
                 ("use_partitioned_data", False), ("mode", "Expert"),
                 ("search_direction", "Up"), ("target_value", "Y"),
                 ("max_rules", p["list_depth"]), ("min_group_size", p["minimum"]),
                 ("min_group_size_pct", 5.0), ("confidence_level", p["confidence"] * 100.0),
                 ("max_models_per_cycle", p["list_width"]),
                 ("max_rules_per_cycle", p["rule_width"]),
                 ("max_segments_per_rule", p["rule_depth"]),
                 ("bin_method", "EqualCount"), ("bin_count", 10),
                 ("segment_growth", 2.0), ("include_missing", False), ("reuse_fields", True)]
        entry["properties"] = props
        stage = "build_properties"
        set_properties(builder, props, entry)
        entry["builder_effective"] = read_properties(builder)
        stage = "train"
        results = []
        builder.run(results)
        models = [result for result in results if isinstance(result, modeler.api.ModelOutput)]
        if len(models) != 1:
            raise RuntimeError("expected one ModelOutput, got " + str(len(models)))
        placed = appliers(stream)
        if len(placed) > 1:
            raise RuntimeError("unexpected multiple automatically placed nuggets")
        entry["auto_placed_applier"] = bool(placed)
        if placed:
            nugget = placed[0]
            nugget.setXYPosition(480, 260)
        else:
            nugget = stream.createModelApplierAt(models[0], "Train scores", 480, 260)
            stream.link(training, nugget)
        table = stream.createAt("table", "Train scores", 760, 260)
        stream.link(nugget, table)
        entry["nugget_type"] = unicode(nugget.getTypeName())
        entry["nugget_id"] = unicode(nugget.getID())
        entry["terminals"] = [unicode(table.getID())]
        stage = "scoring_type"
        grid = typed_source(stream, entry, entry["name"] + "_grid.csv", 420)
        grid_nugget = stream.createModelApplierAt(models[0], "Grid scores", 480, 420)
        stream.link(grid, grid_nugget)
        grid_table = stream.createAt("table", "Grid scores", 760, 420)
        stream.link(grid_nugget, grid_table)
        entry["terminals"].append(unicode(grid_table.getID()))
        stage = "save_stream"
        taskrunner.saveStreamToFile(stream, DIR + "/" + entry["name"] + ".str")
        manifest["saved"].append(entry["name"] + ".str")
        entry["status"] = "saved"
        stage = "score"
        for terminal in [table, grid_table]:
            terminal.run([])
    except ERRORS as exc:
        if entry["status"] == "saved":
            entry["status"] = "saved_with_score_errors"
        else:
            entry["status"] = "failed"
        manifest["errors"].append({"case": entry["name"], "stage": stage, "message": message(exc)})
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS as exc:
                manifest["errors"].append({"case": entry["name"], "stage": "clear", "message": message(exc)})
        write_manifest()


try:
    taskrunner = modeler.script.session().getTaskRunner()
    prepare_directory()
    write_manifest()
    for entry in CASES:
        build_case(entry)
except ERRORS as exc:
    manifest["errors"].append({"case": None, "stage": "setup", "message": message(exc)})
finally:
    write_manifest()
