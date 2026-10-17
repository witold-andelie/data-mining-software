# Probe: which records C&R Tree holds out as its overfit prevention set. A regression tree of
# y = id on x = id, grown until every leaf holds one record, lists the growing records in its
# leaves. Files in corpus/cart_ofs/. Each stream is source -> Type -> C&R Tree build node.
# Run with `cargo xtask probe cart_ofs_streams` (IBM SPSS Modeler, Jython 2.7, public API only).
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/cart_ofs"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"saved": [], "errors": [], "cases": []}


def message(exc):
    get = getattr(exc, "getMessage", None)
    if get is not None:
        return unicode(get())
    return unicode(exc)


def ensure_clean_dir(path):
    d = java.io.File(path)
    if d.exists():
        children = d.listFiles()
        if children is not None:
            for child in children:
                if child.isFile():
                    child.delete()
    else:
        d.mkdirs()


def write_csv(file_name, count):
    lines = ["x,y"]
    for i in range(1, count + 1):
        lines.append("%d,%d" % (i, i))
    handle = open("%s/%s" % (DIR, file_name), "w")
    try:
        handle.write(NL.join(lines) + NL)
    finally:
        handle.close()
    manifest["saved"].append(file_name)


def instantiate_node(stream, node):
    table = stream.createAt("table", "TempTable", 760, 360)
    stream.link(node, table)
    try:
        table.run([])
    finally:
        stream.delete(table)


def build_case(case_name, csv_name, props):
    stream = None
    try:
        stream = taskrunner.createStream(case_name, False, False)
        stream.setPropertyValue("decimal_places", 15)
        src = stream.createAt("variablefile", csv_name, 100, 100)
        src.setPropertyValue("full_filename", "%s/%s" % (DIR, csv_name))
        src.setPropertyValue("read_field_names", True)
        type_node = stream.createAt("type", "Type", 260, 100)
        stream.link(src, type_node)
        instantiate_node(stream, type_node)
        type_node.setKeyedPropertyValue("type", "x", "Range")
        type_node.setKeyedPropertyValue("type", "y", "Range")
        type_node.setKeyedPropertyValue("direction", "x", "Input")
        type_node.setKeyedPropertyValue("direction", "y", "Target")
        instantiate_node(stream, type_node)
        node = stream.createAt("cart", case_name, 460, 100)
        stream.link(type_node, node)
        node.setPropertyValue("use_model_name", True)
        node.setPropertyValue("model_name", case_name)
        node.setPropertyValue("use_max_depth", "Custom")
        node.setPropertyValue("max_depth", 30)
        node.setPropertyValue("use_percentage", False)
        node.setPropertyValue("min_parent_records_abs", 2)
        node.setPropertyValue("min_child_records_abs", 1)
        node.setPropertyValue("min_impurity", 0.0)
        for name, value in props:
            node.setPropertyValue(name, value)
        results = []
        node.run(results)
        file_name = "%s.str" % case_name
        taskrunner.saveStreamToFile(stream, "%s/%s" % (DIR, file_name))
        manifest["saved"].append(file_name)
        manifest["cases"].append({"case": case_name, "file": file_name, "data": csv_name,
                                  "properties": [[n, v] for n, v in props]})
    except ERRORS, e:
        manifest["errors"].append({"case": case_name, "message": message(e)})
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS:
                pass


ensure_clean_dir(DIR)
write_csv("ids200.csv", 200)
write_csv("ids50.csv", 50)

OFS = [("prune_tree", False), ("set_random_seed", True)]
build_case("ofs_s1", "ids200.csv", OFS + [("train_pct", 30), ("seed", 1)])
build_case("ofs_s2", "ids200.csv", OFS + [("train_pct", 30), ("seed", 2)])
build_case("ofs_s54752075", "ids200.csv", OFS + [("train_pct", 30), ("seed", 54752075)])
build_case("ofs_s1_pct50", "ids200.csv", OFS + [("train_pct", 50), ("seed", 1)])
build_case("ofs_s1_n50", "ids50.csv", OFS + [("train_pct", 30), ("seed", 1)])
build_case("ofs_s1_pruned", "ids200.csv",
           [("prune_tree", True), ("set_random_seed", True), ("train_pct", 30), ("seed", 1)])

handle = open(OUT_DIR + "/manifest.json", "w")
try:
    json.dump(manifest, handle, indent=2, sort_keys=True)
finally:
    handle.close()
