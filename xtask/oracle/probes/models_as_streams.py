# Probe: Random Trees, Tree-AS, Linear-AS, and LSVM (Analytic Engine models).
# Coordinator only: cargo xtask probe models_as_streams (Modeler 19, Jython 2.7).
# Writes generated inputs and one stream per case to corpus/models_as; public API only.
# A node the license does not provide fails that case into manifest["errors"]; later cases still run.
# score_ok is true only when every scoring Table ran. A Table that throws is also listed under
# errors with stage "score" and that table's label. The stream is saved before those Tables run.
# Generators below are fixed loops. No random draws.
# Data generators are the models_next2 segment set (519 rows, flag target respond) and numeric set.
import json
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/models_as"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)
MODEL_NAME = "models_as"

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "models_as", "saved": [], "errors": [], "cases": [], "inputs": []}

# name, storage, measurement, explicit categorical domain (false before true for flags).
SEGMENT_SCHEMA = [
    ("id", "Integer", "Range", None),
    ("region", "String", "Set", ["east", "north", "south", "west"]),
    ("channel", "String", "Set", ["phone", "store", "web"]),
    ("tier", "String", "Set", ["bronze", "gold", "silver"]),
    ("spend", "Real", "Range", None),
    ("visits", "Real", "Range", None),
    ("respond", "String", "Flag", ["N", "Y"]),
]
SEGMENT_INPUTS = ["region", "channel", "tier", "spend", "visits"]
SEGMENT_SPENDS = [10.0, 40.0, 90.0]
SEGMENT_VISITS = [1.0, 4.0, 8.0]
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

# Names below are the scripting-guide rows. A few identifiers are split across lines in the
# extracted guide; those are marked and are only read, never set.
# randomtrees Table 143 (lines 15370-15482): no seed property. The table ends at topN.
# treeas Table 152 (lines 16738-16826): no seed property.
# linearasnode Table 137 (lines 14539-14603): no seed property.
# lsvmnode Table 139 (lines 14923-14968): no seed property.
BUILD_PROPERTIES = {
    "randomtrees": ["target", "number_of_models", "use_number_of_predictors", "number_of_predictors",
                    "use_stop_rule_for_accuracy", "sample_size", "handle_imbalanced_data",
                    "use_weighted_sampling", "max_node_number", "max_depth", "min_child_node_size",
                    "use_costs", "default_cost_increase", "max_pct_missing",
                    "exclude_single_cat_pct", "max_category_number", "min_field_variation",
                    "num_bins", "topN"],
    "treeas": ["target", "method", "max_depth", "num_bins", "record_threshold", "split_alpha",
               "merge_alpha", "bonferroni_adjustment",
               "effect_size_threshold_cont", "effect_size_threshold_cat",
               "split_merged_categories", "grouping_sig_level", "chi_square", "minimum_record_use",
               "min_parent_records_pc", "min_child_records_pc", "min_parent_records_abs",
               "min_child_records_abs", "epsilon", "max_iterations", "use_costs",
               "default_cost_increase", "calculate_conf", "display_rule_id"],
    "linearasnode": ["target", "inputs", "weight_field", "custom_fields", "intercept",
                     "detect_2way_interaction", "cin", "factor_order", "var_select_method",
                     "criteria_for_forward_stepwise", "pin", "pout", "use_custom_max_effects",
                     "max_effects", "use_custom_max_steps", "max_steps", "criteria_for_best_subsets"],
    "lsvmnode": ["intercept", "target_order", "precision", "exclude_missing_values",
                 "penalty_function", "lambda", "calculate_variable_importance"],
}
# applyrandomtrees Table 182, applytreeas Table 189, applylinearasnode Table 175,
# applylsvmnode Table 177. enable_sql_generation udf uses the Modeler Server scoring adapter;
# native scores inside Modeler. applylinearasnode states the default is udf (line 17705).
APPLY_PROPERTIES = {
    "randomtrees": ["calculate_conf", "enable_sql_generation"],
    "treeas": ["calculate_conf", "display_rule_id", "enable_sql_generation"],
    "linearasnode": ["enable_sql_generation"],
    "lsvmnode": ["calculate_raw_propensities", "enable_sql_generation"],
}
# models_next rt_default read these back: number_of_models 100, max_depth 10,
# min_child_node_size 5. Both changed values are documented integers (Table 143).
RT_FEWER = [("number_of_models", 10)]
RT_SHALLOW = [("max_depth", 3), ("min_child_node_size", 10)]
RT_BATCH_BUILD = [("number_of_models", 10), ("max_depth", 3), ("min_child_node_size", 10)]
RT_CONF = [("calculate_conf", True)]
RT_NOCONF = [("calculate_conf", False)]
RT_NATIVE = [("enable_sql_generation", "native")]
RT_BATCH_APPLY = [("calculate_conf", False), ("enable_sql_generation", "native")]


