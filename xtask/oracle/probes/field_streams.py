# Probe: build deterministic Field Operations corpus streams and input CSVs in corpus/fields/.
# Each stream is source -> Type -> the node under test -> Table, with decimal_places = 15.
# Run with `cargo xtask probe field_streams` (IBM SPSS Modeler, Jython 2.7, public API only).
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
FIELDS_DIR = "D:/data-mining-software/corpus/fields"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"saved": [], "errors": [], "cases": []}

# 16 rows. x/y: negatives, reals, ties, and a null. grp repeats (aggregate key).
# cat is the set-to-flag field. note has a quoted empty string and blank cells.
# row_name is a unique instantiated field for transpose names.
# Columns: id Range, grp/cat/note/row_name Set, flg Flag, x/y Range.
FIELDS_MAIN_CSV = [
    "id,grp,cat,flg,x,y,note,row_name",
    "1,A,red,T,-10,-3.5,alpha,r01",
    "2,A,red,F,-10,0,beta,r02",
    "3,A,blue,T,-2.5,1.25,\"\",r03",
    "4,A,blue,F,0,-2.5,Apple,r04",
    "5,B,green,T,0,4.5,zebra,r05",
    "6,B,green,F,0,10,apple,r06",
    "7,B,green,T,2.5,-1.5,Banana,r07",
    "8,C,blue,F,,2.5,cherry,r08",
    "9,C,green,T,5,8,Cherry,r09",
    "10,C,blue,F,5,0,,r10",
    "11,A,red,T,7.5,-3.5,alpha,r11",
    "12,,blue,F,10,,banana,r12",
    "13,B,red,T,10,3.5,,r13",
    "14,C,,F,10,1.25,zebra,r14",
    "15,B,green,T,15,9,gamma,r15",
    "16,C,blue,F,20,-0.5,delta,r16",
]

SCHEMA = [
    ("id", "Range"),
    ("grp", "Set"),
    ("cat", "Set"),
    ("flg", "Flag"),
    ("x", "Range"),
    ("y", "Range"),
    ("note", "Set"),
    ("row_name", "Set"),
]

DATA_FILE = "fields_main.csv"


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
    path = "%s/%s" % (FIELDS_DIR, file_name)
    handle = open(path, "w")
    try:
        handle.write(NL.join(lines) + NL)
    finally:
        handle.close()
    manifest["saved"].append(file_name)


def snapshot(value):
    if isinstance(value, list):
        return [snapshot(item) for item in value]
    return value


def set_props(node, items):
    # A 3-tuple is a keyed property (name, key, value). A 2-tuple is a plain property.
    recorded = []
    for item in items:
        if len(item) == 3:
            name, key, value = item
            node.setKeyedPropertyValue(name, key, value)
            recorded.append({"property": name, "key": key, "value": snapshot(value)})
        else:
            name, value = item
            node.setPropertyValue(name, value)
            recorded.append({"property": name, "value": snapshot(value)})
    return recorded


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
    src.setPropertyValue("full_filename", "%s/%s" % (FIELDS_DIR, csv_name))
    src.setPropertyValue("read_field_names", True)
    type_node = stream.createAt("type", "Type", 250, 100)
    stream.link(src, type_node)
    instantiate_node(stream, type_node)
    for field_name, measure in SCHEMA:
        type_node.setKeyedPropertyValue("type", field_name, measure)
    instantiate_node(stream, type_node)
    return stream, type_node


def finish_case(case_name, stream, op_node, csv_name, node_type, properties):
    table = stream.createAt("table", "Table", 560, 100)
    stream.link(op_node, table)
    table.run([])
    file_name = "%s.str" % case_name
    path = "%s/%s" % (FIELDS_DIR, file_name)
    taskrunner.saveStreamToFile(stream, path)
    manifest["saved"].append(file_name)
    manifest["cases"].append({
        "case": case_name,
        "file": file_name,
        "data": csv_name,
        "type": unicode(op_node.getTypeName()),
        "node_type": node_type,
        "properties": properties,
    })


def build_case(case_name, csv_name, node_type, configure):
    stream = None
    try:
        stream, type_node = new_stream(case_name, csv_name)
        op_node = stream.createAt(node_type, case_name, 400, 100)
        stream.link(type_node, op_node)
        properties = configure(stream, op_node, type_node)
        finish_case(case_name, stream, op_node, csv_name, node_type, properties)
    except ERRORS as e:
        manifest["errors"].append({"case": case_name, "message": message(e)})
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS:
                pass


ensure_clean_dir(FIELDS_DIR)
write_csv(DATA_FILE, FIELDS_MAIN_CSV)


# --- 1. Binning ---
# Tie values are the guide enums: Next (add to next) and Current (keep in current).

def cfg_bin_width_x(stream, node, type_node):
    return set_props(node, [
        ("fields", ["x"]),
        ("method", "FixedWidth"),
        ("fixed_bin_method", "Width"),
        ("fixed_bin_width", 5.0),
    ])


