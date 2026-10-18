# Training evidence for Sequence ordering and Decision List search (Modeler 19).
# Coordinator only: cargo xtask probe train_next_streams, then oracle train_next <case>.str.
# Jython 2.7; public modeler.api only. Fixed loops, no random draws or external packages.
# Runtime artifacts go to the dedicated corpus label; this file does not run on import in CPython.
# Source: IBM SPSS Modeler 19.0 Python Scripting and Automation Guide, Tables 110,
# 124, 145, 165; line numbers below refer to ModelerScriptingAutomation-19.0.txt.
# Source/Type, nugget placement, saving and scoring follow models_next2_streams.py.
import json
from itertools import combinations
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/train_next"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
NL = chr(10)
MODEL_NAME = "train_next"
GUIDE_FILE = "ModelerScriptingAutomation-19.0.txt"
GUIDE_LINES = {
    "id_field": 15648, "time_field": 15656, "use_time_field": 15657,
    "content_fields": 15658, "contiguous": 15659, "min_supp": 15660,
    "min_conf": 15661, "max_size": 15662,
    "max_rules": 12814, "min_group_size": 12815, "min_group_size_pct": 12816,
    "confidence_level": 12817, "max_segments_per_rule": 12832, "mode": 12833,
    "bin_method": 12843, "bin_count": 12845,
    "max_models_per_cycle": 12846, "max_rules_per_cycle": 12847,
}

manifest = {"label": "train_next", "saved": [], "errors": [], "cases": [], "inputs": [],
            "guide": GUIDE_FILE, "design": {},
            "common_build_guide_lines": {"use_model_name": 11452, "model_name": 11453},
            "apply_guide_lines": {"sequence": [17935, 17939],
                                  "decisionlist": [17373, 17394]}}
taskrunner = None

# Deliberately separate domain order, discovery order and decreasing customer support.
ITEMS = ["amber", "bravo", "copper", "indigo", "mint", "teal", "violet", "zeta"]
A, B, C, D, E, F, G, H = "zeta", "amber", "mint", "bravo", "violet", "copper", "teal", "indigo"
SEQ_SCHEMA = [("customer", "Integer", "Range", None),
              ("time", "Integer", "Range", None),
              ("item", "String", "Set", ITEMS)]
SEQUENCE_FIELDS = [("id_field", "customer"), ("contiguous", True),
                   ("use_time_field", True), ("time_field", "time"),
                   ("content_fields", ["item"])]
SEGMENT_SCHEMA = [
    ("id", "Integer", "Range", None),
    ("region", "String", "Set", ["east", "west"]),
    ("channel", "String", "Set", ["store", "web"]),
    ("tier", "String", "Set", ["bronze", "gold"]),
    ("spend", "Real", "Range", None),
    ("visits", "Real", "Range", None),
    ("respond", "String", "Flag", ["N", "Y"]),
]
SEGMENT_INPUTS = ["region", "channel", "tier", "spend", "visits"]
# region=east, channel=web, tier=gold, row count, positive count.
SEGMENT_CELLS = [(1, 1, 1, 80, 72), (1, 1, 0, 80, 48),
                 (1, 0, 1, 80, 36), (1, 0, 0, 80, 36),
                 (0, 1, 1, 80, 40), (0, 1, 0, 160, 20),
                 (0, 0, 1, 160, 16), (0, 0, 0, 480, 32)]
BUILD_PROPERTIES = {
    "sequence": ["id_field", "contiguous", "use_time_field", "time_field", "content_fields",
                 "min_supp", "min_conf", "max_size", "max_predictions", "mode",
                 "use_max_duration", "max_duration", "use_gaps", "min_item_gap",
                 "max_item_gap", "use_pruning", "pruning_value", "set_mem_sequences",
                 "mem_sequences"],
    "decisionlist": ["search_direction", "target_value", "max_rules", "min_group_size",
                     "min_group_size_pct", "confidence_level", "max_segments_per_rule", "mode",
                     "bin_method", "bin_count", "max_models_per_cycle", "max_rules_per_cycle",
                     "segment_growth", "include_missing", "final_results_only", "reuse_fields",
                     "max_alternatives"],
}
APPLY_PROPERTIES = {"sequence": [],
                    "decisionlist": ["calculate_raw_propensities", "calculate_adjusted_propensities"]}


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
            manifest["errors"].append({"stage": "read_property", "property": name,
                                       "node": unicode(node.getID()), "message": message(exc)})
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
                manifest["errors"].append({"case": entry["case"], "stage": "read_type",
                                           "field": field, "property": prop,
                                           "message": message(exc)})
        type_entry["fields"].append(actual)
    return node


def list_appliers(stream):
    found = []
    iterator = stream.iterator()
    while iterator.hasNext():
        node = iterator.next()
        if unicode(node.getTypeName()).startswith("apply"):
            found.append(node)
    return found


