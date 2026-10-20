# Probe: deterministic source-node streams and inputs in corpus/sources/.
# Run by the coordinator with `cargo xtask probe source_streams` (Jython 2.7).
# Public Modeler scripting API only; the XLSX fixtures use Python's standard library.
# See probe-plans/source_streams.md for coverage, references, and unresolved questions.
import json
import zipfile
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
SOURCES_DIR = "D:/data-mining-software/corpus/sources"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
taskrunner = modeler.script.session().getTaskRunner()
manifest = {
    "label": "sources", "saved": [], "errors": [], "expected_errors": [],
    "observation_errors": [], "cases": [], "inputs": [],
    "references": [
        "IBM SPSS Modeler 19.0 Python Scripting and Automation Guide: "
        "Tables 25-26, 42, 49, 51, 53, 59, 61",
        "IBM SPSS Modeler User Guide (local ug.txt; source options are in the "
        "separate public Source, Process, and Output Nodes help)",
    ],
}


def message(exc):
    get = getattr(exc, "getMessage", None)
    if get is not None:
        return unicode(get())
    return unicode(exc)


def ensure_clean_dir(path):
    # Same flat-file cleanup as field_streams.py. Never recursively delete a tree.
    directory = java.io.File(path)
    if directory.exists():
        children = directory.listFiles()
        if children is None:
            raise IOError("Cannot list probe directory: " + path)
        for child in children:
            if child.isFile() and not child.delete():
                raise IOError("Cannot remove old probe file: " + unicode(child))
    elif not directory.mkdirs():
        raise IOError("Cannot create probe directory: " + path)


def snapshot(value):
    if value is None or isinstance(value, (bool, int, long, float, basestring)):
        return value
    if isinstance(value, (list, tuple, java.util.List)):
        return [snapshot(item) for item in value]
    return unicode(value)


def write_bytes(name, data, description):
    handle = open(SOURCES_DIR + "/" + name, "wb")
    try:
        handle.write(data)
    finally:
        handle.close()
    manifest["saved"].append(name)
    manifest["inputs"].append({"file": name, "bytes": len(data), "tests": description})


def write_text(name, text, description):
    write_bytes(name, unicode(text).encode("utf-8"), description)


def read_properties(node, names):
    values = {}
    for name in names:
        try:
            values[name] = snapshot(node.getPropertyValue(name))
        except ERRORS as exc:
            values[name] = {"read_error": message(exc)}
    return values


PROPERTY_NAMES = {
    "fixedfile": ["record_len", "line_oriented", "decimal_symbol", "skip_header",
                  "auto_recognize_datetime", "lines_to_scan", "fields", "strip_spaces",
                  "invalid_char_mode", "invalid_char_replacement", "encoding"],
    "userinput": ["names", "data_mode"],
    "jsonimport": ["string_format", "auto_label"],
    "xmlimport": ["read", "recurse", "directory_name", "full_schema_filename",
                  "records", "mode", "fields"],
    "excelimport": ["excel_file_type", "use_named_range", "named_range",
                    "worksheet_mode", "worksheet_index", "worksheet_name",
                    "data_range_mode", "blank_rows", "explicit_range_start",
                    "explicit_range_end", "read_field_names", "lines_to_scan"],
}
STREAM_PROPERTY_NAMES = ["decimal_places", "decimal_symbol", "encoding", "date_format",
                         "time_format", "date_baseline", "date_2digit_baseline",
                         "import_datetime_as_string"]


def set_props(node, items, entry):
    # Preserve request and readback separately, including property coercion or rejection.
    for item in items:
        entry["setting_in_progress"] = snapshot(item)
        if len(item) == 3:
            name, key, value = item
            node.setKeyedPropertyValue(name, key, value)
            actual = node.getKeyedPropertyValue(name, key)
            entry["readback"].append({"property": name, "key": key,
                                      "value": snapshot(actual)})
        else:
            name, value = item
            node.setPropertyValue(name, value)
            actual = node.getPropertyValue(name)
            entry["readback"].append({"property": name, "value": snapshot(actual)})
    entry.pop("setting_in_progress", None)


def table_observation(results):
    # Public Table Content Model (Scripting Guide Tables 25-26). CSV alone loses storage
    # and can conflate null with "". Keep raw cell text/None and column storage as evidence.
    if not results:
        raise ValueError("Table.run returned no output")
    content = results[0].getContentModel("table")
    row_count = content.getRowCount()
    col_count = content.getColumnCount()
    if row_count > 300 or col_count > 64:
        raise ValueError("Unexpected probe table size: %s x %s" % (row_count, col_count))
    columns = []
    for col in range(col_count):
        columns.append({"name": unicode(content.getColumnName(col)),
                        "storage": unicode(content.getStorageType(col))})
    rows = []
    for row in range(row_count):
        cells = []
        for col in range(col_count):
            value = content.getValueAt(row, col)
            cells.append(None if value is None else unicode(value))
        rows.append(cells)
    return {"row_count": row_count, "columns": columns, "cell_text": rows}


def instantiate_node(table):
    # The final Table also instantiates the source. No Type node changes its inference.
    results = []
    table.run(results)
    return results


