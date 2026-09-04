#!/usr/bin/env bash
# kyu quality gates — configuration C1 ("local + SQL guard"),
# approved at the Phase 5 gate. Called by check-commit.sh before every
# git commit; a non-zero exit blocks the commit (standing rule 7).
set -euo pipefail

# ── Standing rule 7: a gate that does not predict the build is not a gate ──
# The checks below rewrite files. cargo updates Cargo.lock, formatters
# rewrite sources — and anything rewritten AFTER `git add` is green here
# and absent from the commit. kyu's 1.0.0 commit carried a lock file
# still naming version 0.0.0; the container build refused it one step
# before a release tag, and nothing local had objected. So: fingerprint
# the tree now, compare once the checks are done, and refuse rather than
# report a green run over a tree that moved underneath it.
gate_tree_fingerprint() {
  { git status --porcelain; git diff; } | sha256sum | cut -d' ' -f1
}
gate_tree_before=$(gate_tree_fingerprint)

cd "${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel)}"

if [ -f Cargo.toml ]; then
  cargo fmt --all -- --check
  cargo clippy --all-targets -- -D warnings
  cargo test --all
else
  # Loud, not silent (standing rule 12): before L0 there is no crate yet.
  echo "gates: no Cargo.toml yet (pre-L0) — Rust gates SKIPPED; SQL guard still runs." >&2
fi

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

# W13 · the vendored kp-themes files. Two checks, guarding two risks.
#
# kyu has no npm, so five files from @kp-soft/themes are COPIES here. A copy
# fails in two different ways, and one check cannot see both.

# (a) ARE THEY WHAT WE CLAIM? Verified against the release's own SHA256SUMS,
# recorded in static/KP_THEMES.sha256. This holds offline and does not depend
# on any other directory: it says the five copies are unmodified and are the
# tag named in that file. Catches an edited copy, a truncated copy, and a
# copy taken from a working tree that had drifted past its tag — which is a
# real hazard, because kp-themes' HEAD sits four commits beyond v1.0.0.
if ! (cd static && sha256sum -c KP_THEMES.sha256 >/dev/null 2>&1); then
  {
    echo "gates: a vendored kp-themes file does not match the release checksums."
    (cd static && sha256sum -c KP_THEMES.sha256 2>&1 | grep -v ': OK$')
    echo
    echo "What now: these files are vendored VERBATIM — do not edit them here."
    echo "If kp-themes released a new version, re-copy the five files and"
    echo "refresh the checksums in the same commit:"
    echo "  gh release download <tag> --repo kennypassenier/kp-themes -p SHA256SUMS -O -"
    echo "Then read MIGRATION.md there: a new version may change the markup"
    echo "contract the picker attaches to, which templates/layout.html writes."
  } >&2
  exit 1
fi

# (b) HAS THE PACKAGE MOVED ON? The checksums above can never notice a new
# version — they are pinned to the one kyu vendored, and stay green forever
# while the package advances without us. That is the risk the whole exercise
# started from, so it gets its own check: compare against the sibling
# repository when it is on this machine, which is where all of Kenny's work
# happens. It is a NOTICE, not a refusal: being behind a release is a
# decision to make, not a broken commit.
KYU_THEME_UPSTREAM=${KYU_THEME_UPSTREAM:-$HOME/Projects/kp-themes}
if [ -d "$KYU_THEME_UPSTREAM" ]; then
  behind=""
  for pair in "css/themes.css:themes.css" \
              "css/components.css:components.css" \
              "js/theme-core.js:theme-core.js" \
              "js/theme-picker.js:theme-picker.js" \
              "js/theme-registry.js:theme-registry.js"; do
    upstream="$KYU_THEME_UPSTREAM/${pair%%:*}"
    [ -f "$upstream" ] || { behind="$behind  ${pair%%:*} no longer exists upstream\n"; continue; }
    diff -q "static/${pair##*:}" "$upstream" >/dev/null 2>&1 || behind="$behind  ${pair%%:*}\n"
  done
  if [ -n "$behind" ]; then
    {
      echo "gates: NOTICE — kp-themes has moved on since kyu vendored v1.0.0."
      printf "%b" "$behind"
      echo "  (the commit is not blocked; check MIGRATION.md and decide)"
    } >&2
  fi
else
  echo "gates: kp-themes is not on this machine ($KYU_THEME_UPSTREAM), so the" >&2
  echo "       'has it moved on' check did not run. The checksums above did." >&2
fi

# Standing rule 7, second clause: see gate_tree_fingerprint above.
if [ "$(gate_tree_fingerprint)" != "$gate_tree_before" ]; then
  {
    echo "gates: the checks rewrote the working tree while they ran."
    echo "A file changed after it was staged, so what this commit carries is"
    echo "NOT what was just tested. Most often this is cargo refreshing"
    echo "Cargo.lock; the changed paths are listed below."
    echo
    git status --porcelain
    echo
    echo "What now: run 'git add -A' and commit again."
  } >&2
  exit 1
fi
