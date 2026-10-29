# Probe: Apriori rejected settings (phase A, label apriori_more).
# Coordinator only: cargo xtask probe apriori_more_streams (Modeler 19, Jython 2.7).
# Writes generated inputs and one stream per case to corpus/apriori_more; public API only.
# A node the license does not provide fails that case into manifest["errors"]; later cases still run.
# Generators below are fixed loops of the row index. No random draws.
import json
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/apriori_more"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "apriori_more", "saved": [], "errors": [], "cases": [], "inputs": []}

ITEMS = ["bread", "milk", "eggs", "butter", "jam"]
BASKET_SCHEMA = [
    ("basket", "Integer", "Range", None),
    ("bread", "String", "Flag", ["F", "T"]),
    ("milk", "String", "Flag", ["F", "T"]),
    ("eggs", "String", "Flag", ["F", "T"]),
    ("butter", "String", "Flag", ["F", "T"]),
    ("jam", "String", "Flag", ["F", "T"]),
]
TRANS_SCHEMA = [
    ("basket", "Integer", "Range", None),
    ("item", "String", "Set", ["bread", "milk", "eggs", "butter", "jam"]),
]
NOMINAL_SCHEMA = [
    ("id", "Integer", "Range", None),
    ("color", "String", "Set", ["red", "green", "blue"]),
    ("shape", "String", "Set", ["circle", "square"]),
]
NOMINAL_INPUTS = ["color", "shape"]

BUILD_PROPERTIES = {
    "apriori": ["custom_fields", "consequents", "antecedents", "min_supp", "min_conf",
                "max_antecedents", "true_flags", "optimize", "use_transactional_data", "contiguous",
                "id_field", "content_field", "mode", "evaluation", "lower_bound"],
}
APPLY_PROPERTIES = {
    "apriori": ["max_predictions", "allow_repeats", "check_basket", "criterion"],
}
APRIORI_TABULAR = [
    ("custom_fields", True),
    ("use_transactional_data", False),
    ("consequents", ITEMS),
    ("antecedents", ITEMS),
    ("true_flags", True),
]
APRIORI_TRANS = [
    ("custom_fields", True),
    ("use_transactional_data", True),
    ("id_field", "basket"),
    ("contiguous", True),
    ("content_field", "item"),
]

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
        common_build = [("use_model_name", True), ("model_name", "apriori_more")]
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


def basket_rows(presents):
    rows = []
    for basket_id, present in enumerate(presents):
        row = [basket_id + 1]
        for item in ITEMS:
            row.append("T" if item in present else "F")
        rows.append(row)
    return rows


def trans_rows(presents, first_id):
    rows = []
    for offset, present in enumerate(presents):
        basket_id = first_id + offset
        for item in ITEMS:
            if item in present:
                rows.append([basket_id, item])
        for item in present:
            if item not in ITEMS:
                rows.append([basket_id, item])
    return rows


