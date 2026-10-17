# Probe: C5.0 global pruning on synthetic exception blocks (c50_boost_streams copy).
# Coordinator only: cargo xtask probe c50_globalprune_streams (Modeler 19, Jython 2.7).
# Writes generated inputs and one stream per case to corpus/c50_globalprune; public API only.
# Each group g has m records with z = F (r of them the minority class) and c records with
# z = T (q of them the minority class): splitting the group on z removes q + r - (c - q) -
# r errors; the blocks span local error-bound margins from 0.01 to 3.5.
import json
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/c50_globalprune"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
SEED = 12345
NL = chr(10)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "c50_globalprune", "saved": [], "errors": [], "cases": [], "inputs": []}

# name, storage, measurement, explicit categorical domain (false before true for flags).
TREE_SCHEMA = [
    ("id", "Integer", "Range", None),
    ("x", "Real", "Range", None),
    ("cat", "String", "Set", ["amber", "blue", "green", "red"]),
    ("flag", "String", "Flag", ["F", "T"]),
    ("ord", "String", "OrderedSet", ["low", "middle", "high"]),
    ("code", "Integer", "Set", [1, 2, 3]),
    ("constant", "Real", "Range", None),
    ("noise", "String", "Set", ["u", "v", "w"]),
    ("y", "String", "Set", ["A", "B", "C"]),
    ("binary", "String", "Flag", ["N", "Y"]),
    ("part", "Integer", "Set", [1, 2, 3]),
    ("weight", "Real", "Range", None),
]
GP_SCHEMA = [
    ("id", "Integer", "Range", None),
    ("g", "String", "Set", ["g%02d" % i for i in range(1, 41)]),
    ("z", "String", "Flag", ["F", "T"]),
    ("y", "String", "Set", ["P", "Q"]),
    ("weight", "Real", "Range", None),
]
GP_INPUTS = ["g", "z"]
VALUE_COUNTS = [4, 8, 16, 32]
RATIOS = [1.5, 2, 2.5, 3, 3.5, 4, 5, 6]
SUB_NS = [56, 58, 60, 62, 64, 66, 68, 70]
SUB_SCHEMA = [("id", "Integer", "Range", None), ("g", "String", "Set", ["g%02d" % i for i in range(1, 17)]), ("z", "String", "Flag", ["F", "T"]), ("y", "String", "Set", ["P", "Q"]), ("weight", "Real", "Range", None), ("h", "String", "Flag", ["F", "T"])]
FINE = {8: [26, 28, 29, 30, 31, 32], 16: [56, 58, 60, 62, 64], 32: [96, 100, 104, 108, 112], 64: [192, 208, 224, 240, 256]}


def schema_values(v):
    return [GP_SCHEMA[0], ("g", "String", "Set", ["g%02d" % i for i in range(1, v + 1)])] + GP_SCHEMA[2:]
# The same fields with g declaring only the groups the copies data uses.
GP_SMALL = [GP_SCHEMA[0], ("g", "String", "Set", ["g01", "g02", "g20", "g21"])] + GP_SCHEMA[2:]
# (m, c, q, r) per block, by increasing local margin (CF 0.25).
GRID = [(11, 7, 4, 2), (6, 5, 3, 0), (8, 3, 2, 0), (32, 3, 2, 0), (9, 4, 3, 3), (8, 6, 4, 1),
        (32, 4, 3, 2), (39, 2, 2, 0), (12, 7, 5, 3), (6, 7, 5, 1), (16, 5, 4, 1), (39, 3, 3, 0),
        (13, 8, 6, 3), (5, 6, 5, 0), (24, 4, 4, 0), (40, 6, 5, 0)]
