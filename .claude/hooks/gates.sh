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

# W13 · the vendored theme stylesheet must still match kp-themes.
#
# static/themes.css is a COPY of @kp-soft/themes css/themes.css, because kyu
# has no npm and no build step. A copy goes stale in silence: the colours
# still work, the page still renders, and nothing anywhere says the shared
# package moved on. That is the same shape as the backup that failed for two
# nights while its timer reported it had fired (F179).
#
# So: whenever the sibling repository is on this machine — which is where all
# of Kenny's work happens — compare the two and refuse the commit if they
# differ. Everything above the marker line is kyu's own provenance note and
# is not part of the comparison.
#
# On CI the sibling does not exist. The check says so out loud rather than
# passing quietly, because a check that silently does nothing is worse than
# no check at all (standing rule 12).
KYU_THEME_UPSTREAM=${KYU_THEME_UPSTREAM:-$HOME/Projects/kp-themes/css/themes.css}
if [ -f "$KYU_THEME_UPSTREAM" ]; then
  # Cut everything up to the marker, then drop the blank line that
  # separates the note from the copy: whitespace between the two must not
  # be able to report a difference that is not there.
  vendored=$(sed '1,/end of kyu.s provenance note/d' static/themes.css | sed '/./,$!d')
  if ! printf '%s\n' "$vendored" | diff -q - "$KYU_THEME_UPSTREAM" >/dev/null 2>&1; then
    {
      echo "gates: static/themes.css no longer matches kp-themes."
      echo
      printf '%s\n' "$vendored" | diff - "$KYU_THEME_UPSTREAM" | head -20
      echo
      echo "What now: if kp-themes released a new version, re-copy it and update"
      echo "the version and commit in the provenance note at the top:"
      echo "  { head -n \"\$(grep -n 'end of kyu.s provenance note' static/themes.css | cut -d: -f1)\" static/themes.css; cat $KYU_THEME_UPSTREAM; } > /tmp/themes.css && mv /tmp/themes.css static/themes.css"
      echo "If instead someone edited kyu's copy: don't. The note at the top says why."
    } >&2
    exit 1
  fi
else
  echo "gates: kp-themes is not on this machine ($KYU_THEME_UPSTREAM), so the" >&2
  echo "       vendored static/themes.css was NOT compared against it." >&2
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
