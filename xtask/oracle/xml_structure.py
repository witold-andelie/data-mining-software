"""Give saved XML source streams the structure Modeler's XML source dialog would scan.

Modeler 19 builds an XML source node's fields only in the node dialog, which scripting cannot
open: a scripted node runs as "Node -1 has no exports" in batch and in the client. The dialog
stores what it scanned in two places, both visible in a dialog-saved stream
(corpus/sources/xml_template.str, saved by the user from source_records.xml, records
/records/record, 2026-10-20):

- the node's parameter attachment: `xmlReader.xpaths` (field_name, xpath),
  `xmlReader.storages` and `xmlReader.ui_storages` (field_name, override=false, storage), as
  `StructuredValue`s;
- the node's `<cachedDataModel>` in the stream XML, one `<field>` per field in storage order.

Modeler needs both: parameters alone still give "no exports"; a data model alone gives
"Incorrect number of fields detected in the server data model". This script writes both into
copies of streams and stores the stream XML as plain text, which Modeler opens; the
`xml_scan_streams` probe then re-saves each copy through Modeler.

The structure follows the template's scan of source_records.xml: the record's leaf child
elements in first-appearance order, then its attributes; storage `integer` when every value
present is a non-empty integer, otherwise `string` (the template has qty, price and day as
string because record 5 holds empty elements, and the attribute id as integer). These are
stream settings like any other: the oracles show how Modeler reads a stream that holds them.

Usage (CPython 3, from the repository root):
  py -I xtask/oracle/xml_structure.py <dms-cli.exe> <out-dir> <stream.str>...
"""

import os
import re
import subprocess
import sys
import xml.etree.ElementTree as ET
import zipfile
from xml.sax.saxutils import quoteattr, escape

INTEGER = re.compile(r"^[+-]?\d+$")


def parameters(text):
    return dict(re.findall(r'<Parameter name="(xmlReader\.[^"]+)" value="([^"]*)"', text))


def data_files(params):
    if params.get("xmlReader.mode") == "directory":
        root = params["xmlReader.name"]
        files = []
        for base, dirs, names in os.walk(root):
            dirs.sort()
            files += [os.path.join(base, n) for n in sorted(names) if n.lower().endswith(".xml")]
            if params.get("xmlReader.recurse") != "true":
                break
        return files
    return [params["xmlReader.full_filename"]]


def scan(files, records):
    """(field, xpath, storage) for the record elements at the absolute path `records`."""
    steps = [s for s in records.split("/") if s]
    elements, attributes, values = [], [], {}
    for path in files:
        try:
            root = ET.parse(path).getroot()
        except (ET.ParseError, OSError):
            continue
        if root.tag != steps[0]:
            continue
        found = [root]
        for step in steps[1:]:
            found = [child for node in found for child in node if child.tag == step]
        for record in found:
            for child in record:
                if len(child) == 0 and child.tag not in elements:
                    elements.append(child.tag)
                if len(child) == 0:
                    values.setdefault(child.tag, []).append((child.text or ""))
            for name, value in record.attrib.items():
                if name not in attributes:
                    attributes.append(name)
                values.setdefault("@" + name, []).append(value)
    fields = []
    for name in elements:
        fields.append((name, records + "/" + name, values[name]))
    for name in attributes:
        fields.append((name, records + "/@" + name, values["@" + name]))
    return [
        (name, xpath, "integer" if vals and all(INTEGER.match(v) for v in vals) else "string")
        for name, xpath, vals in fields
    ]


def structured(items):
    body = "".join(
        "<StructuredValue>\n"
        + "".join('<Attribute name="%s" value=%s/>\n' % (k, quoteattr(v)) for k, v in item)
        + "</StructuredValue>\n"
        for item in items
    )
    return "<ListValue>\n" + body + "</ListValue>"


def set_list(text, key, items):
    new = '<Parameter name="%s">\n%s\n</Parameter>' % (key, structured(items))
    out, n = re.subn(r'<Parameter name="%s">\s*<ListValue/>\s*</Parameter>' % re.escape(key), new, text)
    if n != 1:
        raise SystemExit("%s: expected one empty %s" % (key, key))
    return out


def data_model(fields):
    out = ["<cachedDataModel>"]
    for name, _, storage in fields:
        measure = "range" if storage == "integer" else "discrete"
        out.append(
            "<field>\n<name>%s</name>\n<type>\n<storage>%s</storage>\n<measure>%s</measure>\n"
            "<values/>\n<truncated>false</truncated>\n<storageSize>-1</storageSize>\n"
            "<elementCount>0</elementCount>\n</type>\n</field>" % (escape(name), storage, measure)
        )
    out.append("</cachedDataModel>")
    return "\n".join(out)


def main():
    cli, out_dir, streams = sys.argv[1], sys.argv[2], sys.argv[3:]
    os.makedirs(out_dir, exist_ok=True)
    for stream in streams:
        archive = zipfile.ZipFile(stream)
        infos = archive.infolist()
        parts = {i.filename: archive.read(i.filename) for i in infos}
        main_part = next(i.filename for i in infos if not i.filename.startswith("data/"))
        xml = subprocess.run([cli, "xml", stream], check=True, capture_output=True).stdout.decode("utf-8")
        report = []
        for node in re.finditer(r'<node [^>]*extensionID="spss\.xml_data"[^>]*properties="([^"]+)"', xml):
            member = node.group(1)
            text = parts[member].decode("utf-8")
            params = parameters(text)
            fields = scan(data_files(params), params["xmlReader.records"])
            if fields:
                text = set_list(text, "xmlReader.xpaths", [[("field_name", f), ("xpath", x)] for f, x, _ in fields])
                storages = [[("field_name", f), ("override", "false"), ("storage", s)] for f, _, s in fields]
                text = set_list(text, "xmlReader.storages", storages)
                text = set_list(text, "xmlReader.ui_storages", storages)
                parts[member] = text.encode("utf-8")
                start = node.end()
                empty = xml.index("<cachedDataModel/>", start)
                xml = xml[:empty] + data_model(fields) + xml[empty + len("<cachedDataModel/>"):]
            report.append("%s: %s" % (member, ", ".join("%s %s" % (f, s) for f, _, s in fields) or "no fields"))
        parts[main_part] = xml.encode("utf-8")
        target = os.path.join(out_dir, os.path.basename(stream))
        with zipfile.ZipFile(target, "w", zipfile.ZIP_DEFLATED) as out:
            for info in infos:
                out.writestr(info, parts[info.filename])
        print("%s: %s" % (os.path.basename(stream), "; ".join(report)))


if __name__ == "__main__":
    main()
