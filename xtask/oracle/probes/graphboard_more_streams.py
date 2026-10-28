# Probe: Graphboard remaining graphs, phase A (label graphboard_more). VIZML embeddedSource
# behind each graph on fixed tables in D:/data-mining-software/corpus/graphboard_more/.
# Each stream is Variable File -> Type -> graphboard.
# Run with `cargo xtask probe graphboard_more_streams` (IBM SPSS Modeler, Jython 2.7, public API only).
# Property names and values: Modeler 19.0 Python Scripting and Automation Guide, Table 101
# (graphboardnode), excerpt target/gb-more/scripting-guide-graphboard.txt, pp.192-194.
# The Detailed-tab Summary control is not scriptable (note before the example, p.191).
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/graphboard_more"
CSV_MAIN = "graphboard_more_main.csv"
CSV_CONST = "graphboard_more_const.csv"
CSV_BOX_OUT = "graphboard_more_box_outliers.csv"
CSV_BOX_EMPTY = "graphboard_more_box_empty.csv"
CSV_LARGE = "graphboard_more_large.csv"
# `except EXC as e` is legal Jython 2.7. The Python 2 comma form is a Python 3 syntax error.
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {
    "label": "graphboard_more",
    "input": CSV_MAIN,
    "rows": 60,
    "extra_inputs": {
        CSV_CONST: 20,
        CSV_BOX_OUT: 30,
        CSV_BOX_EMPTY: 12,
        CSV_LARGE: 2500,
    },
    "saved": [],
    "errors": [],
    "cases": [],
    "notes": [
        "Table 101 graph_type values sampled: Histogram, Bar, Pie, Line, Scatterplot, Boxplot, Heatmap and 3-D/map types (pp.192-193).",
        "Table 101 slots used: x_field, y_field, z_field, color_field, categories_field, values_field, rows_field, columns_field, panelrow_field, panelcol_field, animation_field, longitude_field, latitude_field, map_color_field, data_key_field (p.194).",
        "panelrow_field and panelcol_field are typed string. This probe sets field names (as in graphboard_streams.py).",
        "animation_field is typed string (p.194). This probe sets a field name.",
        "Bar and Pie use category/value bindings; Heatmap uses rows/columns/color, not axis bindings (as in graphboard_streams.py).",
        "Bar/Pie on x_field/y_field and Heatmap on x_field/y_field test the guide note that an invalid property is ignored (p.190).",
        "The Detailed-tab Summary field is not scriptable (p.191), so no axis-summary case is probed.",
        "Histogram bin count/width, overlay, transparency, shape, data labels, templates and the large-dataset threshold have no graphboardnode properties in Table 101, so no cases set them (see CASES.md).",
        "Each case changes one thing against its control where possible; the control is named in a comment.",
    ],
}

# 60-row main table. region/channel/segment are categories (region is blank every 29th row).
# amount/score/weight are reals with fractions; each is blank on its own modulus.
NROWS_MAIN = 60
NROWS_CONST = 20
NROWS_BOX_OUT = 30
NROWS_BOX_EMPTY = 12
NROWS_LARGE = 2500
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


def row_cells_main(i):
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


def row_cells_const(i):
    cells = row_cells_main(i)
    if i % 10 == 0:
        cells[3] = ""
    else:
        cells[3] = "4.0"
    return cells


def row_cells_box_outliers(i):
    cells = row_cells_main(i)
    if i < 20:
        cells[0] = "East"
        cells[3] = "5.0"
    elif i < 23:
        cells[0] = "East"
        cells[3] = "%d" % (100 + (i - 20))
    elif i < 27:
        cells[0] = "North"
        cells[3] = "%d" % (1 + (i - 23))
    else:
        cells[0] = "South"
        cells[3] = "%d" % (10 + (i - 27))
    return cells


def row_cells_box_empty(i):
    cells = row_cells_main(i)
    if i < 4:
        cells[0] = "East"
        cells[3] = ""
    elif i < 8:
        cells[0] = "North"
        cells[3] = "%d" % (1 + (i - 4))
    else:
        cells[0] = "South"
        cells[3] = "%d" % (10 + (i - 8))
    return cells


def write_csv(name, rows):
    if len(rows) == 0:
        raise ValueError("no rows")
    header = ",".join([field[0] for field in SCHEMA])
    lines = [header]
    for cells in rows:
        if len(cells) != len(SCHEMA):
            raise ValueError("wrong width")
        lines.append(",".join(cells))
    handle = open(DIR + "/" + name, "wb")
    try:
        handle.write(NL.join(lines) + NL)
    finally:
        handle.close()
    manifest["saved"].append(name)


