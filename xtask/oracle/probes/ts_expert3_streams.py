# Time Series Expert Modeler: round 3 automatic model selection probe. Coordinator runs this in
# Modeler 19 / Jython 2.7 with `cargo xtask probe ts_expert3_streams`. Label: ts_expert3.
# Public modeler.api only. No IBM implementation inspection.
#
# Purpose: discriminate remaining ARIMA Expert hypotheses after Round 3 resolved the Exponential
# Smoothing Expert (22/22) and the AllModels cross-family NBIC rule (see target/ts-expert/REPORT3.md
# and PREDICTIONS3.md). Every case is the same stream shape as ts_expert2 (Type -> Time Series
# builder -> nugget + three tables), with the Expert Modeler restricted to exponential smoothing,
# to ARIMA, or free (All). The selected specification is the exported PMML, or the saved nugget if
# XML export fails.
import json
import math
import xml.etree.ElementTree as ET
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/ts_expert3"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)
MODEL_NAME = "ts_expert3"
taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "ts_expert3", "saved": [], "errors": [], "cases": [], "inputs": []}

# OFFLINE_BEGIN -- Python 2.7 / 3.13 compatible, no Modeler calls. target/ts-expert/lab3.py
# executes exactly this block to build the series and the case list for PREDICTIONS3.md.
H = 6
SERIES_SCHEMA = [("t", "Integer", "Range", None),
                 ("d", "Date", "Range", None),
                 ("y", "Real", "Range", None)]
TS_BASE = [("custom_fields", True), ("targets", ["y"]), ("candidate_inputs", []),
           ("use_period", True), ("period_field", "t"), ("input_interval", "Month"),
           ("forecastperiods", H), ("extend_records_into_future", True),
           ("conf_limits", True), ("conf_limit_pct", 95.0), ("cal_PI", False),
           ("noise_res", False), ("detect_outliers", False),
           ("arima_detect_outliers", False), ("consider_newesmodels", False)]
DATE_BASED = [("use_period", False), ("date_time_field", "d"), ("input_interval", "Month")]
TS_FORECAST = [("Forecasts", [("extend_records_into_future", True),
                              ("forecastperiods", H), ("conf_limits", True)])]


