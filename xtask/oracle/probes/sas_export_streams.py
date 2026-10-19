# Probe: SAS export node (label sas_export).
# Run with `cargo xtask probe sas_export_streams` (IBM SPSS Modeler, Jython 2.7, public API only).
# Each stream is Variable File (export_input.csv) -> Type -> SAS export, one case per Table 253 format
# and name option. The misc_next probe saw "output encoder was not open" (-11) and empty files;
# that came with Windows' UTF-8 system code page, which is now off (see sas_streams.py).
# `cargo xtask oracle sas_export <case>.str` captures the written file under exports/.
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/sas_export"
CSV_NAME = "export_input.csv"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "sas_export", "saved": [], "errors": [], "cases": []}

# name, storage, cells (None = empty cell)
COLUMNS = [
    ("id", "Integer", ["1", "2", "3", "4"]),
    ("name", "String", ["Alice", "Bob", "", "Dan"]),
    ("amount", "Real", ["12.5", None, "-7.25", "1000"]),
    ("joined", "Date", ["2020-01-01", "2021-03-15", None, "2019-12-31"]),
    ("long_field_name_x", "Real", ["0.25", "-1.5", "3", None]),
]


def message(exc):
    get = getattr(exc, "getMessage", None)
    if get is not None:
        return unicode(get())
    return unicode(exc)


def write_manifest():
    handle = open(OUT_DIR + "/manifest.json", "w")
    try:
        json.dump(manifest, handle, indent=2, sort_keys=True)
    finally:
        handle.close()


def write_csv():
    lines = [",".join([c[0] for c in COLUMNS])]
    for i in range(4):
        cells = []
        for name, storage, values in COLUMNS:
            value = values[i]
            if value is None:
                cells.append("")
            elif storage == "String":
                cells.append('"' + value + '"')
            else:
                cells.append(value)
        lines.append(",".join(cells))
    handle = open(DIR + "/" + CSV_NAME, "wb")
    try:
        handle.write(chr(10).join(lines) + chr(10))
    finally:
        handle.close()
    manifest["saved"].append(CSV_NAME)


def build_case(case_name, ext, props):
    stream = None
    stage = "stream"
    file_name = "%s.str" % case_name
    data_path = "%s/%s.%s" % (DIR, case_name, ext)
    entry = {"case": case_name, "file": file_name, "properties": props, "status": "error",
             "output": data_path}
    try:
        stream = taskrunner.createStream(case_name, False, False)
        stream.setPropertyValue("decimal_places", 15)
        stream.setPropertyValue("date_format", "YYYY-MM-DD")
        src = stream.createAt("variablefile", CSV_NAME, 100, 100)
        src.setPropertyValue("full_filename", DIR + "/" + CSV_NAME)
        src.setPropertyValue("read_field_names", True)
        src.setPropertyValue("delimit_comma", True)
        src.setPropertyValue("quotes_2", "PairAndDiscard")
        src.setPropertyValue("decimal_symbol", "Period")
        for name, storage, values in COLUMNS:
            src.setKeyedPropertyValue("custom_storage_type", name, storage)
        # The export needs instantiated types (otherwise "Field 'id' is unknown or has
        # uninstantiated type"): a Type node read through a temporary Table.
        type_node = stream.createAt("type", "Type", 200, 100)
        stream.link(src, type_node)
        temp = stream.createAt("table", "Instantiate", 200, 250)
        stream.link(type_node, temp)
        temp.run([])
        stream.delete(temp)
        stage = "configure"
        export = stream.createAt("sasexport", case_name, 300, 100)
        stream.link(type_node, export)
        export.setPropertyValue("full_filename", data_path)
        for name, value in props:
            export.setPropertyValue(name, value)
        entry["readback"] = [[name, unicode(export.getPropertyValue(name))] for name, _ in props]
        taskrunner.saveStreamToFile(stream, "%s/%s" % (DIR, file_name))
        manifest["saved"].append(file_name)
        entry["status"] = "saved"
        stage = "run"
        export.run([])
        written = java.io.File(data_path)
        entry["bytes"] = written.length() if written.exists() else None
        entry["status"] = "ran"
    except ERRORS as exc:
        manifest["errors"].append({"case": case_name, "stage": stage, "message": message(exc)})
    finally:
        manifest["cases"].append(entry)
        if stream is not None:
            try:
                stream.clear()
            except ERRORS:
                pass


try:
    java.io.File(DIR).mkdirs()
    write_csv()
    build_case("sasx_sas8", "sas7bdat", [("format", "SAS8"), ("export_names", "NamesAndLabels")])
    build_case("sasx_sas7", "sas7bdat", [("format", "SAS7"), ("export_names", "NamesAndLabels")])
    build_case("sasx_windows", "sd2", [("format", "Windows"), ("export_names", "NamesAndLabels")])
    build_case("sasx_unix", "ssd", [("format", "UNIX"), ("export_names", "NamesAndLabels")])
    build_case("sasx_names_as_labels", "sas7bdat", [("format", "SAS8"),
                                                     ("export_names", "NamesAsLabels")])
except ERRORS as exc:
    manifest["errors"].append({"case": None, "stage": "setup", "message": message(exc)})
finally:
    write_manifest()
