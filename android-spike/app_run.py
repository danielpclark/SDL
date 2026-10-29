#!/usr/bin/env python3
"""Run the spike app once (the prototype of android_run.py's app mode).

app_run.py MODE [--port N] [--timeout S] [--no-stop] [--pkg P]

Listens on 127.0.0.1:N and N+1, `adb reverse`s both, starts the activity
with `--es mode MODE --ei port N`, prints what arrives on the app's stdout
and stderr (fds 1 and 2, connected by the app), runs the app's
`spike-request: adb ...` lines (and `spike-request-bg:` in the
background), and ends on EOF or the timeout. Prints a RESULT line with the
sentinel and the timings, then the logcat lines of interest.
"""

import argparse
import shlex
import socket
import subprocess
import sys
import threading
import time

ap = argparse.ArgumentParser()
ap.add_argument("mode")
ap.add_argument("--port", type=int, default=38100)
ap.add_argument("--timeout", type=float, default=90)
ap.add_argument("--pkg", default="org.libsdl.sdl3_rust.spike")
ap.add_argument("--logcat", default="spike:V AndroidRuntime:E DEBUG:V libc:V art:W ActivityManager:W ActivityTaskManager:I *:F")
a = ap.parse_args()

t0 = time.time()
lock = threading.Lock()
lines = {"out": [], "err": []}
first = {}
bg = []


def ts():
    return time.time() - t0


def adb(*args, check=False):
    return subprocess.run(["adb"] + list(args), capture_output=True, text=True, timeout=120)


def handle_request(line, background):
    cmd = shlex.split(line)
    if cmd and cmd[0] == "adb":
        cmd = cmd[1:]
    t = time.time()
    if background:
        p = subprocess.Popen(["adb"] + cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        bg.append((cmd, p))
        print("  [host %.2fs] started in background: adb %s" % (ts(), " ".join(cmd)), flush=True)
        return
    r = subprocess.run(["adb"] + cmd, capture_output=True, text=True, timeout=60)
    print("  [host %.2fs] adb %s -> rc=%d %.2fs %s" % (ts(), " ".join(cmd), r.returncode, time.time() - t,
                                                   (r.stdout + r.stderr).strip()[-300:]), flush=True)


def reader(conn, name):
    buf = b""
    while True:
        try:
            d = conn.recv(65536)
        except OSError as e:
            print("  [%s] recv error %s" % (name, e), flush=True)
            break
        if not d:
            break
        first.setdefault(name, ts())
        buf += d
        while b"\n" in buf:
            l, buf = buf.split(b"\n", 1)
            s = l.decode("utf-8", "replace")
            with lock:
                lines[name].append(s)
                print("  [%s %.2fs] %s" % (name, ts(), s), flush=True)
            if s.startswith("spike-request: "):
                handle_request(s[len("spike-request: "):], False)
            elif s.startswith("spike-request-bg: "):
                handle_request(s[len("spike-request-bg: "):], True)
    if buf:
        s = buf.decode("utf-8", "replace")
        with lock:
            lines[name].append(s)
            print("  [%s %.2fs] %s  (no newline)" % (name, ts(), s), flush=True)
    print("  [%s %.2fs] EOF" % (name, ts()), flush=True)


socks = []
for p in (a.port, a.port + 1):
    s = socket.socket()
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind(("127.0.0.1", p))
    s.listen(1)
    s.settimeout(a.timeout)
    socks.append(s)
    r = adb("reverse", "tcp:%d" % p, "tcp:%d" % p)
    print("adb reverse tcp:%d -> rc=%d %s" % (p, r.returncode, r.stdout.strip()), flush=True)

adb("logcat", "-c")
adb("logcat", "-b", "crash", "-c")
print("=== app mode %s" % a.mode, flush=True)
ts_start = ts()
r = adb("shell", "am", "start", "-W", "-S", "-n", "%s/org.libsdl.app.SDLActivity" % a.pkg, "--es", "mode", a.mode,
        "--ei", "port", str(a.port))
print("am start -W (%.2fs): %s" % (ts() - ts_start, " | ".join(r.stdout.strip().splitlines())), flush=True)

threads = []
for s, name in zip(socks, ("out", "err")):
    def acc(s=s, name=name):
        try:
            c, _ = s.accept()
        except socket.timeout:
            print("  [%s] no connection within the timeout" % name, flush=True)
            return
        print("  [%s %.2fs] connected" % (name, ts()), flush=True)
        reader(c, name)
    th = threading.Thread(target=acc, daemon=True)
    th.start()
    threads.append(th)
deadline = t0 + a.timeout
for th in threads:
    th.join(max(0, deadline - time.time()))
timed_out = any(th.is_alive() for th in threads)
sent = [l for l in lines["out"] if l.startswith("sdl3-android-runner: exit ")]
pid = adb("shell", "pidof", a.pkg).stdout.strip()
print("RESULT mode=%s sentinel=%s timed_out=%s elapsed=%.2fs first_stdout=%s first_stderr=%s out_lines=%d err_lines=%d pid_after=%r"
      % (a.mode, sent, timed_out, ts(), "%.2fs" % first["out"] if "out" in first else None,
         "%.2fs" % first["err"] if "err" in first else None, len(lines["out"]), len(lines["err"]), pid), flush=True)
for cmd, p in bg:
    try:
        o, _ = p.communicate(timeout=30)
    except subprocess.TimeoutExpired:
        p.kill()
        o, _ = p.communicate()
    print("  [host] background adb %s -> rc=%s: %s" % (" ".join(cmd), p.returncode, o.strip()[-500:]), flush=True)
time.sleep(0.5)
lc = adb("logcat", "-d", "-v", "time", "-s", *a.logcat.split())
print("--- logcat (%s), last 80 lines" % a.logcat)
for l in lc.stdout.splitlines()[-80:]:
    print("  | " + l)
cr = adb("logcat", "-d", "-b", "crash")
if cr.stdout.strip():
    print("--- logcat crash buffer, last 40 lines")
    for l in cr.stdout.splitlines()[-40:]:
        print("  | " + l)
adb("shell", "am", "force-stop", a.pkg)
for p in (a.port, a.port + 1):
    adb("reverse", "--remove", "tcp:%d" % p)
sys.exit(0 if sent else 1)