def build_case(case_name, catalog_id, node_type, inputs, props, tests,
               expectation="success", stream_props=None):
    stream = None
    entry = {"case": case_name, "file": case_name + ".str", "catalog_id": catalog_id,
             "node_type": node_type, "inputs": inputs, "properties": snapshot(props),
             "stream_properties": [["decimal_places", 15]], "readback": [],
             "tests": tests, "expectation": expectation, "saved": False}
    manifest["cases"].append(entry)
    stage = "create"
    try:
        stream = taskrunner.createStream(case_name, False, False)
        entry["stream_defaults"] = read_properties(stream, STREAM_PROPERTY_NAMES)
        stream.setPropertyValue("decimal_places", 15)
        for name, value in (stream_props or []):
            stream.setPropertyValue(name, value)
            entry["stream_properties"].append([name, snapshot(value)])
        entry["stream_effective"] = read_properties(stream, STREAM_PROPERTY_NAMES)
        node = stream.createAt(node_type, case_name, 100, 100)
        entry["type"] = unicode(node.getTypeName())
        entry["defaults"] = read_properties(node, PROPERTY_NAMES[node_type])
        stage = "configure"
        # XML's field chooser needs the source structure first. Defer selection
        # until after getOutputDataModel(), keeping the uninstantiated stream too.
        xml_selection = []
        initial_props = props
        if node_type == "xmlimport":
            xml_selection = [item for item in props if item[0] in ("mode", "fields")]
            initial_props = [item for item in props if item[0] not in ("mode", "fields")]
        set_props(node, initial_props, entry)
        entry["effective"] = read_properties(node, PROPERTY_NAMES[node_type])
        table = stream.createAt("table", "Table", 380, 100)
        stream.link(node, table)
        # Save BEFORE execution too: empty/malformed/unsupported input must remain a
        # reviewable stream for the coordinator, even if Table.run rejects that input.
        stage = "save_uninstantiated"
        path = SOURCES_DIR + "/" + entry["file"]
        taskrunner.saveStreamToFile(stream, path)
        manifest["saved"].append(entry["file"])
        entry["saved"] = True
        if node_type == "xmlimport":
            stage = "read_xml_structure"
            data_model = node.getOutputDataModel()
            entry["xml_structure_fields"] = (None if data_model is None else
                                               [unicode(name) for name in data_model.nameIterator()])
            stage = "configure"
            set_props(node, xml_selection, entry)
            entry["effective"] = read_properties(node, PROPERTY_NAMES[node_type])
            stage = "save_uninstantiated"
            taskrunner.saveStreamToFile(stream, path)
        stage = "run_table"
        results = instantiate_node(table)
        entry["status"] = "ran" if expectation != "rejection" else "unexpected_acceptance"
        try:
            entry["table"] = table_observation(results)
        except ERRORS as exc:
            manifest["observation_errors"].append({"case": case_name,
                                                     "message": message(exc)})
        stage = "save_instantiated"
        taskrunner.saveStreamToFile(stream, path)
    except ERRORS as exc:
        error = {"case": case_name, "stage": stage, "message": message(exc)}
        entry["status"] = "error"
        if stage in ("read_xml_structure", "run_table") and expectation in ("rejection", "observe"):
            entry["status"] = "rejected"
            manifest["expected_errors"].append(error)
        else:
            manifest["errors"].append(error)
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS as exc:
                manifest["errors"].append({"case": case_name, "stage": "clear",
                                           "message": message(exc)})


def filename(name):
    return ("full_filename", SOURCES_DIR + "/" + name)


# --- Fixed File: [name, 1-based start, LENGTH], not inclusive endpoints. ---
# Run 1 persisted the third item as fixedFileColumnSpecs/length; the guide's
# example numbers are misleading. Every slice must fit inside record_len=82.
FIXED_FIELDS = [["id", 1, 3], ["code", 4, 6], ["qty", 10, 6], ["rv", 16, 18],
                ["note", 34, 12], ["dt", 46, 10], ["tm", 56, 8], ["stamp", 64, 19]]
FIXED_WIDTHS = [3, 6, 6, 18, 12, 10, 8, 19]
FIXED_ROWS = [
    ["3", "001", "-10", "-3.5", "  apple  ", "2020-02-29", "01:02:03", "2020-02-29 01:02:03"],
    ["1", "002", "0", "0", "Banana", "1999-12-31", "23:59:59", "1999-12-31 23:59:59"],
    ["2", "001", "10", "0.123456789012345", "", "2021-01-01", "00:00:00", "2021-01-01 00:00:00"],
    ["3", "001", "-10", "-3.5", "  apple  ", "2020-02-29", "01:02:03", "2020-02-29 01:02:03"],
    ["5", "010", "", "", "left  ", "", "", ""],
    ["6", "020", "5", "2.5", "  right", "2020-01-15", "12:30:00", "2020-01-15 12:30:00"],
    ["7", "030", "8", "-0.25", "say \"hi\",x", "2020-03-01", "06:07:08", "2020-03-01 06:07:08"],
    ["8", "040", "late", "9", "NULL", "2020-03-02", "06:07:09", "2020-03-02 06:07:09"],
]


def fixed_line(values):
    if len(values) != len(FIXED_WIDTHS):
        raise ValueError("Bad fixed fixture column count")
    parts = []
    for value, width in zip(values, FIXED_WIDTHS):
        value = unicode(value)
        if len(value) > width:
            raise ValueError("Fixed fixture cell too long: " + value)
        parts.append(value.ljust(width))
    return u"".join(parts)


def fixed_props(name, extra=None):
    return [filename(name), ("record_len", 82), ("fields", FIXED_FIELDS)] + (extra or [])


# --- Minimal deterministic OOXML input writer (no Excel, POI, or third-party packages). ---
def xml_escape(text):
    return unicode(text).replace("&", "&amp;").replace("<", "&lt;").replace(
        ">", "&gt;").replace('"', "&quot;")


def cell(kind, value, style=0):
    return (kind, value, style)


# Modeler 19's Excel reader needs two things Excel always writes (found 2026-10-20 by swapping
# parts with Demos/bank_customer_data.xlsx): an XML declaration on xl/_rels/workbook.xml.rels
# (without it every read fails with the misleading E3341 "Insufficient memory for JVM") and a
# <dimension> on each worksheet (without it "The data source defines no fields"). Every part
# gets the declaration; the dimension spans the cells present.
XML_DECLARATION = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'


