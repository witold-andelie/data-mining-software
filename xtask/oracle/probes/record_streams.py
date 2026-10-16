# Probe: build deterministic Record Operations corpus streams and input CSVs in corpus/records/.
# Each stream tests one setting family and ends in a Table node, with decimal_places = 15.
# Run with `cargo xtask probe record_streams` (IBM SPSS Modeler, Jython 2.7, public API only).
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
RECORDS_DIR = "D:/data-mining-software/corpus/records"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"saved": [], "errors": [], "cases": [], "notes": {}}

REC_MAIN_CSV = [
    "id,grp,sub,iv,rv,s",
    "1,B,y,10,-3.5,banana",
    "2,B,x,20,0.0,Apple",
    "3,A,x,-10,1.25,Banana",
    "4,A,y,0,2.5,apple",
    "5,B,y,10,1.5,Apple",
    "6,A,x,10,5.0,zebra",
    "7,C,z,7,-2.25,Zebra",
    "8,,x,5,0.5,apple",
    "9,B,x,40,4.5,Banana",
    "10,A,y,30,10.0,banana",
    "11,C,,,,",
    "12,B,z,80,9.0,cherry",
    "13,,,15,3.5,Cherry",
    "14,A,x,70,,APPLE",
    "15,B,,,18.5,",
    "16,D,z,,,banana",
]

MERGE_LEFT_CSV = [
    "k,l_id,l_val,l_num",
    "B,1,10,1.5",
    "A,2,20,-2.0",
    "L1,3,30,3.25",
    ",4,40,0.0",
    "B,5,50,4.5",
    "C,6,,6.0",
    ",7,-10,1.0",
]

MERGE_RIGHT_CSV = [
    "k,r_id,r_val,r_num",
    "B,101,100,-1.0",
    ",102,200,2.5",
    "A,103,300,3.0",
    "R1,104,400,",
    "B,105,500,-5.5",
]

MERGE_ORDER_RIGHT_CSV = [
    "r_k,r_id,r_val,r_num",
    "B,101,100,-1.0",
    ",102,200,2.5",
    "A,103,300,3.0",
    "R1,104,400,",
    "B,105,500,-5.5",
]

MERGE_DUP_LEFT_CSV = [
    "k,l_id,val,l_num",
    "B,1,10,1.5",
    "A,2,20,-2.0",
    "L1,3,30,3.25",
    ",4,40,0.0",
    "B,5,50,4.5",
    "C,6,,6.0",
    ",7,-10,1.0",
]

MERGE_DUP_RIGHT_CSV = [
    "k,r_id,val,r_num",
    "B,101,100,-1.0",
    ",102,200,2.5",
    "A,103,300,3.0",
    "R1,104,400,",
    "B,105,500,-5.5",
]

APPEND_1_CSV = [
    "id,Name,num,only_main",
    "1,Alice,10,m1",
    "2,bob,-5,",
    "3,,0,m3",
]

APPEND_2_CSV = [
    "id,name,num,extra_col,fifth",
    "4,Charlie,2.5,e1,100",
    "5,DAVE,-1.25,e2,200",
]

