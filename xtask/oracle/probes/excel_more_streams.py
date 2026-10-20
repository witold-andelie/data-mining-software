# Probe: Excel source follow-ups in corpus/excel_more/ (label excel_more).
# Run by the coordinator with `cargo xtask probe excel_more_streams` (Jython 2.7).
# Public Modeler scripting API only; the XLSX fixture uses Python's standard library. The
# helpers are those of source_streams.py; sheet_xml can omit empty <row> elements as Excel does.
import json
import zipfile
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
SOURCES_DIR = "D:/data-mining-software/corpus/excel_more"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
taskrunner = modeler.script.session().getTaskRunner()
manifest = {
    "label": "excel_more", "saved": [], "errors": [], "expected_errors": [],
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


def sheet_xml(rows, empty_rows=True):
    parts = [u'<?xml version="1.0" encoding="UTF-8"?>',
             u'<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">',
             u'<dimension ref="%s"/><sheetData>' % sheet_dimension(rows)]
    for row_num, row in enumerate(rows, 1):
        if not empty_rows and all(value is None for value in row):
            continue
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
    for index, sheet in enumerate(sheets, 1):
        sheet_name, rows = sheet[0], sheet[1]
        empty_rows = sheet[2] if len(sheet) > 2 else True
        content_types.append('<Override PartName="/xl/worksheets/sheet%d.xml" '
                             'ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>' % index)
        workbook.append('<sheet name="%s" sheetId="%d" r:id="rId%d"/>' %
                        (xml_escape(sheet_name), index, index))
        relationships.append('<Relationship Id="rId%d" Type="%s/worksheet" '
                             'Target="worksheets/sheet%d.xml"/>' % (index, rel_ns, index))
        parts.append(("xl/worksheets/sheet%d.xml" % index, sheet_xml(rows, empty_rows)))
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
                               "sheets": [[s[0], len(s[1])] + list(s[2:]) for s in sheets]})




# --- Cases. Follow-up to the `sources` excel_* oracles (2026-10-20): each sheet or range
# separates one open question about Modeler's Excel reader.
ensure_clean_dir(SOURCES_DIR)
MAIN_ROWS = [
    ["id", "integer", "real", "text", "mixed", "flag", "day", "formula"],
    [cell("n", 3), cell("n", -10), cell("n", -3.5), "banana", cell("n", 1), cell("b", 1), cell("n", 43890, 2), cell("f", ("1+2", 3))],
    [cell("n", 1), cell("n", 0), cell("n", "0.123456789012345"), "", "oops", cell("b", 0), cell("n", 36525, 2), cell("f", ("2+3", 5))],
    [cell("n", 2), cell("n", 7), cell("n", 2.5), None, cell("n", 2), None, None, None],
    [cell("n", 3), cell("n", -10), cell("n", -3.5), "banana", cell("n", 1), cell("b", 1), cell("n", 43890, 2), cell("f", ("1+2", 3))],
    [cell("n", 5), None, None, "  spaced  ", "", cell("b", 0), cell("n", 44197, 2), cell("n", 0)],
    [cell("n", 6), cell("n", 8), cell("n", -0.25), u"café 汉字", cell("n", 3), cell("b", 1), cell("n", 43845, 2), cell("n", 7)],
    [cell("n", 7), cell("n", 9), cell("n", 9), 'quote " slash \\ comma,', cell("n", 4), cell("b", 0), cell("n", 43846, 2), cell("n", -1)],
]
OFFSET_ROWS = [[], []] + [[None] + list(row) for row in MAIN_ROWS]
NUMERIC = [["id", "x", "y"]] + [[cell("n", i), cell("n", i * 10), cell("n", i + 0.5)] for i in range(1, 7)]
BLANK_ROWS = [["id", "s"], [cell("n", 3), "before"], [], [cell("n", 1), "after_single_blank"], [], [],
              [cell("n", 2), "after_two_blanks"], [cell("n", 2), "duplicate"]]
BLANK2 = [["id", "s"], [cell("n", 1), "a"], [], [cell("n", 2), "b"], [cell("n", 3), "c"], [], [cell("n", 4), "d"]]
BLANK3 = [["id", "s"], [], [cell("n", 1), "a"], [cell("n", 2), "b"]]
COMMAS = [["id", "s"]] + [[cell("n", i), text] for i, text in enumerate(
    ["a,b", ",lead", "trail,", "x,y,z", "1,5", "plain", "dot.", "a;b", "3,000.5", "comma, space"], 1)]
