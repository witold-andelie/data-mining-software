# Probe: QUEST, Random Trees, Decision List, Kohonen, Anomaly Detection,
# PCA/Factor, Apriori, CARMA, Sequence, and the current Time Series node (ts).
# Coordinator only: cargo xtask probe models_next_streams (Modeler 19, Jython 2.7).
# Writes generated inputs and one stream per case to corpus/models_next; public API only.
# A node the license does not provide fails that case into manifest["errors"]; later cases still run.
# Generators below are fixed loops. No random draws.
import json
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/models_next"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
SEED = 12345
NL = chr(10)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "models_next", "saved": [], "errors": [], "cases": [], "inputs": []}

# name, storage, measurement, explicit categorical domain (false before true for flags).
CLASS_SCHEMA = [
    ("id", "Integer", "Range", None),
    ("x", "Real", "Range", None),
    ("z", "Real", "Range", None),
    ("cat", "String", "Set", ["amber", "blue", "green", "red"]),
    ("flag", "String", "Flag", ["F", "T"]),
    ("ord", "String", "OrderedSet", ["low", "middle", "high"]),
    ("y", "String", "Set", ["A", "B", "C"]),
    ("binary", "String", "Flag", ["N", "Y"]),
]
CLASS_INPUTS = ["x", "z", "cat", "flag", "ord"]
NUM_SCHEMA = [
    ("id", "Integer", "Range", None),
    ("x1", "Real", "Range", None),
    ("x2", "Real", "Range", None),
    ("x3", "Real", "Range", None),
    ("x4", "Real", "Range", None),
    ("x5", "Real", "Range", None),
]
NUM_INPUTS = ["x1", "x2", "x3", "x4", "x5"]
ITEMS = ["bread", "milk", "eggs", "butter", "jam"]
BASKET_SCHEMA = [
    ("basket", "Integer", "Range", None),
    ("bread", "String", "Flag", ["F", "T"]),
    ("milk", "String", "Flag", ["F", "T"]),
    ("eggs", "String", "Flag", ["F", "T"]),
    ("butter", "String", "Flag", ["F", "T"]),
    ("jam", "String", "Flag", ["F", "T"]),
]
TRANS_SCHEMA = [
    ("basket", "Integer", "Range", None),
    ("item", "String", "Set", ["bread", "milk", "eggs", "butter", "jam"]),
]
SEQ_SCHEMA = [
    ("customer", "Integer", "Range", None),
    ("time", "Integer", "Range", None),
    ("item", "String", "Set", ["bread", "milk", "eggs", "butter", "jam"]),
]
SERIES_SCHEMA = [
    ("t", "Integer", "Range", None),
    ("sales", "Real", "Range", None),
    ("cost", "Real", "Range", None),
]
SERIES_TARGETS = ["sales", "cost"]
# Month index 0..11. Integers and halves only, so the CSV decimals are exact in binary.
SEASON = [0.0, 1.0, 2.0, 3.0, 2.0, 1.0, 0.0, -1.0, -2.0, -1.0, 0.0, 2.0]

