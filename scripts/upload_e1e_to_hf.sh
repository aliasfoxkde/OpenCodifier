#!/usr/bin/env bash
# Upload E1-E 2B adapter to HuggingFace Hub.
# Run this AFTER getting operator approval and adding org name to MODEL_CARD.md.
#
# Prerequisites:
#   pip install huggingface_hub
#   huggingface-cli login  (with a token that can write to the org)
#
# Usage:
#   ./upload_e1e_to_hf.sh <org-name> [--dry-run]
#
# Example:
#   ./upload_e1e_to_hf.sh my-org-name

set -euo pipefail

ORG="${1:?Usage: $0 <org-name> [--dry-run]}"
REPO_ID="${ORG}/opencodifier-e1e-2b"
TEMP_DIR="$(mktemp -d)"
trap "rm -rf $TEMP_DIR" EXIT

DRY_RUN=""
if [[ "${2:-}" == "--dry-run" ]]; then
  DRY_RUN="--dry-run"
  echo "=== DRY RUN — no changes will be made ==="
fi

echo "Staging files to $TEMP_DIR ..."

# --- Adapter files (from T5500 or local copy) ---
ADAPTER_DIR="${ADAPTER_DIR:-/home/mkinney/t5500-bundle/runs/e1-e-qwen2b/adapter-best}"
cp "$ADAPTER_DIR/adapter_model.safetensors" "$TEMP_DIR/"
cp "$ADAPTER_DIR/adapter_config.json" "$TEMP_DIR/"

# --- Write model card from template ---
# Fill in BOTH org placeholders (line 92's bare [OPERATOR] went
# unsubstituted before this — the card shipped with a raw placeholder;
# the post-substitution grep below is the guard). Each pattern is
# escaped as a literal so sed never treats it as a character class, and
# `|` is the delimiter so an org name never collides with the pattern;
# HF orgs are [A-Za-z0-9-_] but the delimiter makes that a
# non-requirement.
sed -e "s|\[OPERATOR: add org name\]|$ORG|g" \
    -e "s|\[OPERATOR\]|$ORG|g" \
  docs/MODEL_CARD_E1E.md > "$TEMP_DIR/README.md"
if grep -q "\[OPERATOR" "$TEMP_DIR/README.md"; then
  echo "ERROR: unsubstituted placeholder remains in README.md:" >&2
  grep -n "\[OPERATOR" "$TEMP_DIR/README.md" >&2
  exit 1
fi

# --- Calibration artifact ---
# Copy the fitted calibration artifact so users can recalibrate
CALIB_SRC="benchmarks/decision-model/results/calibration/e1e-qwen3.5-2b-holdout.json"
if [[ -f "$CALIB_SRC" ]]; then
  mkdir -p "$TEMP_DIR/calibration"
  cp "$CALIB_SRC" "$TEMP_DIR/calibration/fit.json"
fi

# --- Upload ---
echo ""
echo "Repo ID:   $REPO_ID"
echo "Org:       $ORG"
echo "Files:"
ls -lh "$TEMP_DIR"
echo ""

if [[ -z "$DRY_RUN" ]]; then
  echo "Uploading ... (Ctrl+C to abort)"
  huggingface-cli upload \
    "$REPO_ID" \
    "$TEMP_DIR/" \
    --repo-type model \
    --commit-message "Initial upload: E1-E 2B verdict-slot adapter (opencodifier-e1e-2b)"
  echo ""
  echo "Done. Model available at: https://huggingface.co/$REPO_ID"
else
  echo "Would upload to: https://huggingface.co/$REPO_ID"
  echo "Run without --dry-run to upload."
fi
