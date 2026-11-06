# GLE binomial identity/log iteration probe (coordinator). Modeler 19 / Jython 2.7,
# `cargo xtask probe gle2_streams`. Label: gle2. Same data as gle; iteration limits and
# estimation methods vary so Modeler reports its intermediate iterates. Public modeler.api only.
# Python 2 syntax only. Do not delete corpus files; skip a case whose .str exists.
import json
import math
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/gle2"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)
DATA_FILE = "gle_train.csv"
taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "gle2", "saved": [], "errors": [], "cases": [], "inputs": []}

SCHEMA = [
    ("id", "Integer", "Range", None),
    ("x", "Real", "Range", None),
    ("g", "String", "Set", ["A", "B", "C"]),
    ("y_normal", "Real", "Range", None),
    ("y_gamma", "Real", "Range", None),
    ("y_poisson", "Integer", "Range", None),
    ("y_flag", "String", "Flag", ["F", "T"]),
]
BUILD_PROPERTIES = [
    "custom_target", "target_field", "use_trials", "use_trials_field_or_value",
    "trials_field", "trials_value", "use_custom_target_reference",
    "target_reference_value", "dist_link_combination", "target_distribution",
    "link_function_type", "link_function_param", "tweedie_param",
    "use_predefined_inputs", "model_effects_list", "use_intercept",
    "regression_weight_field", "use_offset", "offset_value", "offset_field",
    "target_category_order", "inputs_category_order", "max_iterations",
    "confidence_level", "test_fixed_effects_coefficients", "detect_outliers",
    "conduct_trend_analysis", "estimation_method", "max_fisher_iterations",
    "scale_parameter_method", "scale_value", "negative_binomial_method",
    "negative_binomial_value", "non_neg_least_squares", "use_p_converge",
    "p_converge", "p_converge_type", "use_l_converge", "l_converge",
    "l_converge_type", "use_h_converge", "h_converge", "h_converge_type",
    "sing_tolerance", "use_model_selection", "method",
    "detect_two_way_interactions", "automatic_penalty_params",
    "lasso_penalty_param", "elastic_net_penalty_param1",
    "elastic_net_penalty_param2", "probability_entry", "probability_removal",
    "use_max_effects", "max_effects", "use_max_steps", "max_steps",
    "use_model_name", "model_name", "usePI", "custom_fields", "targets",
    "inputs", "use_partitioned_data",
]
APPLY_PROPERTIES = ["enable_sql_generation"]


def noise(seed):
    state = (1664525 * seed + 1013904223) % 4294967296
    return math.sqrt(12.0) * ((state + 0.5) / 4294967296.0 - 0.5)


def generated_rows():
    rows = []
    for i in range(24):
        x = 0.5 + 0.5 * float(i)
        gi = i % 3
        g = ["A", "B", "C"][gi]
        e = noise(1000 + i)
        g_n = [0.0, 1.5, -1.0][gi]
        g_g = [0.0, 0.2, -0.15][gi]
        g_p = [0.0, 0.3, -0.2][gi]
        g_b = [0.0, 1.0, -0.5][gi]
        y_normal = 4.0 + 0.8 * x + g_n + 0.15 * e
        y_gamma = math.exp(0.4 + 0.08 * x + g_g + 0.05 * e)
        lam = math.exp(0.2 + 0.12 * x + g_p)
        y_poisson = int(lam + 0.5)
        logit = -2.0 + 0.25 * x + g_b
        y_flag = "T" if logit > 0.0 else "F"
        row = [i + 1, x, g, y_normal, y_gamma, y_poisson, y_flag]
        if i in (5, 13):
            row[1] = None
        if i == 15:
            row[2] = None
        if i == 7:
            row[3] = None
        if i == 8:
            row[4] = None
        if i == 9:
            row[5] = None
        if i == 10:
            row[6] = None
        rows.append(row)
    return rows