def cfg_bin_count_xy(stream, node, type_node):
    return set_props(node, [
        ("fields", ["x", "y"]),
        ("method", "FixedWidth"),
        ("fixed_bin_method", "Count"),
        ("fixed_bin_count", 4),
    ])


def cfg_bin_quartile_next_x(stream, node, type_node):
    return set_props(node, [
        ("fields", ["x"]),
        ("method", "EqualCount"),
        ("equal_count_method", "RecordCount"),
        ("tile4", True),
        ("tile10", False),
        ("tied_values_method", "Next"),
    ])


def cfg_bin_decile_current_xy(stream, node, type_node):
    return set_props(node, [
        ("fields", ["x", "y"]),
        ("method", "EqualCount"),
        ("equal_count_method", "RecordCount"),
        ("tile4", False),
        ("tile10", True),
        ("tied_values_method", "Current"),
    ])


def cfg_bin_rank_x(stream, node, type_node):
    return set_props(node, [
        ("fields", ["x"]),
        ("method", "Rank"),
        ("rank", True),
        ("rank_fractional", False),
        ("rank_pct", False),
    ])


def cfg_bin_rank_fractional_x(stream, node, type_node):
    return set_props(node, [
        ("fields", ["x"]),
        ("method", "Rank"),
        ("rank", False),
        ("rank_fractional", True),
        ("rank_pct", False),
    ])


def cfg_bin_rank_pct_xy(stream, node, type_node):
    return set_props(node, [
        ("fields", ["x", "y"]),
        ("method", "Rank"),
        ("rank", False),
        ("rank_fractional", False),
        ("rank_pct", True),
    ])


def cfg_bin_sdev1_x(stream, node, type_node):
    return set_props(node, [
        ("fields", ["x"]),
        ("method", "SDev"),
        ("sdev_count", "One"),
    ])


def cfg_bin_sdev2_xy(stream, node, type_node):
    return set_props(node, [
        ("fields", ["x", "y"]),
        ("method", "SDev"),
        ("sdev_count", "Two"),
    ])


build_case("bin_width_x", DATA_FILE, "binning", cfg_bin_width_x)
build_case("bin_count_xy", DATA_FILE, "binning", cfg_bin_count_xy)
build_case("bin_quartile_next_x", DATA_FILE, "binning", cfg_bin_quartile_next_x)
build_case("bin_decile_current_xy", DATA_FILE, "binning", cfg_bin_decile_current_xy)
build_case("bin_rank_x", DATA_FILE, "binning", cfg_bin_rank_x)
build_case("bin_rank_fractional_x", DATA_FILE, "binning", cfg_bin_rank_fractional_x)
build_case("bin_rank_pct_xy", DATA_FILE, "binning", cfg_bin_rank_pct_xy)
build_case("bin_sdev1_x", DATA_FILE, "binning", cfg_bin_sdev1_x)
build_case("bin_sdev2_xy", DATA_FILE, "binning", cfg_bin_sdev2_xy)


# --- 2. Set to Flag ---
# The guide property for grouping by a key is `aggregate` (with `keys`), not use_aggregate.

def cfg_flag_default(stream, node, type_node):
    return set_props(node, [
        ("fields_from", "cat", ["red", "blue", "green"]),
        ("true_value", "T"),
        ("false_value", "F"),
    ])


def cfg_flag_custom_values(stream, node, type_node):
    return set_props(node, [
        ("fields_from", "cat", ["red", "blue", "green"]),
        ("true_value", "1"),
        ("false_value", "0"),
    ])


def cfg_flag_aggregate(stream, node, type_node):
    return set_props(node, [
        ("fields_from", "cat", ["red", "blue", "green"]),
        ("true_value", "T"),
        ("false_value", "F"),
        ("aggregate", True),
        ("keys", ["grp"]),
    ])


def cfg_flag_ext_prefix(stream, node, type_node):
    return set_props(node, [
        ("fields_from", "cat", ["red", "blue", "green"]),
        ("true_value", "T"),
        ("false_value", "F"),
        ("use_extension", True),
        ("extension", "f_"),
        ("add_as", "Prefix"),
    ])


def cfg_flag_ext_suffix(stream, node, type_node):
    return set_props(node, [
        ("fields_from", "cat", ["red", "blue", "green"]),
        ("true_value", "T"),
        ("false_value", "F"),
        ("use_extension", True),
        ("extension", "_f"),
        ("add_as", "Suffix"),
    ])


build_case("flag_default", DATA_FILE, "settoflag", cfg_flag_default)
build_case("flag_custom_values", DATA_FILE, "settoflag", cfg_flag_custom_values)
build_case("flag_aggregate", DATA_FILE, "settoflag", cfg_flag_aggregate)
build_case("flag_ext_prefix", DATA_FILE, "settoflag", cfg_flag_ext_prefix)
build_case("flag_ext_suffix", DATA_FILE, "settoflag", cfg_flag_ext_suffix)


