"""Run every corpus stream with the release dms-cli: did it run, and how long did it take?

Build first: `cargo build --release -p dms-cli`. Run: `python xtask/scorecard/run_all.py`.
Writes target/scorecard/results.tsv (one row per stream). Modeler's time is the oracle run's
elapsed_secs (clemb batch, including its start-up and exports) when the stream has an oracle.
Data directories follow `cargo xtask compare`: the label's corpus directory, ibm-demo-data and
$DMS_MODELER_HOME/Demos.
"""
import csv, json, os, shutil, subprocess, sys, time

sys.stdout.reconfigure(encoding="utf-8")
ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
CLI = os.path.join(ROOT, "target", "release", "dms-cli" + (".exe" if os.name == "nt" else ""))
OUT = os.path.join(ROOT, "target", "scorecard", "out")
MODELER_HOME = os.environ.get("DMS_MODELER_HOME")

streams = []
for line in open(ROOT + "/xtask/corpus.tsv", encoding="utf-8"):
    if line.startswith("#") or not line.strip():
        continue
    label, path = line.split("\t")[:2]
    if path.endswith(".str"):
        streams.append((label, path))

rows = []
for index, (label, path) in enumerate(streams):
    stem = path[:-4]
    out = os.path.join(OUT, label, stem)
    shutil.rmtree(out, ignore_errors=True)
    os.makedirs(out, exist_ok=True)
    args = [CLI, "run", "%s/corpus/%s/%s" % (ROOT, label, path), "--out", out]
    for d in ("%s/corpus/%s" % (ROOT, label), ROOT + "/corpus/ibm-demo-data",
              (MODELER_HOME + "/Demos") if MODELER_HOME else None):
        if d and os.path.isdir(d):
            args += ["--data-dir", d]
    start = time.perf_counter()
    try:
        proc = subprocess.run(args, capture_output=True, text=True, timeout=900)
        code, err = proc.returncode, proc.stderr.strip().splitlines()[-1:] if proc.stderr else []
    except subprocess.TimeoutExpired:
        code, err = "timeout", []
    secs = time.perf_counter() - start
    exported = failed = 0
    causes = []
    try:
        report = json.load(open(os.path.join(out, "run.json"), encoding="utf-8"))
        exported, failed = len(report.get("exported", [])), len(report.get("failed", []))
        causes = [str(c)[:160] for c in report.get("root_causes", [])]
    except (OSError, ValueError):
        pass
    modeler = None
    prov = "%s/oracle/%s/%s/provenance.json" % (ROOT, label, stem)
    if os.path.exists(prov):
        modeler = json.load(open(prov, encoding="utf-8")).get("elapsed_secs")
    status = ("ok" if failed == 0 and exported > 0 else
              "partial" if exported > 0 else "failed")
    rows.append([label, path, status, exported, failed, "%.3f" % secs,
                 "" if modeler is None else "%.3f" % modeler, code, " | ".join(causes)[:400]])
    if index % 100 == 0:
        print(index, "/", len(streams), label, path, status, "%.2fs" % secs, flush=True)

os.makedirs(os.path.join(ROOT, "target", "scorecard"), exist_ok=True)
with open(os.path.join(ROOT, "target", "scorecard", "results.tsv"), "w", encoding="utf-8", newline="") as f:
    w = csv.writer(f, delimiter="\t")
    w.writerow(["label", "stream", "status", "exported", "failed", "ours_secs", "modeler_secs", "exit", "root_causes"])
    w.writerows(rows)
print("done", len(rows))
