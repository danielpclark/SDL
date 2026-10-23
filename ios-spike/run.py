#!/usr/bin/env python3
"""Spike prototype of tools/ios_run.py (stdlib only): boot, bundle, launch,
section. Prints what it does and how long each step takes."""

import json
import os
import plistlib
import shutil
import struct
import subprocess
import sys
import time


def log(*a):
    print("[run.py %s]" % time.strftime("%H:%M:%S"), *a, flush=True)


def sh(cmd, check=True, timeout=None, **kw):
    t = time.monotonic()
    log("$", " ".join(cmd))
    r = subprocess.run(cmd, text=True, capture_output=True, timeout=timeout, **kw)
    dt = time.monotonic() - t
    if r.stdout.strip():
        print(r.stdout.rstrip(), flush=True)
    if r.stderr.strip():
        print("(stderr)", r.stderr.rstrip(), flush=True)
    log("-> rc=%d in %.2fs" % (r.returncode, dt))
    if check and r.returncode != 0:
        sys.exit("command failed: %s" % cmd)
    return r


def simctl_json(*what):
    r = subprocess.run(["xcrun", "simctl", "list", "-j", *what], text=True, capture_output=True, check=True)
    return json.loads(r.stdout)


def cmd_boot(args):
    want_rt = os.environ.get("SPIKE_RUNTIME")
    want_dt = os.environ.get("SPIKE_DEVICE_TYPE")
    data = simctl_json("runtimes")
    ios = [r for r in data["runtimes"] if r.get("platform") == "iOS" and r.get("isAvailable")]
    for r in ios:
        log("iOS runtime:", r["identifier"], r["version"], r.get("buildversion"))
    if not ios:
        sys.exit("no iOS runtime")
    rt = None
    if want_rt:
        rt = next((r for r in ios if r["identifier"] == want_rt), None)
        if not rt:
            log("wanted runtime %s not found; using the newest" % want_rt)
    prefix = os.environ.get("SPIKE_RUNTIME_PREFIX")
    if not rt and prefix:
        cands = [r for r in ios if r["version"].startswith(prefix)]
        if cands:
            rt = sorted(cands, key=lambda r: [int(x) for x in r["version"].split(".")])[-1]
        else:
            log("no runtime with version prefix %s; using the newest" % prefix)
    if not rt:
        rt = sorted(ios, key=lambda r: [int(x) for x in r["version"].split(".")])[-1]
    dts = [d["identifier"] for d in rt.get("supportedDeviceTypes", [])]
    dt = None
    for cand in [want_dt, "com.apple.CoreSimulator.SimDeviceType.iPhone-16",
                 "com.apple.CoreSimulator.SimDeviceType.iPhone-17"]:
        if cand and cand in dts:
            dt = cand
            break
    if not dt:
        dt = [d for d in dts if "iPhone" in d][-1]
    log("chosen runtime", rt["identifier"], "device type", dt)
    t0 = time.monotonic()
    udid = None
    if os.environ.get("SPIKE_USE_EXISTING"):
        devs = simctl_json("devices")["devices"].get(rt["identifier"], [])
        for d in devs:
            if d.get("deviceTypeIdentifier") == dt and d.get("isAvailable"):
                udid = d["udid"]
                log("using the image's existing device", d["name"], udid, d.get("state"), "dataPathSize=%s" % d.get("dataPathSize"))
                break
    if not udid:
        udid = sh(["xcrun", "simctl", "create", "spike-" + dt.rsplit(".", 1)[-1], dt, rt["identifier"]]).stdout.strip()
    t1 = time.monotonic()
    sh(["xcrun", "simctl", "boot", udid])
    t2 = time.monotonic()
    sh(["xcrun", "simctl", "bootstatus", udid, "-b"], timeout=600)
    t3 = time.monotonic()
    log("TIMING create=%.1fs boot=%.1fs bootstatus=%.1fs total=%.1fs" % (t1 - t0, t2 - t1, t3 - t2, t3 - t0))
    gh = os.environ.get("GITHUB_ENV")
    if gh:
        with open(gh, "a") as f:
            f.write("SPIKE_UDID=%s\n" % udid)
    print(udid)


def write_plist(app, exe_name, bundle_id, scene, extra=None):
    info = {
        "CFBundleIdentifier": bundle_id,
        "CFBundleExecutable": exe_name,
        "CFBundleName": exe_name,
        "CFBundlePackageType": "APPL",
        "CFBundleVersion": "1",
        "CFBundleShortVersionString": "1.0",
        "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundleSupportedPlatforms": ["iPhoneSimulator"],
        "MinimumOSVersion": "14.0",
        "UIDeviceFamily": [1, 2],
        "UILaunchScreen": {},
        "UISupportedInterfaceOrientations": [
            "UIInterfaceOrientationPortrait",
            "UIInterfaceOrientationPortraitUpsideDown",
            "UIInterfaceOrientationLandscapeLeft",
            "UIInterfaceOrientationLandscapeRight",
        ],
        "UIApplicationSupportsIndirectInputEvents": True,
        "CADisableMinimumFrameDurationOnPhone": True,
        "NSCameraUsageDescription": "test",
        "NSMicrophoneUsageDescription": "test",
    }
    if scene:
        info["UIApplicationSceneManifest"] = {"UIApplicationSupportsMultipleScenes": False}
    if extra:
        info.update(extra)
    with open(os.path.join(app, "Info.plist"), "wb") as f:
        plistlib.dump(info, f, fmt=plistlib.FMT_BINARY)


