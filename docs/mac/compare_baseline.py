#!/usr/bin/env python3
"""Compare a `cargo xtask regress --out` TSV with the Windows baseline, stream by stream.

    python3 docs/mac/compare_baseline.py docs/mac/windows-baseline.tsv target/mac/mac-regress.tsv [out.txt]

Prints every stream whose class differs (match / statistical / known-gap / regression),
streams present on one side only, and a summary count per transition. With a third
argument the same text is also written to that file. Exits 1 when any stream differs.
"""
import collections
import csv
import sys


def load(path):
    rows = {}
    with open(path, encoding="utf-8", newline="") as handle:
        for row in csv.DictReader(handle, delimiter="\t"):
            rows[(row["label"], row["stem"])] = row
    return rows


def main():
    if len(sys.argv) not in (3, 4):
        sys.exit("usage: compare_baseline.py <windows-baseline.tsv> <mac.tsv> [out.txt]")
    base, mac = load(sys.argv[1]), load(sys.argv[2])
    lines = []
    transitions = collections.Counter()
    changed = []
    for key in sorted(set(base) | set(mac)):
        b = base.get(key, {}).get("class", "absent")
        m = mac.get(key, {}).get("class", "absent")
        transitions[(b, m)] += 1
        if b != m:
            changed.append((key, b, m, mac.get(key, {}).get("problems", "")))
    lines.append(f"windows streams: {len(base)}, mac streams: {len(mac)}")
    for (b, m), count in sorted(transitions.items()):
        marker = "" if b == m else "   <-- differs"
        lines.append(f"  {b:>12} -> {m:<12} {count}{marker}")
    lines.append("")
    for (label, stem), b, m, problems in changed:
        lines.append(f"{label}/{stem}: {b} -> {m}  {problems[:300]}")
    text = "\n".join(lines) + "\n"
    sys.stdout.write(text)
    if len(sys.argv) == 4:
        with open(sys.argv[3], "w", encoding="utf-8", newline="\n") as handle:
            handle.write(text)
    sys.exit(1 if changed else 0)


if __name__ == "__main__":
    main()
