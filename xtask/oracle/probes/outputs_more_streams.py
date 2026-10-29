# Probe: output node remaining settings the outputs_next label does not cover (label outputs_more):
# file output for Matrix, Means, Analysis, Statistics and Data Audit; missing values in flag and
# numeric matrices and in the Means paired t-test; Evaluation profit and ROI with variable cost,
# revenue and weight fields.
# Run with `cargo xtask probe outputs_more_streams` (IBM SPSS Modeler, Jython 2.7, public API
# only), then corpus record outputs_more and oracle. Framework as output_next_streams.py.
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
OUTPUTS_DIR = "D:/data-mining-software/corpus/outputs_more"
CSV_NAME = "out_more.csv"
MISSING_CSV = "out_more_missing.csv"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"saved": [], "errors": [], "cases": []}


def base_values(i):
    # Deterministic formulas of the row index, as in output_next_streams.py, extended with
    # categorical grp/target (specified Matrix) and per-record cost/revenue/weight fields.
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
    score = ((i * 29) % 60 + 0.5) / 61.0
    resp = "Y" if (i * 17) % 60 < 27 + (9 if score > 0.5 else -9) else "N"
    pred = "Y" if score > 0.5 else "N"
    conf = score if pred == "Y" else 1.0 - score
    grp = ["A", "B", "C"][i % 3]
    target = ["cat1", "cat2", "cat3"][(i * 2) % 3]
    cost_f = 1.0 + (i % 5)
    rev_f = 5.0 + (i % 4)
    wgt_f = 1.0 + (i % 3) * 0.5
    return (f1, f2, f3, n1, n2, n3, before1, after1, before2, after2, resp, pred, conf,
            grp, target, cost_f, rev_f, wgt_f)


def rows():
    # 60 records, no missing values: the clean base for file output and variable money cases.
    header = "id,f1,f2,f3,n1,n2,n3,before1,after1,before2,after2,resp,$R-resp,$RC-resp,grp,target,cost_f,rev_f,wgt_f"
    lines = [header]
    for i in range(60):
        (f1, f2, f3, n1, n2, n3, before1, after1, before2, after2, resp, pred, conf,
         grp, target, cost_f, rev_f, wgt_f) = base_values(i)
        lines.append("%d,%s,%s,%s,%d,%d,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s" % (
            i + 1, f1, f2, f3, n1, n2, repr(n3), repr(before1), repr(after1), repr(before2),
            repr(after2), resp, pred, repr(conf), grp, target,
            repr(float(cost_f)), repr(float(rev_f)), repr(float(wgt_f))))
    return lines


def rows_missing():
    # Same 60 records but with blanks ('""', a quoted empty string) and nulls ('', an empty
    # unquoted CSV cell giving ,,) in the counted fields: flags f1/f2, numerics n1/n2 and the
    # paired Means fields before1/after1/before2/after2. All other fields stay clean.
    header = "id,f1,f2,f3,n1,n2,n3,before1,after1,before2,after2,resp,$R-resp,$RC-resp,grp,target,cost_f,rev_f,wgt_f"
    lines = [header]
    for i in range(60):
        (f1, f2, f3, n1, n2, n3, before1, after1, before2, after2, resp, pred, conf,
         grp, target, cost_f, rev_f, wgt_f) = base_values(i)
        f1v = f1
        f2v = f2
        n1v = str(n1)
        n2v = str(n2)
        b1v = repr(before1)
        a1v = repr(after1)
        b2v = repr(before2)
        a2v = repr(after2)
        if i % 15 == 5:
            f1v = ""
        if i % 15 == 10:
            f1v = '""'
        if i % 15 == 6:
            f2v = ""
        if i % 15 == 11:
            f2v = '""'
        if i % 15 == 7:
            n1v = ""
        if i % 15 == 12:
            n1v = '""'
        if i % 15 == 8:
            n2v = ""
        if i % 15 == 13:
            n2v = '""'
        if i % 12 == 7:
            b1v = ""
        if i % 12 == 9:
            b1v = '""'
        if i % 12 == 8:
            a1v = ""
        if i % 12 == 10:
            a1v = '""'
        if i % 20 == 15:
            b2v = ""
        if i % 20 == 16:
            b2v = '""'
        if i % 20 == 17:
            a2v = ""
        if i % 20 == 18:
            a2v = '""'
        lines.append("%d,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s" % (
            i + 1, f1v, f2v, f3, n1v, n2v, repr(n3), b1v, a1v, b2v,
            a2v, resp, pred, repr(conf), grp, target,
            repr(float(cost_f)), repr(float(rev_f)), repr(float(wgt_f))))
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


