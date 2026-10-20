# Sequence tie-order experiments, for the coordinator to run in Modeler 19 ONLY.
# This task only writes this probe; it does not run Modeler or alter shared data.
# Jython 2.7, public modeler.api. No implementation inspection or external packages.
# Sources: Algorithms Guide 19.0, Sequence Algorithm; Python Scripting and
# Automation Guide 19.0, sequence properties (Table 145) and source/type APIs.
# Same source/type/train/save API pattern as train_next_streams.py.
import json
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/seq_ties"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
ITEMS = ["amber", "bravo", "copper", "indigo", "mint", "teal", "violet", "zeta"]
A, B, C, D, E, F, G, H = "zeta", "amber", "mint", "bravo", "violet", "copper", "teal", "indigo"
FIELDS = [("id_field", "customer"), ("contiguous", True),
          ("use_time_field", True), ("time_field", "time"),
          ("content_fields", ["item"])]
BUILD_PROPERTIES = ["id_field", "contiguous", "use_time_field", "time_field",
                    "content_fields", "min_supp", "min_conf", "max_size",
                    "max_predictions", "mode", "use_pruning", "pruning_value",
                    "use_gaps", "use_max_duration", "set_mem_sequences", "mem_sequences"]
manifest = {"label": "seq_ties", "saved": [], "errors": [], "cases": [],
            "inputs": [], "design": {}, "model_name": "train_next"}


def patterns():
    # Identical semantic customers to train_next_streams.py, in original order.
    result = []
    for i in range(16):
        result.append([A, B, C, B, C, D] if i < 4 else [A, B, C, D])
    for i in range(8):
        result.append([A, B, C] + ([E] if i < 4 else []))
    for i in range(8):
        result.append([A, B] + ([F] if i < 4 else []) + [E])
    result += [[A, F, E] for i in range(8)]
    result += [[G, F, E] for i in range(8)]
    result += [([A] if i == 0 else []) + [G, H] for i in range(8)]
    result += [[C, B, C, D] for i in range(8)]
    return result


def designs():
    base = patterns()
    blocks = [range(0, 16), range(16, 24), range(24, 32), range(32, 40),
              range(40, 48), range(48, 56), range(56, 64)]
    interleaved = [block[i] for i in range(16) for block in blocks if i < len(block)]
    identity = dict((x, x) for x in ITEMS)
    rename = dict(zip(ITEMS, list(reversed(ITEMS))))
    # Each tuple: name, customer permutation, item bijection, Type domain,
    # minimum confidence, maximum size, rare extra event, purpose.
    return [
        ("baseline", list(range(64)), identity, ITEMS, 20.0, 10, False,
         "Reproduce train_next/sequence_support10, including model name and Type domain"),
        ("repeat", list(range(64)), identity, ITEMS, 20.0, 10, False,
         "Repeat baseline in a fresh stream; test run-to-run determinism"),
        ("reverse", list(reversed(range(64))), identity, ITEMS, 20.0, 10, False,
         "Reverse whole customers, assign increasing ids; preserve each history"),
        ("rotate24", list(range(24, 64)) + list(range(24)), identity, ITEMS, 20.0, 10, False,
         "Move copper/violet customers before the repeated amber/mint histories"),
        ("interleave", interleaved, identity, ITEMS, 20.0, 10, False,
         "Interleave seven customer blocks to change parent-entry times"),
        ("rename", list(range(64)), rename, [rename[x] for x in ITEMS], 20.0, 10, False,
         "Reverse lexical identities while preserving semantic domain positions"),
        ("domain_reverse", list(range(64)), identity, list(reversed(ITEMS)), 20.0, 10, False,
         "Reverse only declared Type value order"),
        ("confidence75", list(range(64)), identity, ITEMS, 75.0, 10, False,
         "Filter other rules; test whether final sorting perturbs surviving ties"),
        ("size3", list(range(64)), identity, ITEMS, 20.0, 3, False,
         "Remove size-four sequences; test lattice/rule-table context dependence"),
        ("noise_domain", list(range(64)), identity, ITEMS + ["umber"], 20.0, 10, False,
         "Add an unused Type value as control for the rare-event case"),
        ("noise_event", list(range(64)), identity, ITEMS + ["umber"], 20.0, 10, True,
         "Append umber at time 7 for customer 3 only; no frequent rule changes"),
    ]


