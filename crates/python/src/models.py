# Data Mining Python model nodes: training and scoring with scikit-learn. Run by the bridge
# runner with `modelerData` (a pandas DataFrame) and `dmsParameters`:
#   kind         random_forest | xgboost_tree | xgboost_linear | gaussian_mixture | one_class_svm
#                | hdbscan | kde | extension_model
# smote(), tsne() and kde_simulation() transform data instead (SMOTE, t-SNE, KDE Simulation).
#   target       target field (supervised kinds)
#   target_kind  "categorical" or "continuous"
#   inputs       input fields
#   name         model name (score field names of unsupervised kinds)
#   options      the node's settings
# Training writes model.pkl (the fitted model and how to rebuild its design matrix) and
# summary.json. Scoring reads model.pkl and appends score fields to modelerData.
import json
import pickle

import numpy as np
import pandas as pd

FORMAT = 1


def _design(frame, spec):
    """The numeric matrix for `spec`: numeric fields (missing values replaced by the training
    median) and one indicator column per training category of every other field."""
    columns = []
    for item in spec:
        column = frame[item["name"]] if item["name"] in frame.columns else None
        if column is None:
            raise ValueError(f"input field {item['name']!r} is missing")
        if item["kind"] == "num":
            values = pd.to_numeric(column, errors="coerce").astype("float64")
            columns.append(values.fillna(item["median"]).to_numpy(dtype="float64"))
        else:
            text = column.astype("string")
            for category in item["categories"]:
                columns.append((text == category).fillna(False).to_numpy(dtype="float64"))
    if not columns:
        return np.empty((len(frame), 0))
    return np.column_stack(columns)


def _spec(frame, inputs):
    spec = []
    for name in inputs:
        column = frame[name]
        numeric = pd.api.types.is_numeric_dtype(column) and not pd.api.types.is_bool_dtype(column)
        if numeric:
            median = pd.to_numeric(column, errors="coerce").astype("float64").median()
            spec.append({"name": name, "kind": "num", "median": 0.0 if pd.isna(median) else float(median)})
        else:
            categories = sorted(column.dropna().astype(str).unique().tolist())
            if len(categories) > 1000:
                raise ValueError(f"input field {name!r} has more than 1000 categories")
            spec.append({"name": name, "kind": "cat", "categories": categories})
    return spec


def _columns(spec):
    names = []
    for item in spec:
        if item["kind"] == "num":
            names.append(item["name"])
        else:
            names.extend(f"{item['name']}={c}" for c in item["categories"])
    return names


def _free(frame, prefix, base):
    """Modeler's naming: `$R-target`, or `$R1-target` when that exists, and so on."""
    if f"{prefix}-{base}" not in frame.columns:
        return ""
    index = 1
    while f"{prefix}{index}-{base}" in frame.columns:
        index += 1
    return str(index)


SUPERVISED = ("random_forest", "xgboost_tree", "xgboost_linear")
# Score field prefixes: prediction, probability.
PREFIX = {"random_forest": ("$R", "$RP"), "xgboost_tree": ("$XT", "$XTP"), "xgboost_linear": ("$XL", "$XLP")}


def _xgboost():
    try:
        import xgboost
    except ModuleNotFoundError:
        raise
    except Exception as exc:  # the library is installed but its native part does not load
        text = str(exc)
        hint = ""
        if "libomp" in text:
            hint = " On macOS install the OpenMP runtime with: brew install libomp"
        raise RuntimeError("xgboost is installed but cannot load." + hint) from None
    return xgboost


def _cluster_name(label):
    return "noise" if label < 0 else f"cluster-{label + 1}"


