# Feature Selection and GenLin behavioral corpus. Jython 2.7, public scripting API only.
# Coordinator only: cargo xtask probe models_stats_streams
# source -> [Derive: explicit string edge values] -> Type -> builder (terminal)
#                                                     -> saved nugget -> Table (terminal)
# See probe-plans/models_stats_streams.md for case comparisons and export limitations.
import json
import math
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
DATA_DIR = "D:/data-mining-software/corpus/models_stats"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)
taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "models_stats", "saved": [], "errors": [], "cases": [],
            "datasets": [], "decimal_places": 15,
            "seed_policy": "Neither builder documents a random seed; no random data generation."}

FS_NAMES = [
    "screen_single_category", "max_single_category", "screen_missing_values",
    "max_missing_values", "screen_num_categories", "max_num_categories",
    "screen_std_dev", "min_std_dev", "screen_coeff_of_var", "min_coeff_of_var",
    "criteria", "unimportant_below", "important_above", "unimportant_label",
    "marginal_label", "important_label", "selection_mode", "select_important",
    "select_marginal", "select_unimportant", "importance_value", "top_n",
    "use_partitioned_data", "partition",
]
GL_NAMES = [
    "use_weight", "weight_field", "use_frequency", "frequency_field",
    "target_represents_trials", "trials_type", "trials_field", "trials_number",
    "model_type", "offset_type", "offset_field", "offset_value", "base_category",
    "include_intercept", "mode", "distribution", "negbin_para_type",
    "negbin_parameter", "tweedie_parameter", "link_function", "power", "method",
    "max_fisher_iterations", "scale_method", "scale_value", "covariance_matrix",
    "max_iterations", "max_step_halving", "check_separation", "start_iteration",
    "estimates_change", "estimates_change_min", "estimates_change_type",
    "loglikelihood_change", "loglikelihood_change_min", "loglikelihood_change_type",
    "hessian_convergence", "hessian_convergence_min", "hessian_convergence_type",
    "case_summary", "contrast_matrices", "descriptive_statistics", "estimable_functions",
    "model_info", "iteration_history", "goodness_of_fit", "print_interval",
    "model_summary", "lagrange_multiplier", "parameter_estimates", "include_exponential",
    "covariance_estimates", "correlation_estimates", "analysis_type", "statistics",
    "citype", "tolerancelevel", "confidence_interval", "loglikelihood_function",
    "singularity_tolerance", "value_order", "calculate_variable_importance",
    "calculate_raw_propensities", "calculate_adjusted_propensities",
    "adjusted_propensity_partition", "use_partitioned_data", "partition",
]


def message(exc):
    get = getattr(exc, "getMessage", None)
    return unicode(get()) if get is not None else unicode(exc)


def json_value(value):
    if value is None or isinstance(value, (bool, int, long, float, basestring)):
        return value
    if isinstance(value, dict):
        return dict([(unicode(k), json_value(v)) for k, v in value.items()])
    try:
        return [json_value(v) for v in value]
    except (TypeError, AttributeError):
        return unicode(value)


def snapshot(node, names):
    values = {}
    for name in names:
        try:
            values[name] = {"value": json_value(node.getPropertyValue(name))}
        except ERRORS, e:
            values[name] = {"read_error": message(e)}
    return values


def ensure_clean_dir(path, owned_names):
    # Never recursively clean a corpus label or follow its children. Remove ONLY our known
    # flat filenames; unrelated files and subdirectories survive a coordinator rerun.
    if path != DATA_DIR:
        raise RuntimeError("refusing to clean a directory other than the models_stats label")
    directory = java.io.File(path)
    if not directory.exists() and not directory.mkdirs():
        raise RuntimeError("cannot create " + path)
    for name in owned_names:
        if "/" in name or "\\" in name or name in ["", ".", ".."]:
            raise RuntimeError("invalid generated filename: " + name)
        child = java.io.File(directory, name)
        if child.exists() and (not child.isFile() or not child.delete()):
            raise RuntimeError("cannot remove old generated file: " + name)


def csv_cell(value):
    if value is None:
        return ""
    if isinstance(value, basestring):
        return '"' + value.replace('"', '""') + '"'
    if isinstance(value, float):
        return "%.17g" % value
    return str(value)


