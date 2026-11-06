# Probe: Apriori item rank among equal supports, round 2 (label apriori_tie2).
# apriori_tie showed the tie break follows co-occurrence blocks; this varies block sizes,
# block count and the columns a block occupies.
# Coordinator only: cargo xtask probe apriori_tie2_streams (Modeler 19, Jython 2.7).
# Every case has k flag items that are always present together (or all absent), so every subset
# is frequent with confidence 1 and the rule order exposes the full rank of the k equal items.
# Cases vary k, the field names and the column order, to tell a name-based order (hash), a
# position-based order (sort) and a data-order rule apart (apriori_more tie_equal / tie_twonew).
# Writes generated inputs and one stream per case to corpus/apriori_tie; public API only.
# Python 2 syntax only. Generators are fixed loops; no random draws.
import json
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/apriori_tie2"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "apriori_tie2", "saved": [], "errors": [], "cases": [], "inputs": []}

BUILD_PROPERTIES = {
    "apriori": ["custom_fields", "consequents", "antecedents", "min_supp", "min_conf",
                "max_antecedents", "true_flags", "optimize", "use_transactional_data", "contiguous",
                "id_field", "content_field", "mode", "evaluation", "lower_bound"],
}
APPLY_PROPERTIES = {
    "apriori": ["max_predictions", "allow_repeats", "check_basket", "criterion"],
}

PATTERN_16 = [
    ["bread", "milk"],
    ["bread", "milk", "eggs"],
    ["bread", "milk"],
    ["bread", "milk", "eggs"],
    ["bread", "milk", "butter"],
    ["bread", "butter"],
    ["bread", "butter", "jam"],
    ["bread", "butter", "jam"],
    ["butter", "jam"],
    ["butter", "jam"],
    ["bread", "butter", "jam"],
    ["milk", "eggs"],
    ["bread", "milk", "eggs"],
    ["bread", "jam"],
    ["bread", "milk", "butter", "jam"],
    ["eggs", "milk"],
]

TAB_GRID_BASKETS = [
    ["bread"],
    ["eggs", "bread"],
    ["jam", "bread"],
    ["eggs", "jam", "bread"],
    ["eggs", "milk"],
    [],
]

TRANS_GRID_BASKETS = [
    ["bread"],
    ["eggs", "bread"],
    ["jam", "bread"],
    ["eggs", "jam", "bread"],
    ["eggs", "milk"],
    ["eggs", "bread", "cheese"],
]


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
    # Deliberately non-recursive, restricted to this probe's dedicated label directory.
    if path != DIR:
        raise ValueError("unexpected corpus directory: " + path)
    directory = java.io.File(path)
    if directory.exists():
        children = directory.listFiles()
        if children is None:
            raise IOError("cannot list " + path)
        for child in children:
            if child.isFile() and unicode(child.getName()).endswith((".csv", ".str")):
                if not child.delete():
                    raise IOError("cannot remove old probe artifact " + unicode(child))
    elif not directory.mkdirs():
        raise IOError("cannot create " + path)


def csv_cell(value):
    if value is None:
        return ""
    # Quote every string, including the intentional empty string and whitespace.
    if isinstance(value, basestring):
        return '"' + value.replace('"', '""') + '"'
    if isinstance(value, float):
        return "%.17g" % value
    return str(value)


def write_csv(name, schema, rows, tests):
    lines = [",".join([field[0] for field in schema])]
    for row in rows:
        if len(row) != len(schema):
            raise ValueError("wrong CSV width in " + name)
        lines.append(",".join([csv_cell(value) for value in row]))
    handle = open(DIR + "/" + name, "wb")
    try:
        handle.write((NL.join(lines) + NL).encode("utf-8"))
    finally:
        handle.close()
    manifest["saved"].append(name)
    manifest["inputs"].append({"file": name, "rows": len(rows), "tests": tests})


def read_properties(node, names):
    result = {}
    for name in names:
        try:
            result[name] = snapshot(node.getPropertyValue(name))
        except ERRORS as exc:
            result[name] = {"read_error": message(exc)}
    return result


def set_properties(node, props, entry):
    for name, value in props:
        entry["setting_in_progress"] = [name, snapshot(value)]
        node.setPropertyValue(name, value)
        # A rejected setting fails the case; a coerced setting is visible in the manifest.
        entry.setdefault("readback", []).append(
            [name, snapshot(node.getPropertyValue(name))])
    entry.pop("setting_in_progress", None)


def instantiate_node(stream, node):
    table = stream.createAt("table", "Instantiate", 1000, 800)
    stream.link(node, table)
    try:
        table.run([])
    finally:
        stream.delete(table)


