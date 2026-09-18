#!/usr/bin/env bash
#
# One-time setup: make git run .github/hooks/pre-push before every push.
#
# The change is repo-local (core.hooksPath, not --global), idempotent, and
# can be undone with:
#   git config --unset core.hooksPath

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
HOOKS_PATH=".github/hooks"

HOOK_FILE="$REPO_ROOT/$HOOKS_PATH/pre-push"
if [[ -x "$HOOK_FILE" ]]; then
  :
else
  printf 'error: %s is missing or not executable (run: chmod +x %s)\n' "$HOOK_FILE" "$HOOK_FILE" >&2
  exit 1
fi

git config core.hooksPath "$HOOKS_PATH"

printf 'Installed: git will run %s/pre-push on every `git push`.\n' "$HOOKS_PATH"
printf 'To disable: git config --unset core.hooksPath\n'