def sheet_dimension(rows):
    cells = [(row_num, column) for row_num, row in enumerate(rows, 1)
             for column, value in enumerate(row) if value is not None]
    if not cells:
        return u"A1"
    first = (min(r for r, _ in cells), min(c for _, c in cells))
    last = (max(r for r, _ in cells), max(c for _, c in cells))
    return u"%s%d:%s%d" % (chr(65 + first[1]), first[0], chr(65 + last[1]), last[0])


def sheet_xml(rows):
    parts = [u'<?xml version="1.0" encoding="UTF-8"?>',
             u'<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">',
             u'<dimension ref="%s"/><sheetData>' % sheet_dimension(rows)]
    for row_num, row in enumerate(rows, 1):
        parts.append(u'<row r="%d">' % row_num)
        for column, value in enumerate(row):
            if value is None:
                continue  # absent cell, distinct from an explicit inline empty string
            if column >= 26:
                raise ValueError("Fixture writer only supports columns A-Z")
            address = "%s%d" % (chr(65 + column), row_num)
            if not isinstance(value, tuple):
                value = cell("s", value)
            kind, payload, style = value
            prefix = u'<c r="%s" s="%d"' % (address, style)
            if kind == "s":
                parts.append(prefix + u' t="inlineStr"><is><t xml:space="preserve">' +
                             xml_escape(payload) + u'</t></is></c>')
            elif kind == "f":
                formula, cached = payload
                parts.append(prefix + u'><f>' + xml_escape(formula) + u'</f><v>' +
                             unicode(cached) + u'</v></c>')
            elif kind in ("n", "b"):
                parts.append(prefix + u' t="%s"><v>%s</v></c>' %
                             (kind, xml_escape(payload)))
            else:
                raise ValueError("Unknown XLSX fixture cell type: " + kind)
        parts.append(u'</row>')
    parts.append(u'</sheetData></worksheet>')
    return u"".join(parts)


def write_xlsx(name, sheets, date1904=False):
    main_ns = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
    rel_ns = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
    pkg_ns = "http://schemas.openxmlformats.org/package/2006/relationships"
    content_types = [
        '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">',
        '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>',
        '<Default Extension="xml" ContentType="application/xml"/>',
        '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>',
        '<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>',
    ]
    workbook = ['<workbook xmlns="%s" xmlns:r="%s">' % (main_ns, rel_ns),
                '<workbookPr date1904="%s"/><sheets>' % ("1" if date1904 else "0")]
    relationships = ['<Relationships xmlns="%s">' % pkg_ns]
    parts = []
    for index, (sheet_name, rows) in enumerate(sheets, 1):
        content_types.append('<Override PartName="/xl/worksheets/sheet%d.xml" '
                             'ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>' % index)
        workbook.append('<sheet name="%s" sheetId="%d" r:id="rId%d"/>' %
                        (xml_escape(sheet_name), index, index))
        relationships.append('<Relationship Id="rId%d" Type="%s/worksheet" '
                             'Target="worksheets/sheet%d.xml"/>' % (index, rel_ns, index))
        parts.append(("xl/worksheets/sheet%d.xml" % index, sheet_xml(rows)))
    workbook.append('</sheets><definedNames><definedName name="OffsetBlock">'
                    "'Offset Data'!$B$3:$I$10</definedName></definedNames></workbook>")
    relationships.append('<Relationship Id="rIdStyles" Type="%s/styles" Target="styles.xml"/></Relationships>' % rel_ns)
    content_types.append('</Types>')
    styles = ('<styleSheet xmlns="%s"><numFmts count="3">'
              '<numFmt numFmtId="164" formatCode="yyyy-mm-dd"/>'
              '<numFmt numFmtId="165" formatCode="hh:mm:ss"/>'
              '<numFmt numFmtId="166" formatCode="yyyy-mm-dd hh:mm:ss"/></numFmts>'
              '<fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts>'
              '<fills count="2"><fill><patternFill patternType="none"/></fill>'
              '<fill><patternFill patternType="gray125"/></fill></fills>'
              '<borders count="1"><border/></borders>'
              '<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>'
              '<cellXfs count="5">'
              '<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>'
              '<xf numFmtId="49" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>'
              '<xf numFmtId="164" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>'
              '<xf numFmtId="165" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>'
              '<xf numFmtId="166" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>'
              '</cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>') % main_ns
    parts.extend([
        ("[Content_Types].xml", "".join(content_types)),
        ("_rels/.rels", '<Relationships xmlns="%s"><Relationship Id="rId1" Type="%s/officeDocument" Target="xl/workbook.xml"/></Relationships>' % (pkg_ns, rel_ns)),
        ("xl/workbook.xml", "".join(workbook)),
        ("xl/_rels/workbook.xml.rels", "".join(relationships)),
        ("xl/styles.xml", styles),
    ])
    archive = zipfile.ZipFile(SOURCES_DIR + "/" + name, "w", zipfile.ZIP_STORED)
    try:
        for part_name, text in parts:
            info = zipfile.ZipInfo(part_name, (2000, 1, 1, 0, 0, 0))
            if not text.startswith("<?xml"):
                text = XML_DECLARATION + text
            archive.writestr(info, unicode(text).encode("utf-8"))
    finally:
        archive.close()
    manifest["saved"].append(name)
    manifest["inputs"].append({"file": name, "date1904": date1904,
                               "sheets": [[n, len(r)] for n, r in sheets]})


def input_props(columns, mode=None):
    props = [("names", [name for name, storage, data in columns])]
    for name, storage, data in columns:
        if storage is not None:
            props.append(("custom_storage", name, storage))
        props.append(("data", name, data))
    if mode is not None:
        props.append(("data_mode", mode))
    return props


ensure_clean_dir(SOURCES_DIR)
ensure_clean_dir(SOURCES_DIR + "/source_xml_dir")
ensure_clean_dir(SOURCES_DIR + "/source_xml_dir/nested")