def _user_model():
    """Extension Model: runs the user's build script, which leaves its model in `modelerModel`;
    the score script is saved with it."""
    p = dmsParameters  # noqa: F821
    o = p.get("options", {})
    build, score = o.get("python_build_syntax", ""), o.get("python_score_syntax", "")
    if not build.strip():
        raise ValueError("the build script is empty")
    if not score.strip():
        raise ValueError("the score script is empty")
    space = {
        "__name__": "__main__",
        "modelerData": modelerData,  # noqa: F821
        "modelerDataModel": modelerDataModel,  # noqa: F821
        "modelerModel": None,
    }
    exec(compile(build, "<script>", "exec"), space)
    if space.get("modelerModel") is None:
        raise ValueError("the build script must leave its model in modelerModel")
    saved = {
        "format": FORMAT,
        "kind": "extension_model",
        "name": p.get("name") or "model",
        "model": space["modelerModel"],
        "score": score,
    }
    with open("model.pkl", "wb") as handle:
        pickle.dump(saved, handle, protocol=4)
    with open("summary.json", "w", encoding="utf-8") as handle:
        json.dump({"records": int(len(modelerData))}, handle)  # noqa: F821


def train():
    p = dmsParameters  # noqa: F821 (set by the runner)
    if p["kind"] == "extension_model":
        return _user_model()
    o = p.get("options", {})
    frame = modelerData  # noqa: F821
    inputs = list(p["inputs"])
    if not inputs:
        raise ValueError("no input fields: set them in the node or with a Type node")
    kind = p["kind"]
    target = p.get("target")
    if kind in SUPERVISED:
        frame = frame[frame[target].notna()]
    if len(frame) == 0:
        raise ValueError("no records to train on")
    spec = _spec(frame, inputs)
    x = _design(frame, spec)
    summary = {"records": int(len(frame)), "inputs": inputs, "columns": len(_columns(spec))}
    seed = int(o.get("random_seed", 1))
    if kind == "random_forest":
        from sklearn.ensemble import RandomForestClassifier, RandomForestRegressor

        classify = p["target_kind"] == "categorical"
        settings = dict(
            n_estimators=int(o.get("n_estimators", 10)),
            max_depth=int(o["max_depth"]) if int(o.get("max_depth", 0)) > 0 else None,
            min_samples_leaf=int(o.get("min_samples_leaf", 1)),
            bootstrap=bool(o.get("bootstrap", True)),
            random_state=seed,
        )
        if classify:
            features = o.get("max_features", "sqrt")
            settings["max_features"] = None if features == "all" else features
            model = RandomForestClassifier(**settings)
            y = frame[target].astype(str).to_numpy()
        else:
            features = o.get("max_features", "all")
            settings["max_features"] = None if features == "all" else features
            model = RandomForestRegressor(**settings)
            y = pd.to_numeric(frame[target], errors="coerce").astype("float64").to_numpy()
        model.fit(x, y)
        importance = sorted(
            zip(_columns(spec), model.feature_importances_.tolist()), key=lambda t: -t[1]
        )
        summary["importance"] = importance[:50]
        if classify:
            summary["classes"] = model.classes_.tolist()
            summary["training_accuracy"] = float(model.score(x, y))
        else:
            summary["training_r2"] = float(model.score(x, y))
        state = {"classify": classify}
    elif kind in ("xgboost_tree", "xgboost_linear"):
        xgboost = _xgboost()
        classify = p["target_kind"] == "categorical"
        settings = dict(
            n_estimators=int(o.get("num_boost_round", 10)),
            learning_rate=float(o.get("eta", 0.3)),
            reg_lambda=float(o.get("lambda", 1.0 if kind == "xgboost_tree" else 0.0)),
            reg_alpha=float(o.get("alpha", 0.0)),
            random_state=seed,
            n_jobs=1,
        )
        if kind == "xgboost_tree":
            settings.update(
                booster="gbtree",
                max_depth=int(o.get("max_depth", 6)),
                min_child_weight=float(o.get("min_child_weight", 1.0)),
                subsample=float(o.get("subsample", 1.0)),
                colsample_bytree=float(o.get("colsample_bytree", 1.0)),
                gamma=float(o.get("gamma", 0.0)),
            )
        else:
            settings.update(booster="gblinear")
        state = {"classify": classify}
        if classify:
            labels = frame[target].astype(str)
            classes = sorted(labels.unique().tolist())
            if len(classes) < 2:
                raise ValueError(f"the target {target!r} has only one value")
            y = labels.map({c: i for i, c in enumerate(classes)}).to_numpy()
            model = xgboost.XGBClassifier(**settings)
            model.fit(x, y)
            summary["classes"] = classes
            summary["training_accuracy"] = float((model.predict(x) == y).mean())
            state["classes"] = classes
        else:
            y = pd.to_numeric(frame[target], errors="coerce").astype("float64").to_numpy()
            model = xgboost.XGBRegressor(**settings)
            model.fit(x, y)
            summary["training_r2"] = float(model.score(x, y))
        try:
            weights = np.abs(np.asarray(model.feature_importances_, dtype="float64")).ravel()
            importance = sorted(zip(_columns(spec), weights.tolist()), key=lambda t: -t[1])
            summary["importance"] = importance[:50]
        except Exception:
            pass
    elif kind == "hdbscan":
        import hdbscan

        model = hdbscan.HDBSCAN(
            min_cluster_size=int(o.get("min_cluster_size", 5)),
            min_samples=int(o["min_samples"]) if int(o.get("min_samples", 0)) > 0 else None,
            cluster_selection_method=o.get("cluster_selection_method", "eom"),
            prediction_data=True,
        )
        model.fit(x)
        labels = model.labels_
        summary["clusters"] = [
            [_cluster_name(k), int((labels == k).sum())] for k in sorted(set(labels.tolist()))
        ]
        state = {}
    elif kind == "kde":
        from sklearn.neighbors import KernelDensity

        model = KernelDensity(
            bandwidth=float(o.get("bandwidth", 1.0)), kernel=o.get("kernel", "gaussian")
        )
        model.fit(x)
        summary["mean_log_density"] = float(model.score_samples(x).mean())
        state = {}
    elif kind == "gaussian_mixture":
        from sklearn.mixture import GaussianMixture

        model = GaussianMixture(
            n_components=int(o.get("n_components", 2)),
            covariance_type=o.get("covariance_type", "full"),
            max_iter=int(o.get("max_iter", 100)),
            random_state=seed,
        )
        model.fit(x)
        labels = model.predict(x)
        summary["clusters"] = [
            [f"cluster-{k + 1}", int((labels == k).sum())] for k in range(model.n_components)
        ]
        summary["converged"] = bool(model.converged_)
        summary["bic"] = float(model.bic(x))
        state = {}
    elif kind == "one_class_svm":
        from sklearn.svm import OneClassSVM

        model = OneClassSVM(
            kernel=o.get("kernel", "rbf"),
            nu=float(o.get("nu", 0.5)),
            gamma=o.get("gamma", "scale"),
        )
        model.fit(x)
        outliers = int((model.predict(x) == -1).sum())
        summary["training_outliers"] = outliers
        state = {}
    else:
        raise ValueError(f"unknown model kind {kind!r}")
    saved = {
        "format": FORMAT,
        "kind": kind,
        "target": target,
        "name": p.get("name") or kind,
        "spec": spec,
        "model": model,
        **state,
    }
    with open("model.pkl", "wb") as handle:
        pickle.dump(saved, handle, protocol=4)
    with open("summary.json", "w", encoding="utf-8") as handle:
        json.dump(summary, handle)