def new_stream(case_name, csv_name):
    stream = taskrunner.createStream(case_name, False, False)
    stream.setPropertyValue("decimal_places", 15)
    src = stream.createAt("variablefile", csv_name, 100, 100)
    src.setPropertyValue("full_filename", "%s/%s" % (OUTPUTS_DIR, csv_name))
    src.setPropertyValue("read_field_names", True)
    type_node = stream.createAt("type", "Type", 250, 100)
    stream.link(src, type_node)
    instantiate_node(stream, type_node)
    type_node.setKeyedPropertyValue("direction", "id", "None")
    for flag in ["f1", "f2", "f3"]:
        type_node.setKeyedPropertyValue("type", flag, "Flag")
    for numeric in ["n1", "n2", "n3", "before1", "after1", "before2", "after2",
                    "cost_f", "rev_f", "wgt_f"]:
        type_node.setKeyedPropertyValue("type", numeric, "Range")
    type_node.setKeyedPropertyValue("type", "resp", "Flag")
    type_node.setKeyedPropertyValue("values", "resp", ["N", "Y"])
    type_node.setKeyedPropertyValue("direction", "resp", "Target")
    type_node.setKeyedPropertyValue("type", "$R-resp", "Flag")
    type_node.setKeyedPropertyValue("values", "$R-resp", ["N", "Y"])
    type_node.setKeyedPropertyValue("direction", "$R-resp", "None")
    type_node.setKeyedPropertyValue("type", "$RC-resp", "Range")
    type_node.setKeyedPropertyValue("direction", "$RC-resp", "None")
    type_node.setKeyedPropertyValue("type", "grp", "Set")
    type_node.setKeyedPropertyValue("direction", "grp", "Input")
    type_node.setKeyedPropertyValue("type", "target", "Set")
    type_node.setKeyedPropertyValue("direction", "target", "Input")
    type_node.setKeyedPropertyValue("direction", "cost_f", "Input")
    type_node.setKeyedPropertyValue("direction", "rev_f", "Input")
    type_node.setKeyedPropertyValue("direction", "wgt_f", "Input")
    instantiate_node(stream, type_node)
    return stream, type_node


def build_case(case_name, csv_name, node_type, props):
    stream = None
    entry = {"case": case_name, "file": case_name + ".str", "data": csv_name,
             "properties": props, "readback": []}
    manifest["cases"].append(entry)
    try:
        stream, upstream = new_stream(case_name, csv_name)
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
write_csv(MISSING_CSV, rows_missing())

# Control outputs/matrix_counts: specified CrossTabs Counts with totals and missing included.
MATRIX_BASE = [("row", "grp"), ("column", "target"), ("cell_contents", "CrossTabs"),
               ("display", ["Counts"]), ("include_totals", True),
               ("include_missing_values", True)]
build_case("matrix_file_formatted", CSV_NAME, "matrix",
           MATRIX_BASE + [("output_mode", "File"), ("output_format", "Formatted"),
                          ("full_filename", OUTPUTS_DIR + "/matrix_file_formatted.tab")])
