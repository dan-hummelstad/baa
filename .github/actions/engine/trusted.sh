#!/usr/bin/env bash
# Exit 0 when the workflow run JSON on stdin (GET repos/{repo}/actions/runs/{id})
# is a manual chromium.yml dispatch in repo $1. Artifact names are guessable and
# any PR run can upload one, so only the producer counts. Not display_title: a
# workflow file sets that itself through run-name.
set -eu
jq -e --arg repo "$1" '
  .path == ".github/workflows/chromium.yml"
  and .event == "workflow_dispatch"
  and .head_repository.full_name == $repo
  and .repository.full_name == $repo' >/dev/null
