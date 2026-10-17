# Probe: random record nodes (Partition, Sample random %, Balance) on a 1000-record id file in
# corpus/random/, to recover how Modeler draws its random numbers from a seed.
# Each stream is source -> node under test -> Table, with decimal_places = 15.
# Run with `cargo xtask probe random_streams` (IBM SPSS Modeler, Jython 2.7, public API only).
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
RANDOM_DIR = "D:/data-mining-software/corpus/random"
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
    lines = ["id,y"]
    for i in range(1, count + 1):
        y = 0
        if i % 10 == 0:
            y = 1
        lines.append("%d,%d" % (i, y))
    handle = open("%s/%s" % (RANDOM_DIR, file_name), "w")
    try:
        handle.write(NL.join(lines) + NL)
    finally:
        handle.close()
    manifest["saved"].append(file_name)


def build_case(case_name, csv_name, node_type, props):
    stream = None
    try:
        stream = taskrunner.createStream(case_name, False, False)
        stream.setPropertyValue("decimal_places", 15)
        src = stream.createAt("variablefile", csv_name, 100, 100)
        src.setPropertyValue("full_filename", "%s/%s" % (RANDOM_DIR, csv_name))
        src.setPropertyValue("read_field_names", True)
        node = stream.createAt(node_type, case_name, 260, 100)
        stream.link(src, node)
        for name, value in props:
            node.setPropertyValue(name, value)
        table = stream.createAt("table", "Table", 420, 100)
        stream.link(node, table)
        table.run([])
        file_name = "%s.str" % case_name
        taskrunner.saveStreamToFile(stream, "%s/%s" % (RANDOM_DIR, file_name))
        manifest["saved"].append(file_name)
        manifest["cases"].append({"case": case_name, "file": file_name, "data": csv_name,
                                  "node_type": node_type,
                                  "properties": [[n, v] for n, v in props]})
    except ERRORS, e:
        manifest["errors"].append({"case": case_name, "message": message(e)})
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS:
                pass


ensure_clean_dir(RANDOM_DIR)
write_csv("ids1000.csv", 1000)
write_csv("ids10.csv", 10)
write_csv("ids100.csv", 100)

SYSTEM = [("value_mode", "System"), ("set_random_seed", True)]
build_case("part_default", "ids1000.csv", "partition", SYSTEM + [("random_seed", 1234567)])
build_case("part_seed1", "ids1000.csv", "partition", SYSTEM + [("random_seed", 1)])
build_case("part_seed2", "ids1000.csv", "partition", SYSTEM + [("random_seed", 2)])
build_case("part_seed0", "ids1000.csv", "partition", SYSTEM + [("random_seed", 0)])
build_case("part_30_70", "ids1000.csv", "partition",
           SYSTEM + [("random_seed", 1234567), ("training_size", 30), ("testing_size", 70)])
build_case("part_3way", "ids1000.csv", "partition",
           SYSTEM + [("random_seed", 1234567), ("create_validation", True),
                     ("training_size", 40), ("testing_size", 30), ("validation_size", 30)])
build_case("part_n10", "ids10.csv", "partition", SYSTEM + [("random_seed", 1234567)])
build_case("part_n100", "ids100.csv", "partition", SYSTEM + [("random_seed", 1234567)])
build_case("sample_rand50", "ids1000.csv", "sample",
           [("sample_type", "RandomPct"), ("rand_pct", 50.0), ("set_random_seed", True),
            ("random_seed", 1234567)])
build_case("sample_rand20_seed1", "ids1000.csv", "sample",
           [("sample_type", "RandomPct"), ("rand_pct", 20.0), ("set_random_seed", True),
            ("random_seed", 1)])
build_case("balance_boost", "ids1000.csv", "balance",
           [("directives", [[2.5, "y = 1"]])])
build_case("balance_reduce", "ids1000.csv", "balance",
           [("directives", [[0.3, "y = 0"]])])

handle = open(OUT_DIR + "/manifest.json", "w")
try:
    json.dump(manifest, handle, indent=2, sort_keys=True)
finally:
    handle.close()
