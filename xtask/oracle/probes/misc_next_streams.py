# Probe: SAS export/import, Transform, Auto Data Prep, and an imported PMML nugget.
# Coordinator only: cargo xtask probe misc_next_streams (Modeler 19, Jython 2.7).
# Writes inputs, streams, and exported SAS/PMML files to corpus/misc_next (label misc_next).
# Public modeler.api only. Every stream sets decimal_places = 15.
# A rejected setting or a batch-mode failure is stored in manifest["errors"]; later cases still run.
# Property names and values are from the Modeler 19.0 Python Scripting and Automation Guide.
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/misc_next"
CSV_NAME = "misc_next_main.csv"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)
# Modeler 19 batch fails these cases (2026-10-18): SAS export/import report "output/input
# encoder was not open" (-11/-12) and write empty files; openModelFromFile cannot read a
# PMML XML file (EOFException) and the guide documents no PMML import. Set True to retry.
BLOCKED_CASES = False

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "misc_next", "saved": [], "errors": [], "cases": [], "inputs": [],
            "notes": [
                "Export formats are Table 253: Windows, UNIX, SAS7, SAS8. Transport and SAS9 are import-only (Table 54).",
                "import_names is NamesAndLabels or LabelsasNames in Table 54; the sasimport example sets True. Both are separate cases.",
                "The guide's only SAS filename is the SAS8 example .sas7bdat. Windows and UNIX paths use .sd2 and .ssd.",
                "fixed_date is typed as date with no literal. adp_dates passes 2020-03-31 with stream date_format YYYY-MM-DD.",
                "objective Custom is set on the rescale and date cases so those Table 79 flags are the settings under test.",
                "transform output_format HTML follows the short token. Table 242 prints HTML (.html) and Output (.cou).",
                "Table 79 lists fixed_date_units twice; the Hours/Minutes/Seconds column is not set.",
            ]}

# 24 rows. id/iv integers (negative, zero, null); rv/pos reals (fraction, zero, negative, null);
# txt strings (spaces, empty, whitespace, non-ASCII letters); dt dates (leap day, null);
# ts timestamps (null). y is A when iv is negative or null, otherwise B.
CAFE = u"caf\u00e9 latte"
NAIVE = u"na\u00efve"
ALAND = u"\u00c5land"
ELEVE = u"\u00e9l\u00e8ve"
SCHEMA = [
    ("id", "Integer", "Range", "None", None, "Row id"),
    ("grp", "String", "Set", "Input", None, "Group label"),
    ("iv", "Integer", "Range", "Input", None, "Integer value"),
    ("rv", "Real", "Range", "Input", None, "Real value"),
    ("pos", "Real", "Range", "Input", None, "Positive real"),
    ("txt", "String", "Typeless", "None", None, "Text label"),
    ("flg", "String", "Flag", "Input", ["F", "T"], "Flag value"),
    ("dt", "Date", "Range", "Input", None, "When date"),
    ("ts", "Timestamp", "Range", "Input", None, "When time"),
    ("y", "String", "Set", "Target", ["A", "B"], "Class label"),
]
ROWS = [
    [1, "A", -4, -1.5, 0.125, "banana", "T", "2020-01-15", "2020-01-15 08:00:00", "A"],
    [2, "A", -2, -0.5, 0.25, "Apple", "F", "2020-01-16", "2020-01-16 09:30:00", "A"],
    [3, "B", -1, 0.0, 0.5, "apple", "T", "2020-02-29", "2020-02-29 00:00:00", "A"],
    [4, "B", 0, 0.0, 1.0, "", "F", "2020-03-01", "2020-03-01 12:00:00", "B"],
    [5, "A", 0, 1.25, 1.5, None, "T", "2020-03-02", "2020-03-02 23:59:59", "B"],
    [6, "C", 1, 2.5, 2.0, u"  ", "F", "2020-03-03", None, "B"],
    [7, "B", 2, 0.123456789012345, 2.5, u"red blue", "T", "2020-03-04", "2020-03-04 00:00:01", "B"],
    [8, "A", 3, -2.25, 3.0, CAFE, "F", "2020-03-05", "2020-03-05 13:45:00", "B"],
    [9, "C", 4, 4.5, 4.0, NAIVE, "T", None, None, "B"],
    [10, "A", 5, 8.0, 5.5, ALAND, "F", "1999-12-31", "1999-12-31 00:00:00", "B"],
    [11, "B", 6, None, 6.0, ELEVE, "T", "2020-06-15", "2020-06-15 18:30:15", "B"],
    [12, "C", 8, 10.0, 7.25, u"has space", "F", "2020-07-04", "2020-07-04 00:00:00", "B"],
    [13, "A", -8, -3.5, 0.333333333333333, "banana", "T", "2020-08-08", "2020-08-08 08:08:08", "A"],
    [14, "B", -6, 1.5, 9.0, CAFE, "F", "2020-09-09", "2020-09-09 09:09:09", "A"],
    [15, "C", 10, 2.75, 10.0, "Apple", "T", "2020-10-10", "2020-10-10 10:10:10", "B"],
    [16, "A", None, 0.5, 11.0, u"  spaced  ", "F", "2020-11-11", "2020-11-11 11:11:11", "A"],
    [17, "B", 7, 6.5, 12.5, "omega", "T", "2020-12-01", "2020-12-01 01:02:03", "B"],
    [18, "C", 9, -0.25, 0.0625, "end", "F", "2021-01-01", "2021-01-01 00:00:00", "B"],
    [19, "A", -3, 3.333333333333333, 1.0, NAIVE, "T", "2020-02-28", "2020-02-28 23:59:58", "A"],
    [20, "", 4, 7.0, 2.0, u"last one", None, "2020-03-18", "2020-03-18 16:00:00", "B"],
    [21, "A", 1, 1.0, 3.0, CAFE, "T", "2020-04-01", "2020-04-01 00:00:01", "B"],
    [22, "B", 2, 2.0, 4.0, "banana", "F", "2020-04-02", "2020-04-02 00:00:00", "B"],
    [23, "C", -5, 5.0, 8.0, "Apple", "T", "2020-04-03", "2020-04-03 00:00:00", "A"],
    [24, "A", 12, 0.1, 0.2, ELEVE, "F", "2020-04-04", "2020-04-04 00:00:00", "B"],
]


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