TREE_INPUTS = ["x", "cat", "flag", "ord", "code", "constant", "noise"]
BN_SCHEMA = [
    ("id", "Integer", "Range", None),
    ("a", "String", "Flag", ["a0", "a1"]),
    ("b", "String", "Flag", ["b0", "b1"]),
    ("c", "String", "Flag", ["c0", "c1"]),
    ("noise", "String", "Set", ["u", "v", "w"]),
    ("x", "Real", "Range", None),
    ("y", "String", "Flag", ["N", "Y"]),
    ("ynum", "Integer", "Set", [1, 2]),
    ("part", "Integer", "Set", [1, 2, 3]),
]
BN_INPUTS = ["a", "b", "c", "noise"]
NN_SCHEMA = [
    ("id", "Integer", "Range", None),
    ("x", "Real", "Range", None),
    ("z", "Real", "Range", None),
    ("cat", "String", "Set", ["blue", "green", "red"]),
    ("flag", "Integer", "Flag", [0, 1]),
    ("y", "String", "Set", ["A", "B", "C"]),
    ("r", "Real", "Range", None),
    ("binary", "Integer", "Flag", [0, 1]),
    ("part", "Integer", "Set", [1, 2, 3]),
]
NN_INPUTS = ["x", "z", "cat", "flag"]

BUILD_PROPERTIES = {
    "c50": ["output_type", "group_symbolics", "use_boost", "boost_num_trials",
            "use_xval", "xval_num_folds", "mode", "favor", "expected_noise",
            "min_child_records", "pruning_severity", "use_global_pruning", "use_winnowing",
            "use_costs", "costs", "use_weight", "weight_field", "use_partitioned_data",
            "calculate_variable_importance", "calculate_raw_propensities",
            "calculate_adjusted_propensities", "adjusted_propensity_partition"],
    "bayesnet": ["structure_type", "parameter_learning_method", "mode", "missing_values",
                 "all_probabilities", "independence", "significance_level",
                 "maximal_conditioning_set", "use_feature_selection", "inputs_always_selected",
                 "maximum_number_inputs", "continue_training_existing_model",
                 "use_partitioned_data", "calculate_variable_importance",
                 "calculate_raw_propensities", "calculate_adjusted_propensities",
                 "adjusted_propensity_partition"],
    "neuralnetwork": ["objective", "method", "use_custom_layers", "first_layer_units",
                      "second_layer_units", "use_max_time", "max_time", "use_max_cycles",
                      "max_cycles", "use_min_accuracy", "min_accuracy", "overfit_prevention_pct",
                      "use_random_seed", "random_seed", "missing_values", "use_partitioned_data",
                      "continue", "component_models_n", "combining_rule_categorical",
                      "combining_rule_continuous", "confidence", "score_category_probabilities",
                      "max_categories", "score_propensity"],
}
APPLY_PROPERTIES = {
    "c50": ["calculate_raw_propensities", "calculate_adjusted_propensities"],
    "bayesnet": ["all_probabilities", "calculate_raw_propensities",
                 "calculate_adjusted_propensities"],
    "neuralnetwork": ["confidence", "score_category_probabilities", "max_categories",
                      "score_propensity"],
}


def message(exc):
    get = getattr(exc, "getMessage", None)
    if get is not None:
        return unicode(get())
    return unicode(exc)


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


def ensure_clean_dir(path):
    # Deliberately non-recursive, restricted to this probe's dedicated label directory.
    if path != DIR:
        raise ValueError("unexpected corpus directory: " + path)
    directory = java.io.File(path)
    if directory.exists():
        children = directory.listFiles()
        if children is None:
            raise IOError("cannot list " + path)
        for child in children:
            if child.isFile() and unicode(child.getName()).endswith((".csv", ".str")):
                if not child.delete():
                    raise IOError("cannot remove old probe artifact " + unicode(child))
    elif not directory.mkdirs():
        raise IOError("cannot create " + path)


def csv_cell(value):
    if value is None:
        return ""
    # Quote every string, including the intentional empty string and whitespace.
    if isinstance(value, basestring):
        return '"' + value.replace('"', '""') + '"'
    if isinstance(value, float):
        return "%.17g" % value
    return str(value)


def write_csv(name, schema, rows, tests):
    lines = [",".join([field[0] for field in schema])]
    for row in rows:
        if len(row) != len(schema):
            raise ValueError("wrong CSV width in " + name)
        lines.append(",".join([csv_cell(value) for value in row]))
    handle = open(DIR + "/" + name, "wb")
    try:
        handle.write((NL.join(lines) + NL).encode("utf-8"))
    finally:
        handle.close()
    manifest["saved"].append(name)
    manifest["inputs"].append({"file": name, "rows": len(rows), "tests": tests})