def month_date(index):
    return "%04d-%02d-01" % (2015 + index // 12, 1 + index % 12)


def noise(seed, length):
    state = seed
    result = []
    for i in range(length):
        state = (1664525 * state + 1013904223) % 4294967296
        result.append(math.sqrt(12.0) * ((state + 0.5) / 4294967296.0 - 0.5))
    return result


def arma(seed, n, ar, ma, burn=300):
    e = noise(seed, n + burn)
    w = []
    for i in range(n + burn):
        v = e[i]
        for lag, c in ar.items():
            if i >= lag:
                v += c * w[i - lag]
        for lag, c in ma.items():
            if i >= lag:
                v += c * e[i - lag]
        w.append(v)
    return w[burn:]


# Ten complete family triplets (30 cases total).
SERIES = {
    "sqrthet": (96, "quadratic trend with square-root proportional noise; tests whether Expert ARIMA ever chooses sqrt vs log/none"),
    "off200": (96, "growth series plus 200; intermediate relative-spread bracket for log transform gate"),
    "off1000": (96, "growth series plus 1000; upper offset bracket for log transform gate"),
    "linehet": (96, "linear trend with decreasing innovation variance; tests Jacobian-variance vs range-based log gate"),
    "ar188": (96, "near-unit-root AR(1) with ADF_c(1) around -2.45; brackets unit-root differencing threshold between rwflat and rwunit"),
    "ima11nc": (96, "zero-drift IMA(1,1); tests whether constant is dropped at d=1 when q=1 and drift t-stat is insignificant"),
    "ar2": (120, "only AR lag 2; tests whether subset lag pruning produces sparse AR([2],0,0) like MA(0,0,[2])"),
    "ma3": (120, "only MA lag 3; tests whether initial MA order search reaches lag 3 and prunes lags 1..2"),
    "seas36": (36, "3-season (36-month) additive seasonal prefix; tests whether seasonal ARIMA activates at N=3s"),
    "seas48": (48, "4-season (48-month) additive seasonal prefix; tests seasonal ARIMA activation and D=0 vs D=1 at N=4s"),
}


def generated_data():
    data = {}
    data["sqrthet"] = [(6.0 + 0.08 * i + 0.25 * v) ** 2 for i, v in enumerate(noise(503, 96))]
    growth = [math.exp(3.0 + 0.03 * i + 0.08 * v) for i, v in enumerate(noise(433, 96))]
    data["off200"] = [200.0 + v for v in growth]
    data["off1000"] = [1000.0 + v for v in growth]
    data["linehet"] = [40.0 + 0.5 * i + (1.6 if i < 48 else 0.8) * v for i, v in enumerate(noise(509, 96))]
    data["ar188"] = [100.0 + v for v in arma(521, 96, {1: 0.92}, {})]
    e_ima = noise(523, 97)
    level = 100.0
    ima_vals = []
    for i in range(96):
        level += e_ima[i + 1] - 0.65 * e_ima[i]
        ima_vals.append(level)
    data["ima11nc"] = ima_vals
    data["ar2"] = [100.0 + v for v in arma(541, 120, {2: 0.72}, {})]
    data["ma3"] = [100.0 + v for v in arma(547, 120, {}, {3: 0.75})]
    pattern = [8.0 * math.sin(2.0 * math.pi * j / 12.0) + 3.0 * math.cos(4.0 * math.pi * j / 12.0) for j in range(12)]
    seas_full = [100.0 + pattern[i % 12] + 1.5 * v for i, v in enumerate(noise(557, 48))]
    data["seas36"] = seas_full[:36]
    data["seas48"] = seas_full[:48]
    return data


def declare_cases():
    for key in ["sqrthet", "off200", "off1000", "linehet", "ar188", "ima11nc", "ar2", "ma3", "seas36", "seas48"]:
        seasonal = key in ["seas36", "seas48"]
        for suffix, family in [("es", "Exsmooth"), ("ar", "Arima"), ("all", "All")]:
            name = "p3_" + key + "_" + suffix
            props = [("method", "ExpertModeler"), ("consider_seasonal", seasonal)]
            if family != "All":
                props.append(("expert_modeler_method", family))
            if seasonal:
                props.extend(DATE_BASED)
            manifest["cases"].append({"case": name, "file": name + ".str", "family": "ts",
                "data": key + "_train.csv", "data_key": key, "targets": ["y"], "inputs": [],
                "properties": props, "keyed_properties": [], "apply_properties": [],
                "tests": SERIES[key][1] + "; compare all three family restrictions",
                "fit": {"kind": "expert", "family": family, "seasonal": seasonal},
                "compare_to": None, "score_variants": TS_FORECAST, "status": "planned"})
    if len(manifest["cases"]) > 30:
        raise ValueError("at most 30 cases: Modeler server memory limit")
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
        values = generated[key]
        if len(values) != SERIES[key][0] or min(values) <= 0.:
            raise ValueError("series length or positivity violated: " + key)
        rows = [[i + 1, month_date(i), values[i]] for i in range(len(values))]
        write_csv(key + "_train.csv", SERIES_SCHEMA, rows, SERIES[key][1])
    return generated


def selected_specification(path):
    root = ET.parse(path).getroot()
    selection = []
    wanted = ["TimeSeriesModel", "ARIMA", "NonseasonalComponent", "SeasonalComponent",
              "ExponentialSmoothing", "Level", "Trend_ExpoSmooth", "Seasonality_ExpoSmooth"]
    for element in root.iter():
        tag = element.tag.split("}")[-1]
        if tag in wanted:
            record = {"element": tag, "attributes": dict(element.attrib)}
            if tag in ["NonseasonalComponent", "SeasonalComponent"]:
                record["coefficients"] = []
                for child in element:
                    component = child.tag.split("}")[-1]
                    if component in ["AR", "MA"]:
                        for array in child.iter():
                            if array.tag.split("}")[-1] == "Array":
                                record["coefficients"].append({"component": component,
                                    "attributes": dict(array.attrib), "text": array.text})
            selection.append(record)
        elif tag == "Extension" and element.get("name") in ["degreesOfFreedom", "errVariance", "periodLength"]:
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
            entry.setdefault("keyed_readback", []).append(
                [prop, field, snapshot(builder.getKeyedPropertyValue(prop, field))])
        entry.pop("setting_in_progress", None)
        entry["builder_effective"] = read_properties(builder, BUILD_PROPERTIES)
        stage = "save_untrained"
        taskrunner.saveStreamToFile(stream, DIR + "/" + entry["file"])
        entry["status"] = "untrained_saved"
        stage = "train"
        results = []
        builder.run(results)
        models = [r for r in results if isinstance(r, modeler.api.ModelOutput)]
        if len(models) != 1:
            raise RuntimeError("expected one TS ModelOutput, got %d" % len(models))
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