def write_manifest():
    handle = open(OUT_DIR + "/manifest.json", "w")
    try:
        json.dump(manifest, handle, indent=2, sort_keys=True)
    finally:
        handle.close()


def remember(name):
    if name not in manifest["saved"]:
        manifest["saved"].append(name)


def file_length(path):
    handle = java.io.File(path)
    if not handle.isFile():
        return None
    return int(handle.length())


def ensure_clean_dir(path):
    # Non-recursive, and only this probe's label directory.
    if path != DIR:
        raise ValueError("unexpected corpus directory")
    directory = java.io.File(path)
    if directory.exists():
        if not directory.isDirectory():
            raise IOError("not a directory: " + path)
        children = directory.listFiles()
        if children is None:
            raise IOError("cannot list " + path)
        for child in children:
            if child.isFile() and not child.delete():
                raise IOError("cannot remove old probe artifact " + unicode(child))
    elif not directory.mkdirs():
        raise IOError("cannot create " + path)


def csv_cell(value):
    if value is None:
        return ""
    if isinstance(value, basestring):
        return '"' + value.replace('"', '""') + '"'
    if isinstance(value, float):
        return "%.17g" % value
    return str(value)


def write_csv(name, schema, rows):
    if len(rows) < 10 or len(rows) > 60:
        raise ValueError("CSV row count out of range: " + name)
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
    remember(name)
    manifest["inputs"].append({"file": name, "rows": len(rows)})


def java_name(obj):
    try:
        return unicode(obj.getClass().getName())
    except ERRORS:
        return unicode(type(obj))


def node_types(stream):
    found = []
    iterator = stream.iterator()
    while iterator.hasNext():
        found.append(unicode(iterator.next().getTypeName()))
    return found


def put(node, entry, name, value):
    entry["setting_in_progress"] = [name, snapshot(value)]
    node.setPropertyValue(name, value)
    entry["properties"][name] = snapshot(value)
    entry.pop("setting_in_progress", None)


def keyed(node, entry, prop, key, value):
    entry.setdefault("keyed_properties", []).append([prop, key, snapshot(value)])
    node.setKeyedPropertyValue(prop, key, value)


def readback(node, names):
    values = {}
    for name in names:
        try:
            values[name] = snapshot(node.getPropertyValue(name))
        except ERRORS as exc:
            values[name] = {"read_error": message(exc)}
    return values


def instantiate_node(stream, node):
    table = stream.createAt("table", "TempTable", 1000, 700)
    stream.link(node, table)
    try:
        table.run([])
    finally:
        try:
            stream.delete(table)
        except ERRORS:
            pass


