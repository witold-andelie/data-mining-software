# Probe: Database source node on an ODBC data source (label database).
# Run with `cargo xtask probe database_streams` (IBM SPSS Modeler, Jython 2.7, public API only).
# The user DSN `dms_access` (Microsoft Access Driver, 64-bit) points at
# D:/spss-modeler/rea-work/odbc/dms_test.accdb, table `customers` (5 rows: integer id, text name
# and grp, double amount and score, datetime joined; NULLs in amount, joined, score; an empty
# name). Property names: Modeler 19.0 Python Scripting and Automation Guide, Table 46
# (databasenode) and Table 246 (databaseexportnode). Import streams are Database -> Table.
# db_export writes export.csv (Variable File -> Database export) to a new table `exp_create` (Create, drop
# existing); db_export_readback reads that table back. Run the db_export oracle before the
# readback oracle: the table is what the latest export wrote.
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/database"
DSN = "dms_access"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "database", "dsn": DSN, "saved": [], "errors": [], "cases": []}


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


def ensure_dir(path):
    directory = java.io.File(path)
    if not directory.exists() and not directory.mkdirs():
        raise IOError("cannot create " + path)


CSV_NAME = "export.csv"
# name, storage, cells (None = empty cell)
EXPORT_COLUMNS = [
    ("id", "Integer", ["1", "2", "3", "4"]),
    ("name", "String", ["Alice", "Bob", "", "Dan"]),
    ("amount", "Real", ["12.5", None, "-7.25", "1000"]),
    ("joined", "Date", ["2020-01-01", "2021-03-15", None, "2019-12-31"]),
    ("stamp", "Timestamp", ["2020-01-01 08:30:00", None, "2021-06-30 23:59:59", "2019-12-31 00:00:00"]),
]


def write_export_csv():
    lines = [",".join([c[0] for c in EXPORT_COLUMNS])]
    for i in range(4):
        cells = []
        for name, storage, values in EXPORT_COLUMNS:
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


def build_export(case_name, props):
    stream = None
    stage = "stream"
    file_name = "%s.str" % case_name
    entry = {"case": case_name, "file": file_name, "properties": props, "status": "error"}
    try:
        stream = taskrunner.createStream(case_name, False, False)
        stream.setPropertyValue("decimal_places", 15)
        stream.setPropertyValue("date_format", "YYYY-MM-DD")
        stream.setPropertyValue("time_format", "HH:MM:SS")
        src = stream.createAt("variablefile", CSV_NAME, 100, 100)
        src.setPropertyValue("full_filename", DIR + "/" + CSV_NAME)
        src.setPropertyValue("read_field_names", True)
        src.setPropertyValue("delimit_comma", True)
        src.setPropertyValue("quotes_2", "PairAndDiscard")
        src.setPropertyValue("decimal_symbol", "Period")
        for name, storage, values in EXPORT_COLUMNS:
            src.setKeyedPropertyValue("custom_storage_type", name, storage)
        stage = "configure"
        export = stream.createAt("databaseexport", case_name, 300, 100)
        stream.link(src, export)
        for name, value in props:
            export.setPropertyValue(name, value)
        entry["readback"] = [[name, unicode(export.getPropertyValue(name))] for name, _ in props]
        stage = "run"
        export.run([])
        taskrunner.saveStreamToFile(stream, "%s/%s" % (DIR, file_name))
        manifest["saved"].append(file_name)
        entry["status"] = "saved"
    except ERRORS as exc:
        manifest["errors"].append({"case": case_name, "stage": stage, "message": message(exc)})
    finally:
        manifest["cases"].append(entry)
        if stream is not None:
            try:
                stream.clear()
            except ERRORS:
                pass


def build_case(case_name, props):
    stream = None
    stage = "stream"
    file_name = "%s.str" % case_name
    entry = {"case": case_name, "file": file_name, "properties": props, "status": "error"}
    try:
        stream = taskrunner.createStream(case_name, False, False)
        stream.setPropertyValue("decimal_places", 15)
        stage = "configure"
        source = stream.createAt("database", case_name, 100, 100)
        for name, value in props:
            source.setPropertyValue(name, value)
        entry["readback"] = [[name, unicode(source.getPropertyValue(name))] for name, _ in props]
        table = stream.createAt("table", "Table", 300, 100)
        stream.link(source, table)
        stage = "run"
        table.run([])
        taskrunner.saveStreamToFile(stream, "%s/%s" % (DIR, file_name))
        manifest["saved"].append(file_name)
        entry["status"] = "saved"
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
    ensure_dir(DIR)
    build_case("db_table", [("datasource", DSN), ("mode", "Table"), ("tablename", "customers")])
    build_case("db_query", [("datasource", DSN), ("mode", "Query"),
                            ("query", "SELECT id, name, amount FROM customers WHERE id > 1")])
    write_export_csv()
    build_export("db_export", [("datasource", DSN), ("table_name", "exp_create"),
                               ("write_mode", "Create"), ("drop_existing_table", True)])
    build_case("db_export_readback", [("datasource", DSN), ("mode", "Table"),
                                      ("tablename", "exp_create")])
except ERRORS as exc:
    manifest["errors"].append({"case": None, "stage": "setup", "message": message(exc)})
finally:
    write_manifest()
