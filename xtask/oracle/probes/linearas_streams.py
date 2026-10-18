# Probe: Linear-AS model selection on noisy, non-degenerate data (label linearas).
# Coordinator only: cargo xtask probe linearas_streams (Modeler 19, Jython 2.7).
# Writes generated inputs and one stream per case to corpus/linearas; public API only.
# models_as/linearas_default fits y exactly, so whether a squared term enters after the five main
# effects is floating-point noise. Here y has a real squared term, a weak input, an input without
# effect and a fixed deterministic residual, so each selection step has a clear winner.
# Algorithms Guide, Linear-AS: squared terms of all covariates are candidates (section 5), then
# forward stepwise on adjusted R squared (section 6.2).
# Only the node defaults are covered: in Modeler 19 neither the Table 137 names
# (var_select_method, detect_2way_interaction, criteria_for_forward_stepwise,
# use_custom_max_effects) nor the saved element names (_varSelectionMethod, ...) can be set
# from a script ("The property ... is undefined", first run of this probe, 2026-10-19).
# Generators below are fixed loops. No random draws.
import json
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/linearas"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)
MODEL_NAME = "linearas"

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "linearas", "saved": [], "errors": [], "cases": [], "inputs": []}

NUM_SCHEMA = [
    ("id", "Integer", "Range", None),
    ("x1", "Real", "Range", None),
    ("x2", "Real", "Range", None),
    ("x3", "Real", "Range", None),
    ("x4", "Real", "Range", None),
    ("x5", "Real", "Range", None),
    ("y", "Real", "Range", None),
]
NUM_INPUTS = ["x1", "x2", "x3", "x4", "x5"]
FACTOR_SCHEMA = [
    ("id", "Integer", "Range", None),
    ("grp", "String", "Set", ["a", "b", "c"]),
    ("x1", "Real", "Range", None),
    ("x2", "Real", "Range", None),
    ("y", "Real", "Range", None),
]
FACTOR_INPUTS = ["grp", "x1", "x2"]

BUILD_PROPERTIES = {
    "linearasnode": ["target", "inputs", "custom_fields", "intercept", "factor_order",
                     "max_effects", "max_steps"],
}
APPLY_PROPERTIES = {
    "linearasnode": ["enable_sql_generation"],
}


def linear_fields(target, inputs):
    # Table 137 puts "The default value is TRUE" on the custom_fields row. Repeat the Type
    # roles on the builder so a true default does not drop the target. Fresh list each case.
    return [("custom_fields", True), ("target", target), ("inputs", list(inputs))]


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


def list_appliers(stream):
    found = []
    iterator = stream.iterator()
    while iterator.hasNext():
        node = iterator.next()
        if unicode(node.getTypeName()).startswith("apply"):
            found.append(node)
    return found


def find_applier(stream):
    found = list_appliers(stream)
    if len(found) > 1:
        raise RuntimeError("unexpected multiple automatically placed nuggets")
    if found:
        return found[0]
    return None


def add_case(name, family, data, schema, targets, inputs, props, tests,
             score_data=None, missing=None, roles=None, stream_props=None,
             score_variants=None, compare_to=None, apply_properties=None):
    entry = {"case": name, "file": name + ".str", "family": family, "data": data,
             "schema": schema, "targets": targets, "inputs": inputs,
             "properties": props, "tests": tests,
             "score_data": score_data, "missing": missing or [], "roles": roles or {},
             "stream_properties": stream_props or [], "score_variants": score_variants or [],
             "compare_to": compare_to, "apply_properties": apply_properties or [],
             "status": "planned"}
    manifest["cases"].append(entry)


TYPE_NAMES = {
    "linearasnode": ["linearas", "linearasnode"],
}


def build_case(entry):
    stream = None
    stage = "create_stream"
    family = entry["family"]
    entry["score_ok"] = None
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
        builder = None
        tried = []
        # The guide gives no create example for these two: try the plausible type names.
        for type_name in TYPE_NAMES.get(family, [family]):
            try:
                builder = stream.createAt(type_name, entry["case"], 480, 100)
                entry["created_as"] = type_name
                break
            except ERRORS as exc:
                tried.append(type_name + ": " + message(exc))
        if builder is None:
            raise RuntimeError("cannot create " + family + " (" + "; ".join(tried) + ")")
        stream.link(training, builder)
        entry["build_type"] = unicode(builder.getTypeName())
        entry["builder_defaults"] = read_properties(builder, BUILD_PROPERTIES[family])
        # Identical model names across repeated cases: case labels must not perturb training.
        # Tables 143, 152, 137, and 139 list no seed property, so none is set.
        common_build = [("use_model_name", True), ("model_name", MODEL_NAME)]
        entry["common_build_properties"] = common_build
        set_properties(builder, common_build + entry["properties"], entry)
        entry["builder_effective"] = read_properties(builder, BUILD_PROPERTIES[family])
        stage = "train"
        results = []
        builder.run(results)
        appliers = list_appliers(stream)
        entry["placed_nugget_types"] = [unicode(node.getTypeName()) for node in appliers]
        applier = find_applier(stream)
        entry["auto_placed_applier"] = applier is not None
        models = [result for result in results if isinstance(result, modeler.api.ModelOutput)]
        if not models:
            raise RuntimeError("builder returned no ModelOutput for saved scoring nuggets")
        entry["model_output_count"] = len(models)
        if applier is None:
            applier = stream.createModelApplierAt(models[0], "Train scores", 480, 260)
            stream.link(training, applier)
            entry["placed_nugget_types"] = [unicode(applier.getTypeName())]
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
            settings["applier_type"] = unicode(nugget.getTypeName())
            table = stream.createAt("table", label, 760, position)
            stream.link(nugget, table)
            settings["applier_id"] = unicode(nugget.getID())
            settings["table_id"] = unicode(table.getID())
            entry["terminals"].append(settings)
            terminals.append(table)

        apply_props = entry["apply_properties"]
        attach(applier, "Train scores", 260, apply_props)
        scoring = training
        stage = "scoring_type"
        if entry["score_data"] is not None:
            scoring = typed_source(stream, entry["score_data"], schema, entry["targets"],
                                   entry["inputs"], entry["roles"], entry["missing"], 420, entry)
        variants = [("Grid scores", apply_props)] + entry["score_variants"]
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
        entry["score_ok"] = True
        entry["score_failed_tables"] = []
        stage = "score"
        for table in terminals:
            try:
                table.run([])
            except ERRORS as exc:
                entry["score_failed_tables"].append(unicode(table.getLabel()))
                entry["score_ok"] = False
                manifest["errors"].append({"case": entry["case"], "stage": "score",
                                           "table": unicode(table.getLabel()),
                                           "message": message(exc)})
                entry["status"] = "saved_with_score_errors"
    except ERRORS as exc:
        entry["status"] = "failed"
        entry["score_ok"] = False
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




