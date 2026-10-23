#!/usr/bin/env python3
"""Run a program built for the iOS Simulator (aarch64-apple-ios-sim,
x86_64-apple-ios) on a simulated device: Cargo's runner for those targets
(.cargo/config.toml), for the test programs and the doctests. It needs a
Mac with Xcode (`xcrun simctl`) and an iOS Simulator runtime.

Usage:
  ios_run.py PROGRAM [ARGUMENTS...]   run PROGRAM on the simulator
  ios_run.py --boot                   pick (or create) the simulator, boot
                                      it, and print its UDID
  ios_run.py --bundle PROGRAM OUT.app write PROGRAM's app bundle

A program runs in one of two ways:

* spawned (the default: libtest programs, doctests, the other
  `harness = false` programs): `xcrun simctl spawn UDID PROGRAM ARGS...`.
  The program runs as a plain process of the simulated device, with this
  script's stdin, stdout and stderr, and its exit status (128 + N for a
  signal N) is this script's. Its working directory is the device's data
  directory (as are HOME and TMPDIR), but it reads and writes the host's
  files by their paths (`env!("CARGO_MANIFEST_DIR")`, the target
  directory). Only `SIMCTL_CHILD_NAME` variables reach it (as NAME): this
  script passes on SDL3_*, SDL_*, RUST_* and RUSTC_BOOTSTRAP that way, and
  sets SDL_ASSERT to `abort` unless it is set, so that a failed assertion
  doesn't wait for an answer on the terminal. (Set SIMCTL_CHILD_NAME for
  any other variable.)

* as an app, when the program asks for it: a section `__DATA,__sdl3_runner`
  in its Mach-O file holding the word `app` (a
  `#[used] #[link_section = "__DATA,__sdl3_runner"]` static of words
  separated by spaces or NULs, as sdl3/tests/ios_runner.rs has). A UIKit
  program has to be an app: UIApplicationMain() can't start in a spawned
  process. The program is put in an app bundle (as `--bundle` writes it),
  installed and launched with `xcrun simctl launch --console-pty
  --terminate-running-process`, with the same variables, and its output
  (stdout and stderr together) is copied to stdout. simctl doesn't give
  an app's exit status, so the program ends by printing
  `sdl3-ios-runner: exit N` on a line of its own, then exits: N is this
  script's exit status (the line isn't copied). Without that line (a
  crash, an uncaught panic, an app that returned from main and stays up
  until the timeout) the run fails, and the end of the device's log for
  the program follows. The app is then terminated and uninstalled. A word
  `privacy=SERVICE,...` grants it those services first (`simctl privacy
  grant`: camera, microphone, photos, ...).

The simulator: SDL3_IOS_SIMULATOR's (a UDID or a device name), else a
booted iPhone or iPad simulator (one of SDL3_IOS_DEVICE_TYPE on
SDL3_IOS_RUNTIME when those are set), else the device `sdl3-TYPE-RUNTIME`,
which is created when it doesn't exist yet: of SDL3_IOS_DEVICE_TYPE (an
identifier, `com.apple.CoreSimulator.SimDeviceType.iPhone-16`, or a name,
`iPhone 16`; by default the iPhone 16, or the newest iPhone the runtime
has) on SDL3_IOS_RUNTIME (an identifier,
`com.apple.CoreSimulator.SimRuntime.iOS-18-5`, a version, `18.5`, or a
name, `iOS 18.5`; by default the newest iOS runtime installed). It is
booted when it isn't, and its boot waited for (`simctl bootstatus -b`,
which takes a minute or two on a new device) before the first program
runs. `--boot` does all that but the wait, and prints the UDID at once, so
the device can finish booting while the programs build:

  export SDL3_IOS_SIMULATOR=$(tools/ios_run.py --boot)

Environment:
  SDL3_IOS_SIMULATOR    the simulator (a UDID or a name)
  SDL3_IOS_DEVICE_TYPE  the device type of the simulator made or picked
  SDL3_IOS_RUNTIME      the iOS runtime of the simulator made or picked
  SDL3_IOS_TIMEOUT      seconds an app may run (default 900; 0: no limit)
  SDL3_IOS_CODESIGN     the identity app bundles are signed with (default
                        `-`, ad-hoc, which is all the simulator needs; 0:
                        not signed)
  SDL3_IOS_BUNDLE_ID    the bundle identifier (default
                        org.libsdl.sdl3-rust.NAME, NAME the program's)
  SDL3_IOS_VERBOSE      1: the simctl commands, with their times, on stderr

A program built for devices (aarch64-apple-ios) can't run on the
simulator: `--bundle` writes its bundle, which needs signing for a device.
"""
import fcntl
import json
import os
import plistlib
import re
import selectors
import shlex
import shutil
import struct
import subprocess
import sys
import tempfile
import time

