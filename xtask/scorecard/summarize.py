"""Import rate and timing summary from target/scorecard/results.tsv (see run_all.py).

`python xtask/scorecard/summarize.py [regress.tsv]`: with the output of
`cargo xtask regress --out regress.tsv`, a stream that fails where the regression says it
matches Modeler (Modeler fails the same way) counts as behaving like Modeler.
"""
import csv, os, sys, statistics as st, re
ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
from collections import Counter, defaultdict
sys.stdout.reconfigure(encoding="utf-8")
R = list(csv.DictReader(open(os.path.join(ROOT, "target", "scorecard", "results.tsv"), encoding="utf-8"), delimiter="\t"))

BLOCKED = ("bayes_net", "svm", "discriminant", "genlin", "cox")          # license / not installed
OUT_OF_SCOPE = ("cplex_optimization", "simulation_generate", "space_time_boxes",
                "simulation_fit", "simulation_evaluation")
LATER = ("slrm",)


def category(r):
    if r["exported"] == "0" and r["failed"] == "0":
        return "no terminal (settings probe)"
    if r["status"] == "ok":
        return "ok"
    c = r["root_causes"]
    m = re.search(r"unsupported node type (\w+)", c)
    if m:
        t = m.group(1)
        if t in BLOCKED: return "node license-blocked in Modeler 19 (%s)" % t
        if t in OUT_OF_SCOPE: return "node out of scope (%s)" % t
        if t in LATER: return "node deferred (%s)" % t
        return "node not implemented (%s)" % t
    if "GAP: worksheet" in c: return "Excel fixture with cell-less rows (refused)"
    if "no scanned fields" in c: return "XML source without scanned fields"
    if re.search(r"corpus/ibm-demo-data\\\\|corpus/ibm-demo-data\\", c) or "cannot find" in c.lower() or "not found" in c.lower():
        return "data file missing from corpus"
    m = re.search(r"(\w+) is not supported yet", c)
    if m: return "setting not supported: " + m.group(1)
    m = re.search(r"GAP: ([^'|]{0,60})", c)
    if m: return "GAP: " + m.group(1).strip()
    return "other: " + c[:80]


cats = Counter(category(r) for r in R)
by_label = defaultdict(Counter)
for r in R:
    by_label[r["label"]][category(r) == "ok"] += 1
runnable = [r for r in R if category(r) != "no terminal (settings probe)"]
ok = [r for r in runnable if r["status"] == "ok"]
partial = [r for r in runnable if r["status"] == "partial"]
blocked = [r for r in runnable if category(r).startswith(("node license", "node out of scope"))]
print("streams in corpus: %d; with terminals: %d" % (len(R), len(runnable)))
if len(sys.argv) > 1:
    classes = {(g["label"], g["stem"]): g["class"] for g in csv.DictReader(open(sys.argv[1], encoding="utf-8"), delimiter="	")}
    alike = [r for r in runnable if r["status"] != "ok" and classes.get((r["label"], r["stream"][:-4])) == "match"]
    print("fail exactly as Modeler does (regress match): %d; run or fail like Modeler: %d of %d (%.1f %%)" % (
        len(alike), len(ok_streams := [r for r in runnable if r["status"] == "ok"]) + len(alike), len(runnable),
        100 * (len(ok_streams) + len(alike)) / len(runnable)))
print("run completely: %d (%.1f %%); partially: %d; not at all: %d" % (
    len(ok), 100 * len(ok) / len(runnable), len(partial), len(runnable) - len(ok) - len(partial)))
fair = [r for r in runnable if r not in blocked]
print("excluding %d streams whose node Modeler 19 here cannot run either / out of scope: %d of %d (%.1f %%)" % (
    len(blocked), len(ok), len(fair), 100 * len(ok) / len(fair)))
print()
for k, v in cats.most_common():
    print("%4d  %s" % (v, k))

print("\n== timing (release build; Modeler = oracle clemb run incl. start-up and exports)")
ours = [float(r["ours_secs"]) for r in runnable]
both = [(float(r["ours_secs"]), float(r["modeler_secs"])) for r in runnable if r["modeler_secs"]]
startup = min(m for _, m in both)
print("ours per stream: median %.3f s, p90 %.3f s, max %.3f s" % (
    st.median(ours), sorted(ours)[int(.9 * len(ours))], max(ours)))
print("Modeler per stream: median %.2f s (minimum %.2f s, taken as start-up)" % (st.median([m for _, m in both]), startup))
heavy = [(o, m) for o, m in both if m - startup > 2.0]
r2 = sorted((m - startup) / o for o, m in heavy)
print("%d streams where Modeler needs > 2 s beyond start-up: Modeler/ours median %.0fx, min %.1fx" % (len(heavy), st.median(r2), r2[0]))
