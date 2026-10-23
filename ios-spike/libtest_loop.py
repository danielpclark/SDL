#!/usr/bin/env python3
"""Run a libtest binary under `simctl spawn` with JSON events; when the
process dies (abort/signal), skip the tests that had started but not
finished, and run again. Prints a tally. Args: UDID EXE [DEADLINE_S]."""

import json
import os
import subprocess
import sys
import time

udid, exe = sys.argv[1], sys.argv[2]
deadline = time.monotonic() + float(sys.argv[3] if len(sys.argv) > 3 else 2400)
env = dict(os.environ, SIMCTL_CHILD_RUSTC_BOOTSTRAP="1", SIMCTL_CHILD_RUST_TEST_THREADS="4",
           SIMCTL_CHILD_RUST_BACKTRACE="0")
skips, suspects_all = [], []
failed_all = {}
it = 0
while it < 30 and time.monotonic() < deadline:
    it += 1
    cmd = ["xcrun", "simctl", "spawn", udid, exe, "-Zunstable-options", "--format", "json", "--exact"]
    for s in skips:
        cmd += ["--skip", s]
    t0 = time.monotonic()
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=env, text=True, errors="replace")
    started, finished, other, suite = {}, set(), [], None
    counts = {"ok": 0, "failed": 0, "ignored": 0}
    for line in p.stdout:
        line = line.rstrip("\n")
        try:
            ev = json.loads(line)
        except Exception:
            other.append(line)
            continue
        if ev.get("type") == "test":
            name, e = ev.get("name"), ev.get("event")
            if e == "started":
                started[name] = time.monotonic()
            else:
                finished.add(name)
                if e in counts:
                    counts[e] += 1
                if e == "failed":
                    failed_all[name] = (ev.get("stdout") or "")[-1500:]
        elif ev.get("type") == "suite" and ev.get("event") in ("ok", "failed"):
            suite = ev
    rc = p.wait()
    dt = time.monotonic() - t0
    print("iteration %d: rc=%d %.1fs started=%d ok=%d failed=%d ignored=%d skips=%d"
          % (it, rc, dt, len(started), counts["ok"], counts["failed"], counts["ignored"], len(skips)), flush=True)
    if suite is not None:
        print("suite finished:", json.dumps(suite), flush=True)
        break
    sus = [n for n in started if n not in finished]
    print("process died; tests started but not finished:", sus, flush=True)
    print("  last non-JSON output:", flush=True)
    for l in other[-25:]:
        print("   |", l[:300], flush=True)
    if not sus:
        print("no suspects; stopping", flush=True)
        break
    suspects_all += sus
    skips += sus

print("\n=== TALLY for", os.path.basename(exe))
print("aborting/crashing tests (skipped to continue): %d" % len(suspects_all))
for s in suspects_all:
    print("  CRASH", s)
print("failed tests: %d" % len(failed_all))
for n, out in sorted(failed_all.items()):
    print("  FAIL", n)
    tail = [l for l in out.splitlines() if l.strip()][-6:]
    for l in tail:
        print("     ", l[:240])
