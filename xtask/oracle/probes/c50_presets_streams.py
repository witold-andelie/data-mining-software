# Probe: C5.0 Simple-mode presets (favor generality, expected noise) and winnowing.
# Coordinator only: cargo xtask probe c50_presets_streams (Modeler 19, Jython 2.7).
# Writes generated inputs and one stream per case to corpus/c50_presets; public API only.
# Data: the exception-block grid of c50_globalprune_streams (sensitive to pruning), the
# same grid with weak noise fields (winnowing), and IBM's DRUG1n demo data.
import json
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/c50_presets"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
SEED = 12345
NL = chr(10)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "c50_presets", "saved": [], "errors": [], "cases": [], "inputs": []}

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
GW_SCHEMA = GP_SCHEMA + [("n1", "String", "Set", ["a", "b", "c"]), ("n2", "Real", "Range", None), ("n3", "String", "Flag", ["F", "T"])]
GW_INPUTS = ["g", "z", "n1", "n2", "n3"]
DRUG_SCHEMA = [("Age", "Integer", "Range", None), ("Sex", "String", "Set", ["F", "M"]),
               ("BP", "String", "Set", ["HIGH", "LOW", "NORMAL"]), ("Cholesterol", "String", "Set", ["HIGH", "NORMAL"]),
               ("Na", "Real", "Range", None), ("K", "Real", "Range", None),
               ("Drug", "String", "Set", ["drugA", "drugB", "drugC", "drugX", "drugY"])]
DRUG_INPUTS = ["Age", "Sex", "BP", "Cholesterol", "Na", "K"]
DRUG_NOISE_SCHEMA = DRUG_SCHEMA + [("e1", "String", "Set", ["u", "v", "w"]), ("e2", "Real", "Range", None), ("e3", "String", "Flag", ["F", "T"])]
WN_SCHEMA = [("id", "Integer", "Range", None), ("a", "String", "Set", ["a1", "a2", "a3"]), ("b", "Real", "Range", None),
             ("c1", "String", "Flag", ["F", "T"]), ("c2", "String", "Flag", ["F", "T"]), ("c3", "String", "Flag", ["F", "T"]),
             ("c4", "String", "Flag", ["F", "T"]), ("d", "Real", "Range", None), ("y", "String", "Set", ["P", "Q"])]
WN_INPUTS = ["a", "b", "c1", "c2", "c3", "c4", "d"]
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


def block(rows, group, m, c, q, r, weight=1.0):
    major, minor = ("P", "Q") if group % 2 else ("Q", "P")
    name = "g%02d" % group
    for i in range(m):
        rows.append([len(rows) + 1, name, "F", minor if i < r else major, weight])
    for i in range(c):
        rows.append([len(rows) + 1, name, "T", minor if i < q else major, weight])