def read_properties(node, names):
    result = {}
    for name in names:
        try:
            result[name] = snapshot(node.getPropertyValue(name))
        except ERRORS as exc:
            result[name] = {"read_error": message(exc)}
    return result


def set_properties(node, props, entry):
    for name, value in props:
        entry["setting_in_progress"] = [name, snapshot(value)]
        node.setPropertyValue(name, value)
        # A rejected setting fails the case; a coerced setting is visible in the manifest.
        entry.setdefault("readback", []).append(
            [name, snapshot(node.getPropertyValue(name))])
    entry.pop("setting_in_progress", None)


def instantiate_node(stream, node):
    table = stream.createAt("table", "Instantiate", 1000, 800)
    stream.link(node, table)
    try:
        table.run([])
    finally:
        stream.delete(table)


def typed_source(stream, name, schema, targets, inputs, roles, missing, y_position, entry):
    source = stream.createAt("variablefile", name, 60, y_position)
    source_props = [("full_filename", DIR + "/" + name), ("read_field_names", True),
                    ("delimit_comma", True), ("delimit_space", False), ("delimit_tab", False),
                    ("quotes_2", "PairAndDiscard"), ("strip_spaces", "None"),
                    ("decimal_symbol", "Period"), ("encoding", "UTF-8"),
                    ("auto_recognize_datetime", False), ("lines_to_scan", 1000)]
    source_entry = {"data": name, "properties": source_props, "storage": []}
    entry.setdefault("sources", []).append(source_entry)
    set_properties(source, source_props, source_entry)
    instantiate_node(stream, source)
    for field, storage, measure, values in schema:
        source_entry["setting_in_progress"] = ["custom_storage_type", field, storage]
        source.setKeyedPropertyValue("custom_storage_type", field, storage)
        source_entry["storage"].append([field, storage])
    source_entry.pop("setting_in_progress", None)

    node = stream.createAt("type", "Type " + name, 260, y_position)
    stream.link(source, node)
    instantiate_node(stream, node)
    type_entry = {"data": name, "fields": [], "missing": missing}
    entry.setdefault("types", []).append(type_entry)
    for field, storage, measure, values in schema:
        node.setKeyedPropertyValue("type", field, measure)
        if values is not None:
            node.setKeyedPropertyValue("values", field, values)
            node.setKeyedPropertyValue("extend_values", field, False)
        direction = roles.get(field, "None")
        if field in targets:
            direction = "Target"
        elif field in inputs:
            direction = "Input"
        node.setKeyedPropertyValue("direction", field, direction)
        node.setKeyedPropertyValue("check", field, "None")
        if storage == "Real":
            node.setKeyedPropertyValue("export_places", field, 15)
        for prop, key, value in missing:
            if key == field:
                node.setKeyedPropertyValue(prop, field, value)
    instantiate_node(stream, node)
    for field, storage, measure, values in schema:
        actual = {"name": field, "storage_requested": storage}
        for prop in ["storage", "type", "direction", "values", "enable_missing", "missing_values",
                     "null_missing", "whitespace_missing"]:
            try:
                actual[prop] = snapshot(node.getKeyedPropertyValue(prop, field))
            except ERRORS as exc:
                actual[prop] = {"read_error": message(exc)}
        type_entry["fields"].append(actual)
    return node


def find_applier(stream):
    found = []
    iterator = stream.iterator()
    while iterator.hasNext():
        node = iterator.next()
        if unicode(node.getTypeName()).startswith("apply"):
            found.append(node)
    if len(found) > 1:
        raise RuntimeError("unexpected multiple automatically placed nuggets")
    if found:
        return found[0]
    return None


