# Build the misc corpus with Modeler 19's public scripting API (Jython 2.7).
# Coordinator only: cargo xtask probe misc_streams. Do not run during preparation.
# Catalog ids differ from API names: set_globals/setglobals, rfm_aggregate/
# rfmaggregate, rfm_analysis/rfmanalysis, time_intervals/timeintervals.
# See probe-plans/misc_streams.md for sources, case intent, and capture limitations.
import datetime
import json
import java.io.File
import java.lang.Exception
import java.text.SimpleDateFormat
import java.util.Date
import modeler.api

OUT_DIR = @OUT_DIR@
MISC_DIR = "D:/data-mining-software/corpus/misc"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)
taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "misc", "saved": [], "errors": [], "cases": [], "inputs": [],
            "generated_on": java.text.SimpleDateFormat("yyyy-MM-dd").format(java.util.Date())}

# Python None is an unquoted empty CSV cell. "" is a quoted empty string.
# Source import can conflate these: base_input below also constructs explicit
# string null/empty/whitespace values with CLEM, independently of CSV parsing.
MAIN_SCHEMA = [("id", "Integer", "Range"), ("grp", "String", "Set"),
               ("iv", "Integer", "Range"), ("rv", "Real", "Range"),
               ("pos", "Real", "Range"), ("constant", "Real", "Range"),
               ("flg", "String", "Flag"), ("s", "String", "Set"),
               ("dt", "Date", "Range"), ("ts", "Timestamp", "Range")]
MAIN_ROWS = [
    [1, "B", -4, -4.0, 0.125, 7.0, "T", "banana", "2020-02-27", "2020-02-27 23:59:58"],
    [2, "A", -2, -2.5, 0.25, 7.0, "F", "Apple", "2020-02-28", "2020-02-28 00:00:01"],
    [3, "B", -1, -1.0, 0.5, 7.0, "T", "apple", "2020-02-29", "2020-02-29 12:00:00"],
    [4, "C", 0, 0.0, 1.0, 7.0, "F", "", "2020-03-01", "2020-03-01 00:00:00"],
    [5, "A", 0, 0.0, 1.0, 7.0, "T", None, "2020-03-02", "2020-03-02 09:00:00"],
    [6, "C", 0, 0.0, 1.0, 7.0, "F", "  ", "2020-03-03", "2020-03-03 09:00:01"],
    [7, "B", 1, 0.125, 2.0, 7.0, "T", "has,comma", "2020-03-04", "2020-03-04 09:00:00"],
    [8, "A", 1, 0.25, 3.0, 7.0, "F", 'say "hi"', "2020-03-05", "2020-03-05 09:00:00"],
    [9, "B", 2, 0.5, 4.0, 7.0, "T", "<tag>&", "2020-03-06", "2020-03-06 09:00:00"],
    [10, "C", 2, 1.0, 5.0, 7.0, "F", "banana", "2020-03-07", "2020-03-07 09:00:00"],
    [11, "A", 2, 1.0, 6.0, 7.0, "T", "Apple", "2020-03-08", "2020-03-08 09:00:00"],
    [12, "B", 3, 1.25, 7.0, 7.0, "F", "apple", "2020-03-09", "2020-03-09 09:00:00"],
    [13, "A", 4, 2.0, 8.0, 7.0, "T", "beta", "2020-03-10", "2020-03-10 09:00:00"],
    [14, "C", 4, 2.5, 9.0, 7.0, "F", "beta", "2020-03-11", "2020-03-11 09:00:00"],
    [15, "B", 5, 3.0, 10.0, 7.0, "T", "zebra", "2020-03-12", "2020-03-12 09:00:00"],
    [16, "A", 8, 4.0, 11.0, 7.0, "F", "zebra", "2020-03-13", "2020-03-13 09:00:00"],
    [17, "C", 8, 4.0, 12.0, 7.0, "T", "banana", "2020-03-14", "2020-03-14 09:00:00"],
    [18, "B", 9, 5.0, 16.0, 7.0, "F", "omega", "2020-03-15", "2020-03-15 09:00:00"],
    [19, "A", 12, 6.0, 20.0, 7.0, "T", "omega", "2020-03-16", "2020-03-16 09:00:00"],
    [20, "C", 20, 8.0, 25.0, 7.0, "F", "end", "2020-03-17", "2020-03-17 09:00:00"],
    [21, None, None, 0.123456789012345, 2.0, 7.0, None, "missing", None, None],
    [22, "", 4, None, 2.0, 7.0, "T", "last", "2020-03-18", "2020-03-18 09:00:00"],
]

TX_SCHEMA = [("id", "Integer", "Range"), ("customer", "String", "Set"),
             ("purchased", "Date", "Range"), ("amount", "Real", "Range"),
             ("stamp", "Timestamp", "Range")]
# Interleaved customers, duplicate transactions/times, threshold +/- epsilon,
# leap-month/year cutoffs, a future purchase, one/two/three/four purchases per ID.
TX_ROWS = [
    [1, "B", "2020-03-31", 10.0, "2020-03-31 00:00:00"],
    [2, "A", "2020-02-28", 9.99, "2020-02-28 23:59:59"],
    [3, "C", "2020-03-24", 10.01, "2020-03-24 00:00:00"],
    [4, "A", "2020-02-29", 10.0, "2020-02-29 00:00:00"],
    [5, "B", "2020-03-31", 10.0, "2020-03-31 00:00:00"],
    [6, "D", "2020-03-23", 0.0, "2020-03-23 23:59:59"],
    [7, "E", "2020-03-17", -5.0, "2020-03-17 00:00:00"],
    [8, "A", "2020-03-01", 20.0, "2020-03-01 00:00:00"],
    [9, "E", "2020-03-16", 50.0, "2020-03-16 23:59:59"],
    [10, "F", "2019-03-30", 100.0, "2019-03-30 23:59:59"],
    [11, "G", "2019-03-31", 200.0, "2019-03-31 00:00:00"],
    [12, "H", "2019-04-01", 300.0, "2019-04-01 00:00:00"],
    [13, "B", "2020-03-30", 5.0, "2020-03-30 12:00:00"],
    [14, "A", "2020-03-31", 30.0, "2020-03-31 23:59:59"],
    [15, "I", "2020-04-01", 40.0, "2020-04-01 00:00:01"],
    [16, "J", "2020-03-24", 9.99, "2020-03-24 00:00:00"],
    [17, "K", "2020-03-24", 10.0, "2020-03-24 00:00:00"],
    [18, "L", "2020-03-24", 10.01, "2020-03-24 00:00:00"],
    [19, "M", "2020-03-31", -10.0, "2020-03-31 00:00:00"],
    [20, "N", "2020-03-31", 0.0, "2020-03-31 00:00:00"],
]
TX_MISSING_ROWS = TX_ROWS + [
    [21, "A", None, 99.0, None], [22, "B", "2020-03-29", None, "2020-03-29 00:00:00"],
    [23, None, "2020-03-28", 7.0, "2020-03-28 00:00:00"],
    [24, "", "2020-03-27", 8.0, "2020-03-27 00:00:00"],
    [25, "Z", None, None, None],
]