def linear_fields():
    # Table 137 puts "The default value is TRUE" on the custom_fields row. Repeat the Type
    # roles on the builder so a true default does not drop the target. Fresh list each case.
    return [("custom_fields", True), ("target", "y"),
            ("inputs", ["x1", "x2", "x3", "x4", "x5"])]


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
    "linearasnode": ["linearas", "linearasnode", "linear-as", "linearAS"],
    "lsvmnode": ["lsvm", "lsvmnode", "linearsvm"],
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
        # Auto nodes can place the ensemble nugget and component nuggets together.
        # Record every getTypeName(), and score the nugget whose type contains the family.
        if family.startswith("auto") and len(appliers) > 1:
            applier = appliers[-1]
            for node in appliers:
                if family in unicode(node.getTypeName()):
                    applier = node
                    break
        else:
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


def respond_value(region, channel, tier):
    # Three disjoint pure cells. Each hot value occurs only inside its own cell,
    # so a one-field rule covers that cell and no cold row.
    if region == "north" or channel == "web" or tier == "gold":
        return "Y"
    return "N"


def segment_row(region, channel, tier, spend, visit):
    return [0, region, channel, tier, spend, visit, respond_value(region, channel, tier)]


def numeric_fields(index):
    # Shared trend plus a different modular residual on every column.
    # The residuals stop an exact linear relation such as x2 = 2*x1 + 1.
    shared = float(index)
    x1 = shared + float((index * 3) % 7)
    x2 = 0.5 * shared + float((index * 5) % 11)
    x3 = 0.25 * shared + float((index * 2) % 5)
    x4 = float((index * 7) % 13) + 0.5 * float(index % 3)
    x5 = float((index * 4) % 9) + 0.25 * float((index * 3) % 5)
    return [x1, x2, x3, x4, x5]


def numeric_target(fields):
    x1, x2, x3, x4, x5 = fields
    return 2.0 * x1 - 0.5 * x2 + 0.25 * x3 + x4 - 0.5 * x5