def find_applier(stream):
    found = list_appliers(stream)
    if len(found) > 1:
        raise RuntimeError("unexpected multiple automatically placed nuggets")
    if found:
        return found[0]
    return None


def applier_ids(stream):
    found = []
    for node in list_appliers(stream):
        found.append(unicode(node.getID()))
    return found


def add_case(name, family, data, schema, targets, inputs, props, tests,
             score_data=None, missing=None, roles=None, stream_props=None,
             score_variants=None, compare_to=None):
    entry = {"case": name, "file": name + ".str", "family": family, "data": data,
             "schema": schema, "targets": targets, "inputs": inputs,
             "properties": props, "tests": tests,
             "score_data": score_data, "missing": missing or [], "roles": roles or {},
             "stream_properties": stream_props or [], "score_variants": score_variants or [],
             "compare_to": compare_to, "status": "planned"}
    entry["property_guide_lines"] = dict((key, GUIDE_LINES[key]) for key, value in props)
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
        common_build = [("use_model_name", True), ("model_name", MODEL_NAME)]
        entry["common_build_properties"] = common_build
        set_properties(builder, common_build + entry["properties"], entry)
        entry["builder_effective"] = read_properties(builder, BUILD_PROPERTIES[family])
        stage = "train"
        results = []
        builder.run(results)
        appliers = list_appliers(stream)
        entry["placed_nugget_types"] = [unicode(node.getTypeName()) for node in appliers]
        applier = find_applier(stream)
        entry["auto_placed_applier"] = applier is not None
        models = [result for result in results if isinstance(result, modeler.api.ModelOutput)]
        if not models:
            raise RuntimeError("builder returned no ModelOutput for saved scoring nuggets")
        entry["model_output_count"] = len(models)
        if applier is None:
            applier = stream.createModelApplierAt(models[0], "Train scores", 480, 260)
            stream.link(training, applier)
            entry["placed_nugget_types"] = [unicode(applier.getTypeName())]
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
            settings["applier_type"] = unicode(nugget.getTypeName())
            table = stream.createAt("table", label, 760, position)
            stream.link(nugget, table)
            settings["applier_id"] = unicode(nugget.getID())
            settings["table_id"] = unicode(table.getID())
            entry["terminals"].append(settings)
            terminals.append(table)

        attach(applier, "Train scores", 260, [])
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


def sequence_patterns():
    patterns = []
    for index in range(16):
        # Four customers repeat B,C: identical customer support, different embeddings.
        if index < 4:
            patterns.append([A, B, C, B, C, D])
        else:
            patterns.append([A, B, C, D])
    for index in range(8):
        patterns.append([A, B, C] + ([E] if index < 4 else []))
    for index in range(8):
        patterns.append([A, B] + ([F] if index < 4 else []) + [E])
    for index in range(8):
        patterns.append([A, F, E])
    for index in range(8):
        patterns.append([G, F, E])
    for index in range(8):
        patterns.append(([A] if index == 0 else []) + [G, H])
    for index in range(8):
        patterns.append([C, B, C, D])
    return patterns


def sequence_rows(patterns, first_id, simultaneous=False):
    rows = []
    for index, pattern in enumerate(patterns):
        time = 0
        previous = None
        for item in pattern:
            # Change only the time column; all twenty F,E pairs become one itemset.
            if not (simultaneous and previous == F and item == E):
                time += 1
            rows.append([first_id + index, time, item])
            previous = item
    return rows


