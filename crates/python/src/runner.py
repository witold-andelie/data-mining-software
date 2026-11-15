# Data Mining Python bridge runner. Written to a temporary folder and run as
#   python -I -u runner.py request.json
# The request names the user's script, the input Arrow files and where to write results.
# Script API: `modelerData` (a pandas DataFrame of the input, or None) and `modelerDataModel`
# (a list of dicts: name, storage, measurement, role). Import and transform scripts leave the
# output in `modelerData`. Whatever the script prints is its text output.
import json
import sys
import traceback


# Import names whose pip package is called differently.
PIP_NAMES = {"sklearn": "scikit-learn", "imblearn": "imbalanced-learn"}


def write_result(path, result):
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(result, handle)


def script_error(exc):
    """The message and the user's script line of an exception raised in the script."""
    line = None
    frames = traceback.extract_tb(exc.__traceback__)
    for frame in frames:
        if frame.filename == "<script>":
            line = frame.lineno
    if isinstance(exc, SyntaxError) and exc.filename == "<script>":
        line = exc.lineno
    shown = [f for f in frames if f.filename != __file__]
    text = "".join(traceback.format_list(shown)) + "".join(
        traceback.format_exception_only(type(exc), exc)
    )
    message = "".join(traceback.format_exception_only(type(exc), exc)).strip()
    return {"error": "script", "message": message, "line": line, "traceback": text}


def read_input(pa, path):
    with pa.memory_map(path, "r") as source:
        table = pa.ipc.open_file(source).read_all()
    model = []
    for field in table.schema:
        meta = {k.decode(): v.decode() for k, v in (field.metadata or {}).items()}
        model.append(
            {
                "name": field.name,
                "storage": meta.get("dms.storage", "unknown"),
                "measurement": meta.get("dms.measurement", "Default"),
                "role": meta.get("dms.role", "Input"),
            }
        )
    import pandas as pd

    mapping = {
        pa.int64(): pd.Int64Dtype(),
        pa.float64(): pd.Float64Dtype(),
        pa.string(): pd.StringDtype(),
    }
    frame = table.to_pandas(types_mapper=mapping.get, date_as_object=True)
    return frame, model


def normalize(pa, table):
    """Casts every column to a type the app reads: int64, float64, string, date32, time64[us],
    timestamp[us]. Booleans become 0/1; time zones are converted to UTC."""
    import pyarrow.compute as pc

    columns = []
    names = []
    for name, column in zip(table.column_names, table.columns):
        kind = column.type
        if pa.types.is_dictionary(kind):
            column = column.cast(kind.value_type)
            kind = column.type
        if pa.types.is_boolean(kind):
            column = column.cast(pa.int64())
        elif pa.types.is_integer(kind):
            column = column.cast(pa.int64())
        elif pa.types.is_floating(kind) or pa.types.is_decimal(kind):
            column = column.cast(pa.float64())
        elif pa.types.is_string(kind) or pa.types.is_large_string(kind) or pa.types.is_null(kind):
            column = column.cast(pa.string())
        elif hasattr(pa.types, "is_string_view") and pa.types.is_string_view(kind):
            column = column.cast(pa.string())
        elif pa.types.is_date(kind):
            column = column.cast(pa.date32())
        elif pa.types.is_time(kind):
            column = column.cast(pa.time64("us"))
        elif pa.types.is_timestamp(kind):
            if kind.tz is not None:
                column = pc.cast(column, pa.timestamp(kind.unit, tz="UTC"))
                column = column.cast(pa.timestamp(kind.unit))
            column = column.cast(pa.timestamp("us"))
        else:
            raise TypeError(
                f"column {name!r} has type {kind}; convert it to numbers, text, dates or times"
            )
        names.append(str(name))
        columns.append(column)
    return pa.table(columns, names=names)


def write_output(pa, frame, path):
    import pandas as pd

    if not isinstance(frame, pd.DataFrame):
        raise TypeError(
            "modelerData must be a pandas DataFrame at the end of the script, not "
            + type(frame).__name__
        )
    if len(set(map(str, frame.columns))) != len(frame.columns):
        raise ValueError("modelerData has duplicate column names")
    table = normalize(pa, pa.Table.from_pandas(frame, preserve_index=False))
    with pa.OSFile(path, "wb") as sink:
        with pa.ipc.new_file(sink, table.schema) as writer:
            writer.write_table(table)
    return table.num_rows


def main():
    with open(sys.argv[1], encoding="utf-8") as handle:
        request = json.load(handle)
    result_path = request["result"]
    try:
        import pyarrow as pa
        import pyarrow.ipc  # noqa: F401
        import pandas  # noqa: F401
    except ImportError as exc:
        write_result(result_path, {"error": "missing_package", "package": exc.name or str(exc)})
        return
    namespace = {
        "__name__": "__main__",
        "modelerData": None,
        "modelerDataModel": [],
        "dmsParameters": request.get("parameters") or {},
    }
    inputs = request.get("inputs", [])
    if inputs:
        frame, model = read_input(pa, inputs[0])
        namespace["modelerData"] = frame
        namespace["modelerDataModel"] = model
    try:
        code = compile(request["script"], "<script>", "exec")
        exec(code, namespace)
    except ModuleNotFoundError as exc:
        sys.stdout.flush()
        package = PIP_NAMES.get(exc.name, exc.name) if exc.name else None
        if package:
            write_result(result_path, {"error": "missing_package", "package": package})
        else:
            write_result(result_path, script_error(exc))
        return
    except BaseException as exc:  # the user's script: report everything, including SystemExit
        sys.stdout.flush()
        write_result(result_path, script_error(exc))
        return
    sys.stdout.flush()
    result = {"ok": True}
    if request.get("output"):
        try:
            result["rows"] = write_output(pa, namespace.get("modelerData"), request["output"])
        except Exception as exc:
            write_result(result_path, {"error": "output", "message": str(exc)})
            return
    write_result(result_path, result)


if __name__ == "__main__":
    main()
