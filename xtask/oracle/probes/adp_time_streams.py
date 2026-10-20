# Probe: Auto Data Prep date, time and timestamp handling (label adp_time).
# Coordinator only: cargo xtask probe adp_time_streams, then corpus record adp_time and oracle.
# Same framework as adp_next_streams.py, plus date/time formats on the source and the stream.
# Jython 2.7, public modeler.api only (Scripting Guide Table 79, autodataprepnode). Deterministic
# data: each case switches on one ADP step on data built to trigger it. Streams are saved before
# they run, like misc_next/adp_*: the node analyses its input when the Table executes.
import json
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/adp_time"
CSV_NAME = "adp_time.csv"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)
taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "adp_time", "saved": [], "errors": [], "cases": [], "inputs": []}

# (name, storage, measure, values)
SCHEMA = [("id", "Integer", "Range", None),
          ("y", "Real", "Range", None),
          ("d", "Date", "Range", None),
          ("t", "Time", "Range", None),
          ("ts", "Timestamp", "Range", None)]
ADP_PROPERTIES = ["compute_time_until_time", "reference_time", "fixed_time",
                  "units_for_time_durations", "fixed_time_units", "extract_hour_from_time",
                  "extract_minute_from_time", "extract_second_from_time", "compute_time_until_date",
                  "reference_date", "fixed_date", "units_for_date_durations", "fixed_date_units",
                  "prepare_dates_and_times",
                  "objective", "replace_outliers_inputs", "outlier_cutoff_value", "outlier_method",
                  "rescale_continuous_inputs", "rescaling_method", "min_max_minimum",
                  "min_max_maximum", "z_score_final_mean", "z_score_final_sd",
                  "replace_missing_nominal_inputs", "replace_missing_continuous_inputs",
                  "transform_select_input_fields", "maximize_association_with_target",
                  "p_value_for_merging", "merge_ordinal_features", "merge_nominal_features",
                  "minimum_cases_in_category", "bin_continuous_fields", "p_value_for_binning",
                  "perform_feature_selection", "p_value_for_selection",
                  "perform_feature_construction", "rescale_continuous_target",
                  "target_final_mean", "target_final_sd", "prepare_inputs_and_target",
                  "exclude_low_quality_inputs", "excluded_fields"]


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
    directory = java.io.File(path)
    if directory.exists():
        for child in directory.listFiles():
            if child.isFile() and unicode(child.getName()).endswith((".csv", ".str")):
                if not child.delete():
                    raise IOError("cannot remove " + unicode(child))
    elif not directory.mkdirs():
        raise IOError("cannot create " + path)


def two(n):
    return ("0" + str(n))[-2:]


def rows():
    # 120 records: dates through 2019-2021 (day, month and year all vary), times of day from
    # 00:05:07 to 23:59, timestamps combining both; y depends on the hour.
    out = []
    for i in range(120):
        year = 2019 + i % 3
        month = 1 + (i * 5) % 12
        day = 1 + (i * 7) % 28
        hour = (i * 11) % 24
        minute = (i * 13 + 5) % 60
        second = (i * 17 + 7) % 60
        date = "%d-%s-%s" % (year, two(month), two(day))
        time = "%s:%s:%s" % (two(hour), two(minute), two(second))
        y = 3.0 + hour / 4.0 + ((i * 29) % 13 - 6) / 6.0
        out.append([i + 1, y, date, time, date + " " + time])
    return out


def write_data():
    lines = [",".join([f[0] for f in SCHEMA])]
    for row in rows():
        cells = []
        for value in row:
            if value is None:
                cells.append("")
            elif isinstance(value, basestring):
                cells.append('"' + value + '"')
            elif isinstance(value, float):
                cells.append("%.17g" % value)
            else:
                cells.append(str(value))
        lines.append(",".join(cells))
    handle = open(DIR + "/" + CSV_NAME, "wb")
    try:
        handle.write((NL.join(lines) + NL).encode("utf-8"))
    finally:
        handle.close()
    manifest["inputs"].append({"file": CSV_NAME, "rows": len(lines) - 1})


def instantiate(stream, node):
    table = stream.createAt("table", "Instantiate", 900, 700)
    stream.link(node, table)
    try:
        table.run([])
    finally:
        stream.delete(table)


