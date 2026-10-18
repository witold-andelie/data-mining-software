# Probe: Decision List, Anomaly Detection, PCA/Factor, and Time Series cases that
# models_next_streams left without an oracle, plus Ensemble and the Auto nodes.
# Coordinator only: cargo xtask probe models_next2_streams (Modeler 19, Jython 2.7).
# Writes generated inputs and one stream per case to corpus/models_next2; public API only.
# A node the license does not provide fails that case into manifest["errors"]; later cases still run.
# Generators below are fixed loops. No random draws.
import json
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/models_next2"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
SEED = 12345
NL = chr(10)
MODEL_NAME = "models_next2"

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "models_next2", "saved": [], "errors": [], "cases": [], "inputs": []}

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
SEGMENT_REGIONS = ["east", "north", "south", "west"]
SEGMENT_CHANNELS = ["phone", "store", "web"]
SEGMENT_TIERS = ["bronze", "gold", "silver"]
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
SERIES_SCHEMA = [
    ("t", "Integer", "Range", None),
    ("sales", "Real", "Range", None),
    ("cost", "Real", "Range", None),
]
SERIES_TARGETS = ["sales", "cost"]

BUILD_PROPERTIES = {
    "decisionlist": ["search_direction", "target_value", "max_rules", "min_group_size",
                     "min_group_size_pct", "confidence_level", "max_segments_per_rule", "mode",
                     "bin_method", "bin_count"],
    "anomalydetection": ["mode", "anomaly_method", "index_level", "percent_records", "num_records",
                         "num_fields", "impute_missing_values", "adjustment_coeff",
                         "peer_group_num_auto", "min_num_peer_groups", "max_num_peer_groups",
                         "num_peer_groups", "noise_level", "noise_ratio"],
    "factor": ["method", "mode", "max_iterations", "complete_records", "matrix", "extract_factors",
               "min_eigenvalue", "max_factor", "rotation", "delta", "kappa", "sort_values",
               "hide_values", "hide_below"],
    "ts": ["custom_fields", "targets", "candidate_inputs", "use_period", "date_time_field",
           "input_interval", "period_field", "is_same_interval", "method", "expert_modeler_method",
           "consider_seasonal", "exsmooth_model_type", "exsmooth_transformation_type",
           "arima.p", "arima.d", "arima.q", "arima.sp", "arima.sd", "arima.sq",
           "arima_transformation_type", "arima_include_constant", "forecastperiods",
           "extend_records_into_future", "conf_limits", "conf_limit_pct"],
    "c50": ["output_type", "group_symbolics", "use_boost", "boost_num_trials",
            "use_xval", "xval_num_folds", "mode", "favor", "expected_noise",
            "min_child_records", "pruning_severity", "use_global_pruning", "use_winnowing",
            "use_costs", "costs", "use_weight", "weight_field", "use_partitioned_data",
            "calculate_variable_importance", "calculate_raw_propensities",
            "calculate_adjusted_propensities", "adjusted_propensity_partition"],
    "chaid": ["method", "model_output_type", "use_max_depth", "max_depth", "use_percentage",
              "min_parent_records_abs", "min_child_records_abs", "min_parent_records_pc",
              "min_child_records_pc", "split_alpha", "merge_alpha", "bonferroni_adjustment",
              "split_merged_categories", "chi_square", "epsilon", "max_iterations",
              "set_random_seed", "seed", "use_partitioned_data", "calculate_variable_importance",
              "calculate_raw_propensities", "calculate_adjusted_propensities",
              "adjusted_propensity_partition"],
    "autoclassifier": ["ranking_measure", "ranking_dataset", "number_of_models",
                       "calculate_variable_importance", "enable_accuracy_limit", "accuracy_limit",
                       "enable_profit_limit", "profit_limit", "enable_lift_limit", "lift_limit",
                       "use_costs", "lift_percentile", "enable_model_build_time_limit",
                       "model_build_time_limit", "enable_stop_after_time_limit",
                       "stop_after_time_limit",
                       "cart", "chaid", "quest", "c50", "logreg", "decisionlist",
                       "bayesnet", "discriminant", "svm", "knn"],
    "autonumeric": ["custom_fields", "ranking_measure", "ranking_dataset", "number_of_models",
                    "calculate_variable_importance", "enable_correlation_limit",
                    "correlation_limit", "enable_number_of_fields_limit", "number_of_fields_limit",
                    "enable_relative_error_limit", "relative_error_limit",
                    "enable_model_build_time_limit", "model_build_time_limit",
                    "stop_if_valid_model",
                    "cart", "chaid", "neuralnetwork", "genlin", "svm", "regression", "linear",
                    "knn"],
    "autocluster": ["ranking_measure", "ranking_dataset", "summary_limit",
                    "enable_silhouette_limit", "silhouette_limit",
                    "twostep", "k-means", "kohonen"],
}
APPLY_PROPERTIES = {
    "decisionlist": ["calculate_raw_propensities", "calculate_adjusted_propensities"],
    "anomalydetection": ["anomaly_score_method", "num_fields", "discard_records",
                         "discard_anomalous_records"],
    "factor": [],
    "ts": ["extend_records_into_future", "ext_future_num", "compute_future_values_input",
           "forecastperiods", "noise_res", "conf_limits", "target_fields", "target_series",
           "includeTargets"],
    "c50": ["calculate_conf", "calculate_raw_propensities", "calculate_adjusted_propensities"],
    "chaid": ["calculate_conf", "display_rule_id", "calculate_raw_propensities",
              "calculate_adjusted_propensities"],
    "autoclassifier": ["flag_ensemble_method", "flag_evaluation_selection",
                       "filter_individual_model_output", "flag_voting_tie_selection",
                       "set_ensemble_method", "set_voting_tie_selection"],
    "autonumeric": ["calculate_standard_error"],
    "autocluster": [],
}
ENSEMBLE_PROPERTIES = ["ensemble_target_field", "filter_individual_model_output",
                       "flag_ensemble_method", "flag_voting_tie_selection",
                       "set_ensemble_method", "set_voting_tie_selection",
                       "calculate_standard_error"]
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
# Documented flag-target ensemble methods (Table 83). Tie selection is set only for the
# four methods whose names are voting methods.
FLAG_ENSEMBLE_METHODS = [
    ("voting", "Voting", True, "none"),
    ("confidence", "ConfidenceWeightedVoting", True, "none"),
    ("raw_vote", "RawPropensityWeightedVoting", True, "raw"),
    ("adjusted_vote", "AdjustedPropensityWeightedVoting", True, "adjusted"),
    ("highest", "HighestConfidence", False, "none"),
    ("avg_raw", "AverageRawPropensity", False, "raw"),
    ("avg_adjusted", "AverageAdjustedPropensity", False, "adjusted"),
]
# Auto Classifier algorithm flags documented under "Setting Algorithm Properties".
AUTO_CLASSIFIER_ALGORITHMS = ["cart", "chaid", "quest", "c50", "logreg", "decisionlist",
                              ]  # bayesnet, discriminant, svm, knn: not licensed here (undefined properties)