RFM_SCHEMA = [("id", "Integer", "Range"), ("r", "Real", "Range"),
              ("f", "Integer", "Range"), ("m", "Real", "Range"),
              ("last_date", "Date", "Range"), ("last_stamp", "Timestamp", "Range")]
# Independent permutations, boundary ties, negatives/zero, partial/all nulls.
RFM_ROWS = []
R_VALUES = [0, 1, 1, 1, 2, 2, 4, 4, 4, 6, 8, 10, 10, 10, 12, 16, 20, 24, 30, 40]
F_VALUES = [10, 1, 1, 4, 4, 4, 2, 8, 3, 10, 2, 3, 9, 1, 8, 5, 5, 7, 6, 20]
M_VALUES = [0, 100, 50, 50, 200, 20, 50, 400, 200, 10, 100, 100, 300, 500, 20, 30, 600, 200, 800, 1000]
for i in range(20):
    # Exact numeric/date counterparts relative to 2020-03-31. Timestamp splits
    # date ties by seconds, independently of the frequency/monetary ordering.
    date = (datetime.date(2020, 3, 31) - datetime.timedelta(days=R_VALUES[i])).isoformat()
    RFM_ROWS.append([i + 1, R_VALUES[i], F_VALUES[i], M_VALUES[i], date,
                     "%s 12:00:%02d" % (date, i)])
RFM_ROWS += [[21, -1, 0, -10.0, "2020-04-01", "2020-04-01 00:00:00"],
             [22, None, 3, 50.0, None, None],
             [23, 4, None, 50.0, "2020-03-27", "2020-03-27 12:00:00"],
             [24, 4, 3, None, "2020-03-27", "2020-03-27 12:00:00"],
             [25, None, None, None, None, None]]

# Copies allow all eight keyed aggregation functions to be observed in one case.
TIME_SCHEMA = [("id", "Integer", "Range"), ("tick", "Timestamp", "Range")]
TIME_SCHEMA += [(name, "Real", "Range") for name in
                ["x_mean", "x_sum", "x_mode", "x_min", "x_max", "x_first", "x_last"]]
TIME_SCHEMA += [("cat", "String", "Set"), ("flg", "String", "Flag")]
TIME_ROWS = []
TIME_POINTS = [("2020-02-28 23:59:58", 2.0, "B", "F"),
               ("2020-02-28 23:59:58", 8.0, "A", "T"),
               ("2020-02-28 23:59:59", 8.0, "A", "F"),
               ("2020-02-29 00:00:02", 20.0, "C", "F"),
               ("2020-02-29 00:00:05", None, "", "T"),
               ("2020-02-29 00:02:00", 10.0, "B", "T"),
               ("2020-02-29 00:02:00", 30.0, "A", "F"),
               ("2020-02-29 02:00:00", 40.0, "C", "F"),
               ("2020-03-02 00:00:00", 50.0, "A", "T"),
               ("2020-03-02 00:00:00", 70.0, "B", "T"),
               ("2020-03-04 00:00:00", 90.0, "A", "F")]
for i, point in enumerate(TIME_POINTS):
    tick, value, cat, flg = point
    TIME_ROWS.append([i + 1, tick] + [value] * 7 + [cat, flg])
# Dense, bounded sub-day fixtures: no accidental million-row padding.
TIME_HOURS = TIME_ROWS[:8]
TIME_MINUTES = TIME_ROWS[:7]
TIME_SECONDS = TIME_ROWS[:5]


def message(exc):
    get = getattr(exc, "getMessage", None)
    if get is not None:
        return unicode(get())
    return unicode(exc)


def snapshot(value):
    if value is None or isinstance(value, (bool, int, long, float, basestring)):
        return value
    if isinstance(value, (list, tuple)):
        return [snapshot(item) for item in value]
    return unicode(value)


def ensure_clean_dir(path):
    # Unlike the older probes, never delete arbitrary files from a shared label.
    # Each owned file is replaced by name; stale cases are excluded by manifest.
    if path != MISC_DIR:
        raise ValueError("refusing an unexpected corpus directory")
    directory = java.io.File(path)
    if not directory.exists() and not directory.mkdirs():
        raise IOError("cannot create " + path)
    if not directory.isDirectory():
        raise IOError("not a directory: " + path)


def csv_cell(value):
    if value is None:
        return ""
    if isinstance(value, basestring):
        return '"' + value.replace('"', '""') + '"'
    return repr(value)


def write_csv(name, schema, rows):
    lines = [",".join([field[0] for field in schema])]
    for row in rows:
        if len(row) != len(schema):
            raise ValueError("wrong row width in " + name)
        lines.append(",".join([csv_cell(value) for value in row]))
    handle = open(MISC_DIR + "/" + name, "w")
    try:
        handle.write(NL.join(lines) + NL)
    finally:
        handle.close()
    manifest["saved"].append(name)
    manifest["inputs"].append({"file": name, "records": len(rows), "schema": schema})


def set_props(node, props, record):
    for item in props:
        entry = {"node": unicode(node.getLabel()), "property": item[0],
                 "value": snapshot(item[-1])}
        if len(item) == 3:
            entry["key"] = item[1]
        record.append(entry)
        # Record before setting, so a rejected property remains diagnosable.
        if len(item) == 3:
            node.setKeyedPropertyValue(item[0], item[1], item[2])
        else:
            node.setPropertyValue(item[0], item[1])


def get_props(node, props):
    values = []
    for item in props:
        entry = {"property": item[0]}
        try:
            if len(item) == 3:
                entry["key"] = item[1]
                value = node.getKeyedPropertyValue(item[0], item[1])
            else:
                value = node.getPropertyValue(item[0])
            entry["value"] = snapshot(value)
        except ERRORS, exc:
            entry["read_error"] = message(exc)
        values.append(entry)
    return values