PROGRAM = os.path.basename(__file__)

VERBOSE = os.environ.get("SDL3_IOS_VERBOSE", "") not in ("", "0")

SENTINEL = re.compile(r"sdl3-ios-runner: exit (-?\d+)")

# The variables a spawned or launched program gets (as SIMCTL_CHILD_NAME).
FORWARD_PREFIXES = ("SDL3_", "SDL_", "RUST_")
FORWARD_NAMES = ("RUSTC_BOOTSTRAP",)

DEFAULT_DEVICE_TYPES = ("com.apple.CoreSimulator.SimDeviceType.iPhone-16",)

# LC_BUILD_VERSION's platforms
PLATFORM_IOS = 2
PLATFORM_MACCATALYST = 6
PLATFORM_IOSSIMULATOR = 7
PLATFORM_NAMES = {
    1: "macOS",
    PLATFORM_IOS: "iOS devices",
    3: "tvOS devices",
    4: "watchOS devices",
    PLATFORM_MACCATALYST: "Mac Catalyst",
    PLATFORM_IOSSIMULATOR: "the iOS Simulator",
    8: "the tvOS Simulator",
    9: "the watchOS Simulator",
    11: "visionOS devices",
    12: "the visionOS Simulator",
}

CPU_TYPE_X86_64 = 0x01000007
CPU_TYPE_ARM64 = 0x0100000C


class Failure(Exception):
    """An error this script reports (and exits 1 for)."""


def note(message):
    print(f"{PROGRAM}: {message}", file=sys.stderr, flush=True)


def xcrun_simctl(*args, check=True, timeout=None, env=None):
    """`xcrun simctl ARGS...`, its output captured."""
    argv = ["xcrun", "simctl", *args]
    start = time.monotonic()
    try:
        result = subprocess.run(
            argv,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            errors="replace",
            timeout=timeout,
            env=env,
        )
    except FileNotFoundError:
        raise Failure("no xcrun: the iOS Simulator needs Xcode (xcode-select -s /Applications/Xcode.app)") from None
    except subprocess.TimeoutExpired:
        raise Failure(f"{shlex.join(argv)} took more than {timeout} s") from None
    if VERBOSE:
        note(f"{shlex.join(argv)}: {result.returncode} ({time.monotonic() - start:.2f} s)")
    if check and result.returncode != 0:
        output = (result.stderr.strip() or result.stdout.strip())
        raise Failure(f"{shlex.join(argv)} failed ({result.returncode}): {output}")
    return result


def simctl_list(what):
    return json.loads(xcrun_simctl("list", "-j", what).stdout)


# ---------------------------------------------------------------------------
# The program's Mach-O file


