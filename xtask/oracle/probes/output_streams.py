# Probe: build deterministic Output node corpus streams and input CSVs in corpus/outputs/.
# Each stream tests one output node setting family and ends in the output node itself,
# with decimal_places = 15.
# Run with `cargo xtask probe output_streams` (IBM SPSS Modeler, Jython 2.7, public API only).
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
OUTPUTS_DIR = "D:/data-mining-software/corpus/outputs"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"saved": [], "errors": [], "cases": []}

# Deterministic dataset with integer (`id`, `iv`), real (`rv`, `score`), string (`grp`, `s`),
# flag (`flg`), categorical target (`target`), predicted target (`$R-target`), and prediction
# confidence (`$RC-target`), including null values across numeric and string fields.
OUT_MAIN_CSV = [
    "id,grp,iv,rv,score,flg,s,target,$R-target,$RC-target",
    "1,A,10,1.5,2.0,T,alpha,cat1,cat1,0.9",
    "2,A,20,3.25,4.5,F,beta,cat1,cat2,0.6",
    "3,A,10,2.75,3.0,T,alpha,cat2,cat2,0.85",
    "4,A,,4.0,5.5,F,,cat1,cat1,0.75",
    "5,B,30,-1.5,0.5,T,gamma,cat2,cat2,0.95",
    "6,B,40,0.0,1.0,F,beta,cat2,cat1,0.55",
    "7,B,30,2.5,3.5,T,alpha,cat3,cat3,0.8",
    "8,B,50,,6.0,T,gamma,cat3,cat2,0.65",
    "9,C,15,5.0,4.0,F,beta,cat1,cat1,0.92",
    "10,C,25,-2.25,-1.0,T,alpha,cat3,cat3,0.88",
    "11,C,35,1.0,2.5,F,gamma,cat2,cat3,0.52",
    "12,,20,3.0,3.5,T,,cat1,cat1,0.7",
]


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
    path = "%s/%s" % (OUTPUTS_DIR, file_name)
    handle = open(path, "w")
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
    type_node.setKeyedPropertyValue("type", "grp", "Set")
    type_node.setKeyedPropertyValue("type", "iv", "Range")
    type_node.setKeyedPropertyValue("type", "rv", "Range")
    type_node.setKeyedPropertyValue("type", "score", "Range")
    type_node.setKeyedPropertyValue("type", "flg", "Flag")
    type_node.setKeyedPropertyValue("type", "s", "Set")
    type_node.setKeyedPropertyValue("type", "target", "Set")
    type_node.setKeyedPropertyValue("direction", "target", "Target")
    type_node.setKeyedPropertyValue("type", "$R-target", "Set")
    type_node.setKeyedPropertyValue("direction", "$R-target", "None")
    type_node.setKeyedPropertyValue("type", "$RC-target", "Range")
    type_node.setKeyedPropertyValue("direction", "$RC-target", "None")
    instantiate_node(stream, type_node)
    return stream, type_node


def build_case(case_name, csv_name, node_type, configure):
    stream = None
    try:
        stream, upstream = new_stream(case_name, csv_name)
        out_node = stream.createAt(node_type, case_name, 450, 100)
        stream.link(upstream, out_node)
        configure(stream, out_node, upstream)
        out_node.run([])
        file_name = "%s.str" % case_name
        path = "%s/%s" % (OUTPUTS_DIR, file_name)
        taskrunner.saveStreamToFile(stream, path)
        manifest["saved"].append(file_name)
        manifest["cases"].append({
            "case": case_name,
            "file": file_name,
            "type": unicode(out_node.getTypeName()),
        })
    except ERRORS, e:
        manifest["errors"].append({"case": case_name, "message": message(e)})
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS:
                pass


ensure_clean_dir(OUTPUTS_DIR)
write_csv("out_main.csv", OUT_MAIN_CSV)


# --- 1. Data Audit cases ---

def cfg_dataaudit_default(stream, node, upstream):
    pass


