# Probe: C5.0, Bayesian Network and the current Neural Net (neuralnetwork).
# Coordinator only: cargo xtask probe models_ml_streams (Modeler 19, Jython 2.7).
# Writes generated inputs and one stream per case to corpus/models_ml; public API only.
# See probe-plans/models_ml_streams.md for contrasts and oracle capture limitations.
import json
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/models_ml"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
SEED = 12345
NL = chr(10)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "models_ml", "saved": [], "errors": [], "cases": [], "inputs": []}

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


def declare_inputs():
    tree = []
    for rep in range(6):
        for xi in range(8):
            x = float(xi * 2)
            for ci, cat in enumerate(["amber", "blue", "green", "red"]):
                for fi, flag in enumerate(["F", "T"]):
                    y = "A" if x < 4 else ("B" if ci < 2 else "C")
                    if fi == 1 and x >= 10:
                        y = "A"
                    # Local exceptions and conflicting duplicate predictors exercise pruning,
                    # costs and confidence; there is no perfectly separating predictor.
                    if rep == 0 and xi in [2, 5] and ci == 3:
                        y = "B"
                    part = 1 + rep // 2
                    # Held-out distributions differ, so accidentally training on all partitions
                    # changes the fitted model rather than just its reported accuracy.
                    if part == 2 and xi == 3:
                        y = "A"
                    if part == 3 and xi == 3:
                        y = "C"
                    tree.append([len(tree) + 1, x, cat, flag,
                                 ["low", "middle", "high"][rep % 3], 1 + rep % 3, 7.0,
                                 ["u", "v", "w"][(rep + ci + fi) % 3], y,
                                 "Y" if y == "C" else "N", part,
                                 6.0 if rep == 0 and y == "B" else 1.0])
    write_csv("tree_train.csv", TREE_SCHEMA, tree, "384 rows; noisy piecewise 3-class rule, categorical grouping, irrelevant inputs, conflicting duplicates, shifted holdouts")
    tree_missing = [list(row) for row in tree]
    for index, row in enumerate(tree_missing):
        if index % 19 == 0:
            row[1] = None
        if index % 23 == 0:
            row[2] = ""
        if index % 29 == 0:
            row[2] = "MISSING"
        if index % 31 == 0:
            row[8] = ""
    write_csv("tree_missing.csv", TREE_SCHEMA, tree_missing, "numeric null, empty categorical input/target, explicit MISSING token")
    tree_grid = []
    for x in [-2.0, 0.0, 1.999999, 2.0, 2.000001, 2.999999, 3.0, 3.000001,
              3.999999, 4.0, 4.000001, 8.999999, 9.0, 9.000001,
              9.999999, 10.0, 10.000001, 15.0]:
        for cat in ["amber", "blue", "green", "red"]:
            for flag in ["F", "T"]:
                tree_grid.append([10001 + len(tree_grid), x, cat, flag, "middle", 2,
                                  7.0, "v", "A", "N", 1, 1.0])
    # Duplicate score rows have different IDs only; preserve the original record order.
    tree_grid.append(list(tree_grid[0]))
    tree_grid[-1][0] = 10001 + len(tree_grid) - 1
    write_csv("tree_grid.csv", TREE_SCHEMA, tree_grid, "145 rows; observed-value/midpoint thresholds and neighbours, outside range, all symbolics, duplicate predictors")
    edge = [list(row) for row in tree_grid[:8]]
    edge[0][1] = None
    edge[1][2] = ""
    edge[2][2] = "   "
    edge[3][2] = "MISSING"
    edge[4][2] = "unseen"
    edge[5][1], edge[5][2], edge[5][3] = None, "", ""
    edge[6][8] = ""
    edge[7][1] = -99.0
    for index, row in enumerate(edge):
        row[0] = 10001 + len(tree_grid) + index
    write_csv("tree_edge_grid.csv", TREE_SCHEMA, tree_grid + edge, "153 rows; null, empty, whitespace, declared missing, unknown category, absent target, numeric missing sentinel")

    bn = []
    counts = [("N", 0, 0, 18), ("N", 0, 1, 2), ("N", 1, 0, 3), ("N", 1, 1, 7),
              ("Y", 0, 0, 2), ("Y", 0, 1, 8), ("Y", 1, 0, 12), ("Y", 1, 1, 0)]
    for target, a, b, count in counts:
        for k in range(count):
            for noise in ["u", "v", "w"]:
                c = b if k % 5 else 1 - b
                x = [0.0, 1.0, 21.0, 41.0, 99.0, 100.0][k % 6]
                part = 1 + k % 3
                yn = target
                if part == 2 and a == 0 and b == 1:
                    yn = "N"
                if part == 3 and a == 1 and b == 0:
                    yn = "N"
                bn.append([len(bn) + 1, "a%d" % a, "b%d" % b, "c%d" % c,
                           noise, x, yn, 1 if yn == "N" else 2, part])
    write_csv("bn_train.csv", BN_SCHEMA, bn, "156 rows; dependent predictors, zero Y/a1/b1 cell, independent balanced noise, unequal priors, gapped continuous input")
    write_csv("bn_reverse.csv", BN_SCHEMA, list(reversed(bn)), "same IDs/records in reverse order; duplicates retained")
    bn_missing = [list(row) for row in bn]
    for index, row in enumerate(bn_missing):
        if index % 11 == 0:
            row[1] = ""
        if index % 13 == 0:
            row[2] = "MISSING"
        if index % 17 == 0:
            row[5] = None
        if index % 23 == 0:
            row[6] = ""
    write_csv("bn_missing.csv", BN_SCHEMA, bn_missing, "incomplete predictors and missing targets; pairwise versus complete-case CPT estimation")
    bn_grid = []
    for a in ["a0", "a1"]:
        for b in ["b0", "b1"]:
            for c in ["c0", "c1"]:
                for noise in ["u", "v", "w"]:
                    bn_grid.append([10001 + len(bn_grid), a, b, c, noise, 21.0, "N", 1, 1])
    # Exact candidate five-bin cuts and their neighbours, including an empty-bin interval.
    for x in [-1.0, 0.0, 19.999999, 20.0, 20.000001, 39.999999, 40.0, 40.000001,
              59.999999, 60.0, 60.000001, 79.999999, 80.0, 80.000001, 100.0, 101.0]:
        bn_grid.append([10001 + len(bn_grid), "a0", "b1", "c1", "u", x, "N", 1, 1])
    write_csv("bn_grid.csv", BN_SCHEMA, bn_grid, "40 rows; full categorical Cartesian grid and five-bin boundary neighbours")
    edge = [list(row) for row in bn_grid[:7]]
    edge[0][1] = ""
    edge[1][2] = "MISSING"
    edge[2][3] = "   "
    edge[3][5] = None
    edge[4][1] = "unseen"
    edge[5][1], edge[5][2], edge[5][3] = "", "", ""
    edge[6][6] = ""
    for index, row in enumerate(edge):
        row[0] = 10001 + len(bn_grid) + index
    write_csv("bn_edge_grid.csv", BN_SCHEMA, bn_grid + edge, "47 rows; missing and unknown evidence, empty/whitespace, absent target")

    nn = []
    for rep in range(6):
        for xi in range(10):
            x = float(xi) / 2.0
            for ci, cat in enumerate(["blue", "green", "red"]):
                flag = (rep + xi) % 2
                z = float((xi * 3 + rep) % 7) / 3.0
                y = ["A", "B", "C"][(ci + (1 if x >= 2.5 else 0) + flag) % 3]
                part = 1 + rep // 2
                if part == 2 and xi == 5:
                    y = "A"
                if part == 3 and xi == 5:
                    y = "C"
                r = 0.25 + 1.5 * x - 0.75 * z + ci + 0.2 * flag
                nn.append([len(nn) + 1, x, z, cat, flag, y, r, 1 if y == "C" else 0, part])
    write_csv("nn_train.csv", NN_SCHEMA, nn, "180 ordered records; mixed inputs, three-class target and continuous target, shifted holdouts")
    write_csv("nn_reverse.csv", NN_SCHEMA, list(reversed(nn)), "same IDs/records, reversed training order")
    nn_missing = [list(row) for row in nn]
    # Skew the observed x distribution: midpoint of range differs strongly from mean.
    for index, row in enumerate(nn_missing):
        if index % 5 != 0:
            row[1] = 0.0
        if index % 11 == 0:
            row[1] = None
        if index % 13 == 0:
            row[3] = ""
        if index % 17 == 0:
            row[3] = "MISSING"
        if index % 19 == 0:
            row[5] = ""
        if index % 23 == 0:
            row[6] = None
        # Create a unique categorical mode, avoiding an arbitrary modal tie.
        if index % 4 == 1:
            row[3] = "blue"
    write_csv("nn_missing.csv", NN_SCHEMA, nn_missing, "skewed x and modal blue distinguish midpoint/mode imputation from mean or zero; missing targets stay excluded")
    nn_grid = []
    for x in [-1.0, 0.0, 2.0, 2.25, 4.0, 4.5, 6.0]:
        for cat in ["blue", "green", "red"]:
            for flag in [0, 1]:
                nn_grid.append([10001 + len(nn_grid), x, 0.5, cat, flag, "A", 0.0, 0, 1])
    nn_grid.append(list(nn_grid[0]))
    nn_grid[-1][0] = 10001 + len(nn_grid) - 1
    write_csv("nn_grid.csv", NN_SCHEMA, nn_grid, "43 rows; full input categories, explicit imputation midpoint 2, interpolation/extrapolation and duplicate predictors")
    edge = [list(row) for row in nn_grid[:8]]
    edge[0][1] = None
    edge[1][3] = ""
    edge[2][3] = "   "
    edge[3][3] = "MISSING"
    edge[4][3] = "unseen"
    edge[5][1], edge[5][2], edge[5][3], edge[5][4] = None, None, "", None
    edge[6][5] = ""
    edge[6][6] = None
    edge[7][1] = -99.0
    for index, row in enumerate(edge):
        row[0] = 10001 + len(nn_grid) + index
    write_csv("nn_edge_grid.csv", NN_SCHEMA, nn_grid + edge, "51 rows; numeric null/sentinel, empty/whitespace, unknown category, all missing inputs, both targets absent")


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
    # Default cases leave algorithm controls untouched. Explicit common controls, seeds,
    # types and stream precision are separately recorded, not described as factory defaults.
    def tree(name, props, tests, **kwargs):
        data = kwargs.pop("data", "tree_train.csv")
        target = kwargs.pop("target", "y")
        grid = kwargs.pop("score_data", "tree_grid.csv")
        add_case("c50_" + name, "c50", data, TREE_SCHEMA, [target], TREE_INPUTS,
                 props, tests, score_data=grid, **kwargs)

    tree("default", [], "New C5.0 defaults: multiway symbols, continuous thresholds, confidence, noisy duplicates")
    tree("rules_voting", [("output_type", "RuleSet")], "Direct rules and rule-voting confidence versus default tree", stream_props=[("ruleset_evaluation", "Voting")], compare_to="c50_default")
    tree("rules_firsthit", [("output_type", "RuleSet")], "Same rules, first matching rule versus confidence voting", stream_props=[("ruleset_evaluation", "FirstHit")], compare_to="c50_rules_voting")
    tree("group_symbols", [("group_symbolics", True)], "Merge amber/blue and green/red category branches", compare_to="c50_default")
    tree("boost_tree", [("use_boost", True), ("boost_num_trials", 5)], "Boosted tree vote normalization; actual trial count may stop early", compare_to="c50_default")
    tree("boost_rules", [("output_type", "RuleSet"), ("use_boost", True), ("boost_num_trials", 5)], "Boosted direct rules: per-rule and cross-trial votes", stream_props=[("ruleset_evaluation", "Voting")], compare_to="c50_rules_voting")
    tree("generality", [("mode", "Simple"), ("favor", "Generality")], "Simple generality preset versus accuracy", compare_to="c50_default")
    tree("noise20", [("mode", "Simple"), ("expected_noise", 20)], "Expected noise on conflicting repeated predictors", compare_to="c50_default")
    tree("expert", [("mode", "Expert")], "Expert defaults control, separating mode changes from individual expert settings", compare_to="c50_default")
    tree("min_child", [("mode", "Expert"), ("min_child_records", 20)], "Minimum child support: at least two branches must meet the limit", compare_to="c50_expert")
    tree("prune90", [("mode", "Expert"), ("pruning_severity", 90)], "Stronger local pruning on rare exceptions", compare_to="c50_expert")
    tree("no_global_prune", [("mode", "Expert"), ("use_global_pruning", False)], "Global versus local pruning", compare_to="c50_expert")
    tree("winnow", [("mode", "Expert"), ("use_winnowing", True)], "Winnowing with constant and balanced irrelevant fields", compare_to="c50_expert")
    tree("costs", [("use_costs", True), ("costs", [["B", "C", 8.0], ["C", "B", 2.0]])], "Asymmetric B/C errors reveal cost matrix direction", compare_to="c50_default")
    tree("weight", [("use_weight", True), ("weight_field", "weight")], "Real case weights emphasize conflicting B observations", compare_to="c50_default")
    tree("missing", [], "Training and scoring with nulls, empty strings, declared blanks and unknown symbols", data="tree_missing.csv", score_data="tree_edge_grid.csv", missing=missing_settings(TREE_SCHEMA, TREE_INPUTS + ["y"]), compare_to="c50_default")
    tree("partition", [("use_partitioned_data", True), ("calculate_raw_propensities", True), ("calculate_adjusted_propensities", True), ("adjusted_propensity_partition", "Test")], "Train partition 1 only; score all records; flag raw and held-out adjusted propensity", target="binary", roles={"part": "Partition"}, score_variants=[("No propensity", [("calculate_raw_propensities", False), ("calculate_adjusted_propensities", False)])])

    bn_probs = [("All probabilities", [("all_probabilities", True)]),
                ("Predicted probability only", [("all_probabilities", False)])]

    def bayes(name, props, tests, **kwargs):
        data = kwargs.pop("data", "bn_train.csv")
        target = kwargs.pop("target", "y")
        inputs = kwargs.pop("inputs", BN_INPUTS)
        grid = kwargs.pop("score_data", "bn_grid.csv")
        variants = kwargs.pop("score_variants", bn_probs)
        add_case("bn_" + name, "bayesnet", data, BN_SCHEMA, [target], inputs,
                 props, tests, score_data=grid, score_variants=variants, **kwargs)

    bayes("default", [], "Default TAN/likelihood; zero cell and dependent inputs separate TAN from naive Bayes")
    bayes("bayes_adjustment", [("parameter_learning_method", "Bayes")], "Small-cell Dirichlet adjustment versus ML, including zero evidence", compare_to="bn_default")
    mb = [("mode", "Expert"), ("structure_type", "MarkovBlanket"),
          ("independence", "Likelihood"), ("significance_level", 0.01),
          ("maximal_conditioning_set", 5)]
    bayes("markov", mb, "Markov blanket versus TAN, likelihood independence test", compare_to="bn_default")
    bayes("markov_pearson", mb + [("independence", "Pearson")], "Pearson versus likelihood conditional independence", compare_to="bn_markov")
    bayes("markov_alpha", mb + [("significance_level", 0.2)], "Relax independence cutoff from 0.01 to 0.2", compare_to="bn_markov")
    bayes("markov_cond0", mb + [("maximal_conditioning_set", 0)], "Unconditional tests only versus conditional-set size 5", compare_to="bn_markov")
    fs = [("mode", "Expert"), ("structure_type", "TAN"), ("use_feature_selection", True)]
    bayes("feature_selection", fs, "Feature preprocessing drops independent balanced noise", compare_to="bn_default")
    bayes("feature_cap", fs + [("maximum_number_inputs", 2), ("inputs_always_selected", ["noise"])], "Input cap and forced candidate; forced input need not survive later independence tests", compare_to="bn_feature_selection")
    bayes("continuous_bins", [], "Five equal-width bins, boundary inclusivity and empty-bin merging", inputs=["a", "b", "x"], compare_to="bn_default")
    blanks = missing_settings(BN_SCHEMA, BN_INPUTS + ["x", "y"])
    bayes("missing_complete", [("mode", "Expert"), ("missing_values", True)], "Complete-record training; absent targets; scoring with partial evidence", data="bn_missing.csv", score_data="bn_edge_grid.csv", inputs=BN_INPUTS + ["x"], missing=blanks)
    bayes("missing_pairwise", [("mode", "Expert"), ("missing_values", False)], "Pairwise available counts versus listwise deletion", data="bn_missing.csv", score_data="bn_edge_grid.csv", inputs=BN_INPUTS + ["x"], missing=blanks, compare_to="bn_missing_complete")
    bayes("reverse", [], "Reversed records and duplicated observations expose order/root/tie sensitivity", data="bn_reverse.csv", compare_to="bn_default")
    bayes("numeric_target", [], "Integer nominal target 1/2 versus string flag N/Y", target="ynum", compare_to="bn_default")
    propensity = [("use_partitioned_data", True), ("calculate_raw_propensities", True),
                  ("calculate_adjusted_propensities", True)]
    bayes("partition_test", propensity + [("adjusted_propensity_partition", "Test")], "Training partition only; raw and adjusted flag propensity using test partition", roles={"part": "Partition"}, compare_to="bn_default")
    bayes("partition_validation", propensity + [("adjusted_propensity_partition", "Validation")], "Same training rows, adjust using validation rather than test", roles={"part": "Partition"}, compare_to="bn_partition_test")

    # Fixed topology and cycle controls remove architecture-search, time and holdout variation.
    # They do not prove that the vendor's optimizer itself is reproducible: the pairs test that.
    fixed = [("objective", "Standard"), ("method", "MultilayerPerceptron"),
             ("use_custom_layers", True), ("first_layer_units", 4), ("second_layer_units", 2),
             ("use_max_time", False), ("use_max_cycles", True), ("max_cycles", 40),
             ("use_min_accuracy", False), ("overfit_prevention_pct", 0),
             ("missing_values", "listwiseDeletion"), ("use_partitioned_data", False)]
    nn_scores = [("No category probabilities", [("score_category_probabilities", False)]),
                 ("All category probabilities", [("score_category_probabilities", True), ("max_categories", 3)]),
                 ("One category probability", [("score_category_probabilities", True), ("max_categories", 1)]),
                 ("Confidence probability", [("confidence", "onProbability")]),
                 ("Confidence increase", [("confidence", "onIncrease")])]

    def neural(name, props, tests, **kwargs):
        data = kwargs.pop("data", "nn_train.csv")
        targets = kwargs.pop("targets", ["y"])
        grid = kwargs.pop("score_data", "nn_grid.csv")
        variants = kwargs.pop("score_variants", [])
        add_case("nn_" + name, "neuralnetwork", data, NN_SCHEMA, targets, NN_INPUTS,
                 props, tests, score_data=grid, score_variants=variants, **kwargs)

    neural("default_a", [], "Default architecture/stopping, explicit seed 12345; repeat A")
    neural("default_b", [], "Identical default/seed/data repeat B; compare scores and weights, ignoring IDs/timestamps", compare_to="nn_default_a")
    neural("fixed_a", fixed, "Fixed 4/2 hidden layers, 40-cycle cap, no time cap or overfit holdout; repeat A", score_variants=nn_scores)
    neural("fixed_b", fixed, "Identical controlled repeat B; same seed, names, fields, row order", score_variants=nn_scores, compare_to="nn_fixed_a")
    neural("seed2", fixed + [("random_seed", 54321)], "Seed-only change distinguishes seed support from coincidental equality", compare_to="nn_fixed_a")
    neural("reverse", fixed, "Same IDs/records in reverse training order; scoring grid order unchanged", data="nn_reverse.csv", compare_to="nn_fixed_a")
    neural("one_cycle", fixed + [("max_cycles", 1)], "One full data pass and best-weight checkpoint versus 40-cycle cap", compare_to="nn_fixed_a")
    neural("accuracy", fixed + [("use_min_accuracy", True), ("min_accuracy", 40)], "Accuracy stopping enabled with the same 40-cycle safety cap", compare_to="nn_fixed_a")
    neural("overfit30", fixed + [("overfit_prevention_pct", 30)], "Internal random holdout and best-overfit checkpoint versus no holdout", compare_to="nn_fixed_a")
    neural("regression", fixed, "Continuous target, output identity and rescaling", targets=["r"], compare_to="nn_fixed_a")
    neural("mixed_targets", fixed, "Joint categorical and continuous targets; squared error versus all-categorical softmax", targets=["y", "r"], compare_to="nn_fixed_a")
    neural("rbf", fixed + [("method", "RadialBasisFunction"), ("second_layer_units", 0)], "RBF single hidden layer, centers/widths and output least squares; MLP stopping controls ignored", targets=["r"], compare_to="nn_regression")
    blanks = missing_settings(NN_SCHEMA, NN_INPUTS + ["y", "r"])
    neural("missing_listwise", fixed, "Listwise predictor exclusion, empty/missing targets and score-time missingness", data="nn_missing.csv", score_data="nn_edge_grid.csv", missing=blanks, targets=["y", "r"])
    neural("missing_impute", fixed + [("missing_values", "missingValueImputation")], "Range-midpoint/mode imputation versus listwise; not mean substitution", data="nn_missing.csv", score_data="nn_edge_grid.csv", missing=blanks, targets=["y", "r"], compare_to="nn_missing_listwise")
    neural("partition", fixed + [("use_partitioned_data", True), ("score_propensity", True)], "External partition training with flag target; score all partitions and emit propensity", targets=["binary"], roles={"part": "Partition"}, score_variants=[("No propensity", [("score_propensity", False)])])
    ensemble = fixed + [("component_models_n", 3), ("overfit_prevention_pct", 30)]
    neural("bag_vote_mean", ensemble + [("objective", "Bagging"), ("combining_rule_categorical", "Voting"), ("combining_rule_continuous", "Mean")], "Three bagged mixed-target components; categorical vote and continuous mean", targets=["y", "r"])
    neural("bag_highest_median", ensemble + [("objective", "Bagging"), ("combining_rule_categorical", "HighestProbability"), ("combining_rule_continuous", "Median")], "Same seed/components; highest individual probability and median aggregation", targets=["y", "r"], compare_to="nn_bag_vote_mean")
    neural("boost_meanprob", ensemble + [("objective", "Boosting"), ("combining_rule_categorical", "HighestMeanProbability"), ("combining_rule_continuous", "Mean")], "Three boosted components; mean category probability and continuous mean", targets=["y", "r"], compare_to="nn_bag_vote_mean")


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