def rows(histories, mapping, first_id=1):
    return [[first_id + i, t + 1, mapping.get(item, item)]
            for i, history in enumerate(histories) for t, item in enumerate(history)]


def snapshot(value):
    if value is None or isinstance(value, (bool, int, long, float, basestring)):
        return value
    if isinstance(value, (list, tuple, java.util.List)):
        return [snapshot(x) for x in value]
    return unicode(value)


def error(exc):
    get = getattr(exc, "getMessage", None)
    return unicode(get()) if get is not None else unicode(exc)


def write_manifest():
    handle = open(OUT_DIR + "/manifest.json", "w")
    try:
        json.dump(manifest, handle, indent=2, sort_keys=True)
    finally:
        handle.close()


def set_properties(node, props, entry):
    for name, value in props:
        node.setPropertyValue(name, value)
        actual = snapshot(node.getPropertyValue(name))
        entry.setdefault("readback", []).append([name, actual])
        expected = snapshot(value)
        # Enumerations read back upper-cased (decimal_symbol Period -> PERIOD).
        same_text = (isinstance(actual, basestring) and isinstance(expected, basestring)
                     and actual.lower() == expected.lower())
        if actual != expected and not same_text:
            raise ValueError("coerced property " + name + ": " + unicode(actual))


def instantiate(stream, node):
    table = stream.createAt("table", "Instantiate", 1000, 800)
    stream.link(node, table)
    try:
        table.run([])
    finally:
        stream.delete(table)


def typed_source(stream, filename, domain, y, entry):
    source = stream.createAt("variablefile", filename, 60, y)
    source_entry = {"file": filename}
    entry.setdefault("sources", []).append(source_entry)
    set_properties(source, [("full_filename", DIR + "/" + filename),
        ("read_field_names", True), ("delimit_comma", True), ("delimit_space", False),
        ("delimit_tab", False), ("quotes_2", "PairAndDiscard"), ("strip_spaces", "None"),
        ("decimal_symbol", "Period"), ("encoding", "UTF-8"),
        ("auto_recognize_datetime", False), ("lines_to_scan", 1000)], source_entry)
    instantiate(stream, source)
    for field, storage in [("customer", "Integer"), ("time", "Integer"), ("item", "String")]:
        source.setKeyedPropertyValue("custom_storage_type", field, storage)
    typed = stream.createAt("type", "Type " + filename, 260, y)
    stream.link(source, typed)
    instantiate(stream, typed)
    for field, measure in [("customer", "Range"), ("time", "Range"), ("item", "Set")]:
        typed.setKeyedPropertyValue("type", field, measure)
        typed.setKeyedPropertyValue("direction", field, "Input" if field == "item" else "None")
        typed.setKeyedPropertyValue("check", field, "None")
    typed.setKeyedPropertyValue("values", "item", domain)
    typed.setKeyedPropertyValue("extend_values", "item", False)
    instantiate(stream, typed)
    actual = snapshot(typed.getKeyedPropertyValue("values", "item"))
    source_entry["effective_domain"] = actual
    if actual != domain:
        raise ValueError("Type domain reordered/coerced: " + unicode(actual))
    return typed


def write_csv(filename, data):
    lines = ["customer,time,item"]
    for customer, time, item in data:
        lines.append('%d,%d,"%s"' % (customer, time, item.replace('"', '""')))
    handle = open(DIR + "/" + filename, "wb")
    try:
        handle.write(("\n".join(lines) + "\n").encode("utf-8"))
    finally:
        handle.close()
    manifest["saved"].append(filename)
    manifest["inputs"].append({"file": filename, "rows": len(data)})