AUTO_CLASSIFIER_KEEP = ["c50", "chaid", "logreg"]


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


def applier_ids(stream):
    found = []
    for node in list_appliers(stream):
        found.append(unicode(node.getID()))
    return found


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
        common_build = [("use_model_name", True), ("model_name", MODEL_NAME)]
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


def propensity_props(level):
    props = []
    if level == "raw" or level == "adjusted":
        props.append(("calculate_raw_propensities", True))
    if level == "adjusted":
        props.append(("calculate_adjusted_propensities", True))
    return props


def claim_family_applier(stream, before_ids, model, label, x_pos, y_pos, family):
    fresh = []
    for node in list_appliers(stream):
        if unicode(node.getID()) not in before_ids:
            fresh.append(node)
    chosen = None
    for node in fresh:
        if family in unicode(node.getTypeName()):
            chosen = node
            break
    if chosen is None and fresh:
        chosen = fresh[0]
    if chosen is None:
        chosen = stream.createModelApplierAt(model, label, x_pos, y_pos)
        return chosen, []
    stream.disconnect(chosen)
    chosen.setLabel(label)
    chosen.setXYPosition(x_pos, y_pos)
    chosen_id = unicode(chosen.getID())
    extras = []
    for node in fresh:
        if unicode(node.getID()) != chosen_id:
            extras.append(unicode(node.getTypeName()))
            # Drop component nuggets the builder parked beside the one we chain.
            stream.delete(node)
    return chosen, extras