fixed_lines = [fixed_line(row) for row in FIXED_ROWS]
write_text("source_fixed.txt", u"\r\n".join(fixed_lines) + u"\r\n", "82-character records, CRLF; unsorted ids, duplicate full row, missing cells, late mixed type")
write_text("source_fixed_packed.txt", u"".join(fixed_lines), "same records, no record separators")
write_text("source_fixed_header.txt", u"header one\r\nheader two\r\n" + u"\r\n".join(fixed_lines) + u"\r\n", "two header lines of different lengths")
comma_rows = [list(row) for row in FIXED_ROWS]
for row in comma_rows:
    row[3] = row[3].replace(".", ",")
write_text("source_fixed_comma.txt", u"\r\n".join([fixed_line(r) for r in comma_rows]) + u"\r\n", "comma decimals; otherwise identical")
invalid_rows = [list(row) for row in FIXED_ROWS]
invalid_rows[0][4] = "a" + chr(0) + "b"
invalid_rows[1][4] = "x" + chr(0) + chr(0) + "y"
write_text("source_fixed_nul.txt", u"\r\n".join([fixed_line(r) for r in invalid_rows]) + u"\r\n", "NUL inside fixed-width fields; later columns detect offset shifts")
utf_rows = [list(row) for row in FIXED_ROWS]
utf_rows[0][4] = u"caf\u00e9"
utf_rows[1][4] = u"\u6c49\u5b57"
write_text("source_fixed_utf8.txt", u"\r\n".join([fixed_line(r) for r in utf_rows]) + u"\r\n", "UTF-8, non-ASCII before date columns; character versus byte offsets")
write_text("source_fixed_short.txt", fixed_lines[0] + u"\r\n" + u"\r\n" + fixed_lines[1][:33] + u"\r\n" + fixed_lines[2] + u"\r\n" + u"009abc", "blank line, short middle record, partial final record without newline")

build_case("fixed_default", "fixed_file", "fixedfile", ["source_fixed.txt"], fixed_props("source_fixed.txt"), "Only filename, record_len and fields set; all import-option defaults, start/length layout, inference, duplicates and order")
for trim in ["Left", "Right", "Both"]:
    build_case("fixed_trim_" + trim.lower(), "fixed_file", "fixedfile", ["source_fixed.txt"], fixed_props("source_fixed.txt", [("strip_spaces", trim)]), "Change strip_spaces only; leading/trailing/space-only strings")
build_case("fixed_packed", "fixed_file", "fixedfile", ["source_fixed_packed.txt"], fixed_props("source_fixed_packed.txt", [("line_oriented", False)]), "Contiguous records; record_len controls framing")
build_case("fixed_skip_headers", "fixed_file", "fixedfile", ["source_fixed_header.txt"], fixed_props("source_fixed_header.txt", [("line_oriented", True), ("skip_header", 2)]), "Skip two variable-length header lines before fixed record framing")
build_case("fixed_decimal_comma", "fixed_file", "fixedfile", ["source_fixed_comma.txt"], fixed_props("source_fixed_comma.txt", [("decimal_symbol", "Comma")]), "Comma decimals and full real precision")
build_case("fixed_scan_one", "fixed_file", "fixedfile", ["source_fixed.txt"], fixed_props("source_fixed.txt", [("lines_to_scan", 1)]), "Late qty='late' versus numeric first row; compare default scan")
build_case("fixed_datetime_off", "fixed_file", "fixedfile", ["source_fixed.txt"], fixed_props("source_fixed.txt", [("auto_recognize_datetime", False)]), "Date, time, timestamp detection disabled; compare default storage")
custom_rows = [list(row) for row in FIXED_ROWS]
for row in custom_rows:
    if row[5]:
        row[5] = row[5][8:10] + "/" + row[5][5:7] + "/" + row[5][:4]
    row[6] = row[6].replace(":", ".")
write_text("source_fixed_custom.txt", u"\r\n".join([fixed_line(r) for r in custom_rows]) + u"\r\n", "DD/MM/YYYY dates and HH.MM.SS times for explicit format overrides")
custom_props = [("auto_recognize_datetime", False), ("decimal_symbol", "Period")]
for field, storage in [("code", "String"), ("qty", "Integer"), ("rv", "Real"),
                       ("dt", "Date"), ("tm", "Time"), ("stamp", "Timestamp")]:
    custom_props.append(("custom_storage", field, storage))
custom_props.extend([("custom_date_format", "dt", "DD/MM/YYYY"),
                     ("custom_time_format", "tm", "HH.MM.SS"),
                     ("custom_date_format", "stamp", "YYYY-MM-DD"),
                     ("custom_time_format", "stamp", "HH:MM:SS")])
build_case("fixed_custom_storage", "fixed_file", "fixedfile", ["source_fixed_custom.txt"], fixed_props("source_fixed_custom.txt", custom_props), "String leading zeros, all six concrete storages, numeric conversion failure, explicit date/time formats")
for invalid in ["Discard", "Replace"]:
    build_case("fixed_invalid_" + invalid.lower(), "fixed_file", "fixedfile", ["source_fixed_nul.txt"], fixed_props("source_fixed_nul.txt", [("invalid_char_mode", invalid), ("invalid_char_replacement", "?")]), "Discard versus replace NUL; preserve later field boundaries", "observe")
build_case("fixed_utf8", "fixed_file", "fixedfile", ["source_fixed_utf8.txt"], fixed_props("source_fixed_utf8.txt", [("encoding", "UTF-8")]), "Unicode characters versus bytes in field positions", "observe")
build_case("fixed_short_records", "fixed_file", "fixedfile", ["source_fixed_short.txt"], fixed_props("source_fixed_short.txt", [("line_oriented", True)]), "Blank/short lines and truncated final record: padding, discard or rejection", "observe")
build_case("fixed_fields_reordered", "fixed_file", "fixedfile", ["source_fixed.txt"], [filename("source_fixed.txt"), ("record_len", 82), ("fields", [["last", 64, 19], ["first_char", 1, 1], ["code", 4, 6], ["overlap", 2, 4]])], "Field-list order, one-character length, gaps and overlapping slices", "observe")


