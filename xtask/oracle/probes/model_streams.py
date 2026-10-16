# Probe: build deterministic model training corpus streams and copy input datasets into
# corpus/models/. Each stream has:
#   source -> Type (explicit roles: one target, inputs, unused = None) -> build_node
#   and Type -> model_nugget -> Table
# with decimal_places = 15, explicit model names, no Partition/Sample/Balance nodes,
# and fixed random seeds on every node that accepts one.
# Run with `cargo xtask probe model_streams` (IBM SPSS Modeler, Jython 2.7, public API only).
import json
import java.io.File
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
MODELS_DIR = "D:/data-mining-software/corpus/models"
DEMOS_DIR = "D:/Program Files/spss/Demos"
FALLBACK_DEMO_DIR = "D:/data-mining-software/corpus/ibm-demo-data"
TEAM_DIR = "D:/data-mining-software/corpus/team"
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)
SEED = 12345

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"saved": [], "errors": [], "cases": []}

DRUG_SCHEMA = [
    ("Age", "Range"),
    ("Sex", "Flag"),
    ("BP", "Set"),
    ("Cholesterol", "Flag"),
    ("Na", "Range"),
    ("K", "Range"),
    ("Drug", "Set"),
]

CREDIT_SCHEMA = [
    ("Credit_rating", "Set"),
    ("Age", "Range"),
    ("Income", "OrderedSet"),
    ("Credit_cards", "Set"),
    ("Education", "Set"),
    ("Car_loans", "Set"),
]

HEART_SCHEMA = [
    ("age", "Range"),
    ("sex ", "Flag"),
    ("chest pain type", "Set"),
    ("resting blood pressure", "Range"),
    ("serum cholestoral", "Range"),
    ("fasting blood sugar", "Flag"),
    ("resting electrocardiographic results", "Set"),
    ("max heart rate", "Range"),
    ("exercise induced angina", "Flag"),
    ("oldpeak", "Range"),
    ("ST segment", "Set"),
    ("major vessels", "Range"),
    ("thal", "Set"),
    ("heart disease", "Flag"),
]


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


def copy_data_file(file_name, candidates):
    src_path = None
    for candidate in candidates:
        f = java.io.File(candidate)
        if f.exists() and f.isFile():
            src_path = candidate
            break
    if src_path is None:
        manifest["errors"].append({
            "case": file_name,
            "stage": "copy_data",
            "message": "source file not found in candidates",
        })
        return
    dst_path = "%s/%s" % (MODELS_DIR, file_name)
    src_handle = open(src_path, "rb")
    try:
        data = src_handle.read()
    finally:
        src_handle.close()
    dst_handle = open(dst_path, "wb")
    try:
        dst_handle.write(data)
    finally:
        dst_handle.close()
    manifest["saved"].append(file_name)


def instantiate_node(stream, node):
    table = stream.createAt("table", "TempTable", 760, 360)
    stream.link(node, table)
    try:
        table.run([])
    finally:
        stream.delete(table)


def new_stream(case_name, data_file, schema, target, inputs):
    stream = taskrunner.createStream(case_name, False, False)
    stream.setPropertyValue("decimal_places", 15)
    stream.setPropertyValue("create_model_applier_for_new_models", True)
    stream.setPropertyValue("create_model_applier_update_links", "createEnabled")

    full_path = "%s/%s" % (MODELS_DIR, data_file)
    if data_file.lower().endswith(".sav"):
        src = stream.createAt("statisticsimport", data_file, 100, 100)
        src.setPropertyValue("full_filename", full_path)
    else:
        src = stream.createAt("variablefile", data_file, 100, 100)
        src.setPropertyValue("full_filename", full_path)
        src.setPropertyValue("read_field_names", True)

    type_node = stream.createAt("type", "Type", 260, 100)
    stream.link(src, type_node)
    instantiate_node(stream, type_node)

    input_set = dict([(name, True) for name in inputs])
    for field_name, measure in schema:
        type_node.setKeyedPropertyValue("type", field_name, measure)
        if target is not None and field_name == target:
            type_node.setKeyedPropertyValue("direction", field_name, "Target")
        elif field_name in input_set:
            type_node.setKeyedPropertyValue("direction", field_name, "Input")
        else:
            type_node.setKeyedPropertyValue("direction", field_name, "None")

    instantiate_node(stream, type_node)
    return stream, type_node


