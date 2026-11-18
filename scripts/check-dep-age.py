#!/usr/bin/env python3
"""List crates.io packages in Cargo.lock released fewer than N days ago.

Usage: scripts/check-dep-age.py [days=7]
Exit 1 if any are too new. Pin them back with
  cargo update -p <name>@<version> --precise <older>
"""
import datetime, json, re, sys, urllib.request

days = int(sys.argv[1]) if len(sys.argv) > 1 else 7
cutoff = datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(days=days)
# Plain parsing so it runs on any Python 3 (no tomllib before 3.11).
text = open("Cargo.lock").read()
pkgs = re.findall(r'name = "([^"]+)"\nversion = "([^"]+)"\nsource = "registry\+', text)
bad = []
for name, version in pkgs:
    p = {"name": name, "version": version}
    url = f"https://crates.io/api/v1/crates/{p['name']}/{p['version']}"
    req = urllib.request.Request(url, headers={"User-Agent": "ligus-term dep-age check"})
    v = json.load(urllib.request.urlopen(req))["version"]
    created = datetime.datetime.fromisoformat(v["created_at"].replace("Z", "+00:00"))
    if created > cutoff:
        bad.append((p["name"], p["version"], v["created_at"][:10]))
for b in bad:
    print("too new: %s %s (%s)" % b)
sys.exit(1 if bad else 0)