BUILD_PROPERTIES = {
    "quest": ["use_max_depth", "max_depth", "prune_tree", "use_std_err", "std_err_multiplier",
              "max_surrogates", "split_alpha", "use_percentage", "min_parent_records_abs",
              "min_child_records_abs", "train_pct", "set_random_seed", "seed"],
    "randomtrees": ["number_of_models", "use_number_of_predictors", "number_of_predictors",
                    "use_stop_rule_for_accuracy", "sample_size", "handle_imbalanced_data",
                    "use_weighted_sampling", "max_node_number", "max_depth", "min_child_node_size",
                    "num_bins", "topN"],
    "decisionlist": ["search_direction", "target_value", "max_rules", "min_group_size",
                     "min_group_size_pct", "confidence_level", "max_segments_per_rule", "mode",
                     "bin_method", "bin_count"],
    "kohonen": ["set_random_seed", "random_seed", "stop_on", "time", "optimize", "mode",
                "width", "length", "decay_style", "phase1_neighborhood", "phase1_eta",
                "phase1_cycles", "phase2_neighborhood", "phase2_eta", "phase2_cycles",
                "continue", "show_feedback", "cluster_label"],
    "anomalydetection": ["mode", "anomaly_method", "index_level", "percent_records", "num_records",
                         "num_fields", "impute_missing_values", "adjustment_coeff",
                         "peer_group_num_auto", "min_num_peer_groups", "max_num_peer_groups",
                         "num_peer_groups", "noise_level", "noise_ratio"],
    "factor": ["method", "mode", "max_iterations", "complete_records", "matrix", "extract_factors",
               "min_eigenvalue", "max_factor", "rotation", "delta", "kappa", "sort_values",
               "hide_values", "hide_below"],
    "apriori": ["custom_fields", "consequents", "antecedents", "min_supp", "min_conf",
                "max_antecedents", "true_flags", "optimize", "use_transactional_data", "contiguous",
                "id_field", "content_field", "mode", "evaluation", "lower_bound"],
    "carma": ["custom_fields", "inputs", "id_field", "contiguous", "use_transactional_data",
              "content_field", "min_supp", "min_conf", "max_size", "mode", "exclude_multiple",
              "use_pruning", "pruning_value", "vary_support", "estimated_transactions",
              "rules_without_antecedents"],
    "sequence": ["id_field", "time_field", "use_time_field", "content_fields", "contiguous",
                 "min_supp", "min_conf", "max_size", "max_predictions", "mode", "use_max_duration",
                 "max_duration", "use_gaps", "min_item_gap", "max_item_gap", "use_pruning",
                 "pruning_value", "set_mem_sequences", "mem_sequences"],
    "ts": ["custom_fields", "targets", "candidate_inputs", "use_period", "date_time_field",
           "input_interval", "period_field", "is_same_interval", "method", "expert_modeler_method",
           "consider_seasonal", "exsmooth_model_type", "exsmooth_transformation_type",
           "arima.p", "arima.d", "arima.q", "arima.sp", "arima.sd", "arima.sq",
           "arima_transformation_type", "arima_include_constant", "forecastperiods",
           "extend_records_into_future", "conf_limits", "conf_limit_pct"],
}
APPLY_PROPERTIES = {
    "quest": ["calculate_conf", "display_rule_id", "calculate_raw_propensities",
              "calculate_adjusted_propensities"],
    "randomtrees": ["calculate_conf"],
    "decisionlist": ["calculate_raw_propensities", "calculate_adjusted_propensities"],
    "kohonen": [],
    "anomalydetection": ["anomaly_score_method", "num_fields", "discard_records",
                         "discard_anomalous_records"],
    "factor": [],
    "apriori": ["max_predictions", "allow_repeats", "check_basket", "criterion"],
    "carma": [],
    "sequence": [],
    "ts": ["extend_records_into_future", "ext_future_num", "compute_future_values_input",
           "forecastperiods", "noise_res", "conf_limits", "target_fields", "target_series",
           "includeTargets"],
}
# Field binding shared by the tabular Apriori cases. true_flags keeps the false flag
# value from becoming its own item. Algorithm thresholds stay at the node default.
APRIORI_TABULAR = [
    ("custom_fields", True),
    ("use_transactional_data", False),
    ("consequents", ITEMS),
    ("antecedents", ITEMS),
    ("true_flags", True),
]
APRIORI_TRANS = [
    ("custom_fields", True),
    ("use_transactional_data", True),
    ("id_field", "basket"),
    ("contiguous", True),
    ("content_field", "item"),
]
CARMA_TABULAR = [
    ("custom_fields", True),
    ("use_transactional_data", False),
    ("inputs", ITEMS),
]
SEQUENCE_FIELDS = [
    ("id_field", "customer"),
    ("contiguous", True),
    ("use_time_field", True),
    ("time_field", "time"),
    ("content_fields", ["item"]),
]
# Interval definition plus a six-step forecast. Method-specific cases add to this list.
# is_same_interval is read back but not set: the extracted guide does not say what True means.
TS_BASE = [
    ("custom_fields", True),
    ("targets", SERIES_TARGETS),
    ("use_period", True),
    ("period_field", "t"),
    ("input_interval", "Month"),
    ("forecastperiods", 6),
    ("extend_records_into_future", True),
    ("conf_limits", True),
]
TS_FORECAST = [("Forecasts", [
    ("extend_records_into_future", True),
    ("forecastperiods", 6),
    ("conf_limits", True),
])]


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
        common_build = [("use_model_name", True), ("model_name", "models_next")]
        # Guide examples document these seeds. randomtrees lists no seed property.
        if family == "kohonen":
            common_build += [("set_random_seed", True), ("random_seed", SEED)]
        if family == "quest":
            common_build += [("set_random_seed", True), ("seed", SEED)]
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