build_case("matrix_file_delimited", CSV_NAME, "matrix",
           MATRIX_BASE + [("output_mode", "File"), ("output_format", "Delimited"),
                          ("full_filename", OUTPUTS_DIR + "/matrix_file_delimited.csv")])
build_case("matrix_file_html", CSV_NAME, "matrix",
           MATRIX_BASE + [("output_mode", "File"), ("output_format", "HTML"),
                          ("full_filename", OUTPUTS_DIR + "/matrix_file_html.html")])
build_case("matrix_file_output", CSV_NAME, "matrix",
           MATRIX_BASE + [("output_mode", "File"), ("output_format", "Output"),
                          ("full_filename", OUTPUTS_DIR + "/matrix_file_output.cou")])

# Control outputs_next/means_fields: paired t-test between fields, advanced view.
MEANS_BASE = [("means_mode", "BetweenFields"),
              ("paired_fields", [["before1", "after1"], ["before2", "after2"]]),
              ("output_view", "Advanced")]
build_case("means_file_formatted", CSV_NAME, "means",
           MEANS_BASE + [("output_mode", "File"), ("output_format", "Formatted"),
                         ("full_filename", OUTPUTS_DIR + "/means_file_formatted.tab")])
build_case("means_file_delimited", CSV_NAME, "means",
           MEANS_BASE + [("output_mode", "File"), ("output_format", "Delimited"),
                         ("full_filename", OUTPUTS_DIR + "/means_file_delimited.csv")])
build_case("means_file_html", CSV_NAME, "means",
           MEANS_BASE + [("output_mode", "File"), ("output_format", "HTML"),
                         ("full_filename", OUTPUTS_DIR + "/means_file_html.html")])
build_case("means_file_output", CSV_NAME, "means",
           MEANS_BASE + [("output_mode", "File"), ("output_format", "Output"),
                         ("full_filename", OUTPUTS_DIR + "/means_file_output.cou")])

# Control outputs/analysis_default: accuracy only, detection by name.
ANALYSIS_BASE = [("coincidence", False), ("field_detection_method", "Name")]
build_case("analysis_file_text", CSV_NAME, "analysis",
           ANALYSIS_BASE + [("output_mode", "File"), ("output_format", "Text"),
                            ("full_filename", OUTPUTS_DIR + "/analysis_file_text.txt")])
build_case("analysis_file_html", CSV_NAME, "analysis",
           ANALYSIS_BASE + [("output_mode", "File"), ("output_format", "HTML"),
                            ("full_filename", OUTPUTS_DIR + "/analysis_file_html.html")])
build_case("analysis_file_output", CSV_NAME, "analysis",
           ANALYSIS_BASE + [("output_mode", "File"), ("output_format", "Output"),
                            ("full_filename", OUTPUTS_DIR + "/analysis_file_output.cou")])

# Control outputs/statistics_correlations (fields n1/n2/n3 stand in for iv/rv/score).
STATISTICS_BASE = [("examine", ["n1", "n2", "n3"]),
                   ("statistics", ["count", "mean", "sum", "min", "max", "range",
                                   "variance", "sdev", "semean", "median", "mode"]),
                   ("correlate", ["n1", "n2", "n3"]), ("label_correlations", True)]
build_case("statistics_file_text", CSV_NAME, "statistics",
           STATISTICS_BASE + [("output_mode", "File"), ("output_format", "Text"),
                              ("full_filename", OUTPUTS_DIR + "/statistics_file_text.txt")])
build_case("statistics_file_html", CSV_NAME, "statistics",
           STATISTICS_BASE + [("output_mode", "File"), ("output_format", "HTML"),
                              ("full_filename", OUTPUTS_DIR + "/statistics_file_html.html")])
build_case("statistics_file_output", CSV_NAME, "statistics",
           STATISTICS_BASE + [("output_mode", "File"), ("output_format", "Output"),
                              ("full_filename", OUTPUTS_DIR + "/statistics_file_output.cou")])

