#!/usr/bin/env bash
# kyu's own gates (chassis 1.6.0, M1): run by the kit's gates.sh and CI after
# fmt, clippy and the tests. Project-owned; `chassis sync` never touches it.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

# AR11: every statement must be parameterized. Refuse string-built SQL.
# Escape hatch for genuine cases: append `gates:allow-sql` on the line.
if [ -d src ]; then
  hits=$(grep -rnE '(format!|write!|writeln!|push_str)[^\n]*\b(SELECT|INSERT|UPDATE|DELETE|WHERE|VALUES)\b' src/ \
    | grep -v 'gates:allow-sql' || true)
  if [ -n "$hits" ]; then
    {
      echo "gates: string-built SQL detected (AR11 — use parameterized statements)."
      echo "$hits"
      echo "If a case is genuinely safe, mark that line with: gates:allow-sql"
    } >&2
    exit 1
  fi
fi

# The container smoke (every verb through the door, in the built image) is a
# release gate: it needs docker and minutes, so a local commit skips it,
# loudly. `chassis release` (chassis-rs 3.0.0) sets CHASSIS_RELEASE_GATE=1
# when its gate runs this file — the local stand-in for the CI=true the
# GitHub runner exported until 2026-09-29, which still counts. The kit's
# image check builds its own image, so this one builds the image it smokes.
if { [ "${CHASSIS_RELEASE_GATE:-}" = "1" ] || [ "${CI:-}" = "true" ]; } && [ -x scripts/container-smoke.sh ]; then
  docker build -q -t kyu:smoke . >/dev/null
  scripts/container-smoke.sh kyu:smoke
else
  echo "gates.project: container smoke skipped outside the release gate (chassis release runs it; CHASSIS_RELEASE_GATE=1 by hand)" >&2
fi
