# GLE binomial identity/log probe, round 5 (coordinator): where does Modeler stop on the G data
# (corpus/gle/gle_train.csv, 21 valid records)? Under the Guide-literal Fisher switch all six gle2
# G runs lie on our IRLS path and Modeler stops early (gle-lab NOTES round 6). This probe varies
# max_fisher_iterations (K = 2..5) so the stop lands at different points, and repeats the G data
# twice (n = 42) to separate a count rule from a fraction rule. Modeler 19 / Jython 2.7,
# `cargo xtask probe gle5_streams`. Label: gle5. Public modeler.api only.
# Python 2 syntax only. Do not delete corpus files; skip a case whose .str exists.
import json
import math
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/gle5"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)
DATA_FILES = {"gle5_train.csv": 1, "gle5_double.csv": 2}
taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "gle5", "saved": [], "errors": [], "cases": [], "inputs": []}

SCHEMA = [
    ("id", "Integer", "Range", None),
    ("x", "Real", "Range", None),
    ("y", "String", "Flag", ["F", "T"]),
]
# The 21 valid (x, y_flag) records of corpus/gle/gle_train.csv, in file order.
G_ROWS = [(0.5, "F"), (1.0, "F"), (1.5, "F"), (2.0, "F"), (2.5, "F"), (3.5, "F"), (4.0, "F"),
          (4.5, "F"), (5.0, "F"), (6.0, "F"), (6.5, "F"), (7.5, "F"), (8.0, "F"), (8.5, "T"),
          (9.0, "F"), (9.5, "T"), (10.0, "T"), (10.5, "T"), (11.0, "T"), (11.5, "T"), (12.0, "T")]
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


def generated_rows(copies):
    rows = []
    for c in range(copies):
        for x, y in G_ROWS:
            rows.append([len(rows) + 1, x, y])
    return rows


def declare_cases():
    cases = []
    def add(tag, data, link, extra, tests):
        cases.append({
            "case": "g5_" + tag, "file": "g5_" + tag + ".str", "data": data,
            "target": "y", "inputs": ["x"],
            "properties": [("dist_link_combination", "CUSTOM"), ("target_distribution", "Binomial"),
                           ("link_function_type", link)] + extra,
            "apply_properties": [], "tests": tests,
        })
    for link in ("IDENTITY", "LOG"):
        short = link.lower()[:3]
        for k in (2, 3, 4, 5):
            add("%s_k%d" % (short, k), "gle5_train.csv", link, [("max_fisher_iterations", k)],
                "%s, hybrid with %d Fisher iterations" % (link, k))
    for link in ("IDENTITY", "LOG"):
        short = link.lower()[:3]
        add("%s_dbl" % short, "gle5_double.csv", link, [], "%s, default hybrid, data twice" % link)
        for method in ("FISHER_SCORING", "NEWTON_RAPHSON"):
            add("%s_dbl_%s" % (short, method.lower()[:6]), "gle5_double.csv", link,
                [("estimation_method", method)], "%s, %s, data twice" % (link, method))
    if len(cases) > 18:
        raise ValueError("at most 18 cases")
    for case in cases:
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
    for name in sorted(DATA_FILES.keys()):
        path = DIR + "/" + name
        if java.io.File(path).exists():
            manifest["inputs"].append({"file": name, "status": "skipped_existing"})
            continue
        rows = generated_rows(DATA_FILES[name])
        lines = [",".join([field[0] for field in SCHEMA])]
        for row in rows:
            lines.append(",".join([csv_cell(value) for value in row]))
        handle = open(path, "wb")
        try:
            handle.write((NL.join(lines) + NL).encode("utf-8"))
        finally:
            handle.close()
        manifest["saved"].append(name)
        manifest["inputs"].append({"file": name, "rows": len(rows)})


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
    data_file = entry["data"]
    source = stream.createAt("variablefile", data_file, 60, 100)
    source_props = [("full_filename", DIR + "/" + data_file), ("read_field_names", True),
                    ("delimit_comma", True), ("delimit_space", False), ("delimit_tab", False),
                    ("quotes_2", "PairAndDiscard"), ("strip_spaces", "None"),
                    ("decimal_symbol", "Period"), ("encoding", "UTF-8"),
                    ("auto_recognize_datetime", False), ("lines_to_scan", 1000)]
    source_entry = {"data": data_file, "properties": source_props}
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
