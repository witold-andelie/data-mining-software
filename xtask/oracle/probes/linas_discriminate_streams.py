# Coordinator-only probe. Do not run as part of the numerical investigation.
# Modeler 19 / Jython 2.7, public scripting API, following linearas_streams.py.
# Fixed synthetic data; each case changes one aspect of exact_base or a stated
# control. Direct PMML exports preserve parameters even if scoring is identical.
# See target/linas/PREDICTIONS.md for falsifiable numerical predictions.
# No existing files are deleted or overwritten: use an absent/empty DIR.
import json
import java.io.File
import java.lang.Exception
import java.util.List
import modeler.api

OUT_DIR = @OUT_DIR@
DIR = "D:/data-mining-software/corpus/linas_discriminate"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
SCHEMA = [("id", "Integer")] + [(name, "Real") for name in
                                    ["x1", "x2", "x3", "x4", "x5", "y"]]
PROPERTIES = ["custom_fields", "target", "inputs", "intercept", "factor_order",
              "max_effects", "max_steps"]
taskrunner = modeler.script.session().getTaskRunner()
manifest = {"label": "linas_discriminate", "cases": [], "errors": [], "saved": []}


def numeric_fields(index):
    # Same public synthetic generator as models_as_streams.py, no corpus copying.
    shared = float(index)
    return [shared + float((index * 3) % 7),
            0.5 * shared + float((index * 5) % 11),
            0.25 * shared + float((index * 2) % 5),
            float((index * 7) % 13) + 0.5 * float(index % 3),
            float((index * 4) % 9) + 0.25 * float((index * 3) % 5)]


def numeric_target(fields):
    x1, x2, x3, x4, x5 = fields
    return 2.0 * x1 - 0.5 * x2 + 0.25 * x3 + x4 - 0.5 * x5