def declare_inputs():
    grid = []
    for i, spec in enumerate(GRID):
        block(grid, i + 1, *spec)
    write_csv("gp_grid.csv", GP_SCHEMA, grid, "16 exception blocks by local margin (c50_globalprune)")
    noisy = [list(r) for r in grid]
    for i in range(16):
        block(noisy, 17 + i, 20, 2, 0, 4)
    write_csv("gp_noisy.csv", GP_SCHEMA, noisy, "grid plus 16 groups of 22 with 4 unsplittable errors")
    gw = []
    for k, r in enumerate(grid):
        n3 = r[2] if k % 5 else ("T" if r[2] == "F" else "F")
        gw.append(r + [["a", "b", "c"][(k * 7) % 3], float(k % 10), n3])
    write_csv("gw_grid.csv", GW_SCHEMA, gw, "grid plus n1 (cyclic), n2 (k mod 10), n3 (z with every fifth flipped)")
    handle = open("D:/data-mining-software/corpus/ibm-demo-data/DRUG1n", "rb")
    try:
        lines = handle.read().decode("utf-8").replace(chr(13), "").split(chr(10))
    finally:
        handle.close()
    drug = []
    for line in lines[1:]:
        if not line.strip():
            continue
        a = line.split(",")
        drug.append([int(a[0]), a[1], a[2], a[3], float(a[4]), float(a[5]), a[6]])
    write_csv("drug.csv", DRUG_SCHEMA, drug, "IBM DRUG1n demo data (200 records)")
    state = [12345]

    def rand():
        # Park-Miller minimal standard generator: deterministic, documented in the probe.
        state[0] = (state[0] * 48271) % 2147483647
        return state[0] / 2147483647.0
    wn = []
    for k in range(400):
        a = ["a1", "a2", "a3"][int(rand() * 3)]
        b = round(rand() * 10, 1)
        y = "P" if (a == "a1" and b < 6) or (a != "a1" and b >= 4) else "Q"
        if rand() < 0.1:
            y = "Q" if y == "P" else "P"
        flips = [0.3, 0.4, 0.45, 0.5]
        cs = []
        for f in flips:
            v = y == "P"
            if rand() < f:
                v = not v
            cs.append("T" if v else "F")
        wn.append([k + 1, a, b] + cs + [round(rand() * 100, 2), y])
    write_csv("wn.csv", WN_SCHEMA, wn, "y from a, b with 10% noise; c1..c4 copy y with flip rates 30/40/45/50%; d noise")
    write_csv("wn100.csv", WN_SCHEMA, wn[:100], "first 100 records of wn.csv")
    noisy_drug = []
    for k, r in enumerate(drug):
        noisy_drug.append(r + [["u", "v", "w"][(k * 5) % 3], float((k * 37) % 100), "T" if (k * 11) % 7 < 3 else "F"])
    write_csv("drug_noise.csv", DRUG_NOISE_SCHEMA, noisy_drug, "DRUG1n plus e1 (cyclic set), e2 (k*37 mod 100), e3 (flag)")
    write_csv("drug_half.csv", DRUG_SCHEMA, drug[:100], "first 100 DRUG1n records")
    write_csv("drug_twice.csv", DRUG_SCHEMA, drug + drug, "DRUG1n twice")

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
    data = [("grid", "gp_grid.csv", GP_SCHEMA, GP_INPUTS, "y"), ("noisy", "gp_noisy.csv", GP_SCHEMA, GP_INPUTS, "y"),
            ("drug", "drug.csv", DRUG_SCHEMA, DRUG_INPUTS, "Drug")]
    for tag, csv, schema, inputs, target in data:
        def case(name, props, tests):
            add_case("c50_%s_%s" % (tag, name), "c50", csv, schema, [target], inputs, props, tests, score_data=csv)
        case("simple", [("mode", "Simple")], "Simple mode, favor accuracy (baseline)")
        case("generality", [("mode", "Simple"), ("favor", "Generality")], "Simple mode, favor generality")
        for noise in [5, 10, 20, 30, 50]:
            case("noise%d" % noise, [("mode", "Simple"), ("expected_noise", noise)], "Simple mode, expected noise %d%%" % noise)
        case("generality_noise20", [("mode", "Simple"), ("favor", "Generality"), ("expected_noise", 20)], "Generality with 20% noise")
        case("winnow", [("mode", "Expert"), ("use_winnowing", True)], "Winnowing")
    for tag, csv, schema, inputs, target in [("wn", "wn.csv", WN_SCHEMA, WN_INPUTS, "y"), ("wn100", "wn100.csv", WN_SCHEMA, WN_INPUTS, "y"),
                                            ("drugnoise", "drug_noise.csv", DRUG_NOISE_SCHEMA, DRUG_INPUTS + ["e1", "e2", "e3"], "Drug"),
                                            ("drughalf", "drug_half.csv", DRUG_SCHEMA, DRUG_INPUTS, "Drug"), ("drugtwice", "drug_twice.csv", DRUG_SCHEMA, DRUG_INPUTS, "Drug")]:
        for name, props in [("expert", [("mode", "Expert")]), ("winnow", [("mode", "Expert"), ("use_winnowing", True)]),
                            ("winnow_noglobal", [("mode", "Expert"), ("use_winnowing", True), ("use_global_pruning", False)]),
                            ("winnow_sev90", [("mode", "Expert"), ("use_winnowing", True), ("pruning_severity", 90)])]:
            add_case("c50_%s_%s" % (tag, name), "c50", csv, schema, [target], inputs, props, "Winnowing evidence", score_data=csv)
    add_case("c50_gw_expert", "c50", "gw_grid.csv", GW_SCHEMA, ["y"], GW_INPUTS, [("mode", "Expert")], "Weak noise fields, no winnowing", score_data="gw_grid.csv")
    add_case("c50_gw_winnow", "c50", "gw_grid.csv", GW_SCHEMA, ["y"], GW_INPUTS, [("mode", "Expert"), ("use_winnowing", True)], "Weak noise fields, winnowing", score_data="gw_grid.csv")

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