def typed_source(stream, name, schema, targets, inputs, roles, missing, y_position, entry):
    source = stream.createAt("variablefile", name, 60, y_position)
    source_props = [("full_filename", DIR + "/" + name), ("read_field_names", True),
                    ("delimit_comma", True), ("delimit_space", False), ("delimit_tab", False),
                    ("quotes_2", "PairAndDiscard"), ("strip_spaces", "None"),
                    ("decimal_symbol", "Period"), ("encoding", "UTF-8"),
                    ("auto_recognize_datetime", False), ("lines_to_scan", 1000)]
    source_entry = {"data": name, "properties": source_props, "storage": []}
    entry.setdefault("sources", []).append(source_entry)
    set_properties(source, source_props, source_entry)
    instantiate_node(stream, source)
    for field, storage, measure, values in schema:
        source_entry["setting_in_progress"] = ["custom_storage_type", field, storage]
        source.setKeyedPropertyValue("custom_storage_type", field, storage)
        source_entry["storage"].append([field, storage])
    source_entry.pop("setting_in_progress", None)

    node = stream.createAt("type", "Type " + name, 260, y_position)
    stream.link(source, node)
    instantiate_node(stream, node)
    type_entry = {"data": name, "fields": [], "missing": missing}
    entry.setdefault("types", []).append(type_entry)
    for field, storage, measure, values in schema:
        node.setKeyedPropertyValue("type", field, measure)
        if values is not None:
            node.setKeyedPropertyValue("values", field, values)
            node.setKeyedPropertyValue("extend_values", field, False)
        direction = roles.get(field, "None")
        if field in targets:
            direction = "Target"
        elif field in inputs:
            direction = "Input"
        node.setKeyedPropertyValue("direction", field, direction)
        node.setKeyedPropertyValue("check", field, "None")
        if storage == "Real":
            node.setKeyedPropertyValue("export_places", field, 15)
        for prop, key, value in missing:
            if key == field:
                node.setKeyedPropertyValue(prop, field, value)
    instantiate_node(stream, node)
    for field, storage, measure, values in schema:
        actual = {"name": field, "storage_requested": storage}
        for prop in ["storage", "type", "direction", "values", "enable_missing", "missing_values",
                     "null_missing", "whitespace_missing"]:
            try:
                actual[prop] = snapshot(node.getKeyedPropertyValue(prop, field))
            except ERRORS as exc:
                actual[prop] = {"read_error": message(exc)}
        type_entry["fields"].append(actual)
    return node


def find_applier(stream):
    found = []
    iterator = stream.iterator()
    while iterator.hasNext():
        node = iterator.next()
        if unicode(node.getTypeName()).startswith("apply"):
            found.append(node)
    if len(found) > 1:
        raise RuntimeError("unexpected multiple automatically placed nuggets")
    if found:
        return found[0]
    return None


def add_case(name, family, data, schema, targets, inputs, props, tests,
             score_data=None, missing=None, roles=None, stream_props=None,
             score_variants=None, compare_to=None, applier_props=None):
    entry = {"case": name, "file": name + ".str", "family": family, "data": data,
             "schema": schema, "targets": targets, "inputs": inputs,
             "properties": props, "tests": tests,
             "score_data": score_data, "missing": missing or [], "roles": roles or {},
             "stream_properties": stream_props or [], "score_variants": score_variants or [],
             "compare_to": compare_to, "applier_props": applier_props or [], "status": "planned"}
    manifest["cases"].append(entry)


def build_case(entry):
    stream = None
    stage = "create_stream"
    family = entry["family"]
    try:
        stream = taskrunner.createStream(entry["case"], False, False)
        common_stream = [("decimal_places", 15), ("decimal_symbol", "Period"),
                         ("enable_parallelism", False),
                         ("create_model_applier_for_new_models", True),
                         ("create_model_applier_update_links", "createEnabled")]
        entry["common_stream_properties"] = common_stream
        set_properties(stream, common_stream + entry["stream_properties"], entry)
        stage = "training_type"
        schema = entry["schema"]
        training = typed_source(stream, entry["data"], schema, entry["targets"],
                                entry["inputs"], entry["roles"], entry["missing"], 100, entry)
        stage = "build_properties"
        builder = stream.createAt(family, entry["case"], 480, 100)
        stream.link(training, builder)
        entry["build_type"] = unicode(builder.getTypeName())
        entry["builder_defaults"] = read_properties(builder, BUILD_PROPERTIES[family])
        # Identical model names across repeated cases: case labels must not perturb training.
        common_build = [("use_model_name", True), ("model_name", "apriori_tie2")]
        entry["common_build_properties"] = common_build
        set_properties(builder, common_build + entry["properties"], entry)
        entry["builder_effective"] = read_properties(builder, BUILD_PROPERTIES[family])
        stage = "train"
        results = []
        builder.run(results)
        applier = find_applier(stream)
        entry["auto_placed_applier"] = applier is not None
        models = [result for result in results if isinstance(result, modeler.api.ModelOutput)]
        if not models:
            raise RuntimeError("builder returned no ModelOutput for saved scoring nuggets")
        if applier is None:
            applier = stream.createModelApplierAt(models[0], "Train scores", 480, 260)
            stream.link(training, applier)
        else:
            applier.setXYPosition(480, 260)
        entry["applier_type"] = unicode(applier.getTypeName())
        entry["applier_defaults"] = read_properties(applier, APPLY_PROPERTIES[family])
        terminals = []
        entry["terminals"] = []

        def attach(nugget, label, position, properties):
            settings = {"label": label, "properties": properties}
            set_properties(nugget, properties, settings)
            settings["effective"] = read_properties(nugget, APPLY_PROPERTIES[family])
            table = stream.createAt("table", label, 760, position)
            stream.link(nugget, table)
            settings["applier_id"] = unicode(nugget.getID())
            settings["table_id"] = unicode(table.getID())
            entry["terminals"].append(settings)
            terminals.append(table)

        attach(applier, "Train scores", 260, entry["applier_props"])
        scoring = training
        stage = "scoring_type"
        if entry["score_data"] is not None:
            scoring = typed_source(stream, entry["score_data"], schema, entry["targets"],
                                   entry["inputs"], entry["roles"], entry["missing"], 420, entry)
        variants = [("Grid scores", [])] + entry["score_variants"]
        stage = "scoring_properties"
        for index, (label, properties) in enumerate(variants):
            position = 420 + index * 100
            nugget = stream.createModelApplierAt(models[0], label, 480, position)
            stream.link(scoring, nugget)
            attach(nugget, label, position, properties)

        # Preserve a usable stream even when an edge-value Table execution fails.
        stage = "save_stream"
        taskrunner.saveStreamToFile(stream, DIR + "/" + entry["file"])
        manifest["saved"].append(entry["file"])
        entry["status"] = "saved"
        stage = "score"
        for table in terminals:
            try:
                table.run([])
            except ERRORS as exc:
                manifest["errors"].append({"case": entry["case"], "stage": "score",
                                           "table": unicode(table.getLabel()),
                                           "message": message(exc)})
                entry["status"] = "saved_with_score_errors"
    except ERRORS as exc:
        entry["status"] = "failed"
        manifest["errors"].append({"case": entry["case"], "stage": stage,
                                   "message": message(exc)})
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS as exc:
                manifest["errors"].append({"case": entry["case"], "stage": "clear",
                                           "message": message(exc)})
        write_manifest()