def pmml_data_fields(output):
    try:
        content = output.getContentModel("PMML")
    except ERRORS as exc:
        return {"read_error": message(exc)}
    if content is None:
        return None
    try:
        names = content.getStringValues("/PMML/DataDictionary/DataField", "name")
        return [unicode(name) for name in names]
    except ERRORS as exc:
        return {"read_error": message(exc)}


def describe_results(results):
    described = []
    for result in results:
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
            item["model_output"] = isinstance(result, modeler.api.ModelOutput)
            item["document_output"] = isinstance(result, modeler.api.DocumentOutput)
        except ERRORS as exc:
            item["type_check_error"] = message(exc)
        fields = pmml_data_fields(result)
        if fields is not None:
            item["pmml_data_fields"] = fields
        described.append(item)
    return described


def open_stream(case_name, box, entry):
    entry["stage"] = "create_stream"
    stream = taskrunner.createStream(case_name, False, False)
    box["stream"] = stream
    stream_props = [("decimal_places", 15), ("decimal_symbol", "Period"),
                    ("encoding", "UTF-8"), ("date_format", "YYYY-MM-DD"),
                    ("time_format", "HH:MM:SS")]
    for name, value in stream_props:
        stream.setPropertyValue(name, value)
    entry["stream_properties"] = [[name, snapshot(value)] for name, value in stream_props]
    return stream


def add_typed_source(stream, entry):
    # Variable File -> Type. Storage is set before the first instantiate (Table 43 / Table 60).
    entry["stage"] = "source"
    source = stream.createAt("variablefile", CSV_NAME, 80, 100)
    source_props = [("full_filename", DIR + "/" + CSV_NAME), ("read_field_names", True),
                    ("delimit_comma", True), ("delimit_space", False), ("delimit_tab", False),
                    ("quotes_2", "PairAndDiscard"), ("strip_spaces", "None"),
                    ("decimal_symbol", "Period"), ("encoding", "UTF-8"),
                    ("auto_recognize_datetime", False), ("lines_to_scan", 1000)]
    entry["source_properties"] = [[name, snapshot(value)] for name, value in source_props]
    for name, value in source_props:
        source.setPropertyValue(name, value)
    entry["storage"] = []
    for field, storage, measure, direction, values, label in SCHEMA:
        source.setKeyedPropertyValue("custom_storage_type", field, storage)
        if storage in ("Date", "Timestamp"):
            source.setKeyedPropertyValue("custom_date_format", field, "YYYY-MM-DD")
        if storage == "Timestamp":
            source.setKeyedPropertyValue("custom_time_format", field, "HH:MM:SS")
        entry["storage"].append([field, storage])
    type_node = stream.createAt("type", "Type", 280, 100)
    stream.link(source, type_node)
    instantiate_node(stream, type_node)
    for field, storage, measure, direction, values, label in SCHEMA:
        keyed(type_node, entry, "type", field, measure)
        keyed(type_node, entry, "direction", field, direction)
        keyed(type_node, entry, "check", field, "None")
        keyed(type_node, entry, "description", field, label)
        if values is not None:
            keyed(type_node, entry, "values", field, values)
            keyed(type_node, entry, "extend_values", field, False)
    instantiate_node(stream, type_node)
    return type_node


def save_stream(stream, entry):
    entry["stage"] = "save"
    taskrunner.saveStreamToFile(stream, DIR + "/" + entry["file"])
    remember(entry["file"])
    if entry["status"] == "planned":
        entry["status"] = "saved"


def run_node(node, entry, stage):
    entry["stage"] = stage
    results = []
    try:
        node.run(results)
    except ERRORS as exc:
        manifest["errors"].append({"case": entry["case"], "stage": stage, "message": message(exc)})
        entry["status"] = "saved_with_run_error"
        entry[stage + "_error"] = message(exc)
        return []
    try:
        entry[stage + "_results"] = describe_results(results)
    except ERRORS as exc:
        entry[stage + "_results_error"] = message(exc)
    return results


def run_case(case_name, build):
    box = {"stream": None}
    entry = {"case": case_name, "file": case_name + ".str", "status": "planned",
             "properties": {}, "csv": CSV_NAME, "stage": "start"}
    manifest["cases"].append(entry)
    try:
        build(entry, box)
        if entry["status"] == "planned":
            raise RuntimeError("case returned without saving")
    except ERRORS as exc:
        entry["status"] = "failed"
        manifest["errors"].append({"case": case_name, "stage": entry.get("stage", "build"),
                                   "message": message(exc)})
    finally:
        stream = box["stream"]
        if stream is not None:
            try:
                stream.clear()
            except ERRORS as exc:
                manifest["errors"].append({"case": case_name, "stage": "clear",
                                           "message": message(exc)})
        try:
            write_manifest()
        except ERRORS as exc:
            manifest["errors"].append({"case": case_name, "stage": "manifest",
                                       "message": message(exc)})