def set_model_name_and_fields(build_node, node_type, case_name, target, inputs):
    try:
        build_node.setPropertyValue("use_model_name", True)
        build_node.setPropertyValue("model_name", case_name)
    except ERRORS:
        pass
    if node_type == "linear":
        try:
            build_node.setPropertyValue("use_custom_model_name", True)
            build_node.setPropertyValue("custom_model_name", case_name)
        except ERRORS:
            pass
    try:
        build_node.setPropertyValue("custom_fields", True)
        if target is not None:
            if node_type == "neuralnetwork":
                build_node.setPropertyValue("targets", [target])
            else:
                build_node.setPropertyValue("target", target)
        build_node.setPropertyValue("inputs", list(inputs))
    except ERRORS:
        pass


def find_applier(stream):
    it = stream.iterator()
    while it.hasNext():
        n = it.next()
        t = unicode(n.getTypeName())
        if t.startswith("apply"):
            return n
    return None


def build_case(case_name, data_file, schema, target, inputs, node_type, configure):
    stream = None
    try:
        stream, type_node = new_stream(case_name, data_file, schema, target, inputs)
        build_node = stream.createAt(node_type, case_name, 460, 100)
        stream.link(type_node, build_node)
        set_model_name_and_fields(build_node, node_type, case_name, target, inputs)
        configure(stream, build_node, type_node)

        results = []
        build_node.run(results)

        applier = find_applier(stream)
        auto_placed = applier is not None
        if applier is None:
            if not results:
                raise RuntimeError("build_node produced no ModelOutput and no applier node")
            applier = stream.createModelApplierAt(results[0], case_name, 460, 240)
            stream.link(type_node, applier)
        else:
            applier.setXYPosition(460, 240)

        table_node = stream.createAt("table", "Table", 640, 240)
        stream.link(applier, table_node)
        table_node.run([])

        file_name = "%s.str" % case_name
        path = "%s/%s" % (MODELS_DIR, file_name)
        taskrunner.saveStreamToFile(stream, path)
        manifest["saved"].append(file_name)
        manifest["cases"].append({
            "case": case_name,
            "file": file_name,
            "data": data_file,
            "target": target,
            "inputs": list(inputs),
            "build_type": unicode(build_node.getTypeName()),
            "applier_type": unicode(applier.getTypeName()),
            "auto_placed_applier": auto_placed,
        })
    except ERRORS, e:
        manifest["errors"].append({"case": case_name, "message": message(e)})
    finally:
        if stream is not None:
            try:
                stream.clear()
            except ERRORS:
                pass


ensure_clean_dir(MODELS_DIR)
copy_data_file("DRUG1n", [
    "%s/DRUG1n" % DEMOS_DIR,
    "%s/DRUG1n" % FALLBACK_DEMO_DIR,
])
copy_data_file("tree_credit.sav", [
    "%s/tree_credit.sav" % DEMOS_DIR,
    "%s/tree_credit.sav" % FALLBACK_DEMO_DIR,
])
copy_data_file("HEART.csv", [
    "%s/HEART.csv" % TEAM_DIR,
])