def declare_inputs():
    # Tabular base: 128 transactions, 8 repeats of the 16-basket pattern.
    tab_presents = [PATTERN_16[i % 16] for i in range(128)]
    write_csv("tab_train.csv", BASKET_SCHEMA, basket_rows(tab_presents),
              "128 baskets; 8 repeats of the models_next 16-basket pattern via i % 16")
    write_csv("tab_grid.csv", BASKET_SCHEMA, basket_rows(TAB_GRID_BASKETS),
              "6 score baskets: bread; eggs+bread; jam+bread; eggs+jam+bread; eggs+milk; empty")
    # Transactional base: same 128 baskets in id/item form (328 item rows).
    write_csv("trans_train.csv", TRANS_SCHEMA, trans_rows(tab_presents, 1),
              "same 128 baskets, one row per item, ids 1-128 contiguous")
    write_csv("trans_grid.csv", TRANS_SCHEMA, trans_rows(TRANS_GRID_BASKETS, 101),
              "same 6 score baskets in transactional form, ids 101-106; last has unknown cheese")
    # Split version: basket 50 ([bread, milk, eggs]) split into [bread] at 50 and
    # [milk, eggs] after 60. Same items, 129 contiguous blocks vs 128 grouped ids.
    split_presents = [list(p) for p in tab_presents]
    moving = []
    kept = []
    for item in split_presents[49]:
        if item == "bread":
            kept.append(item)
        else:
            moving.append(item)
    split_presents[49] = kept
    trans_split = trans_rows(split_presents, 1)
    # Relocate the moved rows of id 50 to after id 60, keeping id 50.
    head = [r for r in trans_split if r[0] <= 60]
    moved = [r for r in trans_split if r[0] == 50 and r[1] in moving]
    # head currently holds id-50 bread plus everything up to 60; remove moved part.
    head = [r for r in head if not (r[0] == 50 and r[1] in moving)]
    tail = [r for r in trans_split if r[0] > 60]
    reinsert = [[50, item] for item in moving]
    ordered = head + reinsert + tail
    # Renumber contiguous display ids for head/tail? No: keep original ids so the
    # split id 50 appears in two blocks; contiguous grouping counts 129 blocks.
    write_csv("trans_split_train.csv", TRANS_SCHEMA, ordered,
              "128 baskets with id 50 split into two blocks ([bread] at 50, [milk, eggs] after 60)")
    # Equal supports: 60x [bread, milk, eggs] + 60x [butter, jam]; every item 60/120.
    equal_presents = [["bread", "milk", "eggs"] for _i in range(60)]
    equal_presents = equal_presents + [["butter", "jam"] for _i in range(60)]
    write_csv("tie_equal_train.csv", BASKET_SCHEMA, basket_rows(equal_presents),
              "120 baskets; every item support 60/120; confidences 1.0; pins tie order")
    # Two new items: bread (i%4<2), milk (i%4 in 0,2), eggs when bread+milk and i%5!=0
    # (40 rows) plus noise when i%20 in (3,7) (20 rows). No 1-antecedent rule passes
    # 80%; bread+milk=>eggs is 40/50 = 0.8 and its antecedent is two new items.
    twonew_presents = []
    for i in range(200):
        present = []
        bread = (i % 4) < 2
        milk = (i % 4) == 0 or (i % 4) == 2
        if bread:
            present.append("bread")
        if milk:
            present.append("milk")
        eggs = (bread and milk and (i % 5) != 0) or (i % 20) == 3 or (i % 20) == 7
        if eggs:
            present.append("eggs")
        twonew_presents.append(present)
    write_csv("tie_twonew_train.csv", BASKET_SCHEMA, basket_rows(twonew_presents),
              "200 baskets; P=bread Q=milk R=eggs interaction; 1-antecedent max 0.66, P+Q=>R 0.8")
    # Nominal: color by i%3, shape circle iff red else square.
    nominal_rows = []
    colors = ["red", "green", "blue"]
    for i in range(120):
        color = colors[i % 3]
        if color == "red":
            shape = "circle"
        else:
            shape = "square"
        nominal_rows.append([i + 1, color, shape])
    write_csv("nominal_train.csv", NOMINAL_SCHEMA, nominal_rows,
              "120 rows; color=i%3, shape=circle iff red else square; red=>circle 1.0")
    nominal_grid = [[1001, "red", "circle"], [1002, "green", "square"], [1003, "blue", "circle"]]
    write_csv("nominal_grid.csv", NOMINAL_SCHEMA, nominal_grid,
              "3 score rows: seen red/circle, seen green/square, unseen blue/circle")