def class_targets(x, cat, flag):
    # Three-class rule uses x, flag, and cat. The flag target needs more than one segment:
    # red, or (flag true and x >= 3), or (green, flag false, x >= 5).
    if x < 3.0:
        y = "A" if flag == "F" else "B"
    elif cat == "amber" or cat == "blue":
        y = "B"
    else:
        y = "C"
    binary = "N"
    if cat == "red" or (flag == "T" and x >= 3.0) or (cat == "green" and flag == "F" and x >= 5.0):
        binary = "Y"
    return y, binary


def class_row(x, cat, flag, ord_value, z):
    y, binary = class_targets(x, cat, flag)
    return [0, x, z, cat, flag, ord_value, y, binary]


def numeric_fields(index):
    # x2 tracks x1; x4 tracks x3 with a half-weight of x1; x5 is a small independent step.
    x1 = float(index)
    x2 = 2.0 * x1 + 1.0
    x3 = float((index * 3) % 11)
    x4 = 3.0 * x3 + 0.5 * x1
    x5 = float(index % 5)
    return [x1, x2, x3, x4, x5]


def series_fields(index):
    # index is 0-based. t is the monthly period 1..36. Trend plus the SEASON table.
    t = index + 1
    season = SEASON[index % 12]
    sales = 20.0 + 0.5 * float(t) + season
    cost = 8.0 + 0.25 * float(t) + 0.5 * season
    return [t, sales, cost]


def basket_rows(presents):
    rows = []
    for basket_id, present in enumerate(presents):
        row = [basket_id + 1]
        for item in ITEMS:
            row.append("T" if item in present else "F")
        rows.append(row)
    return rows


def trans_rows(presents, first_id):
    rows = []
    for offset, present in enumerate(presents):
        basket_id = first_id + offset
        for item in ITEMS:
            if item in present:
                rows.append([basket_id, item])
    return rows