DRUG_INPUTS_ALL = ["Age", "Sex", "BP", "Cholesterol", "Na", "K"]
DRUG_INPUTS_FOR_NA = ["Age", "Sex", "BP", "Cholesterol", "K"]
DRUG_INPUTS_CONT = ["Age", "Na", "K"]
DRUG_INPUTS_FOR_NA_CONT = ["Age", "K"]
CREDIT_INPUTS_ALL = ["Age", "Income", "Credit_cards", "Education", "Car_loans"]
CREDIT_INPUTS_FOR_AGE = ["Credit_rating", "Income", "Credit_cards", "Education", "Car_loans"]
HEART_INPUTS_LOGREG = ["age", "resting blood pressure", "max heart rate"]


# ==============================================================================
# 1. C&R Tree (cart)
# ==============================================================================

def cfg_cart_default(stream, node, type_node):
    node.setPropertyValue("impurity_measure", "Gini")
    node.setPropertyValue("train_pct", 0)
    node.setPropertyValue("set_random_seed", True)
    node.setPropertyValue("seed", SEED)


def cfg_cart_depth3(stream, node, type_node):
    node.setPropertyValue("use_max_depth", "Custom")
    node.setPropertyValue("max_depth", 3)
    node.setPropertyValue("impurity_measure", "Gini")
    node.setPropertyValue("train_pct", 0)
    node.setPropertyValue("set_random_seed", True)
    node.setPropertyValue("seed", SEED)


def cfg_cart_unpruned(stream, node, type_node):
    node.setPropertyValue("prune_tree", False)
    node.setPropertyValue("impurity_measure", "Gini")
    node.setPropertyValue("train_pct", 0)
    node.setPropertyValue("set_random_seed", True)
    node.setPropertyValue("seed", SEED)


def cfg_cart_min_records(stream, node, type_node):
    node.setPropertyValue("prune_tree", False)
    node.setPropertyValue("use_percentage", False)
    node.setPropertyValue("min_parent_records_abs", 30)
    node.setPropertyValue("min_child_records_abs", 15)
    node.setPropertyValue("impurity_measure", "Gini")
    node.setPropertyValue("train_pct", 0)
    node.setPropertyValue("set_random_seed", True)
    node.setPropertyValue("seed", SEED)


def cfg_cart_regression(stream, node, type_node):
    node.setPropertyValue("prune_tree", False)
    node.setPropertyValue("use_max_depth", "Custom")
    node.setPropertyValue("max_depth", 4)
    node.setPropertyValue("train_pct", 0)
    node.setPropertyValue("set_random_seed", True)
    node.setPropertyValue("seed", SEED)


def cfg_cart_surrogates(stream, node, type_node):
    node.setPropertyValue("prune_tree", False)
    node.setPropertyValue("use_max_depth", "Custom")
    node.setPropertyValue("max_depth", 3)
    node.setPropertyValue("max_surrogates", 2)
    node.setPropertyValue("train_pct", 0)
    node.setPropertyValue("set_random_seed", True)
    node.setPropertyValue("seed", SEED)


def cfg_cart_no_surrogates(stream, node, type_node):
    node.setPropertyValue("prune_tree", False)
    node.setPropertyValue("max_surrogates", 0)
    node.setPropertyValue("train_pct", 0)
    node.setPropertyValue("set_random_seed", True)
    node.setPropertyValue("seed", SEED)


def cfg_cart_twoing(stream, node, type_node):
    node.setPropertyValue("prune_tree", False)
    node.setPropertyValue("impurity_measure", "Twoing")
    node.setPropertyValue("train_pct", 0)
    node.setPropertyValue("set_random_seed", True)
    node.setPropertyValue("seed", SEED)


