# Probe: build deterministic Export node corpus streams and input CSVs in corpus/exports/.
# Each stream is variablefile -> Type (instantiated) -> the export node as the terminal,
# with decimal_places = 15. The export node's output path is inside the corpus directory;
# `cargo xtask oracle` redirects that path into exports/.
# Run with `cargo xtask probe export_streams` (IBM SPSS Modeler, Jython 2.7, public API only).
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
EXPORTS_DIR = "D:/data-mining-software/corpus/exports"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)
TAB = chr(9)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"saved": [], "errors": [], "cases": []}

# 18 rows. Columns: id (int), grp (set, repeated, blanks), iv (int, ties, negative, null),
# rv (real, negative, long fraction, null), s (text: embedded quote, comma, semicolon, tab,
# empty string, blank, spaces), flg (T/F/blank), dt (ISO date, leap day, blank).
# No field value contains a newline.
EXPORT_MAIN_CSV = [
    "id,grp,iv,rv,s,flg,dt",
    "1,A,10,-3.5,banana,T,2020-01-15",
    "2,A,10,0.0,Apple,F,2020-01-16",
    '3,B,20,1.25,"say ""hi""",T,2020-02-29',
    "4,B,-10,2.5,a;b,F,2020-03-01",
    '5,A,10,-1.5,"has,comma",T,1999-12-31',
    '6,C,0,10.0,"",F,2020-06-15',
    "7,C,7,-2.25,zebra,T,",
    "8,,5,0.5,apple,,2020-07-04",
    "9,B,40,4.5,\"tab" + TAB + "here\",T,2020-08-08",
    "10,A,30,-0.25,banana,F,2020-09-09",
    "11,C,,,,T,2020-10-10",
    "12,B,80,9.0,cherry,F,2020-11-11",
    '13,A,15,3.5,"q ""and"" semi;colon,comma",T,2020-12-01',
    "14,D,70,,APPLE,F,2021-01-01",
    "15,B,,18.5,,T,2021-02-02",
    '16,A,10,-3.5,"  spaced  ",F,2020-01-15',
    "17,,5,-2.25,zebra,F,",
    "18,D,70,0.123456789012345,APPLE,T,2021-01-01",
]


def message(exc):
    get = getattr(exc, "getMessage", None)
    if get is not None:
        return unicode(get())
    return unicode(exc)


def ensure_clean_dir(path):
    d = java.io.File(path)
    if d.exists():
        children = d.listFiles()
        if children is not None:
            for child in children:
                if child.isFile():
                    child.delete()
    else:
        d.mkdirs()


def write_csv(file_name, lines):
    path = "%s/%s" % (EXPORTS_DIR, file_name)
    handle = open(path, "w")
    try:
        handle.write(NL.join(lines) + NL)
    finally:
        handle.close()
    manifest["saved"].append(file_name)


def instantiate_node(stream, node):
    table = stream.createAt("table", "TempTable", 700, 300)
    stream.link(node, table)
    try:
        table.run([])
    finally:
        stream.delete(table)


def new_stream(case_name, csv_name):
    stream = taskrunner.createStream(case_name, False, False)
    stream.setPropertyValue("decimal_places", 15)
    src = stream.createAt("variablefile", csv_name, 100, 100)
    src.setPropertyValue("full_filename", "%s/%s" % (EXPORTS_DIR, csv_name))
    src.setPropertyValue("read_field_names", True)
    # Type.storage is read-only. Date storage is set on the source (Table 43 / Table 60).
    src.setKeyedPropertyValue("custom_storage_type", "dt", "Date")
    src.setKeyedPropertyValue("custom_date_format", "dt", "YYYY-MM-DD")
    type_node = stream.createAt("type", "Type", 250, 100)
    stream.link(src, type_node)
    instantiate_node(stream, type_node)
    type_node.setKeyedPropertyValue("direction", "id", "None")
    type_node.setKeyedPropertyValue("type", "id", "Range")
    type_node.setKeyedPropertyValue("direction", "grp", "Input")
    type_node.setKeyedPropertyValue("type", "grp", "Set")
    type_node.setKeyedPropertyValue("description", "grp", "Group")
    type_node.setKeyedPropertyValue("direction", "iv", "Input")
    type_node.setKeyedPropertyValue("type", "iv", "Range")
    type_node.setKeyedPropertyValue("direction", "rv", "Input")
    type_node.setKeyedPropertyValue("type", "rv", "Range")
    type_node.setKeyedPropertyValue("description", "rv", "Real value")
    type_node.setKeyedPropertyValue("direction", "s", "Input")
    type_node.setKeyedPropertyValue("type", "s", "Typeless")
    type_node.setKeyedPropertyValue("description", "s", "Text")
    type_node.setKeyedPropertyValue("direction", "flg", "Input")
    type_node.setKeyedPropertyValue("type", "flg", "Flag")
    type_node.setKeyedPropertyValue("direction", "dt", "Input")
    type_node.setKeyedPropertyValue("date_format", "dt", "YYYY-MM-DD")
    type_node.setKeyedPropertyValue("description", "dt", "When")
    instantiate_node(stream, type_node)
    return stream, type_node


def put(node, props, name, value):
    node.setPropertyValue(name, value)
    props[name] = value


def finish_case(case_name, stream, export_node, props, csv_name):
    export_node.run([])
    file_name = "%s.str" % case_name
    path = "%s/%s" % (EXPORTS_DIR, file_name)
    taskrunner.saveStreamToFile(stream, path)
    manifest["saved"].append(file_name)
    out_name = props["full_filename"].replace("\\", "/").split("/")[-1]
    manifest["saved"].append(out_name)
    manifest["cases"].append({
        "case": case_name,
        "file": file_name,
        "input": csv_name,
        "type": unicode(export_node.getTypeName()),
        "properties": props,
    })