def add_case(name, family, data, schema, targets, inputs, props, tests,
             score_data=None, missing=None, roles=None, stream_props=None,
             score_variants=None, compare_to=None):
    entry = {"case": name, "file": name + ".str", "family": family, "data": data,
             "schema": schema, "targets": targets, "inputs": inputs,
             "properties": props, "tests": tests,
             "score_data": score_data, "missing": missing or [], "roles": roles or {},
             "stream_properties": stream_props or [], "score_variants": score_variants or [],
             "compare_to": compare_to, "status": "planned"}
    manifest["cases"].append(entry)


def build_case(entry):
    stream = None
    stage = "create_stream"
    family = entry["family"]
    try:
        stream = taskrunner.createStream(entry["case"], False, False)
        common_stream = [("decimal_places", 15), ("decimal_symbol", "Period"),
                         ("enable_parallelism", False),
                         ("create_model_applier_for_new_models", True),
                         ("create_model_applier_update_links", "createEnabled")]
        entry["common_stream_properties"] = common_stream
        set_properties(stream, common_stream + entry["stream_properties"], entry)
        stage = "training_type"
        schema = entry["schema"]
        training = typed_source(stream, entry["data"], schema, entry["targets"],
                                entry["inputs"], entry["roles"], entry["missing"], 100, entry)
        stage = "build_properties"
        builder = stream.createAt(family, entry["case"], 480, 100)
        stream.link(training, builder)
        entry["build_type"] = unicode(builder.getTypeName())
        entry["builder_defaults"] = read_properties(builder, BUILD_PROPERTIES[family])
        # Identical model names across repeated cases: case labels must not perturb training.
        common_build = [("use_model_name", True), ("model_name", "models_ml")]
        if family == "neuralnetwork":
            common_build += [("use_random_seed", True), ("random_seed", SEED)]
        entry["common_build_properties"] = common_build
        set_properties(builder, common_build + entry["properties"], entry)
        entry["builder_effective"] = read_properties(builder, BUILD_PROPERTIES[family])
        stage = "train"
        results = []
        builder.run(results)
        applier = find_applier(stream)
        entry["auto_placed_applier"] = applier is not None
        models = [result for result in results if isinstance(result, modeler.api.ModelOutput)]
        if not models:
            raise RuntimeError("builder returned no ModelOutput for saved scoring nuggets")
        if applier is None:
            applier = stream.createModelApplierAt(models[0], "Train scores", 480, 260)
            stream.link(training, applier)
        else:
            applier.setXYPosition(480, 260)
        entry["applier_type"] = unicode(applier.getTypeName())
        entry["applier_defaults"] = read_properties(applier, APPLY_PROPERTIES[family])
        terminals = []
        entry["terminals"] = []

        def attach(nugget, label, position, properties):
            settings = {"label": label, "properties": properties}
            set_properties(nugget, properties, settings)
            settings["effective"] = read_properties(nugget, APPLY_PROPERTIES[family])
            table = stream.createAt("table", label, 760, position)
            stream.link(nugget, table)
            settings["applier_id"] = unicode(nugget.getID())
            settings["table_id"] = unicode(table.getID())
            entry["terminals"].append(settings)
            terminals.append(table)

        attach(applier, "Train scores", 260, [])
        scoring = training
        stage = "scoring_type"
        if entry["score_data"] is not None:
            scoring = typed_source(stream, entry["score_data"], schema, entry["targets"],
                                   entry["inputs"], entry["roles"], entry["missing"], 420, entry)
        variants = [("Grid scores", [])] + entry["score_variants"]
        stage = "scoring_properties"
        for index, (label, properties) in enumerate(variants):
            position = 420 + index * 100
            nugget = stream.createModelApplierAt(models[0], label, 480, position)
            stream.link(scoring, nugget)
            attach(nugget, label, position, properties)

        # Preserve a usable stream even when an edge-value Table execution fails.
        stage = "save_stream"
        taskrunner.saveStreamToFile(stream, DIR + "/" + entry["file"])
        manifest["saved"].append(entry["file"])
        entry["status"] = "saved"
        stage = "score"
        for table in terminals:
            try:
                table.run([])
            except ERRORS as exc:
                manifest["errors"].append({"case": entry["case"], "stage": "score",
                                           "table": unicode(table.getLabel()),
                                           "message": message(exc)})
                entry["status"] = "saved_with_score_errors"
    except ERRORS as exc:
        entry["status"] = "failed"
        manifest["errors"].append({"case": entry["case"], "stage": stage,
                                   "message": message(exc)})
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS as exc:
                manifest["errors"].append({"case": entry["case"], "stage": "clear",
                                           "message": message(exc)})
        write_manifest()