build_case("cart_default", "DRUG1n", DRUG_SCHEMA, "Drug", DRUG_INPUTS_ALL, "cart", cfg_cart_default)
build_case("cart_depth3", "DRUG1n", DRUG_SCHEMA, "Drug", DRUG_INPUTS_ALL, "cart", cfg_cart_depth3)
build_case("cart_unpruned", "DRUG1n", DRUG_SCHEMA, "Drug", DRUG_INPUTS_ALL, "cart", cfg_cart_unpruned)
build_case("cart_min_records", "DRUG1n", DRUG_SCHEMA, "Drug", DRUG_INPUTS_ALL, "cart", cfg_cart_min_records)
build_case("cart_regression", "DRUG1n", DRUG_SCHEMA, "Na", DRUG_INPUTS_FOR_NA, "cart", cfg_cart_regression)
build_case("cart_surrogates", "tree_credit.sav", CREDIT_SCHEMA, "Credit_rating", CREDIT_INPUTS_ALL, "cart", cfg_cart_surrogates)
build_case("cart_no_surrogates", "DRUG1n", DRUG_SCHEMA, "Drug", DRUG_INPUTS_ALL, "cart", cfg_cart_no_surrogates)
build_case("cart_twoing", "DRUG1n", DRUG_SCHEMA, "Drug", DRUG_INPUTS_ALL, "cart", cfg_cart_twoing)


# ==============================================================================
# 2. CHAID (chaid)
# ==============================================================================

def cfg_chaid_default(stream, node, type_node):
    node.setPropertyValue("method", "Chaid")
    node.setPropertyValue("set_random_seed", True)
    node.setPropertyValue("seed", SEED)


def cfg_chaid_exhaustive(stream, node, type_node):
    node.setPropertyValue("method", "ExhaustiveChaid")
    node.setPropertyValue("set_random_seed", True)
    node.setPropertyValue("seed", SEED)


def cfg_chaid_depth2(stream, node, type_node):
    node.setPropertyValue("method", "Chaid")
    node.setPropertyValue("use_max_depth", "Custom")
    node.setPropertyValue("max_depth", 2)
    node.setPropertyValue("set_random_seed", True)
    node.setPropertyValue("seed", SEED)


def cfg_chaid_alpha(stream, node, type_node):
    node.setPropertyValue("method", "Chaid")
    node.setPropertyValue("split_alpha", 0.01)
    node.setPropertyValue("merge_alpha", 0.10)
    node.setPropertyValue("bonferroni_adjustment", False)
    node.setPropertyValue("set_random_seed", True)
    node.setPropertyValue("seed", SEED)


def cfg_chaid_regression(stream, node, type_node):
    node.setPropertyValue("method", "Chaid")
    node.setPropertyValue("use_max_depth", "Custom")
    node.setPropertyValue("max_depth", 3)
    node.setPropertyValue("set_random_seed", True)
    node.setPropertyValue("seed", SEED)


build_case("chaid_default", "DRUG1n", DRUG_SCHEMA, "Drug", DRUG_INPUTS_ALL, "chaid", cfg_chaid_default)
build_case("chaid_exhaustive", "tree_credit.sav", CREDIT_SCHEMA, "Credit_rating", CREDIT_INPUTS_ALL, "chaid", cfg_chaid_exhaustive)
build_case("chaid_depth2", "DRUG1n", DRUG_SCHEMA, "Drug", DRUG_INPUTS_ALL, "chaid", cfg_chaid_depth2)
build_case("chaid_alpha", "tree_credit.sav", CREDIT_SCHEMA, "Credit_rating", CREDIT_INPUTS_ALL, "chaid", cfg_chaid_alpha)
build_case("chaid_regression", "tree_credit.sav", CREDIT_SCHEMA, "Age", CREDIT_INPUTS_FOR_AGE, "chaid", cfg_chaid_regression)


# ==============================================================================
# 3. K-Means (kmeans)
# ==============================================================================

def cfg_kmeans_3clusters(stream, node, type_node):
    node.setPropertyValue("num_clusters", 3)
    node.setPropertyValue("gen_distance", True)


def cfg_kmeans_5clusters(stream, node, type_node):
    node.setPropertyValue("num_clusters", 5)
    node.setPropertyValue("gen_distance", True)


def cfg_kmeans_categorical(stream, node, type_node):
    node.setPropertyValue("num_clusters", 4)
    node.setPropertyValue("gen_distance", True)