def build_case(case_name, csv_name, node_type, ext, configure):
    stream = None
    try:
        stream, upstream = new_stream(case_name, csv_name)
        export_node = stream.createAt(node_type, case_name, 450, 100)
        stream.link(upstream, export_node)
        props = {}
        put(export_node, props, "full_filename", "%s/%s.%s" % (EXPORTS_DIR, case_name, ext))
        configure(stream, export_node, upstream, props)
        put(export_node, props, "generate_import", False)
        finish_case(case_name, stream, export_node, props, csv_name)
    except ERRORS as e:
        manifest["errors"].append({"case": case_name, "message": message(e)})
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS:
                pass


def flat_common(node, props):
    put(node, props, "encoding", "UTF-8")
    put(node, props, "use_newline_after_records", True)


ensure_clean_dir(EXPORTS_DIR)
write_csv("export_main.csv", EXPORT_MAIN_CSV)


# --- 1. Flat File (outputfile) ---
# quote_mode in Table 252 is None, Single, Double, or Other. There is no "All".

def cfg_flat_default(stream, node, upstream, props):
    flat_common(node, props)


def cfg_flat_tab(stream, node, upstream, props):
    flat_common(node, props)
    put(node, props, "delimit_mode", "Tab")


def cfg_flat_semicolon(stream, node, upstream, props):
    flat_common(node, props)
    put(node, props, "delimit_mode", "Other")
    put(node, props, "other_delimiter", ";")


def cfg_flat_no_field_names(stream, node, upstream, props):
    flat_common(node, props)
    put(node, props, "inc_field_names", False)


def cfg_flat_quote_double(stream, node, upstream, props):
    flat_common(node, props)
    put(node, props, "quote_mode", "Double")


def cfg_flat_quote_none(stream, node, upstream, props):
    flat_common(node, props)
    put(node, props, "quote_mode", "None")


def cfg_flat_quote_single(stream, node, upstream, props):
    flat_common(node, props)
    put(node, props, "quote_mode", "Single")


def cfg_flat_decimal_comma(stream, node, upstream, props):
    flat_common(node, props)
    put(node, props, "decimal_symbol", "Comma")


def cfg_flat_overwrite(stream, node, upstream, props):
    flat_common(node, props)
    put(node, props, "write_mode", "Overwrite")


build_case("flat_default", "export_main.csv", "outputfile", "txt", cfg_flat_default)
build_case("flat_tab", "export_main.csv", "outputfile", "txt", cfg_flat_tab)
build_case("flat_semicolon", "export_main.csv", "outputfile", "txt", cfg_flat_semicolon)
build_case("flat_no_field_names", "export_main.csv", "outputfile", "txt", cfg_flat_no_field_names)
build_case("flat_quote_double", "export_main.csv", "outputfile", "txt", cfg_flat_quote_double)
build_case("flat_quote_none", "export_main.csv", "outputfile", "txt", cfg_flat_quote_none)
build_case("flat_quote_single", "export_main.csv", "outputfile", "txt", cfg_flat_quote_single)
build_case("flat_decimal_comma", "export_main.csv", "outputfile", "txt", cfg_flat_decimal_comma)
build_case("flat_overwrite", "export_main.csv", "outputfile", "txt", cfg_flat_overwrite)


# --- 2. Excel (excelexport) ---

def cfg_excel_field_names(stream, node, upstream, props):
    put(node, props, "excel_file_type", "Excel2007")
    put(node, props, "export_mode", "Create")
    put(node, props, "inc_field_names", True)
    put(node, props, "launch_application", False)


def cfg_excel_no_field_names(stream, node, upstream, props):
    put(node, props, "excel_file_type", "Excel2007")
    put(node, props, "export_mode", "Create")
    put(node, props, "inc_field_names", False)
    put(node, props, "launch_application", False)


build_case("excel_field_names", "export_main.csv", "excelexport", "xlsx", cfg_excel_field_names)
build_case("excel_no_field_names", "export_main.csv", "excelexport", "xlsx", cfg_excel_no_field_names)


# --- 3. Statistics (statisticsexport) ---

def cfg_stat_default(stream, node, upstream, props):
    put(node, props, "file_type", "sav")
    put(node, props, "launch_application", False)


def cfg_stat_names_as_labels(stream, node, upstream, props):
    put(node, props, "file_type", "sav")
    put(node, props, "launch_application", False)
    put(node, props, "export_names", "NamesAsLabels")


build_case("stat_default", "export_main.csv", "statisticsexport", "sav", cfg_stat_default)
build_case("stat_names_as_labels", "export_main.csv", "statisticsexport", "sav", cfg_stat_names_as_labels)


# --- 4. JSON (jsonexport) ---

def cfg_json_records(stream, node, upstream, props):
    put(node, props, "string_format", "records")


def cfg_json_values(stream, node, upstream, props):
    put(node, props, "string_format", "values")


build_case("json_records", "export_main.csv", "jsonexport", "json", cfg_json_records)
build_case("json_values", "export_main.csv", "jsonexport", "json", cfg_json_values)


# --- 5. XML (xmlexport) ---

def cfg_xml_default(stream, node, upstream, props):
    pass


build_case("xml_default", "export_main.csv", "xmlexport", "xml", cfg_xml_default)


handle = open(OUT_DIR + "/manifest.json", "w")
try:
    json.dump(manifest, handle, indent=2, sort_keys=True)
finally:
    handle.close()
