# Probe: Graphboard node on a fixed table in corpus/graphboard/ (label graphboard). The oracle
# exports each graph as VIZML, whose embeddedSource holds the rows the graph draws. Each stream
# is Variable File -> Type -> graphboard.
# Run with `cargo xtask probe graphboard_streams` (IBM SPSS Modeler, Jython 2.7, public API only).
# Property names and values: Modeler 19.0 Python Scripting and Automation Guide, Table 101
# (graphboardnode). The Detailed-tab Summary control is not scriptable (note before the example).
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/graphboard"
CSV_NAME = "graphboard_main.csv"
# `except EXC as e` is legal Jython 2.7. The Python 2 comma form is a Python 3 syntax error.
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {
    "label": "graphboard",
    "input": CSV_NAME,
    "rows": 60,
    "saved": [],
    "errors": [],
    "cases": [],
    "notes": [
        "Table 101 graph_type value for a parallel chart is Parallel. ParallelCoordinates is not listed.",
        "The Summary field on the Detailed tab is not scriptable, so Bar and Pie use the template default.",
        "panelrow_field and panelcol_field are typed string. This probe sets field names.",
        "Pie and Heatmap each have two documented field bindings (axes, and categories or rows/columns).",
        "Parallel sets fields to a list. Table 101 types fields as field, not as a list.",
        "A property that does not apply to the graph type is ignored (guide note; y_field on a histogram).",
    ],
}