def build_sas_export(fmt, export_names, generate_import, ext):
    def build(entry, box):
        stream = open_stream(entry["case"], box, entry)
        upstream = add_typed_source(stream, entry)
        entry["stage"] = "configure"
        # Table 253. full_filename is the file written beside the stream.
        node = stream.createAt("sasexport", entry["case"], 500, 100)
        stream.link(upstream, node)
        entry["type"] = unicode(node.getTypeName())
        data_file = entry["case"] + "." + ext
        data_path = DIR + "/" + data_file
        put(node, entry, "full_filename", data_path)
        put(node, entry, "format", fmt)
        put(node, entry, "export_names", export_names)
        put(node, entry, "generate_import", generate_import)
        entry["readback"] = readback(node, ["format", "export_names", "generate_import",
                                            "full_filename"])
        entry["data_file"] = data_file
        save_stream(stream, entry)
        run_node(node, entry, "export")
        entry["export_bytes"] = file_length(data_path)
        if entry["export_bytes"] is not None:
            remember(data_file)
        entry["nodes"] = node_types(stream)
    return build


def build_sas_import(data_file, props):
    def build(entry, box):
        # SAS File source -> Table. No Variable File node: the input is the exported file.
        stream = open_stream(entry["case"], box, entry)
        entry["stage"] = "configure"
        node = stream.createAt("sasimport", entry["case"], 100, 100)
        entry["type"] = unicode(node.getTypeName())
        data_path = DIR + "/" + data_file
        put(node, entry, "full_filename", data_path)
        for name, value in props:
            put(node, entry, name, value)
        entry["readback"] = readback(node, list(entry["properties"].keys()))
        entry["data_file"] = data_file
        table = stream.createAt("table", "Table", 400, 100)
        stream.link(node, table)
        entry["terminal"] = unicode(table.getTypeName())
        save_stream(stream, entry)
        run_node(table, entry, "import")
    return build


def build_transform(props):
    def build(entry, box):
        stream = open_stream(entry["case"], box, entry)
        upstream = add_typed_source(stream, entry)
        entry["stage"] = "configure"
        # Table 242. The Transform node is the terminal and produces an output object.
        node = stream.createAt("transform", entry["case"], 500, 100)
        stream.link(upstream, node)
        entry["type"] = unicode(node.getTypeName())
        for name, value in props:
            put(node, entry, name, value)
        entry["readback"] = readback(node, list(entry["properties"].keys()))
        entry["terminal"] = entry["type"]
        save_stream(stream, entry)
        run_node(node, entry, "transform")
        html_name = entry["case"] + ".html"
        if file_length(DIR + "/" + html_name) is not None:
            remember(html_name)
    return build


def place_model(stream, model, type_node, entry, label, y_pos):
    applier = stream.createModelApplierAt(model, label, 640, y_pos)
    # The type name is the feasibility result. Record it before linking can fail.
    entry["applier_type"] = unicode(applier.getTypeName())
    entry["applier_label"] = label
    stream.link(type_node, applier)
    table = stream.createAt("table", label, 860, y_pos)
    stream.link(applier, table)
    return table


