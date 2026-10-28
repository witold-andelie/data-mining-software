# Probe: Auto Data Prep remaining steps (label adp_rest).
# Coordinator only: cargo xtask probe adp_rest_streams, then corpus record adp_rest and oracle.
# Same framework as adp_more_streams.py; only the header comment, DIR/CSV_NAME/label,
# SCHEMA, rows(), ADP_PROPERTIES additions and the build(...) cases differ, plus the
# minimal csv parameter on build() for the second CSV the brief allows (date/time fields).
# Jython 2.7, public modeler.api only (Scripting Guide Table 79, autodataprepnode). Deterministic
# data: each case switches on one ADP step on data built to trigger it. Streams are saved before
# they run, like misc_next/adp_*: the node analyses its input when the Table executes.
import json
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/adp_rest"
CSV_NAME = "adp_rest.csv"
CSV_DATES = "adp_rest_dates.csv"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)
taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "adp_rest", "saved": [], "errors": [], "cases": [], "inputs": []}

# (name, storage, measure, values)
MANY80_VALUES = ["cat%02d" % k for k in range(80)]
SCHEMA = [("id", "Integer", "Range", None),
          ("yc", "Real", "Range", None),
          ("ycm", "Real", "Range", None),
          ("yco", "Real", "Range", None),
          ("ycomb", "Real", "Range", None),
          ("yn", "String", "Flag", ["N", "Y"]),
          ("yo_num", "Integer", "OrderedSet", [1, 2, 3, 4]),
          ("yo_str", "String", "OrderedSet", ["low", "mid", "high", "veryhigh"]),
          ("c_rel", "Real", "Range", None),
          ("c_noise", "Real", "Range", None),
          ("c_miss", "Real", "Range", None),
          ("n1", "String", "Set", ["k1", "k2", "k3", "k4"]),
          ("n_miss", "String", "Set", ["p", "q", "r"]),
          ("o4", "Integer", "OrderedSet", [1, 2, 3, 4]),
          ("om", "Integer", "OrderedSet", [1, 2, 3, 4, 5]),
          ("os", "String", "OrderedSet", ["s1", "s2", "s3", "s4"]),
          ("o_med", "Integer", "OrderedSet", [1, 2]),
          ("wfreq", "Integer", "Range", None),
          ("wweight", "Real", "Range", None),
          ("acat", "String", "Set", ["A", "B"]),
          ("m40", "Real", "Range", None),
          ("many80", "String", "Set", MANY80_VALUES),
          ("const1", "Real", "Range", None)]
SCHEMA_DATES = [("id", "Integer", "Range", None),
                ("y", "Real", "Range", None),
                ("x1", "Real", "Range", None),
                ("x2", "Real", "Range", None),
                ("ncat", "String", "Set", ["k1", "k2", "k3", "k4"]),
                ("d", "Date", "Range", None),
                ("t", "Time", "Range", None),
                ("ts", "Timestamp", "Range", None)]
ADP_PROPERTIES = ["objective", "replace_outliers_inputs", "outlier_cutoff_value", "outlier_method",
                  "rescale_continuous_inputs", "rescaling_method", "min_max_minimum",
                  "min_max_maximum", "z_score_final_mean", "z_score_final_sd",
                  "replace_missing_nominal_inputs", "replace_missing_continuous_inputs",
                  "transform_select_input_fields", "maximize_association_with_target",
                  "p_value_for_merging", "merge_ordinal_features", "merge_nominal_features",
                  "minimum_cases_in_category", "bin_continuous_fields", "p_value_for_binning",
                  "perform_feature_selection", "p_value_for_selection",
                  "perform_feature_construction", "rescale_continuous_target",
                  "target_final_mean", "target_final_sd", "prepare_inputs_and_target",
                  "exclude_low_quality_inputs", "excluded_fields",
                  "replace_outliers_target", "replace_missing_continuous_target",
                  "replace_missing_ordinal_inputs", "reorder_nominal_inputs",
                  "custom_fields", "target", "inputs", "use_frequency", "frequency_field",
                  "use_weight", "weight_field", "exclude_too_many_missing",
                  "maximum_percentage_missing", "exclude_too_many_categories",
                  "maximum_number_categories", "exclude_if_large_category",
                  "maximum_percentage_category", "compute_time_until_date", "reference_date",
                  "fixed_date", "units_for_date_durations", "fixed_date_units",
                  "compute_time_until_time", "reference_time", "fixed_time",
                  "units_for_time_durations", "fixed_time_units", "extract_hour_from_time",
                  "extract_minute_from_time", "extract_second_from_time",
                  "prepare_dates_and_times"]


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