class MachO:
    """What this script reads of a Mach-O file (thin, or the first slice of
    a fat one with 64-bit slices): the CPU, the platform and minimum OS
    version (LC_BUILD_VERSION), and the words of `__DATA,__sdl3_runner`."""

    def __init__(self, path):
        self.cpu = None
        self.platform = None
        self.minos = None
        self.words = []
        try:
            with open(path, "rb") as f:
                data = f.read()
        except OSError as e:
            raise Failure(f"can't read {path}: {e}") from None
        if len(data) < 8:
            raise Failure(f"{path} isn't a Mach-O file")
        magic = struct.unpack_from(">I", data, 0)[0]
        if magic in (0xCAFEBABE, 0xCAFEBABF):
            # (fat: big-endian headers, 64-bit offsets for 0xCAFEBABF)
            count = struct.unpack_from(">I", data, 4)[0]
            offsets = []
            for i in range(count):
                if magic == 0xCAFEBABE:
                    _, _, offset, _, _ = struct.unpack_from(">iiIII", data, 8 + 20 * i)
                else:
                    _, _, offset, _, _, _ = struct.unpack_from(">iiQQII", data, 8 + 32 * i)
                offsets.append(offset)
        else:
            offsets = [0]
        for base in offsets:
            if self._slice(data, base):
                return
        raise Failure(f"{path} isn't a 64-bit Mach-O file")

    def _slice(self, data, base):
        try:
            magic, cpu, _, _, ncmds, _, _, _ = struct.unpack_from("<IiiIIIII", data, base)
        except struct.error:
            return False
        if magic != 0xFEEDFACF:
            return False
        self.cpu = cpu & 0xFFFFFFFF
        offset = base + 32
        for _ in range(ncmds):
            cmd, size = struct.unpack_from("<II", data, offset)
            if cmd == 0x19:  # LC_SEGMENT_64
                nsects = struct.unpack_from("<I", data, offset + 64)[0]
                section = offset + 72
                for _ in range(nsects):
                    sectname = data[section:section + 16].rstrip(b"\0")
                    segname = data[section + 16:section + 32].rstrip(b"\0")
                    _, sect_size, sect_offset = struct.unpack_from("<QQI", data, section + 32)
                    if segname == b"__DATA" and sectname == b"__sdl3_runner":
                        contents = data[base + sect_offset:base + sect_offset + sect_size]
                        self.words += contents.decode(errors="replace").replace("\0", " ").split()
                    section += 80
            elif cmd == 0x32:  # LC_BUILD_VERSION
                self.platform, minos = struct.unpack_from("<II", data, offset + 8)
                self.minos = f"{minos >> 16}.{(minos >> 8) & 0xFF}"
                if minos & 0xFF:
                    self.minos += f".{minos & 0xFF}"
            offset += size
        return True

    def platform_name(self):
        return PLATFORM_NAMES.get(self.platform, f"platform {self.platform}")

    def is_simulator(self):
        # (no LC_BUILD_VERSION: taken for the iOS Simulator)
        return self.platform in (None, PLATFORM_IOSSIMULATOR)


def program_name(program):
    """The program's name without the hash Cargo adds (`sdl3-0123...`)."""
    name = os.path.basename(program)
    stem, _, suffix = name.rpartition("-")
    if stem and len(suffix) == 16 and all(c in "0123456789abcdef" for c in suffix):
        name = stem
    return name


def bundle_id(name):
    return os.environ.get("SDL3_IOS_BUNDLE_ID") or "org.libsdl.sdl3-rust." + re.sub(r"[^A-Za-z0-9.-]", "-", name)


# ---------------------------------------------------------------------------
# The app bundle


def write_bundle(program, macho, app, identifier):
    """PROGRAM's app bundle at APP: the executable, Info.plist and PkgInfo,
    signed as SDL3_IOS_CODESIGN says."""
    name = program_name(program)
    executable = re.sub(r"[^A-Za-z0-9_.-]", "-", name)
    os.makedirs(app)
    shutil.copy2(program, os.path.join(app, executable))
    simulator = macho.platform != PLATFORM_IOS
    info = {
        "CFBundleIdentifier": identifier,
        "CFBundleExecutable": executable,
        "CFBundleName": name[:15],
        "CFBundleDisplayName": name,
        "CFBundlePackageType": "APPL",
        "CFBundleVersion": "1",
        "CFBundleShortVersionString": "1.0",
        "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundleDevelopmentRegion": "en",
        "CFBundleSupportedPlatforms": ["iPhoneSimulator" if simulator else "iPhoneOS"],
        "MinimumOSVersion": macho.minos or "14.0",
        "UIDeviceFamily": [1, 2],
        # (without a launch screen the app runs letterboxed, in the
        # compatibility mode of old apps, with that mode's screen size)
        "UILaunchScreen": {},
        "UISupportedInterfaceOrientations": [
            "UIInterfaceOrientationPortrait",
            "UIInterfaceOrientationPortraitUpsideDown",
            "UIInterfaceOrientationLandscapeLeft",
            "UIInterfaceOrientationLandscapeRight",
        ],
        "UIApplicationSupportsIndirectInputEvents": True,
        "CADisableMinimumFrameDurationOnPhone": True,
        "NSCameraUsageDescription": "The SDL tests check the camera.",
        "NSMicrophoneUsageDescription": "The SDL tests check audio recording.",
    }
    with open(os.path.join(app, "Info.plist"), "wb") as f:
        plistlib.dump(info, f, fmt=plistlib.FMT_BINARY)
    with open(os.path.join(app, "PkgInfo"), "w") as f:
        f.write("APPL????")
    identity = os.environ.get("SDL3_IOS_CODESIGN") or "-"
    if identity != "0":
        argv = ["codesign", "--force", "--sign", identity, "--timestamp=none", app]
        try:
            result = subprocess.run(argv, stdin=subprocess.DEVNULL, capture_output=True, text=True, errors="replace")
        except FileNotFoundError:
            raise Failure("no codesign (set SDL3_IOS_CODESIGN=0 not to sign)") from None
        if result.returncode != 0:
            raise Failure(f"{shlex.join(argv)} failed: {result.stderr.strip()}")


