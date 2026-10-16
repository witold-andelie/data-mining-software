# Probe: stream properties (date/time formats, baselines, decimals, ...) -> saved stream property elements.
# Each step sets ONE stream property and saves; compare with the previous file. See docs/formats/stream-properties.md.
# Run with `cargo xtask probe stream_formats` (IBM SPSS Modeler, Jython 2.7, public API only).
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
             "default": text(old), "base": STATE["base"], "node": text(node.getTypeName() if hasattr(node, "getTypeName") else "stream")}
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


stream = taskrunner.createStream("stream_formats", False, False)
STATE["stream"] = stream
STATE["node"] = stream
save("base.str")
save("base_resave.str")
for p in ["date_format", "time_format", "date_baseline", "date_2digit_baseline", "decimal_places", "decimal_symbol"]:
    info("default_" + p, lambda p=p: stream.getPropertyValue(p))

DATE = ["DDMMYY", "MMDDYY", "YYMMDD", "YYYYMMDD", "YYYYDDD", "DAY", "MONTH", "DD-MM-YY", "DD-MM-YYYY", "MM-DD-YY",
        "MM-DD-YYYY", "DD-MON-YY", "DD-MON-YYYY", "YYYY-MM-DD", "DD.MM.YY", "DD.MM.YYYY", "MM.DD.YY", "MM.DD.YYYY",
        "DD.MON.YY", "DD.MON.YYYY", "DD/MM/YY", "DD/MM/YYYY", "MM/DD/YY", "MM/DD/YYYY", "DD/MON/YY", "DD/MON/YYYY",
        "MON YYYY", "q Q YYYY", "ww WK YYYY"]
TIME = ["HHMMSS", "HHMM", "MMSS", "HH:MM:SS", "HH:MM", "MM:SS", "(H)H:(M)M:(S)S", "(H)H:(M)M", "(M)M:(S)S",
        "HH.MM.SS", "HH.MM", "MM.SS", "(H)H.(M)M.(S)S", "(H)H.(M)M", "(M)M.(S)S"]
for v in DATE:
    prop("date_format", v)
for v in TIME:
    prop("time_format", v)
prop("date_baseline", 1950)
prop("date_baseline", 0)
prop("date_baseline", 2100)
prop("date_2digit_baseline", 1950)
prop("date_2digit_baseline", 1900)
prop("date_2digit_baseline", 2099)
prop("time_rollover", True)
prop("import_datetime_as_string", True)
prop("decimal_places", 7)
prop("decimal_places", 30)
prop("decimal_places", 100)
prop("decimal_places", -1)
for v in ["Comma", "Period", "Default"]:
    prop("decimal_symbol", v)
prop("angles_in_radians", True)
prop("use_max_set_size", True)
prop("max_set_size", 777)
prop("max_set_size", 0)
for v in ["FirstHit", "Voting"]:
    prop("ruleset_evaluation", v)
prop("refresh_source_nodes", True)
prop("annotation", "stream note")
prop("script", "print 1")
for v in ["SystemDefault", "UTF-8", "StreamDefault"]:
    prop("encoding", v)
for v in ["Script", "Normal"]:
    prop("execute_method", v)
for p in ["stream_rewriting", "stream_rewriting_maximise_sql", "stream_rewriting_optimise_clem_execution",
          "stream_rewriting_optimise_syntax_execution", "enable_parallelism", "sql_generation", "database_caching",
          "sql_logging", "sql_generation_logging", "sql_log_native", "sql_log_prettyprint", "record_count_suppress_output",
          "use_stream_auto_create_node_settings", "create_model_applier_for_new_models", "create_source_node_from_builders",
          "has_coordinate_system"]:
    prop(p, False)
    prop(p, True)
prop("record_count_feedback_interval", 333)
for v in ["createDisabled", "doNotCreate", "createEnabled"]:
    prop("create_model_applier_update_links", v)
for v in ["createDisabled", "doNotCreate", "createEnabled"]:
    prop("create_source_node_update_links", v)
prop("coordinate_system", "ETRS_1989_EPSG_Arctic_zone_5-47")
for v in ["Scoring", "ModelRefresh", "None"]:
    prop("deployment_area", v)
prop("scoring_terminal_node_id", "id123")
prop("scoring_node_id", "id456")
prop("model_build_node_id", "id789")


finish()