def rows():
    # 300 records. yc depends on c_rel and on n1/o4/os/om (k1/k2, 1/2, s1/s2 share an
    # effect, so supervised merges should join them). ycm is yc with 9 missing, yco yc
    # with two outliers, ycomb both. yn flags yc above its middle. yo_num/yo_str are
    # ordinal versions of yc. c_miss is c_rel with 15 missing, n_miss has 12 blanks,
    # om has 10 missing and a 4-record level 5, os has 12 missing, o_med is 145 ones and
    # 145 twos with 10 missing (median between levels). wfreq/wweight correlate with yc
    # so weights visibly shift statistics. acat is 90 percent A, m40 40 percent missing,
    # many80 80 categories, const1 constant.
    effect_n = {"k1": 0.0, "k2": 0.0, "k3": 3.0, "k4": -3.0}
    effect_o = {1: 0.0, 2: 0.0, 3: 2.5, 4: 5.0, 5: 1.0}
    effect_s = {"s1": 0.0, "s2": 0.0, "s3": 2.5, "s4": 5.0}
    out = []
    med_seq = 0
    for i in range(300):
        c_rel = ((i * 37) % 101) / 10.0 - 5.0
        c_noise = ((i * 71) % 89) / 8.9
        n1 = ["k1", "k2", "k3", "k4"][(i * 3 + i // 7) % 4]
        o4 = 1 + (i * 7 + i // 5) % 4
        om_level = 1 + (i * 3) % 4
        if i in (11, 111, 211, 251):
            om_level = 5
        os_level = ["s1", "s2", "s3", "s4"][(i * 3) % 4]
        wobble = ((i * 29) % 13 - 6) / 6.0
        yc = 10.0 + 0.8 * c_rel + effect_n[n1] + effect_o[o4] + effect_s[os_level] + effect_o[om_level] + wobble
        ycm = None if i % 33 == 4 else yc
        yco = yc + (40.0 if i == 50 else (-35.0 if i == 160 else 0.0))
        ycomb_base = yc + (40.0 if i == 50 else (-35.0 if i == 160 else 0.0))
        ycomb = None if i % 33 == 4 else ycomb_base
        yn = "Y" if yc > 13.0 else "N"
        if yc < 11.0:
            yo_num = 1
            yo_str = "low"
        elif yc < 13.0:
            yo_num = 2
            yo_str = "mid"
        elif yc < 15.0:
            yo_num = 3
            yo_str = "high"
        else:
            yo_num = 4
            yo_str = "veryhigh"
        c_miss = None if i % 20 == 3 else c_rel
        n_miss = None if i % 25 == 3 else ["p", "q", "r", "p"][(i * 5) % 4]
        om = None if i % 30 == 7 else om_level
        os = None if i % 25 == 5 else os_level
        if i % 30 == 0:
            o_med = None
        else:
            med_seq = med_seq + 1
            o_med = 1 if med_seq % 2 == 1 else 2
        wfreq = 3 if yc > 14.0 else 1
        wweight = 2.0 if yc > 14.0 else 0.5
        acat = "B" if i % 10 == 0 else "A"
        m40 = None if i % 5 < 2 else ((i * 37) % 101) / 10.0 - 5.0
        many80 = "cat%02d" % (i % 80)
        const1 = 5.0
        out.append([i + 1, yc, ycm, yco, ycomb, yn, yo_num, yo_str, c_rel, c_noise,
                    c_miss, n1, n_miss, o4, om, os, o_med, wfreq, wweight, acat, m40,
                    many80, const1])
    return out


def two(n):
    return ("0" + str(n))[-2:]


def rows_dates():
    # 200 records: dates through 2019-2021, times of day, x1/x2 one signal (r > .9),
    # ncat balanced, y from x1, timestamps combining date and time.
    out = []
    for i in range(200):
        year = 2019 + i % 3
        month = 1 + (i * 5) % 12
        day = 1 + (i * 7) % 28
        hour = (i * 11) % 24
        minute = (i * 13 + 5) % 60
        second = (i * 17 + 7) % 60
        date = "%d-%s-%s" % (year, two(month), two(day))
        time = "%s:%s:%s" % (two(hour), two(minute), two(second))
        u = ((i * 37) % 101) / 10.0 - 5.0
        e = ((i * 29) % 13 - 6) / 30.0
        x1 = u
        x2 = u + e
        ncat = ["k1", "k2", "k3", "k4"][(i * 3 + i // 7) % 4]
        y = 2.0 * u + ((i * 17) % 11 - 5) / 5.0
        out.append([i + 1, y, x1, x2, ncat, date, time, date + " " + time])
    return out


def write_data_for(csv_name, schema, rows_data):
    header = ",".join([f[0] for f in schema])
    lines = [header]
    for row in rows_data:
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
    handle = open(DIR + "/" + csv_name, "wb")
    try:
        handle.write((NL.join(lines) + NL).encode("utf-8"))
    finally:
        handle.close()
    manifest["inputs"].append({"file": csv_name, "rows": len(lines) - 1})


def write_data():
    write_data_for(CSV_NAME, SCHEMA, rows())
    write_data_for(CSV_DATES, SCHEMA_DATES, rows_dates())


def instantiate(stream, node):
    table = stream.createAt("table", "Instantiate", 900, 700)
    stream.link(node, table)
    try:
        table.run([])
    finally:
        stream.delete(table)


def build(case, roles, props, tests, csv_name=None):
    if csv_name is None:
        csv_name = CSV_NAME
    if csv_name == CSV_DATES:
        schema = SCHEMA_DATES
        is_dates = True
    else:
        schema = SCHEMA
        is_dates = False
    entry = {"case": case, "file": case + ".str", "roles": roles, "properties": props,
             "tests": tests, "status": "planned", "readback": []}
    manifest["cases"].append(entry)
    stream = None
    stage = "create"
    try:
        stream = taskrunner.createStream(case, False, False)
        stream.setPropertyValue("decimal_places", 15)
        if is_dates:
            stream.setPropertyValue("date_format", "YYYY-MM-DD")
            stream.setPropertyValue("time_format", "HH:MM:SS")
        source = stream.createAt("variablefile", csv_name, 80, 100)
        for name, value in [("full_filename", DIR + "/" + csv_name), ("read_field_names", True),
                            ("delimit_comma", True), ("quotes_2", "PairAndDiscard"),
                            ("strip_spaces", "None"), ("decimal_symbol", "Period"),
                            ("encoding", "UTF-8"), ("lines_to_scan", 1000)]:
            source.setPropertyValue(name, value)
        for field, storage, measure, values in schema:
            source.setKeyedPropertyValue("custom_storage_type", field, storage)
            if is_dates and storage in ("Date", "Timestamp"):
                source.setKeyedPropertyValue("custom_date_format", field, "YYYY-MM-DD")
            if is_dates and storage in ("Time", "Timestamp"):
                source.setKeyedPropertyValue("custom_time_format", field, "HH:MM:SS")
        stage = "type"
        type_node = stream.createAt("type", "Type", 280, 100)
        stream.link(source, type_node)
        instantiate(stream, type_node)
        for field, storage, measure, values in schema:
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


MAIN_FIELDS = ["id", "yc", "ycm", "yco", "ycomb", "yn", "yo_num", "yo_str", "c_rel",
               "c_noise", "c_miss", "n1", "n_miss", "o4", "om", "os", "o_med", "wfreq",
               "wweight", "acat", "m40", "many80", "const1"]
DATES_FIELDS = ["id", "y", "x1", "x2", "ncat", "d", "t", "ts"]


def roles_main(target=None, inputs=()):
    result = {}
    for field in MAIN_FIELDS:
        result[field] = "None"
    if target is not None:
        result[target] = "Target"
    for field in inputs:
        result[field] = "Input"
    return result


def roles_dates(target=None, inputs=()):
    result = {}
    for field in DATES_FIELDS:
        result[field] = "None"
    if target is not None:
        result[target] = "Target"
    for field in inputs:
        result[field] = "Input"
    return result


CUSTOM = [("objective", "Custom")]
SELECT = CUSTOM + [("transform_select_input_fields", True)]

try:
    ensure_clean_dir(DIR)
    write_data()
    build("rest_base", roles_main("yc", ("c_rel", "c_noise", "c_miss", "n1", "n_miss", "o4",
          "om", "os", "o_med", "acat", "m40", "many80", "const1")), CUSTOM,
          "Custom: baseline, const1 screened as poor quality")
    build("rest_target_outliers_delete", roles_main("yco", ("c_rel",)),
          CUSTOM + [("replace_outliers_target", True),
          ("outlier_cutoff_value", 3.0), ("outlier_method", "Delete")],
          "Target outliers set to missing (Delete)")
    build("rest_target_combined", roles_main("ycomb", ("c_rel",)),
          CUSTOM + [("replace_outliers_target", True),
          ("outlier_cutoff_value", 3.0), ("outlier_method", "Replace"),
          ("replace_missing_continuous_target", True),
          ("rescale_continuous_target", True),
          ("target_final_mean", 0), ("target_final_sd", 1)],
          "Combined target outliers, missing and Box-Cox rescale")
    build("rest_ordinal_num", roles_main("yo_num", ("c_rel",)), CUSTOM,
          "Numeric ordinal target")
    build("rest_ordinal_str", roles_main("yo_str", ("c_rel",)), CUSTOM,
          "String ordinal target")
    build("rest_strord_missing", roles_main("yc", ("os",)),
          CUSTOM + [("replace_missing_ordinal_inputs", True)],
          "String ordinal missing values replaced")
    build("rest_strord_merge", roles_main("yc", ("os",)),
          SELECT + [("maximize_association_with_target", True),
          ("merge_ordinal_features", True), ("p_value_for_merging", 0.05)],
          "Supervised merge of string ordinal levels")
    build("rest_nominal_missing_merge", roles_main("yc", ("n_miss",)),
          SELECT + [("maximize_association_with_target", True),
          ("merge_nominal_features", True), ("p_value_for_merging", 0.05)],
          "Nominal with missing values under supervised merge")
    build("rest_cont_missing_binning", roles_main("yn", ("c_miss",)),
          CUSTOM + [("transform_select_input_fields", True),
          ("bin_continuous_fields", True), ("p_value_for_binning", 0.05)],
          "Continuous with missing values under supervised binning")
    build("rest_ordinal_merge_missing", roles_main("yc", ("om",)),
          SELECT + [("maximize_association_with_target", True),
          ("merge_ordinal_features", True), ("p_value_for_merging", 0.05),
          ("replace_missing_ordinal_inputs", True)],
          "Ordinal merge with missing values replaced")
    build("rest_nominal_noreorder", roles_main("yc", ("n1",)),
          SELECT + [("maximize_association_with_target", True),
          ("merge_nominal_features", True), ("p_value_for_merging", 0.05),
          ("reorder_nominal_inputs", False)],
          "Merge without reordering nominal categories")
    build("rest_selection_cattarget", roles_main("yn", ("c_rel", "c_noise")),
          CUSTOM + [("transform_select_input_fields", True),
          ("perform_feature_selection", True), ("p_value_for_selection", 0.05)],
          "Feature selection against a categorical target")
    # Weights are custom field settings (Scripting Guide: with custom_fields false the Type
    # node's settings are used), so these set custom_fields with the Type node's own roles.
    SAME_FIELDS = [("custom_fields", True), ("target", "yc"), ("inputs", ["c_rel", "n1"])]
    build("rest_weight_control", roles_main("yc", ("c_rel", "n1")), CUSTOM + SAME_FIELDS,
          "Control for the weights: custom_fields repeating the Type roles")
    build("rest_freq_weight", roles_main("yc", ("c_rel", "n1")),
          CUSTOM + SAME_FIELDS + [("use_frequency", True), ("frequency_field", "wfreq")],
          "Frequency weight wfreq (3 on high yc)")
    build("rest_analysis_weight", roles_main("yc", ("c_rel", "n1")),
          CUSTOM + SAME_FIELDS + [("use_weight", True), ("weight_field", "wweight")],
          "Analysis weight wweight (2.0 on high yc)")
    # Modeler rejects all three analysis-weight cases at run time ("Metadata (on input/output
    # fields) not valid", 2026-10-28), with or without custom_fields and for a real or an integer
    # weight, so they are not recorded in the corpus; the learner keeps rejecting use_weight.
    build("rest_analysis_weight_nocustom", roles_main("yc", ("c_rel", "n1")),
          CUSTOM + [("use_weight", True), ("weight_field", "wweight")],
          "Analysis weight wweight without custom_fields")
    build("rest_analysis_weight_int", roles_main("yc", ("c_rel", "n1")),
          CUSTOM + SAME_FIELDS + [("use_weight", True), ("weight_field", "wfreq")],
          "Analysis weight on the integer field wfreq (1 or 3)")
    build("rest_custom_fields", roles_main("yc", ("c_rel", "c_noise", "n1")),
          CUSTOM + [("custom_fields", True), ("target", "yc"), ("inputs", ["c_rel"])],
          "custom_fields with explicit target and subset inputs")
    build("rest_almost_constant", roles_main("yc", ("acat", "c_rel")),
          CUSTOM + [("exclude_if_large_category", True),
          ("maximum_percentage_category", 80)],
          "Almost-constant screening (90 percent A, threshold 80)")
    build("rest_too_many_missing", roles_main("yc", ("m40", "c_rel")),
          CUSTOM + [("exclude_too_many_missing", True),
          ("maximum_percentage_missing", 30)],
          "Missing-share screening (40 percent missing, threshold 30)")
    build("rest_too_many_categories", roles_main("yc", ("many80", "c_rel")),
          CUSTOM + [("exclude_too_many_categories", True),
          ("maximum_number_categories", 50)],
          "Many-categories screening (80 cats, threshold 50)")
    build("rest_ordinal_median_between", roles_main("yc", ("o_med",)),
          CUSTOM + [("replace_missing_ordinal_inputs", True)],
          "Ordinal median falling between two levels")
    build("rest_dates_base", roles_dates("y", ("x1", "x2", "ncat", "d", "t", "ts")),
          CUSTOM + [("prepare_dates_and_times", True),
          ("compute_time_until_date", True), ("reference_date", "Fixed"),
          ("fixed_date", "2022-01-01"), ("units_for_date_durations", "Fixed"),
          ("fixed_date_units", "Days"), ("compute_time_until_time", True),
          ("reference_time", "Fixed"), ("fixed_time", "12:00:00"),
          ("units_for_time_durations", "Fixed"), ("fixed_time_units", "Hours")],
          "Dates baseline: day duration and hour duration", CSV_DATES)
    build("rest_dates_selection", roles_dates("y", ("x1", "x2", "ncat", "d", "t", "ts")),
          CUSTOM + [("prepare_dates_and_times", True),
          ("compute_time_until_date", True), ("reference_date", "Fixed"),
          ("fixed_date", "2022-01-01"), ("units_for_date_durations", "Fixed"),
          ("fixed_date_units", "Days"), ("compute_time_until_time", True),
          ("reference_time", "Fixed"), ("fixed_time", "12:00:00"),
          ("units_for_time_durations", "Fixed"), ("fixed_time_units", "Hours"),
          ("transform_select_input_fields", True),
          ("perform_feature_selection", True), ("p_value_for_selection", 0.05)],
          "Feature selection with date-derived fields", CSV_DATES)
    build("rest_dates_minmax", roles_dates("y", ("x1", "x2", "ncat", "d", "t", "ts")),
          CUSTOM + [("prepare_dates_and_times", True),
          ("compute_time_until_date", True), ("reference_date", "Fixed"),
          ("fixed_date", "2022-01-01"), ("units_for_date_durations", "Fixed"),
          ("fixed_date_units", "Days"), ("compute_time_until_time", True),
          ("reference_time", "Fixed"), ("fixed_time", "12:00:00"),
          ("units_for_time_durations", "Fixed"), ("fixed_time_units", "Hours"),
          ("rescale_continuous_inputs", True), ("rescaling_method", "MinMax"),
          ("min_max_minimum", 0), ("min_max_maximum", 100)],
          "Min-max rescaling of date-derived fields", CSV_DATES)
    build("rest_time_both", roles_dates("y", ("x1", "x2", "ncat", "d", "t", "ts")),
          CUSTOM + [("prepare_dates_and_times", True),
          ("compute_time_until_date", True), ("reference_date", "Fixed"),
          ("fixed_date", "2022-01-01"), ("units_for_date_durations", "Fixed"),
          ("fixed_date_units", "Days"), ("compute_time_until_time", True),
          ("reference_time", "Fixed"), ("fixed_time", "12:00:00"),
          ("units_for_time_durations", "Fixed"), ("fixed_time_units", "Hours"),
          ("extract_hour_from_time", True), ("extract_minute_from_time", True),
          ("extract_second_from_time", True)],
          "Time duration together with time elements", CSV_DATES)
    build("rest_construct_order", roles_dates("y", ("x1", "x2", "ncat", "d", "t", "ts")),
          CUSTOM + [("prepare_dates_and_times", True),
          ("compute_time_until_date", True), ("reference_date", "Fixed"),
          ("fixed_date", "2022-01-01"), ("units_for_date_durations", "Fixed"),
          ("fixed_date_units", "Days"), ("compute_time_until_time", True),
          ("reference_time", "Fixed"), ("fixed_time", "12:00:00"),
          ("units_for_time_durations", "Fixed"), ("fixed_time_units", "Hours"),
          ("transform_select_input_fields", True),
          ("perform_feature_construction", True)],
          "Construction with categorical and date fields (order)", CSV_DATES)
except ERRORS as exc:
    manifest["errors"].append({"case": None, "stage": "setup", "message": message(exc)})
finally:
    write_manifest()