SHEETS = [
    ("Offset Data", OFFSET_ROWS),                                   # as in source_book.xlsx
    ("OffNoEmpty", OFFSET_ROWS, False),                             # no empty <row> elements
    ("Off5", [[]] * 4 + [[None, None] + list(r) for r in NUMERIC], False),   # C5:E11
    ("OffA3", [[], []] + NUMERIC, False),                           # A3:C9, row offset only
    ("OffC1", [[None, None] + list(r) for r in NUMERIC], False),    # C1:E7, column offset only
    ("OffTyped", [[], []] + [[None] + list(r) for r in NUMERIC]),   # B3:D9, empty <row>s
    ("Commas", COMMAS, False),
    ("BlankNoEmpty", BLANK_ROWS, False),
    ("Blank2", BLANK2),
    ("Blank3", BLANK3),
    ("Blank2NoEmpty", BLANK2, False),
    ("Blank3NoEmpty", BLANK3, False),
]
write_xlsx("excel_more.xlsx", SHEETS)
BOOK = "excel_more.xlsx"


def excel_case(case_name, extra, tests):
    build_case(case_name, "excel_import", "excelimport", [BOOK], [filename(BOOK)] + extra,
               tests, "observe")


def sheet(name, *extra):
    return [("worksheet_mode", "Name"), ("worksheet_name", name)] + list(extra)


def explicit(name, start, end, *extra):
    return sheet(name, ("data_range_mode", "ExplicitRange"), ("explicit_range_start", start),
                 ("explicit_range_end", end), *extra)


RETURN = ("blank_rows", "ReturnBlankRows")
excel_case("more_offset", sheet("Offset Data"), "Repeat of sources/excel_sheet_name (B3:I10, empty rows 1-2)")
excel_case("more_off_noempty", sheet("OffNoEmpty"), "Offset B3:I10 without empty <row> elements")
excel_case("more_off5", sheet("Off5"), "Numeric block at C5:E11")
excel_case("more_offa3", sheet("OffA3"), "Numeric block at A3:C9 (row offset only)")
excel_case("more_offc1", sheet("OffC1"), "Numeric block at C1:E7 (column offset only)")
excel_case("more_offtyped", sheet("OffTyped"), "Numeric block at B3:D9 with empty <row> elements")
excel_case("more_commas", sheet("Commas"), "Commas inside text cells")
excel_case("more_blank_noempty_return", sheet("BlankNoEmpty", RETURN), "sources/excel_blank_return without empty <row> elements")
excel_case("more_blank2_return", sheet("Blank2", RETURN), "data, blank, data, data, blank, data")
excel_case("more_blank3_return", sheet("Blank3", RETURN), "Blank row right after the header")
excel_case("more_blank2_stop", sheet("Blank2", ("blank_rows", "StopReading")), "StopReading on Blank2")
excel_case("more_blank_noempty_stop", sheet("BlankNoEmpty", ("blank_rows", "StopReading")), "StopReading without empty <row> elements")
excel_case("more_blank2_noempty_return", sheet("Blank2NoEmpty", RETURN), "Blank2 without empty <row> elements")
excel_case("more_blank2_noempty_stop", sheet("Blank2NoEmpty", ("blank_rows", "StopReading")), "Blank2 StopReading without empty <row> elements")
excel_case("more_blank3_noempty_return", sheet("Blank3NoEmpty", RETURN), "Blank3 without empty <row> elements")
excel_case("more_explicit_exact", explicit("Offset Data", "B3", "I10"), "Range equal to the dimension")
excel_case("more_explicit_inside", explicit("Offset Data", "B3", "I8"), "Range ending inside the data")
excel_case("more_explicit_beyond1", explicit("Offset Data", "B3", "I11"), "Range one row past the dimension")
excel_case("more_explicit_noheader", explicit("Offset Data", "B4", "I8", ("read_field_names", False)), "Data rows only, no field names")
excel_case("more_explicit_numeric", explicit("OffNoEmpty", "C4", "E9"), "Sub-block, field names from row 4")
excel_case("more_explicit_a3_beyond", explicit("OffA3", "A3", "C12"), "Range past the dimension on a row-offset sheet")


manifest["counts"] = {"excel_import": len(manifest["cases"])}
handle = open(OUT_DIR + "/manifest.json", "w")
try:
    json.dump(manifest, handle, indent=2, sort_keys=True, ensure_ascii=True)
finally:
    handle.close()