def write_data():
    main_rows = []
    for i in range(NROWS_MAIN):
        main_rows.append(row_cells_main(i))
    if NROWS_MAIN < 40 or NROWS_MAIN > 80:
        raise ValueError("row count out of range")
    write_csv(CSV_MAIN, main_rows)
    const_rows = []
    for i in range(NROWS_CONST):
        const_rows.append(row_cells_const(i))
    if NROWS_CONST < 10 or NROWS_CONST > 40:
        raise ValueError("row count out of range")
    write_csv(CSV_CONST, const_rows)
    out_rows = []
    for i in range(NROWS_BOX_OUT):
        out_rows.append(row_cells_box_outliers(i))
    if NROWS_BOX_OUT < 10 or NROWS_BOX_OUT > 40:
        raise ValueError("row count out of range")
    write_csv(CSV_BOX_OUT, out_rows)
    empty_rows = []
    for i in range(NROWS_BOX_EMPTY):
        empty_rows.append(row_cells_box_empty(i))
    if NROWS_BOX_EMPTY < 10 or NROWS_BOX_EMPTY > 40:
        raise ValueError("row count out of range")
    write_csv(CSV_BOX_EMPTY, empty_rows)
    large_rows = []
    for i in range(NROWS_LARGE):
        large_rows.append(row_cells_main(i))
    if NROWS_LARGE < 2001 or NROWS_LARGE > 3000:
        raise ValueError("row count out of range")
    write_csv(CSV_LARGE, large_rows)


def instantiate_node(stream, node):
    table = stream.createAt("table", "TempTable", 760, 360)
    stream.link(node, table)
    try:
        table.run([])
    finally:
        stream.delete(table)


def build_case(case_name, props, csv_name=None):
    if csv_name is None:
        csv_name = CSV_MAIN
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
             "input": csv_name, "properties": recorded, "status": "error"}
    try:
        stream = taskrunner.createStream(case_name, False, False)
        stream.setPropertyValue("decimal_places", 15)
        src = stream.createAt("variablefile", csv_name, 100, 100)
        src.setPropertyValue("full_filename", DIR + "/" + csv_name)
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

# Table 101, printed pages 192-194. Invalid properties are ignored (p.190), so each case
# sets only the slots named below unless the case tests extra slots.

# Control for H1-H3, T3, L2: histogram_amount (Histogram, x_field amount).
# H1: constant x. Same slots, constant data file. GAP item 1.
build_case("hist_constant_amount", [
    ("graph_type", "Histogram"),
    ("x_field", "amount"),
], CSV_CONST)

# H2: different Heckbert ceiling (score span ceilings to 10, amount to 20).
# Control histogram_amount. GAP item 1.
build_case("hist_score_range", [
    ("graph_type", "Histogram"),
    ("x_field", "score"),
])

# H3: different Heckbert ceiling (weight span ceilings to 1.0). Control histogram_amount.
# GAP item 1.
build_case("hist_weight_range", [
    ("graph_type", "Histogram"),
    ("x_field", "weight"),
])

# B1: several outliers in one category (East 20x 5.0 plus 100,101,102).
# Control boxplot_region_amount. GAP item 2.
build_case("box_many_outliers", [
    ("graph_type", "Boxplot"),
    ("x_field", "region"),
    ("y_field", "amount"),
], CSV_BOX_OUT)

# B2: a category with no present y (East amount always blank).
# Control boxplot_region_amount. GAP item 2.
build_case("box_empty_category", [
    ("graph_type", "Boxplot"),
    ("x_field", "region"),
    ("y_field", "amount"),
], CSV_BOX_EMPTY)

# S1: colour on a summed Bar. Control bar_region_amount. GAP item 4.
# color_field (p.194); BarCounts-with-color use is unverified, summed-Bar use is new.
build_case("bar_color_summed", [
    ("graph_type", "Bar"),
    ("categories_field", "region"),
    ("values_field", "amount"),
    ("color_field", "segment"),
])

# S2: colour on a Pie. Control pie_categories. GAP item 4.
build_case("pie_color_summed", [
    ("graph_type", "Pie"),
    ("categories_field", "region"),
    ("values_field", "amount"),
    ("color_field", "segment"),
])

