# Time Series coverage, phase A. Coordinator runs this in Modeler 19 / Jython 2.7.
# Public modeler.api only. No IBM implementation inspection. Label: ts_est.
# Derived from ts_more_streams.py (2026-11-09): short ARIMA series for the Guide estimation path.
import json
import math
import xml.etree.ElementTree as ET
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/ts_est"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)
MODEL_NAME = "ts_est"
taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "ts_est", "saved": [], "errors": [], "cases": [], "inputs": []}

# OFFLINE_BEGIN -- Python 2.7 / 3.13 compatible, no Modeler calls.
N = 40
H = 6
SERIES_SCHEMA = [("t", "Integer", "Range", None),
                 ("d", "Date", "Range", None),
                 ("y", "Real", "Range", None),
                 ("x1", "Real", "Range", None),
                 ("x2", "Real", "Range", None)]
TS_BASE = [("custom_fields", True), ("targets", ["y"]), ("candidate_inputs", []),
           ("use_period", True), ("period_field", "t"), ("input_interval", "Month"),
           ("forecastperiods", H), ("extend_records_into_future", True),
           ("conf_limits", True), ("conf_limit_pct", 95.0), ("cal_PI", False),
           ("noise_res", False), ("detect_outliers", False),
           ("arima_detect_outliers", False), ("consider_newesmodels", False)]
DATE_BASED = [("use_period", False), ("date_time_field", "d"), ("input_interval", "Month")]