def build_cases():
    base = [[i + 1] + numeric_fields(i) + [numeric_target(numeric_fields(i))]
            for i in range(48)]
    inputs = ["x1", "x2", "x3", "x4", "x5"]
    cases = []

    def add(name, rows, test, selected=None):
        cases.append({"case": name, "rows": rows, "inputs": list(
            inputs if selected is None else selected), "test": test})

    add("exact_base", base, "48-row exact-linear reference; expected observed oracle M5+x4^2")
    add("exact_repeat", base, "Identical reference again: repeated training determinism")
    add("exact_reverse", list(reversed(base)), "Record order reversed; all values and IDs retained")
    add("exact_rotate1", base[1:] + base[:1], "Move first record to last; all values retained")
    add("exact_even_odd", base[::2] + base[1::2], "Even record positions followed by odd positions")
    add("exact_sort_x1", sorted(base, key=lambda row: row[1]), "Sort records by x1; stable ties")
    add("inputs_reverse", base, "Reverse builder inputs; physical CSV schema fixed", list(reversed(inputs)))
    add("inputs_rotate1", base, "Rotate builder inputs by one; physical CSV schema fixed", inputs[1:] + inputs[:1])
    for name, scale in [("x_scale_up", 1024.0), ("x_scale_down", 1.0 / 1024.0)]:
        add(name, [[r[0]] + [scale * x for x in r[1:6]] + [r[6]] for r in base],
            "Scale every x by %.17g; y fixed (power of two, exact binary scaling)" % scale)
    add("x_shift", [[r[0]] + [x + 1048576.0 for x in r[1:6]] + [r[6]] for r in base],
        "Translate every x by 2^20; true linear span unchanged")
    add("y_scale_up", [r[:6] + [r[6] * 1024.0] for r in base],
        "Scale y by 2^10; x fixed")
    add("y_shift", [r[:6] + [r[6] + 1048576.0] for r in base],
        "Translate y by 2^20; x fixed")
    for name, amplitude in [("residual_tiny", 2.0 ** -20), ("residual_real", 0.125)]:
        add(name, [r[:6] + [r[6] + amplitude * float((i * 37) % 23 - 11)]
                   for i, r in enumerate(base)],
            "Add fixed residual amplitude %.17g; compare exact_base" % amplitude)

    # Orthogonal balanced bits. x2=x1+delta*v; y=u+v+w/8. After x2,
    # x1 has tolerance delta^2/(1+delta^2), far from either tested boundary.
    def tolerance_rows(delta, scale):
        rows = []
        for i in range(64):
            u, v, w = [1.0 if i & (1 << b) else -1.0 for b in range(3)]
            rows.append([i + 1, scale * u, scale * (u + delta * v),
                         0.0, 0.0, 0.0, u + v + 0.125 * w])
        return rows

    add("tol_mid", tolerance_rows(2.0 ** -10, 1.0),
        "1e-12 < conditional correlation diagonal ~9.54e-7 < 1e-4", ["x1", "x2"])
    add("tol_mid_scale_up", tolerance_rows(2.0 ** -10, 1024.0),
        "Only scale tol_mid x by 2^10: correlation invariant, absolute raw pivots change", ["x1", "x2"])
    add("tol_low", tolerance_rows(2.0 ** -22, 1.0),
        "Conditional correlation diagonal ~5.68e-14, below both tolerances", ["x1", "x2"])
    add("tol_high", tolerance_rows(2.0 ** -5, 1.0),
        "Conditional correlation diagonal ~9.76e-4, above both tolerances", ["x1", "x2"])

    condition = []
    tie = []
    for i in range(64):
        u, v, w = [1.0 if i & (1 << b) else -1.0 for b in range(3)]
        condition.append([i + 1, u, u + v / 32.0, v + w / 8.0,
                          0.0, 0.0, 32.0 * u + 2.0 * v - w / 8.0])
        tie.append([i + 1, u, float((i // 2) % 4) - 1.5,
                    0.0, 0.0, 0.0, 2.0 * u])
    add("condition_b", condition,
        "After x2,x1, x3 passes a (~0.01538) but inflates x2 inverse diagonal to 66560 (maximum 66625)",
        ["x1", "x2", "x3"])
    add("tie_exact", tie, "Perfect y=2*x1; independent x2; strict > versus >= on exact score 1",
        ["x1", "x2"])
    return cases


def message(exc):
    get = getattr(exc, "getMessage", None)
    return unicode(get()) if get is not None else unicode(exc)


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


def prepare_directory():
    directory = java.io.File(DIR)
    if directory.exists():
        children = directory.listFiles()
        if children is None or len(children) != 0:
            raise IOError("probe requires absent or empty dedicated directory: " + DIR)
    elif not directory.mkdirs():
        raise IOError("cannot create " + DIR)


def write_csv(case):
    name = case["case"] + ".csv"
    lines = [",".join(field[0] for field in SCHEMA)]
    for row in case["rows"]:
        if len(row) != len(SCHEMA):
            raise ValueError("invalid row width in " + name)
        lines.append(",".join([str(row[0])] + ["%.17g" % x for x in row[1:]]))
    handle = open(DIR + "/" + name, "wb")
    try:
        handle.write((chr(10).join(lines) + chr(10)).encode("utf-8"))
    finally:
        handle.close()
    manifest["saved"].append(name)


def set_properties(node, properties, entry):
    for key, value in properties:
        node.setPropertyValue(key, value)
        entry.setdefault("readback", []).append([key, snapshot(node.getPropertyValue(key))])


def read_properties(node):
    result = {}
    for key in PROPERTIES:
        try:
            result[key] = snapshot(node.getPropertyValue(key))
        except ERRORS as exc:
            result[key] = {"read_error": message(exc)}
    return result


def instantiate(stream, node):
    temporary = stream.createAt("table", "Instantiate", 1000, 800)
    stream.link(node, temporary)
    try:
        temporary.run([])
    finally:
        stream.delete(temporary)


def build_case(case):
    entry = {key: value for key, value in case.items() if key != "rows"}
    entry["row_count"] = len(case["rows"])
    entry["status"] = "planned"
    manifest["cases"].append(entry)
    stream = None
    stage = "write_csv"
    try:
        write_csv(case)
        stage = "create_stream"
        stream = taskrunner.createStream(case["case"], False, False)
        set_properties(stream, [("decimal_places", 15), ("decimal_symbol", "Period"),
            ("enable_parallelism", False), ("create_model_applier_for_new_models", True),
            ("create_model_applier_update_links", "createEnabled")], entry)
        stage = "source"
        source = stream.createAt("variablefile", case["case"] + ".csv", 60, 100)
        set_properties(source, [("full_filename", DIR + "/" + case["case"] + ".csv"),
            ("read_field_names", True), ("delimit_comma", True), ("delimit_space", False),
            ("delimit_tab", False), ("decimal_symbol", "Period"), ("encoding", "UTF-8"),
            ("auto_recognize_datetime", False), ("lines_to_scan", 1000)], entry)
        instantiate(stream, source)
        for field, storage in SCHEMA:
            source.setKeyedPropertyValue("custom_storage_type", field, storage)
        type_node = stream.createAt("type", "Types and roles", 260, 100)
        stream.link(source, type_node)
        instantiate(stream, type_node)
        for field, storage in SCHEMA:
            type_node.setKeyedPropertyValue("type", field, "Range")
            role = "Target" if field == "y" else "Input" if field in case["inputs"] else "None"
            type_node.setKeyedPropertyValue("direction", field, role)
            type_node.setKeyedPropertyValue("check", field, "None")
            if storage == "Real":
                type_node.setKeyedPropertyValue("export_places", field, 15)
        stage = "instantiate"
        instantiate(stream, type_node)
        stage = "builder"
        builder = stream.createAt("linearas", "Linear AS", 480, 100)
        stream.link(type_node, builder)
        entry["builder_defaults"] = read_properties(builder)
        set_properties(builder, [("use_model_name", True), ("model_name", "linas_discriminate"),
            ("custom_fields", True), ("target", "y"), ("inputs", list(case["inputs"]))], entry)
        entry["builder_effective"] = read_properties(builder)
        stage = "train"
        results = []
        builder.run(results)
        models = [result for result in results if isinstance(result, modeler.api.ModelOutput)]
        if len(models) != 1:
            raise RuntimeError("expected exactly one ModelOutput, got %d" % len(models))
        stage = "export_pmml"
        pmml = DIR + "/" + case["case"] + ".xml"
        taskrunner.exportModelToFile(models[0], pmml, modeler.api.FileFormat.XML)
        manifest["saved"].append(case["case"] + ".xml")
        stage = "score"
        appliers = []
        iterator = stream.iterator()
        while iterator.hasNext():
            node = iterator.next()
            if unicode(node.getTypeName()).startswith("apply"):
                appliers.append(node)
        if len(appliers) > 1:
            raise RuntimeError("unexpected multiple placed nuggets")
        applier = appliers[0] if appliers else stream.createModelApplierAt(models[0], "Scores", 480, 260)
        if not appliers:
            stream.link(type_node, applier)
        table = stream.createAt("table", "Scores", 760, 260)
        stream.link(applier, table)
        table.run([])
        stage = "save_stream"
        taskrunner.saveStreamToFile(stream, DIR + "/" + case["case"] + ".str")
        manifest["saved"].append(case["case"] + ".str")
        entry["status"] = "saved"
    except ERRORS as exc:
        entry["status"] = "failed"
        manifest["errors"].append({"case": case["case"], "stage": stage, "message": message(exc)})
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS as exc:
                manifest["errors"].append({"case": case["case"], "stage": "clear", "message": message(exc)})
        write_manifest()


try:
    prepare_directory()
    for case in build_cases():
        build_case(case)
except ERRORS as exc:
    manifest["errors"].append({"case": None, "stage": "setup", "message": message(exc)})
finally:
    write_manifest()