def sequence_design(patterns):
    # This is an independent count of customer-level ordered subsequences, NOT an
    # assertion about Modeler's support denominator, repeated-event occurrence or pruning.
    # All patterns have <=6 events, so exhaustive combinations here are bounded.
    counts = {}
    embeddings = {}
    instances = dict((item, 0) for item in ITEMS)
    first_seen = []
    for pattern in patterns:
        seen = set()
        for item in pattern:
            instances[item] += 1
            if item not in first_seen:
                first_seen.append(item)
        for size in range(1, len(pattern) + 1):
            for indices in combinations(range(len(pattern)), size):
                key = tuple(pattern[index] for index in indices)
                embeddings[key] = embeddings.get(key, 0) + 1
                seen.add(key)
        for key in seen:
            counts[key] = counts.get(key, 0) + 1
    witnesses = []
    for pattern in [(F, E), (A, F, E), (G, F, E), (C, B, C), (C, B, D),
                    (C, B, C, D), (C, C, D), (C, D), (A, B, C), (B, C, D),
                    (G, F), (G, E), (G, H), (A, B, D)]:
        antecedent = pattern[:-1]
        witnesses.append({"antecedent": antecedent, "consequent": pattern[-1],
                          "antecedent_customers": counts[antecedent],
                          "joint_customers": counts[pattern],
                          "antecedent_support": counts[antecedent] / 64.0,
                          "joint_support": counts[pattern] / 64.0,
                          "confidence": counts[pattern] / float(counts[antecedent]),
                          "antecedent_embeddings": embeddings[antecedent],
                          "joint_embeddings": embeddings[pattern]})
    manifest["design"]["sequence"] = {
        "customers": len(patterns), "rows": sum(len(pattern) for pattern in patterns),
        "patterns_in_customer_order": patterns, "first_appearance": first_seen,
        "item_customer_counts": dict((item, counts[(item,)]) for item in ITEMS),
        "item_instance_counts": instances, "rule_witnesses": witnesses,
        "count_hypothesis": "ordered subsequences; one final-item consequent; customer support",
        "candidate_counts": [],
        "uncertainties": ["PMML support may be antecedent support (as in earlier oracle), not joint support",
                          "Repeated-event occurrence may count customers, embeddings or another unit",
                          "Modeler may prune or represent different consequent sequences/itemsets",
                          "Simultaneous F,E is an unordered transaction, never a strict F then E witness"]}
    # Report both possible threshold interpretations, never predict exact PMML rule counts.
    for support, maximum, confidence in [(20, 10, 20), (10, 10, 20), (20, 2, 20),
                                          (10, 10, 75), (20, 3, 20)]:
        ante_count = 0
        joint_count = 0
        for pattern, count in counts.items():
            if len(pattern) < 2 or len(pattern) > maximum:
                continue
            ante = counts[pattern[:-1]]
            if ante * 100 < support * 64 or count * 100 < confidence * ante:
                continue
            ante_count += 1
            if count * 100 >= support * 64:
                joint_count += 1
        manifest["design"]["sequence"]["candidate_counts"].append(
            {"min_supp": support, "max_size": maximum, "min_conf": confidence,
             "antecedent_threshold": ante_count, "joint_threshold": joint_count})


def segment_rows():
    rows = []
    # Every cell has four predictor profiles, each replicated with BOTH flag values.
    # Multiplication by 7 permutes the 20/40/120 replicates; no target leakage via inputs.
    for repeat in range(120):
        for position in range(4):
            for east, web, gold, count, positives in SEGMENT_CELLS:
                copies = count // 4
                if repeat >= copies:
                    continue
                positive = (repeat * 7 + position * 3) % copies < positives // 4
                rows.append([len(rows) + 1, "east" if east else "west",
                             "web" if web else "store", "gold" if gold else "bronze",
                             10.0 + 60.0 * east + 15.0 * gold + 5.0 * position,
                             1.0 + 6.0 * web + 2.0 * gold + 0.5 * position,
                             "Y" if positive else "N"])
    return rows


def segment_design(rows):
    segments = []
    for name, east, web, gold in [("base", None, None, None),
                                  ("east", 1, None, None), ("web", None, 1, None),
                                  ("gold", None, None, 1), ("east_web", 1, 1, None),
                                  ("east_gold", 1, None, 1), ("web_gold", None, 1, 1),
                                  ("east_web_gold", 1, 1, 1)]:
        selected = [row for row in rows
                    if (east is None or (row[1] == "east") == bool(east))
                    and (web is None or (row[2] == "web") == bool(web))
                    and (gold is None or (row[3] == "gold") == bool(gold))]
        positives = sum(1 for row in selected if row[-1] == "Y")
        segments.append({"segment": name, "rows": len(selected), "responses": positives,
                         "rate": positives / float(len(selected))})
    manifest["design"]["decisionlist"] = {
        "cells": SEGMENT_CELLS, "segments": segments, "rows": len(rows),
        "profile_count": 32, "every_profile_has_both_targets": True,
        "numeric_formulas": ["spend=10+60*east+15*gold+5*position",
                             "visits=1+6*web+2*gold+0.5*position; position=0..3"],
        "overlap": "east_web=160/120 Y; east_gold=160/108 Y; web_gold=160/112 Y; triple=80/72 Y",
        "uncertainties": ["Selected first-hit segments have residual rates after earlier rules remove rows",
                          "Search/confidence changes may retain the same winning model",
                          "Numeric bin boundaries and beam tie order require the new oracle"]}


def save_input(name, schema, rows, tests):
    # Failure of one dataset must not suppress the other family's cases.
    try:
        write_csv(name, schema, rows, tests)
    except ERRORS as exc:
        manifest["errors"].append({"case": None, "stage": "write_input", "file": name,
                                   "message": message(exc)})