def build(entry, taskrunner):
    stream = None
    stage = "create_stream"
    try:
        stream = taskrunner.createStream(entry["case"], False, False)
        set_properties(stream, [("decimal_places", 15), ("decimal_symbol", "Period"),
            ("enable_parallelism", False), ("create_model_applier_for_new_models", True),
            ("create_model_applier_update_links", "createEnabled")], entry)
        stage = "training_source"
        training = typed_source(stream, entry["data"], entry["domain"], 100, entry)
        builder = stream.createAt("sequence", entry["case"], 480, 100)
        stream.link(training, builder)
        stage = "builder_properties"
        props = FIELDS + [("use_model_name", True), ("model_name", "train_next"),
            ("min_supp", 10.0), ("min_conf", entry["confidence"]),
            ("max_size", entry["max_size"])]
        entry["properties"] = props
        set_properties(builder, props, entry)
        effective = {}
        for name in BUILD_PROPERTIES:
            effective[name] = snapshot(builder.getPropertyValue(name))
        entry["builder_effective"] = effective
        stage = "train"
        results = []
        builder.run(results)
        models = [result for result in results if isinstance(result, modeler.api.ModelOutput)]
        if not models:
            raise RuntimeError("no ModelOutput returned")
        placed = []
        iterator = stream.iterator()
        while iterator.hasNext():
            node = iterator.next()
            if unicode(node.getTypeName()).startswith("apply"):
                placed.append(node)
        if len(placed) > 1:
            raise RuntimeError("unexpected multiple automatically placed nuggets")
        stage = "score_source"
        grid = typed_source(stream, entry["score_data"], entry["domain"], 420, entry)
        terminals = []
        for source, name, y in [(training, "Train scores", 260), (grid, "Grid scores", 420)]:
            if name == "Train scores" and placed:
                nugget = placed[0]
                nugget.setXYPosition(480, y)
            else:
                nugget = stream.createModelApplierAt(models[0], name, 480, y)
                stream.link(source, nugget)
            table = stream.createAt("table", name, 760, y)
            stream.link(nugget, table)
            terminals.append(table)
        stage = "save_stream"
        taskrunner.saveStreamToFile(stream, DIR + "/" + entry["file"])
        manifest["saved"].append(entry["file"])
        entry["status"] = "saved"
        stage = "score"
        for table in terminals:
            table.run([])
    except ERRORS as exc:
        entry["status"] = "failed"
        manifest["errors"].append({"case": entry["case"], "stage": stage, "message": error(exc)})
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS as exc:
                manifest["errors"].append({"case": entry["case"], "stage": "clear", "message": error(exc)})
        write_manifest()


def main():
    # Refuse existing outputs instead of deleting or overwriting a previous experiment.
    directory = java.io.File(DIR)
    if not directory.exists() and not directory.mkdirs():
        raise IOError("cannot create dedicated label " + DIR)
    grid = [[A], [F], [G], [C], [A,B], [B,C], [C,B], [A,F], [G,F],
            [C,B,C], [A,B,C,B,C], [E,H], [A,F,E], [G,F,E]]
    base = patterns()
    for name, permutation, mapping, domain, confidence, maximum, noise, purpose in designs():
        case = "sequence_" + name
        entry = {"case": case, "file": case + ".str", "data": case + "_train.csv",
                 "score_data": case + "_grid.csv", "domain": domain,
                 "confidence": confidence, "max_size": maximum, "status": "planned",
                 "purpose": purpose, "source_customer_indices": permutation,
                 "item_bijection": mapping, "noise_event": noise}
        manifest["cases"].append(entry)
    for entry in manifest["cases"]:
        for filename in [entry["file"], entry["data"], entry["score_data"]]:
            if java.io.File(DIR + "/" + filename).exists():
                raise IOError("refusing to overwrite existing probe artifact: " + filename)
    for entry in manifest["cases"]:
        histories = [list(base[i]) for i in entry["source_customer_indices"]]
        if entry["noise_event"]:
            histories[2].append("umber")
        entry["patterns_in_customer_order"] = histories
        write_csv(entry["data"], rows(histories, entry["item_bijection"]))
        write_csv(entry["score_data"], rows(grid, entry["item_bijection"], 1001))
    write_manifest()
    runner = modeler.script.session().getTaskRunner()
    for entry in manifest["cases"]:
        build(entry, runner)


try:
    main()
except ERRORS as exc:
    manifest["errors"].append({"case": None, "stage": "setup", "message": error(exc)})
finally:
    write_manifest()
