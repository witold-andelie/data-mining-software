# Probe: Auto Data Prep ordinal inputs and target handling (label adp_more).
# Coordinator only: cargo xtask probe adp_more_streams, then corpus record adp_more and oracle.
# Same framework as adp_next_streams.py; only the data and the cases differ.
# Jython 2.7, public modeler.api only (Scripting Guide Table 79, autodataprepnode). Deterministic
# data: each case switches on one ADP step on data built to trigger it. Streams are saved before
# they run, like misc_next/adp_*: the node analyses its input when the Table executes.
import json
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/adp_more"
CSV_NAME = "adp_more.csv"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)
taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "adp_more", "saved": [], "errors": [], "cases": [], "inputs": []}

# (name, storage, measure, values)
SCHEMA = [("id", "Integer", "Range", None),
          ("yc", "Real", "Range", None),
          ("ycm", "Real", "Range", None),
          ("yco", "Real", "Range", None),
          ("yn", "String", "Set", ["a", "b", "c"]),
          ("ynm", "String", "Set", ["a", "b", "c"]),
          ("o4", "Integer", "OrderedSet", [1, 2, 3, 4]),
          ("om", "Integer", "OrderedSet", [1, 2, 3, 4, 5]),
          ("o12", "Integer", "OrderedSet", range(1, 13)),
          ("c1", "Real", "Range", None)]
ADP_PROPERTIES = ["objective", "replace_outliers_inputs", "outlier_cutoff_value", "outlier_method",
                  "rescale_continuous_inputs", "rescaling_method", "min_max_minimum",
                  "min_max_maximum", "z_score_final_mean", "z_score_final_sd",
                  "replace_missing_nominal_inputs", "replace_missing_continuous_inputs",
                  "transform_select_input_fields", "maximize_association_with_target",
                  "p_value_for_merging", "merge_ordinal_features", "merge_nominal_features",
                  "minimum_cases_in_category", "bin_continuous_fields", "p_value_for_binning",
                  "perform_feature_selection", "p_value_for_selection",
                  "perform_feature_construction", "rescale_continuous_target",
                  "target_final_mean", "target_final_sd", "prepare_inputs_and_target",
                  "exclude_low_quality_inputs", "excluded_fields"]


def message(exc):
    get = getattr(exc, "getMessage", None)
    if get is not None:
        return unicode(get())
    return unicode(exc)


def snapshot(value):
    if value is None or isinstance(value, (bool, int, long, float, basestring)):
        return value
    if isinstance(value, (list, tuple, java.util.List)):
        return [snapshot(item) for item in value]
    return unicode(value)


def write_manifest():
    handle = open(OUT_DIR + "/manifest.json", "w")
    try:
        json.dump(manifest, handle, indent=2, sort_keys=True)
    finally:
        handle.close()


def ensure_clean_dir(path):
    directory = java.io.File(path)
    if directory.exists():
        for child in directory.listFiles():
            if child.isFile() and unicode(child.getName()).endswith((".csv", ".str")):
                if not child.delete():
                    raise IOError("cannot remove " + unicode(child))
    elif not directory.mkdirs():
        raise IOError("cannot create " + path)


