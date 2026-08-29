#!/usr/bin/env bash
# mailbox quality gates — configuration C1 ("local + SQL guard"),
# approved at the Phase 5 gate. Called by check-commit.sh before every
# git commit; a non-zero exit blocks the commit (standing rule 7).
set -euo pipefail

# ── Standing rule 7: a gate that does not predict the build is not a gate ──
# The checks below rewrite files. cargo updates Cargo.lock, formatters
# rewrite sources — and anything rewritten AFTER `git add` is green here
# and absent from the commit. mailbox's 1.0.0 commit carried a lock file
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
