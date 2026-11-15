# Data Mining Python model nodes: training and scoring with scikit-learn. Run by the bridge
# runner with `modelerData` (a pandas DataFrame) and `dmsParameters`:
#   kind         random_forest | gaussian_mixture | one_class_svm
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


def train():
    p = dmsParameters  # noqa: F821 (set by the runner)
    o = p.get("options", {})
    frame = modelerData  # noqa: F821
    inputs = list(p["inputs"])
    if not inputs:
        raise ValueError("no input fields: set them in the node or with a Type node")
    kind = p["kind"]
    target = p.get("target")
    if kind == "random_forest":
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
    x = _design(frame, saved["spec"])
    model = saved["model"]
    kind = saved["kind"]
    out = {}
    if kind == "random_forest":
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