def numeric_fields(index):
    # Each column cycles with its own period; none is an affine copy of another.
    x1 = float(index % 15) + 0.5 * float(index % 4)
    x2 = 0.8 * float((index * 7) % 11) + 0.1 * float(index % 3)
    x3 = float((index * 5) % 13) - 6.0 + 0.25 * float(index % 2)
    x4 = 0.3 * float((index * 11) % 17)
    x5 = 0.2 * float((index * 13) % 19) + 0.05 * float(index % 5)
    return [x1, x2, x3, x4, x5]


def residual(index):
    # Fixed deterministic residual in [-1.95, 1.95].
    return 0.15 * float((index * 37) % 23 - 11) + 0.1 * float((index * 17) % 7 - 3)


def numeric_target(fields, index):
    x1, x2, x3, x4, x5 = fields
    return 3.0 + 1.5 * x1 + 0.02 * x2 + 0.4 * x3 * x3 + 0.8 * x4 + residual(index)


GROUP_SHIFT = {"a": 0.0, "b": 1.5, "c": -1.0}


def factor_row(index):
    grp = ["a", "b", "c"][(index * 2) % 3]
    x1 = float(index % 15) + 0.5 * float(index % 4)
    x2 = 0.8 * float((index * 7) % 11) + 0.1 * float(index % 3)
    slope = 1.5
    if grp == "b":
        slope = 2.1
    y = 2.0 + GROUP_SHIFT[grp] + slope * x1 + 0.3 * x2 + residual(index)
    return [index + 1, grp, x1, x2, y]


def declare_inputs():
    numbers = []
    for index in range(120):
        fields = numeric_fields(index)
        numbers.append([index + 1] + fields + [numeric_target(fields, index)])
    write_csv("noisy_train.csv", NUM_SCHEMA, numbers,
              "120 rows; y = 3 + 1.5*x1 + 0.02*x2 + 0.4*x3^2 + 0.8*x4 + residual(i); x5 has no effect")
    grid = []
    for index in [0, 7, 59, 119, 130]:
        fields = numeric_fields(index)
        grid.append([10001 + len(grid)] + fields + [numeric_target(fields, index)])
    for fields in ([0.0, 0.0, 0.0, 0.0, 0.0], [20.0, -3.0, 9.0, 6.0, -2.0]):
        grid.append([10001 + len(grid)] + fields + [0.0])
    write_csv("noisy_grid.csv", NUM_SCHEMA, grid,
              "7 rows; five formula points (i=0, 7, 59, 119, 130) and two vectors outside the training ranges")
    factors = [factor_row(index) for index in range(90)]
    write_csv("factor_train.csv", FACTOR_SCHEMA, factors,
              "90 rows; y = 2 + shift(grp) + slope(grp)*x1 + 0.3*x2 + residual(i); slope 2.1 for grp b, else 1.5")
    factor_grid = []
    for grp, x1, x2 in [("a", 0.0, 0.0), ("b", 5.0, 2.0), ("c", 12.0, 7.0), ("b", 20.0, -1.0)]:
        factor_grid.append([20001 + len(factor_grid), grp, x1, x2, 0.0])
    write_csv("factor_grid.csv", FACTOR_SCHEMA, factor_grid,
              "4 rows; one per group plus x1 outside training for group b")


def declare_cases():
    def noisy(name, settings, tests):
        add_case("lin_" + name, "linearasnode", "noisy_train.csv", NUM_SCHEMA, ["y"],
                 NUM_INPUTS, linear_fields("y", NUM_INPUTS) + settings, tests,
                 score_data="noisy_grid.csv")

    noisy("default", [],
          "Node defaults: ForwardStepwise on AdjustedRSquare, two-way interaction detection on (saved defaults in models_as)")
    add_case("lin_factor", "linearasnode", "factor_train.csv", FACTOR_SCHEMA, ["y"],
             FACTOR_INPUTS, linear_fields("y", FACTOR_INPUTS), "Factor grp with a grp-by-x1 slope difference: covariate-by-factor interaction test (section 5.2)",
             score_data="factor_grid.csv")


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
