# Probe: SAS File source on real SAS files (label sas).
# Run with `cargo xtask probe sas_streams` (IBM SPSS Modeler, Jython 2.7, public API only).
# corpus/sas/sastest.sas7bdat and sastest.xpt were written by SAS OnDemand for Academics
# (SAS Studio, 2026-10-19; DATA step `work.sastest`, 5 rows: numeric id,
# amount (format 10.2, label), joined (date9., label), score; character name $12 (label), grp $1;
# missing values in amount, joined, score). The misc_next probe only had Modeler's own
# (empty) SAS exports to read. Property names: Modeler 19.0 Python Scripting and Automation
# Guide, Table 54 (sasimportnode). Each stream is SAS File -> Flat File (Table 252
# outputfile: comma, double quotes, field names) writing corpus/sas/gui/<case>.csv.
# "The input encoder was not open" (-12) came from Windows' UTF-8 system code page (ACP 65001,
# "Beta: Use Unicode UTF-8"); with the zh-CN code page 936 batch reads both files (2026-10-19).
# The streams are saved before running, so `cargo xtask oracle sas <case>.str` exports them.
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/sas"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "sas", "saved": [], "errors": [], "cases": []}


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


def build_case(case_name, data_file, props):
    stream = None
    stage = "stream"
    file_name = "%s.str" % case_name
    entry = {"case": case_name, "file": file_name, "data": data_file, "properties": props,
             "status": "error"}
    try:
        stream = taskrunner.createStream(case_name, False, False)
        stream.setPropertyValue("decimal_places", 15)
        stage = "configure"
        source = stream.createAt("sasimport", case_name, 100, 100)
        source.setPropertyValue("full_filename", DIR + "/" + data_file)
        for name, value in props:
            source.setPropertyValue(name, value)
        entry["readback"] = [[name, unicode(source.getPropertyValue(name))] for name, _ in props]
        java.io.File(DIR + "/gui").mkdirs()
        export = stream.createAt("outputfile", "Export", 300, 100)
        export.setPropertyValue("full_filename", "%s/gui/%s.csv" % (DIR, case_name))
        export.setPropertyValue("write_mode", "Overwrite")
        export.setPropertyValue("inc_field_names", True)
        export.setPropertyValue("quote_mode", "Double")
        stream.link(source, export)
        taskrunner.saveStreamToFile(stream, "%s/%s" % (DIR, file_name))
        manifest["saved"].append(file_name)
        entry["status"] = "saved"
        stage = "run"
        export.run([])
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
    build_case("sas9_names", "sastest.sas7bdat",
               [("format", "SAS9"), ("read_formats", False), ("import_names", "NamesAndLabels")])
    build_case("sas9_labels", "sastest.sas7bdat",
               [("format", "SAS9"), ("read_formats", False), ("import_names", "LabelsasNames")])
    build_case("sas_transport", "sastest.xpt",
               [("format", "Transport"), ("member_name", "SASTEST"), ("read_formats", False),
                ("import_names", "NamesAndLabels")])
except ERRORS as exc:
    manifest["errors"].append({"case": None, "stage": "setup", "message": message(exc)})
finally:
    write_manifest()