def build_adp(props):
    def build(entry, box):
        stream = open_stream(entry["case"], box, entry)
        type_node = add_typed_source(stream, entry)
        entry["stage"] = "configure"
        # Table 79. Analysis is node.run (Table 28: autodataprep output container "PMML").
        node = stream.createAt("autodataprep", entry["case"], 500, 100)
        stream.link(type_node, node)
        entry["type"] = unicode(node.getTypeName())
        for name, value in props:
            put(node, entry, name, value)
        entry["readback"] = readback(node, list(entry["properties"].keys()))
        # A process node: batch run() reports "No executable nodes found"; the Table runs it.
        results = []
        prepared = None
        for result in results:
            if isinstance(result, modeler.api.ModelOutput):
                entry["stage"] = "place_applier"
                try:
                    prepared = place_model(stream, result, type_node, entry,
                                           "Prepared fields", 260)
                except ERRORS as exc:
                    manifest["errors"].append({"case": entry["case"], "stage": "place_applier",
                                               "message": message(exc)})
                    entry["place_applier_error"] = message(exc)
                break
        entry["stage"] = "link_table"
        direct = stream.createAt("table", "Table", 760, 100)
        can_link = False
        try:
            can_link = bool(stream.isValidLink(node, direct))
        except ERRORS as exc:
            manifest["errors"].append({"case": entry["case"], "stage": "link_table",
                                       "message": message(exc)})
            entry["link_check_error"] = message(exc)
        if can_link:
            try:
                stream.link(node, direct)
                entry["table_from"] = unicode(node.getTypeName())
            except ERRORS as exc:
                manifest["errors"].append({"case": entry["case"], "stage": "link_table",
                                           "message": message(exc)})
                entry["link_error"] = message(exc)
                can_link = False
        if not can_link:
            try:
                stream.delete(direct)
            except ERRORS:
                pass
            direct = None
            entry["table_from"] = None
        if direct is None and prepared is None:
            raise RuntimeError("ADP produced no Table path and no model applier")
        entry["nodes"] = node_types(stream)
        save_stream(stream, entry)
        if direct is not None:
            run_node(direct, entry, "table")
        if prepared is not None:
            run_node(prepared, entry, "prepared_table")
    return build


def build_pmml(entry, box):
    stream = open_stream(entry["case"], box, entry)
    # Table 42: keep the imported applier as the only nugget this script places.
    stream.setPropertyValue("create_model_applier_for_new_models", False)
    entry["stream_properties"].append(["create_model_applier_for_new_models", False])
    type_node = add_typed_source(stream, entry)
    entry["stage"] = "configure"
    # Table 119. Small C5.0, same training shape as models_ml_streams.py.
    builder = stream.createAt("c50", entry["case"], 500, 100)
    stream.link(type_node, builder)
    entry["type"] = unicode(builder.getTypeName())
    put(builder, entry, "use_model_name", True)
    put(builder, entry, "model_name", "misc_next")
    put(builder, entry, "output_type", "DecisionTree")
    put(builder, entry, "use_partitioned_data", False)
    entry["readback"] = readback(builder, list(entry["properties"].keys()))
    entry["stage"] = "train"
    results = []
    builder.run(results)
    models = [result for result in results if isinstance(result, modeler.api.ModelOutput)]
    try:
        entry["train_results"] = describe_results(results)
    except ERRORS as exc:
        entry["train_results_error"] = message(exc)
    if not models:
        raise RuntimeError("C5.0 returned no ModelOutput")
    entry["stage"] = "export_pmml"
    pmml_name = "pmml_c50.xml"
    pmml_path = DIR + "/" + pmml_name
    taskrunner.exportModelToFile(models[0], pmml_path, modeler.api.FileFormat.XML)
    entry["pmml_file"] = pmml_name
    entry["pmml_bytes"] = file_length(pmml_path)
    if entry["pmml_bytes"] is not None:
        remember(pmml_name)
    entry["stage"] = "open_pmml"
    opened = taskrunner.openModelFromFile(pmml_path, False)
    entry["opened_model_class"] = java_name(opened)
    entry["stage"] = "place_applier"
    try:
        table = place_model(stream, opened, type_node, entry, "Imported PMML", 280)
    finally:
        # Keep the type name even when the link or the Table node is rejected.
        if "applier_type" in entry:
            manifest["imported_pmml_applier_type"] = entry["applier_type"]
    entry["nodes"] = node_types(stream)
    save_stream(stream, entry)
    run_node(table, entry, "score")