def cmd_bundle(args):
    if len(args) != 2:
        raise Failure("usage: --bundle PROGRAM OUT.app")
    program, app = args
    macho = MachO(program)
    if macho.platform not in (None, PLATFORM_IOS, PLATFORM_IOSSIMULATOR):
        raise Failure(f"{program} is built for {macho.platform_name()}, not iOS")
    if os.path.lexists(app):
        if not os.path.isfile(os.path.join(app, "Info.plist")):
            raise Failure(f"{app} exists and isn't an app bundle")
        shutil.rmtree(app)
    write_bundle(program, macho, app, bundle_id(program_name(program)))
    return 0


# ---------------------------------------------------------------------------
# The simulator


def ios_runtimes():
    runtimes = []
    for runtime in simctl_list("runtimes")["runtimes"]:
        platform = runtime.get("platform") or runtime.get("name", "").split(" ")[0]
        if platform == "iOS" and runtime.get("isAvailable", True):
            runtimes.append(runtime)
    return sorted(runtimes, key=lambda r: [int(p) if p.isdigit() else 0 for p in r.get("version", "0").split(".")])


def pick_runtime(runtimes):
    if not runtimes:
        raise Failure("no iOS Simulator runtime installed (Xcode > Settings > Components)")
    want = os.environ.get("SDL3_IOS_RUNTIME")
    if not want:
        return runtimes[-1]
    for runtime in runtimes:
        if want in (runtime.get("identifier"), runtime.get("version"), runtime.get("name")):
            return runtime
    raise Failure(
        f"SDL3_IOS_RUNTIME is {want!r}, which isn't an installed iOS runtime ("
        + ", ".join(r.get("identifier", "?") for r in runtimes)
        + ")"
    )


def pick_device_type(runtime):
    types = runtime.get("supportedDeviceTypes")
    if types is None:
        types = simctl_list("devicetypes")["devicetypes"]
    want = os.environ.get("SDL3_IOS_DEVICE_TYPE")
    if want:
        for device_type in types:
            if want in (device_type.get("identifier"), device_type.get("name")):
                return device_type
        raise Failure(f"SDL3_IOS_DEVICE_TYPE is {want!r}, which {runtime.get('name')} doesn't have")
    for identifier in DEFAULT_DEVICE_TYPES:
        for device_type in types:
            if device_type.get("identifier") == identifier:
                return device_type
    iphones = [t for t in types if t.get("productFamily") == "iPhone" or "iPhone" in t.get("name", "")]
    if not iphones:
        raise Failure(f"{runtime.get('name')} has no iPhone device type (set SDL3_IOS_DEVICE_TYPE)")
    return iphones[-1]


def short(identifier):
    return identifier.rsplit(".", 1)[-1]