def build(case, roles, props, tests):
    entry = {"case": case, "file": case + ".str", "roles": roles, "properties": props,
             "tests": tests, "status": "planned", "readback": []}
    manifest["cases"].append(entry)
    stream = None
    stage = "create"
    try:
        stream = taskrunner.createStream(case, False, False)
        stream.setPropertyValue("decimal_places", 15)
        stream.setPropertyValue("date_format", "YYYY-MM-DD")
        stream.setPropertyValue("time_format", "HH:MM:SS")
        source = stream.createAt("variablefile", CSV_NAME, 80, 100)
        for name, value in [("full_filename", DIR + "/" + CSV_NAME), ("read_field_names", True),
                            ("delimit_comma", True), ("quotes_2", "PairAndDiscard"),
                            ("strip_spaces", "None"), ("decimal_symbol", "Period"),
                            ("encoding", "UTF-8"), ("lines_to_scan", 1000)]:
            source.setPropertyValue(name, value)
        for field, storage, measure, values in SCHEMA:
            source.setKeyedPropertyValue("custom_storage_type", field, storage)
            if storage in ("Date", "Timestamp"):
                source.setKeyedPropertyValue("custom_date_format", field, "YYYY-MM-DD")
            if storage in ("Time", "Timestamp"):
                source.setKeyedPropertyValue("custom_time_format", field, "HH:MM:SS")
        stage = "type"
        type_node = stream.createAt("type", "Type", 280, 100)
        stream.link(source, type_node)
        instantiate(stream, type_node)
        for field, storage, measure, values in SCHEMA:
            type_node.setKeyedPropertyValue("type", field, measure)
            if values is not None:
                type_node.setKeyedPropertyValue("values", field, values)
                type_node.setKeyedPropertyValue("extend_values", field, False)
            type_node.setKeyedPropertyValue("direction", field, roles.get(field, "Input"))
            type_node.setKeyedPropertyValue("check", field, "None")
        instantiate(stream, type_node)
        stage = "configure"
        node = stream.createAt("autodataprep", case, 500, 100)
        stream.link(type_node, node)
        entry["defaults"] = read(node)
        for name, value in props:
            entry["setting_in_progress"] = [name, snapshot(value)]
            node.setPropertyValue(name, value)
            entry["readback"].append([name, snapshot(node.getPropertyValue(name))])
        entry.pop("setting_in_progress", None)
        entry["effective"] = read(node)
        table = stream.createAt("table", "Table", 760, 100)
        stream.link(node, table)
        stage = "save"
        taskrunner.saveStreamToFile(stream, DIR + "/" + entry["file"])
        manifest["saved"].append(entry["file"])
        entry["status"] = "saved"
        stage = "run"
        results = []
        table.run(results)
        content = results[0].getContentModel("table")
        entry["columns"] = [[unicode(content.getColumnName(c)), unicode(content.getStorageType(c))]
                            for c in range(content.getColumnCount())]
        entry["row_count"] = content.getRowCount()
        entry["status"] = "ran"
    except ERRORS as exc:
        manifest["errors"].append({"case": case, "stage": stage, "message": message(exc)})
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS as exc:
                manifest["errors"].append({"case": case, "stage": "clear", "message": message(exc)})
        write_manifest()


def read(node):
    values = {}
    for name in ADP_PROPERTIES:
        try:
            values[name] = snapshot(node.getPropertyValue(name))
        except ERRORS as exc:
            values[name] = {"read_error": message(exc)}
    return values




ROLES = {"id": "None", "y": "Target"}
CUSTOM = [("objective", "Custom"), ("prepare_dates_and_times", True)]
NO_DATE = [("compute_time_until_date", False)]
NO_TIME = [("compute_time_until_time", False)]
FIXED_TIME = [("compute_time_until_time", True), ("reference_time", "Fixed"),
              ("fixed_time", "12:00:00")]
FIXED_DATE = [("compute_time_until_date", True), ("reference_date", "Fixed"),
              ("fixed_date", "2022-01-01")]

try:
    ensure_clean_dir(DIR)
    write_data()
    build("time_hours", ROLES, CUSTOM + NO_DATE + FIXED_TIME + [
          ("units_for_time_durations", "Fixed"), ("fixed_time_units", "Hours")],
          "Time duration in hours from 12:00:00")
    build("time_minutes", ROLES, CUSTOM + NO_DATE + FIXED_TIME + [
          ("units_for_time_durations", "Fixed"), ("fixed_time_units", "Minutes")],
          "Time duration in minutes")
    build("time_seconds", ROLES, CUSTOM + NO_DATE + FIXED_TIME + [
          ("units_for_time_durations", "Fixed"), ("fixed_time_units", "Seconds")],
          "Time duration in seconds")
    build("time_auto", ROLES, CUSTOM + NO_DATE + FIXED_TIME + [
          ("units_for_time_durations", "Automatic")], "Time duration, automatic unit")
    build("time_extract", ROLES, CUSTOM + NO_DATE + NO_TIME + [
          ("extract_hour_from_time", True), ("extract_minute_from_time", True),
          ("extract_second_from_time", True)], "Hour, minute and second elements")
    build("date_auto", ROLES, CUSTOM + NO_TIME + FIXED_DATE + [
          ("units_for_date_durations", "Automatic")], "Date duration, automatic unit")
    build("date_years", ROLES, CUSTOM + NO_TIME + FIXED_DATE + [
          ("units_for_date_durations", "Fixed"), ("fixed_date_units", "Years")],
          "Date duration in years")
    build("date_auto_after", ROLES, CUSTOM + NO_TIME + [("compute_time_until_date", True),
          ("reference_date", "Fixed"), ("fixed_date", "2018-01-01"),
          ("units_for_date_durations", "Automatic")],
          "Reference before every date: elapsed days 365..1460 (min-rule months, max-rule years)")
    build("time_auto_midnight", ROLES, CUSTOM + NO_DATE + [("compute_time_until_time", True),
          ("reference_time", "Fixed"), ("fixed_time", "00:00:00"),
          ("units_for_time_durations", "Automatic")],
          "Reference 00:00:00: elapsed 307..86087 s (min-rule minutes, max-rule hours)")
    build("time_hours_raw", ROLES, CUSTOM + NO_DATE + FIXED_TIME + [
          ("units_for_time_durations", "Fixed"), ("fixed_time_units", "Hours"),
          ("rescale_continuous_inputs", False)], "Unscaled hours: sign and unit visible")
    build("date_years_raw", ROLES, CUSTOM + NO_TIME + FIXED_DATE + [
          ("units_for_date_durations", "Fixed"), ("fixed_date_units", "Years"),
          ("rescale_continuous_inputs", False)], "Unscaled years")
    build("datetime_off", ROLES, CUSTOM + NO_TIME + NO_DATE, "No durations, no extraction")
except ERRORS as exc:
    manifest["errors"].append({"case": None, "stage": "setup", "message": message(exc)})
finally:
    write_manifest()
