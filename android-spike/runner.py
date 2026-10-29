#!/usr/bin/env python3
"""A throwaway cargo runner for x86_64-linux-android (the prototype of
android_run.py's spawn mode).

runner.py EXE ARGS...: adb push EXE to /data/local/tmp/sdl3/bin/, then
`adb shell 'cd RUN && VAR=... exec BIN ARGS'` (shell protocol: rc and the
two streams pass through). Forwards SDL3_*, SDL_*, RUST_* and
RUSTC_BOOTSTRAP; sets SDL_ASSERT=abort, HOME and TMPDIR.

With SPIKE_TALLY_DIR set, a libtest program is instead run with libtest's
JSON events, skipping the tests that crash or hang the process and running
again, and its tally is written to SPIKE_TALLY_DIR/NAME.json.
"""

import hashlib
import json
import os
import queue
import re
import shlex
import subprocess
import sys
import threading
import time

ROOT = "/data/local/tmp/sdl3"
exe, args = sys.argv[1], sys.argv[2:]
tally_dir = os.environ.get("SPIKE_TALLY_DIR")
data = open(exe, "rb").read()
is_libtest = b"RUST_TEST_THREADS" in data
base = os.path.basename(exe)
if base == "rust_out" or "rustdoctest" in exe:
    base = "doctest-" + hashlib.sha1(exe.encode()).hexdigest()[:10]
dev = "%s/bin/%s" % (ROOT, base)


def log(msg):
    if tally_dir:
        with open(os.path.join(tally_dir, "runner.log"), "a") as f:
            f.write(msg + "\n")


t = time.time()
subprocess.run(["adb", "shell", "mkdir -p %s/bin %s/run %s/tmp %s/home" % (ROOT, ROOT, ROOT, ROOT)], check=True)
r = subprocess.run(["adb", "push", exe, dev], capture_output=True, text=True)
push_s = time.time() - t
if r.returncode:
    sys.stderr.write("runner: adb push failed: %s\n" % (r.stdout + r.stderr))
    sys.exit(125)
log("PUSH %s %d bytes %.2fs %.1f MB/s" % (base, len(data), push_s, len(data) / 1e6 / max(push_s, 1e-3)))

env = {}
for k, v in os.environ.items():
    if re.match(r"^(SDL3_|SDL_|RUST_|RUSTC_BOOTSTRAP$)", k):
        env[k] = v
env.setdefault("SDL_ASSERT", "abort")
if tally_dir:
    # test_support::skip's log, on the device, collected after each program
    env["SDL3_TEST_SKIP_LOG"] = ROOT + "/skips.log"
env["HOME"] = ROOT + "/home"
env["TMPDIR"] = ROOT + "/tmp"


def shell_line(extra_args, extra_env=None):
    e = dict(env, **(extra_env or {}))
    pre = " ".join("%s=%s" % (k, shlex.quote(v)) for k, v in sorted(e.items()))
    return "cd %s/run && %s exec %s %s" % (ROOT, pre, dev, " ".join(shlex.quote(a) for a in extra_args))


def cleanup():
    subprocess.run(["adb", "shell", "rm", "-f", dev])
    if tally_dir:
        r = subprocess.run(["adb", "shell", "cat %s/skips.log 2>/dev/null; rm -f %s/skips.log" % (ROOT, ROOT)],
                           capture_output=True, text=True)
        if r.stdout.strip():
            with open(os.path.join(tally_dir, "skips.tsv"), "a") as f:
                for line in r.stdout.splitlines():
                    f.write("%s\t%s\n" % (base, line))


if not (tally_dir and is_libtest):
    t = time.time()
    rc = subprocess.run(["adb", "shell", shell_line(args)]).returncode
    log("RUN %s rc=%d %.2fs args=%s" % (base, rc, time.time() - t, args))
    cleanup()
    sys.exit(rc)