def block(rows, group, m, c, q, r, weight=1.0):
    major, minor = ("P", "Q") if group % 2 else ("Q", "P")
    name = "g%02d" % group
    for i in range(m):
        rows.append([len(rows) + 1, name, "F", minor if i < r else major, weight])
    for i in range(c):
        rows.append([len(rows) + 1, name, "T", minor if i < q else major, weight])


def declare_inputs():
    def grid(extra):
        rows = []
        for i, spec in enumerate(GRID):
            block(rows, i + 1, *spec)
        extra(rows)
        return rows
    def none(rows):
        pass
    def pure(rows):
        for i in range(16):
            block(rows, 17 + i, 20, 2, 0, 0)
    def noisy(rows):
        for i in range(16):
            block(rows, 17 + i, 20, 2, 0, 4)
    write_csv("gp_grid.csv", GP_SCHEMA, grid(none), "16 exception blocks by local margin")
    write_csv("gp_pure.csv", GP_SCHEMA, grid(pure), "grid plus 16 pure groups of 22 records")
    write_csv("gp_noisy.csv", GP_SCHEMA, grid(noisy), "grid plus 16 groups of 22 with 4 unsplittable errors")
    for copies in [1, 2, 4, 8]:
        rows = []
        for i in range(copies):
            block(rows, i + 1, 8, 6, 4, 1)
            block(rows, 20 + i, 30, 2, 0, 0)
        write_csv("gp_copies%d.csv" % copies, GP_SCHEMA, rows, "%d copies of block (8, 6, 4, 1), margin 1.07, with pure partners" % copies)
    rows = []
    block(rows, 1, 8, 6, 4, 1)
    block(rows, 20, 300, 2, 0, 0)
    write_csv("gp_copies1_big.csv", GP_SCHEMA, rows, "copies1 with a 302-record pure partner")
    for v in VALUE_COUNTS:
        for ratio in RATIOS:
            n = int(round(v * ratio))
            rows = []
            block(rows, 1, 8, 6, 4, 1)
            block(rows, 2, max(n - 16, 2), 2, 0, 0)
            write_csv("gv%d_%s.csv" % (v, str(ratio).replace(".", "_")), schema_values(v), rows, "g declares %d values; %d records" % (v, len(rows)))
    for n in SUB_NS:
        rows = []
        block(rows, 1, 8, 6, 4, 1)
        block(rows, 2, n - 16, 2, 0, 0)
        write_csv("gf16_%d.csv" % n, schema_values(16), rows, "16 values, %d records" % n)
    for v, ns in FINE.items():
        for n in ns:
            for kind in ["a", "b"]:
                rows = []
                if kind == "a":
                    block(rows, 1, 8, 6, 4, 1)
                else:
                    block(rows, 1, 12, 2, 0, 0)
                block(rows, 2, n - 16, 2, 0, 0)
                write_csv("gf%d_%d%s.csv" % (v, n, kind), schema_values(v), rows, "g declares %d values; %d records; block %s" % (v, n, kind))
    for n in SUB_NS:
        rows = []
        block(rows, 1, 8, 6, 4, 1)
        block(rows, 2, n - 16, 2, 0, 0)
        # h = T: a pure P group the root separates first; g's values repeat there.
        rows = [r + ["F"] for r in rows] + [[1000 + i, "g%02d" % (1 + i % 16), "F", "P", 1.0, "T"] for i in range(200)]
        write_csv("gs16_%d.csv" % n, SUB_SCHEMA, rows, "h = F holds the 16-value scan data with %d records" % n)
    write_csv("gp_grid_w2.csv", GP_SCHEMA, [r[:4] + [2.0 if r[0] % 3 == 0 else 1.0] for r in grid(none)], "grid with every third record weight 2")