def write_csv(data):
    handle = open(DATA_DIR + "/" + data["file"], "w")
    try:
        handle.write(",".join([f[0] for f in data["source_schema"]]) + NL)
        for row in data["rows"]:
            handle.write(",".join([csv_cell(v) for v in row]) + NL)
    finally:
        handle.close()
    manifest["saved"].append(data["file"])
    manifest["datasets"].append({"file": data["file"], "records": len(data["rows"]),
                                 "source_schema": data["source_schema"],
                                 "derivations": data["derivations"]})


def dataset(name, schema, rows, derivations=None):
    derivations = derivations or []
    return {"file": name + ".csv", "source_schema": schema, "rows": rows,
            "schema": schema + [(n, s, m) for n, s, m, expr in derivations],
            "derivations": derivations}


def make_datasets():
    # No IBM/team data, randomness, fitting, or third-party implementations are used.
    cat_schema = [("rid", "Integer", "Range"), ("target", "String", "Set")]
    cat_fields = ["signal", "signal_copy", "weak", "marginal", "independent", "holdout_signal",
                  "dominant63", "dominant64", "dominant65", "cats7", "cats8", "cats9",
                  "constant", "unique_id"]
    cat_schema += [(n, "String", "Set") for n in cat_fields]
    cat_schema += [("part", "Integer", "Set")]
    cat_rows = []
    for i in range(80):
        y = int(i % 8 >= 4)
        signal = y if i % 10 != 0 else 1 - y
        weak = y if i % 4 != 0 else 1 - y
        marginal = 1 - y if i % 8 in [0, 1, 4] else y
        cat_rows.append([i + 1, ["no", "yes"][y], str(signal), str(signal), str(weak),
                         str(marginal), str(i % 2), str(y if i < 40 else 1 - y),
                         "A" if i < 63 else "B", "A" if i < 64 else "B",
                         "A" if i < 65 else "B", "c%d" % (i % 7),
                         "c%d" % (i % 8), "c%d" % (i % 9), "K", "u%03d" % i,
                         1 if i < 40 else (2 if i < 60 else 3)])
    cat_derivations = [
        ("miss39", "String", "Set", "if rid <= 39 then undef else signal endif"),
        ("miss40", "String", "Set", "if rid <= 40 then undef else signal endif"),
        ("miss41", "String", "Set", "if rid <= 41 then undef else signal endif"),
        ("all_null", "String", "Set", "if rid < 0 then signal else undef endif"),
    ]
    cat = dataset("fs_categories", cat_schema, cat_rows, cat_derivations)
    # Only physical column order changes, including the first two exactly tied predictors.
    order = [0, 1, 3, 2] + list(range(4, len(cat_schema)))
    cat_reordered = dataset("fs_categories_reordered", [cat_schema[i] for i in order],
                            [[r[i] for i in order] for r in cat_rows], cat_derivations)

    cont_fields = ["linear", "linear_copy", "negative", "noise", "sd_low", "sd_at",
                   "sd_high", "cv_low", "cv_at", "cv_high", "cv_negative", "zero_mean"]
    cont_schema = [("rid", "Integer", "Range"), ("y", "Real", "Range")]
    cont_schema += [(n, "Real", "Range") for n in cont_fields]
    cont_schema += [("group", "String", "Set")]
    cont_rows = []
    unit = math.sqrt(79.0 / 80.0)  # balanced +/-unit has SAMPLE standard deviation 1
    for i in range(80):
        v = unit if i % 2 else -unit
        jitter = (i % 5 - 2) * 0.2
        cont_rows.append([i + 1, 2 * v + jitter, v, v, -v, jitter,
                          0.49 * v, 0.50 * v, 0.51 * v,
                          10 + 0.49 * v, 10 + 0.50 * v, 10 + 0.51 * v,
                          -10 + 0.50 * v, v, "A" if v < 0 else "B"])
    cont = dataset("fs_continuous", cont_schema, cont_rows)

    mixed_schema = [("rid", "Integer", "Range"), ("x", "Real", "Range"),
                    ("z", "Real", "Range"), ("numeric_cat", "Integer", "Set"),
                    ("cat", "String", "Set"), ("y", "Real", "Range"),
                    ("w", "Real", "Range"), ("freq", "Real", "Range"),
                    ("part", "Integer", "Set")]
    mixed_rows = []
    for rep in range(4):
        for group in range(3):
            for xi in range(6):
                x = xi - 2
                z = (rep + xi) % 3 - 1
                y = 4 + 1.25 * x - 0.5 * z + group + 0.6 * x * group
                y += [-0.4, 0.1, 0.3, 0.0][rep]
                y += 7 if rep == 2 else (-3 if rep == 3 else 0)
                mixed_rows.append([len(mixed_rows) + 1, float(x), float(z),
                                   (xi + rep) % 3, ["C", "A", "B"][group], y,
                                   float(1 + (xi + group) % 3), float(1 + xi % 2),
                                   1 if rep < 2 else (2 if rep == 2 else 3)])
    for j in range(12):
        row = list(mixed_rows[j])
        row[0] = 73 + j
        if j == 0:
            row[1] = None
        elif j == 1:
            row[5] = None
        elif j == 2:
            row[1], row[2], row[3] = None, None, None
        elif j == 6:
            row[1] = -999.0
        elif j in [7, 8, 9]:
            row[6] = [0.0, -1.0, None][j - 7]
        elif j in [10, 11]:
            row[7] = [0.4, 1.6][j - 10]
        mixed_rows.append(row)
    mixed_rows.append(list(mixed_rows[0]))  # exact duplicate, including rid
    mixed_derivations = [
        ("cat_edge", "String", "Set",
         'if rid = 78 then undef else if rid = 76 then "" '
         'else if rid = 77 then "  " else cat endif endif endif'),
        ("x_copy", "Real", "Range", "x"),
    ]
    mixed = dataset("mixed_edges", mixed_schema, mixed_rows, mixed_derivations)

    count_schema = [("rid", "Integer", "Range"), ("x", "Real", "Range"),
                    ("cat", "String", "Set"), ("count", "Real", "Range"),
                    ("offset", "Real", "Range")]
    count_rows = []
    positive_rows = []
    for x in range(5):
        for group in range(2):
            for rep in range(6):
                rid = len(count_rows) + 1
                off = math.log(1 + group + rep % 3)
                count_rows.append([rid, float(x), ["B", "A"][group],
                                   float([0, 0, 1, 2, 5, 12][rep] * (x + 1) + group), off])
                positive_rows.append([rid, float(x), ["B", "A"][group],
                                      (x + 1) * (1 + 0.4 * group) *
                                      [0.4, 0.7, 1.0, 1.3, 1.8, 2.4][rep], off])
    for bad in [-1.0, 0.5, None]:
        count_rows.append([len(count_rows) + 1, 2.0, "A", bad, 0.0])
    count_rows.append([64, 2.0, "A", 4.0, None])
    for bad in [0.0, -1.0, None]:
        positive_rows.append([len(positive_rows) + 1, 2.0, "A", bad, 0.0])
    counts = dataset("count_edges", count_schema, count_rows)
    positives = dataset("positive_edges", count_schema, positive_rows)

    binary_schema = [("rid", "Integer", "Range"), ("x", "Real", "Range"),
                     ("cat", "String", "Set"), ("event", "Integer", "Flag"),
                     ("part", "Integer", "Set")]
    binary_rows = []
    for xi in range(5):
        for rep in range(20):
            # Every x has both outcomes; no complete separation. Holdout calibration differs.
            event = int((rep * 7) % 20 < [3, 6, 9, 13, 17][xi])
            binary_rows.append([len(binary_rows) + 1, float(xi - 2),
                                "B" if rep % 2 else "A", event,
                                1 if rep < 12 else (2 if rep < 16 else 3)])
    binary = dataset("binary", binary_schema, binary_rows)

    trials_schema = [("rid", "Integer", "Range"), ("x", "Real", "Range"),
                     ("events", "Real", "Range"), ("trials", "Real", "Range")]
    trials_rows = []
    for x in range(5):
        for group in range(3):
            trials_rows.append([len(trials_rows) + 1, float(x - 2),
                                float((x + 1) * (group + 1) + group % 2),
                                float(10 + 5 * group)])
    for events, trials in [(-1, 10), (1.5, 10), (None, 10), (11, 10),
                           (1, 0), (1, -1), (1, 2.5), (1, None), (0, 10), (10, 10)]:
        trials_rows.append([len(trials_rows) + 1, 0.0, events, trials])
    trials = dataset("trials_edges", trials_schema, trials_rows)

    ordinal_schema = [("rid", "Integer", "Range"), ("x", "Real", "Range"),
                      ("grade", "String", "OrderedSet")]
    ordinal_rows = []
    for xi in range(5):
        low, middle = [12, 10, 8, 6, 4][xi], 8
        # Permuted record order deliberately starts with the middle category.
        for rep in range(24):
            k = (rep * 5 + 13) % 24
            grade = "low" if k < low else ("middle" if k < low + middle else "high")
            ordinal_rows.append([len(ordinal_rows) + 1, float(xi - 2), grade])
    ordinal = dataset("ordinal", ordinal_schema, ordinal_rows)
    return dict([(d["file"], d) for d in
                 [cat, cat_reordered, cont, mixed, counts, positives, binary, trials, ordinal]])