def find_simulator():
    """The simulator's UDID and whether it's booted (made when needed)."""
    want = os.environ.get("SDL3_IOS_SIMULATOR")
    devices = simctl_list("devices")["devices"]
    available = [
        (runtime, device)
        for runtime, listed in devices.items()
        for device in listed
        if device.get("isAvailable", True)
    ]
    if want:
        matches = [(r, d) for r, d in available if want in (d.get("udid"), d.get("name"))]
        if not matches:
            raise Failure(f"SDL3_IOS_SIMULATOR is {want!r}, which isn't an available simulator (xcrun simctl list devices)")
        matches.sort(key=lambda m: m[1].get("state") != "Booted")
        device = matches[0][1]
        return device["udid"], device.get("state") == "Booted"

    explicit = os.environ.get("SDL3_IOS_DEVICE_TYPE") or os.environ.get("SDL3_IOS_RUNTIME")
    if not explicit:
        for runtime, device in available:
            if ".SimRuntime.iOS-" in runtime and device.get("state") == "Booted":
                return device["udid"], True

    runtime = pick_runtime(ios_runtimes())
    device_type = pick_device_type(runtime)
    name = f"sdl3-{short(device_type['identifier'])}-{short(runtime['identifier'])}"
    candidates = [
        d for r, d in available
        if r == runtime["identifier"] and d.get("deviceTypeIdentifier") == device_type["identifier"]
    ]
    booted = [d for d in candidates if d.get("state") == "Booted"]
    if booted:
        return booted[0]["udid"], True
    for device in candidates:
        if device.get("name") == name:
            return device["udid"], False
    udid = xcrun_simctl("create", name, device_type["identifier"], runtime["identifier"]).stdout.strip()
    if VERBOSE:
        note(f"created {name} ({udid})")
    return udid, False


def boot(udid):
    result = xcrun_simctl("boot", udid, check=False)
    if result.returncode != 0 and "Booted" not in result.stderr:
        raise Failure(f"xcrun simctl boot {udid} failed ({result.returncode}): {result.stderr.strip()}")


class Lock:
    """An exclusive lock on a file in the temporary directory, around the
    choice (and creation) of the simulator, which programs run at the same
    time would otherwise both make."""

    def __enter__(self):
        path = os.path.join(tempfile.gettempdir(), f"sdl3-ios-run-{os.getuid()}.lock")
        self.fd = os.open(path, os.O_RDWR | os.O_CREAT, 0o600)
        fcntl.flock(self.fd, fcntl.LOCK_EX)
        return self

    def __exit__(self, *exc):
        os.close(self.fd)


def simulator():
    """The booted simulator's UDID, its boot finished."""
    with Lock():
        udid, booted = find_simulator()
        if not booted:
            boot(udid)
        # (at once once it has booted; each time, since the device may
        # have been shut down since)
        result = xcrun_simctl("bootstatus", udid, "-b", check=False, timeout=900)
        if result.returncode != 0:
            raise Failure(f"the simulator {udid} didn't boot: {result.stdout[-2000:]}{result.stderr}")
    return udid


def cmd_boot(args):
    if args:
        raise Failure("usage: --boot")
    with Lock():
        udid, booted = find_simulator()
        if not booted:
            boot(udid)
    print(udid, flush=True)
    return 0


# ---------------------------------------------------------------------------
# Running


def child_environment():
    env = dict(os.environ)
    for name, value in os.environ.items():
        if name.startswith(FORWARD_PREFIXES) or name in FORWARD_NAMES:
            env.setdefault("SIMCTL_CHILD_" + name, value)
    env.setdefault("SIMCTL_CHILD_SDL_ASSERT", "abort")
    return env


def check_runnable(program, macho):
    if macho.is_simulator():
        return
    if macho.platform == PLATFORM_IOS:
        raise Failure(
            f"{program} is built for iOS devices (aarch64-apple-ios), which this script can't run: "
            "build it for the Simulator (--target aarch64-apple-ios-sim), or bundle it with --bundle"
        )
    raise Failure(f"{program} is built for {macho.platform_name()}, not the iOS Simulator")


def spawn(udid, program, args, macho):
    argv = ["xcrun", "simctl", "spawn"]
    if macho.cpu == CPU_TYPE_X86_64:
        argv.append("--arch=x86_64")
    argv += [udid, program, *args]
    if VERBOSE:
        note("running " + shlex.join(argv))
    sys.stdout.flush()
    sys.stderr.flush()
    try:
        os.execvpe("xcrun", argv, child_environment())
    except OSError as e:
        raise Failure(f"can't run xcrun: {e}") from None


def device_log(udid, process):
    """The end of the device's log for `process`, from the last minutes."""
    result = xcrun_simctl(
        "spawn", udid, "log", "show", "--last", "5m", "--style", "compact",
        "--predicate", f'process == "{process}"', check=False, timeout=120,
    )
    lines = (result.stdout + result.stderr).splitlines()
    return lines[-200:]