# 60 rows. region/channel/segment are categories (region is blank every 29th row).
# amount/score/weight are reals with fractions; each is blank on its own modulus.
NROWS = 60
REGIONS = ["North", "South", "East"]
CHANNELS = ["Store", "Web"]
SEGMENTS = ["Retail", "Trade"]
# (name, variablefile custom_storage_type, Type measurement). Every direction is Input.
SCHEMA = [
    ("region", "String", "Set"),
    ("channel", "String", "Set"),
    ("segment", "String", "Set"),
    ("amount", "Real", "Range"),
    ("score", "Real", "Range"),
    ("weight", "Real", "Range"),
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


def decimal_text(numerator, scale, digits):
    # Exact decimal text. scale is 10**digits. No binary float.
    if numerator < 0:
        return "-" + decimal_text(-numerator, scale, digits)
    whole = numerator // scale
    frac = numerator % scale
    if frac == 0:
        return "%d" % whole
    text = ("%0*d" % (digits, frac)).rstrip("0")
    return "%d.%s" % (whole, text)


def row_cells(i):
    region = REGIONS[i % 3]
    if i % 29 == 0:
        region = ""
    channel = CHANNELS[(i // 3) % 2]
    segment = SEGMENTS[(i // 6) % 2]
    if i % 11 == 0:
        amount = ""
    elif i % 23 == 4:
        amount = "12.123456789012345"
    else:
        # (i % 17) - 8 + (i % 4) * 0.25
        hundredths = ((i % 17) - 8) * 100 + (i % 4) * 25
        amount = decimal_text(hundredths, 100, 2)
    if i % 13 == 0:
        score = ""
    else:
        # (i % 13) * 0.5 + 0.125
        thousandths = (i % 13) * 500 + 125
        score = decimal_text(thousandths, 1000, 3)
    if i % 17 == 0:
        weight = ""
    else:
        # 1 + (i % 9) * 0.1
        tenths = 10 + (i % 9)
        weight = decimal_text(tenths, 10, 1)
    return [region, channel, segment, amount, score, weight]


def write_data():
    if NROWS < 40 or NROWS > 80:
        raise ValueError("row count out of range")
    lines = [",".join([field[0] for field in SCHEMA])]
    for i in range(NROWS):
        cells = row_cells(i)
        if len(cells) != len(SCHEMA):
            raise ValueError("wrong width")
        lines.append(",".join(cells))
    handle = open(DIR + "/" + CSV_NAME, "wb")
    try:
        handle.write(NL.join(lines) + NL)
    finally:
        handle.close()
    manifest["saved"].append(CSV_NAME)


def instantiate_node(stream, node):
    table = stream.createAt("table", "TempTable", 760, 360)
    stream.link(node, table)
    try:
        table.run([])
    finally:
        stream.delete(table)


def build_case(case_name, props):
    stream = None
    stage = "stream"
    file_name = "%s.str" % case_name
    recorded = []
    for name, value in props:
        if isinstance(value, list):
            recorded.append([name, list(value)])
        else:
            recorded.append([name, value])
    entry = {"case": case_name, "file": file_name, "node_type": "graphboard",
             "input": CSV_NAME, "properties": recorded, "status": "error"}
    try:
        stream = taskrunner.createStream(case_name, False, False)
        stream.setPropertyValue("decimal_places", 15)
        src = stream.createAt("variablefile", CSV_NAME, 100, 100)
        src.setPropertyValue("full_filename", DIR + "/" + CSV_NAME)
        src.setPropertyValue("read_field_names", True)
        src.setPropertyValue("delimit_comma", True)
        src.setPropertyValue("decimal_symbol", "Period")
        for field_name, storage, measure in SCHEMA:
            src.setKeyedPropertyValue("custom_storage_type", field_name, storage)
        type_node = stream.createAt("type", "Type", 250, 100)
        stream.link(src, type_node)
        instantiate_node(stream, type_node)
        for field_name, storage, measure in SCHEMA:
            type_node.setKeyedPropertyValue("type", field_name, measure)
            type_node.setKeyedPropertyValue("direction", field_name, "Input")
        instantiate_node(stream, type_node)
        stage = "configure"
        graph = stream.createAt("graphboard", case_name, 460, 100)
        stream.link(type_node, graph)
        for name, value in props:
            graph.setPropertyValue(name, value)
        stage = "run"
        graph.run([])
        taskrunner.saveStreamToFile(stream, "%s/%s" % (DIR, file_name))
        manifest["saved"].append(file_name)
        entry["status"] = "saved"
    except ERRORS as exc:
        manifest["errors"].append({"case": case_name, "stage": stage, "message": message(exc)})
    finally:
        manifest["cases"].append(entry)
        if stream is not None:
            try:
                stream.clear()
            except ERRORS:
                pass


ensure_clean_dir(DIR)
write_data()

# Table 101, printed pages 192-194. Example (p.191): graph_type Line, x_field, y_field.
# Invalid properties are ignored (p.190), so each case sets only the slots named below.

# Bar (p.192). x_field category, y_field scale. Summary is not scriptable.
build_case("bar_region_amount", [
    ("graph_type", "Bar"),
    ("x_field", "region"),
    ("y_field", "amount"),
])

# BarCounts (p.192). Counts of x_field. y_field is not set.
build_case("barcounts_region", [
    ("graph_type", "BarCounts"),
    ("x_field", "region"),
])

# Histogram (p.193). The guide's ignore-note names y_field as invalid on a histogram, so only x_field.
build_case("histogram_amount", [
    ("graph_type", "Histogram"),
    ("x_field", "amount"),
])

# Pie (p.193), same axis slots as the Line example.
build_case("pie_region_amount", [
    ("graph_type", "Pie"),
    ("x_field", "region"),
    ("y_field", "amount"),
])

# Pie again, on categories_field and values_field (p.194). Descriptions there are blank.
build_case("pie_categories", [
    ("graph_type", "Pie"),
    ("categories_field", "region"),
    ("values_field", "amount"),
])

# PieCounts (p.193). Counts of x_field.
build_case("piecounts_region", [
    ("graph_type", "PieCounts"),
    ("x_field", "region"),
])

# Line (p.193). The example sets x_field and y_field to two scale fields.
build_case("line_score_amount", [
    ("graph_type", "Line"),
    ("x_field", "score"),
    ("y_field", "amount"),
])

# Scatterplot (p.194). Two scale fields.
build_case("scatter_score_amount", [
    ("graph_type", "Scatterplot"),
    ("x_field", "score"),
    ("y_field", "amount"),
])

# Boxplot (p.192). x_field category, y_field scale. Which slot the template reads is not described.
build_case("boxplot_region_amount", [
    ("graph_type", "Boxplot"),
    ("x_field", "region"),
    ("y_field", "amount"),
])

# Area (p.192). Same two scale fields as Line.
build_case("area_score_amount", [
    ("graph_type", "Area"),
    ("x_field", "score"),
    ("y_field", "amount"),
])

# Dotplot (p.193), not 2DDotplot. One scale field on x_field.
build_case("dotplot_amount", [
    ("graph_type", "Dotplot"),
    ("x_field", "amount"),
])

# Heatmap (p.193). color_field is described as "Used in heat maps." (p.194).
# rows_field and columns_field are the other matrix slots on that page.
build_case("heatmap_rows", [
    ("graph_type", "Heatmap"),
    ("rows_field", "region"),
    ("columns_field", "channel"),
    ("color_field", "amount"),
])

# Heatmap again, if the template uses the axis slots instead of rows/columns.
build_case("heatmap_axes", [
    ("graph_type", "Heatmap"),
    ("x_field", "region"),
    ("y_field", "channel"),
    ("color_field", "amount"),
])

# Parallel (p.193). Not ParallelCoordinates. fields is the plural slot (p.194).
build_case("parallel_fields", [
    ("graph_type", "Parallel"),
    ("fields", ["amount", "score", "weight"]),
])

# Bubble (p.192). size_field is described as "Used in bubble plots." (p.194).
build_case("bubble_score_amount", [
    ("graph_type", "Bubble"),
    ("x_field", "score"),
    ("y_field", "amount"),
    ("size_field", "weight"),
])

# Color and panel on a scatter. color_field, panelrow_field, panelcol_field (p.194).
build_case("scatter_color_panel", [
    ("graph_type", "Scatterplot"),
    ("x_field", "score"),
    ("y_field", "amount"),
    ("color_field", "segment"),
    ("panelrow_field", "region"),
    ("panelcol_field", "channel"),
])

handle = open(OUT_DIR + "/manifest.json", "w")
try:
    json.dump(manifest, handle, indent=2, sort_keys=True)
finally:
    handle.close()
