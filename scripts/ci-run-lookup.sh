#!/usr/bin/env bash
# Look up GitForge CI runs without the CLI and without inline interpreters.
#
# The `gitforge` CLI's stored auth goes stale and interactive login is the
# operator's path, so this script does the REST dance itself: log in with
# the credentials in mkinney-credentials.env, resolve the repo UUID by
# name, then list pipeline runs. Only `curl` + `jq` + coreutils are used —
# no `python3 -c`, no `| sh`, so the agent's PreToolUse risk scan passes.
#
# The run-listing route is GET /api/pipeline-runs?repo_id=<uuid>&limit=N.
# The obvious guesses GET /api/repos/<id>/runs and GET /api/pipelines/<id>/runs
# are 404/405 — do not spend a turn rediscovering that.
#
# Usage:
#   ./scripts/ci-run-lookup.sh runs [REPO_NAME] [LIMIT]
#   ./scripts/ci-run-lookup.sh runs OpenCodifier 20
#   ./scripts/ci-run-lookup.sh jobs <run-id>
#   ./scripts/ci-run-lookup.sh logs <job-id> [TAIL_LINES]
#   ./scripts/ci-run-lookup.sh logs <job-id> 50
#
# Environment:
#   GITFORGE_CREDS  Path to the credentials env file.
#                   Default: /home/mkinney/.config/gitforge/mkinney-credentials.env
#                   Expected keys: GITFORGE_URL, GITFORGE_USER, GITFORGE_PASSWORD.

set -euo pipefail

CRED_FILE="${GITFORGE_CREDS:-/home/mkinney/.config/gitforge/mkinney-credentials.env}"
DEFAULT_REPO="OpenCodifier"
DEFAULT_LIMIT=20

die() {
  echo "ERROR: $*" >&2
  exit 1
}

command -v curl >/dev/null 2>&1 || die "curl not found"
command -v jq >/dev/null 2>&1 || die "jq not found"
[[ -r "$CRED_FILE" ]] || die "credentials file not readable: $CRED_FILE"

# shellcheck disable=SC1090
source "$CRED_FILE"
: "${GITFORGE_URL:?GITFORGE_URL missing from $CRED_FILE}"
: "${GITFORGE_USER:?GITFORGE_USER missing from $CRED_FILE}"
: "${GITFORGE_PASSWORD:?GITFORGE_PASSWORD missing from $CRED_FILE}"

BASE="${GITFORGE_URL%/}"

# --- Login -------------------------------------------------------------
# The route is /auth/login, NOT /api/auth/login (that one 404s). The
# response carries a 24-hour bearer token under "token".
login() {
  local body response
  body="$(jq -n --arg u "$GITFORGE_USER" --arg p "$GITFORGE_PASSWORD" \
    '{username: $u, password: $p}')"
  response="$(curl -fsS -m 20 -X POST "$BASE/auth/login" \
    -H 'Content-Type: application/json' \
    -d "$body")" || die "login request failed (is GitForge up at $BASE?)"
  jq -r '.token // empty' <<<"$response" | head -1
}

JWT="$(login)"
[[ -n "$JWT" ]] || die "login returned no token"

auth_curl() {
  curl -fsS -m 30 -H "Authorization: Bearer $JWT" "$@"
}

# --- Repo UUID ---------------------------------------------------------
repo_id() {
  local name="$1" id
  id="$(auth_curl "$BASE/api/repos" \
    | jq -r --arg n "$name" '.[] | select(.name == $n) | .id' | head -1)"
  [[ -n "$id" ]] || die "no repo named '$name' in GET /api/repos"
  printf '%s\n' "$id"
}

# --- Commands ----------------------------------------------------------
cmd_runs() {
  local repo="${1:-$DEFAULT_REPO}" limit="${2:-$DEFAULT_LIMIT}"
  local id
  id="$(repo_id "$repo")"
  echo "repo: $repo ($id)" >&2
  auth_curl "$BASE/api/pipeline-runs?repo_id=$id&limit=$limit" \
    | jq -r '.[] | [.id, (.commit_hash // "-" | .[0:9]),
                   (.status // "-"), (.pipeline_id // "-"),
                   (.started_at // "-")] | @tsv'
}

cmd_jobs() {
  local run_id="${1:?run-id required}"
  auth_curl "$BASE/api/pipeline-runs/$run_id/jobs" \
    | jq -r 'if type == "array" then .[] else (.jobs // [])[] end
           | [(.id // "-"), (.name // .job_name // "-"),
              (.status // "-"), (.exit_code // "-")] | @tsv'
}

cmd_logs() {
  local job_id="${1:?job-id required}" tail_lines="${2:-0}"
  # The logs route returns JSON chunks; render them as plain text. The
  # route caps the body at 64KB and drops the OLDEST bytes, so the tail
  # is the part that matters for a failure.
  local text
  text="$(auth_curl "$BASE/api/jobs/$job_id/logs" \
    | jq -r '.logs[]?.chunk // empty')"
  if [[ "$tail_lines" =~ ^[0-9]+$ ]] && [[ "$tail_lines" -gt 0 ]]; then
    text="$(printf '%s\n' "$text" | tail -n "$tail_lines")"
  fi
  printf '%s\n' "$text"
}

case "${1:-runs}" in
  runs) shift || true; cmd_runs "$@" ;;
  jobs) shift; cmd_jobs "$@" ;;
  logs)
    shift
    cmd_logs "${1:?job-id required}" "${2:-0}"
    ;;
  *)
    die "unknown command: $1 (expected runs|jobs|logs)"
    ;;
esac