# --- User Input (11 cases). Whitespace separates values; commas define ranges. ---
USER_BASE = [("z", "Integer", "3 1 3"), ("a", "String", "beta alpha beta")]
build_case("user_default", "user_input", "userinput", [], input_props(USER_BASE), "Default data_mode, 3x3 combinations versus 3 ordered rows; duplicates and field order")
build_case("user_ordered", "user_input", "userinput", [], input_props(USER_BASE, "Ordered"), "Ordered alignment and duplicate rows")
build_case("user_combined", "user_input", "userinput", [], input_props(USER_BASE, "Combined"), "Cartesian enumeration, fastest-changing column, duplicate multiplicity")
build_case("user_unequal", "user_input", "userinput", [], input_props([("id", "Integer", "4 1 2 3 5"), ("short", "String", "A B"), ("empty", "Real", "")], "Ordered"), "Unequal lengths pad with null; entirely blank numeric column")
build_case("user_combined_blank", "user_input", "userinput", [], input_props([("a", "Integer", "2 1"), ("blank", "String", ""), ("b", "String", "x y")], "Combined"), "Blank column in Cartesian data: null singleton versus zero records", "observe")
build_case("user_strings", "user_input", "userinput", [], input_props([("id", "Integer", "1 2 3 4 5 6"), ("text", "String", u'"" "  " "Bill Smith" "has,comma" caf\u00e9 "Bill Smith"')], "Ordered"), "Empty/space-only/quoted strings, comma inside quotes, UTF-8 and duplicates", "observe")
build_case("user_integer_ranges", "user_input", "userinput", [], input_props([("series", "Integer", "1 5 7 10,20,2 21 23"), ("negative", "Integer", "-3,3,2")], "Ordered"), "Documented embedded ranges, inclusive endpoints, negative starts and unequal lengths")
build_case("user_real_ranges", "user_input", "userinput", [], input_props([("x", "Real", "-0.5,0.5,0.25"), ("y", "Real", "0,0.3,0.1"), ("precise", "Real", "0.123456789012345 -3.5 0 1e3")], "Ordered"), "Real steps, floating endpoint inclusion, scientific notation and 15-place precision", "observe")
build_case("user_descending_range", "user_input", "userinput", [], input_props([("x", "Integer", "5,1,-2"), ("y", "Integer", "9 8 7")], "Ordered"), "Descending range support and negative step", "observe")
build_case("user_inferred", "user_input", "userinput", [], input_props([("ints", None, "001 -2 0 7"), ("real", None, "1 2.5 -3 0"), ("mixed", None, "1 text 2"), ("blank", None, "")], "Ordered"), "No custom_storage: numeric strings, int-real promotion, mixed and all-null inference", "observe")
build_case("user_temporal", "user_input", "userinput", [], input_props([("day", "Date", "1999-12-31 2020-02-29 2021-01-01"), ("clock", "Time", "00:00:00 12:34:56 23:59:59"), ("stamp", "Timestamp", '"1999-12-31 00:00:00" "2020-02-29 12:34:56"')], "Ordered"), "Date/time/timestamp storage and null padding with explicit stream formats", stream_props=[("date_format", "YYYY-MM-DD"), ("time_format", "HH:MM:SS")])


# --- JSON (12 cases). Literal JSON keeps key order and duplicate keys intentional. ---
JSON_MAIN = u'''[
 {"id":3,"i":-10,"r":-3.5,"flag":true,"s":"banana"},
 {"id":1,"i":0,"r":0.123456789012345,"flag":false,"s":"Apple"},
 {"id":2,"i":7,"r":2.5,"flag":true,"s":""},
 {"id":3,"i":-10,"r":-3.5,"flag":true,"s":"banana"},
 {"id":5,"i":null,"r":null,"flag":null,"s":null},
 {"id":6,"i":8,"r":0.0,"flag":false,"s":"  spaced  "}
]'''
JSON_FIXTURES = [
    ("source_records.json", JSON_MAIN, "ordered keys, unsorted rows, duplicate full row, booleans, int/real, null, empty and spaces"),
    ("source_values.json", '[[3,-10,-3.5,true,"banana"],[1,0,0.123456789012345,false,"Apple"],[2,7,2.5,true,""],[3,-10,-3.5,true,"banana"],[5,null,null,null,null],[6,8,0.0,false,"  spaced  "]]', "same values without field names"),
    ("source_sparse.json", '[{"z":3,"a":"A","all_null":null},{"a":"","z":1,"late":9},{"z":2,"a":null},{"late":7,"z":4}]', "key union/order, absent versus null versus empty, all-null column"),
    ("source_mixed.json", '[{"id":1,"x":1,"y":true},{"id":2,"x":"text","y":2},{"id":3,"x":2.5,"y":false},{"id":4,"x":null,"y":null}]', "heterogeneous column coercion and bool/numeric promotion"),
    ("source_numbers.json", '[{"id":1,"x":9007199254740991,"r":1e-12},{"id":2,"x":9007199254740993,"r":1e20},{"id":3,"x":9223372036854775807,"r":-0.0},{"id":4,"x":null,"r":0.123456789012345}]', "integer precision limits, null promotion, exponents and negative zero"),
    ("source_dates.json", '[{"id":1,"d":"2020-02-29","t":"01:02:03","ts":"2020-02-29T01:02:03.000Z"},{"id":2,"d":"1999-12-31","t":"23:59:59","ts":"1999-12-31T23:59:59+02:00"},{"id":3,"d":null,"t":"","ts":null}]', "date-looking strings and timezone text: automatic conversion or strings"),
    ("source_unicode.json", u'[{"id":1,"s":"caf\u00e9 \u6c49\u5b57"},{"id":2,"s":"quote: \\" backslash: \\\\ comma, tab\\tend"},{"id":3,"s":"line1\\nline2"}]', "literal UTF-8, JSON escapes, tab and newline"),
    ("source_duplicate_keys.json", '[{"id":1,"x":10,"x":20},{"id":2,"x":30,"x":40}]', "duplicate JSON object key precedence or rejection"),
    ("source_empty.json", '[]', "empty array with no inferable columns"),
    ("source_nested.json", '[{"id":1,"x":{"a":1}},{"id":2,"x":{"a":2}}]', "documented unsupported nested objects"),
    ("source_bad.json", '[{"id":1}, {"id":2,}]', "malformed JSON, trailing comma"),
]
for name, text, description in JSON_FIXTURES:
    write_text(name, text, description)


