# Probe: which scripting names switch the Modeler 19 Auto Classifier / Auto Numeric component
# algorithms on and off (the guide lists only the pre-19 names). No training: each candidate
# name is set to False on a fresh node, the read-back or error is recorded, and the node is
# saved so the stream's ensembleAlgorithms list shows which algorithm the name disabled.
# Coordinator only: cargo xtask probe auto_names_streams (Modeler 19, Jython 2.7).
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/auto_names"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "auto_names", "saved": [], "errors": [], "cases": []}

CANDIDATES = ["cart", "chaid", "quest", "c50", "logreg", "decisionlist", "bayesnet",
              "discriminant", "svm", "knn", "neuralnetwork", "genlin", "regression", "linear",
              "randomtrees", "random_trees", "rf", "lsvm", "linearsvm", "linear_svm", "treeas",
              "tree_as", "linearas", "linear_as", "xgboosttree", "xgboost_tree",
              "xgboostlinear", "xgboost_linear", "xgboostas", "xgboost_as", "gle", "gle_builder"]


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
    if path != DIR:
        raise ValueError("unexpected corpus directory: " + path)
    directory = java.io.File(path)
    if not directory.exists() and not directory.mkdirs():
        raise IOError("cannot create " + path)


def probe(family):
    entry = {"case": family + "_names", "family": family, "file": family + "_names.str",
             "names": {}}
    manifest["cases"].append(entry)
    stream = taskrunner.createStream(entry["case"], False, False)
    try:
        node = stream.createAt(family, family, 200, 100)
        for name in CANDIDATES:
            result = {}
            try:
                result["default"] = unicode(node.getPropertyValue(name))
            except ERRORS as exc:
                result["read_error"] = message(exc)
            try:
                node.setPropertyValue(name, False)
                result["after"] = unicode(node.getPropertyValue(name))
            except ERRORS as exc:
                result["set_error"] = message(exc)
            entry["names"][name] = result
        taskrunner.saveStreamToFile(stream, DIR + "/" + entry["file"])
        manifest["saved"].append(entry["file"])
    except ERRORS as exc:
        manifest["errors"].append({"case": entry["case"], "message": message(exc)})
    finally:
        stream.clear()
        write_manifest()


try:
    ensure_dir(DIR)
    for family in ["autoclassifier", "autonumeric"]:
        probe(family)
except ERRORS as exc:
    manifest["errors"].append({"case": None, "message": message(exc)})
finally:
    write_manifest()