def instantiate_node(stream, node):
    table = stream.createAt("table", "TempTable", 1000, 500)
    stream.link(node, table)
    try:
        table.run([])
    finally:
        stream.delete(table)


def source(stream, name, schema, record, y=100):
    src = stream.createAt("variablefile", name, 80, y)
    props = [("full_filename", MISC_DIR + "/" + name), ("read_field_names", True),
             ("delimit_comma", True), ("delimit_space", False), ("delimit_tab", False),
             ("decimal_symbol", "Period"), ("strip_spaces", "None"),
             ("quotes_2", "PairAndDiscard"), ("encoding", "UTF-8")]
    for field, storage, measure in schema:
        props.append(("custom_storage_type", field, storage))
        if storage in ["Date", "Timestamp"]:
            props.append(("custom_date_format", field, "YYYY-MM-DD"))
        if storage in ["Time", "Timestamp"]:
            props.append(("custom_time_format", field, "HH:MM:SS"))
    set_props(src, props, record)
    typ = stream.createAt("type", "Type_" + name, 220, y)
    stream.link(src, typ)
    instantiate_node(stream, typ)
    for field, storage, measure in schema:
        set_props(typ, [("type", field, measure), ("direction", field, "Input")], record)
        if measure == "Flag":
            set_props(typ, [("values", field, ["F", "T"])], record)
    instantiate_node(stream, typ)
    return typ


def derive(stream, upstream, name, expression, measure, record):
    node = stream.createAt("derive", name, 320, 220)
    stream.link(upstream, node)
    set_props(node, [("new_name", name), ("result_type", "Formula"),
                     ("formula_expr", expression), ("formula_measure_type", measure)], record)
    return node


def base_input(stream, csv_name, schema, entry, options):
    upstream = source(stream, csv_name, schema, entry["setup_properties"])
    if schema == MAIN_SCHEMA:
        upstream = derive(stream, upstream, "text_edge",
                          'if id = 4 then "" else if id = 5 then undef '
                          'else if id = 6 then "  " else s endif endif endif',
                          "Set", entry["setup_properties"])
        if options.get("all_null_field"):
            upstream = derive(stream, upstream, "all_null", "if id < 0 then rv else undef endif",
                              "Range", entry["setup_properties"])
        instantiate_node(stream, upstream)
    if options.get("user_missing"):
        typ = stream.createAt("type", "UserMissing", 360, 160)
        stream.link(upstream, typ)
        set_props(typ, [("enable_missing", "rv", True), ("missing_values", "rv", [0.0]),
                        ("enable_missing", "grp", True), ("missing_values", "grp", ["B"])],
                  entry["setup_properties"])
        upstream = typ
    if options.get("integer_as_set"):
        typ = stream.createAt("type", "IntegerNominal", 360, 160)
        stream.link(upstream, typ)
        set_props(typ, [("type", "iv", "Set")], entry["setup_properties"])
        upstream = typ
    if options.get("empty") or options.get("singleton") or options.get("first_two"):
        select = stream.createAt("select", "Subset", 380, 180)
        stream.link(upstream, select)
        condition = "id = 1"
        if options.get("first_two"):
            condition = "id <= 2"
        if options.get("empty"):
            condition = "id < 0"
        set_props(select, [("mode", "Include"), ("condition", condition)], entry["setup_properties"])
        upstream = select
    return upstream


def terminal_order(stream):
    labels = []
    iterator = stream.iterator()
    while iterator.hasNext():
        node = iterator.next()
        if node.isTerminal():
            labels.append(unicode(node.getLabel()))
    return labels


def build_case(name, api_type, catalog_id, csv_name, schema, props, purpose,
               options=None):
    if options is None:
        options = {}
    stream = None
    entry = {"case": name, "file": "misc_" + name + ".str", "data": csv_name,
             "node_type": api_type, "catalog_id": catalog_id, "purpose": purpose,
             "properties": [], "setup_properties": [], "options": options}
    manifest["cases"].append(entry)
    stage = "create"
    try:
        stream = taskrunner.createStream(name, False, False)
        stream.setPropertyValue("decimal_places", 15)
        entry["decimal_places"] = 15
        stage = "source"
        upstream = base_input(stream, csv_name, schema, entry, options)
        node = stream.createAt(api_type, name, 480, 100)
        stream.link(upstream, node)
        entry["defaults"] = get_props(node, DEFAULTS[api_type])
        stage = "configure"
        set_props(node, props, entry["properties"])
        terminal = node
        if api_type not in ["transform", "report"]:
            terminal = stream.createAt("table", "Table", 740, 100)
            stream.link(node, terminal)
        entry["terminal"] = unicode(terminal.getTypeName())
        entry["terminal_order"] = terminal_order(stream)
        # Preserve configured streams even when the oracle rejects a domain/type,
        # all-null input, empty series, or a padding limit at execution time.
        stage = "save_configured"
        path = MISC_DIR + "/" + entry["file"]
        taskrunner.saveStreamToFile(stream, path)
        manifest["saved"].append(entry["file"])
        stage = "validate"
        terminal.run([])
        entry["validation"] = "ok"
        entry["effective_properties"] = get_props(node, DEFAULTS[api_type])
        # Persist computed bin thresholds and instantiated output schemas.
        stage = "save_validated"
        taskrunner.saveStreamToFile(stream, path)
    except ERRORS, exc:
        entry["validation"] = "error"
        entry["error_stage"] = stage
        entry["error"] = message(exc)
        manifest["errors"].append({"case": name, "stage": stage, "message": message(exc)})
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS, exc:
                manifest["errors"].append({"case": name, "stage": "clear", "message": message(exc)})


OUTPUT_COMMON = [(name, None) for name in ["use_output_name", "output_name", "output_mode",
                                        "output_format", "full_filename", "lines_per_page"]]
