# Probe: E-Plot (Beta) node (label eplot, case prefix eplot_). At most 16 cases.
# Public API only: Modeler 19.0 Python Scripting and Automation Guide, Table 107
# (eplotnode Properties), excerpt target/eplot/ModelerScriptingAutomation-19.0.txt.
# The user guide (ug.txt) lists E-Plot under Graphs / Analytic Server support and
# has no further options. Follow graph_streams.py / graphboard_more_streams.py.
# Jython 2.7: no f-strings, no *rest, no nonlocal; real division uses float literals.
# Do not delete the corpus directory: the coordinator may run this probe in batches.
# Skip a case whose .str already exists.
# Run with `cargo xtask probe eplot_streams` (IBM SPSS Modeler, coordinator only).
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/eplot"
CSV_MAIN = "eplot_main.csv"
CSV_LARGE = "eplot_large.csv"
# `except EXC as e` is legal Jython 2.7. The Python 2 comma form is a Python 3 syntax error.
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {
    "label": "eplot",
    "input": CSV_MAIN,
    "rows": 40,
    "extra_inputs": {CSV_LARGE: 80},
    "saved": [],
    "errors": [],
    "cases": [],
    "notes": [
        "Table 107 properties: x_field, y_field, color_field, size_field, shape_field, interested_fields, records_limit, if_over_limit.",
        "records_limit default 2000. if_over_limit is typed Boolean; prose names Sample (default, random sample to the limit) and Use all data.",
        "interested_fields is typed string; prose says fields. This probe tries a list first, then a comma-separated string.",
        "if_over_limit tries PlotSample/PlotAll (plotnode), then Sample/All, then Boolean True/False, then UseAll.",
        "The first case (eplot_xy) records property defaults, run results, and every DocumentOutput export attempt.",
        "Export order matches xtask/oracle/modeler_oracle.py: csv, json, text, vizml, html.",
        "Existing .str files are left in place; those cases are skipped.",
    ],
}

NROWS_MAIN = 40
NROWS_LARGE = 80
REGIONS = ["North", "South", "East"]
CHANNELS = ["Store", "Web"]
SEGMENTS = ["Retail", "Trade"]
# (name, variablefile custom_storage_type, Type measurement). Every direction is Input.
SCHEMA = [
    ("id", "Integer", "Range"),
    ("region", "String", "Set"),
    ("channel", "String", "Set"),
    ("segment", "String", "Set"),
    ("amount", "Real", "Range"),
    ("score", "Real", "Range"),
    ("weight", "Real", "Range"),
    ("when", "Date", "Range"),
]
EPLOT_PROPS = [
    "x_field", "y_field", "color_field", "size_field", "shape_field",
    "interested_fields", "records_limit", "if_over_limit",
]
# Same priority as xtask/oracle/modeler_oracle.py DOCUMENT_FORMATS.
DOCUMENT_FORMATS = [
    ("csv", "csv", "COMMA_DELIMITED"),
    ("json", "json", "JSON"),
    ("text", "txt", "PLAIN_TEXT"),
    ("vizml", "vizml", "VIZML"),
    ("html", "html", "HTML"),
]


def message(exc):
    get = getattr(exc, "getMessage", None)
    if get is not None:
        return unicode(get())
    return unicode(exc)


def text(value):
    if value is None:
        return None
    if isinstance(value, (bool, int, long, float)):
        return value
    if isinstance(value, (list, tuple)):
        return [text(v) for v in value]
    try:
        return unicode(value)
    except Exception:
        return repr(value)


def java_name(obj):
    try:
        return unicode(obj.getClass().getName())
    except ERRORS:
        return unicode(type(obj))


def ensure_dir(path):
    d = java.io.File(path)
    if not d.exists():
        d.mkdirs()


def decimal_text(numerator, scale, digits):
    # Exact decimal text. scale is 10**digits. No binary float.
    if numerator < 0:
        return "-" + decimal_text(-numerator, scale, digits)
    whole = numerator // scale
    frac = numerator % scale
    if frac == 0:
        return "%d" % whole
    frac_text = ("%0*d" % (digits, frac)).rstrip("0")
    return "%d.%s" % (whole, frac_text)


def ymd_from_day(offset):
    # 2020-01-01 plus offset days. 2020 is a leap year.
    year = 2020
    month = 1
    day = 1 + offset
    mdays = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    while day > mdays[month - 1]:
        day = day - mdays[month - 1]
        month = month + 1
        if month > 12:
            month = 1
            year = year + 1
            if year % 4 == 0 and (year % 100 != 0 or year % 400 == 0):
                mdays[1] = 29
            else:
                mdays[1] = 28
    return "%04d-%02d-%02d" % (year, month, day)