def cmd_bundle(args):
    exe, app, bundle_id = args[0], args[1], args[2]
    scene = "--scene" in args
    t = time.monotonic()
    if os.path.exists(app):
        shutil.rmtree(app)
    os.makedirs(app)
    name = os.path.basename(exe)
    shutil.copy2(exe, os.path.join(app, name))
    write_plist(app, name, bundle_id, scene)
    with open(os.path.join(app, "PkgInfo"), "w") as f:
        f.write("APPL????")
    sh(["codesign", "--force", "--sign", "-", "--timestamp=none", app])
    sh(["codesign", "-dv", app], check=False)
    log("TIMING bundle+sign %.2fs" % (time.monotonic() - t))
    sh(["plutil", "-p", os.path.join(app, "Info.plist")], check=False)


def cmd_launch(args):
    # launch UDID APP BUNDLE_ID MODE TIMEOUT [ARGS...]
    udid, app, bundle_id, mode, timeout = args[0], args[1], args[2], args[3], float(args[4])
    rest = args[5:]
    t = time.monotonic()
    sh(["xcrun", "simctl", "install", udid, app])
    log("TIMING install %.2fs" % (time.monotonic() - t))
    cmd = ["xcrun", "simctl", "launch", "--terminate-running-process"]
    outdir = os.environ.get("RUNNER_TEMP", "/tmp")
    out_path = os.path.join(outdir, "app-stdout.txt")
    err_path = os.path.join(outdir, "app-stderr.txt")
    if mode == "pty":
        cmd.append("--console-pty")
    elif mode == "console":
        cmd.append("--console")
    elif mode == "files":
        for p in (out_path, err_path):
            if os.path.exists(p):
                os.remove(p)
        cmd += ["--stdout=" + out_path, "--stderr=" + err_path]
    cmd += [udid, bundle_id] + rest
    log("$", " ".join(cmd), "(timeout %ss)" % timeout)
    t0 = time.monotonic()
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL)
    lines = []
    first = None
    timed_out = False
    import selectors
    sel = selectors.DefaultSelector()
    sel.register(p.stdout, selectors.EVENT_READ)
    buf = b""
    while True:
        left = timeout - (time.monotonic() - t0)
        if left <= 0:
            timed_out = True
            break
        ev = sel.select(timeout=min(left, 1.0))
        if not ev:
            if p.poll() is not None:
                break
            continue
        chunk = os.read(p.stdout.fileno(), 65536)
        if not chunk:
            break
        buf += chunk
        while b"\n" in buf:
            line, buf = buf.split(b"\n", 1)
            s = line.decode(errors="replace").rstrip("\r")
            if first is None:
                first = time.monotonic() - t0
            lines.append(s)
            print("  | %7.3f %s" % (time.monotonic() - t0, s), flush=True)
    if buf:
        print("  | (partial) %r" % buf, flush=True)
        lines.append(buf.decode(errors="replace"))
    if timed_out:
        log("TIMEOUT after %.1fs: simctl launch did not return; terminating the app" % timeout)
        sh(["xcrun", "simctl", "terminate", udid, bundle_id], check=False)
        try:
            p.wait(timeout=20)
        except subprocess.TimeoutExpired:
            p.kill()
            p.wait()
    else:
        p.wait()
    dt = time.monotonic() - t0
    sentinel = [l for l in lines if l.startswith("sdl3-ios-runner: exit")]
    log("RESULT mode=%s simctl-launch rc=%s returned_by_itself=%s elapsed=%.2fs first_line_at=%s sentinel=%s"
        % (mode, p.returncode, not timed_out, dt, None if first is None else "%.2fs" % first, sentinel))
    if mode == "files":
        time.sleep(1)
        for pth in (out_path, err_path):
            log("contents of", pth, "exists=%s" % os.path.exists(pth))
            if os.path.exists(pth):
                print(open(pth, errors="replace").read()[-6000:], flush=True)
    sh(["xcrun", "simctl", "terminate", udid, bundle_id], check=False)
    sh(["xcrun", "simctl", "uninstall", udid, bundle_id], check=False)


def find_section(path, segname, sectname):
    data = open(path, "rb").read()
    magic = struct.unpack_from("<I", data, 0)[0]
    slices = []
    if magic in (0xCAFEBABE, 0xBEBAFECA):  # fat (big-endian header)
        n = struct.unpack_from(">I", data, 4)[0]
        for i in range(n):
            cpu, sub, off, size, align = struct.unpack_from(">iiIII", data, 8 + 20 * i)
            slices.append(off)
    else:
        slices.append(0)
    found = []
    for base in slices:
        m = struct.unpack_from("<I", data, base)[0]
        if m != 0xFEEDFACF:
            continue
        ncmds = struct.unpack_from("<I", data, base + 16)[0]
        off = base + 32
        for _ in range(ncmds):
            cmd, size = struct.unpack_from("<II", data, off)
            if cmd == 0x19:  # LC_SEGMENT_64
                seg = data[off + 8:off + 24].rstrip(b"\0").decode()
                nsects = struct.unpack_from("<I", data, off + 64)[0]
                so = off + 72
                for _ in range(nsects):
                    sname = data[so:so + 16].rstrip(b"\0").decode()
                    sseg = data[so + 16:so + 32].rstrip(b"\0").decode()
                    addr, ssize, soff = struct.unpack_from("<QQI", data, so + 32)
                    if sseg == segname and sname == sectname:
                        found.append(data[base + soff:base + soff + ssize])
                    so += 80
            off += size
    return found


def cmd_section(args):
    for p in args:
        log("section __DATA,__sdl3_runner in %s: %r" % (p, find_section(p, "__DATA", "__sdl3_runner")))


def main():
    cmd, args = sys.argv[1], sys.argv[2:]
    {"boot": cmd_boot, "bundle": cmd_bundle, "launch": cmd_launch, "section": cmd_section}[cmd](args)


if __name__ == "__main__":
    main()
