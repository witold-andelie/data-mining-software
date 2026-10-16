# Probe: .sav -> CSV oracle. For every .sav in the team and IBM demo corpora, read it with a
# Statistics File node, export it with a Flat File node (UTF-8, comma, double quotes, header,
# period decimal) and dump the source node's output data model as <name>.fields.json.
# Run with `cargo xtask probe sav_to_csv` (IBM SPSS Modeler, Jython 2.7, public scripting API only).
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
ROOTS = ["D:/data-mining-software/corpus/team", "D:/data-mining-software/corpus/ibm-demo-data"]
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
MAX_VALUES = 200
# The stream default (3) rounds real numbers written by the Flat File export; raise it.
DECIMAL_PLACES = 30

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"saved": [], "files": [], "errors": [], "source_defaults": {}}


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
    return unicode(value)


def safe(call):
    try:
        return text(call())
    except ERRORS, e:
        return None


def column_info(c):
    info = {
        "name": text(c.getColumnName()),
        "label": safe(c.getColumnLabel),
        "storage": safe(c.getStorageType),
        "measure": safe(c.getMeasureType),
        "role": safe(c.getModelingRole),
        "discrete": safe(c.isMeasureDiscrete),
        "lower": safe(c.getLowerBound),
        "upper": safe(c.getUpperBound),
        "true_flag": safe(c.getTrueFlag),
        "false_flag": safe(c.getFalseFlag),
    }
    try:
        values = c.getSetValues()
        if values is None:
            info["set_values"] = None
        else:
            info["set_value_count"] = len(values)
            listed = []
            labels = {}
            for v in values[:MAX_VALUES]:
                listed.append(text(v))
                lab = c.getValueLabel(v)
                if lab:
                    labels[unicode(v)] = text(lab)
            info["set_values"] = listed
            info["value_labels"] = labels
    except ERRORS, e:
        info["set_values_error"] = message(e)
    return info


def sav_files():
    found = []
    for root in ROOTS:
        children = java.io.File(root).listFiles()
        if children is None:
            continue
        for child in children:
            if child.isFile() and child.getName().lower().endswith(".sav"):
                found.append((root, child.getName(), child.getPath().replace("\\", "/")))
    found.sort(key=lambda t: (t[1].lower(), t[0]))
    return found


def convert(root, name, path):
    base = name[:-4]
    csv_path = "%s/%s.csv" % (OUT_DIR, base)
    entry = {"source": path, "name": name, "csv": base + ".csv"}
    stream = taskrunner.createStream("sav_" + base, False, False)
    if "stream_decimal_places_default" not in manifest:
        manifest["stream_decimal_places_default"] = text(stream.getPropertyValue("decimal_places"))
    stream.setPropertyValue("decimal_places", DECIMAL_PLACES)
    source = stream.createAt("statisticsimport", "SAV", 100, 100)
    source.setPropertyValue("full_filename", path)
    if not manifest["source_defaults"]:
        for key in ("import_names", "import_data", "use_field_format_for_storage", "file_encrypted"):
            try:
                manifest["source_defaults"][key] = text(source.getPropertyValue(key))
            except ERRORS, e:
                manifest["source_defaults"][key] = "ERROR: " + message(e)
    export = stream.createAt("outputfile", "CSV", 300, 100)
    export.setPropertyValue("full_filename", csv_path)
    export.setPropertyValue("write_mode", "Overwrite")
    export.setPropertyValue("inc_field_names", True)
    export.setPropertyValue("use_newline_after_records", True)
    export.setPropertyValue("delimit_mode", "Comma")
    export.setPropertyValue("quote_mode", "Double")
    export.setPropertyValue("decimal_symbol", "Period")
    export.setPropertyValue("encoding", "UTF-8")
    export.setPropertyValue("generate_import", False)
    stream.link(source, export)
    export.run([])
    model = source.getOutputDataModel()
    columns = []
    it = model.columnIterator()
    while it.hasNext():
        columns.append(column_info(it.next()))
    fields = {"source": path, "columns": columns}
    handle = open("%s/%s.fields.json" % (OUT_DIR, base), "w")
    try:
        json.dump(fields, handle, indent=1)
    finally:
        handle.close()
    entry["columns"] = len(columns)
    manifest["saved"].append(base + ".csv")
    manifest["saved"].append(base + ".fields.json")
    manifest["files"].append(entry)


for root, name, path in sav_files():
    try:
        convert(root, name, path)
    except ERRORS, e:
        manifest["errors"].append({"file": name, "message": message(e)})

handle = open(OUT_DIR + "/manifest.json", "w")
try:
    json.dump(manifest, handle, indent=2, sort_keys=True)
finally:
    handle.close()
