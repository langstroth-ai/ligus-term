#!/usr/bin/env python3
"""Check that a draft release holds every asset before it is published.

    scripts/check-release-assets.py --repo owner/repo --release-id ID --tag vX.Y.Z
    scripts/check-release-assets.py --from-dir DIR --tag vX.Y.Z

release.yml's publish job runs this on the draft after every build has
succeeded. The `needs:` graph already guarantees the jobs passed; this checks
what they left on THIS release, so a job that "succeeded" without uploading
(or uploaded elsewhere) cannot get a release published. It fails unless:

  - every target in TARGETS has lt-<target> and lt-<target>.sha256, the
    .sha256 names that file, and its hash equals the binary's digest as
    GitHub recorded it (when GitHub reports one),
  - every asset finished uploading (state "uploaded").

--from-dir reads assets.json ([{name, state, digest}]) and the .sha256 files
from DIR instead of the GitHub API (used by the tests).

Copied from the ligus repo's release pipeline, without the desktop assets.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile

# Must match release.yml's build matrix, ci.yml's targets matrix and the
# formula job in release.yml (scripts/test-release-workflow.rb checks all four).
TARGETS = (
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
)
TAG_RE = re.compile(r"^v(\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?)$")


def check(assets: list[dict], sums: dict[str, str], tag: str) -> list[str]:
    m = TAG_RE.match(tag)
    if not m:
        return [f"malformed tag {tag!r}"]
    by_name = {a["name"]: a for a in assets}
    errors = []
    for a in assets:
        if a.get("state", "uploaded") != "uploaded":
            errors.append(f"{a['name']}: upload not finished (state {a.get('state')!r})")
    for t in TARGETS:
        binary, sumfile = f"lt-{t}", f"lt-{t}.sha256"
        if binary not in by_name:
            errors.append(f"{binary}: missing from the release")
            continue
        if sumfile not in by_name:
            errors.append(f"{sumfile}: missing from the release")
            continue
        text = sums.get(sumfile, "")
        parts = text.split()
        if len(parts) != 2 or not re.fullmatch(r"[0-9a-f]{64}", parts[0]) or parts[1].lstrip("*") != binary:
            errors.append(f"{sumfile}: not a sha256 line for {binary}: {text.strip()!r}")
            continue
        digest = by_name[binary].get("digest") or ""
        if digest.startswith("sha256:") and digest[len("sha256:"):] != parts[0]:
            errors.append(f"{binary}: GitHub's digest {digest} differs from {sumfile}")
    return errors


def gh(*args: str) -> bytes:
    return subprocess.run(["gh", *args], check=True, capture_output=True).stdout


def fetch(repo: str, release_id: str, out: str) -> None:
    assets = json.loads(gh("api", "--paginate", "--slurp", f"repos/{repo}/releases/{release_id}/assets?per_page=100").decode() or "[]")
    assets = [{"id": a["id"], "name": a["name"], "state": a.get("state"), "digest": a.get("digest")}
              for page in assets for a in page]
    with open(os.path.join(out, "assets.json"), "w") as f:
        json.dump(assets, f)
    for a in assets:
        if a["name"].endswith(".sha256"):
            data = gh("api", "-H", "Accept: application/octet-stream", f"repos/{repo}/releases/assets/{a['id']}")
            with open(os.path.join(out, os.path.basename(a["name"])), "wb") as f:
                f.write(data)


def load_dir(d: str):
    with open(os.path.join(d, "assets.json")) as f:
        assets = json.load(f)
    sums = {}
    for a in assets:
        n = os.path.basename(a["name"])
        p = os.path.join(d, n)
        if n.endswith(".sha256") and os.path.exists(p):
            with open(p) as f:
                sums[n] = f.read()
    return assets, sums


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo")
    ap.add_argument("--release-id")
    ap.add_argument("--from-dir")
    ap.add_argument("--tag", required=True)
    a = ap.parse_args()
    if a.from_dir:
        assets, sums = load_dir(a.from_dir)
    else:
        if not (a.repo and a.release_id):
            print("--repo and --release-id are required without --from-dir", file=sys.stderr)
            return 2
        with tempfile.TemporaryDirectory() as d:
            fetch(a.repo, a.release_id, d)
            assets, sums = load_dir(d)
    errors = check(assets, sums, a.tag)
    for e in errors:
        print(f"::error::release assets: {e}")
    if not errors:
        print(f"release assets OK for {a.tag}: {len(assets)} assets, lt for {', '.join(TARGETS)}")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
