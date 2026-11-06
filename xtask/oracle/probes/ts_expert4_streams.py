# Time Series Expert round 4: 24 discriminating cases; coordinator-run only.
# Label ts_expert4, case prefix p4_. Public Modeler API, deterministic input data.
# Frozen hypotheses and numeric margins: target/ts-expert/PREDICTIONS4.md.
# Never execute this file in offline analysis; only its marked pure-data block is evaluated.
import json
import math
import xml.etree.ElementTree as ET
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/ts_expert4"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)
MODEL_NAME = "ts_expert4"
taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "ts_expert4", "saved": [], "errors": [], "cases": [], "inputs": []}

# OFFLINE_BEGIN -- Python 2.7 / 3.13 compatible, no Modeler calls. target/ts-expert/probe4.py
# executes exactly this block to build the series and the case list for PREDICTIONS4.md.
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



# Twenty series: 18 AR-only controls and two complete family triplets.
RECIPES = {'off500': ['growth', 433, 96, 500, 1], 'off800': ['growth', 433, 96, 800, 1], 'scale500': ['growth', 433, 96, 500, 10], 'adfcrit': ['arma', 611, 96, {'1': 0.92}, {}], 'adflag': ['arma', 617, 96, {'1': 0.92}, {}], 'seasonweak': ['season', 617, 96, 3.5, 2.0, 0.0], 'seasonmid': ['season', 617, 96, 4.0, 2.0, 0.0], 'difforder': ['air', 619, 120, -0.5, -0.6, 0.0], 'weakma1': ['arma', 639, 120, {}, {'1': 0.2}], 'weakma2': ['arma', 648, 120, {}, {'1': 0.2}], 'ma4': ['arma', 651, 144, {}, {'4': 0.72}], 'ma5': ['arma', 652, 144, {}, {'5': 0.72}], 'arstart85': ['arma', 661, 144, {'1': 0.85}, {}], 'arstart92': ['arma', 673, 144, {'1': 0.92}, {}], 'airzero': ['air', 677, 120, -0.5, -0.6, 0.0], 'imazero': ['ima', 691, 120, -0.5, 0.06365018128162621], 'gate35': ['season', 683, 35, 8.0, 1.5, 0.0], 'gate36': ['season', 683, 36, 8.0, 1.5, 0.0], 'scaleall1': ['rwperturb', 307, 96, 0.1, 769], 'scaleall2': ['rwperturb', 307, 96, 0.2, 769]}

SERIES = {'off500': (96, 'Intermediate translation brackets log spread gate; scale500 is an exact x10 scale control of off500.'), 'off800': (96, 'Intermediate translation brackets log spread gate; scale500 is an exact x10 scale control of off500.'), 'scale500': (96, 'Intermediate translation brackets log spread gate; scale500 is an exact x10 scale control of off500.'), 'adfcrit': (96, 'Same lag selection, competing 10% versus 5%-like critical constants; conditional simple AR/drift starts.'), 'adflag': (96, 'ADF augmentation selection changes differencing with more than .2 statistic separation.'), 'seasonweak': (96, 'Seasonal variance-reduction rule versus a lag-12 unit-root regression; selected D is the primary outcome.'), 'seasonmid': (96, 'Seasonal variance-reduction rule versus a lag-12 unit-root regression; selected D is the primary outcome.'), 'difforder': (120, 'Airline construction tests order of ordinary and seasonal unit-root decisions, holding transfer/transform absent.'), 'weakma1': (120, 'Weak but significant MA coefficient separates 1.96 term pruning from BIC deletion.'), 'weakma2': (120, 'Weak but significant MA coefficient separates 1.96 term pruning from BIC deletion.'), 'ma4': (144, 'Strong isolated late MA lag brackets the initial-order scan and tests sparse retained arrays.'), 'ma5': (144, 'Strong isolated late MA lag brackets the initial-order scan and tests sparse retained arrays.'), 'arstart85': (144, 'Conditional d=0 AR1 versus MA4 initial family; report ADF as a competing preprocessing diagnostic.'), 'arstart92': (144, 'Conditional d=0 AR1 versus MA4 initial family; report ADF as a competing preprocessing diagnostic.'), 'airzero': (120, 'Seasonally integrated zero-drift series tests unconditional retention versus a seasonal significance test.'), 'imazero': (120, 'Drift is adjusted offline to give conditional fitted mean zero; unlike p3_ima11nc, fitted-model t_c should be insignificant.'), 'gate35': (35, 'Shared-prefix boundary controls use only the documented 3s threshold; distinguish frequency suppression from D selection.'), 'gate36': (36, 'Shared-prefix boundary controls use only the documented 3s threshold; distinguish frequency suppression from D selection.'), 'scaleall1': (96, 'Full family triplet: seed-307 random walk plus new seed-769 noise, two amplitudes; interior AR2 gives opposite raw/weighted winners.'), 'scaleall2': (96, 'Full family triplet: seed-307 random walk plus new seed-769 noise, two amplitudes; interior AR2 gives opposite raw/weighted winners.')}

