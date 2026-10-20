# Probe: Auto Data Prep steps the misc_next adp_* cases leave untouched (label adp_next).
# Coordinator only: cargo xtask probe adp_next_streams, then corpus record adp_next and oracle.
# Jython 2.7, public modeler.api only (Scripting Guide Table 79, autodataprepnode). Deterministic
# data: each case switches on one ADP step on data built to trigger it. Streams are saved before
# they run, like misc_next/adp_*: the node analyses its input when the Table executes.
import json
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/adp_next"
CSV_NAME = "adp_next.csv"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)
taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "adp_next", "saved": [], "errors": [], "cases": [], "inputs": []}

# (name, storage, measure, values)
SCHEMA = [("id", "Integer", "Range", None),
          ("ycont", "Real", "Range", None),
          ("ycat", "String", "Flag", ["N", "Y"]),
          ("c_out", "Real", "Range", None),
          ("c_skew", "Real", "Range", None),
          ("c_noise", "Real", "Range", None),
          ("n_merge", "String", "Set", ["k1", "k2", "k3", "k4", "rare"]),
          ("n_miss", "String", "Set", ["p", "q", "r"])]
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
    # 300 records. ycont depends on c_out, c_skew and n_merge (k1 and k2 share an effect; rare
    # has 3 records); c_noise is unrelated; n_miss has 12 blanks. ycat is ycont above its middle.
    effect = {"k1": 0.0, "k2": 0.0, "k3": 3.0, "k4": -3.0, "rare": 1.0}
    out = []
    for i in range(300):
        c_out = ((i * 37) % 101) / 10.0 - 5.0
        if i == 17:
            c_out = 60.0
        if i == 211:
            c_out = -45.0
        c_skew = (((i * 53) % 97) / 97.0) ** 3 * 50.0
        c_noise = ((i * 71) % 89) / 8.9
        if i in (5, 105, 205):
            cat = "rare"
        else:
            cat = ["k1", "k2", "k3", "k4"][(i * 3 + i // 7) % 4]
        miss = None if i % 25 == 3 else ["p", "q", "r", "p"][(i * 5) % 4]
        wobble = ((i * 29) % 13 - 6) / 6.0
        ycont = 10.0 + 0.8 * (c_out if abs(c_out) < 10 else 0.0) + 0.1 * c_skew + effect[cat] + wobble
        out.append([i + 1, ycont, None, c_out, c_skew, c_noise, cat, miss])
    middle = sorted([r[1] for r in out])[150]
    for r in out:
        r[2] = "Y" if r[1] > middle else "N"
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


CONT = {"id": "None", "ycont": "Target", "ycat": "None"}
FLAG = {"id": "None", "ycont": "None", "ycat": "Target"}
NONE = {"id": "None", "ycont": "None", "ycat": "None"}
CUSTOM = [("objective", "Custom")]

try:
    ensure_clean_dir(DIR)
    write_data()
    build("adp_custom_base", CONT, CUSTOM, "Custom objective, nothing else changed: the baseline")
    build("adp_outlier_replace", CONT, CUSTOM + [("replace_outliers_inputs", True),
          ("outlier_cutoff_value", 3.0), ("outlier_method", "Replace")],
          "Two extreme c_out values replaced at the cutoff")
    build("adp_outlier_delete", CONT, CUSTOM + [("replace_outliers_inputs", True),
          ("outlier_cutoff_value", 3.0), ("outlier_method", "Delete")],
          "Same outliers, Delete")
    build("adp_minmax", CONT, CUSTOM + [("rescale_continuous_inputs", True),
          ("rescaling_method", "MinMax"), ("min_max_minimum", 0), ("min_max_maximum", 100)],
          "Min-max rescaling to 0..100")
    build("adp_nominal_missing", CONT, CUSTOM + [("replace_missing_nominal_inputs", True)],
          "n_miss blanks replaced by the mode")
    build("adp_merge_supervised", CONT, CUSTOM + [("transform_select_input_fields", True),
          ("maximize_association_with_target", True), ("merge_nominal_features", True),
          ("p_value_for_merging", 0.05)],
          "k1/k2 share a target effect and should merge")
    build("adp_merge_flag", FLAG, CUSTOM + [("transform_select_input_fields", True),
          ("maximize_association_with_target", True), ("merge_nominal_features", True),
          ("p_value_for_merging", 0.05)],
          "Supervised merge with a flag target")
    build("adp_merge_unsupervised", NONE, CUSTOM + [("transform_select_input_fields", True),
          ("merge_nominal_features", True), ("minimum_cases_in_category", 10)],
          "No target: the 3-record category merges")
    build("adp_binning", FLAG, CUSTOM + [("transform_select_input_fields", True),
          ("bin_continuous_fields", True), ("p_value_for_binning", 0.05)],
          "Supervised binning of continuous inputs against a flag target")
    build("adp_selection", CONT, CUSTOM + [("transform_select_input_fields", True),
          ("perform_feature_selection", True), ("p_value_for_selection", 0.05)],
          "c_noise unrelated to the target should be dropped")
    build("adp_target_rescale", CONT, CUSTOM + [("rescale_continuous_target", True),
          ("target_final_mean", 0), ("target_final_sd", 1)],
          "Continuous target z-scored")
    build("adp_target_boxcox", {"id": "None", "ycont": "None", "ycat": "None", "c_skew": "Target"},
          CUSTOM + [("rescale_continuous_target", True), ("target_final_mean", 5),
                    ("target_final_sd", 2)],
          "Skewed target c_skew: Box-Cox lambda other than 1, final mean 5 and SD 2")
    build("adp_construction", CONT, CUSTOM + [("transform_select_input_fields", True),
          ("perform_feature_construction", True)],
          "Feature construction (PCA)")
except ERRORS as exc:
    manifest["errors"].append({"case": None, "stage": "setup", "message": message(exc)})
finally:
    write_manifest()