DEFAULTS = {
    "transform": [(name, None) for name in ["fields", "formula", "formula_inverse",
        "formula_inverse_offset", "formula_log_n", "formula_log_n_offset", "formula_log_10",
        "formula_log_10_offset", "formula_exponential", "formula_square_root", "paginate_output"]]
        + OUTPUT_COMMON,
    "report": [(name, None) for name in ["format", "text", "highlights", "title"]] + OUTPUT_COMMON,
    "rfmaggregate": [(name, None) for name in ["relative_to", "reference_date", "contiguous",
        "id_field", "date_field", "value_field", "extension", "add_as", "discard_low_value_records",
        "discard_records_below", "only_recent_transactions", "specify_transaction_date",
        "transaction_date_after", "transaction_within_last", "transaction_scale", "save_r2", "save_r3"]],
    "rfmanalysis": [(name, None) for name in ["recency", "frequency", "monetary", "recency_bins",
        "frequency_bins", "monetary_bins", "recency_weight", "frequency_weight", "monetary_weight",
        "tied_values_method", "recalculate_bins", "add_outliers", "binned_field", "recency_thresholds",
        "frequency_thresholds", "monetary_thresholds"]],
    "timeintervals": [(name, None) for name in ["interval_type", "mode", "field", "period_start",
        "cycle_start", "year_start", "quarter_start", "month_start", "day_start", "hour_start",
        "minute_start", "second_start", "periods_per_cycle", "fiscal_year_begins", "week_begins_on",
        "day_begins_hour", "day_begins_minute", "day_begins_second", "days_per_week", "hours_per_day",
        "interval_increment", "field_name_extension", "field_name_extension_as_prefix", "date_format",
        "time_format", "agg_mode", "agg_range_default", "agg_set_default", "agg_flag_default",
        "pad_range_default", "pad_set_default", "pad_flag_default", "max_records_to_create",
        "estimation_from_beginning", "estimation_to_end", "estimation_start_offset",
        "estimation_num_holdouts", "create_future_records", "num_future_records", "create_future_field",
        "future_field_name"]],
    "anonymize": [(name, field, None) for field in ["grp", "iv", "rv", "text_edge", "flg"]
        for name in ["enable_anonymize", "use_prefix", "prefix", "transformation", "set_random_seed",
                     "random_seed", "scale", "translate"]],
}


ensure_clean_dir(MISC_DIR)
write_csv("misc_main.csv", MAIN_SCHEMA, MAIN_ROWS)
write_csv("misc_main_rev.csv", MAIN_SCHEMA, list(reversed(MAIN_ROWS)))
write_csv("misc_transactions.csv", TX_SCHEMA, TX_ROWS)
write_csv("misc_transactions_sorted.csv", TX_SCHEMA,
          sorted(TX_ROWS, key=lambda row: (row[1], row[2], row[0])))
write_csv("misc_transactions_missing.csv", TX_SCHEMA, TX_MISSING_ROWS)
write_csv("misc_rfm.csv", RFM_SCHEMA, RFM_ROWS)
write_csv("misc_rfm_rev.csv", RFM_SCHEMA, list(reversed(RFM_ROWS)))
write_csv("misc_rfm_constant.csv", RFM_SCHEMA,
          [[i, 7.0, 2, 10.0, "2020-03-01", "2020-03-01 00:00:00"] for i in range(1, 13)])
write_csv("misc_time_days.csv", TIME_SCHEMA, TIME_ROWS)
write_csv("misc_time_hours.csv", TIME_SCHEMA, TIME_HOURS)
write_csv("misc_time_minutes.csv", TIME_SCHEMA, TIME_MINUTES)
write_csv("misc_time_seconds.csv", TIME_SCHEMA, TIME_SECONDS)
write_csv("misc_time_missing.csv", TIME_SCHEMA,
          TIME_ROWS + [[12, None] + [5.0] * 7 + ["A", "F"]])


# --- Transform output: mandatory field selection, otherwise native defaults. ---
def transform_case(name, props, purpose, options=None):
    # Existing team Transform CSV has only names and empty statistic cells.
    # Keep one true screen-default control; capture each formula directly in
    # HTML as well, rather than relying on that lossy DocumentOutput export.
    if name != "default" and not any(item[0] == "output_mode" for item in props):
        props = props + [("output_mode", "File"), ("output_format", "HTML"),
                         ("full_filename", MISC_DIR + "/misc_tr_" + name + ".html"),
                         ("paginate_output", False)]
    build_case("tr_" + name, "transform", "transform", "misc_main.csv", MAIN_SCHEMA,
               props, purpose, options)


TR_POS = [("fields", ["pos"])]
transform_case("default", TR_POS, "Default All formulas on strictly positive skewed data")
for suffix, flag in [("inverse", "formula_inverse"), ("ln", "formula_log_n"),
                     ("log10", "formula_log_10"), ("exp", "formula_exponential"),
                     ("sqrt", "formula_square_root")]:
    transform_case(suffix, TR_POS + [("formula", "Select"), (flag, True)],
                   "Only " + suffix + ": selected columns, distribution statistics and precision")
for suffix, flag, offset in [("inverse", "formula_inverse", "formula_inverse_offset"),
                             ("ln", "formula_log_n", "formula_log_n_offset"),
                             ("log10", "formula_log_10", "formula_log_10_offset")]:
    transform_case(suffix + "_offset", [("fields", ["rv"]), ("formula", "Select"),
                   (flag, True), (offset, 5)], "Offset across x=-4, zeros, real ties and nulls")
transform_case("fractional_offset", TR_POS + [("formula", "Select"), ("formula_inverse", True),
               ("formula_inverse_offset", 2.5)], "Check observed integer offset serialization (2.5 -> 2)")
transform_case("domains", [("fields", ["rv", "iv", "constant"])],
               "All formulas: negative/zero domains, nulls, duplicates, constant and selected field order")
transform_case("none", TR_POS + [("formula", "Select")],
               "Select mode with all formula flags left false")
TR_FILE = [("fields", ["pos", "rv", "constant"]), ("output_mode", "File"),
           ("output_format", "HTML"), ("full_filename", MISC_DIR + "/misc_transform.html")]
transform_case("html", TR_FILE + [("paginate_output", False)],
               "Direct HTML file may retain statistics/charts missing from DocumentOutput CSV")
transform_case("html_pages", TR_FILE[:-1] + [("full_filename", MISC_DIR + "/misc_transform_pages.html"),
               ("paginate_output", True), ("lines_per_page", 5)], "HTML pagination and companion files")
transform_case("output_object", TR_POS + [("use_output_name", True), ("output_name", "Misc Transform"),
               ("output_mode", "File"), ("output_format", "Output"),
               ("full_filename", MISC_DIR + "/misc_transform.cou")],
               "Custom output name and native output object delivery")


