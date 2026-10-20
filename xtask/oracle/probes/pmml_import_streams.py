# Probe: score an imported PMML model (label pmml_import).
# Coordinator only: cargo xtask probe pmml_import_streams (Modeler 19, Jython 2.7, public API only).
# Scripting cannot import a PMML file as a model (taskrunner.openModelFromFile reads .gm only), so
# the model node comes from the user's dialog-built corpus/misc_next/pmml_template.str: File >
# Models > Import PMML of oracle/cart_ofs2/ofs_s1_pct95/models/01_ofs_s1_pct95.xml (a C&R Tree
# regression of y on x), dragged into a stream (ApplyImportedNode, model in data/0001.dat). This
# probe opens that stream, links a source of corpus/cart_ofs2/ids200.csv (x stored as real, as
# the model's input) and a Table, and saves it as corpus/pmml_import/<case>.str.
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/pmml_import"
TEMPLATE = "D:/data-mining-software/corpus/misc_next/pmml_template.str"
DATA = "D:/data-mining-software/corpus/cart_ofs2/ids200.csv"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "pmml_import", "saved": [], "errors": [], "cases": []}


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


def build(case_name, template, data, storages):
    entry = {"case": case_name, "file": case_name + ".str", "template": template, "data": data,
             "storages": storages, "status": "planned"}
    manifest["cases"].append(entry)
    stream = None
    try:
        stream = taskrunner.openStreamFromFile(template, True)
        nodes = []
        iterator = stream.iterator()
        while iterator.hasNext():
            nodes.append(iterator.next())
        entry["template_nodes"] = [unicode(n.getTypeName()) for n in nodes]
        if len(nodes) != 1:
            raise RuntimeError("expected the imported model node alone in the template")
        model = nodes[0]
        source = stream.createAt("variablefile", data.split("/")[-1], 100, 100)
        source.setPropertyValue("full_filename", data)
        source.setPropertyValue("read_field_names", True)
        for field, storage in storages:
            source.setKeyedPropertyValue("custom_storage_type", field, storage)
        model.setXYPosition(300, 100)
        table = stream.createAt("table", "Scores", 500, 100)
        stream.link(source, model)
        stream.link(model, table)
        taskrunner.saveStreamToFile(stream, DIR + "/" + entry["file"])
        manifest["saved"].append(entry["file"])
        entry["status"] = "saved"
    except ERRORS as exc:
        entry["status"] = "failed"
        manifest["errors"].append({"case": case_name, "message": message(exc)})
    finally:
        if stream is not None:
            stream.close()
        write_manifest()


try:
    directory = java.io.File(DIR)
    if not directory.exists() and not directory.mkdirs():
        raise IOError("cannot create " + DIR)
    build("pmml_cart_regression", TEMPLATE, DATA, [("x", "Real"), ("y", "Real")])
    # Swapping another PMML into the template's attachment is not enough: Modeler keeps the
    # node's saved inputs and outputs (tried 2026-10-20 with models_next/quest_default's
    # classification tree: every record scored $X-y = 0.0), so each model type needs its own
    # dialog-imported template.
except ERRORS as exc:
    manifest["errors"].append({"case": None, "message": message(exc)})
finally:
    write_manifest()