def score():
    global modelerData
    with open("model.pkl", "rb") as handle:
        saved = pickle.load(handle)
    if saved.get("format") != FORMAT:
        raise ValueError("the saved model was written by another version; run its build node again")
    frame = modelerData  # noqa: F821
    kind = saved["kind"]
    if kind == "extension_model":
        space = {
            "__name__": "__main__",
            "modelerData": frame.copy(),
            "modelerDataModel": modelerDataModel,  # noqa: F821
            "modelerModel": saved["model"],
        }
        exec(compile(saved["score"], "<script>", "exec"), space)
        result = space.get("modelerData")
        if not isinstance(result, pd.DataFrame):
            raise TypeError("the score script must leave a pandas DataFrame in modelerData")
        added = [c for c in result.columns if c not in frame.columns]
        if not added:
            raise ValueError("the score script added no fields to modelerData")
        if len(result) != len(frame):
            raise ValueError(
                f"the score script returned {len(result)} records for {len(frame)}"
            )
        modelerData = result[added].reset_index(drop=True)
        return
    x = _design(frame, saved["spec"])
    model = saved["model"]
    out = {}
    if kind in ("xgboost_tree", "xgboost_linear"):
        _xgboost()
        target = saved["target"]
        prediction, probability = PREFIX[kind]
        index = _free(frame, prediction, target)
        if saved["classify"]:
            classes = saved["classes"]
            if len(frame):
                probabilities = model.predict_proba(x)
                best = probabilities.argmax(axis=1)
                labels = [classes[i] for i in best]
                confidence = probabilities.max(axis=1)
            else:
                labels, confidence = [], []
            out[f"{prediction}{index}-{target}"] = pd.array(labels, dtype="string")
            out[f"{probability}{index}-{target}"] = pd.array(confidence, dtype="Float64")
        else:
            predicted = model.predict(x) if len(frame) else []
            out[f"{prediction}{index}-{target}"] = pd.array(predicted, dtype="Float64")
    elif kind == "random_forest":
        target = saved["target"]
        index = _free(frame, "$R", target)
        if len(frame) == 0:
            predicted = []
        else:
            predicted = model.predict(x)
        if saved["classify"]:
            out[f"$R{index}-{target}"] = pd.array([str(v) for v in predicted], dtype="string")
            confidence = model.predict_proba(x).max(axis=1) if len(frame) else []
            out[f"$RP{index}-{target}"] = pd.array(confidence, dtype="Float64")
        else:
            out[f"$R{index}-{target}"] = pd.array(predicted, dtype="Float64")
    elif kind == "hdbscan":
        import hdbscan

        name = saved["name"]
        index = _free(frame, "$HB", name)
        if len(frame):
            labels, strengths = hdbscan.approximate_predict(model, x)
        else:
            labels, strengths = [], []
        out[f"$HB{index}-{name}"] = pd.array([_cluster_name(int(k)) for k in labels], dtype="string")
        out[f"$HBP{index}-{name}"] = pd.array(strengths, dtype="Float64")
    elif kind == "kde":
        name = saved["name"]
        index = _free(frame, "$KDE", name)
        density = model.score_samples(x) if len(frame) else []
        out[f"$KDE{index}-{name}"] = pd.array(density, dtype="Float64")
    elif kind == "gaussian_mixture":
        name = saved["name"]
        index = _free(frame, "$GMM", name)
        if len(frame):
            labels = model.predict(x)
            probability = model.predict_proba(x).max(axis=1)
        else:
            labels, probability = [], []
        out[f"$GMM{index}-{name}"] = pd.array([f"cluster-{k + 1}" for k in labels], dtype="string")
        out[f"$GMMP{index}-{name}"] = pd.array(probability, dtype="Float64")
    elif kind == "one_class_svm":
        name = saved["name"]
        index = _free(frame, "$O", name)
        if len(frame):
            flags = model.predict(x)
            scores = model.decision_function(x)
        else:
            flags, scores = [], []
        out[f"$O{index}-{name}"] = pd.array(["T" if f == -1 else "F" for f in flags], dtype="string")
        out[f"$OP{index}-{name}"] = pd.array(scores, dtype="Float64")
    else:
        raise ValueError(f"unknown model kind {kind!r}")
    # Only the score fields go back; the caller appends them to its own records.
    modelerData = pd.DataFrame(out)


