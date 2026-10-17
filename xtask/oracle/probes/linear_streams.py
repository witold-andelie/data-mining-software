# Probe: Linear node (alm-extension) with automatic data preparation and model selection, in
# corpus/linear/. Each stream is source -> [Derive] -> Type -> linear build node, and
# Type -> nugget -> Table, with decimal_places = 15.
# Run with `cargo xtask probe linear_streams` (IBM SPSS Modeler, Jython 2.7, public API only).
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
LINEAR_DIR = "D:/data-mining-software/corpus/linear"
SOURCES = {
    "DRUG1n": "D:/data-mining-software/corpus/models/DRUG1n",
    "goods.dat": "D:/data-mining-software/corpus/team/goods.dat",
}
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
SEED = 12345

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"saved": [], "errors": [], "cases": []}

DRUG_SCHEMA = [("Age", "Range"), ("Sex", "Flag"), ("BP", "Set"), ("Cholesterol", "Flag"),
               ("Na", "Range"), ("K", "Range"), ("Drug", "Set")]
GOODS_SCHEMA = [("Class", "Set"), ("Cost", "Range"), ("Promotion", "Range"), ("Before", "Range"),
                ("After", "Range"), ("increase", "Range")]


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


def copy_file(name):
    src = open(SOURCES[name], "rb")
    try:
        data = src.read()
    finally:
        src.close()
    dst = open("%s/%s" % (LINEAR_DIR, name), "wb")
    try:
        dst.write(data)
    finally:
        dst.close()
    manifest["saved"].append(name)


def instantiate_node(stream, node):
    table = stream.createAt("table", "TempTable", 760, 360)
    stream.link(node, table)
    try:
        table.run([])
    finally:
        stream.delete(table)


def find_applier(stream):
    it = stream.iterator()
    while it.hasNext():
        n = it.next()
        if unicode(n.getTypeName()).startswith("apply"):
            return n
    return None


def build_case(case_name, data_file, schema, target, inputs, props, derive=None):
    stream = None
    try:
        stream = taskrunner.createStream(case_name, False, False)
        stream.setPropertyValue("decimal_places", 15)
        stream.setPropertyValue("create_model_applier_for_new_models", True)
        stream.setPropertyValue("create_model_applier_update_links", "createEnabled")
        src = stream.createAt("variablefile", data_file, 100, 100)
        src.setPropertyValue("full_filename", "%s/%s" % (LINEAR_DIR, data_file))
        src.setPropertyValue("read_field_names", True)
        last = src
        if derive is not None:
            name, formula = derive
            node = stream.createAt("derive", name, 180, 100)
            node.setPropertyValue("new_name", name)
            node.setPropertyValue("result_type", "Formula")
            node.setPropertyValue("formula_expr", formula)
            stream.link(last, node)
            last = node
        type_node = stream.createAt("type", "Type", 260, 100)
        stream.link(last, type_node)
        instantiate_node(stream, type_node)
        for field_name, measure in schema:
            type_node.setKeyedPropertyValue("type", field_name, measure)
            direction = "None"
            if field_name == target:
                direction = "Target"
            elif field_name in inputs:
                direction = "Input"
            type_node.setKeyedPropertyValue("direction", field_name, direction)
        instantiate_node(stream, type_node)

        build_node = stream.createAt("linear", case_name, 460, 100)
        stream.link(type_node, build_node)
        build_node.setPropertyValue("use_random_seed", True)
        build_node.setPropertyValue("random_seed", SEED)
        for name, value in props:
            build_node.setPropertyValue(name, value)
        results = []
        build_node.run(results)
        applier = find_applier(stream)
        if applier is None:
            applier = stream.createModelApplierAt(results[0], case_name, 460, 240)
            stream.link(type_node, applier)
        else:
            applier.setXYPosition(460, 240)
        table_node = stream.createAt("table", "Table", 640, 240)
        stream.link(applier, table_node)
        table_node.run([])
        file_name = "%s.str" % case_name
        taskrunner.saveStreamToFile(stream, "%s/%s" % (LINEAR_DIR, file_name))
        manifest["saved"].append(file_name)
        manifest["cases"].append({"case": case_name, "file": file_name, "data": data_file,
                                  "target": target, "inputs": list(inputs),
                                  "properties": [[n, v] for n, v in props]})
    except ERRORS, e:
        manifest["errors"].append({"case": case_name, "message": message(e)})
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS:
                pass


ensure_clean_dir(LINEAR_DIR)
copy_file("DRUG1n")
copy_file("goods.dat")

DRUG_INPUTS = ["Age", "Sex", "BP", "Cholesterol", "K"]
GOODS_INPUTS = ["Class", "Cost", "Promotion", "Before"]
INCREASE = ("increase", "(After - Before) / Before * 100")
NO_ADP = ("use_auto_data_preparation", False)
ADP = ("use_auto_data_preparation", True)

# Data preparation alone, then model selection alone, then both (a new node's defaults).
build_case("lin_adp_none_drug", "DRUG1n", DRUG_SCHEMA, "Na", DRUG_INPUTS,
           [ADP, ("model_selection", "None")])
build_case("lin_fwd_drug", "DRUG1n", DRUG_SCHEMA, "Na", DRUG_INPUTS,
           [NO_ADP, ("model_selection", "ForwardStepwise")])
build_case("lin_default_drug", "DRUG1n", DRUG_SCHEMA, "Na", DRUG_INPUTS, [])
build_case("lin_adp_none_goods", "goods.dat", GOODS_SCHEMA, "increase", GOODS_INPUTS,
           [ADP, ("model_selection", "None")], INCREASE)
build_case("lin_fwd_goods", "goods.dat", GOODS_SCHEMA, "increase", GOODS_INPUTS,
           [NO_ADP, ("model_selection", "ForwardStepwise")], INCREASE)
build_case("lin_default_goods", "goods.dat", GOODS_SCHEMA, "increase", GOODS_INPUTS, [], INCREASE)

handle = open(OUT_DIR + "/manifest.json", "w")
try:
    json.dump(manifest, handle, indent=2, sort_keys=True)
finally:
    handle.close()