def train_tree(stream, training, family, label, x_pos, y_pos, nugget_x, nugget_y, props, entry):
    info = {"family": family, "label": label}
    entry.setdefault("builders", []).append(info)
    builder = stream.createAt(family, label, x_pos, y_pos)
    stream.link(training, builder)
    info["build_type"] = unicode(builder.getTypeName())
    info["defaults"] = read_properties(builder, BUILD_PROPERTIES[family])
    common_build = [("use_model_name", True), ("model_name", MODEL_NAME + "_" + family)]
    # CHAID documents set_random_seed and seed. C5.0 lists no seed; boosting and
    # cross-validation default off, so the tree does not draw folds.
    if family == "chaid":
        common_build += [("set_random_seed", True), ("seed", SEED)]
    info["common_build_properties"] = common_build
    set_properties(builder, common_build + props, info)
    info["effective"] = read_properties(builder, BUILD_PROPERTIES[family])
    before = applier_ids(stream)
    results = []
    builder.run(results)
    models = [result for result in results if isinstance(result, modeler.api.ModelOutput)]
    if not models:
        raise RuntimeError("builder returned no ModelOutput for " + family)
    info["model_output_count"] = len(models)
    nugget, extras = claim_family_applier(stream, before, models[0], label + " nugget",
                                          nugget_x, nugget_y, family)
    nugget_props = propensity_props(entry["propensity"])
    nugget_info = {"properties": nugget_props}
    set_properties(nugget, nugget_props, nugget_info)
    nugget_info["effective"] = read_properties(nugget, APPLY_PROPERTIES[family])
    nugget_info["applier_type"] = unicode(nugget.getTypeName())
    nugget_info["extra_applier_types"] = extras
    nugget_info["applier_id"] = unicode(nugget.getID())
    info["nugget"] = nugget_info
    return models[0], nugget


def link_ensemble_chain(stream, source, first, second, label, table_label, x_pos, y_pos, props, entry):
    ensemble = stream.createAt("ensemble", label, x_pos, y_pos)
    info = {"label": label, "properties": props}
    set_properties(ensemble, props, info)
    info["type"] = unicode(ensemble.getTypeName())
    info["effective"] = read_properties(ensemble, ENSEMBLE_PROPERTIES)
    info["node_id"] = unicode(ensemble.getID())
    table = stream.createAt("table", table_label, x_pos + 220, y_pos)
    stream.link(source, first)
    stream.link(first, second)
    stream.link(second, ensemble)
    stream.link(ensemble, table)
    info["table_id"] = unicode(table.getID())
    info["chain"] = [unicode(source.getTypeName()), unicode(first.getTypeName()),
                     unicode(second.getTypeName()), unicode(ensemble.getTypeName()),
                     unicode(table.getTypeName())]
    entry.setdefault("ensembles", []).append(info)
    return table