def month_date(index):
    return "%04d-%02d-01" % (2015 + index // 12, 1 + index % 12)


TS_FORECAST = [("Forecasts", [("extend_records_into_future", True),
                              ("forecastperiods", H), ("conf_limits", True)])]


def noise(seed, length):
    # Fixed integer LCG, then variance-one uniform innovations; no random package.
    state = seed
    result = []
    for i in range(length):
        state = (1664525 * state + 1013904223) % 4294967296
        result.append(math.sqrt(12.0) * ((state + 0.5) / 4294967296.0 - 0.5))
    return result


# Short series (40 months) so that Modeler's Marquardt estimation stops after few iterations.
# (name, seed, AR [(lag, coef)], MA [(lag, coef)] in the plus convention, differences, mean)
SERIES = [
    ("arma_a", 301, [(1, .6)], [(1, .3)], [], 50.),
    ("arma_b", 307, [(1, .4)], [(1, -.35)], [], 50.),
    ("arma_c", 311, [(1, -.3)], [(1, .5)], [], 50.),
    ("ari_a", 313, [(1, .5)], [(1, .3)], [1], .4),
    ("ari_b", 317, [(1, .3)], [(1, -.4)], [1], -.3),
    ("ari_c", 331, [(1, .7)], [(1, .2)], [1], .2),
    ("ima_a", 337, [], [(1, .45)], [1], .3),
    ("ima_b", 347, [], [(1, -.5)], [1], -.2),
    ("iar_a", 349, [(1, .5)], [], [1], .3),
    ("iar_b", 353, [(1, -.4)], [], [1], -.25),
    ("arma21", 359, [(1, .5), (2, -.3)], [(1, .4)], [], 50.),
    ("arma12", 367, [(1, .5)], [(1, .4), (2, .25)], [], 50.),
    ("air_a", 373, [], [(1, -.4), (12, -.45)], [1, 12], .02),
]


def generated_data():
    data = {}
    for name, seed, ar, ma, difference, mean in SERIES:
        ap, mp = [1.], [1.]
        for lag, v in ar:
            new = ap + [0.] * lag
            for j, old in enumerate(ap):
                new[j + lag] -= v * old
            ap = new
        for lag, v in ma:
            new = mp + [0.] * lag
            for j, old in enumerate(mp):
                new[j + lag] += v * old
            mp = new
        innovations = noise(seed, N + 400)
        w = []
        for i, e in enumerate(innovations):
            value = e
            for lag in range(1, len(ap)):
                value -= ap[lag] * (w[i - lag] if i >= lag else 0.)
            for lag in range(1, len(mp)):
                value += mp[lag] * (innovations[i - lag] if i >= lag else 0.)
            w.append(value)
        values = [v + mean for v in w[400:]]
        for lag in reversed(difference):
            integrated = []
            for i, v in enumerate(values):
                integrated.append(v + (integrated[i - lag] if i >= lag else 0.))
            values = integrated
        if difference:
            values = [80. + v for v in values]
        data[name] = values
    data["x1"] = [6. + 3.4 * v for v in noise(227, N + H)]
    data["x2"] = [6. + 3.4 * v for v in noise(229, N + H)]
    return data


def add_case(name, data, props, tests, fit, compare_to=None, inputs=None,
             keyed=None, apply=None):
    manifest["cases"].append({"case": name, "file": name + ".str", "family": "ts",
        "data": data + "_train.csv", "data_key": data, "targets": ["y"],
        "inputs": inputs or [], "properties": props, "keyed_properties": keyed or [],
        "apply_properties": apply or [], "tests": tests, "fit": fit,
        "compare_to": compare_to, "score_variants": TS_FORECAST, "status": "planned"})


def declare_cases():
    def arima(name, data, order, seasonal=(0, 0, 0), constant=True):
        props = [("method", "Arima"), ("arima_include_constant", constant)]
        saved_seasonal = seasonal if any(seasonal) else (0, 1, 1)
        for field, value in zip(["p", "d", "q", "sp", "sd", "sq"], list(order) + list(saved_seasonal)):
            props.append(("arima." + field, value))
        if any(seasonal):
            props.extend(DATE_BASED)
        fit = {"kind": "arima", "order": list(order), "seasonal": list(seasonal),
               "constant": constant, "transform": "None"}
        add_case("est_" + name, data, props, "Custom ARIMA estimation path; all orders explicit",
                 fit, None, keyed=[["arima_transformation_type", "y", "None"]])
    for key in ("arma_a", "arma_b", "arma_c"):
        arima(key, key, (1, 0, 1))
        arima(key + "_nc", key, (1, 0, 1), constant=False)
        arima(key + "_ar", key, (1, 0, 0))
        arima(key + "_ma", key, (0, 0, 1))
    for key in ("ari_a", "ari_b", "ari_c"):
        arima(key, key, (1, 1, 1))
        arima(key + "_nc", key, (1, 1, 1), constant=False)
    for key in ("ima_a", "ima_b"):
        arima(key, key, (0, 1, 1))
        arima(key + "_nc", key, (0, 1, 1), constant=False)
    for key in ("iar_a", "iar_b"):
        arima(key, key, (1, 1, 0))
        arima(key + "_nc", key, (1, 1, 0), constant=False)
    arima("arma21", "arma21", (2, 0, 1))
    arima("arma12", "arma12", (1, 0, 2))
    arima("air_a", "air_a", (0, 1, 1), (0, 1, 1))
    arima("air_a_nc", "air_a", (0, 1, 1), (0, 1, 1), False)
    if len(manifest["cases"]) != 30:
        raise ValueError("expected exactly 30 est cases, got %d" % len(manifest["cases"]))
# OFFLINE_END

BUILD_PROPERTIES = sorted(set([p for p, v in TS_BASE] + [
    "method", "expert_modeler_method", "consider_seasonal", "cyclic_periods",
    "exsmooth_model_type", "exsmooth_transformation_type", "arima.p", "arima.d", "arima.q",
    "arima.sp", "arima.sd", "arima.sq", "arima_transformation_type", "arima_include_constant",
    "futureValue_type_method", "extend_metric_values", "is_same_interval"]))
APPLY_PROPERTIES = ["extend_records_into_future", "ext_future_num", "compute_future_values_input",
                    "forecastperiods", "noise_res", "conf_limits", "target_fields",
                    "target_series", "includeTargets"]


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
        if storage == "Date":
            source.setKeyedPropertyValue("custom_date_format", field, "YYYY-MM-DD")
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


def declare_inputs():
    generated = generated_data()
    for key in sorted(generated):
        if key in ("x1", "x2"):
            continue
        values = generated[key]
        if len(values) != N or min(values) <= 0.:
            raise ValueError("target must have N positive observations: " + key)
        rows = [[i + 1, month_date(i), values[i], generated["x1"][i], generated["x2"][i]]
                for i in range(N)]
        write_csv(key + "_train.csv", SERIES_SCHEMA, rows, "%d monthly periods; fixed formulas in probe" % N)
    future = [[N + i + 1, month_date(N + i), None, generated["x1"][N + i], generated["x2"][N + i]]
              for i in range(H)]
    write_csv("predictor_future.csv", SERIES_SCHEMA, future, "Known predictors; audit file, not estimation input")
    return generated


def selected_specification(path):
    # Read only our own public PMML export, never Modeler implementation files.
    root = ET.parse(path).getroot()
    selection = []
    wanted = ["TimeSeriesModel", "ARIMA", "NonseasonalComponent", "SeasonalComponent",
              "ExponentialSmoothing", "Level", "Trend_ExpoSmooth", "Seasonality_ExpoSmooth"]
    for element in root.iter():
        tag = element.tag.split("}")[-1]
        if tag in wanted:
            selection.append({"element": tag, "attributes": dict(element.attrib)})
    if not selection:
        raise ValueError("PMML export has no Time Series specification")
    return selection


def build_case(entry, generated):
    stream, stage = None, "create_stream"
    try:
        stream = taskrunner.createStream(entry["case"], False, False)
        set_properties(stream, [("decimal_places", 15), ("decimal_symbol", "Period"),
            ("date_format", "YYYY-MM-DD"),
            ("enable_parallelism", False), ("create_model_applier_for_new_models", True),
            ("create_model_applier_update_links", "createEnabled")], entry)
        stage = "training_type"
        training = typed_source(stream, entry["data"], SERIES_SCHEMA, ["y"], entry["inputs"], {}, [], 100, entry)
        stage = "build_properties"
        builder = stream.createAt("ts", MODEL_NAME, 480, 100)
        stream.link(training, builder)
        entry["build_type"] = unicode(builder.getTypeName())
        entry["builder_defaults"] = read_properties(builder, BUILD_PROPERTIES)
        set_properties(builder, [("use_model_name", True), ("model_name", MODEL_NAME)] + TS_BASE + entry["properties"], entry)
        for prop, field, value in entry["keyed_properties"]:
            entry["setting_in_progress"] = [prop, field, value]
            builder.setKeyedPropertyValue(prop, field, value)
            entry.setdefault("keyed_readback", []).append([prop, field, snapshot(builder.getKeyedPropertyValue(prop, field))])
        entry.pop("setting_in_progress", None)
        if entry["inputs"]:
            futures = [[field, "USER_SPECIFY", generated[field][N:N + H]] for field in entry["inputs"]]
            entry["predictor_future_values"] = futures
            set_properties(builder, [("extend_metric_values", futures)], entry)
        entry["builder_effective"] = read_properties(builder, BUILD_PROPERTIES)
        # Save a diagnostic stream even if Modeler rejects a specification at training.
        stage = "save_untrained"
        taskrunner.saveStreamToFile(stream, DIR + "/" + entry["file"])
        entry["status"] = "untrained_saved"
        stage = "train"
        results = []
        builder.run(results)
        models = [r for r in results if isinstance(r, modeler.api.ModelOutput)]
        if len(models) != 1:
            raise RuntimeError("expected one TS ModelOutput, got %d" % len(models))
        # This PMML is also the Expert selection record. It includes coefficients,
        # transformation, orders, and initial states without reading IBM internals.
        stage = "export_pmml"
        pmml_name = entry["case"] + ".xml"
        try:
            taskrunner.exportModelToFile(models[0], DIR + "/" + pmml_name, modeler.api.FileFormat.XML)
            manifest["saved"].append(pmml_name)
            entry["pmml_file"] = pmml_name
            entry["selected_specification"] = selected_specification(DIR + "/" + pmml_name)
        except ERRORS as exc:
            manifest["errors"].append({"case": entry["case"], "stage": stage,
                                       "message": message(exc), "fatal": False})
        applier = find_applier(stream)
        if applier is None:
            applier = stream.createModelApplierAt(models[0], "Train scores", 480, 260)
            stream.link(training, applier)
        applier.setXYPosition(480, 260)
        entry["applier_defaults"] = read_properties(applier, APPLY_PROPERTIES)
        terminals = []
        entry["terminals"] = []
        variants = [("Train scores", [("extend_records_into_future", False), ("conf_limits", False)]),
                    ("Grid scores", [("extend_records_into_future", False), ("conf_limits", True)])] + TS_FORECAST
        stage = "scoring_properties"
        for index, (label, properties) in enumerate(variants):
            position = 260 + index * 140
            nugget = applier if index == 0 else stream.createModelApplierAt(models[0], label, 480, position)
            if index != 0:
                stream.link(training, nugget)
            setting = {"label": label, "properties": properties + [("noise_res", False)] + entry["apply_properties"]}
            set_properties(nugget, setting["properties"], setting)
            setting["effective"] = read_properties(nugget, APPLY_PROPERTIES)
            setting["applier_type"] = unicode(nugget.getTypeName())
            table = stream.createAt("table", label, 760, position)
            stream.link(nugget, table)
            setting["table_id"] = unicode(table.getID())
            entry["terminals"].append(setting)
            terminals.append(table)
        stage = "save_stream"
        taskrunner.saveStreamToFile(stream, DIR + "/" + entry["file"])
        manifest["saved"].append(entry["file"])
        entry["status"] = "saved"
        stage = "score"
        for index, table in enumerate(terminals):
            try:
                outputs = []
                table.run(outputs)
                documents = [r for r in outputs if isinstance(r, modeler.api.DocumentOutput)]
                if len(documents) != 1:
                    raise RuntimeError("expected one scoring Table DocumentOutput")
                # Keep directly reviewable scores as well as the saved stream's
                # three terminals for the coordinator's normal recording pass.
                score_name = "%s_scores_%d.csv" % (entry["case"], index)
                taskrunner.exportDocumentToFile(documents[0], DIR + "/" + score_name,
                                               modeler.api.FileFormat.COMMA_DELIMITED)
                entry["terminals"][index]["csv_file"] = score_name
                manifest["saved"].append(score_name)
            except ERRORS as exc:
                entry["status"] = "saved_with_score_errors"
                manifest["errors"].append({"case": entry["case"], "stage": stage,
                    "table": unicode(table.getLabel()), "message": message(exc)})
    except ERRORS as exc:
        entry["status"] = "failed"
        manifest["errors"].append({"case": entry["case"], "stage": stage, "message": message(exc)})
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS as exc:
                manifest["errors"].append({"case": entry["case"], "stage": "clear", "message": message(exc)})
        write_manifest()


try:
    ensure_dir(DIR)
    generated = declare_inputs()
    declare_cases()
    write_manifest()
    for case in manifest["cases"]:
        build_case(case, generated)
except ERRORS as exc:
    manifest["errors"].append({"case": None, "stage": "setup", "message": message(exc)})
finally:
    write_manifest()