def launch(udid, program, args, macho):
    try:
        timeout = float(os.environ.get("SDL3_IOS_TIMEOUT") or 900)
    except ValueError:
        raise Failure("SDL3_IOS_TIMEOUT isn't a number of seconds") from None
    name = program_name(program)
    identifier = bundle_id(name)
    with tempfile.TemporaryDirectory(prefix="sdl3-ios-") as temp:
        app = os.path.join(temp, re.sub(r"[^A-Za-z0-9_.-]", "-", name) + ".app")
        write_bundle(program, macho, app, identifier)
        executable = re.sub(r"[^A-Za-z0-9_.-]", "-", name)
        xcrun_simctl("install", udid, app, timeout=600)
        try:
            for word in macho.words:
                if word.startswith("privacy="):
                    for service in filter(None, word[len("privacy="):].split(",")):
                        xcrun_simctl("privacy", udid, "grant", service, identifier)
            return run_app(udid, identifier, executable, args, timeout)
        finally:
            xcrun_simctl("terminate", udid, identifier, check=False, timeout=60)
            xcrun_simctl("uninstall", udid, identifier, check=False, timeout=120)


def run_app(udid, identifier, executable, args, timeout):
    argv = ["xcrun", "simctl", "launch", "--console-pty", "--terminate-running-process", udid, identifier, *args]
    if VERBOSE:
        note("running " + shlex.join(argv))
    start = time.monotonic()
    process = subprocess.Popen(
        argv,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        env=child_environment(),
    )
    # (the line simctl prints with the app's process ID, last)
    pid_line = re.compile(re.escape(identifier) + r": \d+")
    status = None
    tail = []
    timed_out = False
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    pending = b""

    def line(raw):
        nonlocal status
        text = raw.decode(errors="replace").rstrip("\r")
        match = SENTINEL.fullmatch(text)
        if match:
            status = int(match.group(1)) & 0xFF
            return
        if pid_line.fullmatch(text):
            return
        tail.append(text)
        del tail[:-40]
        sys.stdout.write(text + "\n")
        sys.stdout.flush()

    while True:
        if timeout and time.monotonic() - start > timeout:
            timed_out = True
            break
        if not selector.select(timeout=1.0):
            continue
        chunk = os.read(process.stdout.fileno(), 1 << 16)
        if not chunk:
            break
        pending += chunk
        *lines, pending = pending.split(b"\n")
        for raw in lines:
            line(raw)
    if pending:
        line(pending)
    if timed_out:
        note(f"the app ran for more than {timeout:g} s (SDL3_IOS_TIMEOUT): terminating it")
        xcrun_simctl("terminate", udid, identifier, check=False, timeout=60)
    try:
        process.wait(timeout=30)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
    if status is not None and not timed_out:
        return status
    if not timed_out:
        if process.returncode != 0:
            note(f"xcrun simctl launch failed ({process.returncode})")
        else:
            note(
                "the app ended without printing `sdl3-ios-runner: exit N` (it crashed, panicked "
                "in a callback, or exited some other way)"
            )
    if tail:
        note("the end of its output:")
        for text in tail[-20:]:
            print(f"  | {text}", file=sys.stderr)
    note(f'the device\'s log for process "{executable}":')
    for text in device_log(udid, executable):
        print(f"  | {text}", file=sys.stderr)
    sys.stderr.flush()
    return 1


def cmd_run(program, args):
    if not os.path.isfile(program):
        raise Failure(f"no program {program}")
    program = os.path.abspath(program)
    macho = MachO(program)
    check_runnable(program, macho)
    udid = simulator()
    if "app" in macho.words:
        return launch(udid, program, args, macho)
    return spawn(udid, program, args, macho)


def main():
    if len(sys.argv) < 2 or sys.argv[1] in ("-h", "--help"):
        print(__doc__.split("\n\n", 2)[1], file=sys.stderr)
        return 2
    try:
        if sys.argv[1] == "--boot":
            return cmd_boot(sys.argv[2:])
        if sys.argv[1] == "--bundle":
            return cmd_bundle(sys.argv[2:])
        return cmd_run(sys.argv[1], sys.argv[2:])
    except Failure as e:
        note(str(e))
        return 1


if __name__ == "__main__":
    sys.exit(main())
