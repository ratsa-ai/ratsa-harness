#!/usr/bin/env python3
"""Pretty-print the RATSA Agentic Score board (and optionally one device)."""
import json
import sys
import urllib.request

TOKEN_FILE = "/tmp/ratsa-tok"
BASE = "http://localhost:8282"


def get(path, token):
    req = urllib.request.Request(BASE + path)
    if token:
        req.add_header("Authorization", "Bearer " + token)
    with urllib.request.urlopen(req) as r:
        return json.load(r)


def main():
    token = open(TOKEN_FILE).read().strip() if len(sys.argv) < 2 else sys.argv[1]
    path = sys.argv[2] if len(sys.argv) > 2 else "/api/agentic/board?sort=score"
    d = get(path, token)
    if "spec" in d:
        w = d["spec"]["weights"]
        print("weights: static %s / measured %s / eval %s" % (w["static"], w["measured"], w["eval"]))
    for r in d.get("items", []):
        ev = r["evidence"]
        comps = " ".join("%s=%s" % (c["id"], c["score"]) for c in r["components"])
        print("#%-2s %3s %-2s %-24s pulls=%-3s measured=%-5s eval=%-5s  [%s]" % (
            r.get("rank", "-"), r["score"], r["grade"], r["name"][:22],
            r["pulls"], ev["measured"], ev["eval"], comps))


if __name__ == "__main__":
    main()