# --- 3. Transpose ---
# Normal transpose (the default). Numeric is every continuous field; Custom is a subset.

def cfg_transpose_all_prefix(stream, node, type_node):
    return set_props(node, [
        ("transpose_type", "Numeric"),
        ("transposed_names", "Prefix"),
        ("prefix", "col"),
        ("num_new_fields", 20),
        ("id_field_name", "src_field"),
    ])


def cfg_transpose_subset_prefix(stream, node, type_node):
    return set_props(node, [
        ("transpose_type", "Custom"),
        ("transpose_fields", ["x", "y"]),
        ("transposed_names", "Prefix"),
        ("prefix", "v"),
        ("num_new_fields", 20),
        ("id_field_name", "src_field"),
    ])


def cfg_transpose_all_from_field(stream, node, type_node):
    return set_props(node, [
        ("transpose_type", "Numeric"),
        ("transposed_names", "Read"),
        ("read_from_field", "row_name"),
        ("max_num_fields", "20"),
        ("id_field_name", "src_field"),
    ])


def cfg_transpose_subset_from_field(stream, node, type_node):
    return set_props(node, [
        ("transpose_type", "Custom"),
        ("transpose_fields", ["x", "y"]),
        ("transposed_names", "Read"),
        ("read_from_field", "row_name"),
        ("max_num_fields", "20"),
        ("id_field_name", "src_field"),
    ])


build_case("transpose_all_prefix", DATA_FILE, "transpose", cfg_transpose_all_prefix)
build_case("transpose_subset_prefix", DATA_FILE, "transpose", cfg_transpose_subset_prefix)
build_case("transpose_all_from_field", DATA_FILE, "transpose", cfg_transpose_all_from_field)
build_case("transpose_subset_from_field", DATA_FILE, "transpose", cfg_transpose_subset_from_field)


# --- 4. Field Reorder ---

def cfg_reorder_custom(stream, node, type_node):
    return set_props(node, [
        ("mode", "Custom"),
        ("start_fields", ["note", "y", "flg"]),
        ("end_fields", ["row_name", "id"]),
    ])


def cfg_reorder_name_asc(stream, node, type_node):
    return set_props(node, [
        ("mode", "Auto"),
        ("sort_by", "Name"),
        ("ascending", True),
    ])


def cfg_reorder_name_desc(stream, node, type_node):
    return set_props(node, [
        ("mode", "Auto"),
        ("sort_by", "Name"),
        ("ascending", False),
    ])


def cfg_reorder_by_type(stream, node, type_node):
    return set_props(node, [
        ("mode", "Auto"),
        ("sort_by", "Type"),
        ("ascending", True),
    ])


build_case("reorder_custom", DATA_FILE, "reorder", cfg_reorder_custom)
build_case("reorder_name_asc", DATA_FILE, "reorder", cfg_reorder_name_asc)
build_case("reorder_name_desc", DATA_FILE, "reorder", cfg_reorder_name_desc)
build_case("reorder_by_type", DATA_FILE, "reorder", cfg_reorder_by_type)


# --- 5. History ---
# Records shorter than the span are handled by `unavailable` (Fill or Discard).

def cfg_history_x_off1_discard(stream, node, type_node):
    return set_props(node, [
        ("fields", ["x"]),
        ("offset", 1),
        ("span", 2),
        ("unavailable", "Discard"),
    ])


def cfg_history_x_off3_fill(stream, node, type_node):
    return set_props(node, [
        ("fields", ["x"]),
        ("offset", 3),
        ("span", 2),
        ("unavailable", "Fill"),
        ("fill_with", -999),
    ])


def cfg_history_xy_off1_fill(stream, node, type_node):
    return set_props(node, [
        ("fields", ["x", "y"]),
        ("offset", 1),
        ("span", 2),
        ("unavailable", "Fill"),
        ("fill_with", -999),
    ])


def cfg_history_xy_off1_discard(stream, node, type_node):
    return set_props(node, [
        ("fields", ["x", "y"]),
        ("offset", 1),
        ("span", 2),
        ("unavailable", "Discard"),
    ])


build_case("history_x_off1_discard", DATA_FILE, "history", cfg_history_x_off1_discard)
build_case("history_x_off3_fill", DATA_FILE, "history", cfg_history_x_off3_fill)
build_case("history_xy_off1_fill", DATA_FILE, "history", cfg_history_xy_off1_fill)
build_case("history_xy_off1_discard", DATA_FILE, "history", cfg_history_xy_off1_discard)


handle = open(OUT_DIR + "/manifest.json", "w")
try:
    json.dump(manifest, handle, indent=2, sort_keys=True)
finally:
    handle.close()
