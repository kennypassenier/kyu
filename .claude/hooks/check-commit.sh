#!/usr/bin/env bash
# Dev-procedure commit gate (option B): a PreToolUse hook on the Bash
# tool. Blocks `git commit` unless (1) the project's gates pass and
# (2) the commit message carries feature/milestone IDs in brackets.
#
# Contract: Claude Code pipes the tool call as JSON on stdin. Exit 0
# allows the command; exit 2 blocks it and feeds stderr back to Claude.
# Parse failures fail OPEN (exit 0) so a broken hook never bricks work.
set -u

payload=$(cat) || exit 0
cmd=$(printf '%s' "$payload" | python3 -c '
import json,sys
try:
    print(json.load(sys.stdin).get("tool_input", {}).get("command", ""))
except Exception:
    pass
' 2>/dev/null) || exit 0

case "$cmd" in
  *"git commit"*) ;;
  *) exit 0 ;;
esac

project_dir="${CLAUDE_PROJECT_DIR:-$PWD}"

# R2 (latch v2 retro, 2026-08-29): gates must run against the tree the
# commit actually targets. A commit issued from a git worktree (e.g.
# `cd <worktree> && git commit`) used to be gated against
# CLAUDE_PROJECT_DIR — the MAIN checkout — silently approving code that
# was not the code being committed. Resolve the target repo root from a
# literal `cd <path>` or `git -C <path>` in the command; variable
# indirection falls back to the project dir, so standing rule 19 asks
# worktree commits to spell the path out literally.
target=$(printf '%s' "$cmd" | python3 -c '
import re, sys
cmd = sys.stdin.read()
m = re.search(r"git\s+-C\s+(\"[^\"]+\"|\x27[^\x27]+\x27|[^\s;&|]+)", cmd)
if not m:
    m = re.search(r"(?:^|[;&|]\s*)cd\s+(\"[^\"]+\"|\x27[^\x27]+\x27|[^\s;&|]+)", cmd)
print(m.group(1).strip("\"\x27") if m else "")
' 2>/dev/null) || target=""
if [ -n "$target" ]; then
  case "$target" in
    "~"*) target="$HOME${target#\~}" ;;
    '$HOME'*) target="$HOME${target#\$HOME}" ;;
  esac
  root=$(git -C "$target" rev-parse --show-toplevel 2>/dev/null || true)
  [ -n "$root" ] && project_dir="$root"
fi

# Gate 1: the project's own quality gates (fmt/lint/tests). The project
# defines what that means in .claude/hooks/gates.sh (see gates.example.sh).
gates="$project_dir/.claude/hooks/gates.sh"
if [ -x "$gates" ]; then
  if ! out=$(cd "$project_dir" && "$gates" 2>&1); then
    {
      echo "COMMIT BLOCKED — gates failed (standing rule 7). Fix, then retry."
      echo "$out" | tail -30
    } >&2
    exit 2
  fi
fi

# Gate 2: traceability (standing rule 4). The message must contain IDs
# in brackets, e.g. [W12, AR9] or [L4b] — or [meta] for infra commits.
if ! printf '%s' "$cmd" | grep -qE '\[(meta|[A-Za-z]{1,4}[0-9])[^]]*\]'; then
  {
    echo "COMMIT BLOCKED — message lacks feature/milestone IDs (standing rule 4)."
    echo "Add the IDs this commit implements, e.g.: feat(L4b): groups [W12a-d, AR9]"
    echo "Pure infrastructure commits use [meta]."
  } >&2
  exit 2
fi

exit 0
