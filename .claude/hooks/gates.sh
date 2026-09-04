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

# W13 · the vendored kp-themes files must still match the package.
#
# kyu has no npm, so five files from @kp-soft/themes are COPIES here. A copy
# goes stale in silence: the colours still work, the page still renders, and
# nothing says the package moved on. That is the same shape as the backup
# that failed for two nights while its timer reported it had fired (F179).
#
# Whenever the sibling repository is on this machine — which is where all of
# Kenny's work happens — compare every copy byte for byte. The files are
# vendored UNMODIFIED for exactly this reason: no header to skip, no marker
# to get wrong, just diff.
#
# On CI the sibling does not exist. The check says so out loud rather than
# passing quietly, because a check that silently does nothing is worse than
# no check at all (standing rule 12).
KYU_THEME_UPSTREAM=${KYU_THEME_UPSTREAM:-$HOME/Projects/kp-themes}
if [ -d "$KYU_THEME_UPSTREAM" ]; then
  drift=""
  for pair in "css/themes.css:themes.css" \
              "css/components.css:components.css" \
              "js/theme-core.js:theme-core.js" \
              "js/theme-picker.js:theme-picker.js" \
              "js/theme-registry.js:theme-registry.js"; do
    upstream="$KYU_THEME_UPSTREAM/${pair%%:*}"
    ours="static/${pair##*:}"
    if [ ! -f "$upstream" ]; then
      drift="$drift  $upstream no longer exists in kp-themes\n"
    elif ! diff -q "$ours" "$upstream" >/dev/null 2>&1; then
      drift="$drift  $ours differs from ${pair%%:*}\n"
    fi
  done
  if [ -n "$drift" ]; then
    {
      echo "gates: the vendored kp-themes files no longer match the package."
      printf "%b" "$drift"
      echo
      echo "What now: if kp-themes released a new version, re-copy the five"
      echo "files and update the version named in src/http/handlers.rs:"
      echo "  for f in css/themes.css css/components.css js/theme-core.js \\"
      echo "           js/theme-picker.js js/theme-registry.js; do"
      echo "    cp \"$KYU_THEME_UPSTREAM/\$f\" \"static/\$(basename \"\$f\")\"; done"
      echo "Then check MIGRATION.md there: a new version may change the markup"
      echo "contract the picker attaches to, which templates/layout.html writes."
      echo "If instead someone edited kyu's copy: don't. They are vendored."
    } >&2
    exit 1
  fi
else
  echo "gates: kp-themes is not on this machine ($KYU_THEME_UPSTREAM), so the" >&2
  echo "       five vendored files were NOT compared against it." >&2
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
