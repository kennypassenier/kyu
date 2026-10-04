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

# No container smoke any more (Kenny, 2026-10-04): kyu ships as a native
# unit and no image is published, so the release gate builds none. The door
# is covered by the in-process suites; scripts/container-smoke.sh stays for
# a hand-run against an image someone builds on purpose.
