# Probe: re-save XML source streams that carry a dialog-style scanned structure (label xml_scan).
# Coordinator only: cargo xtask probe xml_scan_streams (Modeler 19, Jython 2.7, public API only).
# Before running: `py -I xtask/oracle/xml_structure.py <dms-cli.exe> D:/spss-modeler/rea-work/xml-stage
# corpus/sources/xml_*.str` (all but xml_template.str) writes copies of the sources-label XML
# streams with the structure the XML source dialog would scan (see that script). This probe opens
# each copy, which Modeler accepts with plain-text stream XML, and saves it through Modeler as
# corpus/xml_scan/<case>.str, so the corpus holds ordinary Modeler-written streams. Nothing runs.
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
STAGE = "D:/spss-modeler/rea-work/xml-stage"
DIR = "D:/data-mining-software/corpus/xml_scan"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "xml_scan", "saved": [], "errors": [], "cases": []}


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


try:
    directory = java.io.File(DIR)
    if not directory.exists() and not directory.mkdirs():
        raise IOError("cannot create " + DIR)
    names = sorted([unicode(f.getName()) for f in java.io.File(STAGE).listFiles()
                    if unicode(f.getName()).endswith(".str")])
    for name in names:
        entry = {"case": name[:-4], "file": name, "status": "planned"}
        manifest["cases"].append(entry)
        stream = None
        try:
            stream = taskrunner.openStreamFromFile(STAGE + "/" + name, True)
            taskrunner.saveStreamToFile(stream, DIR + "/" + name)
            manifest["saved"].append(name)
            entry["status"] = "saved"
        except ERRORS as exc:
            entry["status"] = "failed"
            manifest["errors"].append({"case": entry["case"], "message": message(exc)})
        finally:
            if stream is not None:
                stream.close()
            write_manifest()
except ERRORS as exc:
    manifest["errors"].append({"case": None, "message": message(exc)})
finally:
    write_manifest()