def json_case(case_name, data, extra, tests, expectation="success"):
    build_case(case_name, "json_import", "jsonimport", [data], [filename(data)] + extra,
               tests, expectation)


json_case("json_default", "source_records.json", [], "Default string_format=records; native JSON types, missing cells, duplicates and order")
json_case("json_records", "source_records.json", [("string_format", "records")], "Explicit records equals default")
json_case("json_values", "source_values.json", [("string_format", "values")], "Values orientation, generated field names; same rows as records")
json_case("json_sparse", "source_sparse.json", [], "Union/order of heterogeneous keys and all-null storage", "observe")
json_case("json_mixed_types", "source_mixed.json", [], "Mixed string/numeric and bool/numeric columns", "observe")
json_case("json_number_limits", "source_numbers.json", [], "Large integer precision with missing cell, exponent and negative zero", "observe")
json_case("json_date_strings", "source_dates.json", [], "Date-like strings and timezone preservation", "observe")
json_case("json_unicode_escapes", "source_unicode.json", [], "Literal UTF-8, escaped quote/backslash/tab/newline")
json_case("json_duplicate_keys", "source_duplicate_keys.json", [], "Last/first duplicate key wins or error", "observe")
json_case("json_empty", "source_empty.json", [], "Zero-record/no-column source versus rejection", "observe")
json_case("json_nested_rejected", "source_nested.json", [], "Unsupported nested-object error boundary", "rejection")
json_case("json_malformed_rejected", "source_bad.json", [], "Malformed-file error boundary", "rejection")


# --- XML (14 cases). Schema, fields, record boundary, directory and recursion. ---
XML_MAIN = u'''<?xml version="1.0" encoding="UTF-8"?>
<records>
 <record id="3"><name>banana</name><qty>-10</qty><price>-3.5</price><day>2020-02-29</day></record>
 <record id="1"><name>Apple</name><qty>0</qty><price>0.123456789012345</price><day>1999-12-31</day></record>
 <record id="2"><name></name><qty>7</qty><price>2.5</price><day>2021-01-01</day></record>
 <record id="3"><name>banana</name><qty>-10</qty><price>-3.5</price><day>2020-02-29</day></record>
 <record id="5"><name/><qty/><price/><day/></record>
 <record id="6"><name>  spaced  </name><qty>8</qty><price>0</price></record>
</records>'''
XSD = '''<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
 <xs:element name="records"><xs:complexType><xs:sequence>
  <xs:element name="record" maxOccurs="unbounded"><xs:complexType><xs:sequence>
   <xs:element name="name" type="xs:string"/>
   <xs:element name="qty" type="xs:integer" minOccurs="0"/>
   <xs:element name="price" type="xs:decimal" minOccurs="0"/>
   <xs:element name="day" type="xs:date" minOccurs="0"/>
  </xs:sequence><xs:attribute name="id" type="xs:integer" use="required"/>
  </xs:complexType></xs:element>
 </xs:sequence></xs:complexType></xs:element>
</xs:schema>'''
DTD = '''<!ELEMENT records (record*)>
<!ELEMENT record (name,qty?,price?,day?)>
<!ATTLIST record id CDATA #REQUIRED>
<!ELEMENT name (#PCDATA)>
<!ELEMENT qty (#PCDATA)>
<!ELEMENT price (#PCDATA)>
<!ELEMENT day (#PCDATA)>'''
XML_FIXTURES = [
    ("source_records.xml", XML_MAIN, "attributes, scalars, empty/self-closing/missing fields, duplicate rows and input order"),
    ("source_schema.xml", XML_MAIN.replace('<record id="5"><name/><qty/><price/><day/></record>', '<record id="5"><name/></record>'), "schema-valid optional numbers/dates absent rather than empty"),
    ("source_records.xsd", XSD, "single-root typed schema; no interactive root selection required"),
    ("source_records.dtd", DTD, "single-root DTD with the same element structure"),
    ("source_nested.xml", '<orders><order id="B"><line n="2"><value>20</value></line><line n="1"><value>10</value></line></order><order id="A"><line n="3"><value>30</value></line></order></orders>', "nested record boundaries and parent attributes"),
    ("source_repeated.xml", '<records><record id="3"><tag>red</tag><tag>blue</tag><v>1</v></record><record id="1"><tag>green</tag><v>2</v></record><record id="2"><v>3</v></record><record id="4"><tag>red</tag><tag>red</tag><v>4</v></record></records>', "repeated siblings: list, columns, row expansion, overwrite or rejection"),
    ("source_text.xml", u'<records><record id="1"><s>caf\u00e9 &amp; &lt;tag&gt;</s></record><record id="2"><s>  left\r\n  right\tend  </s></record><record id="3"><s><![CDATA[a<b & c]]></s></record><record id="4"><s/></record><record id="5"/></records>', "UTF-8, entities, CDATA, CRLF/tab, empty versus absent"),
    ("source_empty.xml", '<records/>', "no default child record boundary"),
    ("source_bad.xml", '<records><record><id>1</id></records>', "mismatched XML tags"),
    ("source_namespace.xml", '<p:records xmlns:p="urn:source-probe"><p:record><p:id>1</p:id></p:record></p:records>', "documented unsupported namespace-qualified names"),
    # Write reverse lexical order deliberately; directory enumeration order is an observation.
    ("source_xml_dir/z-last.xml", '<records><record><id>30</id><s>Z</s></record><record><id>31</id><s>Z2</s></record></records>', "top-level XML file created first"),
    ("source_xml_dir/a-first.xml", '<records><record><id>10</id><s>A</s></record><record><id>11</id><s>A2</s></record></records>', "top-level XML file created second"),
    ("source_xml_dir/nested/m-middle.xml", '<records><record><id>20</id><s>M</s></record><record><id>21</id><s>M2</s></record></records>', "subdirectory input for recurse=True"),
    ("source_xml_dir/ignore.txt", 'not an XML document', "directory mode should select XML files only"),
]
for name, text, description in XML_FIXTURES:
    write_text(name, text, description)


