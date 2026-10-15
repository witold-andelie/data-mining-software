# Oracle script for IBM SPSS Modeler (Jython 2.7), run by `cargo xtask oracle` through
# `clemb -script <this file> -execute`. Uses only the public Modeler Python scripting API
# (IBM SPSS Modeler 19.0 Python Scripting and Automation Guide).
#
# For one stream it writes into OUT_DIR:
#   models/NN_<label>.xml      PMML of every model the stream builds
#   data/NN_<type>_<id>.csv    the records reaching each terminal node (UTF-8, comma, header row)
#   exports/<name>             files written by the stream's own export nodes
#   manifest.json              what was exported, relocated or missing, and every error
#
# Data files referenced by absolute paths from another machine are relocated by file name to a
# match under DATA_DIRS; export targets are redirected into OUT_DIR/exports.
import json
import java.io.File
import java.lang.Exception
import modeler.api

STREAM_PATH = @STREAM_PATH@
OUT_DIR = @OUT_DIR@
DATA_DIRS = @DATA_DIRS@
MAX_SEARCH_DEPTH = 4
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)

manifest = {"stream": STREAM_PATH, "models": [], "data": [], "relocated": [],
            "missing_data": [], "errors": []}


def message(exc):
    get = getattr(exc, "getMessage", None)
    if get is not None:
        return unicode(get())
    return unicode(exc)


def describe(node):
    return {"id": node.getID(), "type": node.getTypeName(), "label": node.getLabel()}


def record_error(stage, node, exc):
    entry = {"stage": stage, "message": message(exc)}
    if node is not None:
        entry["node"] = describe(node)
    manifest["errors"].append(entry)


def file_safe(text):
    return "".join([c if (c.isalnum() or c in "-_.") else "_" for c in text])[:60]


def find_data_file(name):
    wanted = name.lower()
    for root in DATA_DIRS:
        pending = [(java.io.File(root), 0)]
        while pending:
            directory, depth = pending.pop()
            children = directory.listFiles()
            if children is None:
                continue
            for child in children:
                if child.isDirectory():
                    if depth < MAX_SEARCH_DEPTH:
                        pending.append((child, depth + 1))
                elif child.getName().lower() == wanted:
                    return child.getPath().replace("\\", "/")
    return None


def file_property(node):
    try:
        value = node.getPropertyValue("full_filename")
    except ERRORS, e:
        return None
    if value is None or not unicode(value) or unicode(value).startswith("$"):
        return None
    return unicode(value)


def relocate_files(stream):
    nodes = []
    iterator = stream.iterator()
    while iterator.hasNext():
        nodes.append(iterator.next())
    for node in nodes:
        path = file_property(node)
        if path is None:
            continue
        name = path.replace("\\", "/").split("/")[-1]
        if node.isInitial():
            if java.io.File(path).exists():
                continue
            found = find_data_file(name)
            if found is None:
                manifest["missing_data"].append({"node": describe(node), "path": path})
                continue
            target = found
        elif node.isTerminal():
            safe = file_safe(name)
            if not safe.strip("."):
                safe = "export"
            target = "%s/exports/%s" % (OUT_DIR, safe)
        else:
            continue
        try:
            node.setPropertyValue("full_filename", target)
            manifest["relocated"].append({"node": describe(node), "from": path, "to": target})
        except ERRORS, e:
            record_error("relocate", node, e)


def export_models(stream, taskrunner):
    outputs = []
    stream.runAll(outputs)
    index = 0
    for output in outputs:
        if not isinstance(output, modeler.api.ModelOutput):
            continue
        index += 1
        label = output.getLabel()
        path = "%s/models/%02d_%s.xml" % (OUT_DIR, index, file_safe(label))
        entry = {"label": label, "file": path}
        try:
            entry["algorithm"] = output.getModelDetail().getAlgorithmName()
        except ERRORS, e:
            entry["algorithm"] = None
        try:
            taskrunner.exportModelToFile(output, path, modeler.api.FileFormat.XML)
            manifest["models"].append(entry)
        except ERRORS, e:
            record_error("export_model", None, e)


def terminal_nodes(stream):
    terminals = []
    iterator = stream.iterator()
    while iterator.hasNext():
        node = iterator.next()
        if node.isTerminal():
            terminals.append(node)
    return terminals


def export_terminal_inputs(stream):
    exported = {}
    index = 0
    for terminal in terminal_nodes(stream):
        for source in stream.predecessors(terminal):
            if source.getID() in exported:
                continue
            exported[source.getID()] = True
            index += 1
            path = "%s/data/%02d_%s_%s.csv" % (
                OUT_DIR, index, file_safe(source.getTypeName()), file_safe(source.getID()))
            try:
                export = stream.createAt(
                    "outputfile", "oracle export",
                    source.getXPosition() + 60, source.getYPosition() + 60)
                export.setPropertyValue("full_filename", path)
                export.setPropertyValue("write_mode", "Overwrite")
                export.setPropertyValue("inc_field_names", True)
                export.setPropertyValue("delimit_mode", "Comma")
                export.setPropertyValue("quote_mode", "Double")
                export.setPropertyValue("decimal_symbol", "Period")
                export.setPropertyValue("encoding", "UTF-8")
                export.setPropertyValue("generate_import", False)
                stream.link(source, export)
                export.run([])
                entry = describe(source)
                entry["file"] = path
                entry["terminal"] = describe(terminal)
                manifest["data"].append(entry)
            except ERRORS, e:
                record_error("export_data", source, e)


def write_manifest():
    handle = open(OUT_DIR + "/manifest.json", "w")
    try:
        json.dump(manifest, handle, indent=2, sort_keys=True)
    finally:
        handle.close()


session = modeler.script.session()
taskrunner = session.getTaskRunner()
stream = None
try:
    stream = taskrunner.openStreamFromFile(STREAM_PATH, True)
except ERRORS, e:
    record_error("open_stream", None, e)
if stream is not None:
    relocate_files(stream)
    try:
        export_models(stream, taskrunner)
    except ERRORS, e:
        record_error("run_stream", None, e)
    export_terminal_inputs(stream)
write_manifest()