def instantiate_node(stream, node):
    table = stream.createAt("table", "Instantiate", 850, 400)
    stream.link(node, table)
    try:
        table.run([])
    finally:
        stream.delete(table)


def apply_properties(node, properties):
    for name, value in properties:
        node.setPropertyValue(name, value)


def make_type(stream, data, case, entry):
    source = stream.createAt("variablefile", data["file"], 80, 100)
    source_props = [("full_filename", DATA_DIR + "/" + data["file"]),
                    ("read_field_names", True), ("delimit_comma", True),
                    ("delimit_space", False), ("delimit_tab", False),
                    ("strip_spaces", "None"), ("quotes_2", "PairAndDiscard"),
                    ("decimal_symbol", "Period"), ("encoding", "UTF-8")]
    apply_properties(source, source_props)
    entry["source_properties"] = source_props
    for field, storage, measure in data["source_schema"]:
        source.setKeyedPropertyValue("custom_storage_type", field, storage)
    upstream = source
    for name, storage, measure, expression in data["derivations"]:
        derive = stream.createAt("derive", name, 160, 240)
        apply_properties(derive, [("new_name", name), ("result_type", "Formula"),
                                  ("formula_expr", expression), ("formula_measure_type", measure)])
        stream.link(upstream, derive)
        upstream = derive
    typ = stream.createAt("type", "Type", 300, 100)
    stream.link(upstream, typ)
    instantiate_node(stream, typ)
    type_props = []
    for field, storage, measure in data["schema"]:
        direction = "None"
        if field == case["target"]:
            direction = "Target"
        elif field in case["inputs"]:
            direction = "Input"
        if field == case.get("partition"):
            direction = "Partition"
        type_props += [("type", field, measure), ("direction", field, direction),
                       ("check", field, "None")]
        if storage == "Real":
            type_props += [("display_places", field, 15), ("export_places", field, 15)]
        if field == "event":
            type_props.append(("values", field, [0, 1]))
        if field == "grade":
            type_props.append(("values", field, ["low", "middle", "high"]))
        if direction == "Partition":
            type_props.append(("values", field, [1, 2, 3]))
    if data["file"] == "mixed_edges.csv":
        type_props += [("enable_missing", "x", True), ("missing_values", "x", [-999.0]),
                       ("null_missing", "x", True), ("enable_missing", "cat_edge", True),
                       ("null_missing", "cat_edge", True),
                       ("whitespace_missing", "cat_edge", False)]
    entry["type_properties"] = type_props
    for name, field, value in type_props:
        typ.setKeyedPropertyValue(name, field, value)
    instantiate_node(stream, typ)
    entry["field_storage_readback"] = {}
    for field, storage, measure in data["schema"]:
        entry["field_storage_readback"][field] = unicode(typ.getKeyedPropertyValue("storage", field))
    return typ


