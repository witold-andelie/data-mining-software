# Probe: more overfit-prevention-set masks for identifying C&R Tree's generator (follow-up of
# cart_ofs_streams.py; proposals in the ofs-generator report, 2026-10-18). Same construction:
# a regression tree of y = id on x = id grown to one record per leaf lists the growing ids.
# Files in corpus/cart_ofs2/. Run with `cargo xtask probe cart_ofs2_streams`.
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/cart_ofs2"
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


def write_csv(file_name, ids):
    lines = ["x,y"]
    for i in ids:
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
write_csv("ids200.csv", range(1, 201))
write_csv("ids201.csv", range(1, 202))
write_csv("ids400.csv", range(1, 401))
write_csv("ids48.csv", range(1, 49))
write_csv("rev200.csv", range(200, 0, -1))
write_csv("evenodd200.csv", list(range(2, 201, 2)) + list(range(1, 200, 2)))

OFS = [("prune_tree", False), ("set_random_seed", True)]
# 1. Draw intervals: seed 1, N = 200, every 5% except the recorded 30 and 50.
for pct in range(5, 100, 5):
    if pct in (30, 50):
        continue
    build_case("ofs_s1_pct%02d" % pct, "ids200.csv", OFS + [("train_pct", pct), ("seed", 1)])
# 2. Seed initialization, 30%.
for seed in [0, 3, 4, 5, 7, 16, 4357, 5489, 12345, 2147483647]:
    build_case("ofs_s%d_pct30_n200" % seed, "ids200.csv",
               OFS + [("train_pct", 30), ("seed", seed)])
# 3. Input position versus identity, and length.
build_case("ofs_s1_rev200_pct30", "rev200.csv", OFS + [("train_pct", 30), ("seed", 1)])
build_case("ofs_s1_evenodd200_pct30", "evenodd200.csv", OFS + [("train_pct", 30), ("seed", 1)])
build_case("ofs_s1_n201_pct30", "ids201.csv", OFS + [("train_pct", 30), ("seed", 1)])
build_case("ofs_s1_n400_pct30", "ids400.csv", OFS + [("train_pct", 30), ("seed", 1)])
# 4. The QUEST oracles' seed and size: 48 records, seed 12345, 30%.
build_case("ofs_cart_s12345_reg48_pct30", "ids48.csv", OFS + [("train_pct", 30), ("seed", 12345)])

handle = open(OUT_DIR + "/manifest.json", "w")
try:
    json.dump(manifest, handle, indent=2, sort_keys=True)
finally:
    handle.close()