def main():
    try:
        ensure_clean_dir(DIR)
        write_csv(CSV_NAME, SCHEMA, ROWS)
    except ERRORS as exc:
        manifest["errors"].append({"case": None, "stage": "setup", "message": message(exc)})
        return

    if BLOCKED_CASES:
        # --- SAS export: one case per Table 253 format and name/import option. ---
        run_case("sas_export_windows", build_sas_export("Windows", "NamesAndLabels", False, "sd2"))
        run_case("sas_export_unix", build_sas_export("UNIX", "NamesAndLabels", False, "ssd"))
        run_case("sas_export_sas7", build_sas_export("SAS7", "NamesAndLabels", False, "sas7bdat"))
        run_case("sas_export_sas8", build_sas_export("SAS8", "NamesAndLabels", False, "sas7bdat"))
        run_case("sas_export_names_as_labels", build_sas_export("SAS8", "NamesAsLabels", False, "sas7bdat"))
        run_case("sas_export_generate_import", build_sas_export("SAS8", "NamesAndLabels", True, "sas7bdat"))

        # --- SAS import: each exported file, then the other Table 54 options. ---
        run_case("sas_import_windows", build_sas_import(
            "sas_export_windows.sd2", [("format", "Windows"), ("read_formats", False)]))
        run_case("sas_import_unix", build_sas_import(
            "sas_export_unix.ssd", [("format", "UNIX"), ("read_formats", False)]))
        run_case("sas_import_sas7", build_sas_import(
            "sas_export_sas7.sas7bdat", [("format", "SAS7"), ("read_formats", False)]))
        run_case("sas_import_sas8", build_sas_import(
            "sas_export_sas8.sas7bdat", [("format", "SAS8"), ("read_formats", False),
                                        ("import_names", "NamesAndLabels")]))
        run_case("sas_import_names_as_labels", build_sas_import(
            "sas_export_names_as_labels.sas7bdat", [("format", "SAS8"), ("read_formats", False),
                                                   ("import_names", "NamesAndLabels")]))
        run_case("sas_import_generate_import", build_sas_import(
            "sas_export_generate_import.sas7bdat", [("format", "SAS8"), ("read_formats", False)]))
        run_case("sas_import_labels_as_names", build_sas_import(
            "sas_export_sas8.sas7bdat", [("format", "SAS8"), ("read_formats", False),
                                        ("import_names", "LabelsasNames")]))
        run_case("sas_import_names_flag", build_sas_import(
            "sas_export_sas8.sas7bdat", [("format", "SAS8"), ("read_formats", False),
                                        ("import_names", True)]))
        run_case("sas_import_sas9", build_sas_import(
            "sas_export_sas8.sas7bdat", [("format", "SAS9"), ("read_formats", False)]))
        run_case("sas_import_transport", build_sas_import(
            "sas_export_sas8.sas7bdat", [("format", "Transport"), ("member_name", "Test"),
                                        ("read_formats", False)]))
        run_case("sas_import_read_formats", build_sas_import(
            "sas_export_sas8.sas7bdat", [("format", "SAS8"), ("read_formats", True),
                                        ("full_format_filename", DIR + "/misc_next_formats.sas7bcat")]))

    # --- Transform output (Table 242): fields and each documented transformation. ---
    run_case("transform_all", build_transform([
        ("fields", ["pos", "rv"]), ("formula", "All"), ("output_mode", "Screen")]))
    run_case("transform_select", build_transform([
        ("fields", ["pos", "rv"]), ("formula", "Select"),
        ("formula_inverse", True), ("formula_inverse_offset", 1),
        ("formula_log_n", True), ("formula_log_n_offset", 1),
        ("formula_log_10", True), ("formula_log_10_offset", 1),
        ("formula_exponential", True), ("formula_square_root", True),
        ("output_mode", "Screen")]))
    run_case("transform_named", build_transform([
        ("fields", ["pos"]), ("formula", "All"), ("use_output_name", True),
        ("output_name", "Misc Next Transform"), ("output_mode", "Screen")]))
    run_case("transform_html", build_transform([
        ("fields", ["pos"]), ("formula", "Select"), ("formula_square_root", True),
        ("output_mode", "File"), ("output_format", "HTML"), ("paginate_output", True),
        ("lines_per_page", 20), ("full_filename", DIR + "/transform_html.html")]))

    # --- Auto Data Prep (Table 79): default objective, then settings that change fields. ---
    run_case("adp_balanced", build_adp([("objective", "Balanced")]))
    run_case("adp_rescale", build_adp([
        ("objective", "Custom"), ("rescale_continuous_inputs", True),
        ("rescaling_method", "ZScore"), ("z_score_final_mean", 0), ("z_score_final_sd", 1)]))
    run_case("adp_dates", build_adp([
        ("objective", "Custom"), ("prepare_dates_and_times", True),
        ("compute_time_until_date", True), ("reference_date", "Fixed"),
        ("fixed_date", "2020-03-31"), ("units_for_date_durations", "Fixed"),
        ("fixed_date_units", "Days"), ("extract_year_from_date", True),
        ("extract_month_from_date", True), ("extract_day_from_date", True)]))

    # --- Imported PMML: train C5.0, export XML, open it, place the applier, score. ---
    if BLOCKED_CASES:
        run_case("pmml_c50", build_pmml)


try:
    main()
finally:
    write_manifest()
