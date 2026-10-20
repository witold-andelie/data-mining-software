# Probe: Auto Data Prep feature construction (PCA) on correlated inputs (label adp_corr).
# Coordinator only: cargo xtask probe adp_corr_streams, then corpus record adp_corr and oracle.
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
DIR = "D:/data-mining-software/corpus/adp_corr"
CSV_NAME = "adp_corr.csv"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)
taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "adp_corr", "saved": [], "errors": [], "cases": [], "inputs": []}

# (name, storage, measure, values)
SCHEMA = [("id", "Integer", "Range", None), ("y", "Real", "Range", None)] + [
    ("x%d" % k, "Real", "Range", None) for k in range(1, 9)]
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
    # 300 records. x1, x2 and x8 are one signal with small noises (pairwise r > .9, a group of
    # three); x4 and x7 are a signal and its negative (r about -.95: loadings of opposite sign);
    # x3 mixes x1 and x4 with its own noise (no exact linear dependence); x5 and x6 correlate at
    # about .15, between the last two correlation levels; y depends on x1, x4 and x5.
    out = []
    for i in range(300):
        u = ((i * 37) % 101) / 10.0 - 5.0
        v = ((i * 53) % 97) / 9.7 - 5.0
        w = ((i * 71) % 89) / 8.9 - 5.0
        z = ((i * 43) % 83) / 8.3 - 5.0
        e = ((i * 29) % 13 - 6) / 30.0
        f = ((i * 31) % 17 - 8) / 8.0
        g = ((i * 19) % 23 - 11) / 6.0
        x1 = u
        x2 = u + e
        x8 = u + f
        x3 = 0.6 * u + 0.6 * v + g
        x4 = v
        x7 = -v + f
        x5 = w
        x6 = 0.15 * w + z
        y = 2.0 * u + v + 0.5 * w + ((i * 17) % 11 - 5) / 5.0
        out.append([i + 1, y, x1, x2, x3, x4, x5, x6, x7, x8])
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



TARGET = {"id": "None", "y": "Target"}
NONE = {"id": "None", "y": "None"}
CUSTOM = [("objective", "Custom")]
CONSTRUCT = CUSTOM + [("transform_select_input_fields", True), ("perform_feature_construction", True)]

try:
    ensure_clean_dir(DIR)
    write_data()
    build("corr_base", TARGET, CUSTOM, "Custom objective only: z-scores of x1..x5")
    build("corr_construct", TARGET, CONSTRUCT, "Construction with a continuous target")
    build("corr_construct_select", TARGET, CONSTRUCT + [("perform_feature_selection", True),
          ("p_value_for_selection", 0.05)], "Construction and selection (partial correlations)")
    build("corr_construct_notarget", NONE, CONSTRUCT, "Construction without a target")
    build("corr_construct_norescale", TARGET, CONSTRUCT + [("rescale_continuous_inputs", False)],
          "Construction on unrescaled inputs")
except ERRORS as exc:
    manifest["errors"].append({"case": None, "stage": "setup", "message": message(exc)})
finally:
    write_manifest()
