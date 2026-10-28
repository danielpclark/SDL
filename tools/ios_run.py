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
  installed and launched with `xcrun simctl launch
  --terminate-running-process` (a launch the system refuses, as it now
  and then does for an app it hasn't registered yet, is tried up to
  three times), with the same variables, its stdout and
  stderr in files in the device's temporary directory (so that a program
  that ends at once loses nothing), which are copied to this script's as
  they grow, until the app's process is gone. simctl doesn't give an
  app's exit status, so the program ends by printing `sdl3-ios-runner:
  exit N` on a line of its own, then exits: N is this script's exit
  status (the line isn't copied). Without that line (a crash, an uncaught
  panic, an app that returned from main and stays up until the timeout)
  the run fails, and the end of the device's log for the program follows,
  then the reason once more.
  The app is then terminated and uninstalled. A word
  `privacy=SERVICE,...` grants it those services first (`simctl privacy
  grant`: camera, microphone, photos, ...). The app's bundle identifier
  is also a URL scheme it handles (CFBundleURLTypes), and one it may ask
  about (LSApplicationQueriesSchemes), so that it can open its own URLs.

  A running app can ask for what only the host can do, with a line
  `sdl3-ios-runner: request WHAT [ARGUMENTS]` (copied, then acted on):
    background  launch the Settings app (com.apple.Preferences), which
                sends this one to the background
    foreground  launch this app again (without terminating it), which
                brings it back to the foreground, the same process (the
                runner does that itself after a minute: an app in the
                background is soon suspended)
    tap X Y     a finger taps the screen at (X, Y), in points from its
                top left corner (`idb ui tap`)
    swipe X1 Y1 X2 Y2 [SECONDS]
                a finger moves on the screen from (X1, Y1) to (X2, Y2)
                (`idb ui swipe`, in SECONDS, by default idb's)
    key CODE [SECONDS]
                the hardware keyboard's key of USB HID usage CODE (page
                7: 4 is A, 40 Return) is pressed, and released after
                SECONDS (`idb ui key`, by default idb's)
    text TEXT   the hardware keyboard types TEXT, the rest of the line
                (`idb ui text`, a key press for each character, with
                Shift for those that need it)
  The app waits for what follows (its application events, its touches,
  its keys) itself. The touches and keys need idb (fb-idb, Meta's iOS
  development bridge: the `idb` client, a Python program, and its
  `idb_companion`, which drives the simulator's input): SDL3_IOS_IDB's
  command (0: none), else `idb` on PATH, with `idb_companion` on PATH. The
  app learns whether the runner has them in SDL3_IOS_RUNNER_IDB: `1`, or
  why not (and skips what needs them). After an app that used them, `idb
  kill` stops the companions idb started.

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
  SDL3_IOS_IDB          the idb command for the `tap`, `swipe`, `key` and
                        `text` requests (default `idb` on PATH; 0: none)

A program built for devices (aarch64-apple-ios) can't run on the
simulator: `--bundle` writes its bundle, which needs signing for a device.
"""
import fcntl
import json
import os
import plistlib
import re
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
REQUEST = re.compile(r"sdl3-ios-runner: request (\S+)(?: (.*))?")
NUMBER = re.compile(r"-?\d+(\.\d+)?")

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
    # (the output goes to files, not pipes: a process simctl leaves behind
    # would keep a pipe open, and reading it to its end would never end)
    with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
        try:
            process = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=out, stderr=err, env=env)
        except FileNotFoundError:
            raise Failure("no xcrun: the iOS Simulator needs Xcode (xcode-select -s /Applications/Xcode.app)") from None
        try:
            returncode = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
            raise Failure(f"{shlex.join(argv)} took more than {timeout} s") from None
        out.seek(0)
        err.seek(0)
        result = subprocess.CompletedProcess(
            argv, returncode, out.read().decode(errors="replace"), err.read().decode(errors="replace")
        )
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
        # (CoreBluetooth, which HIDAPI's Steam Controller backend starts,
        # ends an app without them)
        "NSBluetoothAlwaysUsageDescription": "The SDL tests check Bluetooth Steam Controllers.",
        "NSBluetoothPeripheralUsageDescription": "The SDL tests check Bluetooth Steam Controllers.",
        # (the bundle identifier as a URL scheme the app handles and may
        # ask about, for its own URLs)
        "CFBundleURLTypes": [{"CFBundleURLName": identifier, "CFBundleURLSchemes": [identifier]}],
        "LSApplicationQueriesSchemes": [identifier],
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


def find_idb():
    """The idb command for the touch and key requests and None, or None
    and why there's none."""
    setting = os.environ.get("SDL3_IOS_IDB", "")
    if setting == "0":
        return None, "SDL3_IOS_IDB is 0"
    command = shutil.which(setting or "idb")
    if not command:
        return None, f"no idb (fb-idb): {setting!r} isn't a command" if setting else "no idb (fb-idb) on PATH"
    if not shutil.which("idb_companion"):
        return None, "no idb_companion on PATH"
    return command, None


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


def device_log(udid, process, identifier):
    """The end of the device's log for `process` and of what the system
    said about the app `identifier` (its launch), from the last minutes,
    and the start of the newest crash report of `process` from then."""
    result = xcrun_simctl(
        "spawn", udid, "log", "show", "--last", "5m", "--style", "compact",
        "--predicate", f'process == "{process}" OR eventMessage CONTAINS "{identifier}"',
        check=False, timeout=120,
    )
    lines = (result.stdout + result.stderr).splitlines()[-200:]
    reports = os.path.expanduser("~/Library/Logs/DiagnosticReports")
    try:
        names = [n for n in os.listdir(reports) if n.startswith(process) and n.endswith(".ips")]
    except OSError:
        names = []
    recent = [n for n in names if time.time() - os.path.getmtime(os.path.join(reports, n)) < 300]
    if recent:
        newest = max(recent, key=lambda n: os.path.getmtime(os.path.join(reports, n)))
        lines.append(f"(crash report {newest})")
        with open(os.path.join(reports, newest), errors="replace") as f:
            lines += f.read().splitlines()[:80]
    return lines


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
        idb, why = find_idb()
        used_idb = []
        try:
            for word in macho.words:
                if word.startswith("privacy="):
                    for service in filter(None, word[len("privacy="):].split(",")):
                        xcrun_simctl("privacy", udid, "grant", service, identifier)
            return run_app(udid, identifier, executable, args, timeout, temp, (idb, why, used_idb))
        finally:
            if used_idb:
                # (the companions idb started for the device: the next
                # request starts one again)
                run_idb(idb, "kill")
            xcrun_simctl("terminate", udid, identifier, check=False, timeout=60)
            xcrun_simctl("uninstall", udid, identifier, check=False, timeout=120)


def alive(pid):
    """Whether the process `pid` (a simulated device's process is one of the
    Mac's) is there."""
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        pass
    return True


class Output:
    """A file the app writes, copied to `stream` as it grows: by lines, the
    `sdl3-ios-runner: exit N` line kept out (and noted) when `sentinel`,
    and the `sdl3-ios-runner: request WHAT [ARGUMENTS]` lines passed to
    `on_request` (after they're copied)."""

    def __init__(self, path, stream, sentinel, on_request=None):
        self.path = path
        self.stream = stream
        self.sentinel = sentinel
        self.on_request = on_request
        self.file = None
        self.pending = b""
        self.status = None
        self.tail = []

    def poll(self, final=False):
        if self.file is None:
            try:
                self.file = open(self.path, "rb")
            except FileNotFoundError:
                return
        chunk = self.file.read()
        self.pending += chunk
        *lines, self.pending = self.pending.split(b"\n")
        if final and self.pending:
            lines.append(self.pending)
            self.pending = b""
        for raw in lines:
            text = raw.decode(errors="replace").rstrip("\r")
            match = SENTINEL.fullmatch(text) if self.sentinel else None
            if match:
                self.status = int(match.group(1)) & 0xFF
                continue
            self.tail.append(text)
            del self.tail[:-40]
            self.stream.write(text + "\n")
            request = REQUEST.fullmatch(text) if self.on_request else None
            if request:
                self.stream.flush()
                self.on_request(request.group(1), request.group(2))
        self.stream.flush()

    def close(self):
        if self.file is not None:
            self.file.close()


def run_idb(idb, *args):
    """Run `idb ARGS...` (and note how it failed): whether it succeeded."""
    argv = [idb, *args]
    start = time.monotonic()
    try:
        result = subprocess.run(
            argv, stdin=subprocess.DEVNULL, capture_output=True, text=True, errors="replace", timeout=120,
        )
    except subprocess.TimeoutExpired:
        note(f"{shlex.join(argv)} didn't finish in 120 s")
        return False
    except OSError as e:
        note(f"can't run {idb}: {e}")
        return False
    if VERBOSE:
        note(f"{shlex.join(argv)} -> {result.returncode} in {time.monotonic() - start:.2f} s")
        for text in (result.stdout + result.stderr).splitlines()[-200:]:
            print(f"  | {text}", file=sys.stderr)
    if result.returncode != 0:
        note(f"{shlex.join(argv)} failed ({result.returncode}): {(result.stderr or result.stdout).strip()}")
        return False
    return True


def run_app(udid, identifier, executable, args, timeout, temp, idb):
    # The app's stdout and stderr go to files, which launchd opens for it
    # before it starts, so that nothing it prints is lost (through `simctl
    # launch --console-pty` a program that is done at once now and then
    # lost all its output), in the device's temporary directory (files in
    # the Mac's stayed empty). simctl returns once the app has started,
    # printing its process ID, a process of the Mac's.
    device_tmp = xcrun_simctl("getenv", udid, "TMPDIR", check=False).stdout.strip()
    if device_tmp and os.path.isdir(device_tmp):
        temp = tempfile.mkdtemp(prefix="sdl3-ios-run-", dir=device_tmp)
    else:
        temp = tempfile.mkdtemp(prefix="sdl3-ios-run-", dir=temp)
    try:
        return run_app_in(udid, identifier, executable, args, timeout, temp, idb)
    finally:
        shutil.rmtree(temp, ignore_errors=True)