def xml_case(case_name, data, extra, tests, expectation="success", dependencies=None):
    # Follow Table 61's example: give a record XPath explicitly instead of
    # relying on a UI-selected/default boundary in a freshly scripted node.
    boundary = [] if any(item[0] == "records" for item in extra) else [("records", "/records/record")]
    build_case(case_name, "xml_import", "xmlimport", [data] + (dependencies or []),
               [filename(data)] + boundary + extra, tests, expectation)


xml_case("xml_default", "source_records.xml", [], "Guide example form: filename and record XPath; default single/read modes")
xml_case("xml_record_path", "source_records.xml", [("records", "/records/record")], "Explicit record boundary; compare the minimal example configuration")
# Try one XPath per structured Fields-table row. Flat string lists were rejected
# at configure in run 1; this inferred representation needs an oracle rerun to
# confirm acceptance (Table 61 has no fields example).
xml_case("xml_fields_absolute", "source_records.xml", [("records", "/records/record"), ("mode", "specify"), ("fields", [["/records/record/price"], ["/records/record/@id"], ["/records/record/name"]])], "Select/reorder scalar elements and attributes with absolute XPath", "observe")
xml_case("xml_fields_relative", "source_records.xml", [("records", "/records/record"), ("mode", "specify"), ("fields", [["price"], ["@id"], ["name"]])], "Same selections with paths relative to record; API path convention", "observe")
xml_case("xml_nested_records", "source_nested.xml", [("records", "/orders/order/line")], "Nested line record boundary; parent attributes, sibling and record ordering", "observe")
xml_case("xml_repeated_elements", "source_repeated.xml", [], "Repeated sibling values, duplicates and missing repeated field", "observe")
xml_case("xml_text_whitespace", "source_text.xml", [], "Escapes/CDATA/UTF-8, whitespace normalization and absent versus empty")
xml_case("xml_xsd", "source_schema.xml", [("full_schema_filename", SOURCES_DIR + "/source_records.xsd"), ("records", "/records/record")], "Schema-driven integer/decimal/date types and missing optional elements", "observe", ["source_records.xsd"])
xml_case("xml_dtd", "source_schema.xml", [("full_schema_filename", SOURCES_DIR + "/source_records.dtd"), ("records", "/records/record")], "DTD structure versus inferred/XSD structure", "observe", ["source_records.dtd"])
XML_DIRECTORY_INPUTS = ["source_xml_dir/z-last.xml", "source_xml_dir/a-first.xml", "source_xml_dir/nested/m-middle.xml", "source_xml_dir/ignore.txt"]
for recurse in [False, True]:
    build_case("xml_directory_recursive" if recurse else "xml_directory", "xml_import", "xmlimport", XML_DIRECTORY_INPUTS, [("read", "directory"), ("directory_name", SOURCES_DIR + "/source_xml_dir"), ("recurse", recurse), ("records", "/records/record")], "Directory XML filtering, within/across-file order and recursion", "observe")
xml_case("xml_empty", "source_empty.xml", [], "No records/columns versus rejection", "observe")
xml_case("xml_malformed_rejected", "source_bad.xml", [], "Malformed-file error boundary", "rejection")
xml_case("xml_namespace_rejected", "source_namespace.xml", [], "Namespace-qualified input limitation", "rejection")


# --- Excel (15 cases). Cells, sheets, ranges, blank rows, scan horizon and epochs. ---
EXCEL_MAIN = [
    ["id", "integer", "real", "text", "mixed", "flag", "day", "formula"],
    [cell("n", 3), cell("n", -10), cell("n", -3.5), "banana", cell("n", 1), cell("b", 1), cell("n", 43890, 2), cell("f", ("1+2", 3))],
    [cell("n", 1), cell("n", 0), cell("n", "0.123456789012345"), "", "oops", cell("b", 0), cell("n", 36525, 2), cell("f", ("2+3", 5))],
    [cell("n", 2), cell("n", 7), cell("n", 2.5), None, cell("n", 2), None, None, None],
    [cell("n", 3), cell("n", -10), cell("n", -3.5), "banana", cell("n", 1), cell("b", 1), cell("n", 43890, 2), cell("f", ("1+2", 3))],
    [cell("n", 5), None, None, "  spaced  ", "", cell("b", 0), cell("n", 44197, 2), cell("n", 0)],
    [cell("n", 6), cell("n", 8), cell("n", -0.25), u"caf\u00e9 \u6c49\u5b57", cell("n", 3), cell("b", 1), cell("n", 43845, 2), cell("n", 7)],
    [cell("n", 7), cell("n", 9), cell("n", 9), 'quote " slash \\ comma,', cell("n", 4), cell("b", 0), cell("n", 43846, 2), cell("n", -1)],
]
offset_rows = [[], []] + [[None] + list(row) for row in EXCEL_MAIN]
blank_rows = [["id", "s"], [cell("n", 3), "before"], [], [cell("n", 1), "after_single_blank"], [], [], [cell("n", 2), "after_two_blanks"], [cell("n", 2), "duplicate"]]
late_rows = [["id", "late_real", "late_text", "late_numeric"]]
for index in range(1, 206):
    late_rows.append([cell("n", index), cell("n", "2.5" if index == 205 else "1"),
                      "text" if index == 205 else cell("n", 7),
                      cell("n", 9) if index == 205 else None])
