#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PRD_FILE="$ROOT_DIR/ralph/prd.json"
PROGRESS_FILE="$ROOT_DIR/ralph/progress.txt"
PROMPT_FILE="$ROOT_DIR/ralph/prompt.md"
TMP_DIR="$ROOT_DIR/.tmp"
LAST_MSG_FILE="$TMP_DIR/ralph-last-message.txt"
ITERATION="1"
DRY_RUN="0"
MODEL="${RALPH_MODEL:-gpt-5.3-codex}"

usage() {
  cat <<USAGE
Usage: $(basename "$0") [--iteration N] [--dry-run]

Options:
  --iteration N   Iteration number annotation passed into the prompt (default: 1)
  --dry-run       Print resolved command + prompt preview, do not call codex
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --iteration)
      ITERATION="${2:-}"
      shift 2
      ;;
    --dry-run)
      DRY_RUN="1"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage
      exit 1
      ;;
  esac
done

for required in "$PRD_FILE" "$PROGRESS_FILE" "$PROMPT_FILE"; do
  if [[ ! -f "$required" ]]; then
    echo "Missing required file: $required" >&2
    exit 1
  fi
done

mkdir -p "$TMP_DIR"

if codex exec --help 2>/dev/null | rg -q -- '--yolo'; then
  YOLO_FLAG=(--yolo)
else
  YOLO_FLAG=(--dangerously-bypass-approvals-and-sandbox)
fi

PROMPT="$(cat <<EOF_PROMPT
You are running Ralph Loop iteration ${ITERATION}.

Repository root: ${ROOT_DIR}

Read and follow:
- ${PRD_FILE}
- ${PROGRESS_FILE}

$(cat "$PROMPT_FILE")
EOF_PROMPT
)"

CMD=(
  codex exec
  --model "$MODEL"
  --cd "$ROOT_DIR"
  "--yolo"
  --output-last-message "$LAST_MSG_FILE"
  "$PROMPT"
)

if [[ "$DRY_RUN" == "1" ]]; then
  echo "[DRY RUN] Command:"
  printf '  %q' "${CMD[@]}"
  echo
  echo "[DRY RUN] Prompt preview:"
  echo "----------------------------------------"
  echo "$PROMPT" | sed -n '1,40p'
  echo "----------------------------------------"
  exit 0
fi

"${CMD[@]}"

if [[ -f "$LAST_MSG_FILE" ]]; then
  cat "$LAST_MSG_FILE"
fi