def declare_inputs():
    segments = []
    # north only, and never web or gold, so region=north is a pure one-field cell of 80.
    for index in range(80):
        channel = ["phone", "store"][index % 2]
        tier = ["bronze", "silver"][(index // 2) % 2]
        spend = SEGMENT_SPENDS[index % 3]
        visit = SEGMENT_VISITS[(index // 3) % 3]
        segments.append(segment_row("north", channel, tier, spend, visit))
    # web only, never north or gold: channel=web is a pure cell of 60.
    for index in range(60):
        region = ["east", "south", "west"][index % 3]
        tier = ["bronze", "silver"][(index // 3) % 2]
        spend = SEGMENT_SPENDS[index % 3]
        visit = SEGMENT_VISITS[(index // 3) % 3]
        segments.append(segment_row(region, "web", tier, spend, visit))
    # gold only, never north or web: tier=gold is a pure cell of 55.
    for index in range(55):
        region = ["east", "south", "west"][index % 3]
        channel = ["phone", "store"][(index // 3) % 2]
        spend = SEGMENT_SPENDS[index % 3]
        visit = SEGMENT_VISITS[(index // 3) % 3]
        segments.append(segment_row(region, channel, "gold", spend, visit))
    cold_regions = ["east", "south", "west"]
    cold_channels = ["phone", "store"]
    cold_tiers = ["bronze", "silver"]
    for _repeat in range(3):
        for region in cold_regions:
            for channel in cold_channels:
                for tier in cold_tiers:
                    for spend in SEGMENT_SPENDS:
                        for visit in SEGMENT_VISITS:
                            segments.append(segment_row(region, channel, tier, spend, visit))
    north_n = 0
    web_n = 0
    gold_n = 0
    for row in segments:
        if row[1] == "north":
            north_n += 1
        if row[2] == "web":
            web_n += 1
        if row[3] == "gold":
            gold_n += 1
        hot = row[1] == "north" or row[2] == "web" or row[3] == "gold"
        if hot and row[6] != "Y":
            raise ValueError("hot segment row is not Y")
        if (not hot) and row[6] != "N":
            raise ValueError("cold segment row is not N")
    if north_n != 80 or web_n != 60 or gold_n != 55 or len(segments) != 519:
        raise ValueError("segment sizes drifted")
    for index, row in enumerate(segments):
        row[0] = index + 1
    write_csv("segment_train.csv", SEGMENT_SCHEMA, segments,
              "519 rows; Y on region=north (80), channel=web (60), tier=gold (55); those values occur in no other row; 324 cold rows")

    grid_spec = [
        ("north", "phone", "bronze", 10.0, 1.0),
        ("north", "store", "silver", 40.0, 4.0),
        ("north", "phone", "silver", 90.0, 8.0),
        ("north", "web", "bronze", 40.0, 4.0),
        ("east", "web", "bronze", 10.0, 1.0),
        ("south", "web", "silver", 40.0, 4.0),
        ("west", "web", "bronze", 90.0, 8.0),
        ("east", "web", "gold", 40.0, 4.0),
        ("east", "phone", "gold", 10.0, 1.0),
        ("south", "store", "gold", 90.0, 8.0),
        ("west", "phone", "gold", 40.0, 4.0),
        ("east", "phone", "bronze", 10.0, 1.0),
        ("south", "store", "silver", 40.0, 4.0),
        ("west", "phone", "silver", 90.0, 8.0),
        ("west", "store", "bronze", 10.0, 1.0),
        ("east", "store", "bronze", 200.0, 12.0),
        ("north", "phone", "bronze", 200.0, 12.0),
        ("south", "store", "gold", 200.0, 0.5),
    ]
    segment_grid = []
    for spec in grid_spec:
        row = segment_row(spec[0], spec[1], spec[2], spec[3], spec[4])
        row[0] = 10001 + len(segment_grid)
        segment_grid.append(row)
    write_csv("segment_grid.csv", SEGMENT_SCHEMA, segment_grid,
              str(len(segment_grid)) + " rows; each pure cell, cross-cell rows, cold rows, and numeric values outside training")

    numbers = []
    for index in range(48):
        fields = numeric_fields(index)
        numbers.append([index + 1] + fields + [numeric_target(fields)])
    # Row layout is id, x1, x2, x3, x4, x5, y. x2 must not be an affine copy of x1.
    residual_0 = numbers[0][2] - 0.5 * numbers[0][1]
    residual_1 = numbers[1][2] - 0.5 * numbers[1][1]
    if residual_0 == residual_1:
        raise ValueError("x2 tracks half of x1")
    write_csv("numeric_train.csv", NUM_SCHEMA, numbers,
              "48 rows; five inputs share a trend but each adds its own modular residual; y = 2*x1 - 0.5*x2 + 0.25*x3 + x4 - 0.5*x5")
    number_grid = []
    for index in [0, 23, 47, 60]:
        fields = numeric_fields(index)
        number_grid.append([10001 + len(number_grid)] + fields + [numeric_target(fields)])
    for fields in ([0.0, 0.0, 0.0, 0.0, 0.0], [12.0, 3.0, 8.0, 1.0, 5.0]):
        number_grid.append([10001 + len(number_grid)] + fields + [numeric_target(fields)])
    write_csv("numeric_grid.csv", NUM_SCHEMA, number_grid,
              "6 rows; four formula points (i=0, 23, 47, 60) and two off-pattern x vectors with y still on the formula")


def declare_cases():
    def trees(name, props, tests, **kwargs):
        target = kwargs.pop("target", "respond")
        data = kwargs.pop("data", "segment_train.csv")
        schema = kwargs.pop("schema", SEGMENT_SCHEMA)
        inputs = kwargs.pop("inputs", SEGMENT_INPUTS)
        grid = kwargs.pop("score_data", "segment_grid.csv")
        add_case("rt_" + name, "randomtrees", data, schema, [target], inputs, props, tests,
                 score_data=grid, **kwargs)

    # Flag target. models_next rt_default / rt_fewer / rt_flag built a nugget and then both
    # Tables failed at stage score with "Error: internal error occurred during job execution".
    trees("default", [],
          "Random Trees defaults on flag target respond. Table 143 lists no seed")
    trees("fewer", RT_FEWER,
          "number_of_models 10. Table 143 integer; models_next read the default back as 100",
          compare_to="rt_default")
    trees("shallow", RT_SHALLOW,
          "max_depth 3 and min_child_node_size 10. Table 143 integers; models_next read 10 and 5",
          compare_to="rt_default")
    trees("native", [],
          "Applier enable_sql_generation native. Table 182: native scores inside Modeler; udf uses the scoring adapter",
          apply_properties=RT_NATIVE, compare_to="rt_default")
    trees("conf", [],
          "Applier calculate_conf true, set explicitly. Table 182 flag",
          apply_properties=RT_CONF, compare_to="rt_default")
    trees("noconf", [],
          "Applier calculate_conf false. Table 182 flag. models_next left this at the default true and scoring failed",
          apply_properties=RT_NOCONF, compare_to="rt_default")
    trees("batch", RT_BATCH_BUILD,
          "Combined: number_of_models 10, max_depth 3, min_child_node_size 10, calculate_conf false, enable_sql_generation native",
          apply_properties=RT_BATCH_APPLY, compare_to="rt_default")
    trees("numeric", [],
          "Random Trees defaults on continuous target y. Table 143 allows a numeric-range target. No seed is documented",
          target="y", data="numeric_train.csv", schema=NUM_SCHEMA, inputs=NUM_INPUTS,
          score_data="numeric_grid.csv")
    trees("numeric_batch", RT_BATCH_BUILD,
          "Same combined batch settings as rt_batch, on continuous target y",
          apply_properties=RT_BATCH_APPLY, compare_to="rt_numeric",
          target="y", data="numeric_train.csv", schema=NUM_SCHEMA, inputs=NUM_INPUTS,
          score_data="numeric_grid.csv")

    def treeas(name, props, tests, **kwargs):
        target = kwargs.pop("target", "respond")
        data = kwargs.pop("data", "segment_train.csv")
        schema = kwargs.pop("schema", SEGMENT_SCHEMA)
        inputs = kwargs.pop("inputs", SEGMENT_INPUTS)
        grid = kwargs.pop("score_data", "segment_grid.csv")
        add_case("treeas_" + name, "treeas", data, schema, [target], inputs, props, tests,
                 score_data=grid, **kwargs)

    treeas("default", [],
           "Tree-AS defaults on flag target respond. Table 152 lists no seed")
    treeas("depth", [("max_depth", 3)],
           "max_depth 3. Table 152: integer from 0 to 20, default 5",
           compare_to="treeas_default")
    treeas("numeric", [],
           "Tree-AS defaults on continuous target y. The node text allows a numeric-range target",
           target="y", data="numeric_train.csv", schema=NUM_SCHEMA, inputs=NUM_INPUTS,
           score_data="numeric_grid.csv", compare_to="treeas_default")

    def linearas(name, props, tests, **kwargs):
        add_case("linearas_" + name, "linearasnode", "numeric_train.csv", NUM_SCHEMA, ["y"],
                 NUM_INPUTS, linear_fields() + props, tests, score_data="numeric_grid.csv", **kwargs)

    linearas("default", [],
             "Linear-AS on continuous target y. Algorithm settings stay at the node default. custom_fields true with target y and inputs x1..x5 because Table 137 puts default TRUE on custom_fields")
    linearas("best_subsets", [("var_select_method", "BestSubsets")],
             "var_select_method BestSubsets. Table 137 values include BestSubsets; the default value is ForwardStepwise",
             compare_to="linearas_default")

    def lsvm(name, props, tests, **kwargs):
        target = kwargs.pop("target", "respond")
        data = kwargs.pop("data", "segment_train.csv")
        schema = kwargs.pop("schema", SEGMENT_SCHEMA)
        inputs = kwargs.pop("inputs", SEGMENT_INPUTS)
        grid = kwargs.pop("score_data", "segment_grid.csv")
        add_case("lsvm_" + name, "lsvmnode", data, schema, [target], inputs, props, tests,
                 score_data=grid, **kwargs)

    lsvm("default", [],
         "LSVM defaults on flag target respond. Table 139: penalty_function default L2. No seed is documented")
    lsvm("numeric", [],
         "LSVM defaults on continuous target y. Table 139 precision is used only when the target measurement is Continuous, so regression is a documented target",
         target="y", data="numeric_train.csv", schema=NUM_SCHEMA, inputs=NUM_INPUTS,
         score_data="numeric_grid.csv", compare_to="lsvm_default")
    lsvm("l1", [("penalty_function", "L1")],
         "penalty_function L1. Table 139 values are L1 and L2; the default value is L2",
         compare_to="lsvm_default")


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
