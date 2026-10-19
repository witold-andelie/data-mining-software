# Probe: client-run copies of the Excel source streams (label excel_gui).
# Run with `cargo xtask probe excel_gui_streams` (IBM SPSS Modeler, Jython 2.7, public API only).
# Modeler 19 batch cannot run the Excel source (E3341 "Insufficient memory for JVM": the batch
# process already hosts a JVM), so corpus/sources/excel_*.str have no oracle. This probe opens
# each of them, links its Excel source to a Flat File export (Table 252 outputfile: comma,
# double quotes, field names) writing corpus/excel_gui/out/<case>.csv, and saves the stream as
# corpus/excel_gui/<case>.str. The exports come from running those streams in the Modeler
# client; nothing here runs them.
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
SOURCE_DIR = "D:/data-mining-software/corpus/sources"
DIR = "D:/data-mining-software/corpus/excel_gui"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "excel_gui", "saved": [], "errors": [], "cases": []}


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


def convert(file_name):
    case_name = file_name[:-4]
    entry = {"case": case_name, "file": file_name, "status": "error"}
    stream = None
    try:
        stream = taskrunner.openStreamFromFile(SOURCE_DIR + "/" + file_name, False)
        sources = []
        iterator = stream.iterator()
        while iterator.hasNext():
            node = iterator.next()
            if unicode(node.getTypeName()) == "excelimport":
                sources.append(node)
        if len(sources) != 1:
            raise RuntimeError("expected one Excel source, found %d" % len(sources))
        export = stream.createAt("outputfile", "Export", 400, 260)
        export.setPropertyValue("full_filename", "%s/out/%s.csv" % (DIR, case_name))
        export.setPropertyValue("write_mode", "Overwrite")
        export.setPropertyValue("inc_field_names", True)
        export.setPropertyValue("quote_mode", "Double")
        stream.link(sources[0], export)
        taskrunner.saveStreamToFile(stream, "%s/%s" % (DIR, file_name))
        manifest["saved"].append(file_name)
        entry["status"] = "saved"
    except ERRORS as exc:
        manifest["errors"].append({"case": case_name, "message": message(exc)})
    finally:
        manifest["cases"].append(entry)
        if stream is not None:
            try:
                stream.close()
            except ERRORS:
                pass


try:
    java.io.File(DIR + "/out").mkdirs()
    names = sorted([unicode(f.getName()) for f in java.io.File(SOURCE_DIR).listFiles()
                    if unicode(f.getName()).startswith("excel_") and unicode(f.getName()).endswith(".str")])
    for name in names:
        convert(name)
except ERRORS as exc:
    manifest["errors"].append({"case": None, "message": message(exc)})
finally:
    write_manifest()