# Control outputs/dataaudit_default: defaults audit every field.
build_case("dataaudit_file_formatted", CSV_NAME, "dataaudit",
           [("output_mode", "File"), ("output_format", "Formatted"),
            ("full_filename", OUTPUTS_DIR + "/dataaudit_file_formatted.tab")])
build_case("dataaudit_file_delimited", CSV_NAME, "dataaudit",
           [("output_mode", "File"), ("output_format", "Delimited"),
            ("full_filename", OUTPUTS_DIR + "/dataaudit_file_delimited.csv")])
build_case("dataaudit_file_html", CSV_NAME, "dataaudit",
           [("output_mode", "File"), ("output_format", "HTML"),
            ("full_filename", OUTPUTS_DIR + "/dataaudit_file_html.html")])
build_case("dataaudit_file_output", CSV_NAME, "dataaudit",
           [("output_mode", "File"), ("output_format", "Output"),
            ("full_filename", OUTPUTS_DIR + "/dataaudit_file_output.cou")])

# Missing values: same settings as the recorded Flags / Numerics / paired cases.
build_case("matrix_flags_missing", MISSING_CSV, "matrix",
           [("fields", "Flags"), ("cell_contents", "CrossTabs"),
            ("display", ["Counts"]), ("include_totals", True)])
build_case("matrix_numerics_missing", MISSING_CSV, "matrix",
           [("fields", "Numerics"), ("row", "n1"), ("column", "n2"),
            ("cell_contents", "CrossTabs"), ("display", ["Counts"]),
            ("include_totals", True)])
build_case("means_fields_missing", MISSING_CSV, "means", MEANS_BASE)

# Variable cost/revenue/weight against the recorded fixed-value profit/ROI cases.
EVAL_BASE = [("field_detection_method", "Name"), ("n_tile", "Deciles"), ("cumulative", True)]
build_case("evaluation_profit_cost", CSV_NAME, "evaluation",
           [("chart_type", "Profit")] + EVAL_BASE + [("use_fixed_cost", False),
            ("cost_field", "cost_f"), ("use_fixed_revenue", True), ("revenue_value", 5.0)])
build_case("evaluation_profit_revenue", CSV_NAME, "evaluation",
           [("chart_type", "Profit")] + EVAL_BASE + [("use_fixed_cost", True),
            ("cost_value", 1.0), ("use_fixed_revenue", False), ("revenue_field", "rev_f")])
build_case("evaluation_profit_weight", CSV_NAME, "evaluation",
           [("chart_type", "Profit")] + EVAL_BASE + [("use_fixed_cost", True),
            ("cost_value", 1.0), ("use_fixed_revenue", True), ("revenue_value", 5.0),
            ("use_fixed_weight", False), ("weight_field", "wgt_f")])
build_case("evaluation_roi_cost", CSV_NAME, "evaluation",
           [("chart_type", "ROI")] + EVAL_BASE + [("use_fixed_cost", False),
            ("cost_field", "cost_f"), ("use_fixed_revenue", True), ("revenue_value", 5.0)])
build_case("evaluation_roi_revenue", CSV_NAME, "evaluation",
           [("chart_type", "ROI")] + EVAL_BASE + [("use_fixed_cost", True),
            ("cost_value", 1.0), ("use_fixed_revenue", False), ("revenue_field", "rev_f")])
build_case("evaluation_roi_weight", CSV_NAME, "evaluation",
           [("chart_type", "ROI")] + EVAL_BASE + [("use_fixed_cost", True),
            ("cost_value", 1.0), ("use_fixed_revenue", True), ("revenue_value", 5.0),
            ("use_fixed_weight", False), ("weight_field", "wgt_f")])

handle = open(OUT_DIR + "/manifest.json", "w")
try:
    json.dump(manifest, handle, indent=2, sort_keys=True)
finally:
    handle.close()
