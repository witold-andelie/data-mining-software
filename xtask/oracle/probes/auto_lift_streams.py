# Probe: how Auto Classifier cuts the lift percentile when q % of the records is not a whole
# number (label auto_lift). Same ac_train.csv and components as auto_train/ac_lift; the lift
# percentile is 48, 52, 56, 57, 58, 61, 63, 78 or 80 of 360 records (172.8 .. 288). On Modeler's own ac_lift
# components the first-ranked model is Logistic or C5.0 depending on the rule: floor gives
# L C L C, ceiling C L C L, rounding C C C C, linear interpolation C C L L at 48/52/56/57;
# 58/61/63/78/80 were added after those four as a pre-registered confirmation.
# Derived from auto_train_streams.py (label auto_train). models_next2/ac_* and an_* ran Modeler 19's full default lists
# (xgboosttree, LSVM, Random Trees, Neural Net, Tree-AS with a 30 % overfit set) on data every
# component fitted perfectly, so they show neither the selection nor the ranking rule.
# Here the data carry deterministic label noise, every other algorithm is switched off by the
# scripting names auto_names_streams verified, and the ranking measure, the number of models
# kept and the limits vary one at a time.
# Coordinator only: cargo xtask probe auto_train_streams (Modeler 19, Jython 2.7).
# Writes generated inputs and one stream per case to corpus/auto_train; public API only.
# Generators are fixed loops. No random draws.
import json
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/auto_lift"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)
MODEL_NAME = "auto_lift"

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "auto_lift", "saved": [], "errors": [], "cases": [], "inputs": []}

CLASS_SCHEMA = [
    ("id", "Integer", "Range", None),
    ("age", "Real", "Range", None),
    ("income", "Real", "Range", None),
    ("region", "String", "Set", ["east", "north", "south", "west"]),
    ("member", "String", "Flag", ["N", "Y"]),
    ("visits", "Real", "Range", None),
    ("buy", "String", "Flag", ["N", "Y"]),
]
CLASS_INPUTS = ["age", "income", "region", "member", "visits"]
NUM_SCHEMA = [
    ("id", "Integer", "Range", None),
    ("x1", "Real", "Range", None),
    ("x2", "Real", "Range", None),
    ("x3", "Real", "Range", None),
    ("cat", "String", "Set", ["a", "b", "c"]),
    ("y", "Real", "Range", None),
]
NUM_INPUTS = ["x1", "x2", "x3", "cat"]

# Every component flag each node accepts (auto_names_streams); a case switches on its own list.
AC_ALGORITHMS = ["c50", "cart", "chaid", "decisionlist", "logreg", "lsvm", "neuralnetwork",
                 "quest", "randomtrees", "rf", "treeas", "xgboostlinear", "xgboosttree"]
AN_ALGORITHMS = ["cart", "chaid", "gle", "linear", "linearas", "lsvm", "neuralnetwork",
                 "randomtrees", "regression", "rf", "treeas", "xgboostas", "xgboostlinear",
                 "xgboosttree"]
AC_EXACT = ["c50", "chaid", "decisionlist", "logreg"]
AN_EXACT = ["chaid", "linear", "linearas", "regression"]

BUILD_PROPERTIES = {
    "autoclassifier": ["ranking_measure", "ranking_dataset", "number_of_models",
                       "calculate_variable_importance", "enable_accuracy_limit", "accuracy_limit",
                       "enable_area_under_curve_limit", "area_under_curve_limit",
                       "enable_profit_limit", "profit_limit", "enable_lift_limit", "lift_limit",
                       "enable_number_of_variables_limit", "number_of_variables_limit",
                       "use_fixed_cost", "fixed_cost", "use_fixed_revenue", "fixed_revenue",
                       "use_fixed_weight", "fixed_weight", "lift_percentile", "use_costs",
                       "use_partitioned_data"] + AC_ALGORITHMS,
    "autonumeric": ["ranking_measure", "ranking_dataset", "number_of_models",
                    "calculate_variable_importance", "enable_correlation_limit",
                    "correlation_limit", "enable_number_of_fields_limit",
                    "number_of_fields_limit", "enable_relative_error_limit",
                    "relative_error_limit", "use_partitioned_data"] + AN_ALGORITHMS,
}
APPLY_PROPERTIES = {
    "autoclassifier": ["flag_ensemble_method", "flag_evaluation_selection",
                       "filter_individual_model_output", "flag_voting_tie_selection",
                       "set_ensemble_method", "set_voting_tie_selection"],
    "autonumeric": ["calculate_standard_error"],
}
# Modeler's default tie rule for flag voting is Random, which no run can reproduce.
AC_APPLY = [("flag_voting_tie_selection", "HighestConfidence")]


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
        entry.setdefault("readback", []).append(
            [name, snapshot(node.getPropertyValue(name))])
    entry.pop("setting_in_progress", None)