# --- Report: fixed, ALL and WHERE scopes are public template syntax. ---
REPORT_ALL = "BEGIN" + NL + "#ALL" + NL + "row=[@INDEX];id=[id];x=[rv];s=[text_edge]" + NL + "#" + NL + "END"
REPORT_HTML = '<html><body><h1>Misc &amp; report</h1><table>' + NL + '#ALL' + NL + \
              '<tr><td>[id]</td><td>[text_edge]</td><td>[rv]</td></tr>' + NL + '#' + NL + '</table></body></html>'


def report_case(name, props, purpose, options=None, csv_name="misc_main.csv"):
    build_case("rep_" + name, "report", "report", csv_name, MAIN_SCHEMA, props, purpose, options)


report_case("default", [("text", "Fixed probe text")], "Fixed text once, remaining settings at defaults")
report_case("all", [("text", REPORT_ALL)], "ALL iteration, @INDEX, null/empty/spaces, real rendering")
report_case("reverse", [("text", REPORT_ALL)], "Source ordering and record index", csv_name="misc_main_rev.csv")
report_case("where", [("text", "before" + NL + "#WHERE [rv >= 1]" + NL + "[id]:[rv * 2]" + NL + "#" + NL + "after")],
            "Conditional iteration: false/null conditions and inline arithmetic")
report_case("scopes", [("text", "HEAD" + NL + "#ALL" + NL + "A[id]" + NL + "#" + NL +
             "MIDDLE" + NL + "#WHERE grp = 'A'" + NL + "B[id]" + NL + "#" + NL + "TAIL")],
            "Multiple blocks: block-major versus record-major output; unbracketed WHERE")
report_case("no_matches", [("text", "HEAD" + NL + "#WHERE id < 0" + NL + "NEVER" + NL + "#" + NL + "TAIL")],
            "Empty conditional block keeps fixed content")
report_case("empty_input", [("text", REPORT_ALL)], "Empty input retains fixed text", {"empty": True})
report_case("auto_html", [("text", REPORT_HTML), ("format", "Auto"), ("output_mode", "File"),
             ("output_format", "HTML"), ("full_filename", MISC_DIR + "/misc_report_auto.html")],
            "Automatic formatting of HTML-like input and markup characters")
report_case("custom_html", [("text", REPORT_HTML), ("format", "Custom"), ("output_mode", "File"),
             ("output_format", "HTML"), ("full_filename", MISC_DIR + "/misc_report_custom.html")],
            "Custom HTML template interpreted rather than automatically formatted")
report_case("decorated", [("text", REPORT_ALL), ("title", "Misc Report Title"), ("highlights", True),
             ("use_output_name", True), ("output_name", "Misc Report"), ("output_mode", "File"),
             ("output_format", "HTML"), ("full_filename", MISC_DIR + "/misc_report_decorated.html"),
             ("lines_per_page", 5)], "Title, expression highlighting, output label, page boundary")
report_case("plain_html", [("text", REPORT_ALL), ("title", "Misc Report Title"), ("highlights", False),
             ("output_mode", "File"), ("output_format", "HTML"),
             ("full_filename", MISC_DIR + "/misc_report_plain.html"), ("lines_per_page", 5)],
            "HTML without highlighted expression values (compare decorated)")
report_case("text_file", [("text", REPORT_ALL), ("output_mode", "File"), ("output_format", "Text"),
             ("full_filename", MISC_DIR + "/misc_report.txt")], "Direct text-file encoding/newlines")
report_case("output_object", [("text", REPORT_ALL), ("output_mode", "File"), ("output_format", "Output"),
             ("full_filename", MISC_DIR + "/misc_report.cou")], "Native output object delivery")


# --- Set Globals: sibling CLEM reader + Table, plus direct public-API snapshot. ---
GLOBAL_TYPES = [("Sum", modeler.api.GlobalValues.Type.SUM), ("Mean", modeler.api.GlobalValues.Type.MEAN),
                ("Min", modeler.api.GlobalValues.Type.MIN), ("Max", modeler.api.GlobalValues.Type.MAX),
                ("SDev", modeler.api.GlobalValues.Type.STDDEV)]
ALL_STATS = [item[0] for item in GLOBAL_TYPES]


def globals_snapshot(stream, fields):
    values = []
    globals_value = stream.getGlobalValues()
    for field in fields:
        for name, kind in GLOBAL_TYPES:
            try:
                value = globals_value.getValue(kind, field)
                values.append({"field": field, "statistic": name, "value": snapshot(value)})
            except ERRORS, exc:
                values.append({"field": field, "statistic": name, "read_error": message(exc)})
    return values


def globals_case(name, selections, purpose, options=None, props=None, prior=False):
    if options is None:
        options = {}
    if props is None:
        props = []
    stream = None
    entry = {"case": "gl_" + name, "file": "misc_gl_" + name + ".str", "data": "misc_main.csv",
             "node_type": "setglobals", "catalog_id": "set_globals", "purpose": purpose,
             "properties": [], "setup_properties": [], "options": options, "decimal_places": 15,
             "capture_warning": "CLEM witness needs Set Globals before Table; oracle ignores stream scripts"}
    manifest["cases"].append(entry)
    stage = "create"
    try:
        stream = taskrunner.createStream(entry["case"], False, False)
        stream.setPropertyValue("decimal_places", 15)
        upstream = base_input(stream, "misc_main.csv", MAIN_SCHEMA, entry, options)
        prior_node = None
        if prior:
            first = stream.createAt("select", "PrimeFirstRecords", 360, 280)
            stream.link(upstream, first)
            set_props(first, [("mode", "Include"), ("condition", "id <= 3")], entry["setup_properties"])
            prior_node = stream.createAt("setglobals", "00_Prime", 480, 220)
            stream.link(first, prior_node)
            set_props(prior_node, [("globals", "iv", ALL_STATS), ("globals", "rv", ALL_STATS),
                                   ("clear_first", True),
                                   ("show_preview", False)], entry["setup_properties"])
        node = stream.createAt("setglobals", "01_SetGlobals", 480, 100)
        stream.link(upstream, node)
        entry["defaults"] = get_props(node, [("clear_first", None), ("show_preview", None)] +
                                      [("globals", field, None) for field, stats in selections])
        stage = "configure"
        set_props(node, [("globals", field, list(stats)) for field, stats in selections] + props,
                  entry["properties"])
        witness = upstream
        for field, stats in selections:
            for stat in stats:
                function = stat.upper()
                witness = derive(stream, witness, "g_" + field + "_" + function,
                                 "@GLOBAL_%s(%s)" % (function, field),
                                 "Range" if stat not in ["Min", "Max"] else
                                 dict([(item[0], item[2]) for item in MAIN_SCHEMA] +
                                      [("all_null", "Range"), ("text_edge", "Set")]).get(field, "Range"),
                                 entry["setup_properties"])
        if prior and ("clear_first", False) in props:
            witness = derive(stream, witness, "retained_iv_sum", "@GLOBAL_SUM(iv)", "Range",
                             entry["setup_properties"])
        table = stream.createAt("table", "02_GlobalValues", 840, 100)
        stream.link(witness, table)
        entry["terminal_order"] = terminal_order(stream)
        entry["terminal"] = ["setglobals", "table"]
        stage = "save_configured"
        path = MISC_DIR + "/" + entry["file"]
        taskrunner.saveStreamToFile(stream, path)
        manifest["saved"].append(entry["file"])
        stage = "validate_globals"
        if prior_node is not None:
            prior_node.run([])
            entry["before"] = globals_snapshot(stream, ["iv", "rv"])
        node.run([])
        fields = sorted(set([field for field, stats in selections] + ["iv", "rv"]))
        entry["after"] = globals_snapshot(stream, fields)
        entry["effective_properties"] = get_props(node, [("clear_first", None), ("show_preview", None)])
        stage = "validate_witness"
        table.run([])
        entry["validation"] = "ok"
        stage = "save_validated"
        taskrunner.saveStreamToFile(stream, path)
    except ERRORS, exc:
        entry["validation"] = "error"
        entry["error_stage"] = stage
        entry["error"] = message(exc)
        manifest["errors"].append({"case": entry["case"], "stage": stage, "message": message(exc)})
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS, exc:
                manifest["errors"].append({"case": entry["case"], "stage": "clear", "message": message(exc)})