def cfg_dataaudit_full_stats(stream, node, upstream):
    node.setPropertyValue("custom_fields", True)
    node.setPropertyValue("fields", ["grp", "iv", "rv", "score", "flg", "s", "target"])
    node.setPropertyValue("display_graphs", False)
    node.setPropertyValue("basic_stats", True)
    node.setPropertyValue("advanced_stats", True)
    node.setPropertyValue("median_stats", True)
    node.setPropertyValue("calculate", ["Count", "Breakdown"])
    node.setPropertyValue("outlier_detection_method", "std")
    node.setPropertyValue("outlier_detection_std_outlier", 1.5)
    node.setPropertyValue("outlier_detection_std_extreme", 2.5)


build_case("dataaudit_default", "out_main.csv", "dataaudit", cfg_dataaudit_default)
build_case("dataaudit_full_stats", "out_main.csv", "dataaudit", cfg_dataaudit_full_stats)


# --- 2. Analysis cases ---

def cfg_analysis_default(stream, node, upstream):
    node.setPropertyValue("coincidence", False)
    node.setPropertyValue("field_detection_method", "Name")


def cfg_analysis_coincidence(stream, node, upstream):
    node.setPropertyValue("coincidence", True)
    node.setPropertyValue("confidence", True)
    node.setPropertyValue("field_detection_method", "Name")
    node.setPropertyValue("by_fields", ["flg"])


build_case("analysis_default", "out_main.csv", "analysis", cfg_analysis_default)
build_case("analysis_coincidence", "out_main.csv", "analysis", cfg_analysis_coincidence)


# --- 3. Matrix cases ---

def cfg_matrix_counts(stream, node, upstream):
    node.setPropertyValue("row", "grp")
    node.setPropertyValue("column", "target")
    node.setPropertyValue("cell_contents", "CrossTabs")
    node.setPropertyValue("display", ["Counts"])
    node.setPropertyValue("include_totals", True)
    node.setPropertyValue("include_missing_values", True)


def cfg_matrix_percentages(stream, node, upstream):
    node.setPropertyValue("row", "grp")
    node.setPropertyValue("column", "target")
    node.setPropertyValue("cell_contents", "CrossTabs")
    node.setPropertyValue("display", ["Counts", "RowPct", "ColumnPct", "TotalPct", "Expected", "Residuals"])
    node.setPropertyValue("include_totals", True)
    node.setPropertyValue("include_missing_values", False)


def cfg_matrix_function(stream, node, upstream):
    node.setPropertyValue("row", "grp")
    node.setPropertyValue("column", "target")
    node.setPropertyValue("cell_contents", "Function")
    node.setPropertyValue("function_field", "rv")
    node.setPropertyValue("function", "Mean")
    node.setPropertyValue("include_totals", True)
    node.setPropertyValue("include_missing_values", False)


build_case("matrix_counts", "out_main.csv", "matrix", cfg_matrix_counts)
build_case("matrix_percentages", "out_main.csv", "matrix", cfg_matrix_percentages)
build_case("matrix_function", "out_main.csv", "matrix", cfg_matrix_function)


# --- 4. Statistics case ---

def cfg_statistics_correlations(stream, node, upstream):
    node.setPropertyValue("examine", ["iv", "rv", "score"])
    node.setPropertyValue("statistics", [
        "count", "mean", "sum", "min", "max", "range", "variance", "sdev", "semean", "median", "mode"
    ])
    node.setPropertyValue("correlate", ["iv", "rv", "score"])
    node.setPropertyValue("label_correlations", True)


build_case("statistics_correlations", "out_main.csv", "statistics", cfg_statistics_correlations)


# --- 5. Means case ---

def cfg_means_by_group(stream, node, upstream):
    node.setPropertyValue("means_mode", "BetweenGroups")
    node.setPropertyValue("grouping_field", "grp")
    node.setPropertyValue("test_fields", ["iv", "rv", "score"])
    node.setPropertyValue("output_view", "Advanced")


build_case("means_by_group", "out_main.csv", "means", cfg_means_by_group)


# --- 6. Table case ---

def cfg_table_basic(stream, node, upstream):
    pass


build_case("table_basic", "out_main.csv", "table", cfg_table_basic)


handle = open(OUT_DIR + "/manifest.json", "w")
try:
    json.dump(manifest, handle, indent=2, sort_keys=True)
finally:
    handle.close()