def set_keyed(node, keyed, entry):
    for algorithm, name, value in keyed:
        record = [algorithm, name, snapshot(value)]
        try:
            node.setKeyedPropertyValue(algorithm, name, value)
            record.append(snapshot(node.getKeyedPropertyValue(algorithm, name)))
        except ERRORS as exc:
            # Recorded, not fatal: the saved stream shows whether the component changed.
            record.append({"error": message(exc)})
        entry.setdefault("keyed_readback", []).append(record)


def instantiate_node(stream, node):
    table = stream.createAt("table", "Instantiate", 1000, 800)
    stream.link(node, table)
    try:
        table.run([])
    finally:
        stream.delete(table)


def typed_source(stream, name, schema, targets, inputs, y_position, entry):
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
        source.setKeyedPropertyValue("custom_storage_type", field, storage)
        source_entry["storage"].append([field, storage])

    node = stream.createAt("type", "Type " + name, 260, y_position)
    stream.link(source, node)
    instantiate_node(stream, node)
    for field, storage, measure, values in schema:
        node.setKeyedPropertyValue("type", field, measure)
        if values is not None:
            node.setKeyedPropertyValue("values", field, values)
            node.setKeyedPropertyValue("extend_values", field, False)
        direction = "None"
        if field in targets:
            direction = "Target"
        elif field in inputs:
            direction = "Input"
        node.setKeyedPropertyValue("direction", field, direction)
        node.setKeyedPropertyValue("check", field, "None")
        if storage == "Real":
            node.setKeyedPropertyValue("export_places", field, 15)
    instantiate_node(stream, node)
    return node


def list_appliers(stream):
    found = []
    iterator = stream.iterator()
    while iterator.hasNext():
        node = iterator.next()
        if unicode(node.getTypeName()).startswith("apply"):
            found.append(node)
    return found


def add_case(name, family, data, score_data, schema, targets, inputs, enabled, props, tests,
             keyed=None):
    algorithms = AC_ALGORITHMS if family == "autoclassifier" else AN_ALGORITHMS
    switches = [(algorithm, algorithm in enabled) for algorithm in algorithms]
    entry = {"case": name, "file": name + ".str", "family": family, "data": data,
             "score_data": score_data, "schema": schema, "targets": targets, "inputs": inputs,
             "enabled": enabled, "properties": switches + props, "keyed": keyed or [],
             "tests": tests, "status": "planned"}
    manifest["cases"].append(entry)


def build_case(entry):
    stream = None
    stage = "create_stream"
    family = entry["family"]
    apply_props = AC_APPLY if family == "autoclassifier" else []
    try:
        stream = taskrunner.createStream(entry["case"], False, False)
        common_stream = [("decimal_places", 15), ("decimal_symbol", "Period"),
                         ("enable_parallelism", False),
                         ("create_model_applier_for_new_models", True),
                         ("create_model_applier_update_links", "createEnabled")]
        set_properties(stream, common_stream, entry)
        stage = "training_type"
        schema = entry["schema"]
        training = typed_source(stream, entry["data"], schema, entry["targets"],
                                entry["inputs"], 100, entry)
        stage = "build_properties"
        builder = stream.createAt(family, entry["case"], 480, 100)
        stream.link(training, builder)
        entry["build_type"] = unicode(builder.getTypeName())
        entry["builder_defaults"] = read_properties(builder, BUILD_PROPERTIES[family])
        common_build = [("use_model_name", True), ("model_name", MODEL_NAME)]
        set_properties(builder, common_build + entry["properties"], entry)
        set_keyed(builder, entry["keyed"], entry)
        entry["builder_effective"] = read_properties(builder, BUILD_PROPERTIES[family])
        stage = "train"
        results = []
        builder.run(results)
        appliers = list_appliers(stream)
        entry["placed_nugget_types"] = [unicode(node.getTypeName()) for node in appliers]
        applier = None
        for node in appliers:
            if family in unicode(node.getTypeName()):
                applier = node
                break
        models = [result for result in results if isinstance(result, modeler.api.ModelOutput)]
        if not models:
            raise RuntimeError("builder returned no ModelOutput")
        entry["model_output_count"] = len(models)
        if applier is None:
            applier = stream.createModelApplierAt(models[0], "Train scores", 480, 260)
            stream.link(training, applier)
        else:
            applier.setXYPosition(480, 260)
        # Component nuggets the builder parked beside the ensemble would be extra terminals.
        for node in appliers:
            if node is not applier and unicode(node.getID()) != unicode(applier.getID()):
                stream.delete(node)
        entry["applier_type"] = unicode(applier.getTypeName())
        entry["applier_defaults"] = read_properties(applier, APPLY_PROPERTIES[family])
        terminals = []
        entry["terminals"] = []

        def attach(nugget, label, position):
            settings = {"label": label, "properties": apply_props}
            set_properties(nugget, apply_props, settings)
            settings["effective"] = read_properties(nugget, APPLY_PROPERTIES[family])
            table = stream.createAt("table", label, 760, position)
            stream.link(nugget, table)
            settings["applier_id"] = unicode(nugget.getID())
            settings["table_id"] = unicode(table.getID())
            entry["terminals"].append(settings)
            terminals.append(table)

        attach(applier, "Train scores", 260)
        stage = "scoring_type"
        scoring = typed_source(stream, entry["score_data"], schema, entry["targets"],
                               entry["inputs"], 420, entry)
        nugget = stream.createModelApplierAt(models[0], "Grid scores", 480, 420)
        stream.link(scoring, nugget)
        attach(nugget, "Grid scores", 420)

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