build_case("kmeans_3clusters", "DRUG1n", DRUG_SCHEMA, None, DRUG_INPUTS_CONT, "kmeans", cfg_kmeans_3clusters)
build_case("kmeans_5clusters", "DRUG1n", DRUG_SCHEMA, None, DRUG_INPUTS_CONT, "kmeans", cfg_kmeans_5clusters)
build_case("kmeans_categorical", "DRUG1n", DRUG_SCHEMA, None, DRUG_INPUTS_ALL, "kmeans", cfg_kmeans_categorical)


# ==============================================================================
# 4. Logistic Regression (logreg)
# ==============================================================================

def cfg_logreg_binomial(stream, node, type_node):
    node.setPropertyValue("logistic_procedure", "Binomial")
    node.setPropertyValue("binomial_method", "Enter")
    node.setPropertyValue("include_constant", True)


def cfg_logreg_multinomial(stream, node, type_node):
    node.setPropertyValue("logistic_procedure", "Multinomial")
    node.setPropertyValue("method", "Enter")
    node.setPropertyValue("include_constant", True)
    node.setPropertyValue("all_probabilities", True)


build_case("logreg_binomial", "HEART.csv", HEART_SCHEMA, "heart disease", HEART_INPUTS_LOGREG, "logreg", cfg_logreg_binomial)
build_case("logreg_multinomial", "DRUG1n", DRUG_SCHEMA, "Drug", DRUG_INPUTS_ALL, "logreg", cfg_logreg_multinomial)


# ==============================================================================
# 5. Linear (linear) and Regression (regression)
# ==============================================================================

def cfg_linear_enter(stream, node, type_node):
    node.setPropertyValue("model_selection", "None")
    node.setPropertyValue("use_auto_data_preparation", False)
    node.setPropertyValue("use_random_seed", True)
    node.setPropertyValue("random_seed", SEED)


def cfg_regression_enter(stream, node, type_node):
    node.setPropertyValue("method", "Enter")
    node.setPropertyValue("include_constant", True)


build_case("linear_enter", "DRUG1n", DRUG_SCHEMA, "Na", DRUG_INPUTS_FOR_NA_CONT, "linear", cfg_linear_enter)
build_case("regression_enter", "DRUG1n", DRUG_SCHEMA, "Na", DRUG_INPUTS_FOR_NA_CONT, "regression", cfg_regression_enter)


# ==============================================================================
# 6. C5.0 (c50) and Neural Net (neuralnetwork)
# ==============================================================================

def cfg_c50_default(stream, node, type_node):
    node.setPropertyValue("output_type", "DecisionTree")


def cfg_neuralnet_default(stream, node, type_node):
    node.setPropertyValue("use_random_seed", True)
    node.setPropertyValue("random_seed", SEED)


build_case("c50_default", "DRUG1n", DRUG_SCHEMA, "Drug", DRUG_INPUTS_ALL, "c50", cfg_c50_default)
build_case("neuralnet_default", "DRUG1n", DRUG_SCHEMA, "Drug", DRUG_INPUTS_ALL, "neuralnetwork", cfg_neuralnet_default)


# ==============================================================================
# 7. TwoStep (twostep)
# ==============================================================================

def cfg_twostep_default(stream, node, type_node):
    node.setPropertyValue("standardize", True)
    node.setPropertyValue("cluster_num_auto", True)
    node.setPropertyValue("min_num_clusters", 2)
    node.setPropertyValue("max_num_clusters", 15)
    node.setPropertyValue("distance_measure", "Loglikelihood")
    node.setPropertyValue("clustering_criterion", "BIC")


build_case("twostep_default", "DRUG1n", DRUG_SCHEMA, None, DRUG_INPUTS_ALL, "twostep", cfg_twostep_default)


handle = open(OUT_DIR + "/manifest.json", "w")
try:
    json.dump(manifest, handle, indent=2, sort_keys=True)
finally:
    handle.close()
