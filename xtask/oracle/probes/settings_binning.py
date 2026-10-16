# Probe: node-settings mapping for one node type (see docs/formats/nodes/). Each step sets ONE
# scripting property to a non-default value and saves the stream, so every saved file differs from
# the previous step's file (`base` in the manifest) by exactly that setting.
# Run with `cargo xtask probe settings_<type>` (IBM SPSS Modeler, Jython 2.7, public API only).
import json
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
DATA_FILE = "D:/data-mining-software/corpus/ibm-demo-data/DRUG1n"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"saved": [], "steps": [], "errors": [], "failed": [], "restore_failed": []}
STATE = {"index": 0, "base": "base.str", "stream": None, "node": None, "source": None}


def message(exc):
    get = getattr(exc, "getMessage", None)
    if get is not None:
        return unicode(get())
    return unicode(exc)


def text(value):
    if value is None:
        return None
    if isinstance(value, (bool, int, long, float)):
        return value
    if isinstance(value, (list, tuple)):
        return [text(v) for v in value]
    if isinstance(value, dict):
        return dict([(unicode(k), text(v)) for k, v in value.items()])
    try:
        return unicode(value)
    except Exception:
        return repr(value)


def save(file_name):
    taskrunner.saveStreamToFile(STATE["stream"], "%s/%s" % (OUT_DIR, file_name))
    manifest["saved"].append(file_name)


def next_file():
    STATE["index"] += 1
    return "s%03d.str" % STATE["index"]


def read_prop(node, name, key):
    try:
        if key is None:
            return node.getPropertyValue(name)
        return node.getKeyedPropertyValue(name, key)
    except ERRORS, e:
        return None


def write_prop(node, name, key, value):
    if key is None:
        node.setPropertyValue(name, value)
    else:
        node.setKeyedPropertyValue(name, key, value)


def step(name, key, value, node=None):
    node = node or STATE["node"]
    label = name if key is None else "%s[%s]" % (name, key)
    file_name = next_file()
    old = read_prop(node, name, key)
    entry = {"file": file_name, "property": name, "key": key, "set": text(value),
             "default": text(old), "base": STATE["base"], "node": text(node.getTypeName())}
    try:
        write_prop(node, name, key, value)
    except ERRORS, e:
        manifest["failed"].append({"property": label, "set": text(value), "message": message(e)})
        return
    entry["readback"] = text(read_prop(node, name, key))
    try:
        save(file_name)
    except ERRORS, e:
        manifest["failed"].append({"property": label, "set": text(value), "message": "save: " + message(e)})
        return
    manifest["steps"].append(entry)
    STATE["base"] = file_name


def prop(name, value):
    step(name, None, value)


def props(name, values):
    for value in values:
        step(name, None, value)


def keyed(name, key, value):
    step(name, key, value)


def custom(description, change, persist=True):
    """Apply change(stream, node), save, and (if persist) use the saved file as the new base."""
    file_name = next_file()
    entry = {"file": file_name, "property": None, "change": description, "base": STATE["base"],
             "node": text(STATE["node"].getTypeName())}
    try:
        change(STATE["stream"], STATE["node"])
        save(file_name)
    except ERRORS, e:
        manifest["failed"].append({"change": description, "message": message(e)})
        return
    manifest["steps"].append(entry)
    STATE["base"] = file_name


def info(label, call):
    """Record a read-only observation (getPropertyValue etc.) into manifest['info']."""
    try:
        manifest.setdefault("info", {})[label] = text(call())
    except ERRORS, e:
        manifest.setdefault("info", {})[label] = "ERROR: " + message(e)


def setup(node_type, needs_input=True, inputs=1, source_type="variablefile"):
    stream = taskrunner.createStream("settings_" + node_type, False, False)
    STATE["stream"] = stream
    sources = []
    if needs_input:
        for i in range(inputs):
            src = stream.createAt(source_type, "Source%d" % (i + 1), 100, 100 + 80 * i)
            if source_type == "variablefile":
                src.setPropertyValue("full_filename", DATA_FILE)
            sources.append(src)
    node = stream.createAt(node_type, "Node", 300, 100)
    for src in sources:
        stream.link(src, node)
    STATE["node"] = node
    STATE["source"] = sources[0] if sources else None
    save("base.str")
    save("base_resave.str")
    return node


def finish():
    handle = open(OUT_DIR + "/manifest.json", "w")
    try:
        json.dump(manifest, handle, indent=1, sort_keys=True)
    finally:
        handle.close()


def run_temp_table(stream, node):
    """Instantiate a node's output (fields) by running a temporary Table node."""
    table = stream.createAt("table", "Temp table", 500, 400)
    stream.link(node, table)
    try:
        table.run(None)
    finally:
        stream.delete(table)


def instantiate_source(description="instantiate fields by running a temporary Table node on the source"):
    custom(description, lambda s, n: run_temp_table(s, STATE["source"]))


node = setup("binning", needs_input=True)
prop("fields", ["Na", "K"])
for v in ["EqualCount", "Rank", "SDev", "Optimal", "FixedWidth"]:
    prop("method", v)
for v in ["Always", "IfNecessary"]:
    prop("recalculate_bins", v)
prop("fixed_width_name_extension", "_bin")
for v in ["Prefix", "Suffix"]:
    prop("fixed_width_add_as", v)
for v in ["Count", "Width"]:
    prop("fixed_bin_method", v)
prop("fixed_bin_count", 7)
prop("fixed_bin_width", 3.5)
prop("equal_count_name_extension", "_tl")
for v in ["Prefix", "Suffix"]:
    prop("equal_count_add_as", v)
for p in ["tile4", "tile5", "tile10", "tile20", "tile100", "use_custom_tile"]:
    prop(p, True)
prop("custom_tile_name_extension", "_ct")
for v in ["Prefix", "Suffix"]:
    prop("custom_tile_add_as", v)
prop("custom_tile", 8)
for v in ["ValueSum", "RecordCount"]:
    prop("equal_count_method", v)
for v in ["Current", "Random", "Next"]:
    prop("tied_values_method", v)
for v in ["Descending", "Ascending"]:
    prop("rank_order", v)
for v in ["Prefix", "Suffix"]:
    prop("rank_add_as", v)
prop("rank", False)
prop("rank_name_extension", "_rk")
prop("rank_fractional", True)
prop("rank_fractional_name_extension", "_frk")
prop("rank_pct", True)
prop("rank_pct_name_extension", "_prk")
prop("sdev_name_extension", "_sd")
for v in ["Prefix", "Suffix"]:
    prop("sdev_add_as", v)
for v in ["One", "Two", "Three"]:
    prop("sdev_count", v)
prop("optimal_name_extension", "_op")
for v in ["Prefix", "Suffix"]:
    prop("optimal_add_as", v)
prop("optimal_supervisor_field", "Drug")
prop("optimal_merge_bins", False)
prop("optimal_small_bin_threshold", 20)
prop("optimal_pre_bin", False)
prop("optimal_max_bins", 9)
for v in ["Exclusive", "Inclusive"]:
    prop("optimal_lower_end_point", v)
for v in ["Bounded", "Unbounded"]:
    prop("optimal_first_bin", v)
for v in ["Bounded", "Unbounded"]:
    prop("optimal_last_bin", v)


finish()