def find_applier(stream):
    iterator = stream.iterator()
    while iterator.hasNext():
        node = iterator.next()
        if unicode(node.getTypeName()).startswith("apply"):
            return node
    return None


def build_case(case, data):
    stream = None
    stage = "create_stream"
    entry = dict(case)
    entry.update({"status": "started", "file": case["case"] + ".str",
                  "records": len(data["rows"]), "schema": data["schema"],
                  "derivations": data["derivations"]})
    manifest["cases"].append(entry)
    try:
        stream = taskrunner.createStream(case["case"], False, False)
        stream_props = [("decimal_places", 15), ("create_model_applier_for_new_models", True),
                        ("create_model_applier_update_links", "createEnabled")]
        apply_properties(stream, stream_props)
        entry["stream_properties"] = stream_props
        stage = "source_and_type"
        typ = make_type(stream, data, case, entry)
        stage = "configure_builder"
        builder = stream.createAt(case["node_type"], case["case"], 470, 100)
        stream.link(typ, builder)
        names = FS_NAMES if case["node_type"] == "featureselection" else GL_NAMES
        entry["builder_defaults"] = snapshot(builder, names)
        common = [("use_model_name", True), ("model_name", case["case"]),
                  ("custom_fields", True), ("target", case["target"]),
                  ("inputs", list(case["inputs"]))]
        if case.get("partition"):
            common.append(("partition", case["partition"]))
        entry["common_properties"] = common
        apply_properties(builder, common)
        apply_properties(builder, case["properties"])
        entry["builder_configured"] = snapshot(builder, names)
        stage = "build_model"
        results = []
        builder.run(results)
        entry["model_output_count"] = len(results)
        stage = "attach_nugget"
        applier = find_applier(stream)
        entry["auto_placed_applier"] = applier is not None
        if applier is None:
            models = [r for r in results if isinstance(r, modeler.api.ModelOutput)]
            if not models:
                raise RuntimeError("builder returned no ModelOutput and no automatic applier")
            applier = stream.createModelApplierAt(models[0], case["case"], 470, 260)
        applier.setXYPosition(470, 260)
        if not list(stream.predecessors(applier)):
            stream.link(typ, applier)
        entry["build_type"] = unicode(builder.getTypeName())
        entry["applier_type"] = unicode(applier.getTypeName())
        applier_names = (["selected_ranked_fields", "selected_screened_fields"]
                         if case["node_type"] == "featureselection" else
                         ["calculate_raw_propensities", "calculate_adjusted_propensities"])
        entry["nugget_before"] = snapshot(applier, applier_names)
        apply_properties(applier, case["nugget_properties"])
        entry["nugget_after"] = snapshot(applier, applier_names)
        stage = "score_table"
        table = stream.createAt("table", "Scores", 690, 260)
        stream.link(applier, table)
        table.run([])
        stage = "save_stream"
        # Oracle reruns the terminal builder to export its model. Preserve the scored nugget
        # (especially manual FS selections) instead of placing/updating another applier.
        stream.setPropertyValue("create_model_applier_for_new_models", False)
        entry["saved_stream_properties"] = [("decimal_places", 15),
                    ("create_model_applier_for_new_models", False),
                    ("create_model_applier_update_links", "createEnabled")]
        taskrunner.saveStreamToFile(stream, DATA_DIR + "/" + entry["file"])
        manifest["saved"].append(entry["file"])
        entry["status"] = "saved"
    except ERRORS, e:
        entry.update({"status": "failed", "stage": stage, "message": message(e)})
        manifest["errors"].append({"case": case["case"], "stage": stage, "message": message(e)})
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS, e:
                manifest["errors"].append({"case": case["case"], "stage": "clear_stream",
                                            "message": message(e)})