def build_ensemble(entry):
    stream = None
    stage = "create_stream"
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
        tree_props = propensity_props(entry["propensity"])
        stage = "c50_train"
        c50_model, c50_nugget = train_tree(stream, training, "c50", "c50", 480, 80,
                                           720, 80, tree_props, entry)
        stage = "chaid_train"
        chaid_model, chaid_nugget = train_tree(stream, training, "chaid", "chaid", 480, 240,
                                               960, 80, tree_props, entry)
        stage = "train_chain"
        terminals = []
        entry["terminals"] = []
        train_table = link_ensemble_chain(stream, training, c50_nugget, chaid_nugget,
                                          "Train ensemble", "Train scores", 1200, 80,
                                          entry["properties"], entry)
        terminals.append(train_table)
        stage = "scoring_type"
        scoring = training
        if entry["score_data"] is not None:
            scoring = typed_source(stream, entry["score_data"], schema, entry["targets"],
                                   entry["inputs"], entry["roles"], entry["missing"], 520, entry)
        stage = "scoring_chain"
        grid_c50 = stream.createModelApplierAt(c50_model, "Grid c50", 720, 520)
        grid_chaid = stream.createModelApplierAt(chaid_model, "Grid chaid", 960, 520)
        for nugget, family in ((grid_c50, "c50"), (grid_chaid, "chaid")):
            settings = {"label": unicode(nugget.getLabel()), "family": family,
                        "properties": tree_props}
            set_properties(nugget, tree_props, settings)
            settings["effective"] = read_properties(nugget, APPLY_PROPERTIES[family])
            settings["applier_type"] = unicode(nugget.getTypeName())
            entry.setdefault("grid_nuggets", []).append(settings)
        grid_table = link_ensemble_chain(stream, scoring, grid_c50, grid_chaid,
                                         "Grid ensemble", "Grid scores", 1200, 520,
                                         entry["properties"], entry)
        terminals.append(grid_table)
        entry["placed_nugget_types"] = [unicode(c50_nugget.getTypeName()),
                                        unicode(chaid_nugget.getTypeName())]
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


def series_fields(index):
    # index is 0-based. t is the monthly period 1..48. Linear trend, no seasonal term.
    t = index + 1
    sales = 20.0 + 0.5 * float(t)
    cost = 8.0 + 0.25 * float(t)
    return [t, sales, cost]


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
              "48 rows; same generator as models_next class_train; 6 x by 4 cat by 2 flag; null x, null z, null cat")

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
              "43 rows; same generator as models_next class_grid; 3 missing predictors")

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

    series = []
    for index in range(48):
        series.append(series_fields(index))
    write_csv("series_train.csv", SERIES_SCHEMA, series,
              "48 months; sales=20+0.5*t, cost=8+0.25*t; no seasonal term; t=1..48")