def declare_cases():
    # Binomial identity/log on y ~ x, as gle_binom_identity / gle_binom_log.
    cases = []
    for link, tag in (("IDENTITY", "id"), ("LOG", "log")):
        base = [("dist_link_combination", "CUSTOM"), ("target_distribution", "Binomial"),
                ("link_function_type", link)]
        for iters in (0, 1, 2, 3, 5, 8):
            cases.append({
                "case": "g2_%s_it%d" % (tag, iters), "file": "g2_%s_it%d.str" % (tag, iters),
                "target": "y_flag", "inputs": ["x"],
                "properties": base + [("max_iterations", iters)], "apply_properties": [],
                "tests": "binomial %s, max_iterations %d (0 = initial values)" % (link, iters),
            })
        for method in ("FISHER_SCORING", "NEWTON_RAPHSON"):
            cases.append({
                "case": "g2_%s_%s" % (tag, method.lower()), "file": "g2_%s_%s.str" % (tag, method.lower()),
                "target": "y_flag", "inputs": ["x"],
                "properties": base + [("estimation_method", method)], "apply_properties": [],
                "tests": "binomial %s, estimation_method %s" % (link, method),
            })
    if len(cases) > 18:
        raise ValueError("at most 18 cases")
    for case in cases:
        case["data"] = DATA_FILE
        case["status"] = "planned"
        manifest["cases"].append(case)


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


def csv_cell(value):
    if value is None:
        return ""
    if isinstance(value, basestring):
        return '"' + value.replace('"', '""') + '"'
    if isinstance(value, float):
        return "%.17g" % value
    return str(value)


def write_csv():
    path = DIR + "/" + DATA_FILE
    if java.io.File(path).exists():
        manifest["inputs"].append({"file": DATA_FILE, "status": "skipped_existing"})
        return
    rows = generated_rows()
    lines = [",".join([field[0] for field in SCHEMA])]
    for row in rows:
        lines.append(",".join([csv_cell(value) for value in row]))
    handle = open(path, "wb")
    try:
        handle.write((NL.join(lines) + NL).encode("utf-8"))
    finally:
        handle.close()
    manifest["saved"].append(DATA_FILE)
    manifest["inputs"].append({"file": DATA_FILE, "rows": len(rows)})


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


def instantiate_node(stream, node):
    table = stream.createAt("table", "Instantiate", 1000, 800)
    stream.link(node, table)
    try:
        table.run([])
    finally:
        stream.delete(table)


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


def write_manifest():
    handle = open(OUT_DIR + "/manifest.json", "wb")
    try:
        handle.write((json.dumps(manifest, indent=2, sort_keys=True) + NL).encode("utf-8"))
    finally:
        handle.close()


def ensure_dir(path):
    directory = java.io.File(path)
    if not directory.exists() and not directory.mkdirs():
        raise IOError("cannot create " + path)


def typed_source(stream, target, inputs, entry):
    source = stream.createAt("variablefile", DATA_FILE, 60, 100)
    source_props = [("full_filename", DIR + "/" + DATA_FILE), ("read_field_names", True),
                    ("delimit_comma", True), ("delimit_space", False), ("delimit_tab", False),
                    ("quotes_2", "PairAndDiscard"), ("strip_spaces", "None"),
                    ("decimal_symbol", "Period"), ("encoding", "UTF-8"),
                    ("auto_recognize_datetime", False), ("lines_to_scan", 1000)]
    source_entry = {"data": DATA_FILE, "properties": source_props}
    entry["source"] = source_entry
    set_properties(source, source_props, source_entry)
    instantiate_node(stream, source)
    for field, storage, measure, values in SCHEMA:
        source.setKeyedPropertyValue("custom_storage_type", field, storage)

    node = stream.createAt("type", "Type", 260, 100)
    stream.link(source, node)
    instantiate_node(stream, node)
    type_entry = {"fields": []}
    entry["type"] = type_entry
    for field, storage, measure, values in SCHEMA:
        node.setKeyedPropertyValue("type", field, measure)
        if values is not None:
            node.setKeyedPropertyValue("values", field, values)
            node.setKeyedPropertyValue("extend_values", field, False)
        direction = "None"
        if field == target:
            direction = "Target"
        elif field in inputs:
            direction = "Input"
        node.setKeyedPropertyValue("direction", field, direction)
        node.setKeyedPropertyValue("check", field, "None")
        if storage == "Real":
            node.setKeyedPropertyValue("export_places", field, 15)
    instantiate_node(stream, node)
    for field, storage, measure, values in SCHEMA:
        actual = {"name": field}
        for prop in ["storage", "type", "direction", "values"]:
            try:
                actual[prop] = snapshot(node.getKeyedPropertyValue(prop, field))
            except ERRORS as exc:
                actual[prop] = {"read_error": message(exc)}
        type_entry["fields"].append(actual)
    return node