def declare_inputs():
    patterns = sequence_patterns()
    sequence_design(patterns)
    grid = [[A], [F], [G], [C], [A, B], [B, C], [C, B], [A, F], [G, F],
            [C, B, C], [A, B, C, B, C], [E, H], [A, F, E], [G, F, E]]
    for simultaneous, suffix in [(False, ""), (True, "_same_time")]:
        save_input("sequence_train" + suffix + ".csv", SEQ_SCHEMA,
                   sequence_rows(patterns, 1, simultaneous),
                   "64 customers, 225 rows, 8 items, 2-6 time points; F,E share time only in same_time")
        save_input("sequence_grid" + suffix + ".csv", SEQ_SCHEMA,
                   sequence_rows(grid, 1001, simultaneous),
                   "14 new customers 1001-1014, partial/repeated/full histories; F,E same-time control")
    rows = segment_rows()
    segment_design(rows)
    save_input("dl_train.csv", SEGMENT_SCHEMA, rows,
               "1200 rows, 300 Y; 32 mixed-label profiles; nested and crossing non-pure segments")
    grid_rows = []
    for region in ["east", "west"]:
        for channel in ["store", "web"]:
            for tier in ["bronze", "gold"]:
                for spend in [9.0, 40.0, 55.0, 70.0, 101.0]:
                    for visits in [0.0, 4.5, 5.75, 7.0, 11.0]:
                        grid_rows.append([10001 + len(grid_rows), region, channel, tier,
                                          spend, visits, "N"])
    save_input("dl_grid.csv", SEGMENT_SCHEMA, grid_rows,
               "200 rows; all 8 categorical profiles crossed with 5x5 numeric boundary/interior/exterior points; respond=N is a scoring placeholder")


def declare_cases():
    def sequence(name, props, tests, same_time=False, compare_to=None):
        suffix = "_same_time" if same_time else ""
        add_case("sequence_" + name, "sequence", "sequence_train" + suffix + ".csv",
                 SEQ_SCHEMA, [], ["item"], SEQUENCE_FIELDS + props, tests,
                 score_data="sequence_grid" + suffix + ".csv", compare_to=compare_to)

    sequence("default", [], "Defaults except required field bindings; capture effective thresholds and size")
    sequence("support10", [("min_supp", 10.0)],
             "Low support ordering baseline; 27 independent one-consequent candidates at confidence 20",
             compare_to="sequence_default")
    # Two controls anchor a size-only 2 versus 3 comparison at the same explicit support.
    sequence("support20_size2", [("min_supp", 20.0), ("max_size", 2)],
             "Explicit support 20 percent and size 2; compare size3 for size only",
             compare_to="sequence_default")
    sequence("conf75", [("min_supp", 10.0), ("min_conf", 75.0)],
             "Confidence 75 percent at support10; boundary includes exact .75 ties",
             compare_to="sequence_support10")
    sequence("size3", [("min_supp", 20.0), ("max_size", 3)],
             "Size 3 at support20; only max_size differs from support20_size2",
             compare_to="sequence_support20_size2")
    sequence("same_time", [("min_supp", 20.0), ("max_size", 3)],
             "Identical items and customer order to size3; only adjacent F,E timestamps merged",
             same_time=True, compare_to="sequence_size3")

    def lists(name, props, tests, inputs=None, compare_to="dl_default"):
        add_case("dl_" + name, "decisionlist", "dl_train.csv", SEGMENT_SCHEMA, ["respond"],
                 SEGMENT_INPUTS if inputs is None else inputs, props, tests,
                 score_data="dl_grid.csv", compare_to=compare_to)

    lists("default", [], "Defaults; target Flag N,Y makes Y the response; no partition/weights",
          compare_to=None)
    lists("max_rules2", [("max_rules", 2)], "List depth 2 excluding remainder")
    lists("max_rules4", [("max_rules", 4)], "List depth 4 excluding remainder")
    lists("search", [("mode", "Expert"), ("max_models_per_cycle", 2),
                      ("max_rules_per_cycle", 3), ("max_segments_per_rule", 2)],
          "Expert list beam width 2, segment-rule beam width 3, segment-rule depth cap 2; widths/depth changed together")
    lists("min_group", [("min_group_size", 120), ("min_group_size_pct", 10.0)],
          "Both size floors are 120/1200; excludes 80-row triple, permits 160-row pair groups")
    lists("confidence80", [("confidence_level", 80.0)],
          "Confidence threshold 80 instead of observed earlier default 95; read back actual default")
    lists("numeric", [("mode", "Expert"), ("bin_method", "EqualWidth"), ("bin_count", 4)],
          "Only spend/visits inputs; Expert four equal-width bins; categories retain storage but role None",
          inputs=["spend", "visits"])


try:
    # Declare every case before any I/O or Modeler setup can fail.
    declare_cases()
    taskrunner = modeler.script.session().getTaskRunner()
    ensure_clean_dir(DIR)
    declare_inputs()
    write_manifest()
    for case in manifest["cases"]:
        build_case(case)
except ERRORS as exc:
    manifest["errors"].append({"case": None, "stage": "setup", "message": message(exc)})
finally:
    write_manifest()
