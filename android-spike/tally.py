#!/usr/bin/env python3
"""Summarize runner.py's tally files: tally.py DIR [MAX_LINES_PER_FAILURE]"""

import collections
import glob
import json
import os
import re
import sys

d = sys.argv[1]
nmax = int(sys.argv[2]) if len(sys.argv) > 2 else 4
rows = [json.load(open(p)) for p in sorted(glob.glob(os.path.join(d, "*.json")))]
tot = collections.Counter()
print("=== per binary (ok failed ignored crash hang elapsed size push)")
for r in rows:
    k = dict(ok=len(r["ok"]), failed=len(r["failed"]), ignored=len(r["ignored"]), crash=len(r["crash"]), hang=len(r["hang"]))
    tot.update(k)
    tot["elapsed"] += r["elapsed"]
    tot["push"] += r["push_s"]
    tot["bytes"] += r["size"]
    print("%-50s %5d %4d %4d %3d %3d %7.1fs %7.1fMB %5.1fs" % (r["binary"][:50], k["ok"], k["failed"], k["ignored"], k["crash"],
                                                              k["hang"], r["elapsed"], r["size"] / 1e6, r["push_s"]))
print("TOTAL binaries=%d ok=%d failed=%d ignored=%d crash=%d hang=%d run=%.0fs pushed=%.0fMB in %.0fs (%.1f MB/s)" % (
    len(rows), tot["ok"], tot["failed"], tot["ignored"], tot["crash"], tot["hang"], tot["elapsed"], tot["bytes"] / 1e6,
    tot["push"], tot["bytes"] / 1e6 / max(tot["push"], 1)))


def message(out):
    lines = out.splitlines()
    for i, l in enumerate(lines):
        if "panicked at" in l:
            return " / ".join(x.strip() for x in lines[i:i + nmax] if x.strip())
    tail = [x for x in lines if x.strip()][-nmax:]
    return " / ".join(tail)


def norm(m):
    m = re.sub(r"panicked at [^:]+:\d+:\d+:", "panicked:", m)
    m = re.sub(r"0x[0-9a-f]+|\d+", "N", m)
    m = re.sub(r"/home/runner/[^ '\"]*", "<host path>", m)
    return m[:160]


groups = collections.defaultdict(list)
print("\n=== failures")
for r in rows:
    for n, out in sorted(r["failed"].items()):
        m = message(out)
        print("FAIL %s :: %s :: %s" % (r["binary"], n, m[:600]))
        groups[norm(m)].append("%s::%s" % (r["binary"].split("-")[0], n))
print("\n=== crashes")
for r in rows:
    for n, c in sorted(r["crash"].items()):
        tail = [x for x in c["tail"] if x.strip()][-8:]
        print("CRASH %s :: %s rc=%s :: %s" % (r["binary"], n, c["rc"], " / ".join(tail)[:900]))
        groups["CRASH rc=%s %s" % (c["rc"], norm(" / ".join(tail[-2:])))].append("%s::%s" % (r["binary"].split("-")[0], n))
print("\n=== hangs")
for r in rows:
    for n in r["hang"]:
        print("HANG %s :: %s" % (r["binary"], n))
        groups["HANG"].append("%s::%s" % (r["binary"].split("-")[0], n))
print("\n=== groups by normalized message (count, message, first examples)")
for m, v in sorted(groups.items(), key=lambda kv: -len(kv[1])):
    print("%4d  %s\n      e.g. %s" % (len(v), m, ", ".join(v[:6])))