def declare_inputs():
    cats = ["amber", "blue", "green", "red"]
    ords = ["low", "middle", "high"]
    xs = [0.0, 1.0, 2.0, 3.0, 5.0, 7.0]
    flags = ["F", "T"]
    rows = []
    for xi, x in enumerate(xs):
        for ci, cat in enumerate(cats):
            for fi, flag in enumerate(flags):
                z = 0.5 * float(xi) + 0.25 * float(ci)
                row = class_row(x, cat, flag, ords[(xi + ci + fi) % 3], z)
                row[0] = len(rows) + 1
                rows.append(row)
    # Three labeled exceptions so pruning and a depth cap have impure nodes to cut.
    flipped = {"A": "B", "B": "A", "C": "B"}
    for index in [6, 17, 28]:
        rows[index][6] = flipped[rows[index][6]]
    # System-missing predictors only. Targets stay filled.
    rows[0][1] = None
    rows[4][2] = None
    rows[9][3] = None
    write_csv("class_train.csv", CLASS_SCHEMA, rows,
              "48 rows; 6 x by 4 cat by 2 flag; y from x/flag/cat with 3 flips; binary is a 3-segment flag rule; null x, null z, null cat")

    grid = []
    for x in [-1.0, 2.0, 3.0, 7.0, 12.0]:
        for cat in cats:
            for flag in flags:
                z = 0.5 * x
                row = class_row(x, cat, flag, ords[len(grid) % 3], z)
                row[0] = 10001 + len(grid)
                grid.append(row)
    edge = [list(grid[0]), list(grid[1]), list(grid[2])]
    edge[0][1] = None
    edge[1][2] = None
    edge[2][3] = None
    edge[0][6], edge[0][7] = "A", "N"
    edge[1][6], edge[1][7] = "A", "N"
    edge[2][6], edge[2][7] = "A", "N"
    for index, row in enumerate(edge):
        row[0] = 10001 + len(grid) + index
    write_csv("class_grid.csv", CLASS_SCHEMA, grid + edge,
              "43 rows; x outside and on the rule cuts, all cat and flag values, cycling ord, 3 missing predictors")

    numbers = []
    for index in range(36):
        numbers.append([index + 1] + numeric_fields(index))
    write_csv("numeric_train.csv", NUM_SCHEMA, numbers,
              "36 rows; x2=2*x1+1, x3=(3*i) mod 11, x4=3*x3+0.5*x1, x5=i mod 5; no nulls")
    number_grid = []
    for index in [0, 17, 35, 40]:
        number_grid.append([10001 + len(number_grid)] + numeric_fields(index))
    number_grid.append([10001 + len(number_grid), 0.0, 0.0, 0.0, 0.0, 0.0])
    number_grid.append([10001 + len(number_grid), 10.0, 10.0, 10.0, 10.0, 10.0])
    write_csv("numeric_grid.csv", NUM_SCHEMA, number_grid,
              "6 rows; four points on the training formulas (i=0, 17, 35, 40) and two off-pattern points")

    presents = [
        ["bread", "milk"],
        ["bread", "milk", "eggs"],
        ["bread", "milk"],
        ["bread", "milk", "eggs"],
        ["bread", "milk", "butter"],
        ["bread", "butter"],
        ["bread", "butter", "jam"],
        ["bread", "butter", "jam"],
        ["butter", "jam"],
        ["butter", "jam"],
        ["bread", "butter", "jam"],
        ["milk", "eggs"],
        ["bread", "milk", "eggs"],
        ["bread", "jam"],
        ["bread", "milk", "butter", "jam"],
        ["eggs", "milk"],
    ]
    write_csv("basket_train.csv", BASKET_SCHEMA, basket_rows(presents),
              "16 baskets; flag T means the item is in the basket; bread+milk and butter+jam repeat")
    write_csv("basket_trans.csv", TRANS_SCHEMA, trans_rows(presents, 1),
              "same 16 baskets, one row per item, basket ids contiguous and items in ITEMS order")
    grid_presents = [
        ["bread"],
        ["bread", "milk"],
        ["butter", "jam"],
        ["bread", "milk", "eggs", "butter", "jam"],
        [],
    ]
    write_csv("basket_grid.csv", BASKET_SCHEMA, basket_rows(grid_presents),
              "5 score baskets: bread, bread+milk, butter+jam, all five, empty")
    # An empty basket has no item rows, so the transactional score file omits it.
    write_csv("basket_trans_grid.csv", TRANS_SCHEMA, trans_rows(grid_presents[:-1], 101),
              "4 score baskets in transactional form, ids 101-104")

    patterns = []
    for _repeat in range(6):
        patterns.append(["bread", "milk", "eggs"])
    for _repeat in range(4):
        patterns.append(["bread", "butter", "jam"])
    for _repeat in range(2):
        patterns.append(["milk", "eggs"])
    sequence = []
    for customer, pattern in enumerate(patterns):
        for step, item in enumerate(pattern):
            sequence.append([customer + 1, step + 1, item])
    write_csv("sequence_train.csv", SEQ_SCHEMA, sequence,
              "12 customers; 6 bread-milk-eggs, 4 bread-butter-jam, 2 milk-eggs; time starts at 1")
    score_patterns = [
        ["bread", "milk"],
        ["bread", "butter"],
        ["milk"],
    ]
    sequence_score = []
    for customer, pattern in enumerate(score_patterns):
        for step, item in enumerate(pattern):
            sequence_score.append([101 + customer, step + 1, item])
    write_csv("sequence_score.csv", SEQ_SCHEMA, sequence_score,
              "3 partial customers 101-103 for next-item scoring")

    series = []
    for index in range(36):
        series.append(series_fields(index))
    write_csv("series_train.csv", SERIES_SCHEMA, series,
              "36 months; sales=20+0.5*t+SEASON, cost=8+0.25*t+0.5*SEASON; t=1..36; SEASON length 12")