# ---- libtest with JSON events, skipping crashing/hanging tests
INACTIVITY = float(os.environ.get("SPIKE_TEST_TIMEOUT", "240"))
DEADLINE = time.time() + float(os.environ.get("SPIKE_BIN_DEADLINE", "1800"))
jenv = {"RUSTC_BOOTSTRAP": "1", "RUST_BACKTRACE": "0"}
env.pop("RUST_TEST_NOCAPTURE", None)
res = {"binary": base, "host_path": exe, "size": len(data), "push_s": round(push_s, 2), "runs": [],
       "crash": {}, "hang": [], "solo": {}}
outcome = {}  # test -> (event, stdout); the last run's outcome wins


def run_once(skips, exact=None):
    a = ["-Zunstable-options", "--format", "json"]
    if exact:
        a += ["--exact", exact]
    else:
        a += ["--exact"] if skips else []
        for s in skips:
            a += ["--skip", s]
    p = subprocess.Popen(["adb", "shell", shell_line(a, jenv)], stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                         text=True, errors="replace")
    q = queue.Queue()

    def rd():
        for line in p.stdout:
            q.put(line.rstrip("\n"))
        q.put(None)
    threading.Thread(target=rd, daemon=True).start()
    started, done, other, suite = {}, {}, [], None
    last = time.time()
    hung = False
    while True:
        try:
            line = q.get(timeout=5)
        except queue.Empty:
            if time.time() - last > INACTIVITY or time.time() > DEADLINE:
                hung = True
                p.kill()
                subprocess.run(["adb", "shell", "pkill", "-9", "-f", dev], capture_output=True)
                break
            continue
        if line is None:
            break
        last = time.time()
        try:
            ev = json.loads(line)
        except ValueError:
            other.append(line)
            continue
        if ev.get("type") == "test":
            n, e = ev.get("name"), ev.get("event")
            if e == "started":
                started[n] = time.time()
            elif e in ("ok", "failed", "ignored", "timeout"):
                done[n] = (e, (ev.get("stdout") or "")[-3000:])
        elif ev.get("type") == "suite" and ev.get("event") in ("ok", "failed"):
            suite = ev
    rc = p.wait()
    return rc, started, done, other, suite, hung


skips = []
t0 = time.time()
while time.time() < DEADLINE and len(res["runs"]) < 40:
    rc, started, done, other, suite, hung = run_once(skips)
    outcome.update(done)
    sus = [n for n in started if n not in done]
    res["runs"].append({"rc": rc, "done": len(done), "suspects": sus, "hung": hung, "tail": other[-30:]})
    if suite is not None or not sus:
        break
    if hung:
        res["hang"] += sus
    else:
        # classify each suspect alone
        for s in sus:
            rc1, st1, dn1, ot1, su1, hg1 = run_once([], exact=s)
            if s in dn1:
                res["solo"][s] = dn1[s][0]
                outcome[s] = dn1[s]
            elif hg1:
                res["hang"].append(s)
            else:
                res["crash"][s] = {"rc": rc1, "tail": ot1[-25:]}
    skips += sus
for s in list(res["crash"]) + res["hang"]:
    outcome.pop(s, None)
res["ok"] = sorted(n for n, (e, _) in outcome.items() if e == "ok")
res["ignored"] = sorted(n for n, (e, _) in outcome.items() if e == "ignored")
res["failed"] = {n: out for n, (e, out) in outcome.items() if e not in ("ok", "ignored")}
res["elapsed"] = round(time.time() - t0, 1)
with open(os.path.join(tally_dir, base + ".json"), "w") as f:
    json.dump(res, f, indent=1)
line = "SPIKE-TALLY %s ok=%d failed=%d ignored=%d crash=%d hang=%d runs=%d elapsed=%.1fs size=%.1fMB push=%.1fs" % (
    base, len(res["ok"]), len(res["failed"]), len(res["ignored"]), len(res["crash"]), len(res["hang"]), len(res["runs"]),
    res["elapsed"], len(data) / 1e6, push_s)
print(line, flush=True)
log(line)
for n in sorted(res["failed"]):
    print("  FAILED %s" % n)
for n in sorted(res["crash"]):
    print("  CRASH %s" % n)
for n in res["hang"]:
    print("  HANG %s" % n)
cleanup()
sys.exit(0 if not (res["failed"] or res["crash"] or res["hang"]) else 101)
