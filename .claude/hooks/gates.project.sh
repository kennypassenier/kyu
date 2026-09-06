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
# CI gate: it needs docker and minutes, so a local commit skips it, loudly.
# The kit's image job builds its own image in another job, so this one
# builds the image it smokes.
if [ "${CI:-}" = "true" ] && [ -x scripts/container-smoke.sh ]; then
  docker build -q -t kyu:smoke . >/dev/null
  scripts/container-smoke.sh kyu:smoke
else
  echo "gates.project: container smoke skipped outside CI (scripts/container-smoke.sh runs there)" >&2
fi
