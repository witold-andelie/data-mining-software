# Probe: build TwoStep training streams on subsets and orderings of DRUG1n in corpus/twostep/,
# to settle how the CF tree orders sub-clusters (cluster names, category order) and the
# settings that change the result. Each stream is source -> Type -> twostep build node, and
# Type -> nugget -> Table, with decimal_places = 15.
# Run with `cargo xtask probe twostep_streams` (IBM SPSS Modeler, Jython 2.7, public API only).
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
TWOSTEP_DIR = "D:/data-mining-software/corpus/twostep"
DRUG = "D:/data-mining-software/corpus/models/DRUG1n"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"saved": [], "errors": [], "cases": []}

SCHEMA = [
    ("Age", "Range"),
    ("Sex", "Flag"),
    ("BP", "Set"),
    ("Cholesterol", "Flag"),
    ("Na", "Range"),
    ("K", "Range"),
    ("Drug", "Set"),
]
ALL_INPUTS = ["Age", "Sex", "BP", "Cholesterol", "Na", "K"]
CONT_INPUTS = ["Age", "Na", "K"]


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


def read_drug():
    handle = open(DRUG, "r")
    try:
        lines = [line.rstrip("\r\n") for line in handle.readlines()]
    finally:
        handle.close()
    lines = [line for line in lines if line.strip()]
    return lines[0], lines[1:]


def write_csv(file_name, header, rows):
    handle = open("%s/%s" % (TWOSTEP_DIR, file_name), "w")
    try:
        handle.write(NL.join([header] + rows) + NL)
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


def find_applier(stream):
    it = stream.iterator()
    while it.hasNext():
        n = it.next()
        if unicode(n.getTypeName()).startswith("apply"):
            return n
    return None


def build_case(case_name, csv_name, inputs, props):
    stream = None
    try:
        stream = taskrunner.createStream(case_name, False, False)
        stream.setPropertyValue("decimal_places", 15)
        stream.setPropertyValue("create_model_applier_for_new_models", True)
        stream.setPropertyValue("create_model_applier_update_links", "createEnabled")
        src = stream.createAt("variablefile", csv_name, 100, 100)
        src.setPropertyValue("full_filename", "%s/%s" % (TWOSTEP_DIR, csv_name))
        src.setPropertyValue("read_field_names", True)
        type_node = stream.createAt("type", "Type", 260, 100)
        stream.link(src, type_node)
        instantiate_node(stream, type_node)
        for field_name, measure in SCHEMA:
            type_node.setKeyedPropertyValue("type", field_name, measure)
            direction = "Input"
            if field_name not in inputs:
                direction = "None"
            type_node.setKeyedPropertyValue("direction", field_name, direction)
        instantiate_node(stream, type_node)

        build_node = stream.createAt("twostep", case_name, 460, 100)
        stream.link(type_node, build_node)
        build_node.setPropertyValue("use_model_name", True)
        build_node.setPropertyValue("model_name", case_name)
        build_node.setPropertyValue("custom_fields", True)
        build_node.setPropertyValue("inputs", list(inputs))
        for name, value in props:
            build_node.setPropertyValue(name, value)
        results = []
        build_node.run(results)

        applier = find_applier(stream)
        if applier is None:
            applier = stream.createModelApplierAt(results[0], case_name, 460, 240)
            stream.link(type_node, applier)
        else:
            applier.setXYPosition(460, 240)
        table_node = stream.createAt("table", "Table", 640, 240)
        stream.link(applier, table_node)
        table_node.run([])

        file_name = "%s.str" % case_name
        taskrunner.saveStreamToFile(stream, "%s/%s" % (TWOSTEP_DIR, file_name))
        manifest["saved"].append(file_name)
        manifest["cases"].append({
            "case": case_name,
            "file": file_name,
            "data": csv_name,
            "inputs": list(inputs),
            "properties": [[name, value] for name, value in props],
        })
    except ERRORS, e:
        manifest["errors"].append({"case": case_name, "message": message(e)})
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS:
                pass


