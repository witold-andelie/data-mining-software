#!/usr/bin/env bash
# Steps 1, 3 and 4 of docs/mac/MAC_VALIDATION.md as fixed commands, so that the opencode
# subagent only needs permission to run this script. Each step keeps its full log in
# target/mac/<step>.log and ends with one summary line. Bash 3.2 compatible (macOS).
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2
mkdir -p target/mac
step="${1:-}"
case "$step" in
  ci)
    cmd=(cargo xtask ci) ;;
  test-corpus)
    cmd=(cargo xtask test-corpus) ;;
  regress)
    export DMS_MODELER_HOME="${DMS_MODELER_HOME:-$HOME/modeler-home}"
    labels=()
    while IFS= read -r label; do
      labels+=(--label "$label")
    done < <(awk -F'\t' 'NR>1 && $1!~/^#/ {print $1}' xtask/corpus.tsv | sort -u \
             | grep -vx -e database -e probes)
    cmd=(cargo xtask regress "${labels[@]}" --jobs 4 --out target/mac/mac-regress.tsv) ;;
  compare)
    cmd=(python3 docs/mac/compare_baseline.py docs/mac/windows-baseline.tsv
         target/mac/mac-regress.tsv target/mac/mac-compare.txt) ;;
  details)
    # One `cargo xtask compare` per stream whose class differs from the Windows baseline
    # (streams absent on either side are skipped).
    export DMS_MODELER_HOME="${DMS_MODELER_HOME:-$HOME/modeler-home}"
    mkdir -p target/mac/details
    start=$(date +%s)
    count=0
    while IFS= read -r line; do
      case "$line" in *absent*) continue ;; esac
      key="${line%%:*}"
      label="${key%%/*}"
      stem="${key#*/}"
      cargo xtask compare "$label" "$stem.str" >| "target/mac/details/${label}__${stem}.txt" 2>&1
      count=$((count + 1))
    done < <(grep -E '^[^ ]+/[^ ]+: ' target/mac/mac-compare.txt)
    echo "mac-steps details: $count streams compared elapsed=$(( $(date +%s) - start ))s dir=target/mac/details" \
      | tee target/mac/details.log
    exit 0 ;;
  *)
    echo "usage: mac-steps.sh ci|test-corpus|regress|compare|details"
    exit 2 ;;
esac
start=$(date +%s)
"${cmd[@]}" 2>&1 | tee "target/mac/$step.log"
code=${PIPESTATUS[0]}
echo "mac-steps $step: exit=$code elapsed=$(( $(date +%s) - start ))s log=target/mac/$step.log" \
  | tee -a "target/mac/$step.log"
exit "$code"
