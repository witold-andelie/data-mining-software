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


node = setup("variablefile", needs_input=False)

prop("full_filename", DATA_FILE)
prop("full_filename", "D:/data/other file.csv")
prop("skip_header", 5)
prop("skip_header", 70000)
prop("num_fields_auto", False)
prop("num_fields", 12)
prop("num_fields", 0)
prop("delimit_space", True)
prop("delimit_tab", True)
prop("delimit_new_line", True)
prop("delimit_non_printing", True)
prop("delimit_comma", False)
prop("delimit_other", True)
prop("other", ";")
prop("other", "")
props("decimal_symbol", ["Comma", "Period", "Default"])
prop("multi_blank", False)
prop("read_field_names", False)
props("strip_spaces", ["Left", "Right", "Both", "None"])
props("invalid_char_mode", ["Replace", "Discard"])
prop("invalid_char_replacement", "|")
prop("break_case_by_newline", True)
prop("lines_to_scan", 500)
prop("lines_to_scan", 0)
prop("auto_recognize_datetime", False)
props("quotes_1", ["Discard", "PairAndDiscard", "IncludeAsText"])
props("quotes_2", ["Discard", "PairAndDiscard", "IncludeAsText"])
props("encoding", ["SystemDefault", "UTF-8", "StreamDefault"])
prop("encoding", "GBK")

prop("full_filename", DATA_FILE)
custom("instantiate by running a temporary Table node", run_temp_table)
# Per-field keyed settings.
keyed("use_custom_values", "Age", True)
keyed("values", "Age", [1, 100])
for v in ["Input", "Target", "Both", "None", "Partition", "Split", "Frequency", "RecordID"]:
    keyed("direction", "Age", v)
for v in ["Range", "Flag", "Set", "Typeless", "Discrete", "OrderedSet", "Default"]:
    keyed("type", "Age", v)
for v in ["Range", "Flag", "Set", "Typeless", "Discrete", "OrderedSet"]:
    keyed("measure_type", "Age", v)
for v in ["None", "Nullify", "Coerce", "Discard", "Warn", "Abort"]:
    keyed("check", "Age", v)
for v in ["Unknown", "String", "Integer", "Real", "Time", "Date", "Timestamp"]:
    keyed("custom_storage_type", "Age", v)
keyed("custom_storage", "Age", "Real")
keyed("custom_date_format", "Age", "DD/MM/YYYY")
keyed("custom_time_format", "Age", "HH:MM:SS")
keyed("custom_decimal_symbol", "Age", ",")
keyed("custom_list_storage_type", "Age", "Integer")
keyed("custom_list_depth", "Age", 2)
keyed("geo_type", "Age", "Point")
keyed("geo_coordinates", "Age", "2D")
keyed("values", "Drug", ["drugA", "drugB"])
keyed("missing_values", "Age", [0])
keyed("description", "Age", "Age of patient")


finish()
