# Probe: Output node settings the outputs label does not cover (label outputs_next): Matrix flag
# and numeric matrices, Means between paired fields, Evaluation profit and ROI charts.
# Run with `cargo xtask probe output_next_streams` (IBM SPSS Modeler, Jython 2.7, public API
# only), then corpus record outputs_next and oracle. Framework as output_streams.py.
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
OUTPUTS_DIR = "D:/data-mining-software/corpus/outputs_next"
CSV_NAME = "out_next.csv"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"saved": [], "errors": [], "cases": []}


def rows():
    # 40 records: flags f1..f3, small-integer numerics n1/n2 (3 and 4 levels), a real n3, paired
    # before/after measurements (two pairs), and a scored flag target resp with $R-resp/$RC-resp.
    header = "id,f1,f2,f3,n1,n2,n3,before1,after1,before2,after2,resp,$R-resp,$RC-resp"
    lines = [header]
    for i in range(40):
        f1 = "T" if i % 2 == 0 else "F"
        f2 = "T" if i % 3 == 0 else "F"
        f3 = "T" if (i * 7) % 5 < 2 else "F"
        n1 = 1 + i % 3
        n2 = 1 + (i * 3) % 4
        n3 = ((i * 37) % 23) / 4.0
        before1 = 50.0 + ((i * 13) % 17)
        after1 = before1 + 2.0 + ((i * 7) % 5 - 2)
        before2 = 10.0 + ((i * 11) % 9) / 2.0
        after2 = before2 - 0.5 + ((i * 5) % 7 - 3) / 4.0
        score = ((i * 29) % 40 + 0.5) / 41.0
        resp = "Y" if (i * 17) % 40 < 18 + (6 if score > 0.5 else -6) else "N"
        pred = "Y" if score > 0.5 else "N"
        conf = score if pred == "Y" else 1.0 - score
        lines.append("%d,%s,%s,%s,%d,%d,%s,%s,%s,%s,%s,%s,%s,%s" % (
            i + 1, f1, f2, f3, n1, n2, repr(n3), repr(before1), repr(after1), repr(before2),
            repr(after2), resp, pred, repr(conf)))
    return lines


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


def write_csv(file_name, lines):
    handle = open("%s/%s" % (OUTPUTS_DIR, file_name), "w")
    try:
        handle.write(NL.join(lines) + NL)
    finally:
        handle.close()
    manifest["saved"].append(file_name)


def instantiate_node(stream, node):
    table = stream.createAt("table", "TempTable", 700, 300)
    stream.link(node, table)
    try:
        table.run([])
    finally:
        stream.delete(table)


def new_stream(case_name):
    stream = taskrunner.createStream(case_name, False, False)
    stream.setPropertyValue("decimal_places", 15)
    src = stream.createAt("variablefile", CSV_NAME, 100, 100)
    src.setPropertyValue("full_filename", "%s/%s" % (OUTPUTS_DIR, CSV_NAME))
    src.setPropertyValue("read_field_names", True)
    type_node = stream.createAt("type", "Type", 250, 100)
    stream.link(src, type_node)
    instantiate_node(stream, type_node)
    type_node.setKeyedPropertyValue("direction", "id", "None")
    for flag in ["f1", "f2", "f3"]:
        type_node.setKeyedPropertyValue("type", flag, "Flag")
    for numeric in ["n1", "n2", "n3", "before1", "after1", "before2", "after2"]:
        type_node.setKeyedPropertyValue("type", numeric, "Range")
    type_node.setKeyedPropertyValue("type", "resp", "Flag")
    type_node.setKeyedPropertyValue("values", "resp", ["N", "Y"])
    type_node.setKeyedPropertyValue("direction", "resp", "Target")
    type_node.setKeyedPropertyValue("type", "$R-resp", "Flag")
    type_node.setKeyedPropertyValue("values", "$R-resp", ["N", "Y"])
    type_node.setKeyedPropertyValue("direction", "$R-resp", "None")
    type_node.setKeyedPropertyValue("type", "$RC-resp", "Range")
    type_node.setKeyedPropertyValue("direction", "$RC-resp", "None")
    instantiate_node(stream, type_node)
    return stream, type_node


def build_case(case_name, node_type, props):
    stream = None
    entry = {"case": case_name, "file": case_name + ".str", "properties": props, "readback": []}
    manifest["cases"].append(entry)
    try:
        stream, upstream = new_stream(case_name)
        out_node = stream.createAt(node_type, case_name, 450, 100)
        stream.link(upstream, out_node)
        for name, value in props:
            out_node.setPropertyValue(name, value)
            entry["readback"].append([name, unicode(out_node.getPropertyValue(name))])
        out_node.run([])
        path = "%s/%s" % (OUTPUTS_DIR, entry["file"])
        taskrunner.saveStreamToFile(stream, path)
        manifest["saved"].append(entry["file"])
        entry["type"] = unicode(out_node.getTypeName())
    except ERRORS, e:
        manifest["errors"].append({"case": case_name, "message": message(e)})
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS:
                pass


ensure_clean_dir(OUTPUTS_DIR)
write_csv(CSV_NAME, rows())

build_case("matrix_flags", "matrix", [("fields", "Flags"), ("cell_contents", "CrossTabs"),
           ("display", ["Counts"]), ("include_totals", True)])
build_case("matrix_flags_pct", "matrix", [("fields", "Flags"), ("cell_contents", "CrossTabs"),
           ("display", ["Counts", "RowPct", "ColumnPct", "TotalPct", "Expected", "Residuals"]),
           ("include_totals", True)])
build_case("matrix_numerics", "matrix", [("fields", "Numerics"), ("row", "n1"), ("column", "n2"),
           ("cell_contents", "CrossTabs"), ("display", ["Counts"]), ("include_totals", True)])
build_case("matrix_numerics_function", "matrix", [("fields", "Numerics"), ("row", "n1"),
           ("column", "n2"), ("cell_contents", "Function"), ("function_field", "n3"),
           ("function", "Sum"), ("include_totals", True)])
build_case("means_fields", "means", [("means_mode", "BetweenFields"),
           ("paired_fields", [["before1", "after1"], ["before2", "after2"]]),
           ("output_view", "Advanced")])
build_case("means_fields_simple", "means", [("means_mode", "BetweenFields"),
           ("paired_fields", [["before1", "after1"], ["before2", "after2"]]),
           ("output_view", "Simple")])
build_case("evaluation_profit", "evaluation", [("chart_type", "Profit"),
           ("field_detection_method", "Name"), ("use_fixed_cost", True), ("cost_value", 1.0),
           ("use_fixed_revenue", True), ("revenue_value", 5.0), ("n_tile", "Deciles"),
           ("cumulative", True)])
build_case("evaluation_roi", "evaluation", [("chart_type", "ROI"),
           ("field_detection_method", "Name"), ("use_fixed_cost", True), ("cost_value", 1.0),
           ("use_fixed_revenue", True), ("revenue_value", 5.0), ("n_tile", "Deciles"),
           ("cumulative", True)])

handle = open(OUT_DIR + "/manifest.json", "w")
try:
    json.dump(manifest, handle, indent=2, sort_keys=True)
finally:
    handle.close()