def missing_settings(schema, fields):
    props = []
    for field, storage, measure, values in schema:
        if field not in fields:
            continue
        props += [("enable_missing", field, True), ("null_missing", field, True)]
        if storage == "String":
            props += [("whitespace_missing", field, True),
                      ("missing_values", field, ["MISSING"])]
        else:
            props += [("missing_values", field, [-99.0])]
    return props


def declare_cases():
    def gp(name, props, tests, data, **kwargs):
        add_case("c50_" + name, "c50", data, GP_SCHEMA, ["y"], GP_INPUTS, props, tests, score_data=data, **kwargs)
    noglobal = [("mode", "Expert"), ("use_global_pruning", False)]
    for data in ["gp_grid", "gp_pure", "gp_noisy"]:
        gp(data, [], "Global pruning decisions per block", data + ".csv")
        gp(data + "_noglobal", noglobal, "Local pruning only", data + ".csv")
    for sev in [50, 90]:
        gp("gp_grid_sev%d" % sev, [("mode", "Expert"), ("pruning_severity", sev)], "Severity %d with global pruning" % sev, "gp_grid.csv")
        gp("gp_grid_sev%d_noglobal" % sev, noglobal + [("pruning_severity", sev)], "Severity %d, local only" % sev, "gp_grid.csv")
    for copies in [1, 2, 4, 8]:
        gp("gp_copies%d" % copies, [], "Identical weak blocks: independent or budgeted?", "gp_copies%d.csv" % copies)
    for copies in [1, 2]:
        gp("gp_copies%d_noglobal" % copies, noglobal, "Local pruning only", "gp_copies%d.csv" % copies)
        add_case("c50_gp_copies%d_small" % copies, "c50", "gp_copies%d.csv" % copies, GP_SMALL, ["y"], GP_INPUTS, [], "g declares only the used groups", score_data="gp_copies%d.csv" % copies)
    gp("gp_copies1_big", [], "Large pure partner", "gp_copies1_big.csv")
    gp("gp_copies1_big_noglobal", noglobal, "Large pure partner, local only", "gp_copies1_big.csv")
    gp("gp_grid_w2", [("use_weight", True), ("weight_field", "weight")], "Grid with case weights", "gp_grid_w2.csv")
    for v in VALUE_COUNTS:
        for ratio in RATIOS:
            name = "gv%d_%s" % (v, str(ratio).replace(".", "_"))
            add_case("c50_" + name, "c50", name + ".csv", schema_values(v), ["y"], GP_INPUTS, [], "Values versus records: is g used?", score_data=name + ".csv")
    for n in SUB_NS:
        add_case("c50_gsub16_%d" % n, "c50", "gs16_%d.csv" % n, SUB_SCHEMA, ["y"], ["g", "z", "h"], [], "Sub-node: is g used at h = F?", score_data="gs16_%d.csv" % n)
        add_case("c50_gmin5_16_%d" % n, "c50", "gf16_%d.csv" % n, schema_values(16), ["y"], GP_INPUTS, [("mode", "Expert"), ("min_child_records", 5)], "Minimum child 5: does the threshold move?", score_data="gf16_%d.csv" % n)
    for v, ns in FINE.items():
        for n in ns:
            for kind in ["a", "b"]:
                name = "gf%d_%d%s" % (v, n, kind)
                add_case("c50_" + name, "c50", name + ".csv", schema_values(v), ["y"], GP_INPUTS, [], "Fine scan of the many-values threshold", score_data=name + ".csv")
    gp("gp_grid_w2_noglobal", noglobal + [("use_weight", True), ("weight_field", "weight")], "Grid with case weights, local only", "gp_grid_w2.csv")

try:
    ensure_clean_dir(DIR)
    declare_inputs()
    declare_cases()
    write_manifest()
    for case in manifest["cases"]:
        build_case(case)
except ERRORS as exc:
    manifest["errors"].append({"case": None, "stage": "setup", "message": message(exc)})
finally:
    write_manifest()
