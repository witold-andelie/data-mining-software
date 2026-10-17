# Probe: graph nodes on DRUG1n in corpus/graphs/. The oracle exports each graph as VIZML, whose
# embeddedSource holds the rows the graph draws. Each stream is source -> Type [-> Derive] ->
# graph node.
# Run with `cargo xtask probe graph_streams` (IBM SPSS Modeler, Jython 2.7, public API only).
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/graphs"
DRUG = "D:/data-mining-software/corpus/models/DRUG1n"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"saved": [], "errors": [], "cases": []}

SCHEMA = [("Age", "Range"), ("Sex", "Flag"), ("BP", "Set"), ("Cholesterol", "Flag"),
          ("Na", "Range"), ("K", "Range"), ("Drug", "Set")]


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


def copy_drug():
    src = open(DRUG, "rb")
    try:
        data = src.read()
    finally:
        src.close()
    dst = open(DIR + "/DRUG1n", "wb")
    try:
        dst.write(data)
    finally:
        dst.close()
    manifest["saved"].append("DRUG1n")


def instantiate_node(stream, node):
    table = stream.createAt("table", "TempTable", 760, 360)
    stream.link(node, table)
    try:
        table.run([])
    finally:
        stream.delete(table)


def build_case(case_name, node_type, props, derives=None, target=None):
    stream = None
    stage = "stream"
    try:
        stream = taskrunner.createStream(case_name, False, False)
        stream.setPropertyValue("decimal_places", 15)
        src = stream.createAt("variablefile", "DRUG1n", 100, 100)
        src.setPropertyValue("full_filename", DIR + "/DRUG1n")
        src.setPropertyValue("read_field_names", True)
        type_node = stream.createAt("type", "Type", 250, 100)
        stream.link(src, type_node)
        instantiate_node(stream, type_node)
        for field_name, measure in SCHEMA:
            type_node.setKeyedPropertyValue("type", field_name, measure)
            if target is not None:
                direction = "Input"
                if field_name == target:
                    direction = "Target"
                type_node.setKeyedPropertyValue("direction", field_name, direction)
        instantiate_node(stream, type_node)
        last = type_node
        for name, formula in (derives or []):
            node = stream.createAt("derive", name, 330, 100)
            node.setPropertyValue("new_name", name)
            node.setPropertyValue("result_type", "Formula")
            node.setPropertyValue("formula_expr", formula)
            stream.link(last, node)
            last = node
        stage = "configure"
        graph = stream.createAt(node_type, case_name, 460, 100)
        stream.link(last, graph)
        for name, value in props:
            graph.setPropertyValue(name, value)
        stage = "run"
        graph.run([])
        file_name = "%s.str" % case_name
        taskrunner.saveStreamToFile(stream, "%s/%s" % (DIR, file_name))
        manifest["saved"].append(file_name)
        manifest["cases"].append({"case": case_name, "file": file_name, "node_type": node_type,
                                  "properties": [[n, v] for n, v in props]})
    except ERRORS, e:
        manifest["errors"].append({"case": case_name, "stage": stage, "message": message(e)})
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS:
                pass


ensure_clean_dir(DIR)
copy_drug()

# Distribution: categories and counts, overlays, normalisation, sort order.
build_case("dist_bp", "distribution", [("x_field", "BP")])
build_case("dist_drug", "distribution", [("x_field", "Drug")])
build_case("dist_bp_color", "distribution", [("x_field", "BP"), ("color_field", "Drug")])
build_case("dist_bp_color_norm", "distribution",
           [("x_field", "BP"), ("color_field", "Drug"), ("normalize", True)])
build_case("dist_drug_by_occurrence", "distribution",
           [("x_field", "Drug"), ("sort_mode", "ByOccurence")])

# Histogram: automatic bins, by number, by width, user range, overlay, normalise.
build_case("hist_age", "histogram", [("field", "Age")])
build_case("hist_na", "histogram", [("field", "Na")])
build_case("hist_age_bins10", "histogram",
           [("field", "Age"), ("bins", "ByNumber"), ("num_bins", 10)])
build_case("hist_age_width5", "histogram",
           [("field", "Age"), ("bins", "ByWidth"), ("bin_width", 5.0)])
build_case("hist_age_range", "histogram",
           [("field", "Age"), ("range_mode", "UserDefined"), ("range_min", 20.0),
            ("range_max", 60.0)])
build_case("hist_age_color", "histogram", [("field", "Age"), ("color_field", "Drug")])
build_case("hist_age_color_norm", "histogram",
           [("field", "Age"), ("color_field", "Drug"), ("normalize", True)])

# Plot: points, overlays.
build_case("plot_age_na", "plot", [("x_field", "Age"), ("y_field", "Na")])
build_case("plot_na_k_color", "plot",
           [("x_field", "Na"), ("y_field", "K"), ("color_field", "Drug")])
build_case("plot_age_na_size", "plot",
           [("x_field", "Age"), ("y_field", "Na"), ("size_field", "K")])

# Multiplot and Time Plot: several y fields.
build_case("multi_age_na_k", "multiplot", [("x_field", "Age"), ("y_fields", ["Na", "K"])])
build_case("multi_age_na_k_norm", "multiplot",
           [("x_field", "Age"), ("y_fields", ["Na", "K"]), ("normalize", True)])
build_case("timeplot_na_k", "timeplot", [("plot_series", "Series"), ("y_fields", ["Na", "K"])])

# Collection: a field's values collected over another.
build_case("collect_na_over_age", "collection",
           [("over_field", "Age"), ("collect_field", "Na")])
build_case("collect_na_over_age_mean", "collection",
           [("over_field", "Age"), ("collect_field", "Na"), ("operation", "Mean")])

# Web: links between categories.
build_case("web_bp_chol_drug", "web", [("fields", ["BP", "Cholesterol", "Drug"])])

# Evaluation on a derived prediction and confidence for the Cholesterol flag.
SCORES = [("$R-Cholesterol", 'if Na > 0.7 then "HIGH" else "NORMAL" endif'),
          ("$RC-Cholesterol", "0.5 + abs(Na - 0.7)")]
EVAL = [("field_detection_method", "Name")]
build_case("eval_gains", "evaluation", EVAL + [("chart_type", "Gains")], SCORES, "Cholesterol")
build_case("eval_lift", "evaluation", EVAL + [("chart_type", "Lift")], SCORES, "Cholesterol")
build_case("eval_gains_noncumulative", "evaluation",
           EVAL + [("chart_type", "Gains"), ("cumulative", False)], SCORES, "Cholesterol")

handle = open(OUT_DIR + "/manifest.json", "w")
try:
    json.dump(manifest, handle, indent=2, sort_keys=True)
finally:
    handle.close()