REGIONS = ["east", "north", "south", "west"]
REGION_EFFECT = {"east": 0.1, "north": 0.6, "south": -0.4, "west": -0.3}
CATS = ["a", "b", "c"]
CAT_EFFECT = {"a": 0.0, "b": 4.0, "c": -2.0}


def class_row(i):
    age = 20.0 + float((i * 7) % 50)
    income = 20.0 + float((i * 13) % 80)
    region = REGIONS[(i * 3) % 4]
    member = ["N", "Y"][(i // 3) % 2]
    visits = float((i * 11) % 9)
    score = (0.06 * (age - 45.0) + 0.04 * (income - 60.0)
             + (0.8 if member == "Y" else -0.5) + REGION_EFFECT[region]
             + 0.15 * (visits - 4.0))
    # Deterministic label noise in [-1, 1]: about one row in six lands on the other side.
    noise = float((i * 37) % 17 - 8) / 8.0
    buy = "Y" if score + noise > 0.0 else "N"
    return [i + 1, age, income, region, member, visits, buy]


def num_row(i):
    x1 = float((i * 7) % 31) / 3.0
    x2 = float((i * 11) % 23)
    x3 = float((i * 5) % 13) - 6.0
    cat = CATS[(i * 2) % 3]
    noise = float((i * 29) % 11 - 5) * 0.3
    y = 3.0 + 1.5 * x1 - 0.2 * x2 + 0.05 * x3 * x3 + CAT_EFFECT[cat] + noise
    return [i + 1, x1, x2, x3, cat, y]


def declare_inputs():
    rows = [class_row(i) for i in range(360)]
    yes = len([row for row in rows if row[6] == "Y"])
    if yes < 100 or yes > 260:
        raise ValueError("class balance drifted: %d Y" % yes)
    write_csv("ac_train.csv", CLASS_SCHEMA, rows,
              "360 rows; buy from a linear score in age, income, member, region, visits plus fixed label noise")
    grid = []
    for k in range(48):
        row = class_row(1000 + 17 * k)
        row[0] = 10001 + k
        grid.append(row)
    write_csv("ac_grid.csv", CLASS_SCHEMA, grid, "48 rows from the same generator at other indices")

    numbers = [num_row(i) for i in range(240)]
    write_csv("an_train.csv", NUM_SCHEMA, numbers,
              "240 rows; y = 3 + 1.5 x1 - 0.2 x2 + 0.05 x3^2 + cat effect + fixed noise")
    number_grid = []
    for k in range(30):
        row = num_row(500 + 13 * k)
        row[0] = 10001 + k
        number_grid.append(row)
    write_csv("an_grid.csv", NUM_SCHEMA, number_grid, "30 rows from the same generator")


def declare_cases():
    for q in [48, 52, 56, 57, 58, 61, 63, 78, 80]:
        add_case("ac_lift_q%d" % q, "autoclassifier", "ac_train.csv", "ac_grid.csv",
                 CLASS_SCHEMA, ["buy"], CLASS_INPUTS, AC_EXACT,
                 [("lift_percentile", q), ("number_of_models", 4)],
                 "Lift at %d %% of 360 records (%.1f), all four components kept in rank order"
                 % (q, q * 3.6))


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