def build_case(entry):
    stream_path = DIR + "/" + entry["file"]
    if java.io.File(stream_path).exists():
        entry["status"] = "skipped_existing"
        write_manifest()
        return
    stream, stage = None, "create_stream"
    try:
        stream = taskrunner.createStream(entry["case"], False, False)
        set_properties(stream, [("decimal_places", 15), ("decimal_symbol", "Period"),
            ("enable_parallelism", False), ("create_model_applier_for_new_models", True),
            ("create_model_applier_update_links", "createEnabled")], entry)
        stage = "training_type"
        training = typed_source(stream, entry["target"], entry["inputs"], entry)
        stage = "create_builder"
        try:
            builder = stream.createAt("gle", entry["case"], 480, 100)
        except ERRORS as exc:
            entry["status"] = "failed"
            entry["batch_execute"] = "refused"
            entry["batch_error"] = message(exc)
            entry["batch_refusal_note"] = (
                "GLE builder could not be created in Modeler 19 batch. "
                "createAt('gle') failed; the error is in batch_error.")
            manifest["errors"].append({"case": entry["case"], "stage": stage,
                                       "message": message(exc)})
            write_manifest()
            return
        stream.link(training, builder)
        entry["build_type"] = unicode(builder.getTypeName())
        entry["builder_defaults"] = read_properties(builder, BUILD_PROPERTIES)
        stage = "build_properties"
        set_properties(builder, entry["properties"], entry)
        entry["builder_effective"] = read_properties(builder, BUILD_PROPERTIES)
        stage = "save_untrained"
        taskrunner.saveStreamToFile(stream, stream_path)
        entry["status"] = "untrained_saved"
        stage = "train"
        results = []
        try:
            builder.run(results)
            entry["batch_execute"] = "ok"
            entry["run_result_count"] = len(results)
        except ERRORS as exc:
            entry["batch_execute"] = "refused"
            entry["batch_error"] = message(exc)
            if entry["case"] == "gle_batch_probe":
                entry["batch_refusal_note"] = (
                    "GLE builder refused to execute in Modeler 19 batch. "
                    "Type name and defaults are in build_type / builder_defaults.")
            manifest["errors"].append({"case": entry["case"], "stage": stage,
                                       "message": message(exc)})
            entry["status"] = "saved_untrained_batch_refused"
            write_manifest()
            return
        models = [r for r in results if isinstance(r, modeler.api.ModelOutput)]
        stage = "nugget"
        applier = find_applier(stream)
        if applier is None:
            if len(models) != 1:
                raise RuntimeError("expected one GLE ModelOutput, got %d" % len(models))
            applier = stream.createModelApplierAt(models[0], "Scores", 480, 260)
            stream.link(training, applier)
        applier.setXYPosition(480, 260)
        entry["applier_type"] = unicode(applier.getTypeName())
        entry["applier_defaults"] = read_properties(applier, APPLY_PROPERTIES)
        set_properties(applier, entry["apply_properties"], entry)
        entry["applier_effective"] = read_properties(applier, APPLY_PROPERTIES)
        table = stream.createAt("table", "Scores", 760, 260)
        stream.link(applier, table)
        entry["table_id"] = unicode(table.getID())
        stage = "save_stream"
        taskrunner.saveStreamToFile(stream, stream_path)
        manifest["saved"].append(entry["file"])
        entry["status"] = "saved"
        stage = "score"
        outputs = []
        table.run(outputs)
        documents = [r for r in outputs if isinstance(r, modeler.api.DocumentOutput)]
        if len(documents) != 1:
            raise RuntimeError("expected one scoring Table DocumentOutput")
        score_name = entry["case"] + "_scores.csv"
        taskrunner.exportDocumentToFile(documents[0], DIR + "/" + score_name,
                                       modeler.api.FileFormat.COMMA_DELIMITED)
        entry["csv_file"] = score_name
        manifest["saved"].append(score_name)
        if len(models) == 1:
            stage = "export_pmml"
            pmml_name = entry["case"] + ".xml"
            try:
                taskrunner.exportModelToFile(models[0], DIR + "/" + pmml_name,
                                            modeler.api.FileFormat.XML)
                manifest["saved"].append(pmml_name)
                entry["pmml_file"] = pmml_name
            except ERRORS as exc:
                manifest["errors"].append({"case": entry["case"], "stage": stage,
                                           "message": message(exc), "fatal": False})
    except ERRORS as exc:
        entry["status"] = "failed"
        if "batch_execute" not in entry:
            entry["batch_execute"] = "refused"
            entry["batch_error"] = message(exc)
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


try:
    ensure_dir(DIR)
    write_csv()
    declare_cases()
    write_manifest()
    for case in manifest["cases"]:
        build_case(case)
except ERRORS as exc:
    manifest["errors"].append({"case": None, "stage": "setup", "message": message(exc)})
finally:
    write_manifest()
