#!/usr/bin/env python3
"""Compare a `cargo xtask regress --out` TSV with the Windows baseline, stream by stream.

    python3 docs/mac/compare_baseline.py docs/mac/windows-baseline.tsv target/mac-regress.tsv

Prints every stream whose class differs (match / statistical / known-gap / regression),
streams present on one side only, and a summary count per transition.
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
    if len(sys.argv) != 3:
        sys.exit("usage: compare_baseline.py <windows-baseline.tsv> <mac.tsv>")
    base, mac = load(sys.argv[1]), load(sys.argv[2])
    transitions = collections.Counter()
    changed = []
    for key in sorted(set(base) | set(mac)):
        b = base.get(key, {}).get("class", "absent")
        m = mac.get(key, {}).get("class", "absent")
        transitions[(b, m)] += 1
        if b != m:
            changed.append((key, b, m, mac.get(key, {}).get("problems", "")))
    print(f"windows streams: {len(base)}, mac streams: {len(mac)}")
    for (b, m), count in sorted(transitions.items()):
        marker = "" if b == m else "   <-- differs"
        print(f"  {b:>12} -> {m:<12} {count}{marker}")
    print()
    for (label, stem), b, m, problems in changed:
        print(f"{label}/{stem}: {b} -> {m}  {problems[:300]}")
    sys.exit(1 if changed else 0)


if __name__ == "__main__":
    main()