globals_case("default", [("iv", ALL_STATS), ("rv", ALL_STATS)],
             "Numeric globals, duplicate/null exclusion and sample versus population SDev")
globals_case("constant", [("constant", ALL_STATS)], "Constant field: SDev zero")
globals_case("singleton", [("rv", ALL_STATS)], "One valid observation: SDev/null convention", {"singleton": True})
globals_case("all_null", [("all_null", ALL_STATS)], "No valid numeric observations", {"all_null_field": True})
# Retain the historical case id; zero records failed before any globals existed.
# Two valid observations exercise the smallest sample with a defined SDev.
globals_case("empty", [("rv", ALL_STATS)], "Minimum valid SDev sample: ids 1 and 2", {"first_two": True})
globals_case("types", [("text_edge", ["Min", "Max"]), ("dt", ["Min", "Max"]),
             ("ts", ["Min", "Max"])], "String/date/timestamp extrema and explicit null/empty/space")
globals_case("missing", [("rv", ALL_STATS)], "User missing zero versus system null", {"user_missing": True})
globals_case("retain", [("rv", ["Sum", "Mean"])], "Keep earlier iv globals, replace selected globals",
             props=[("clear_first", False), ("show_preview", False)], prior=True)
globals_case("clear", [("rv", ["Sum", "Mean"])], "Remove earlier iv globals (direct API before/after)",
             props=[("clear_first", True), ("show_preview", True)], prior=True)


# --- RFM Aggregate: absolute-date cases deterministic; default Today documented. ---
RA_FIELDS = [("id_field", "customer"), ("date_field", "purchased"), ("value_field", "amount")]
RA_FIXED = RA_FIELDS + [("relative_to", "Fixed"), ("reference_date", "2020-03-31")]


def aggregate_case(name, props, purpose, csv_name="misc_transactions.csv"):
    build_case("ra_" + name, "rfmaggregate", "rfm_aggregate", csv_name, TX_SCHEMA, props, purpose)


aggregate_case("default_today", RA_FIELDS, "Native Today default: time-sensitive recency; pin run date")
aggregate_case("fixed", RA_FIXED, "Fixed reference, future/duplicate transactions, output ordering and schema")
aggregate_case("contiguous", RA_FIXED + [("contiguous", True)],
               "Pre-grouped ID optimization preserves totals", "misc_transactions_sorted.csv")
aggregate_case("suffix", RA_FIXED + [("extension", "_Q1"), ("add_as", "Suffix")], "Generated field suffix")
aggregate_case("prefix", RA_FIXED + [("extension", "Q1_"), ("add_as", "Prefix")], "Generated field prefix")
aggregate_case("min_value", RA_FIXED + [("discard_low_value_records", True), ("discard_records_below", 10.0)],
               "Strict threshold: negatives, zero, 9.99/10/10.01; groups losing every transaction")
aggregate_case("date_after", RA_FIXED + [("only_recent_transactions", True),
               ("specify_transaction_date", True), ("transaction_date_after", "2020-02-29")],
               "Absolute cutoff inclusion/exclusion at leap day")
for scale, count in [("Days", 7), ("Weeks", 2), ("Months", 1), ("Years", 1)]:
    aggregate_case("last_" + scale.lower(), RA_FIXED + [("only_recent_transactions", True),
                   ("specify_transaction_date", False), ("transaction_within_last", count),
                   ("transaction_scale", scale)], "Relative calendar window: " + scale + " boundaries")
aggregate_case("second", RA_FIXED + [("save_r2", True)], "Second latest date, duplicate dates and short groups")
aggregate_case("third", RA_FIXED + [("save_r2", True), ("save_r3", True)],
               "Third latest date: duplicates count as transactions or distinct dates")
aggregate_case("timestamp", [("id_field", "customer"), ("date_field", "stamp"), ("value_field", "amount"),
               ("relative_to", "Fixed"), ("reference_date", "2020-03-31"), ("save_r2", True), ("save_r3", True)],
               "Timestamp recency rounding and sub-day ordering")
aggregate_case("missing", RA_FIXED, "Null date/value/ID, empty ID, all-null customer", "misc_transactions_missing.csv")


# --- RFM Analysis: independent bins, orientation by storage and persisted limits. ---
RF_FIELDS = [("recency", "r"), ("frequency", "f"), ("monetary", "m")]
RF_LIMITS = [("recency_bins", 3), ("frequency_bins", 3), ("monetary_bins", 3),
             ("recency_thresholds", [1, 4, 10, 20]), ("frequency_thresholds", [1, 3, 6, 10]),
             ("monetary_thresholds", [20, 100, 200, 500])]


def analysis_case(name, props, purpose, csv_name="misc_rfm.csv", options=None):
    build_case("rf_" + name, "rfmanalysis", "rfm_analysis", csv_name, RFM_SCHEMA, props, purpose, options)