# An app in the background is suspended a few seconds after it gets there;
# one that asked for the background but not to come back by then is
# brought back (so that it can tell what went wrong).
BACKGROUND_LIMIT = 60

# How often a launch the system refuses is tried, and the pause between.
LAUNCH_ATTEMPTS = 3
LAUNCH_PAUSE = 5


class Requests:
    """What a running app asks for (`sdl3-ios-runner: request WHAT`)."""

    def __init__(self, udid, identifier, pid, idb):
        self.udid = udid
        self.identifier = identifier
        self.pid = pid
        self.background_since = None
        # (the idb command, why there's none, and the list told once idb
        # was used)
        self.idb, self.no_idb, self.used_idb = idb

    def handle(self, what, args=""):
        words = (args or "").split()
        if what in ("tap", "swipe", "key"):
            self.touch(what, words)
        elif what == "text":
            self.text(args or "")
        elif what == "background":
            # (the Settings app comes to the foreground, launched anew:
            # this one resigns being active, then enters the background a
            # few seconds later)
            result = xcrun_simctl(
                "launch", "--terminate-running-process", self.udid, "com.apple.Preferences",
                check=False, timeout=120,
            )
            if result.returncode != 0:
                note(f"request background: launching Settings failed ({result.returncode}): {result.stderr.strip()}")
            self.background_since = time.monotonic()
        elif what == "foreground":
            self.foreground()
        else:
            note(f"unknown request {what!r} (ignored)")

    def touch(self, what, words):
        """A `tap`, `swipe` or `key` request (numbers only)."""
        counts = {"tap": (2,), "swipe": (4, 5), "key": (1, 2)}[what]
        if len(words) not in counts or not all(NUMBER.fullmatch(w) for w in words):
            note(f"request {what}: bad arguments {' '.join(words)!r} (ignored)")
            return
        if what == "key" and not words[0].isdigit():
            note(f"request key: {words[0]!r} isn't a key code (ignored)")
            return
        if not self.idb:
            note(f"request {what}: ignored, {self.no_idb}")
            return
        if what == "tap":
            argv = ["ui", "tap", "--udid", self.udid, *words]
        elif what == "swipe":
            duration = ["--duration", words[4]] if len(words) == 5 else []
            argv = ["ui", "swipe", "--udid", self.udid, *duration, *words[:4]]
        else:
            duration = ["--duration", words[1]] if len(words) == 2 else []
            argv = ["ui", "key", "--udid", self.udid, *duration, words[0]]
        self.used_idb.append(what)
        run_idb(self.idb, *argv)

    def text(self, text):
        """A `text` request: TEXT as it is (spaces too)."""
        if not text:
            note("request text: no text (ignored)")
            return
        if not self.idb:
            note(f"request text: ignored, {self.no_idb}")
            return
        self.used_idb.append("text")
        # (`--` keeps a text starting with `-` from being an option)
        run_idb(self.idb, "ui", "text", "--udid", self.udid, "--", text)

    def foreground(self):
        # (the running app, the same process, comes back to the foreground)
        self.background_since = None
        result = xcrun_simctl("launch", self.udid, self.identifier, check=False, timeout=120)
        match = re.search(re.escape(self.identifier) + r": (\d+)", result.stdout)
        if result.returncode != 0 or not match:
            note(f"request foreground: launching the app failed ({result.returncode}): {result.stderr.strip()}")
        elif int(match.group(1)) != self.pid:
            note(f"request foreground: the app was launched again as process {match.group(1)}, not brought back")

    def check(self):
        if self.background_since is not None and time.monotonic() - self.background_since > BACKGROUND_LIMIT:
            note(f"the app didn't ask to come back from the background within {BACKGROUND_LIMIT} s: bringing it back")
            self.foreground()


