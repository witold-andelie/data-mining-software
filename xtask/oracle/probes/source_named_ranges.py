# Probe: Excel source streams that read a named range, in corpus/sources/.
# Run by the coordinator with `cargo xtask probe source_named_ranges` (Jython 2.7).
# Public Modeler scripting API only.
#
# `excel_named_range` (source_streams.py) cannot be recorded: its workbook defines the name on a
# sheet whose name has a space (`'Offset Data'!$B$3:$I$10`) and Modeler's server process crashes
# as soon as `use_named_range` is true (observed with `use_named_range` alone, without any other
# setting). Names on sheets without a space work, so this probe writes a copy of the workbook
# with such names and saves one stream per question:
#   excel_named_main      only the named range (Main!$A$1:$D$4)
#   excel_named_override  the named range plus a different worksheet and explicit range
#   excel_named_headers   a second name (Headers!$A$1:$B$3): its first row holds the field names
#   excel_named_missing   a name the workbook does not define
import json
import zipfile
import java.lang.Exception
import modeler.api
import modeler.script

OUT_DIR = @OUT_DIR@
SOURCES_DIR = "D:/data-mining-software/corpus/sources"
SOURCE_BOOK = "source_book.xlsx"
NAMED_BOOK = "source_book_named.xlsx"
NAMES = [("MainBlock", "Main!$A$1:$D$4"), ("HeadersBlock", "Headers!$A$1:$B$3")]
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "sources", "saved": [], "errors": [], "cases": []}


def message(exc):
    get = getattr(exc, "getMessage", None)
    if get is not None:
        return unicode(get())
    return unicode(exc)


def write_named_book():
    source = zipfile.ZipFile(SOURCES_DIR + "/" + SOURCE_BOOK, "r")
    target = zipfile.ZipFile(SOURCES_DIR + "/" + NAMED_BOOK, "w", zipfile.ZIP_DEFLATED)
    try:
        defined = "".join('<definedName name="%s">%s</definedName>' % item for item in NAMES)
        for name in source.namelist():
            data = source.read(name)
            if name == "xl/workbook.xml":
                start = data.index("<definedNames>")
                end = data.index("</definedNames>") + len("</definedNames>")
                data = data[:start] + "<definedNames>" + defined + "</definedNames>" + data[end:]
            target.writestr(name, data)
    finally:
        target.close()
        source.close()


def named_case(case_name, props, tests):
    entry = {"case": case_name, "file": case_name + ".str", "properties": props,
             "tests": tests, "saved": False}
    manifest["cases"].append(entry)
    stage = "create"
    stream = None
    try:
        stream = taskrunner.createStream(case_name, False, False)
        stream.setPropertyValue("decimal_places", 15)
        node = stream.createAt("excelimport", case_name, 100, 100)
        node.setPropertyValue("full_filename", SOURCES_DIR + "/" + NAMED_BOOK)
        for name, value in props:
            node.setPropertyValue(name, value)
        table = stream.createAt("table", "Table", 380, 100)
        stream.link(node, table)
        stage = "save"
        taskrunner.saveStreamToFile(stream, SOURCES_DIR + "/" + entry["file"])
        manifest["saved"].append(entry["file"])
        entry["saved"] = True
        stage = "run_table"
        results = []
        table.run(results)
        entry["status"] = "ran"
        taskrunner.saveStreamToFile(stream, SOURCES_DIR + "/" + entry["file"])
    except ERRORS as exc:
        entry["status"] = "error"
        manifest["errors"].append({"case": case_name, "stage": stage, "message": message(exc)})
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS as exc:
                manifest["errors"].append({"case": case_name, "stage": "clear",
                                           "message": message(exc)})


write_named_book()
named_case("excel_named_main", [("use_named_range", True), ("named_range", "MainBlock")],
           "Only the named range: Main!A1:D4, first row as field names")
named_case("excel_named_override",
           [("excel_file_type", "Excel2007"), ("worksheet_mode", "Name"),
            ("worksheet_name", "Blank Rows"), ("data_range_mode", "ExplicitRange"),
            ("explicit_range_start", "A1"), ("explicit_range_end", "A1"),
            ("use_named_range", True), ("named_range", "MainBlock")],
           "The named range overrides a different valid worksheet and explicit range")
named_case("excel_named_headers", [("use_named_range", True), ("named_range", "HeadersBlock")],
           "A second name: first row of Headers!A1:B3 gives the field names")
named_case("excel_named_missing", [("use_named_range", True), ("named_range", "NoSuchName")],
           "A name the workbook does not define")
handle = open(OUT_DIR + "/manifest.json", "w")
try:
    json.dump(manifest, handle, indent=2, sort_keys=True, ensure_ascii=True)
finally:
    handle.close()