analysis_case("default", RF_FIELDS, "Default quintiles, Current ties, 100/10/1 weights; partial/all nulls")
analysis_case("reverse", RF_FIELDS, "Row-order invariance for ties", "misc_rfm_rev.csv")
analysis_case("next", RF_FIELDS + [("tied_values_method", "Next")], "Move threshold ties into next bin")
analysis_case("mixed_bins", RF_FIELDS + [("recency_bins", 2), ("frequency_bins", 3), ("monetary_bins", 4)],
              "Unequal bin counts, minimum 2, independent per-measure partitioning")
analysis_case("nine_bins", RF_FIELDS + [("recency_bins", 9), ("frequency_bins", 9), ("monetary_bins", 9)],
              "Maximum 9 bins with ties and fewer distinct values")
analysis_case("weights", RF_FIELDS + [("recency_weight", 0.5), ("frequency_weight", 0), ("monetary_weight", 2.25)],
              "Fractional and zero weights, score arithmetic and storage")
analysis_case("date", [("recency", "last_date"), ("frequency", "f"), ("monetary", "m")],
              "Date recency ranks later dates higher, opposite to numeric elapsed time")
analysis_case("timestamp", [("recency", "last_stamp"), ("frequency", "f"), ("monetary", "m")],
              "Timestamp recency, sub-day distinctions")
analysis_case("constant", RF_FIELDS, "All measures tied", "misc_rfm_constant.csv")
for include in [False, True]:
    analysis_case("limits_" + ("clamp" if include else "null"), RF_FIELDS +
                  [("recalculate_bins", "IfNecessary")] + RF_LIMITS + [("add_outliers", include),
                  ("binned_field", "Monetary" if include else "Frequency")],
                  "Persist explicit limits: boundary equality and out-of-range " + ("clamp" if include else "null"))
analysis_case("always_limits", RF_FIELDS + [("recalculate_bins", "Always")] + RF_LIMITS +
              [("binned_field", "Recency")], "Always recompute versus supplied limits (documentation discrepancy)")
analysis_case("ifnecessary", RF_FIELDS + [("recalculate_bins", "IfNecessary")],
              "No preexisting limits: calculate once, save them, reuse on oracle replay")
analysis_case("singleton", RF_FIELDS, "One customer cannot fill quintiles", options={"singleton": True})
analysis_case("empty", RF_FIELDS, "Zero customers, preserved schema or controlled error", options={"empty": True})


# --- Legacy Time Intervals: 13 usable interval enums and bounded edge cases. ---
def time_case(name, csv_name, schema, props, purpose):
    build_case("ti_" + name, "timeintervals", "time_intervals", csv_name, schema, props, purpose)


def label_case(name, props, purpose):
    time_case(name, "misc_main.csv", MAIN_SCHEMA, [("mode", "Label")] + props, purpose)


# Native interval None is captured by DEFAULTS but cannot execute (run 1).
time_case("default", "misc_main.csv", MAIN_SCHEMA, [("interval_type", "Periods")],
          "Mandatory period interval only; otherwise native label defaults")
# Historical id: use nonperiodic daily labels instead of an unset interval.
label_case("none", [("interval_type", "DaysNonPeriodic"), ("year_start", 2020),
           ("month_start", "February"), ("day_start", 28)],
           "Daily labels through leap day; explicit Label mode versus daily Create")
label_case("periods", [("interval_type", "Periods"), ("period_start", 7)], "Consecutive integer periods start at 7")
label_case("cycles", [("interval_type", "CyclicPeriods"), ("periods_per_cycle", 4),
           ("cycle_start", 2), ("period_start", 3)], "Cycle wrap, starting partway through cycle")
label_case("years", [("interval_type", "Years"), ("year_start", 1999)], "Year labels through century boundary")
label_case("quarters", [("interval_type", "Quarters"), ("year_start", 2019), ("quarter_start", 4),
           ("fiscal_year_begins", "April"), ("field_name_extension", "Q_"),
           ("field_name_extension_as_prefix", True)], "Fiscal quarter wrap and generated-field prefix")
label_case("months", [("interval_type", "Months"), ("year_start", 2019), ("month_start", "December"),
           ("date_format", "DD/MM/YYYY"), ("field_name_extension", "_month"),
           ("field_name_extension_as_prefix", False)], "Month/year rollover, label format, suffix")
label_case("workdays", [("interval_type", "DaysPerWeek"), ("year_start", 2020), ("month_start", "February"),
           ("day_start", 28), ("days_per_week", 4), ("week_begins_on", "Tuesday")],
           "Four-day week beginning Tuesday; skip inactive days, cross leap day")
label_case("workhours", [("interval_type", "HoursPerDay"), ("year_start", 2020), ("month_start", "February"),
           ("day_start", 28), ("hour_start", 9), ("days_per_week", 5), ("week_begins_on", "Monday"),
           ("hours_per_day", 2), ("day_begins_hour", 8)], "Two-hour workday and weekend rollover")
label_case("workminutes", [("interval_type", "MinutesPerDay"), ("year_start", 2020), ("month_start", "February"),
           ("day_start", 28), ("hour_start", 8), ("minute_start", 50), ("interval_increment", 15),
           ("days_per_week", 5), ("week_begins_on", "Monday"), ("hours_per_day", 1),
           ("day_begins_hour", 8), ("day_begins_minute", 5)], "15-minute increments, nonzero business-day minute")
label_case("workseconds", [("interval_type", "SecondsPerDay"), ("year_start", 2020), ("month_start", "February"),
           ("day_start", 28), ("hour_start", 8), ("minute_start", 5), ("second_start", 47),
           ("interval_increment", 30), ("days_per_week", 4), ("week_begins_on", "Tuesday"),
           ("hours_per_day", 1), ("day_begins_hour", 8), ("day_begins_minute", 5),
           ("day_begins_second", 17), ("time_format", "HH.MM.SS")],
           "30-second increments with exact business start 08:05:17 and custom time format")

TI_CREATE = [("mode", "Create"), ("field", "tick"), ("max_records_to_create", 200),
             ("create_future_records", False)]
time_case("days_default", "misc_time_days.csv", TIME_SCHEMA, TI_CREATE +
          [("interval_type", "DaysNonPeriodic")], "Default aggregation/padding: duplicate timestamps, leap day, gaps")
