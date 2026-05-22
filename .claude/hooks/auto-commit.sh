#!/usr/bin/env bash
# Auto-commit all changes and push to the current branch.
# Invoked by the Claude Code Stop hook. Reads hook JSON on stdin (cwd field).
set -euo pipefail

# Determine the working directory from the hook payload, falling back to PWD.
payload="$(cat 2>/dev/null || true)"
cwd="$(printf '%s' "$payload" | jq -r '.cwd // empty' 2>/dev/null || true)"
[ -n "$cwd" ] && cd "$cwd"

# Bail quietly if this isn't a git repo.
git rev-parse --is-inside-work-tree >/dev/null 2>&1 || exit 0

branch="$(git symbolic-ref --quiet --short HEAD 2>/dev/null || true)"
[ -z "$branch" ] && exit 0  # detached HEAD: don't auto-commit

git add -A

# Commit only if there is something staged.
if ! git diff --cached --quiet; then
  git commit -q -m "chore: auto-commit ($(date '+%Y-%m-%d %H:%M:%S'))" || exit 0
fi

# Push if the local branch is ahead of (or has no) upstream.
git push -u origin "$branch" >/dev/null 2>&1 || true