def declare_cases():
    def apriori(name, props, tests, **kwargs):
        data = kwargs.pop("data", "tab_train.csv")
        schema = kwargs.pop("schema", BASKET_SCHEMA)
        inputs = kwargs.pop("inputs", ITEMS)
        grid = kwargs.pop("score_data", "tab_grid.csv")
        add_case(name, "apriori", data, schema, [], inputs, props, tests,
                 score_data=grid, **kwargs)

    # Controls.
    apriori("tab_base", APRIORI_TABULAR,
            "Tabular flags; true values only; support and confidence at the node default")
    apriori("trans_base", APRIORI_TRANS,
            "Same 128 baskets in id/item form", data="trans_train.csv", schema=TRANS_SCHEMA,
            inputs=["item"], score_data="trans_grid.csv", compare_to="tab_base")
    # Target 1: evaluation measures.
    apriori("eval_diffprior",
            APRIORI_TABULAR + [("mode", "Expert"), ("evaluation", "DifferenceToPrior"),
                               ("lower_bound", 0.3)],
            "Confidence difference 0.3", compare_to="tab_base")
    apriori("eval_confratio",
            APRIORI_TABULAR + [("mode", "Expert"), ("evaluation", "ConfidenceRatio"),
                               ("lower_bound", 1.5)],
            "Confidence ratio 1.5", compare_to="tab_base")
    apriori("eval_infdiff",
            APRIORI_TABULAR + [("mode", "Expert"), ("evaluation", "InformationDifference"),
                               ("lower_bound", 1.0)],
            "Information difference 1.0", compare_to="tab_base")
    apriori("eval_chisq",
            APRIORI_TABULAR + [("mode", "Expert"), ("evaluation", "NormalizedChiSquare"),
                               ("lower_bound", 0.3)],
            "Normalized chi-square 0.3", compare_to="tab_base")
    # Target 3: false flags and nominal.
    apriori("trueflags_false",
            [p for p in APRIORI_TABULAR if p[0] != "true_flags"] + [("true_flags", False)],
            "False flags become their own items", compare_to="tab_base")
    apriori("nominal_set",
            [("custom_fields", True), ("use_transactional_data", False),
             ("consequents", NOMINAL_INPUTS), ("antecedents", NOMINAL_INPUTS),
             ("true_flags", True)],
            "Nominal Set fields color/shape", data="nominal_train.csv", schema=NOMINAL_SCHEMA,
            inputs=NOMINAL_INPUTS, score_data="nominal_grid.csv", compare_to="tab_base")
    # Target 4: non-contiguous ids (two cases isolate data split vs grouping).
    apriori("trans_split", APRIORI_TRANS,
            "Split id 50 in two blocks, still contiguous=True (129 blocks)",
            data="trans_split_train.csv", schema=TRANS_SCHEMA, inputs=["item"],
            score_data="trans_grid.csv", compare_to="trans_base")
    apriori("trans_noncontig",
            [p for p in APRIORI_TRANS if p[0] != "contiguous"] + [("contiguous", False)],
            "Same split file grouped by id (128 transactions)",
            data="trans_split_train.csv", schema=TRANS_SCHEMA, inputs=["item"],
            score_data="trans_grid.csv", compare_to="trans_split")
    # Target 5: differing antecedent/consequent sets.
    apriori("antecons_diff_tab",
            [("custom_fields", True), ("use_transactional_data", False),
             ("consequents", ITEMS),
             ("antecedents", ["bread", "milk", "eggs", "butter"]),
             ("true_flags", True)],
            "Tabular antecedents omit jam", compare_to="tab_base")
    apriori("antecons_diff_trans",
            [("custom_fields", True), ("use_transactional_data", True),
             ("id_field", "basket"), ("contiguous", True), ("content_field", "item"),
             ("antecedents", ["eggs", "jam", "bread"]),
             ("consequents", ["milk", "butter"])],
            "Transactional antecedents eggs/jam/bread, consequents milk/butter",
            data="trans_train.csv", schema=TRANS_SCHEMA, inputs=["item"],
            score_data="trans_grid.csv", compare_to="trans_base")
    # Target 6: tie order and two new items.
    apriori("tie_equal", APRIORI_TABULAR,
            "Equal item supports 60/120 pin the tie order",
            data="tie_equal_train.csv", compare_to="tab_base")
    apriori("tie_twonew", APRIORI_TABULAR,
            "No 1-antecedent passes; bread+milk=>eggs has two new antecedent items",
            data="tie_twonew_train.csv", compare_to="tab_base")
    # Target 7: scoring.
    apriori("score_repeats", APRIORI_TABULAR,
            "Scoring allow_repeats=True keeps jam=>butter and jam+bread=>butter",
            compare_to="tab_base", applier_props=[("allow_repeats", True)])
    apriori("score_nocheck", APRIORI_TABULAR,
            "Scoring check_basket=NoCheck fires eggs=>milk on eggs+milk",
            compare_to="tab_base", applier_props=[("check_basket", "NoCheck")])
    apriori("score_support", APRIORI_TABULAR,
            "Scoring criterion=Support ranks butter before milk on eggs+jam+bread",
            compare_to="tab_base", applier_props=[("criterion", "Support")])
    apriori("score_maxpred1", APRIORI_TABULAR,
            "Scoring max_predictions=1 truncates eggs+jam+bread to the top rule",
            compare_to="tab_base", applier_props=[("max_predictions", 1)])
    apriori("score_unmatched", APRIORI_TRANS,
            "Transactional scoring with ignore_unmatched=False on cheese basket",
            data="trans_train.csv", schema=TRANS_SCHEMA, inputs=["item"],
            score_data="trans_grid.csv", compare_to="trans_base",
            # Table 156 spells it ignore_unmatached, which Modeler rejects as undefined.
            applier_props=[("ignore_unmatched", False)])


try:
    ensure_clean_dir(DIR)
    declare_inputs()
    declare_cases()
    write_manifest()
    for case in manifest["cases"]:
        build_case(case)
except ERRORS as exc:
    manifest["errors"].append({"case": None, "stage": "setup", "message": message(exc)})
finally:
    write_manifest()