def run_app_in(udid, identifier, executable, args, timeout, temp, idb):
    out_path = os.path.join(temp, "stdout.txt")
    err_path = os.path.join(temp, "stderr.txt")
    start = time.monotonic()
    env = child_environment()
    env["SIMCTL_CHILD_SDL3_IOS_RUNNER_IDB"] = "1" if idb[0] else idb[1]
    # Now and then the system refuses to launch an app just installed
    # (FrontBoard doesn't know it yet: LaunchServicesDataMismatch). None
    # of the app has run then, so it is launched again after a pause.
    for attempt in range(1, LAUNCH_ATTEMPTS + 1):
        result = xcrun_simctl(
            "launch", "--terminate-running-process", f"--stdout={out_path}", f"--stderr={err_path}",
            udid, identifier, *args,
            check=False, timeout=600, env=env,
        )
        match = re.search(re.escape(identifier) + r": (\d+)", result.stdout)
        if result.returncode == 0 and match:
            break
        failure = f"xcrun simctl launch failed ({result.returncode}): {(result.stderr or result.stdout).strip()}"
        note(failure)
        if attempt < LAUNCH_ATTEMPTS:
            note(f"launching it again in {LAUNCH_PAUSE} s (attempt {attempt + 1} of {LAUNCH_ATTEMPTS})")
            time.sleep(LAUNCH_PAUSE)
    pid = int(match.group(1)) if match else None
    requests = Requests(udid, identifier, pid, idb)
    out = Output(out_path, sys.stdout, True, requests.handle)
    err = Output(err_path, sys.stderr, False)
    timed_out = False
    if not match:
        reason = failure
    else:
        reason = (
            "the app ended without printing `sdl3-ios-runner: exit N` (it crashed, panicked "
            "in a callback, or exited some other way)"
        )
        while alive(pid):
            out.poll()
            err.poll()
            requests.check()
            if timeout and time.monotonic() - start > timeout:
                timed_out = True
                reason = f"the app ran for more than {timeout:g} s (SDL3_IOS_TIMEOUT)"
                note(f"{reason}: terminating it")
                xcrun_simctl("terminate", udid, identifier, check=False, timeout=60)
                break
            time.sleep(0.1)
    out.poll(final=True)
    err.poll(final=True)
    out.close()
    err.close()
    if VERBOSE:
        for path in (out_path, err_path):
            size = os.path.getsize(path) if os.path.exists(path) else None
            note(f"{path}: {'missing' if size is None else f'{size} bytes'}")
        note(f"simctl launch printed {result.stdout.strip()!r} {result.stderr.strip()!r}")
    if out.status is not None and not timed_out:
        return out.status
    if match and not timed_out:
        note(reason)
    for what, output in (("output", out), ("error output", err)):
        if output.tail:
            note(f"the end of its {what}:")
            for text in output.tail[-20:]:
                print(f"  | {text}", file=sys.stderr)
    note(f'the device\'s log for process "{executable}" and {identifier}:')
    for text in device_log(udid, executable, identifier):
        print(f"  | {text}", file=sys.stderr)
    # (once more below the log, where a CI log's last lines show it)
    note(f"{identifier} failed: {reason}")
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