# S3: Bar on x/y instead of categories/values. Control bar_region_amount. GAP item 4.
build_case("bar_xy", [
    ("graph_type", "Bar"),
    ("x_field", "region"),
    ("y_field", "amount"),
])

# S4: Pie on x/y instead of categories/values. Control pie_categories. GAP item 4.
build_case("pie_xy", [
    ("graph_type", "Pie"),
    ("x_field", "region"),
    ("y_field", "amount"),
])

# S5: Heatmap on x/y instead of rows/columns/color. Control heatmap_rows. GAP item 4.
# rows_field, columns_field, color_field (p.193-194) are replaced by axis bindings.
build_case("heatmap_xy", [
    ("graph_type", "Heatmap"),
    ("x_field", "score"),
    ("y_field", "amount"),
])

# P1: panel fields on a Line. Control line_score_amount. GAP item 4.
# panelrow_field, panelcol_field (p.194, typed string).
build_case("line_panel", [
    ("graph_type", "Line"),
    ("x_field", "score"),
    ("y_field", "amount"),
    ("panelrow_field", "region"),
    ("panelcol_field", "channel"),
])

# P2: panel fields on a summed Bar. Control bar_region_amount. GAP item 4.
build_case("bar_panel", [
    ("graph_type", "Bar"),
    ("categories_field", "region"),
    ("values_field", "amount"),
    ("panelrow_field", "region"),
    ("panelcol_field", "channel"),
])

# A1: animation field on a Scatterplot. Control scatter_score_amount. GAP item 5.
# animation_field (p.194, typed string).
build_case("scatter_animation", [
    ("graph_type", "Scatterplot"),
    ("x_field", "score"),
    ("y_field", "amount"),
    ("animation_field", "segment"),
])

# T1: 3-D scatterplot type with the same x/y. Control scatter_score_amount. GAP item 5.
# 3DScatterplot (p.192).
build_case("scatter_3d_plain", [
    ("graph_type", "3DScatterplot"),
    ("x_field", "score"),
    ("y_field", "amount"),
])

# T2: z field on a 3-D scatterplot. Control scatter_3d_plain (new). GAP item 5.
# z_field (p.194, used in some 3-D graphs).
build_case("scatter_3d_z", [
    ("graph_type", "3DScatterplot"),
    ("x_field", "score"),
    ("y_field", "amount"),
    ("z_field", "weight"),
])

# T3: 3-D histogram type with the same x. Control histogram_amount. GAP item 5.
# 3DHistogram (p.192).
build_case("hist_3d", [
    ("graph_type", "3DHistogram"),
    ("x_field", "amount"),
])

# T4: 3-D bar type with the same category/value. Control bar_region_amount. GAP item 5.
# 3DBar (p.192).
build_case("bar_3d", [
    ("graph_type", "3DBar"),
    ("categories_field", "region"),
    ("values_field", "amount"),
])

# M1: map type with the same category/value. Control bar_region_amount. GAP item 5.
# BarMap (p.192).
build_case("bar_map_plain", [
    ("graph_type", "BarMap"),
    ("categories_field", "region"),
    ("values_field", "amount"),
])

# M2: coordinate map with longitude/latitude stand-ins. Control scatter_score_amount
# (same numeric fields, new slots). GAP item 5. longitude_field, latitude_field (p.194).
build_case("map_coordinate_slots", [
    ("graph_type", "CoordinateMap"),
    ("longitude_field", "score"),
    ("latitude_field", "amount"),
])

# M3: choropleth map with a key and a colour stand-in. Control pie_categories
# (same region/amount fields, new slots). GAP item 5.
# data_key_field, map_color_field (p.194).
build_case("map_choropleth_slots", [
    ("graph_type", "ChoroplethMeans"),
    ("data_key_field", "region"),
    ("map_color_field", "amount"),
])

# L1: 2500-record input on a Scatterplot. Control scatter_score_amount. GAP item 6.
build_case("scatter_large_2500", [
    ("graph_type", "Scatterplot"),
    ("x_field", "score"),
    ("y_field", "amount"),
], CSV_LARGE)

# L2: 2500-record input on a Histogram. Control histogram_amount. GAP item 6.
build_case("hist_large_2500", [
    ("graph_type", "Histogram"),
    ("x_field", "amount"),
], CSV_LARGE)

handle = open(OUT_DIR + "/manifest.json", "w")
try:
    json.dump(manifest, handle, indent=2, sort_keys=True)
finally:
    handle.close()