def rows():
    # 300 records. yc depends on o4 (levels 1 and 2 share an effect) and c1; ycm is yc with 9
    # missing values, yco yc with two outliers. yn/ynm are categories of yc (ynm with 8 missing).
    # om is an ordinal with a 4-record level 5 and 10 missing values; o12 has 12 levels.
    effect = {1: 0.0, 2: 0.0, 3: 2.5, 4: 5.0}
    out = []
    for i in range(300):
        o4 = 1 + (i * 7 + i // 5) % 4
        om = None if i % 30 == 7 else (5 if i in (11, 111, 211, 251) else 1 + (i * 3) % 4)
        o12 = 1 + (i * 5) % 12
        c1 = ((i * 37) % 101) / 10.0 - 5.0
        noise = ((i * 29) % 13 - 6) / 6.0
        yc = 10.0 + effect[o4] + 0.5 * c1 + noise
        ycm = None if i % 33 == 4 else yc
        yco = yc + (40.0 if i == 50 else (-35.0 if i == 160 else 0.0))
        yn = "a" if yc < 11.0 else ("b" if yc < 14.0 else "c")
        ynm = None if i % 37 == 2 else yn
        out.append([i + 1, yc, ycm, yco, yn, ynm, o4, om, o12, c1])
    return out


def write_data():
    lines = [",".join([f[0] for f in SCHEMA])]
    for row in rows():
        cells = []
        for value in row:
            if value is None:
                cells.append("")
            elif isinstance(value, basestring):
                cells.append('"' + value + '"')
            elif isinstance(value, float):
                cells.append("%.17g" % value)
            else:
                cells.append(str(value))
        lines.append(",".join(cells))
    handle = open(DIR + "/" + CSV_NAME, "wb")
    try:
        handle.write((NL.join(lines) + NL).encode("utf-8"))
    finally:
        handle.close()
    manifest["inputs"].append({"file": CSV_NAME, "rows": len(lines) - 1})


def instantiate(stream, node):
    table = stream.createAt("table", "Instantiate", 900, 700)
    stream.link(node, table)
    try:
        table.run([])
    finally:
        stream.delete(table)


def build(case, roles, props, tests):
    entry = {"case": case, "file": case + ".str", "roles": roles, "properties": props,
             "tests": tests, "status": "planned", "readback": []}
    manifest["cases"].append(entry)
    stream = None
    stage = "create"
    try:
        stream = taskrunner.createStream(case, False, False)
        stream.setPropertyValue("decimal_places", 15)
        source = stream.createAt("variablefile", CSV_NAME, 80, 100)
        for name, value in [("full_filename", DIR + "/" + CSV_NAME), ("read_field_names", True),
                            ("delimit_comma", True), ("quotes_2", "PairAndDiscard"),
                            ("strip_spaces", "None"), ("decimal_symbol", "Period"),
                            ("encoding", "UTF-8"), ("lines_to_scan", 1000)]:
            source.setPropertyValue(name, value)
        for field, storage, measure, values in SCHEMA:
            source.setKeyedPropertyValue("custom_storage_type", field, storage)
        stage = "type"
        type_node = stream.createAt("type", "Type", 280, 100)
        stream.link(source, type_node)
        instantiate(stream, type_node)
        for field, storage, measure, values in SCHEMA:
            type_node.setKeyedPropertyValue("type", field, measure)
            if values is not None:
                type_node.setKeyedPropertyValue("values", field, values)
                type_node.setKeyedPropertyValue("extend_values", field, False)
            type_node.setKeyedPropertyValue("direction", field, roles.get(field, "Input"))
            type_node.setKeyedPropertyValue("check", field, "None")
        instantiate(stream, type_node)
        stage = "configure"
        node = stream.createAt("autodataprep", case, 500, 100)
        stream.link(type_node, node)
        entry["defaults"] = read(node)
        for name, value in props:
            entry["setting_in_progress"] = [name, snapshot(value)]
            node.setPropertyValue(name, value)
            entry["readback"].append([name, snapshot(node.getPropertyValue(name))])
        entry.pop("setting_in_progress", None)
        entry["effective"] = read(node)
        table = stream.createAt("table", "Table", 760, 100)
        stream.link(node, table)
        stage = "save"
        taskrunner.saveStreamToFile(stream, DIR + "/" + entry["file"])
        manifest["saved"].append(entry["file"])
        entry["status"] = "saved"
        stage = "run"
        results = []
        table.run(results)
        content = results[0].getContentModel("table")
        entry["columns"] = [[unicode(content.getColumnName(c)), unicode(content.getStorageType(c))]
                            for c in range(content.getColumnCount())]
        entry["row_count"] = content.getRowCount()
        entry["status"] = "ran"
    except ERRORS as exc:
        manifest["errors"].append({"case": case, "stage": stage, "message": message(exc)})
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS as exc:
                manifest["errors"].append({"case": case, "stage": "clear", "message": message(exc)})
        write_manifest()


def read(node):
    values = {}
    for name in ADP_PROPERTIES:
        try:
            values[name] = snapshot(node.getPropertyValue(name))
        except ERRORS as exc:
            values[name] = {"read_error": message(exc)}
    return values




OTHERS = {"id": "None", "yc": "None", "ycm": "None", "yco": "None", "yn": "None", "ynm": "None"}


def roles(target=None):
    result = dict(OTHERS)
    if target is not None:
        result[target] = "Target"
    return result


CUSTOM = [("objective", "Custom")]
SELECT = CUSTOM + [("transform_select_input_fields", True)]

try:
    ensure_clean_dir(DIR)
    write_data()
    build("more_base", roles("yc"), CUSTOM, "Custom: ordinal inputs o4/om/o12 as they come")
    build("more_ordinal_missing", roles("yc"), CUSTOM + [("replace_missing_ordinal_inputs", True)],
          "om's missing values replaced (median?)")
    build("more_ordinal_merge_sup", roles("yc"), SELECT + [("maximize_association_with_target", True),
          ("merge_ordinal_features", True), ("p_value_for_merging", 0.05)],
          "Supervised merge of adjacent ordinal levels (o4: 1 and 2 share an effect)")
    build("more_ordinal_merge_unsup", roles(), SELECT + [("merge_ordinal_features", True),
          ("minimum_cases_in_category", 10)], "No target: ordinal merge by counts")
    build("more_target_missing", roles("ycm"), CUSTOM, "Continuous target with missing values")
    build("more_target_missing_replace", roles("ycm"), CUSTOM + [
          ("replace_missing_continuous_target", True)], "Target missing values replaced")
    build("more_target_boxcox_missing", roles("ycm"), CUSTOM + [("rescale_continuous_target", True)],
          "Box-Cox of a target with missing values")
    build("more_target_outliers", roles("yco"), CUSTOM + [("replace_outliers_target", True),
          ("outlier_cutoff_value", 3.0), ("outlier_method", "Replace")], "Target outliers coerced")
    build("more_nominal_target", roles("yn"), CUSTOM + [("reorder_nominal_target", True)],
          "Nominal target recoded")
    build("more_nominal_target_missing", roles("ynm"), CUSTOM + [("reorder_nominal_target", True),
          ("replace_missing_nominal_target", True)], "Nominal target with missing values")
except ERRORS as exc:
    manifest["errors"].append({"case": None, "stage": "setup", "message": message(exc)})
finally:
    write_manifest()