ensure_clean_dir(TWOSTEP_DIR)
header, records = read_drug()
reversed_records = list(records)
reversed_records.reverse()
first8 = records[:8]
first8_reversed = list(first8)
first8_reversed.reverse()
write_csv("drug8.csv", header, first8)
write_csv("drug8_rev.csv", header, first8_reversed)
write_csv("drug9.csv", header, records[:9])
write_csv("drug20.csv", header, records[:20])
write_csv("drug50.csv", header, records[:50])
write_csv("drug_rev.csv", header, reversed_records)
write_csv("drug.csv", header, records)

FIXED3 = [("cluster_num_auto", False), ("num_clusters", 3)]
AUTO = [("cluster_num_auto", True)]

# Sub-cluster order: one leaf (8 records), the same reversed, one split (9), more splits.
build_case("ts_drug8", "drug8.csv", ALL_INPUTS, FIXED3)
build_case("ts_drug8_rev", "drug8_rev.csv", ALL_INPUTS, FIXED3)
build_case("ts_drug9", "drug9.csv", ALL_INPUTS, FIXED3)
build_case("ts_drug20", "drug20.csv", ALL_INPUTS, [("cluster_num_auto", False), ("num_clusters", 4)])
build_case("ts_drug50", "drug50.csv", ALL_INPUTS, AUTO)
build_case("ts_drug_rev", "drug_rev.csv", ALL_INPUTS, AUTO)
build_case("ts_drug8_cont", "drug8.csv", CONT_INPUTS, FIXED3)
build_case("ts_drug_cont", "drug.csv", CONT_INPUTS, AUTO)
# Prefixes between one root split and the full data, to find where the leaf order changes.
for size in [60, 70, 80, 100, 130, 160]:
    write_csv("drug%d.csv" % size, header, records[:size])
    build_case("ts_drug%d" % size, "drug%d.csv" % size, ALL_INPUTS,
               [("cluster_num_auto", False), ("num_clusters", 6)])
# As many clusters as records: every sub-cluster is its own cluster, so the cluster names give
# the CF tree's leaf order directly.
for size in [20, 60, 100]:
    build_case("ts_leaves%d" % size, "drug%d.csv" % size, ALL_INPUTS,
               [("cluster_num_auto", False), ("num_clusters", size)])
build_case("ts_leaves200", "drug.csv", ALL_INPUTS, [("cluster_num_auto", False), ("num_clusters", 200)])
# Every prefix from 9 to 19 records, one cluster per record: the leaf order after each insertion.
for size in range(3, 20):
    write_csv("prefix%d.csv" % size, header, records[:size])
    build_case("ts_prefix%d" % size, "prefix%d.csv" % size, ALL_INPUTS,
               [("cluster_num_auto", False), ("num_clusters", size)])
# One continuous field (Age) only, every record its own cluster.
for size in [8, 9, 12, 16, 17]:
    build_case("ts_age%d" % size, "prefix%d.csv" % size, ["Age"],
               [("cluster_num_auto", False), ("num_clusters", size)])
# Settings on the full data.
build_case("ts_fixed10", "drug.csv", ALL_INPUTS, [("cluster_num_auto", False), ("num_clusters", 10)])
build_case("ts_range3_6", "drug.csv", ALL_INPUTS, [("cluster_num_auto", True), ("min_num_clusters", 3), ("max_num_clusters", 6)])
build_case("ts_aic", "drug.csv", ALL_INPUTS, [("clustering_criterion", "AIC")])
build_case("ts_nostd", "drug.csv", ALL_INPUTS, [("standardize", False)])
build_case("ts_label_number", "drug.csv", ALL_INPUTS, [("cluster_label", "Number")])
build_case("ts_label_prefix", "drug.csv", ALL_INPUTS, [("label_prefix", "TS_")])

handle = open(OUT_DIR + "/manifest.json", "w")
try:
    json.dump(manifest, handle, indent=2, sort_keys=True)
finally:
    handle.close()
