#!/usr/bin/env python3
"""The spike's linker wrapper (the prototype of tools/android_link.py).

link.py TRIPLE RUSTC-LINKER-ARGS...: runs the NDK's <triple>21-clang with
rustc's arguments plus -Wl,-z,max-page-size=16384. A program whose own
object files carry the word `app` in a `.sdl3_runner` section is linked
with -shared instead of -pie (+ --defsym=SDL_main=main, JNI_OnLoad kept):
it is then an app's libmain.so. The `probe` program is also linked twice
more, as experiments for the plan's question 4: OUT.pie_exp (a PIE with
`main` exported) and OUT.shared (-shared, SDL_main=main).
"""

import os
import subprocess
import sys
import time

triple = sys.argv[1]
args = sys.argv[2:]
ndk = os.environ.get("ANDROID_NDK_ROOT") or os.environ.get("ANDROID_NDK_HOME")
if not ndk:
    sys.exit("link.py: set ANDROID_NDK_ROOT")
bindir = os.path.join(ndk, "toolchains", "llvm", "prebuilt", "linux-x86_64", "bin")
clang = os.path.join(bindir, "%s21-clang" % triple)
readelf = os.path.join(bindir, "llvm-readelf")
log = os.environ.get("SPIKE_LINK_LOG")


def expand(a):
    out = []
    for x in a:
        if x.startswith("@") and os.path.exists(x[1:]):
            with open(x[1:]) as f:
                out += [l.rstrip("\n") for l in f if l.strip()]
        else:
            out.append(x)
    return out


full = expand(args)
outp = full[full.index("-o") + 1] if "-o" in full else "?"
base = os.path.basename(outp)


def words():
    """The `.sdl3_runner` words in the program's own object files."""
    found = []
    for x in full:
        if x.endswith(".o") and os.path.exists(x):
            r = subprocess.run([readelf, "-p", ".sdl3_runner", x], capture_output=True, text=True)
            if r.returncode == 0 and "String dump" in r.stdout:
                for line in r.stdout.splitlines():
                    if "]" in line:
                        found.append(line.split("]", 1)[1].strip())
    return found


def run(cmd, label):
    t = time.time()
    r = subprocess.run(cmd)
    if log:
        with open(log, "a") as f:
            f.write("%s %s rc=%d %.2fs size=%s\n  %s\n" % (label, base, r.returncode, time.time() - t,
                    os.path.getsize(cmd[cmd.index("-o") + 1]) if r.returncode == 0 else "-",
                    " ".join(a for a in cmd if not a.endswith((".o", ".rlib")))[:1500]))
    return r.returncode


def shared(cmd):
    c = [a for a in cmd if a not in ("-pie", "-Wl,-pie")]
    return c + ["-shared", "-Wl,--defsym=SDL_main=main", "-Wl,-u,JNI_OnLoad", "-Wl,--export-dynamic-symbol=JNI_OnLoad",
                "-Wl,--export-dynamic-symbol=SDL_main"]


cmd = [clang] + args + ["-Wl,-z,max-page-size=16384"]
w = words() if "-shared" not in args else []
if "app" in w:
    rc = run(shared(cmd), "link[app->-shared words=%s]" % w)
    sys.exit(rc)
rc = run(cmd, "link[exe]")
if rc == 0 and base.startswith("probe-"):
    def other(suffix):
        c = list(cmd)
        c[c.index("-o") + 1] = outp + suffix
        return c
    run(other(".pie_exp") + ["-Wl,--export-dynamic-symbol=main"], "link[probe pie+export main]")
    run(shared(other(".shared")), "link[probe -shared]")
sys.exit(rc)