SEASONS = {'off500': 1, 'off800': 1, 'scale500': 1, 'adfcrit': 1, 'adflag': 1, 'seasonweak': 12, 'seasonmid': 12, 'difforder': 12, 'weakma1': 1, 'weakma2': 1, 'ma4': 1, 'ma5': 1, 'arstart85': 1, 'arstart92': 1, 'airzero': 12, 'imazero': 1, 'gate35': 12, 'gate36': 12, 'scaleall1': 1, 'scaleall2': 1}

TRIPLETS = ['scaleall1', 'scaleall2']

CASE_KEYS = ['off500', 'off800', 'scale500', 'adfcrit', 'adflag', 'seasonweak', 'seasonmid', 'difforder', 'weakma1', 'weakma2', 'ma4', 'ma5', 'arstart85', 'arstart92', 'airzero', 'imazero', 'gate35', 'gate36', 'scaleall1', 'scaleall2']

def values(recipe):
    kind,seed,n=recipe[:3];p=recipe[3:]
    if kind=='growth':
        offset,scale=p
        return [scale*(offset+math.exp(3+.03*i+.08*v)) for i,v in enumerate(noise(seed,n))]
    if kind=='arma':
        ar,ma=p
        return [100+v for v in arma(seed,n,{int(k):v for k,v in ar.items()},{int(k):v for k,v in ma.items()})]
    if kind=='ima':
        theta,drift=p;e=noise(seed,n+1);out=[];level=100.
        for i in range(n):
            level+=drift+e[i+1]+theta*e[i];out.append(level)
        return out
    if kind=='rwperturb':
        scale,other=p;level=100.;out=[];perturb=noise(other,n)
        for i,v in enumerate(noise(seed,n)):
            level+=v;out.append(level+scale*perturb[i])
        return out
    if kind=='season':
        amp,sd,drift=p
        return [100+drift*i+amp*math.sin(2*math.pi*i/12)+sd*v for i,v in enumerate(noise(seed,n))]
    if kind=='air':
        theta,Theta,drift=p;e=noise(seed,n+13);w=[];out=[]
        for i in range(n):
            v=drift+e[i+13]+theta*e[i+12]+Theta*e[i+1]+theta*Theta*e[i]
            w.append(v+(w[i-1] if i else 0))
            out.append(w[i]+(out[i-12] if i>=12 else 0))
        return [1000+v for v in out]
    raise ValueError(recipe)


def generated_data():
    return dict((key, values(RECIPES[key])) for key in CASE_KEYS)


def declare_cases():
    for key in CASE_KEYS:
        seasonal = SEASONS[key] > 1
        families = [("es", "Exsmooth"), ("ar", "Arima"), ("all", "All")] if key in TRIPLETS else [("ar", "Arima")]
        for suffix, family in families:
            name = "p4_" + key + "_" + suffix
            props = [("method", "ExpertModeler"), ("consider_seasonal", seasonal)]
            if family != "All":
                props.append(("expert_modeler_method", family))
            if seasonal:
                props.extend(DATE_BASED)
            manifest["cases"].append({"case": name, "file": name + ".str", "family": "ts",
                "data": key + "_train.csv", "data_key": key, "targets": ["y"], "inputs": [],
                "properties": props, "keyed_properties": [], "apply_properties": [],
                "tests": SERIES[key][1],
                "fit": {"kind": "expert", "family": family, "seasonal": seasonal},
                "compare_to": None, "score_variants": TS_FORECAST, "status": "planned"})
    if len(manifest["cases"]) > 24:
        raise ValueError("at most 24 cases")
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
