#!/usr/bin/env python3
"""Emulator helpers with timings (the prototype of android_run.py --boot).

emu.py create                  make AVD `spike` (4 cores, 4 GB, 12 GB data)
emu.py boot LABEL [ARGS...]    start the emulator (plus ARGS) and wait for
                               sys.boot_completed; prints TIMING lines
emu.py start LABEL [ARGS...]   start only (wait later with `emu.py wait LABEL`)
emu.py wait LABEL
emu.py kill                    adb emu kill, and wait for the process to exit
Env: ANDROID_HOME, SPIKE_IMAGE, RUNNER_TEMP.
"""

import os
import subprocess
import sys
import time

sdk = os.environ["ANDROID_HOME"]
image = os.environ.get("SPIKE_IMAGE", "system-images;android-35;google_apis;x86_64")
tmp = os.environ.get("RUNNER_TEMP", "/tmp")
emulator = os.path.join(sdk, "emulator", "emulator")
avd_dir = os.path.expanduser("~/.android/avd")
BASE = ["-avd", os.environ.get("SPIKE_AVD", "spike"), "-no-window", "-no-boot-anim", "-gpu", os.environ.get("SPIKE_GPU", "swiftshader_indirect"),
        "-camera-back", "emulated", "-camera-front", "emulated", "-accel", "on"]


def sh(*cmd, timeout=60):
    try:
        return subprocess.run(list(cmd), capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return subprocess.CompletedProcess(cmd, 124, "", "timeout")


def create():
    t = time.time()
    avdm = os.path.join(sdk, "cmdline-tools", "latest", "bin", "avdmanager")
    r = subprocess.run([avdm, "create", "avd", "-n", os.environ.get("SPIKE_AVD", "spike"), "-k", image, "-d", "pixel_6", "--force"],
                       input="no\n", capture_output=True, text=True)
    print("avdmanager create rc=%d %.1fs %s" % (r.returncode, time.time() - t, (r.stdout + r.stderr).strip()[-500:]))
    cfg = os.path.join(avd_dir, os.environ.get("SPIKE_AVD", "spike") + ".avd", "config.ini")
    with open(cfg, "a") as f:
        f.write("hw.cpu.ncore=4\nhw.ramSize=4096\ndisk.dataPartition.size=12G\nhw.keyboard=yes\n"
                "hw.camera.back=emulated\nhw.camera.front=emulated\nhw.gpu.enabled=yes\nhw.gpu.mode=swiftshader_indirect\n"
                "hw.audioInput=yes\nhw.audioOutput=yes\n")
    print(open(cfg).read())
    r = subprocess.run([avdm, "list", "avd", "-c"], capture_output=True, text=True)
    print("avdmanager list avd: %s" % r.stdout.strip())


def start(label, extra):
    log = os.path.join(tmp, "emulator-%s.log" % label)
    cmd = [emulator] + BASE + extra
    print("start %s: %s (log %s)" % (label, " ".join(cmd), log), flush=True)
    with open(os.path.join(tmp, "emu-start-%s" % label), "w") as f:
        f.write(str(time.time()))
    subprocess.Popen(cmd, stdout=open(log, "w"), stderr=subprocess.STDOUT, start_new_session=True)


def wait(label, limit=float(os.environ.get("SPIKE_BOOT_LIMIT", "600"))):
    try:
        t0 = float(open(os.path.join(tmp, "emu-start-%s" % label)).read())
    except OSError:
        t0 = time.time()
    t_dev = t_boot = None
    while time.time() - t0 < limit:
        if time.time() - t0 > 15 and sh("pidof", "qemu-system-x86_64", "qemu-system-x86_64-headless", "qemu-system-aarch64").returncode != 0:
            print("emulator process gone after %.1fs" % (time.time() - t0))
            break
        r = sh("adb", "shell", "getprop", "sys.boot_completed", timeout=10)
        if r.returncode == 0 and t_dev is None:
            t_dev = time.time() - t0
        if r.stdout.strip() == "1":
            t_boot = time.time() - t0
            break
        time.sleep(1)
    if t_boot is None:
        print("TIMING boot %s: FAILED after %.1fs (device %s)" % (label, time.time() - t0, t_dev))
        log = os.path.join(tmp, "emulator-%s.log" % label)
        print(open(log, errors="replace").read()[-4000:])
        sys.exit(1)
    while time.time() - t0 < limit:
        if sh("adb", "shell", "pm", "path", "android", timeout=20).stdout.startswith("package:"):
            break
        time.sleep(1)
    t_pm = time.time() - t0
    print("TIMING boot %s: adb device %.1fs, sys.boot_completed %.1fs, pm ready %.1fs" % (label, t_dev, t_boot, t_pm), flush=True)
    log = os.path.join(tmp, "emulator-%s.log" % label)
    for l in open(log, errors="replace").read().splitlines():
        if any(k in l for k in ("snapshot", "Snapshot", "KVM", "accel", "GPU", "gpu", "audio", "Audio", "boot completed", "WARNING", "ERROR")):
            print("  emulator log: " + l[:300])


def kill():
    t = time.time()
    r = sh("adb", "emu", "kill", timeout=60)
    while time.time() - t < 120:
        if sh("pidof", "qemu-system-x86_64", "qemu-system-x86_64-headless").returncode != 0:
            break
        time.sleep(1)
    sh("adb", "kill-server")
    print("TIMING emulator kill: %.1fs (adb emu kill rc=%d %s)" % (time.time() - t, r.returncode, r.stdout.strip()), flush=True)


if __name__ == "__main__":
    c = sys.argv[1]
    if c == "create":
        create()
    elif c == "start":
        start(sys.argv[2], sys.argv[3:])
    elif c == "wait":
        wait(sys.argv[2])
    elif c == "boot":
        start(sys.argv[2], sys.argv[3:])
        wait(sys.argv[2])
    elif c == "kill":
        kill()