ALL_STATS = [
    "Sum",
    "Mean",
    "Min",
    "Max",
    "SDev",
    "Median",
    "Count",
    "Variance",
    "FirstQuartile",
    "ThirdQuartile",
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
    path = "%s/%s" % (RECORDS_DIR, file_name)
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


def new_stream(case_name, csv_files):
    stream = taskrunner.createStream(case_name, False, False)
    stream.setPropertyValue("decimal_places", 15)
    sources = []
    for i, csv_name in enumerate(csv_files):
        src = stream.createAt("variablefile", csv_name, 100, 100 + 100 * i)
        src.setPropertyValue("full_filename", "%s/%s" % (RECORDS_DIR, csv_name))
        src.setPropertyValue("read_field_names", True)
        instantiate_node(stream, src)
        sources.append(src)
    return stream, sources


def finish_case(case_name, stream, op_node):
    table = stream.createAt("table", "Table", 500, 100)
    stream.link(op_node, table)
    table.run([])
    file_name = "%s.str" % case_name
    path = "%s/%s" % (RECORDS_DIR, file_name)
    taskrunner.saveStreamToFile(stream, path)
    manifest["saved"].append(file_name)
    manifest["cases"].append({"case": case_name, "file": file_name, "type": unicode(op_node.getTypeName())})


def build_case(case_name, csv_files, node_type, configure):
    stream = None
    try:
        stream, sources = new_stream(case_name, csv_files)
        op_node = stream.createAt(node_type, case_name, 300, 100)
        for src in sources:
            stream.link(src, op_node)
        configure(stream, op_node, sources)
        finish_case(case_name, stream, op_node)
    except ERRORS, e:
        manifest["errors"].append({"case": case_name, "message": message(e)})
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS:
                pass


ensure_clean_dir(RECORDS_DIR)
write_csv("rec_main.csv", REC_MAIN_CSV)
write_csv("merge_left.csv", MERGE_LEFT_CSV)
write_csv("merge_right.csv", MERGE_RIGHT_CSV)
write_csv("merge_order_right.csv", MERGE_ORDER_RIGHT_CSV)
write_csv("merge_dup_left.csv", MERGE_DUP_LEFT_CSV)
write_csv("merge_dup_right.csv", MERGE_DUP_RIGHT_CSV)
write_csv("append_1.csv", APPEND_1_CSV)
write_csv("append_2.csv", APPEND_2_CSV)


# --- 1. Aggregate cases ---

def cfg_agg_all_stats(stream, node, sources):
    node.setPropertyValue("keys", ["grp"])
    node.setPropertyValue("contiguous", False)
    node.setKeyedPropertyValue("aggregates", "iv", ALL_STATS)
    node.setKeyedPropertyValue("aggregates", "rv", ALL_STATS)
    node.setPropertyValue("inc_record_count", True)


def cfg_agg_ext_prefix(stream, node, sources):
    node.setPropertyValue("keys", ["grp"])
    node.setKeyedPropertyValue("aggregates", "iv", ["Sum", "Max", "Count"])
    node.setKeyedPropertyValue("aggregates", "rv", ["Mean", "SDev", "FirstQuartile", "ThirdQuartile"])
    node.setPropertyValue("extension", "PRE_")
    node.setPropertyValue("add_as", "Prefix")
    node.setPropertyValue("inc_record_count", True)


def cfg_agg_ext_suffix(stream, node, sources):
    node.setPropertyValue("keys", ["grp"])
    node.setKeyedPropertyValue("aggregates", "iv", ["Sum", "Max", "Count"])
    node.setKeyedPropertyValue("aggregates", "rv", ["Mean", "SDev", "Median", "Variance"])
    node.setPropertyValue("extension", "_SUF")
    node.setPropertyValue("add_as", "Suffix")
    node.setPropertyValue("inc_record_count", True)
    node.setPropertyValue("count_field", "N_Recs")


def cfg_agg_no_count(stream, node, sources):
    node.setPropertyValue("keys", ["grp", "sub"])
    node.setKeyedPropertyValue("aggregates", "iv", ["Sum", "Count"])
    node.setKeyedPropertyValue("aggregates", "rv", ["Mean", "Min", "Max"])
    node.setPropertyValue("inc_record_count", False)


def cfg_agg_contiguous(stream, node, sources):
    node.setPropertyValue("keys", ["grp"])
    node.setPropertyValue("contiguous", True)
    node.setKeyedPropertyValue("aggregates", "iv", ["Sum", "Count"])
    node.setKeyedPropertyValue("aggregates", "rv", ["Mean"])
    node.setPropertyValue("inc_record_count", True)


def cfg_agg_no_keys(stream, node, sources):
    node.setPropertyValue("keys", [])
    node.setKeyedPropertyValue("aggregates", "iv", ALL_STATS)
    node.setKeyedPropertyValue("aggregates", "rv", ALL_STATS)
    node.setPropertyValue("inc_record_count", True)


build_case("agg_all_stats", ["rec_main.csv"], "aggregate", cfg_agg_all_stats)
build_case("agg_ext_prefix", ["rec_main.csv"], "aggregate", cfg_agg_ext_prefix)
build_case("agg_ext_suffix", ["rec_main.csv"], "aggregate", cfg_agg_ext_suffix)
build_case("agg_no_count", ["rec_main.csv"], "aggregate", cfg_agg_no_count)
build_case("agg_contiguous", ["rec_main.csv"], "aggregate", cfg_agg_contiguous)
build_case("agg_no_keys", ["rec_main.csv"], "aggregate", cfg_agg_no_keys)


# --- 2. Sort cases ---

def cfg_sort_multi_keys(stream, node, sources):
    node.setPropertyValue("keys", [["grp", "Ascending"], ["iv", "Descending"]])


def cfg_sort_strings_nulls(stream, node, sources):
    node.setPropertyValue("keys", [["s", "Ascending"], ["rv", "Descending"]])


def cfg_sort_desc_ties(stream, node, sources):
    node.setPropertyValue("keys", [["iv", "Descending"], ["grp", "Descending"]])


build_case("sort_multi_keys", ["rec_main.csv"], "sort", cfg_sort_multi_keys)
build_case("sort_strings_nulls", ["rec_main.csv"], "sort", cfg_sort_strings_nulls)
build_case("sort_desc_ties", ["rec_main.csv"], "sort", cfg_sort_desc_ties)


# --- 3. Merge cases ---

def cfg_merge_order(stream, node, sources):
    node.setPropertyValue("method", "Order")


def cfg_merge_key_inner(stream, node, sources):
    node.setPropertyValue("method", "Keys")
    node.setPropertyValue("key_fields", ["k"])
    node.setPropertyValue("common_keys", True)
    node.setPropertyValue("join", "Inner")


def cfg_merge_key_full_outer(stream, node, sources):
    node.setPropertyValue("method", "Keys")
    node.setPropertyValue("key_fields", ["k"])
    node.setPropertyValue("common_keys", True)
    node.setPropertyValue("join", "FullOuter")


def cfg_merge_key_partial_outer(stream, node, sources):
    node.setPropertyValue("method", "Keys")
    node.setPropertyValue("key_fields", ["k"])
    node.setPropertyValue("common_keys", True)
    node.setPropertyValue("join", "PartialOuter")
    node.setKeyedPropertyValue("outer_join_tag", "1", True)


def cfg_merge_key_anti(stream, node, sources):
    node.setPropertyValue("method", "Keys")
    node.setPropertyValue("key_fields", ["k"])
    node.setPropertyValue("common_keys", True)
    node.setPropertyValue("join", "Anti")


def cfg_merge_dup_rename(stream, node, sources):
    node.setPropertyValue("method", "Keys")
    node.setPropertyValue("key_fields", ["k"])
    node.setPropertyValue("common_keys", True)
    node.setPropertyValue("join", "Inner")
    node.setPropertyValue("rename_duplicate_fields", False)
    node.setPropertyValue("rename_duplicate_fields", True)
    methods = [unicode(m) for m in dir(node) if "filter" in m.lower() or "field" in m.lower() or "rename" in m.lower() or "table" in m.lower()]
    manifest["notes"]["merge_methods"] = methods


def cfg_merge_condition(stream, node, sources):
    node.setPropertyValue("method", "Condition")
    node.setPropertyValue("condition", "l_id = r_id - 100 and l_num >= r_num")


build_case("merge_order", ["merge_left.csv", "merge_order_right.csv"], "merge", cfg_merge_order)
build_case("merge_key_inner", ["merge_left.csv", "merge_right.csv"], "merge", cfg_merge_key_inner)
build_case("merge_key_full_outer", ["merge_left.csv", "merge_right.csv"], "merge", cfg_merge_key_full_outer)
build_case("merge_key_partial_outer", ["merge_left.csv", "merge_right.csv"], "merge", cfg_merge_key_partial_outer)
build_case("merge_key_anti", ["merge_dup_left.csv", "merge_dup_right.csv"], "merge", cfg_merge_key_anti)
build_case("merge_dup_rename", ["merge_dup_left.csv", "merge_dup_right.csv"], "merge", cfg_merge_dup_rename)
build_case("merge_condition", ["merge_dup_left.csv", "merge_dup_right.csv"], "merge", cfg_merge_condition)


# --- 4. Append cases ---

def cfg_append_name_main(stream, node, sources):
    node.setPropertyValue("match_by", "Name")
    node.setPropertyValue("match_case", False)
    node.setPropertyValue("include_fields_from", "Main")
    node.setPropertyValue("create_tag_field", True)


def cfg_append_name_all_case(stream, node, sources):
    node.setPropertyValue("match_by", "Name")
    node.setPropertyValue("match_case", True)
    node.setPropertyValue("include_fields_from", "All")
    node.setPropertyValue("create_tag_field", True)
    node.setPropertyValue("tag_field_name", "Src")


def cfg_append_position_all(stream, node, sources):
    node.setPropertyValue("match_by", "Position")
    node.setPropertyValue("include_fields_from", "All")
    node.setPropertyValue("create_tag_field", False)


def cfg_append_position_main(stream, node, sources):
    node.setPropertyValue("match_by", "Position")
    node.setPropertyValue("include_fields_from", "Main")
    node.setPropertyValue("create_tag_field", True)


build_case("append_name_main", ["append_1.csv", "append_2.csv"], "append", cfg_append_name_main)
build_case("append_name_all_case", ["append_1.csv", "append_2.csv"], "append", cfg_append_name_all_case)
build_case("append_position_all", ["append_1.csv", "append_2.csv"], "append", cfg_append_position_all)
build_case("append_position_main", ["append_1.csv", "append_2.csv"], "append", cfg_append_position_main)


# --- 5. Sample cases ---

def cfg_sample_first_n(stream, node, sources):
    node.setPropertyValue("method", "Simple")
    node.setPropertyValue("mode", "Include")
    node.setPropertyValue("sample_type", "First")
    node.setPropertyValue("first_n", 5)


def cfg_sample_first_discard(stream, node, sources):
    node.setPropertyValue("method", "Simple")
    node.setPropertyValue("mode", "Discard")
    node.setPropertyValue("sample_type", "First")
    node.setPropertyValue("first_n", 5)


def cfg_sample_one_in_n(stream, node, sources):
    node.setPropertyValue("method", "Simple")
    node.setPropertyValue("mode", "Include")
    node.setPropertyValue("sample_type", "OneInN")
    node.setPropertyValue("one_in_n", 3)


def cfg_sample_one_in_n_max(stream, node, sources):
    node.setPropertyValue("method", "Simple")
    node.setPropertyValue("mode", "Include")
    node.setPropertyValue("sample_type", "OneInN")
    node.setPropertyValue("one_in_n", 2)
    node.setPropertyValue("use_max_size", True)
    node.setPropertyValue("maximum_size", 3)


def cfg_sample_one_in_n_discard_max(stream, node, sources):
    node.setPropertyValue("method", "Simple")
    node.setPropertyValue("mode", "Discard")
    node.setPropertyValue("sample_type", "OneInN")
    node.setPropertyValue("one_in_n", 2)
    node.setPropertyValue("use_max_size", True)
    node.setPropertyValue("maximum_size", 3)


build_case("sample_first_n", ["rec_main.csv"], "sample", cfg_sample_first_n)
build_case("sample_first_discard", ["rec_main.csv"], "sample", cfg_sample_first_discard)
build_case("sample_one_in_n", ["rec_main.csv"], "sample", cfg_sample_one_in_n)
build_case("sample_one_in_n_max", ["rec_main.csv"], "sample", cfg_sample_one_in_n_max)
build_case("sample_one_in_n_discard_max", ["rec_main.csv"], "sample", cfg_sample_one_in_n_discard_max)


# --- 6. Balance cases ---

def cfg_balance_int_factors(stream, node, sources):
    node.setPropertyValue("training_data_only", False)
    node.setPropertyValue("directives", [
        [2.0, 'grp = "A"'],
        [3.0, 'grp = "C"'],
        [1.0, 'grp = "B"'],
    ])


def cfg_balance_multi_match(stream, node, sources):
    node.setPropertyValue("training_data_only", False)
    node.setPropertyValue("directives", [
        [2.0, 'grp = "A"'],
        [3.0, "iv > 0"],
    ])


build_case("balance_int_factors", ["rec_main.csv"], "balance", cfg_balance_int_factors)
build_case("balance_multi_match", ["rec_main.csv"], "balance", cfg_balance_multi_match)


# --- 7. Distinct and Restructure cases ---

def cfg_distinct_include(stream, node, sources):
    node.setPropertyValue("mode", "Include")
    node.setPropertyValue("grouping_fields", ["grp"])
    node.setPropertyValue("inc_record_count", True)


def cfg_distinct_discard(stream, node, sources):
    node.setPropertyValue("mode", "Discard")
    node.setPropertyValue("grouping_fields", ["grp"])


def cfg_restructure_flags(stream, node, sources):
    node.setKeyedPropertyValue("fields_from", "grp", ["A", "B"])
    node.setPropertyValue("value_mode", "Flags")
    node.setPropertyValue("include_field_name", True)


def cfg_restructure_values(stream, node, sources):
    node.setKeyedPropertyValue("fields_from", "grp", ["A", "B"])
    node.setPropertyValue("value_mode", "OtherFields")
    node.setPropertyValue("value_fields", ["iv", "rv"])
    node.setPropertyValue("include_field_name", True)


build_case("distinct_include", ["rec_main.csv"], "distinct", cfg_distinct_include)
build_case("distinct_discard", ["rec_main.csv"], "distinct", cfg_distinct_discard)
build_case("restructure_flags", ["rec_main.csv"], "restructure", cfg_restructure_flags)
build_case("restructure_values", ["rec_main.csv"], "restructure", cfg_restructure_values)


handle = open(OUT_DIR + "/manifest.json", "w")
try:
    json.dump(manifest, handle, indent=2, sort_keys=True)
finally:
    handle.close()