def case(name, node, data, target, inputs, props, purpose, partition=None, nugget=None):
    return {"case": name, "node_type": node, "data": data, "target": target,
            "inputs": list(inputs), "properties": props, "purpose": purpose,
            "partition": partition, "nugget_properties": nugget or []}


def make_cases(data):
    cases = []
    cat_inputs = [f[0] for f in data["fs_categories.csv"]["schema"]
                  if f[0] not in ["rid", "target", "part"]]
    cont_inputs = [f[0] for f in data["fs_continuous.csv"]["schema"]
                   if f[0] not in ["rid", "y"]]
    mixed_inputs = ["x", "z", "cat_edge", "numeric_cat", "x_copy"]
    off = [("screen_single_category", False), ("screen_missing_values", False),
           ("screen_num_categories", False), ("screen_std_dev", False),
           ("screen_coeff_of_var", False)]
    top = [("selection_mode", "TopN"), ("top_n", 1), ("use_partitioned_data", False)]
    fs_base = off + top + [("criteria", "Pearson")]
    cases.append(case("fs_default_mixed", "featureselection", "mixed_edges.csv", "cat_edge",
                      ["x", "z", "numeric_cat", "x_copy"], [],
                      "Unmodified defaults; categorical target, mixed inputs, null/empty/space targets"))
    for criterion, suffix in [("Pearson", "pearson"), ("Likelihood", "likelihood"),
                               ("CramersV", "cramers_v"), ("Lambda", "lambda")]:
        cases.append(case("fs_" + suffix, "featureselection", "fs_categories.csv", "target",
                          cat_inputs, off + top + [("criteria", criterion)],
                          "Categorical ranking, exact tied copies, independent and asymmetric tables"))
    cases.append(case("fs_tie_column_order", "featureselection", "fs_categories_reordered.csv",
                      "target", ["signal_copy", "signal"] + cat_inputs[2:], fs_base,
                      "Pearson tie-break after swapping physical and input-list order"))
    cases.append(case("fs_continuous_target", "featureselection", "fs_continuous.csv", "y",
                      cont_inputs, fs_base, "Correlation vs ANOVA, negative correlation, exact copies"))
    for name, toggle, threshold, value in [
            ("missing", "screen_missing_values", "max_missing_values", 50.0),
            ("dominance", "screen_single_category", "max_single_category", 80.0),
            ("categories", "screen_num_categories", "max_num_categories", 10.0)]:
        cases.append(case("fs_screen_" + name, "featureselection", "fs_categories.csv", "target",
                          cat_inputs, fs_base + [("top_n", 1000), (toggle, True), (threshold, value)],
                          "Only this optional screen enabled; values immediately below/at/above threshold"))
    for name, toggle, threshold, value in [
            ("sd", "screen_std_dev", "min_std_dev", 0.5),
            ("cv", "screen_coeff_of_var", "min_coeff_of_var", 0.05)]:
        cases.append(case("fs_screen_" + name, "featureselection", "fs_continuous.csv", "y",
                          cont_inputs, fs_base + [("top_n", 1000), (toggle, True), (threshold, value)],
                          "Sample SD .49/.50/.51; CV +/-.049/.050/.051 and zero mean"))
    cases.append(case("fs_importance_value_50", "featureselection", "fs_categories.csv", "target",
                      cat_inputs, fs_base + [("selection_mode", "ImportanceValue"),
                                             ("importance_value", 50.0)],
                      "Importance cutoff uses 0-100 scripting units, excluding weak associations"))
    cases.append(case("fs_importance_levels", "featureselection", "fs_categories.csv", "target",
                      cat_inputs, fs_base + [("selection_mode", "ImportanceLevel"),
                                             ("unimportant_below", 0.90), ("important_above", 0.99),
                                             ("select_important", False), ("select_marginal", True),
                                             ("select_unimportant", True), ("important_label", "High"),
                                             ("marginal_label", "Middle"), ("unimportant_label", "Low")],
                      "Importance bands and all three switches; renamed labels in saved model"))
    cases.append(case("fs_manual_fields", "featureselection", "fs_categories.csv", "target",
                      cat_inputs, fs_base, "Manual ranked selection plus restoring a screened constant",
                      nugget=[("selected_ranked_fields", ["weak"]),
                              ("selected_screened_fields", ["constant"])]))
    cases.append(case("fs_training_partition", "featureselection", "fs_categories.csv", "target",
                      cat_inputs, fs_base + [("use_partitioned_data", True)],
                      "Training-only association differs from all-data ranking; score every partition",
                      partition="part"))

    def gl(name, filename, target, inputs, props, purpose, partition=None, nugget=None):
        cases.append(case("gl_" + name, "genlin", filename, target, inputs, props,
                          purpose, partition, nugget))

    def expert(distribution, link):
        return [("mode", "Expert"), ("distribution", distribution), ("link_function", link)]

    diagnostics = [(n, True) for n in ["case_summary", "contrast_matrices", "descriptive_statistics",
                    "estimable_functions", "model_info", "iteration_history", "goodness_of_fit",
                    "model_summary", "lagrange_multiplier", "parameter_estimates", "include_exponential",
                    "covariance_estimates", "correlation_estimates"]]
    normal = expert("NORMAL", "IDENTITY")
    gl("default_mixed", "mixed_edges.csv", "y", mixed_inputs, [],
       "True Simple-mode defaults; categorical dummies, redundant covariate, duplicates, null/blank")
    gl("interactions", "mixed_edges.csv", "y", mixed_inputs,
       [("model_type", "MainAndAllTwoWayEffects"), ("value_order", "Descending")],
       "Add all two-way effects and reverse factor coding; target contains a real x-by-category term")
    gl("no_intercept_offset", "mixed_edges.csv", "y", ["x", "z", "x_copy"],
       normal + [("include_intercept", False), ("offset_type", "FixedValue"), ("offset_value", 1.25),
                 ("scale_method", "Deviance"),
                 ("analysis_type", "TypeIAndTypeIII"), ("statistics", "LR"), ("citype", "Profile"),
                 ("tolerancelevel", 0.00001), ("confidence_interval", 90), ("print_interval", 2),
                 ("loglikelihood_function", "Kernel"), ("singularity_tolerance", "1E-010")] + diagnostics,
       "Continuous-only no-intercept fit, known offset coefficient 1; diagnostic requests retained")
    gl("weighted", "mixed_edges.csv", "y", mixed_inputs,
       normal + [("use_weight", True), ("weight_field", "w"),
                 ("scale_method", "FixedValue"), ("scale_value", 2.0),
                 ("covariance_matrix", "RobustEstimator"), ("analysis_type", "TypeI"),
                 ("value_order", "DataOrder"), ("calculate_variable_importance", True)],
       "Scale weights, invalid weights, robust covariance and data order; unused frequency sentinel")
    gl("training_partition", "mixed_edges.csv", "y", mixed_inputs,
       [("use_partitioned_data", True)], "Training rows alone; held-out targets have deliberate shifts",
       partition="part")
    count = expert("POISSON", "LOG") + [("offset_type", "Variable"), ("offset_field", "offset"),
                ("scale_method", "FixedValue"), ("scale_value", 1.0)]
    gl("poisson_offset", "count_edges.csv", "count", ["x", "cat"], count,
       "Count-log mean, variable log-exposure offset, zero counts; reject negative/fractional targets")
    gl("negbin_specified", "count_edges.csv", "count", ["x", "cat"],
       count + [("distribution", "NEGBIN"), ("negbin_para_type", "Specify"),
                ("negbin_parameter", 0.5), ("link_function", "NEGBIN")],
       "Overdispersed counts; explicit ancillary parameter and canonical negative-binomial link")
    gl("negbin_estimated", "count_edges.csv", "count", ["x", "cat"],
       count + [("distribution", "NEGBIN"), ("negbin_para_type", "Estimate")],
       "Estimate ancillary parameter with log link; same data and offsets as Poisson")
    gl("gamma_power", "positive_edges.csv", "count", ["x", "cat"],
       expert("GAMMA", "POWER") + [("power", -1.0), ("scale_method", "MaxLikelihoodEstimate"),
                 ("method", "NewtonRaphson"), ("max_iterations", 150),
                 ("max_step_halving", 10), ("loglikelihood_function", "Full"),
                 ("estimates_change", True), ("estimates_change_type", "Relative"),
                 ("estimates_change_min", 1.0e-9), ("loglikelihood_change", True),
                 ("loglikelihood_change_type", "Absolute"), ("loglikelihood_change_min", 1.0e-9),
                 ("hessian_convergence", True), ("hessian_convergence_type", "Relative"),
                 ("hessian_convergence_min", 1.0e-9)],
       "Reciprocal mean; jointly estimate gamma scale; zero/negative/null responses excluded")
    gl("inverse_gaussian", "positive_edges.csv", "count", ["x", "cat"],
       expert("IGAUSS", "LOG") + [("method", "Fisher"), ("scale_method", "Deviance")],
       "Positive response, inverse-Gaussian variance and log mean; Fisher scoring")
    gl("tweedie_power", "positive_edges.csv", "count", ["x", "cat"],
       expert("TWEEDIE", "POWER") + [("tweedie_parameter", 1.5), ("power", 0.5),
                 ("method", "Hybrid"), ("max_fisher_iterations", 2),
                 ("scale_method", "PearsonChiSquare")],
       "Compound Poisson/gamma power 1.5, square-root mean link, valid zero vs invalid negative")
    gl("binary_default", "binary.csv", "event", ["x", "cat"], [],
       "Simple-mode flag-target defaults; both outcomes at every x; reference category and confidence")
    gl("binary_first", "binary.csv", "event", ["x", "cat"], [("base_category", "First")],
       "Only change binary reference category; compare signs and saved probabilities with defaults")
    binomial = expert("BINOMIAL", "LOGIT") + [("scale_method", "FixedValue"), ("scale_value", 1.0)]
    gl("binary_probit", "binary.csv", "event", ["x", "cat"],
       binomial + [("link_function", "PROBIT"), ("method", "Fisher"),
                    ("estimates_change", True), ("estimates_change_min", 1.0e-10),
                    ("estimates_change_type", "Absolute"), ("loglikelihood_change", True),
                    ("loglikelihood_change_min", 1.0e-10), ("loglikelihood_change_type", "Relative"),
                    ("hessian_convergence", True), ("hessian_convergence_min", 1.0e-10),
                    ("hessian_convergence_type", "Absolute"), ("check_separation", True),
                    ("start_iteration", 2), ("max_step_halving", 8)],
       "Normal-CDF inverse link, explicit convergence and separation controls")
    gl("trials_variable_cloglog", "trials_edges.csv", "events", ["x"],
       binomial + [("target_represents_trials", True), ("trials_type", "Variable"),
                    ("trials_field", "trials"), ("link_function", "CLOGLOG")],
       "Events/trials with varying denominators and invalid pairs; complementary log-log")
    gl("trials_fixed_logc", "trials_edges.csv", "events", ["x"],
       binomial + [("target_represents_trials", True), ("trials_type", "FixedValue"),
                    ("trials_number", 20), ("link_function", "LOGC")],
       "Fixed trials ignore the raw trials column; log-complement inverse link")
    for link, name in [("NLOGLOG", "binary_nloglog"), ("ODDSPOWER", "binary_odds_power")]:
        props = binomial + [("link_function", link)]
        if link == "ODDSPOWER":
            props.append(("power", 0.5))
        else:
            props += [("method", "NewtonRaphson"), ("max_iterations", 1),
                      ("check_separation", False), ("estimates_change", False),
                      ("loglikelihood_change", False), ("hessian_convergence", False)]
        gl(name, "binary.csv", "event", ["x", "cat"], props,
           "Noncanonical binary inverse-link behavior on the same nonseparated data")
    gl("binary_propensities", "binary.csv", "event", ["x", "cat"],
       binomial + [("use_partitioned_data", True), ("calculate_raw_propensities", True),
                    ("calculate_adjusted_propensities", True), ("adjusted_propensity_partition", "Test")],
       "Training-only fit with raw and test-adjusted propensity scoring across all partitions",
       partition="part", nugget=[("calculate_raw_propensities", True),
                                 ("calculate_adjusted_propensities", True)])
    for link in ["CUMLOGIT", "CUMPROBIT", "CUMCLOGLOG", "CUMNLOGLOG", "CUMCAUCHIT"]:
        gl("ordinal_" + link.lower(), "ordinal.csv", "grade", ["x"],
           expert("MULTINOMIAL", link),
           "Three explicitly ordered categories, nonseparated groups; cumulative-link thresholds")
    return cases


def write_manifest():
    handle = open(OUT_DIR + "/manifest.json", "w")
    try:
        json.dump(manifest, handle, indent=2, sort_keys=True)
    finally:
        handle.close()


def main():
    try:
        data = make_datasets()
        cases = make_cases(data)
        manifest["planned_cases"] = len(cases)
        ensure_clean_dir(DATA_DIR, list(data.keys()) + [c["case"] + ".str" for c in cases])
        for name in sorted(data.keys()):
            write_csv(data[name])
        for c in cases:
            build_case(c, data[c["data"]])
    except ERRORS, e:
        manifest["errors"].append({"stage": "setup", "message": message(e)})
    finally:
        write_manifest()


main()