def row_cells(i):
    ident = "%d" % (i + 1)
    region = REGIONS[i % 3]
    if i % 29 == 0:
        region = ""
    channel = CHANNELS[(i // 3) % 2]
    segment = SEGMENTS[(i // 6) % 2]
    if i % 11 == 0:
        amount = ""
    elif i % 23 == 4:
        amount = "12.123456789012345"
    else:
        # (i % 17) - 8 + (i % 4) * 0.25
        hundredths = ((i % 17) - 8) * 100 + (i % 4) * 25
        amount = decimal_text(hundredths, 100, 2)
    if i % 13 == 0:
        score = ""
    else:
        # (i % 13) * 0.5 + 0.125
        thousandths = (i % 13) * 500 + 125
        score = decimal_text(thousandths, 1000, 3)
    if i % 17 == 0:
        weight = ""
    else:
        # 1 + (i % 9) * 0.1
        tenths = 10 + (i % 9)
        weight = decimal_text(tenths, 10, 1)
    if i % 19 == 0:
        when = ""
    else:
        when = ymd_from_day(i)
    return [ident, region, channel, segment, amount, score, weight, when]


def write_csv(name, nrows):
    if nrows < 10 or nrows > 200:
        raise ValueError("row count out of range")
    lines = [",".join([field[0] for field in SCHEMA])]
    i = 0
    while i < nrows:
        cells = row_cells(i)
        if len(cells) != len(SCHEMA):
            raise ValueError("wrong width")
        lines.append(",".join(cells))
        i = i + 1
    handle = open(DIR + "/" + name, "wb")
    try:
        handle.write(NL.join(lines) + NL)
    finally:
        handle.close()
    manifest["saved"].append(name)


def instantiate_node(stream, node):
    table = stream.createAt("table", "TempTable", 760, 360)
    stream.link(node, table)
    try:
        table.run([])
    finally:
        stream.delete(table)


def read_prop(node, name):
    try:
        return text(node.getPropertyValue(name))
    except ERRORS as exc:
        return {"error": message(exc)}


def set_first(node, name, candidates, attempts):
    i = 0
    while i < len(candidates):
        value = candidates[i]
        attempt = {"property": name, "set": text(value)}
        try:
            node.setPropertyValue(name, value)
            attempt["ok"] = True
            attempt["readback"] = read_prop(node, name)
            attempts.append(attempt)
            return value
        except ERRORS as exc:
            attempt["ok"] = False
            attempt["message"] = message(exc)
            attempts.append(attempt)
        i = i + 1
    return None


def describe_result(result):
    item = {"class": java_name(result)}
    try:
        item["label"] = unicode(result.getLabel())
    except ERRORS as exc:
        item["label_error"] = message(exc)
    try:
        tags = result.getContentModelTags()
        item["content_model_tags"] = [unicode(tag) for tag in tags]
    except ERRORS as exc:
        item["content_model_tags_error"] = message(exc)
    try:
        item["document_output"] = isinstance(result, modeler.api.DocumentOutput)
        item["model_output"] = isinstance(result, modeler.api.ModelOutput)
    except ERRORS as exc:
        item["type_check_error"] = message(exc)
    return item


def export_documents(results, case_name, record_attempts):
    out = {
        "document_output": False,
        "exportable": False,
        "format": None,
        "file": None,
        "bytes": 0,
        "attempts": [],
    }
    documents = []
    for result in results:
        try:
            if isinstance(result, modeler.api.DocumentOutput):
                documents.append(result)
        except ERRORS:
            pass
    if len(documents) == 0:
        out["note"] = "node ran but produced no DocumentOutput"
        return out
    out["document_output"] = True
    out["document_count"] = len(documents)
    ensure_dir(OUT_DIR + "/outputs")
    last_err = None
    for fmt_name, ext, fmt_attr in DOCUMENT_FORMATS:
        fmt_obj = getattr(modeler.api.FileFormat, fmt_attr, None)
        attempt = {"format": fmt_name, "enum": fmt_attr}
        if fmt_obj is None:
            attempt["ok"] = False
            attempt["message"] = "FileFormat.%s is missing" % fmt_attr
            out["attempts"].append(attempt)
            continue
        path = "%s/outputs/%s.%s" % (OUT_DIR, case_name, ext)
        f = java.io.File(path)
        if f.exists():
            f.delete()
        try:
            taskrunner.exportDocumentToFile(documents[0], path, fmt_obj)
        except ERRORS as exc:
            last_err = exc
            attempt["ok"] = False
            attempt["message"] = message(exc)
            if f.exists():
                attempt["bytes"] = f.length()
                f.delete()
            out["attempts"].append(attempt)
            continue
        size = 0
        if f.exists():
            size = f.length()
        attempt["bytes"] = size
        if f.exists() and size > 0:
            attempt["ok"] = True
            attempt["file"] = path
            out["attempts"].append(attempt)
            if not out["exportable"]:
                out["exportable"] = True
                out["format"] = fmt_name
                out["file"] = path
                out["bytes"] = size
                if not record_attempts:
                    return out
        else:
            attempt["ok"] = False
            attempt["message"] = "empty file"
            if f.exists():
                f.delete()
            out["attempts"].append(attempt)
    if not out["exportable"]:
        if last_err is not None:
            out["note"] = "DocumentOutput present but no export format wrote a non-empty file: %s" % message(last_err)
        else:
            out["note"] = "DocumentOutput present but no export format wrote a non-empty file"
    return out


def build_case(case_name, props, csv_name=None, record_defaults=False):
    if csv_name is None:
        csv_name = CSV_MAIN
    file_name = "%s.str" % case_name
    recorded = []
    for name, value in props:
        if isinstance(value, list):
            recorded.append([name, list(value)])
        else:
            recorded.append([name, value])
    entry = {
        "case": case_name,
        "file": file_name,
        "node_type": "eplot",
        "input": csv_name,
        "properties": recorded,
        "status": "error",
        "ran": False,
        "exportable": False,
    }
    str_path = "%s/%s" % (DIR, file_name)
    if java.io.File(str_path).exists():
        entry["status"] = "skipped"
        entry["note"] = "existing .str left in place"
        manifest["cases"].append(entry)
        return
    stream = None
    stage = "stream"
    try:
        stream = taskrunner.createStream(case_name, False, False)
        stream.setPropertyValue("decimal_places", 15)
        stream.setPropertyValue("date_format", "YYYY-MM-DD")
        src = stream.createAt("variablefile", csv_name, 100, 100)
        src.setPropertyValue("full_filename", DIR + "/" + csv_name)
        src.setPropertyValue("read_field_names", True)
        src.setPropertyValue("delimit_comma", True)
        src.setPropertyValue("decimal_symbol", "Period")
        src.setPropertyValue("encoding", "UTF-8")
        src.setPropertyValue("auto_recognize_datetime", False)
        for field_name, storage, measure in SCHEMA:
            src.setKeyedPropertyValue("custom_storage_type", field_name, storage)
            if storage == "Date":
                src.setKeyedPropertyValue("custom_date_format", field_name, "YYYY-MM-DD")
        type_node = stream.createAt("type", "Type", 250, 100)
        stream.link(src, type_node)
        instantiate_node(stream, type_node)
        for field_name, storage, measure in SCHEMA:
            type_node.setKeyedPropertyValue("type", field_name, measure)
            type_node.setKeyedPropertyValue("direction", field_name, "Input")
        instantiate_node(stream, type_node)
        stage = "configure"
        graph = stream.createAt("eplot", case_name, 460, 100)
        try:
            entry["created_type"] = unicode(graph.getTypeName())
        except ERRORS as exc:
            entry["created_type_error"] = message(exc)
        stream.link(type_node, graph)
        if record_defaults:
            defaults = {}
            for name in EPLOT_PROPS:
                defaults[name] = read_prop(graph, name)
            entry["defaults"] = defaults
        attempts = []
        for name, value in props:
            if name == "if_over_limit" and isinstance(value, list):
                chosen = set_first(graph, name, value, attempts)
                if chosen is None:
                    raise Exception("if_over_limit rejected every candidate")
            elif name == "interested_fields" and isinstance(value, list):
                chosen = set_first(graph, name, [value, ",".join(value)], attempts)
                if chosen is None:
                    raise Exception("interested_fields rejected list and comma string")
            else:
                graph.setPropertyValue(name, value)
        if len(attempts) > 0:
            entry["set_attempts"] = attempts
        readback = {}
        for name in EPLOT_PROPS:
            readback[name] = read_prop(graph, name)
        entry["readback"] = readback
        stage = "save"
        taskrunner.saveStreamToFile(stream, str_path)
        manifest["saved"].append(file_name)
        entry["status"] = "saved"
        stage = "run"
        results = []
        graph.run(results)
        entry["ran"] = True
        described = []
        for result in results:
            described.append(describe_result(result))
        entry["results"] = described
        entry["result_count"] = len(results)
        stage = "export"
        exported = export_documents(results, case_name, record_defaults)
        entry["document_output"] = exported["document_output"]
        entry["exportable"] = exported["exportable"]
        entry["export"] = exported
        if record_defaults and (not exported["exportable"]):
            if exported.get("document_output"):
                entry["batch_note"] = "ran and produced DocumentOutput but no exportable file in batch"
            else:
                entry["batch_note"] = "ran but produced no exportable output in batch"
    except ERRORS as exc:
        manifest["errors"].append({"case": case_name, "stage": stage, "message": message(exc)})
        entry["error_stage"] = stage
        entry["error"] = message(exc)
        if stage in ("run", "export", "save"):
            if entry["status"] != "saved":
                try:
                    taskrunner.saveStreamToFile(stream, str_path)
                    manifest["saved"].append(file_name)
                    entry["status"] = "saved"
                except ERRORS:
                    pass
    finally:
        manifest["cases"].append(entry)
        if stream is not None:
            try:
                stream.clear()
            except ERRORS:
                pass


ensure_dir(DIR)
write_csv(CSV_MAIN, NROWS_MAIN)
write_csv(CSV_LARGE, NROWS_LARGE)

# 1. x/y only. First case records defaults, run, and export (including "ran but no export").
build_case("eplot_xy", [
    ("x_field", "amount"),
    ("y_field", "score"),
], record_defaults=True)

# 2-4. Each overlay field on its own. Control: eplot_xy.
build_case("eplot_color", [
    ("x_field", "amount"),
    ("y_field", "score"),
    ("color_field", "region"),
])
build_case("eplot_size", [
    ("x_field", "amount"),
    ("y_field", "score"),
    ("size_field", "weight"),
])
build_case("eplot_shape", [
    ("x_field", "amount"),
    ("y_field", "score"),
    ("shape_field", "channel"),
])

# 5. All three overlays together (Plot rejects this; Table 107 lists each as optional).
build_case("eplot_overlays", [
    ("x_field", "amount"),
    ("y_field", "score"),
    ("color_field", "region"),
    ("size_field", "weight"),
    ("shape_field", "channel"),
])

# 6. interested_fields. Tries a list, then a comma-separated string.
build_case("eplot_interested", [
    ("x_field", "amount"),
    ("y_field", "score"),
    ("interested_fields", ["segment", "region"]),
])

# 7. 80 rows, default records_limit (2000): under the limit, all records.
build_case("eplot_limit_under", [
    ("x_field", "amount"),
    ("y_field", "score"),
], CSV_LARGE)

# 8-9. records_limit below the record count, if_over_limit both ways.
build_case("eplot_limit_sample", [
    ("x_field", "amount"),
    ("y_field", "score"),
    ("records_limit", 10),
    ("if_over_limit", ["PlotSample", "Sample", True]),
], CSV_LARGE)
build_case("eplot_limit_all", [
    ("x_field", "amount"),
    ("y_field", "score"),
    ("records_limit", 10),
    ("if_over_limit", ["PlotAll", "All", False, "UseAll"]),
], CSV_LARGE)

# 10-13. String and date fields on the axes and as a colour overlay.
build_case("eplot_string_x", [
    ("x_field", "region"),
    ("y_field", "score"),
])
build_case("eplot_string_y", [
    ("x_field", "amount"),
    ("y_field", "channel"),
])
build_case("eplot_date_x", [
    ("x_field", "when"),
    ("y_field", "score"),
])
build_case("eplot_date_y", [
    ("x_field", "amount"),
    ("y_field", "when"),
])
build_case("eplot_date_color", [
    ("x_field", "amount"),
    ("y_field", "score"),
    ("color_field", "when"),
])

# 15. interested_fields mixing integer, date, and string.
build_case("eplot_interested_mixed", [
    ("x_field", "amount"),
    ("y_field", "score"),
    ("interested_fields", ["id", "when", "segment"]),
])

# 16. Every documented slot at once.
build_case("eplot_full", [
    ("x_field", "amount"),
    ("y_field", "score"),
    ("color_field", "region"),
    ("size_field", "weight"),
    ("shape_field", "channel"),
    ("interested_fields", ["id", "when", "segment"]),
])

handle = open(OUT_DIR + "/manifest.json", "w")
try:
    json.dump(manifest, handle, indent=2, sort_keys=True)
finally:
    handle.close()