def declare_cases():
    def lists(name, props, tests, **kwargs):
        add_case("dl_" + name, "decisionlist", "segment_train.csv", SEGMENT_SCHEMA, ["respond"],
                 SEGMENT_INPUTS, props, tests, score_data="segment_grid.csv", **kwargs)

    lists("default", [],
          "Decision List defaults; pure cells of 80, 60, and 55 rows, each above the default minimum segment size of 50")
    lists("max_rules", [("max_rules", 2)],
          "At most 2 segments excluding the remainder", compare_to="dl_default")
    lists("min_group", [("min_group_size", 70)],
          "Minimum segment size 70, which drops the 60-row and 55-row cells",
          compare_to="dl_default")

    def anomaly(name, props, tests, **kwargs):
        add_case("anomaly_" + name, "anomalydetection", "class_train.csv", CLASS_SCHEMA, [],
                 CLASS_INPUTS, props, tests, score_data="class_grid.csv", **kwargs)

    anomaly("index", [("mode", "Expert"), ("anomaly_method", "IndexLevel"), ("index_level", 1.3)],
            "Expert anomaly index cutoff 1.3")
    # NumRecords is the third cutoff next to IndexLevel and PerRecords. The guide table
    # wraps that token onto the index_level row; num_records is the matching count.
    anomaly("records", [("mode", "Expert"), ("anomaly_method", "NumRecords"), ("num_records", 3)],
            "Expert cutoff: flag 3 training records", compare_to="anomaly_index")
    anomaly("peers", [("mode", "Expert"), ("peer_group_num_auto", False), ("num_peer_groups", 3)],
            "Expert fixed peer-group count 3", compare_to="anomaly_index")
    anomaly("adjust", [("mode", "Expert"), ("adjustment_coeff", 3.0)],
            "Expert adjustment coefficient 3", compare_to="anomaly_index")

    add_case("factor_default", "factor", "numeric_train.csv", NUM_SCHEMA, [], NUM_INPUTS, [],
             "PCA/Factor non-expert defaults on 48 full-rank numeric rows",
             score_data="numeric_grid.csv")

    def timeseries(name, props, tests, **kwargs):
        add_case("ts_" + name, "ts", "series_train.csv", SERIES_SCHEMA, SERIES_TARGETS, [],
                 props, tests, score_variants=TS_FORECAST, **kwargs)

    timeseries("simple", TS_BASE + [("method", "Exsmooth"),
                                    ("exsmooth_model_type", "Simple"),
                                    ("exsmooth_transformation_type", "None")],
               "Simple exponential smoothing on a 48-point non-seasonal trend; 6 forecast steps")
    timeseries("holt", TS_BASE + [("method", "Exsmooth"),
                                  ("exsmooth_model_type", "HoltsLinearTrend"),
                                  ("exsmooth_transformation_type", "None")],
               "Holt linear-trend exponential smoothing, same series and forecast settings",
               compare_to="ts_simple")

    def ensemble(name, method, tests, tie, propensity, compare_to):
        props = [
            ("ensemble_target_field", "respond"),
            ("filter_individual_model_output", False),
            ("flag_ensemble_method", method),
        ]
        if tie:
            props.append(("flag_voting_tie_selection", "HighestConfidence"))
        add_case("ens_" + name, "ensemble", "segment_train.csv", SEGMENT_SCHEMA, ["respond"],
                 SEGMENT_INPUTS, props, tests, score_data="segment_grid.csv",
                 compare_to=compare_to)
        manifest["cases"][-1]["propensity"] = propensity

    ensemble("voting", "Voting",
             "Flag voting of C5.0 then CHAID; ties take the higher confidence",
             True, "none", None)
    ensemble("confidence", "ConfidenceWeightedVoting",
             "Confidence-weighted voting; ties take the higher confidence",
             True, "none", "ens_voting")
    ensemble("raw_vote", "RawPropensityWeightedVoting",
             "Raw-propensity-weighted voting; both nuggets emit raw propensity",
             True, "raw", "ens_voting")
    ensemble("adjusted_vote", "AdjustedPropensityWeightedVoting",
             "Adjusted-propensity-weighted voting; both nuggets emit raw and adjusted propensity",
             True, "adjusted", "ens_voting")
    ensemble("highest", "HighestConfidence",
             "Highest-confidence wins; not a voting method, so no tie setting",
             False, "none", "ens_voting")
    ensemble("avg_raw", "AverageRawPropensity",
             "Average of the raw propensities",
             False, "raw", "ens_voting")
    ensemble("avg_adjusted", "AverageAdjustedPropensity",
             "Average of the adjusted propensities",
             False, "adjusted", "ens_voting")

    add_case("ac_default", "autoclassifier", "segment_train.csv", SEGMENT_SCHEMA, ["respond"],
             SEGMENT_INPUTS, [],
             "Auto Classifier defaults on the flag target",
             score_data="segment_grid.csv")
    restricted = []
    for algorithm in AUTO_CLASSIFIER_ALGORITHMS:
        restricted.append((algorithm, algorithm in AUTO_CLASSIFIER_KEEP))
    add_case("ac_restricted", "autoclassifier", "segment_train.csv", SEGMENT_SCHEMA, ["respond"],
             SEGMENT_INPUTS, restricted,
             "Auto Classifier using only c50, chaid, and logreg",
             score_data="segment_grid.csv", compare_to="ac_default")
    add_case("an_default", "autonumeric", "numeric_train.csv", NUM_SCHEMA, ["y"], NUM_INPUTS, [],
             "Auto Numeric defaults; continuous target y, inputs x1..x5",
             score_data="numeric_grid.csv")
    add_case("acluster_default", "autocluster", "numeric_train.csv", NUM_SCHEMA, [], NUM_INPUTS, [],
             "Auto Cluster defaults on x1..x5",
             score_data="numeric_grid.csv")


try:
    ensure_clean_dir(DIR)
    declare_inputs()
    declare_cases()
    write_manifest()
    for case in manifest["cases"]:
        if case["family"] == "ensemble":
            build_ensemble(case)
        else:
            build_case(case)
except ERRORS as exc:
    manifest["errors"].append({"case": None, "stage": "setup", "message": message(exc)})
finally:
    write_manifest()