def declare_cases():
    def quest(name, props, tests, **kwargs):
        add_case("quest_" + name, "quest", "class_train.csv", CLASS_SCHEMA, ["y"], CLASS_INPUTS,
                 props, tests, score_data="class_grid.csv", **kwargs)

    quest("default", [], "QUEST defaults; shared seed only, depth and pruning untouched")
    quest("depth3", [("use_max_depth", "Custom"), ("max_depth", 3)],
          "Custom maximum depth 3", compare_to="quest_default")
    quest("unpruned", [("prune_tree", False)],
          "Pruning off", compare_to="quest_default")

    def trees(name, props, tests, **kwargs):
        target = kwargs.pop("target", "y")
        add_case("rt_" + name, "randomtrees", "class_train.csv", CLASS_SCHEMA, [target],
                 CLASS_INPUTS, props, tests, score_data="class_grid.csv", **kwargs)

    trees("default", [], "Random Trees defaults; the guide lists no seed for this node")
    trees("fewer", [("number_of_models", 10)],
          "Ensemble size 10", compare_to="rt_default")
    trees("flag", [], "Flag target binary; ensemble size left at the default", target="binary",
          compare_to="rt_default")

    def lists(name, props, tests, **kwargs):
        add_case("dl_" + name, "decisionlist", "class_train.csv", CLASS_SCHEMA, ["binary"],
                 CLASS_INPUTS, props, tests, score_data="class_grid.csv", **kwargs)

    lists("default", [], "Decision List defaults on the flag target; true flag value is Y")
    lists("max_rules", [("max_rules", 2)],
          "At most 2 segments excluding the remainder", compare_to="dl_default")

    def kohonen(name, props, tests, **kwargs):
        add_case("kohonen_" + name, "kohonen", "numeric_train.csv", NUM_SCHEMA, [], NUM_INPUTS,
                 props, tests, score_data="numeric_grid.csv", **kwargs)

    kohonen("default", [], "Kohonen defaults; shared set_random_seed and random_seed 12345")
    kohonen("map", [("mode", "Expert"), ("width", 3), ("length", 5)],
            "Expert map width 3 and length 5", compare_to="kohonen_default")

    def anomaly(name, props, tests, **kwargs):
        add_case("anomaly_" + name, "anomalydetection", "class_train.csv", CLASS_SCHEMA, [],
                 CLASS_INPUTS, props, tests, score_data="class_grid.csv", **kwargs)

    anomaly("default", [], "Anomaly defaults on the mixed classification inputs, including 3 nulls")
    anomaly("cutoff", [("mode", "Expert"), ("anomaly_method", "IndexLevel"), ("index_level", 3.0),
                       ("num_fields", 2)],
            "Expert index cutoff 3 and 2 anomaly fields",
            score_variants=[("Two anomaly fields", [("num_fields", 2)])],
            compare_to="anomaly_default")

    def factor(name, props, tests, **kwargs):
        add_case("factor_" + name, "factor", "numeric_train.csv", NUM_SCHEMA, [], NUM_INPUTS,
                 props, tests, score_data="numeric_grid.csv", **kwargs)

    factor("default", [], "PCA/Factor defaults (principal components unless the node default differs)")
    factor("varimax", [("mode", "Expert"), ("rotation", "Varimax")],
           "Expert Varimax rotation; method left at the default", compare_to="factor_default")
    factor("nfactors", [("mode", "Expert"), ("extract_factors", "ByFactors"), ("max_factor", 2)],
           "Expert extraction of a fixed 2 factors", compare_to="factor_default")

    def apriori(name, props, tests, **kwargs):
        data = kwargs.pop("data", "basket_train.csv")
        schema = kwargs.pop("schema", BASKET_SCHEMA)
        inputs = kwargs.pop("inputs", ITEMS)
        grid = kwargs.pop("score_data", "basket_grid.csv")
        add_case("apriori_" + name, "apriori", data, schema, [], inputs, props, tests,
                 score_data=grid, **kwargs)

    apriori("default", APRIORI_TABULAR,
            "Tabular flags; true values only; support and confidence at the node default")
    apriori("thresholds", APRIORI_TABULAR + [("min_supp", 30.0), ("min_conf", 70.0)],
            "Minimum support 30 percent and minimum confidence 70 percent",
            compare_to="apriori_default")
    apriori("transactional", APRIORI_TRANS,
            "Same baskets in id/item form", data="basket_trans.csv", schema=TRANS_SCHEMA,
            inputs=["item"], score_data="basket_trans_grid.csv", compare_to="apriori_default")

    def carma(name, props, tests, **kwargs):
        add_case("carma_" + name, "carma", "basket_train.csv", BASKET_SCHEMA, [], ITEMS,
                 props, tests, score_data="basket_grid.csv", **kwargs)

    carma("default", CARMA_TABULAR, "Tabular flags; CARMA support and confidence at the node default")
    carma("minsupp", CARMA_TABULAR + [("min_supp", 40.0)],
          "Minimum rule support 40 percent", compare_to="carma_default")

    def sequence(name, props, tests, **kwargs):
        add_case("sequence_" + name, "sequence", "sequence_train.csv", SEQ_SCHEMA, [], ["item"],
                 props, tests, score_data="sequence_score.csv", **kwargs)

    sequence("default", SEQUENCE_FIELDS, "Sequence defaults; id, time, and one content field bound")
    sequence("minsupp", SEQUENCE_FIELDS + [("min_supp", 40.0)],
             "Minimum support 40 percent", compare_to="sequence_default")

    def timeseries(name, props, tests, **kwargs):
        add_case("ts_" + name, "ts", "series_train.csv", SERIES_SCHEMA, SERIES_TARGETS, [],
                 props, tests, score_variants=TS_FORECAST, **kwargs)

    timeseries("expert", TS_BASE + [("method", "ExpertModeler")],
               "Expert Modeler; monthly period field; 6 forecast steps with confidence limits")
    timeseries("exsmooth", TS_BASE + [("method", "Exsmooth"),
                                      ("exsmooth_model_type", "WintersAdditive"),
                                      ("exsmooth_transformation_type", "None")],
               "Winters additive exponential smoothing, no transformation",
               compare_to="ts_expert")
    timeseries("arima", TS_BASE + [("method", "Arima"),
                                   ("arima.p", 1), ("arima.d", 1), ("arima.q", 1),
                                   ("arima.sp", 0), ("arima.sd", 1), ("arima.sq", 1)],
               "Fixed ARIMA orders (1,1,1)(0,1,1)", compare_to="ts_expert")


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