def schema_for(fields):
    schema = [("basket", "Integer", "Range", None)]
    for name in fields:
        schema.append((name, "String", "Flag", ["F", "T"]))
    return schema


def block_rows(fields, blocks, n_on, n_off):
    """blocks: list of field-name groups; each group is present together in n_on baskets
    (groups in sequence), then n_off baskets have nothing."""
    rows = []
    for group in blocks:
        for i in range(n_on):
            row = [len(rows) + 1]
            for name in fields:
                row.append("T" if name in group else "F")
            rows.append(row)
    for i in range(n_off):
        row = [len(rows) + 1]
        for name in fields:
            row.append("F")
        rows.append(row)
    return rows


CASES = [
    ("b22", ['a', 'b', 'c', 'd'], [["a", "b"], ["c", "d"]], "blocks 2+2"),
    ("b23", ['a', 'b', 'c', 'd', 'e'], [["a", "b"], ["c", "d", "e"]], "blocks 2+3"),
    ("b32", ['a', 'b', 'c', 'd', 'e'], [["a", "b", "c"], ["d", "e"]], "blocks 3+2 (k5_two control)"),
    ("b33", ['a', 'b', 'c', 'd', 'e', 'f'], [["a", "b", "c"], ["d", "e", "f"]], "blocks 3+3"),
    ("b24", ['a', 'b', 'c', 'd', 'e', 'f'], [["a", "b"], ["c", "d", "e", "f"]], "blocks 2+4"),
    ("b42", ['a', 'b', 'c', 'd', 'e', 'f'], [["a", "b", "c", "d"], ["e", "f"]], "blocks 4+2"),
    ("b43", ['a', 'b', 'c', 'd', 'e', 'f', 'g'], [["a", "b", "c", "d"], ["e", "f", "g"]], "blocks 4+3"),
    ("b222", ['a', 'b', 'c', 'd', 'e', 'f'], [["a", "b"], ["c", "d"], ["e", "f"]], "blocks 2+2+2"),
    ("b322", ['a', 'b', 'c', 'd', 'e', 'f', 'g'], [["a", "b", "c"], ["d", "e"], ["f", "g"]], "blocks 3+2+2"),
    ("b32_cols034", ['a', 'b', 'c', 'd', 'e'], [["a", "d", "e"], ["b", "c"]], "3-block in columns 0, 3, 4"),
    ("b32_cols024", ['a', 'b', 'c', 'd', 'e'], [["a", "c", "e"], ["b", "d"]], "3-block in columns 0, 2, 4"),
]


def declare():
    for name, fields, blocks, tests in CASES:
        data = name + "_train.csv"
        schema = schema_for(fields)
        write_csv(data, schema, block_rows(fields, blocks, 60 if len(blocks) == 2 else 60, 60),
                  tests)
        props = [("custom_fields", True), ("use_transactional_data", False),
                 ("consequents", fields), ("antecedents", fields), ("true_flags", True)]
        add_case(name, "apriori", data, schema, [], fields, props, tests)


try:
    ensure_clean_dir(DIR)
    declare()
    write_manifest()
    for case in manifest["cases"]:
        build_case(case)
except ERRORS as exc:
    manifest["errors"].append({"case": None, "stage": "setup", "message": message(exc)})
finally:
    write_manifest()