TI_KEYED = [("aggregate", name, method) for name, method in
            [("x_mean", "Mean"), ("x_sum", "Sum"), ("x_mode", "Mode"), ("x_min", "Min"),
             ("x_max", "Max"), ("x_first", "First"), ("x_last", "Last"), ("flg", "TrueIfAnyTrue")]]
time_case("days_specify", "misc_time_days.csv", TIME_SCHEMA, TI_CREATE +
          [("interval_type", "DaysNonPeriodic"), ("agg_mode", "Specify"), ("agg_range_default", "Sum"),
           ("agg_set_default", "Last"), ("agg_flag_default", "Last"), ("pad_range_default", "MeanOfRecentPoints"),
           ("pad_set_default", "MostRecentValue"), ("pad_flag_default", "True")] + TI_KEYED +
          [("pad", "x_mean", "MeanOfRecentPoints"), ("pad", "x_sum", "Blank"), ("pad", "flg", "False"),
           ("create_future_records", True), ("num_future_records", 3), ("create_future_field", True),
           ("future_field_name", "is_future")], "All keyed aggregates and padding overrides, future rows/flag")
time_case("hours", "misc_time_hours.csv", TIME_SCHEMA, TI_CREATE +
          [("interval_type", "HoursNonPeriodic"), ("agg_mode", "All"), ("agg_range_default", "Mode"),
           ("agg_set_default", "First"), ("agg_flag_default", "First"), ("pad_range_default", "Blank"),
           ("pad_set_default", "Blank"), ("pad_flag_default", "False"),
           ("estimation_from_beginning", False), ("estimation_start_offset", 1),
           ("estimation_to_end", False), ("estimation_num_holdouts", 1)],
          "Hourly bins: mode/ties, first categorical/flag, estimation start/end exclusions")
time_case("minutes", "misc_time_minutes.csv", TIME_SCHEMA, TI_CREATE +
          [("interval_type", "MinutesNonPeriodic"), ("agg_mode", "All"), ("agg_range_default", "Min"),
           ("agg_set_default", "Mode"), ("agg_flag_default", "Mode"), ("pad_range_default", "MeanOfRecentPoints"),
           ("pad_flag_default", "Blank"), ("create_future_records", True), ("num_future_records", 2),
           ("create_future_field", False)], "Minute bins, mode tie, mean-of-recent gap/future without flag")
time_case("seconds", "misc_time_seconds.csv", TIME_SCHEMA, TI_CREATE +
          [("interval_type", "SecondsNonPeriodic"), ("agg_mode", "All"), ("agg_range_default", "Max"),
           ("agg_set_default", "Last"), ("agg_flag_default", "TrueIfAnyTrue"), ("pad_flag_default", "True"),
           ("create_future_field", True), ("future_field_name", "future_marker")],
          "Second bins: duplicate exact timestamp, max/last/any, null final bin and future flag")
time_case("missing_time", "misc_time_missing.csv", TIME_SCHEMA, TI_CREATE +
          [("interval_type", "DaysNonPeriodic")], "Missing timestamp: discard, null group or controlled error")
time_case("padding_limit", "misc_time_seconds.csv", TIME_SCHEMA, TI_CREATE +
          [("interval_type", "SecondsNonPeriodic"), ("max_records_to_create", 8)],
          "Small sufficient padding cap: eight time slots, four gap records")


# --- Anonymize: Range uses transform; Set/Flag use symbolic replacements. ---
def anonym_case(name, props, purpose, options=None, csv_name="misc_main.csv"):
    build_case("an_" + name, "anonymize", "anonymize", csv_name, MAIN_SCHEMA, props, purpose, options)


def fixed(field, scale, translate):
    return [("enable_anonymize", field, True), ("transformation", field, "Fixed"),
            ("scale", field, scale), ("translate", field, translate)]


def seeded(field, seed):
    return [("enable_anonymize", field, True), ("transformation", field, "Random"),
            ("set_random_seed", field, True), ("random_seed", field, seed)]


anonym_case("default", [], "Untouched node: field/value pass-through")
anonym_case("symbols", [("enable_anonymize", "grp", True)], "Default symbolic replacement, duplicates and empty IDs")
anonym_case("prefix", [("enable_anonymize", "grp", True), ("use_prefix", "grp", True), ("prefix", "grp", "cust_")],
            "Custom symbolic prefix and numbering")
anonym_case("prefix_off", [("enable_anonymize", "grp", True), ("use_prefix", "grp", False),
            ("prefix", "grp", "ignored_")], "Disabled custom prefix is ignored")
anonym_case("multi_symbols", [("enable_anonymize", field, True) for field in ["grp", "text_edge", "flg"]],
            "Independent symbol dictionaries, explicit null/empty/spaces, case, punctuation, flag storage")
anonym_case("reverse", [("enable_anonymize", "grp", True)], "Encounter order versus sorted symbolic numbering",
            csv_name="misc_main_rev.csv")
anonym_case("integer_set", [("enable_anonymize", "iv", True)],
            "Same integer storage, nominal measurement selects symbolic replacement", {"integer_as_set": True})
anonym_case("identity", fixed("rv", 1.0, 0.0), "Fixed identity retains nulls and real precision")
# Run 1 rejects a Java Double at scale; pass Python ints for these fixed cases.
anonym_case("scale", fixed("rv", 2, 0), "Fixed scale only, signed/zero/real/null values")
anonym_case("translate", fixed("rv", 1, 3), "Fixed translation only")
anonym_case("combined", fixed("rv", 2.5, -3.0) + fixed("iv", 2.5, -3.0),
            "Distinguish A*(x+B) from A*x+B, real output from integer input")
anonym_case("seed123", seeded("rv", 123), "Reproducible random continuous parameters, nulls/duplicate preservation")
anonym_case("seed321", seeded("rv", 321), "Seed affects random continuous parameters")
anonym_case("multi_random", seeded("rv", 123) + seeded("iv", 123), "Per-field same seeds and storage-dependent parameters")
anonym_case("missing", fixed("rv", 2.5, -3.0) + [("enable_anonymize", "grp", True)],
            "User-missing numeric/category values versus null/empty", {"user_missing": True})


# The probe manifest is outside the corpus in the standard probe output folder.
# Also retain a corpus copy so the coordinator can pin the case/property evidence.
for path in [OUT_DIR + "/manifest.json", MISC_DIR + "/misc_manifest.json"]:
    handle = open(path, "w")
    try:
        json.dump(manifest, handle, indent=2, sort_keys=True)
    finally:
        handle.close()
