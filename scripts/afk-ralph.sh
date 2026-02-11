#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ITERATIONS="${1:-}"
DRY_RUN="0"

usage() {
  cat <<USAGE
Usage: $(basename "$0") <iterations> [--dry-run]

Examples:
  $(basename "$0") 5
  $(basename "$0") 10 --dry-run
USAGE
}

if [[ -z "${ITERATIONS}" ]] || [[ "${ITERATIONS}" == "--dry-run" ]]; then
  usage
  exit 1
fi

if ! [[ "$ITERATIONS" =~ ^[0-9]+$ ]]; then
  echo "iterations must be a positive integer" >&2
  exit 1
fi

if [[ $# -ge 2 ]]; then
  if [[ "$2" == "--dry-run" ]]; then
    DRY_RUN="1"
  else
    echo "Unknown argument: $2" >&2
    usage
    exit 1
  fi
fi

for ((i=1; i<=ITERATIONS; i++)); do
  echo "========== Ralph Loop iteration ${i}/${ITERATIONS} =========="

  if [[ "$DRY_RUN" == "1" ]]; then
    "$ROOT_DIR/scripts/ralph-once.sh" --iteration "$i" --dry-run
    continue
  fi

  ITERATION_OUTPUT="$($ROOT_DIR/scripts/ralph-once.sh --iteration "$i")"
  echo "$ITERATION_OUTPUT"

  if [[ "$ITERATION_OUTPUT" == *"<promise>COMPLETE</promise>"* ]]; then
    echo "Ralph Loop marked COMPLETE at iteration ${i}."
    exit 0
  fi

done

echo "Ralph Loop reached max iterations (${ITERATIONS}) without COMPLETE marker."
