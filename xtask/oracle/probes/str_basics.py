# Probe: basics of the .str stream file format, for differential observation.
# Run with `cargo xtask probe str_basics` (IBM SPSS Modeler, Jython 2.7, public scripting API only).
#
# Each series mutates ONE stream step by step and saves it after every step, so consecutive files
# differ by exactly one change (plus whatever Modeler rewrites on every save, which the "empty"
# series isolates by saving an unchanged stream twice).
import json
import java.lang.Exception
import modeler.api

OUT_DIR = @OUT_DIR@
ERRORS = (modeler.api.ModelerException, java.lang.Exception, Exception)

taskrunner = modeler.script.session().getTaskRunner()
manifest = {"saved": [], "steps": [], "errors": []}


def message(exc):
    get = getattr(exc, "getMessage", None)
    if get is not None:
        return unicode(get())
    return unicode(exc)


class Series(object):
    def __init__(self, name):
        self.name = name
        self.index = 0
        self.stream = taskrunner.createStream(name, False, False)

    def step(self, description, change):
        """Apply `change(stream)` and save; a failed change is recorded and skipped."""
        self.index += 1
        file_name = "%s_%02d.str" % (self.name, self.index)
        try:
            change(self.stream)
            taskrunner.saveStreamToFile(self.stream, "%s/%s" % (OUT_DIR, file_name))
            manifest["saved"].append(file_name)
            manifest["steps"].append({"file": file_name, "change": description})
        except ERRORS, e:
            manifest["errors"].append({"file": file_name, "change": description, "message": message(e)})


def nothing(stream):
    pass


def set_prop(node_type, name, value):
    def change(stream):
        stream.findByType(node_type, None).setPropertyValue(name, value)
    return change


# Series "empty": what changes on every save of an unchanged stream.
empty = Series("empty")
empty.step("save empty stream", nothing)
empty.step("save the same empty stream again", nothing)

# Series "vf": one Var. File node, then strings, flags, integers, label, position, links.
vf = Series("vf")
vf.step("add variablefile node at (100, 100)",
        lambda s: s.createAt("variablefile", "File", 100, 100))
vf.step("full_filename = 'a'", set_prop("variablefile", "full_filename", "a"))
vf.step("full_filename = 'bb'", set_prop("variablefile", "full_filename", "bb"))
vf.step("full_filename = 200 x 'c'", set_prop("variablefile", "full_filename", "c" * 200))
vf.step("full_filename = 300 x 'd'", set_prop("variablefile", "full_filename", "d" * 300))
vf.step("full_filename = 70000 x 'e'", set_prop("variablefile", "full_filename", "e" * 70000))
vf.step("full_filename = u'\\u6570\\u636e.csv' (Chinese)",
        set_prop("variablefile", "full_filename", u"数据.csv"))
vf.step("read_field_names = False", set_prop("variablefile", "read_field_names", False))
vf.step("read_field_names = True", set_prop("variablefile", "read_field_names", True))
vf.step("skip_header = 1", set_prop("variablefile", "skip_header", 1))
vf.step("skip_header = 300", set_prop("variablefile", "skip_header", 300))
vf.step("skip_header = 70000", set_prop("variablefile", "skip_header", 70000))
vf.step("skip_header = 0", set_prop("variablefile", "skip_header", 0))
vf.step("setLabel('Custom Label')",
        lambda s: s.findByType("variablefile", None).setLabel("Custom Label"))
vf.step("move node to (300, 100)",
        lambda s: s.findByType("variablefile", None).setXYPosition(300, 100))
vf.step("add table node at (500, 100), not linked",
        lambda s: s.createAt("table", "Table", 500, 100))
vf.step("link variablefile -> table",
        lambda s: s.link(s.findByType("variablefile", None), s.findByType("table", None)))

# Series "sample": real numbers.
sample = Series("sample")
sample.step("add sample node", lambda s: s.createAt("sample", "Sample", 100, 100))
sample.step("rand_pct = 50", set_prop("sample", "rand_pct", 50))
sample.step("rand_pct = 12.5", set_prop("sample", "rand_pct", 12.5))
sample.step("rand_pct = 0.1", set_prop("sample", "rand_pct", 0.1))
sample.step("rand_pct = -1.5e300", set_prop("sample", "rand_pct", -1.5e300))

# Does exportStreamToFile(..., FileFormat.XML) produce a readable form of the stream?
try:
    taskrunner.exportStreamToFile(vf.stream, OUT_DIR + "/vf_export.xml", modeler.api.FileFormat.XML)
    manifest["saved"].append("vf_export.xml")
except ERRORS, e:
    manifest["errors"].append({"file": "vf_export.xml", "change": "exportStreamToFile XML",
                               "message": message(e)})

handle = open(OUT_DIR + "/manifest.json", "w")
try:
    json.dump(manifest, handle, indent=2, sort_keys=True)
finally:
    handle.close()