def smote():
    """Appends synthetic records of the smaller target classes (imbalanced-learn SMOTE, or
    SMOTENC when some inputs are categorical). Original records stay first and unchanged;
    synthetic ones have only the inputs and the target."""
    global modelerData
    from imblearn.over_sampling import SMOTE, SMOTENC

    p = dmsParameters  # noqa: F821
    o = p.get("options", {})
    frame = modelerData  # noqa: F821
    target, inputs = p["target"], list(p["inputs"])
    if not inputs:
        raise ValueError("no input fields: set them in the node or with a Type node")
    complete = frame.dropna(subset=inputs + [target])
    if len(complete) == 0:
        raise ValueError("no records with every input and the target present")
    x = pd.DataFrame(index=range(len(complete)))
    categorical = []
    for i, name in enumerate(inputs):
        column = complete[name].reset_index(drop=True)
        if pd.api.types.is_numeric_dtype(column) and not pd.api.types.is_bool_dtype(column):
            x[name] = column.astype("float64").to_numpy()
        else:
            x[name] = column.astype(str).to_numpy()
            categorical.append(i)
    y = complete[target].astype(str).to_numpy()
    seed = int(o.get("random_seed", 1))
    k = int(o.get("k_neighbors", 5))
    strategy = o.get("sampling_strategy", "auto")
    if categorical and len(categorical) == len(inputs):
        raise ValueError("SMOTE needs at least one numeric input")
    if categorical:
        sampler = SMOTENC(categorical_features=categorical, k_neighbors=k, sampling_strategy=strategy, random_state=seed)
    else:
        sampler = SMOTE(k_neighbors=k, sampling_strategy=strategy, random_state=seed)
    xr, yr = sampler.fit_resample(x, y)
    new = pd.DataFrame(xr).iloc[len(x):].reset_index(drop=True)
    labels = pd.Series(yr[len(x):])
    if pd.api.types.is_numeric_dtype(frame[target]) and not pd.api.types.is_bool_dtype(frame[target]):
        labels = pd.to_numeric(labels)
    new[target] = labels.to_numpy()
    for name in inputs:
        if pd.api.types.is_numeric_dtype(frame[name]):
            new[name] = pd.to_numeric(new[name]).astype("Float64")
    modelerData = pd.concat([frame.reset_index(drop=True), new], ignore_index=True)


