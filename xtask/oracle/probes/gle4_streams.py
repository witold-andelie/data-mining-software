# GLE binomial identity/log probe, round 4 (coordinator): max_iterations (saved as
# _interEffMaxOuterIters) and estimation method on the out-of-range datasets of gle3. Modeler 19 / Jython 2.7,
# `cargo xtask probe gle4_streams`. Label: gle4. Five 0/1 targets on one x whose Guide
# starting fit has 0-3 means outside (0, 1); max_iterations 1 exposes the first iterate. Public modeler.api only.
# Python 2 syntax only. Do not delete corpus files; skip a case whose .str exists.
import json
import math
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/gle4"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)
DATA_FILE = "gle3_train.csv"
taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "gle4", "saved": [], "errors": [], "cases": [], "inputs": []}

SCHEMA = [
    ("id", "Integer", "Range", None),
    ("x", "Real", "Range", None),
    ("yA", "String", "Flag", ["F", "T"]),
    ("yB", "String", "Flag", ["F", "T"]),
    ("yC", "String", "Flag", ["F", "T"]),
    ("yD", "String", "Flag", ["F", "T"]),
    ("yE", "String", "Flag", ["F", "T"]),
]
# 1 = F (the modelled category; T is the reference), 0 = T.  Guide start (identity link =
# OLS on x): A all means inside (0, 1); B 1 outside; C 2; D 3.  E: log-link start inside.
PATTERNS = {
    "yA": "00011000001000011111",
    "yB": "10000001111100111111",
    "yC": "00000000011111101011",
    "yD": "00000001000011111011",
    "yE": "00100010000110010000",
}
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
    for i in range(20):
        row = [i + 1, 0.5 * float(i + 1)]
        for name in ("yA", "yB", "yC", "yD", "yE"):
            row.append("F" if PATTERNS[name][i] == "1" else "T")
        rows.append(row)
    return rows


def declare_cases():
    cases = []
    def add(tag, target, link, extra, tests):
        cases.append({
            "case": "g4_" + tag, "file": "g4_" + tag + ".str",
            "target": target, "inputs": ["x"],
            "properties": [("dist_link_combination", "CUSTOM"), ("target_distribution", "Binomial"),
                           ("link_function_type", link)] + extra,
            "apply_properties": [], "tests": tests,
        })
    for t in ("yB", "yC", "yD"):
        for n in (2, 3, 5):
            add("id_%s_mi%d" % (t[1].lower(), n), t, "IDENTITY", [("max_iterations", n)],
                "identity, max_iterations %d" % n)
    for n in (2, 3, 5):
        add("log_d_mi%d" % n, "yD", "LOG", [("max_iterations", n)], "log, max_iterations %d" % n)
    for n in (4, 10):
        add("id_c_mi%d" % n, "yC", "IDENTITY", [("max_iterations", n)], "identity, max_iterations %d" % n)
    for t in ("yC", "yD"):
        for method in ("FISHER_SCORING", "NEWTON_RAPHSON"):
            add("id_%s_%s" % (t[1].lower(), method.lower()[:6]), t, "IDENTITY",
                [("estimation_method", method)], "identity, %s" % method)
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