text_rows = [["id", "general", "as_text", "leading_zeros"],
             [cell("n", 1), cell("n", 12), cell("n", 12, 1), cell("s", "0012", 1)],
             [cell("n", 2), "oops", cell("s", "oops", 1), cell("s", "0000", 1)],
             [cell("n", 3), cell("n", 2.5), cell("n", 2.5, 1), cell("s", "0100", 1)],
             [cell("n", 4), None, cell("s", "", 1), None]]
header_rows = [["dup", "dup", None, cell("n", 17), "with space", "a.b", ""],
               [cell("n", 1), cell("n", 2), "A", "x", "one", "left", "empty_name"],
               [cell("n", 3), cell("n", 4), "B", "y", "two", "right", "last"]]
date_rows = [["id", "day", "clock", "stamp", "serial"]]
for index, serial in enumerate([0, 1, 59, 60, 61, 43890], 1):
    date_rows.append([cell("n", index), cell("n", serial, 2),
                      cell("n", "0.5" if index % 2 else "0.25", 3),
                      cell("n", unicode(serial) + ".5", 4), cell("n", serial)])
SHEETS = [("Main", EXCEL_MAIN), ("Offset Data", offset_rows), ("Blank Rows", blank_rows),
          ("Late Types", late_rows), ("Text Format", text_rows), ("Headers", header_rows),
          ("Dates", date_rows)]
write_xlsx("source_book.xlsx", SHEETS)
write_xlsx("source_book_1904.xlsx", SHEETS, True)


def excel_case(case_name, extra, tests, expectation="success", book="source_book.xlsx"):
    build_case(case_name, "excel_import", "excelimport", [book], [filename(book)] + extra,
               tests, expectation)


excel_case("excel_default", [], "Only filename set: defaults; typed cells, mixed numeric/text, empty/absent, duplicate rows, order, formula cache")
excel_case("excel_no_headers", [("read_field_names", False)], "First row becomes data; generated column names and resulting storage")
excel_case("excel_sheet_index", [("worksheet_mode", "Index"), ("worksheet_index", 1)], "Zero-based sheet index and first nonblank B3 origin")
excel_case("excel_sheet_name", [("worksheet_mode", "Name"), ("worksheet_name", "Offset Data")], "Name selection equals index 1; sheet name with space")
excel_case("excel_explicit_a1", [("worksheet_mode", "Name"), ("worksheet_name", "Offset Data"), ("data_range_mode", "ExplicitRange"), ("explicit_range_start", "B3"), ("explicit_range_end", "I12")], "Inclusive A1 range, offset headers, two trailing blank rows")
excel_case("excel_explicit_r1c1", [("worksheet_mode", "Name"), ("worksheet_name", "Offset Data"), ("data_range_mode", "ExplicitRange"), ("explicit_range_start", "R3C2"), ("explicit_range_end", "R12C9")], "R1C1 notation equals A1 range", "observe")
excel_case("excel_named_range", [("excel_file_type", "Excel2007"), ("worksheet_mode", "Name"), ("worksheet_name", "Blank Rows"), ("data_range_mode", "ExplicitRange"), ("explicit_range_start", "A1"), ("explicit_range_end", "A1"), ("use_named_range", True), ("named_range", "OffsetBlock")], "Named range B3:I10 overrides a different valid worksheet and explicit range", "observe")
for blank_mode in ["StopReading", "ReturnBlankRows"]:
    excel_case("excel_blank_stop" if blank_mode == "StopReading" else "excel_blank_return", [("worksheet_mode", "Name"), ("worksheet_name", "Blank Rows"), ("data_range_mode", "FirstNonBlank"), ("blank_rows", blank_mode)], "One isolated blank row then two consecutive blanks; stop versus return all")
for scan in [1, 250]:
    # scanLineCount is the persisted setting name, rejected as a scripting
    # property in run 1. Use the source-node scripting spelling, as for text.
    excel_case("excel_scan_%d" % scan, [("worksheet_mode", "Name"), ("worksheet_name", "Late Types"), ("lines_to_scan", scan)], "205 data rows; only last row introduces real, text and a previously all-null value", "observe")
excel_case("excel_text_format", [("worksheet_mode", "Name"), ("worksheet_name", "Text Format")], "General mixed cells versus Text format (@), numeric text cells and leading zeros", "observe")
excel_case("excel_header_names", [("worksheet_mode", "Name"), ("worksheet_name", "Headers")], "Duplicate, absent, numeric, space/dot and empty column headers", "observe")
excel_case("excel_dates_1900", [("worksheet_mode", "Name"), ("worksheet_name", "Dates")], "Excel serials 0/1/59/60/61, leap-day anomaly, times, timestamps and underlying serials", "observe")
excel_case("excel_dates_1904", [("worksheet_mode", "Name"), ("worksheet_name", "Dates")], "Same cells with workbook date1904=true; epoch offset", "observe", "source_book_1904.xlsx")


manifest["counts"] = {}
for entry in manifest["cases"]:
    key = entry["catalog_id"]
    manifest["counts"][key] = manifest["counts"].get(key, 0) + 1
handle = open(OUT_DIR + "/manifest.json", "w")
try:
    json.dump(manifest, handle, indent=2, sort_keys=True, ensure_ascii=True)
finally:
    handle.close()
