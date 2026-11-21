#!/usr/bin/env bash
# Commit and push one change to the Homebrew tap (langstroth-ai/homebrew-ligus)
# without racing the other jobs that write to it.
#
#   scripts/tap-push.sh <tap-clone-dir> <commit-message> <apply-cmd> [args...]
#
# <apply-cmd> runs inside the tap clone and must write the wanted state of
# its file(s) idempotently (running it twice changes nothing). The script then
# commits only if the tree changed, and pushes. When the push is rejected
# because another job pushed first (the ligus repo's release also bumps its
# formula and cask on this tap), it pulls with --rebase, re-runs <apply-cmd>
# to re-check the file still needs the change, and pushes again: at most
# TAP_PUSH_ATTEMPTS (default 5) pushes, sleeping TAP_PUSH_BACKOFF*attempt
# seconds (default 3) between them. A tap that already has the change gets
# no commit (no empty or duplicate commits).
#
# Copied from the ligus repo (scripts/tap-push.sh); keep the two in step.
#
# The clone's remote may carry a token in its URL: this script never prints
# the remote URL and never runs with xtrace.
set -euo pipefail

if [ $# -lt 3 ]; then
  echo "usage: $0 <tap-clone-dir> <commit-message> <apply-cmd> [args...]" >&2
  exit 2
fi
dir=$1
msg=$2
shift 2
attempts=${TAP_PUSH_ATTEMPTS:-5}
backoff=${TAP_PUSH_BACKOFF:-3}

cd "$dir"
branch=$(git symbolic-ref --short HEAD)
# The bot identity. Set on the clone (not just the commit) so a rebase can
# rewrite the commit too.
git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"

for ((attempt = 1; ; attempt++)); do
  "$@"
  git add -A
  if ! git diff --cached --quiet; then
    git commit -q -m "$msg"
  fi
  if [ "$(git rev-list --count "origin/${branch}..HEAD")" -eq 0 ]; then
    echo "Tap already up to date (${msg}): nothing to push."
    exit 0
  fi
  if git push -q origin "HEAD:${branch}"; then
    echo "Pushed '${msg}' to the tap (attempt ${attempt})."
    exit 0
  fi
  if ((attempt >= attempts)); then
    echo "::error::tap push still rejected after ${attempts} attempts"
    exit 1
  fi
  echo "Tap push rejected (attempt ${attempt}); rebasing onto the tap and retrying."
  sleep $((backoff * attempt))
  # A rebase drops our commit when the tap already has the same change; on a
  # conflict, start from the tap's tip and let <apply-cmd> redo the change.
  if ! git pull -q --rebase origin "$branch"; then
    git rebase --abort 2>/dev/null || true
    git fetch -q origin "$branch"
    git reset -q --hard "origin/${branch}"
  fi
done