def tsne():
    """A 2-D t-SNE embedding of the inputs, with an optional field to colour by. Records beyond
    `max_records` are sampled (with the seed) first."""
    global modelerData
    from sklearn.manifold import TSNE

    p = dmsParameters  # noqa: F821
    o = p.get("options", {})
    frame = modelerData  # noqa: F821
    inputs = list(p["inputs"])
    if not inputs:
        raise ValueError("no input fields: set them in the node or with a Type node")
    seed = int(o.get("random_seed", 1))
    limit = int(o.get("max_records", 5000))
    if len(frame) > limit:
        frame = frame.sample(n=limit, random_state=seed)
    if len(frame) < 4:
        raise ValueError("t-SNE needs at least 4 records")
    x = _design(frame, _spec(frame, inputs))
    spread = x.std(axis=0)
    spread[spread == 0] = 1.0
    x = (x - x.mean(axis=0)) / spread
    perplexity = min(float(o.get("perplexity", 30.0)), (len(frame) - 1) / 3.0)
    embedding = TSNE(
        n_components=2, perplexity=perplexity, init="pca", random_state=seed
    ).fit_transform(x)
    out = pd.DataFrame({"tSNE-1": embedding[:, 0], "tSNE-2": embedding[:, 1]})
    color = p.get("color")
    if color:
        out[color] = frame[color].astype("string").fillna("$null$").to_numpy()
    modelerData = out


def kde_simulation():
    """Simulated records: fits a kernel density to the numeric inputs and draws `records` new
    records from it (scikit-learn KernelDensity.sample; Gaussian or top-hat kernel)."""
    global modelerData
    from sklearn.neighbors import KernelDensity

    p = dmsParameters  # noqa: F821
    o = p.get("options", {})
    frame = modelerData  # noqa: F821
    inputs = list(p["inputs"])
    if not inputs:
        raise ValueError("no input fields: set them in the node or with a Type node")
    for name in inputs:
        column = frame[name]
        if not pd.api.types.is_numeric_dtype(column) or pd.api.types.is_bool_dtype(column):
            raise ValueError(f"input field {name!r} is not numeric; KDE simulation needs numbers")
    data = frame[inputs].dropna().astype("float64").to_numpy()
    if len(data) == 0:
        raise ValueError("no records with every input present")
    model = KernelDensity(
        bandwidth=float(o.get("bandwidth", 1.0)), kernel=o.get("kernel", "gaussian")
    ).fit(data)
    drawn = model.sample(int(o.get("records", 1000)), random_state=int(o.get("random_seed", 1)))
    modelerData = pd.DataFrame(drawn, columns=inputs).astype("Float64")
