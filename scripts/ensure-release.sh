#!/usr/bin/env bash
# Print the id of the tag's DRAFT GitHub release, creating the draft if there
# is none.
#
#   scripts/ensure-release.sh <owner/repo> <tag>
#
# Used by release.yml's `release` job (copied from the ligus repo's release
# pipeline). Every `lt` binary goes into this one draft, and only
# release.yml's `publish` job, once every build has succeeded, publishes it
# (as a prerelease). So a failed build never leaves a half-filled published
# release: nobody sees a draft.
#
# A new draft is created as a prerelease with a placeholder title and body.
# An existing draft for the tag is reused untouched (a rerun). An existing
# PUBLISHED release for the tag is refused: building into it would put new
# assets on a live release. Delete it (and re-push the tag) to re-cut; see
# README.md, "Releasing". Two drafts for one tag are refused too, since
# uploads by tag would not know which one to use.
#
# After publish, the owner writes the notes and promotes it by hand:
#
#   gh release edit <tag> --title "lt <tag>" --notes-file notes.md \
#     --prerelease=false --latest
#
# Needs GH_TOKEN with contents: write (drafts are invisible without it).
set -euo pipefail

repo="${1:?usage: ensure-release.sh <owner/repo> <tag>}"
tag="${2:?usage: ensure-release.sh <owner/repo> <tag>}"
delay="${ENSURE_RELEASE_RETRY_DELAY:-5}"

# The tag reaches gh's jq filter and URLs below; accept only release tags.
if ! [[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]; then
  echo "ensure-release: refusing malformed tag '$tag'" >&2
  exit 2
fi
if ! [[ "$repo" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
  echo "ensure-release: refusing malformed repo '$repo'" >&2
  exit 2
fi

# Prints the draft's id. Exits the script on a published release or on more
# than one draft; returns 1 when there is no release for the tag.
lookup() {
  local id drafts
  # Published releases (incl. prereleases) are found by tag: refuse them.
  if id=$(gh api "repos/$repo/releases/tags/$tag" --jq .id 2>/dev/null) && [ -n "$id" ]; then
    echo "ensure-release: $tag already has a published release ($id); refusing to build into it." >&2
    echo "ensure-release: to re-cut, delete it and re-push the tag (README.md, Releasing)." >&2
    exit 3
  fi
  # ...drafts are not, so scan the list for them.
  drafts=$(gh api --paginate "repos/$repo/releases?per_page=100" \
    --jq ".[] | select(.draft and .tag_name == \"$tag\") | .id") || drafts=""
  case "$(printf '%s' "$drafts" | grep -c . || true)" in
    0) return 1 ;;
    1) echo "$drafts"; return 0 ;;
    *)
      echo "ensure-release: $tag has several drafts ($(tr "\n" " " <<<"$drafts")); delete all but one" >&2
      exit 3 ;;
  esac
}

# The release must sit on the pushed tag. A draft does not create its tag
# until it is published, so a typo'd or unpushed tag would only surface at
# publish time (as a new tag on the default branch): check it now.
if ! gh api "repos/$repo/git/ref/tags/$tag" --silent 2>/dev/null; then
  echo "ensure-release: tag $tag is not on $repo; push it first" >&2
  exit 1
fi

for attempt in 1 2 3 4 5; do
  if lookup; then
    exit 0
  fi
  # Create through the API so the new draft's id comes straight back: the
  # releases list can lag behind a create, and a lookup that missed the new
  # draft would make a second one. draft: nothing is public until the
  # publish job runs. A failed create is followed by another lookup.
  if id=$(gh api -X POST "repos/$repo/releases" -f tag_name="$tag" -F draft=true -F prerelease=true \
      -f name="lt $tag" -f body="Release notes to follow. The lt binaries and their checksums are attached below." \
      --jq .id) && [[ "$id" =~ ^[0-9]+$ ]]; then
    echo "ensure-release: created draft prerelease $tag ($id)" >&2
    echo "$id"
    exit 0
  fi
  echo "ensure-release: create failed (attempt $attempt), looking again" >&2
  sleep "$delay"
done

echo "ensure-release: no release for $tag after 5 attempts" >&2
exit 1
