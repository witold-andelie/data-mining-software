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


node = setup("logreg", needs_input=True)
instantiate_source()
prop("custom_fields", True)
prop("target", "Drug")
prop("inputs", ["BP", "Cholesterol", "Age"])
prop("partition", "Test")
prop("use_model_name", True)
prop("model_name", "Log_x")
prop("use_partitioned_data", True)
for v in ["Binomial", "Multinomial"]:
    prop("logistic_procedure", v)
prop("include_constant", False)
for v in ["Expert", "Simple"]:
    prop("mode", v)
for v in ["Stepwise", "Forwards", "Backwards", "BackwardsStepwise", "Enter"]:
    prop("method", v)
for v in ["Forwards", "Backwards", "BackwardsStepwise", "Enter"]:
    prop("binomial_method", v)
for v in ["FullFactorial", "Custom", "MainEffects"]:
    prop("model_type", v)
prop("custom_terms", [["BP", "Sex"], ["Age"], ["Na", "K"]])
prop("multinomial_base_category", "BP")
prop("binomial_categorical_input", "Sex")
keyed("binomial_input_contrast", "Sex", "Simple")
for v in ["Indicator", "Difference", "Helmert", "Repeated", "Polynomial", "Deviation"]:
    keyed("binomial_input_contrast", "Sex", v)
for v in ["First", "Last"]:
    keyed("binomial_input_category", "Sex", v)
for v in ["Pearson", "Deviance", "UserDefined", "None"]:
    prop("scale", v)
prop("scale_value", 3.0)
prop("all_probabilities", True)
for v in ["1.0E-5", "1.0E-6", "1.0E-7", "1.0E-8", "1.0E-9", "1.0E-10"]:
    prop("tolerance", v)
prop("min_terms", 7)
prop("use_max_terms", True)
prop("max_terms", 10)
for v in ["LR", "Score"]:
    prop("entry_criterion", v)
for v in ["Wald", "LR"]:
    prop("removal_criterion", v)
prop("probability_entry", 0.03)
prop("probability_removal", 0.05)
prop("binomial_probability_entry", 0.04)
prop("binomial_probability_removal", 0.2)
for v in ["HierarchyAll", "Containment", "None", "HierarchyDiscrete"]:
    prop("requirements", v)
prop("max_iterations", 50)
prop("max_steps", 3)
for v in ["1.0E-5", "1.0E-6", "1.0E-7", "1.0E-8", "1.0E-4"]:
    prop("p_converge", v)
for v in ["0", "1.0E-1", "1.0E-2", "1.0E-3", "1.0E-4", "1.0E-5"]:
    prop("l_converge", v)
prop("delta", 0.03)
for p in ["iteration_history", "summary", "likelihood_ratio", "asymptotic_correlation", "goodness_fit", "parameters", "confidence_interval", "asymptotic_covariance", "classification_table", "stepwise_summary", "info_criteria", "monotonicity_measures"]:
    prop(p, True)
prop("history_steps", 3)
prop("confidence_interval", 90)
for v in ["at_last_step", "at_each_step"]:
    prop("binomial_output_display", v)
for p in ["binomial_goodness_of_fit", "binomial_parameters", "binomial_iteration_history", "binomial_classification_plots", "binomial_ci_enable", "binomial_residual_enable"]:
    prop(p, True)
prop("binomial_ci", 85)
for v in ["all", "outliers"]:
    prop("binomial_residual", v)
prop("binomial_outlier_threshold", 3.5)
prop("binomial_classification_cutoff", 0.6)
for v in ["Wald", "Conditional", "LR"]:
    prop("binomial_removal_criterion", v)
prop("calculate_variable_importance", True)
prop("calculate_raw_propensities", True)


finish()
