#!/usr/bin/env python3
"""Run a program built for Android (x86_64-linux-android,
aarch64-linux-android, and the 32-bit i686-linux-android and
armv7-linux-androideabi) on an Android device or emulator: Cargo's runner
for those targets (.cargo/config.toml), for the test programs and the
doctests. It needs the Android SDK's platform-tools (`adb`), and to
start an emulator the SDK's emulator and a system image; the programs are
linked with the NDK by tools/android_link.py. It runs on Linux and macOS
hosts.

Usage:
  android_run.py PROGRAM [ARGUMENTS...]   run PROGRAM on the device
  android_run.py --boot                   make the emulator's virtual
                                          device when it doesn't exist
                                          yet, start the emulator and
                                          print its serial
  android_run.py --kill [SERIAL]          stop the emulator (this
                                          script's, or SERIAL's), and
                                          adb's server when no device
                                          is left

A program is spawned: pushed to the device (`adb push`, to a directory of
its own under /data/local/tmp/sdl3/run/) and run there as a process of
the shell user (`adb shell`, in the shell protocol, which keeps the
program's stdout and stderr apart and gives its exit status, 128 + N for
a signal N, which is this script's), with this script's stdin, stdout and
stderr. Its arguments are this script's after PROGRAM (a libtest
program's filters and options). adb passes on no environment, so the
variables are set on the command line: SDL3_*, SDL_*, RUST_* and
RUSTC_BOOTSTRAP as they are, HOME (/data/local/tmp/sdl3/home), TMPDIR (a
directory of the run's), and SDL_ASSERT `abort` unless it is set, so that
a failed assertion ends the program rather than waiting for an answer on
the terminal. The program and its directory are removed after it ends.

The device can't see the host's files, so the paths Cargo gives a test
program when it runs it are translated:

* CARGO_MANIFEST_DIR (and CARGO_MANIFEST_PATH), the package's directory:
  the workspace's test data, every directory named `testdata` in the
  workspace (the nearest directory up from CARGO_MANIFEST_DIR whose
  Cargo.toml has `[workspace]`, or that directory), but those in hidden
  directories and target directories, is copied once to
  /data/local/tmp/sdl3/ws/ with the workspace's layout (again when a file
  of it changes: a stamp there tells), and the variable is the package's
  directory there, which is also the program's working directory, as
  Cargo's is on the host. A test that reads its data through the
  variable at run time (falling back on `env!("CARGO_MANIFEST_DIR")`
  where nothing sets it, as a program run by hand) finds it there.
* CARGO_BIN_EXE_NAME, a binary of the package an integration test runs:
  it is pushed to the run's directory too, and the variable is its path
  there.
* CARGO_TARGET_TMPDIR (which Cargo sets when it builds an integration
  test, not when it runs it): set to /data/local/tmp/sdl3/target-tmp, for
  the tests that write there (falling back on the variable at build time
  elsewhere).
* SDL3_TEST_SKIP_LOG: the program writes its skips to a file in the run's
  directory, which is appended to the host's file when it ends.

Other paths (a program's arguments, other variables) stay as they are.

The device: SDL3_ANDROID_SERIAL's (an `adb devices` serial), else
ANDROID_SERIAL's, else the only device adb lists (or the emulator this
script started), else this script's emulator, which it starts: on the
virtual device (AVD) SDL3_ANDROID_AVD, by default `sdl3-API-TAG-ABI` of
the system image SDL3_ANDROID_SYSTEM_IMAGE (by default
`system-images;android-35;google_apis;x86_64`, which must be installed:
`sdkmanager --install 'system-images;android-35;google_apis;x86_64'`, or
its archive unpacked in the SDK's system-images/android-35/google_apis/),
written when it doesn't exist yet (the files avdmanager would write, for a
Pixel 6 with a hardware keyboard, 4 GB of memory and a 12 GB data
partition), started headless, with SwiftShader's GPU (OpenGL ES 3.0 and
Vulkan 1.3), the emulated cameras, and no snapshot saved. The first
program waits for the boot to finish (`sys.boot_completed`; a minute or
two, or SDL3_ANDROID_BOOT_TIMEOUT seconds, after which an emulator of
this script's that hasn't booted, as a hung one, is killed). `--boot`
starts it and prints the serial at once, so that it can boot while the
programs build:

  export SDL3_ANDROID_SERIAL=$(tools/android_run.py --boot)

The emulator's log, and its process ID (by which `--kill` ends a hung
emulator, which has no console to ask), are kept in the temporary
directory's sdl3-android-run-UID/SERIAL/. The virtual device is in
ANDROID_AVD_HOME (by default ANDROID_USER_HOME/avd, or ~/.android/avd),
which the emulator is given, so that it looks there (avdmanager and the
emulator may not agree on the place otherwise). On Linux the
emulator needs KVM (a /dev/kvm the user can open) and PulseAudio's
library, libpulse.so.0 (Debian's and Ubuntu's package libpulse0), even
without sound.

A program must be built for one of the device's ABIs
(`ro.product.cpu.abilist`): an x86_64 emulator's image runs arm64-v8a
programs (aarch64-linux-android) too, translating them (libndk_translation,
which binfmt_misc runs ARM executables through; about three times
slower), but the API 35 image runs no 32-bit programs.

Environment:
  SDL3_ANDROID_SERIAL          the device (an `adb devices` serial)
  SDL3_ANDROID_SYSTEM_IMAGE    the emulator's system image (an sdkmanager
                               package path)
  SDL3_ANDROID_AVD             the emulator's virtual device
  SDL3_ANDROID_EMULATOR_FLAGS  more emulator options (shell syntax)
  SDL3_ANDROID_BOOT_TIMEOUT    seconds the boot may take (default 600)
  SDL3_ANDROID_TIMEOUT         seconds a program may run (default 1800;
                               0: no limit), after which it is killed
  SDL3_ANDROID_VERBOSE         1: the adb commands, with their times, on
                               stderr
  ANDROID_HOME, ANDROID_SDK_ROOT
                               the SDK (adb and the emulator from it, else
                               from PATH)
"""
import fcntl
import hashlib
import os
import re
import shlex
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import time

PROGRAM = os.path.basename(__file__)

VERBOSE = os.environ.get("SDL3_ANDROID_VERBOSE", "") not in ("", "0")

# Where the runs are on the device: the shell user's (and adb push's) place.
DEVICE_ROOT = "/data/local/tmp/sdl3"
DEVICE_WORKSPACE = DEVICE_ROOT + "/ws"
DEVICE_HOME = DEVICE_ROOT + "/home"
DEVICE_TARGET_TMPDIR = DEVICE_ROOT + "/target-tmp"
STAMP = DEVICE_WORKSPACE + "/.sdl3-android-run-stamp"

DEFAULT_SYSTEM_IMAGE = "system-images;android-35;google_apis;x86_64"

# The variables a program gets as they are.
FORWARD_PREFIXES = ("SDL3_", "SDL_", "RUST_")
FORWARD_NAMES = ("RUSTC_BOOTSTRAP",)

# ELF e_machine: the Android ABI and the Rust target.
ABIS = {
    62: ("x86_64", "x86_64-linux-android"),
    183: ("arm64-v8a", "aarch64-linux-android"),
    3: ("x86", "i686-linux-android"),
    40: ("armeabi-v7a", "armv7-linux-androideabi"),
}

# A command line longer than this goes to the device in a script (a
# shell's `-c` argument is at most 128 KB on Linux).
LONGEST_COMMAND = 100_000


class Failure(Exception):
    """An error this script reports (and exits 1 for)."""


def note(message):
    print(f"{PROGRAM}: {message}", file=sys.stderr, flush=True)


def number(name, default):
    value = os.environ.get(name) or default
    try:
        return float(value)
    except ValueError:
        raise Failure(f"{name} isn't a number of seconds: {value!r}") from None


# ---------------------------------------------------------------------------
# The SDK's tools


def sdk_dir():
    for name in ("ANDROID_HOME", "ANDROID_SDK_ROOT"):
        if os.environ.get(name):
            return os.environ[name]
    for candidate in ("~/Android/Sdk", "~/Library/Android/sdk"):
        path = os.path.expanduser(candidate)
        if os.path.isdir(path):
            return path
    return None


def sdk_tool(*parts, name):
    """The SDK's tool at SDK/PARTS..., else NAME from PATH."""
    sdk = sdk_dir()
    if sdk:
        path = os.path.join(sdk, *parts)
        if os.path.isfile(path):
            return path
    return name


def avd_home():
    """Where the virtual devices are: ANDROID_AVD_HOME, which the emulator
    is given (avdmanager and the emulator may look in different places
    otherwise)."""
    if os.environ.get("ANDROID_AVD_HOME"):
        return os.environ["ANDROID_AVD_HOME"]
    if os.environ.get("ANDROID_USER_HOME"):
        return os.path.join(os.environ["ANDROID_USER_HOME"], "avd")
    return os.path.expanduser("~/.android/avd")


def run_tool(argv, check=True, timeout=None, env=None, stdin=None):
    """A command, its output captured (to files: a process it leaves behind,
    as `adb` starting its server does, would keep a pipe open)."""
    start = time.monotonic()
    with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
        try:
            process = subprocess.Popen(
                argv, stdin=subprocess.PIPE if stdin is not None else subprocess.DEVNULL,
                stdout=out, stderr=err, env=env,
            )
        except FileNotFoundError:
            raise Failure(f"no {argv[0]}: install the Android SDK's tools, and set ANDROID_HOME") from None
        try:
            if stdin is not None:
                process.communicate(stdin.encode(), timeout=timeout)
            returncode = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
            raise Failure(f"{shlex.join(argv)} took more than {timeout:g} s") from None
        out.seek(0)
        err.seek(0)
        result = subprocess.CompletedProcess(
            argv, returncode, out.read().decode(errors="replace"), err.read().decode(errors="replace"),
        )
    if VERBOSE:
        note(f"{shlex.join(argv)}: {result.returncode} ({time.monotonic() - start:.2f} s)")
    if check and result.returncode != 0:
        output = result.stderr.strip() or result.stdout.strip()
        raise Failure(f"{shlex.join(argv)} failed ({result.returncode}): {output}")
    return result


def adb(serial, *args, check=True, timeout=120):
    argv = [sdk_tool("platform-tools", "adb", name="adb")]
    if serial:
        argv += ["-s", serial]
    return run_tool(argv + list(args), check=check, timeout=timeout)


def adb_shell(serial, command, check=True, timeout=120):
    """`adb shell COMMAND` (in the shell protocol, without a terminal), its
    output captured."""
    return adb(serial, "shell", "-T", command, check=check, timeout=timeout)


# ---------------------------------------------------------------------------
# The program's ELF file


class Elf:
    """What this script reads of an ELF file: its machine (the ABI) and the
    words of its `.sdl3_runner` section (the way a program asks how it is
    run, as on the Apple platforms)."""

    def __init__(self, path):
        try:
            with open(path, "rb") as f:
                data = f.read()
        except OSError as e:
            raise Failure(f"can't read {path}: {e}") from None
        if len(data) < 52 or data[:4] != b"\x7fELF":
            raise Failure(f"{path} isn't an ELF file")
        bits64 = data[4] == 2
        order = "<" if data[5] == 1 else ">"
        self.machine = struct.unpack_from(order + "H", data, 18)[0]
        self.words = []
        try:
            if bits64:
                shoff, = struct.unpack_from(order + "Q", data, 40)
                shentsize, shnum, shstrndx = struct.unpack_from(order + "HHH", data, 58)
            else:
                shoff, = struct.unpack_from(order + "I", data, 32)
                shentsize, shnum, shstrndx = struct.unpack_from(order + "HHH", data, 46)
            sections = []
            for i in range(shnum):
                base = shoff + i * shentsize
                if bits64:
                    name, _, _, _, offset, size = struct.unpack_from(order + "IIQQQQ", data, base)
                else:
                    name, _, _, _, offset, size = struct.unpack_from(order + "IIIIII", data, base)
                sections.append((name, offset, size))
            if sections and shstrndx < len(sections):
                strtab = sections[shstrndx][1]
                for name, offset, size in sections:
                    end = data.index(b"\0", strtab + name)
                    if data[strtab + name:end] == b".sdl3_runner":
                        contents = data[offset:offset + size]
                        self.words += contents.decode(errors="replace").replace("\0", " ").split()
        except (struct.error, ValueError):
            raise Failure(f"{path}: a malformed ELF file") from None

    def abi(self):
        return ABIS.get(self.machine, (f"ELF machine {self.machine}", None))


# ---------------------------------------------------------------------------
# The device and the emulator


def state_root():
    return os.path.join(tempfile.gettempdir(), f"sdl3-android-run-{os.getuid()}")


class Lock:
    """An exclusive lock on a file in the temporary directory, around what
    programs run at the same time (rustdoc runs doctests in parallel)
    would otherwise both do: choosing (and starting) the emulator, copying
    the test data, appending to the skip log."""

    def __init__(self, what):
        self.what = what

    def __enter__(self):
        os.makedirs(state_root(), exist_ok=True)
        path = os.path.join(state_root(), f"{self.what}.lock")
        self.fd = os.open(path, os.O_RDWR | os.O_CREAT, 0o600)
        fcntl.flock(self.fd, fcntl.LOCK_EX)
        return self

    def __exit__(self, *exc):
        os.close(self.fd)


class Emulator:
    """An emulator this script started: its serial, and in its state
    directory its process ID (that of its process group) and log."""

    def __init__(self, serial):
        self.serial = serial
        self.dir = os.path.join(state_root(), serial)
        self.log = os.path.join(self.dir, "emulator.log")

    @staticmethod
    def started():
        try:
            names = os.listdir(state_root())
        except OSError:
            return []
        return [Emulator(n) for n in sorted(names) if os.path.isfile(os.path.join(state_root(), n, "pid"))]

    def pid(self):
        try:
            with open(os.path.join(self.dir, "pid")) as f:
                return int(f.read().strip())
        except (OSError, ValueError):
            return None

    def alive(self):
        pid = self.pid()
        if pid is None:
            return False
        try:
            # (the process group: the emulator's launcher and its qemu)
            os.killpg(pid, 0)
        except ProcessLookupError:
            return False
        except PermissionError:
            pass
        return True

    def log_tail(self, lines=40):
        try:
            with open(self.log, errors="replace") as f:
                text = f.read().splitlines()[-lines:]
        except OSError:
            return []
        return text

    def why_not(self):
        """What the emulator's log says it lacks, if it's something known."""
        text = "\n".join(self.log_tail(200))
        if "libpulse.so.0" in text:
            return "the emulator needs PulseAudio's library, libpulse.so.0 (apt-get install libpulse0)"
        if "/dev/kvm" in text:
            return "the emulator needs KVM: a /dev/kvm this user can open"
        if "Unknown AVD name" in text:
            return f"the emulator doesn't find the virtual device in ANDROID_AVD_HOME ({avd_home()})"
        return None

    def kill(self):
        """Kill it by its process ID (a hung emulator has no console to ask)."""
        pid = self.pid()
        if pid is not None:
            try:
                os.killpg(pid, signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                pass
        try:
            os.remove(os.path.join(self.dir, "pid"))
        except OSError:
            pass


def system_image():
    return os.environ.get("SDL3_ANDROID_SYSTEM_IMAGE") or DEFAULT_SYSTEM_IMAGE


def avd_name(image):
    want = os.environ.get("SDL3_ANDROID_AVD")
    if want:
        return want
    # (system-images;android-35;google_apis;x86_64: sdl3-35-google_apis-x86_64)
    parts = image.split(";")[1:]
    if parts and parts[0].startswith("android-"):
        parts[0] = parts[0][len("android-"):]
    return "sdl3-" + "-".join(re.sub(r"[^A-Za-z0-9_.]", "_", p) for p in parts)


def tool_environment():
    """The emulator's environment: ANDROID_AVD_HOME as this script's (see
    avd_home()), and the SDK (which the system image's path in the virtual
    device is relative to)."""
    env = dict(os.environ)
    env["ANDROID_AVD_HOME"] = avd_home()
    sdk = sdk_dir()
    if sdk:
        env.setdefault("ANDROID_SDK_ROOT", sdk)
    return env


def properties(path):
    """A `NAME=VALUE` file's settings (a package's source.properties)."""
    settings = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            name, equals, value = line.partition("=")
            if equals and not line.lstrip().startswith("#"):
                settings[name.strip()] = value.strip()
    return settings


# The virtual device's hardware: a Pixel 6's screen and sensors (as
# avdmanager's `pixel_6` profile has them), with a hardware keyboard,
# 4 GB of memory, a 12 GB data partition for the programs and their data,
# the emulated cameras and SwiftShader's GPU.
AVD_HARDWARE = {
    "hw.device.manufacturer": "Google",
    "hw.device.name": "pixel_6",
    "hw.lcd.width": "1080",
    "hw.lcd.height": "2400",
    "hw.lcd.density": "420",
    "hw.mainKeys": "no",
    "hw.dPad": "no",
    "hw.trackBall": "no",
    "hw.keyboard": "yes",
    "hw.accelerometer": "yes",
    "hw.gyroscope": "yes",
    "hw.sensors.orientation": "yes",
    "hw.sensors.proximity": "yes",
    "hw.sensors.light": "yes",
    "hw.sensors.magnetic_field": "yes",
    "hw.sensors.pressure": "yes",
    "hw.gps": "yes",
    "hw.battery": "yes",
    "hw.audioInput": "yes",
    "hw.audioOutput": "yes",
    "hw.camera.back": "emulated",
    "hw.camera.front": "emulated",
    "hw.gpu.enabled": "yes",
    "hw.gpu.mode": "swiftshader_indirect",
    "hw.ramSize": "4096",
    "disk.dataPartition.size": "12G",
}


def create_avd(name, image):
    """Write the virtual device for the system image, unless it exists (as
    avdmanager would, without needing its packages' records: the emulator
    reads only these two files and the image's)."""
    home = avd_home()
    ini = os.path.join(home, name + ".ini")
    if os.path.isfile(ini):
        return
    sdk = sdk_dir()
    if not sdk:
        raise Failure("no Android SDK: set ANDROID_HOME")
    parts = image.split(";")
    sysdir = os.path.join(sdk, *parts)
    try:
        info = properties(os.path.join(sysdir, "source.properties"))
    except OSError:
        raise Failure(f"the system image {image} isn't installed in {sdk}: sdkmanager --install {shlex.quote(image)}") from None
    abi = info.get("SystemImage.Abi") or parts[-1]
    api = info.get("AndroidVersion.ApiLevel") or parts[1].removeprefix("android-")
    tag = info.get("SystemImage.TagId") or parts[2]
    avd = os.path.join(home, name + ".avd")
    os.makedirs(avd, exist_ok=True)
    config = {
        "AvdId": name,
        "avd.ini.displayname": name,
        "avd.ini.encoding": "UTF-8",
        "abi.type": abi,
        "hw.cpu.arch": {"arm64-v8a": "arm64", "armeabi-v7a": "arm", "x86": "x86"}.get(abi, abi),
        "hw.cpu.ncore": str(min(4, os.cpu_count() or 1)),
        "image.sysdir.1": "/".join(parts) + "/",
        "tag.id": tag,
        "tag.display": info.get("SystemImage.TagDisplay", tag),
        "target": f"android-{api}",
        **AVD_HARDWARE,
    }
    with open(os.path.join(avd, "config.ini"), "w") as f:
        f.writelines(f"{k}={v}\n" for k, v in config.items())
    with open(ini, "w") as f:
        f.write(f"avd.ini.encoding=UTF-8\npath={avd}\ntarget=android-{api}\n")
    if VERBOSE:
        note(f"made the virtual device {name} ({image}) in {home}")


def free_console_port():
    """An emulator console port (even, 5554 to 5584) that is free with the
    adb port after it."""
    for port in range(5554, 5586, 2):
        sockets = []
        try:
            for p in (port, port + 1):
                s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
                sockets.append(s)
                s.bind(("127.0.0.1", p))
            return port
        except OSError:
            continue
        finally:
            for s in sockets:
                s.close()
    raise Failure("no free emulator console port (5554 to 5584)")


def start_emulator():
    """Start this script's emulator (making its virtual device first):
    its serial."""
    image = system_image()
    name = avd_name(image)
    create_avd(name, image)
    port = free_console_port()
    emulator = Emulator(f"emulator-{port}")
    os.makedirs(emulator.dir, exist_ok=True)
    argv = [
        sdk_tool("emulator", "emulator", name="emulator"),
        "-avd", name, "-port", str(port),
        "-no-window", "-no-boot-anim", "-no-metrics", "-no-snapshot-save",
        "-gpu", "swiftshader_indirect",
        "-camera-back", "emulated", "-camera-front", "emulated",
        *shlex.split(os.environ.get("SDL3_ANDROID_EMULATOR_FLAGS", "")),
    ]
    with open(emulator.log, "w") as log:
        try:
            process = subprocess.Popen(
                argv, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT,
                env=tool_environment(), start_new_session=True,
            )
        except FileNotFoundError:
            raise Failure(f"no emulator ({argv[0]}): sdkmanager --install emulator, and set ANDROID_HOME") from None
    with open(os.path.join(emulator.dir, "pid"), "w") as f:
        f.write(f"{process.pid}\n")
    note(f"started the emulator on {name} as {emulator.serial} (its log: {emulator.log})")
    return emulator.serial


def listed_devices():
    """adb's devices: serial -> state (`device`, `offline`, ...)."""
    devices = {}
    for line in adb(None, "devices").stdout.splitlines()[1:]:
        fields = line.split()
        if len(fields) >= 2:
            devices[fields[0]] = fields[1]
    return devices


def find_device():
    """The device's serial (this script's emulator started when there's
    none)."""
    serial = os.environ.get("SDL3_ANDROID_SERIAL") or os.environ.get("ANDROID_SERIAL")
    if serial:
        return serial
    with Lock("device"):
        devices = listed_devices()
        ours = [e.serial for e in Emulator.started() if e.alive()]
        # (a device still booting may be listed offline, or not yet)
        candidates = sorted(set(s for s, state in devices.items() if state in ("device", "offline")) | set(ours))
        if len(candidates) == 1:
            return candidates[0]
        if len(candidates) > 1:
            raise Failure("several devices (" + ", ".join(candidates) + "): set SDL3_ANDROID_SERIAL to one")
        return start_emulator()


def device_properties(serial):
    """Whether the device has booted, and its ABIs (None until it's up)."""
    result = adb_shell(serial, "getprop sys.boot_completed; getprop ro.product.cpu.abilist", check=False, timeout=30)
    lines = result.stdout.splitlines()
    if result.returncode != 0 or len(lines) < 2 or lines[0].strip() != "1":
        return None
    return [abi for abi in lines[1].strip().split(",") if abi]


def wait_for_boot(serial):
    """The device's ABIs, once it has booted (waited for)."""
    abis = device_properties(serial)
    if abis is not None:
        return abis
    timeout = number("SDL3_ANDROID_BOOT_TIMEOUT", 600)
    emulator = Emulator(serial)
    ours = emulator.pid() is not None
    start = time.monotonic()
    note(f"waiting for {serial} to boot")
    while True:
        if ours and not emulator.alive():
            for line in emulator.log_tail():
                print(f"  | {line}", file=sys.stderr)
            why = emulator.why_not()
            raise Failure(f"the emulator {serial} ended while booting" + (f": {why}" if why else " (its log above)"))
        abis = device_properties(serial)
        if abis is not None:
            break
        if time.monotonic() - start > timeout:
            if ours:
                note(f"killing the emulator {serial}, which hasn't booted in {timeout:g} s")
                for line in emulator.log_tail():
                    print(f"  | {line}", file=sys.stderr)
                emulator.kill()
            raise Failure(f"{serial} didn't boot in {timeout:g} s (SDL3_ANDROID_BOOT_TIMEOUT)")
        time.sleep(2)
    note(f"{serial} booted in {time.monotonic() - start:.0f} s")
    return abis


def cmd_boot(args):
    if args:
        raise Failure("usage: --boot")
    with Lock("device"):
        # (this script's emulator when it's still there, booted or booting;
        # one that is hung is killed by the first program's wait)
        for emulator in Emulator.started():
            if emulator.alive():
                print(emulator.serial, flush=True)
                return 0
        serial = start_emulator()
    print(serial, flush=True)
    return 0


def cmd_kill(args):
    if len(args) > 1:
        raise Failure("usage: --kill [SERIAL]")
    serial = args[0] if args else os.environ.get("SDL3_ANDROID_SERIAL")
    emulators = [Emulator(serial)] if serial else [e for e in Emulator.started() if e.alive()]
    if not emulators:
        note("no emulator of this script's is running")
        return 0
    for emulator in emulators:
        adb(emulator.serial, "emu", "kill", check=False, timeout=30)
        deadline = time.monotonic() + 60
        while emulator.pid() is not None and emulator.alive() and time.monotonic() < deadline:
            time.sleep(1)
        if emulator.pid() is not None and emulator.alive():
            note(f"the emulator {emulator.serial} didn't stop: killing it")
        emulator.kill()
    # (and adb's server, when it has no device left)
    if not listed_devices():
        adb(None, "kill-server", check=False, timeout=30)
    return 0


# ---------------------------------------------------------------------------
# The test data


def workspace_root(manifest_dir):
    """The workspace the package is in: the nearest directory from
    MANIFEST_DIR up whose Cargo.toml has `[workspace]` (as Cargo finds
    it), else MANIFEST_DIR."""
    directory = manifest_dir
    while True:
        try:
            with open(os.path.join(directory, "Cargo.toml"), encoding="utf-8") as f:
                if re.search(r"^\[workspace\]", f.read(), re.M):
                    return directory
        except OSError:
            pass
        parent = os.path.dirname(directory)
        if parent == directory:
            return manifest_dir
        directory = parent


def testdata_dirs(root):
    """The workspace's `testdata` directories (relative paths), but those in
    hidden directories and the target directory."""
    found = []
    for directory, subdirs, _ in os.walk(root):
        relative = os.path.relpath(directory, root)
        if os.path.basename(directory) == "testdata":
            found.append(relative)
            subdirs[:] = []
            continue
        subdirs[:] = sorted(
            d for d in subdirs
            if not d.startswith(".") and not (relative == "." and d == "target")
            and not os.path.isfile(os.path.join(directory, d, "CACHEDIR.TAG"))
        )
    return found


def testdata_stamp(root, dirs):
    """A digest of the test data's files: their paths, sizes and times."""
    digest = hashlib.sha256(root.encode())
    for relative in dirs:
        for directory, subdirs, files in os.walk(os.path.join(root, relative)):
            subdirs.sort()
            for name in sorted(files):
                path = os.path.join(directory, name)
                info = os.stat(path)
                digest.update(f"{os.path.relpath(path, root)}\0{info.st_size}\0{info.st_mtime_ns}\n".encode())
    return digest.hexdigest()


def copy_testdata(serial, root):
    """The workspace's test data on the device, in DEVICE_WORKSPACE (copied
    when its stamp there isn't the host's)."""
    dirs = testdata_dirs(root)
    stamp = testdata_stamp(root, dirs)
    with Lock("testdata"):
        current = adb_shell(serial, f"cat {STAMP} 2>/dev/null", check=False).stdout.strip()
        if current == stamp:
            return
        start = time.monotonic()
        parents = sorted({posix_join(DEVICE_WORKSPACE, os.path.dirname(d)) for d in dirs} | {DEVICE_WORKSPACE})
        adb_shell(serial, f"rm -rf {DEVICE_WORKSPACE} && mkdir -p " + " ".join(shlex.quote(p) for p in parents))
        for relative in dirs:
            # (into its existing parent directory: `adb push DIR PARENT/`
            # makes PARENT/DIR)
            parent = posix_join(DEVICE_WORKSPACE, os.path.dirname(relative))
            adb(serial, "push", os.path.join(root, relative), parent + "/", timeout=600)
        adb_shell(serial, f"echo {stamp} > {STAMP}")
        if VERBOSE:
            note(f"copied {len(dirs)} testdata directories to {DEVICE_WORKSPACE} in {time.monotonic() - start:.1f} s")


def posix_join(base, relative):
    relative = relative.replace(os.sep, "/")
    return base if relative in ("", ".") else f"{base}/{relative}"


# ---------------------------------------------------------------------------
# Running


class Run:
    """A program's run on the device, in a directory of its own."""

    def __init__(self, serial, program, args):
        self.serial = serial
        self.program = program
        self.args = args
        self.dir = f"{DEVICE_ROOT}/run/{os.getpid()}"
        self.device_program = f"{self.dir}/{os.path.basename(program)}"
        self.skip_log = os.environ.get("SDL3_TEST_SKIP_LOG")

    def environment(self):
        """The program's variables, with the paths translated (see above),
        and its working directory."""
        env = {}
        for name, value in os.environ.items():
            if name.startswith(FORWARD_PREFIXES) or name in FORWARD_NAMES:
                env[name] = value
        env.setdefault("SDL_ASSERT", "abort")
        env["HOME"] = DEVICE_HOME
        env["TMPDIR"] = f"{self.dir}/tmp"
        env["CARGO_TARGET_TMPDIR"] = DEVICE_TARGET_TMPDIR
        if self.skip_log:
            env["SDL3_TEST_SKIP_LOG"] = f"{self.dir}/skips.tsv"
        cwd = self.dir
        manifest = os.environ.get("CARGO_MANIFEST_DIR")
        if manifest and os.path.isdir(manifest):
            manifest = os.path.realpath(manifest)
            root = workspace_root(manifest)
            copy_testdata(self.serial, root)
            cwd = posix_join(DEVICE_WORKSPACE, os.path.relpath(manifest, root))
            env["CARGO_MANIFEST_DIR"] = cwd
            path = os.environ.get("CARGO_MANIFEST_PATH")
            if path:
                env["CARGO_MANIFEST_PATH"] = posix_join(cwd, os.path.basename(path))
        for name, value in os.environ.items():
            if name.startswith("CARGO_BIN_EXE_") and os.path.isfile(value):
                device = f"{self.dir}/exe/{name[len('CARGO_BIN_EXE_'):]}"
                adb(self.serial, "push", value, device, timeout=600)
                env[name] = device
        return env, cwd

    def command(self, env, cwd):
        assignments = " ".join(f"{name}={shlex.quote(value)}" for name, value in sorted(env.items()))
        mkdirs = " ".join(shlex.quote(d) for d in (cwd, env["TMPDIR"], DEVICE_HOME, DEVICE_TARGET_TMPDIR))
        # (its process ID first, for a kill after the timeout: the shell's,
        # which exec makes the program's)
        return (
            f"echo $$ > {self.dir}/pid && mkdir -p {mkdirs} && cd {shlex.quote(cwd)} && "
            f"{assignments} exec {shlex.quote(self.device_program)} {shlex.join(self.args)}"
        )

    def run(self):
        adb(self.serial, "push", self.program, self.device_program, timeout=600)
        try:
            env, cwd = self.environment()
            command = self.command(env, cwd)
            if len(command) > LONGEST_COMMAND:
                with tempfile.NamedTemporaryFile("w", suffix=".sh", delete=False) as script:
                    script.write(command + "\n")
                try:
                    adb(self.serial, "push", script.name, f"{self.dir}/run.sh")
                finally:
                    os.remove(script.name)
                command = f"sh {self.dir}/run.sh"
            return self.spawn(command)
        finally:
            self.clean_up()

    def spawn(self, command):
        timeout = number("SDL3_ANDROID_TIMEOUT", 1800)
        argv = [sdk_tool("platform-tools", "adb", name="adb"), "-s", self.serial, "shell", "-T", command]
        if VERBOSE:
            note(f"running {self.device_program} {shlex.join(self.args)} on {self.serial}")
        sys.stdout.flush()
        sys.stderr.flush()
        process = subprocess.Popen(argv)
        try:
            status = process.wait(timeout=timeout or None)
            # (adb ended by a signal: as a shell gives it)
            return 128 - status if status < 0 else status
        except subprocess.TimeoutExpired:
            note(f"{os.path.basename(self.program)} ran for more than {timeout:g} s (SDL3_ANDROID_TIMEOUT): killing it")
            return 124
        finally:
            if process.poll() is None:
                # (the program too: adb ending ends it, and the kill is for
                # one that doesn't)
                process.kill()
                process.wait()
                adb_shell(self.serial, f"kill -9 $(cat {self.dir}/pid) 2>/dev/null", check=False, timeout=30)

    def clean_up(self):
        result = adb_shell(
            self.serial, f"cat {self.dir}/skips.tsv 2>/dev/null; rm -rf {self.dir}", check=False, timeout=60,
        )
        if self.skip_log and result.stdout:
            with Lock("skips"):
                with open(self.skip_log, "a") as f:
                    f.write(result.stdout)


def interrupted(signum, frame):
    raise KeyboardInterrupt


def cmd_run(program, args):
    if not os.path.isfile(program):
        raise Failure(f"no program {program}")
    program = os.path.abspath(program)
    elf = Elf(program)
    abi, target = elf.abi()
    if target is None:
        raise Failure(f"{program} is built for {abi}, which isn't one of Android's ABIs")
    if "app" in elf.words:
        raise Failure(f"{program} asks to run as an app, which this script can't do yet")
    serial = find_device()
    abis = wait_for_boot(serial)
    if abi not in abis:
        raise Failure(
            f"{program} is built for {abi} ({target}), which {serial} doesn't run: its ABIs are "
            + (", ".join(abis) or "none")
            + " (ro.product.cpu.abilist)"
        )
    signal.signal(signal.SIGTERM, interrupted)
    try:
        return Run(serial, program, args).run()
    except KeyboardInterrupt:
        return 130


def main():
    if len(sys.argv) < 2 or sys.argv[1] in ("-h", "--help"):
        print(__doc__.split("\n\n", 2)[1], file=sys.stderr)
        return 2
    try:
        if sys.argv[1] == "--boot":
            return cmd_boot(sys.argv[2:])
        if sys.argv[1] == "--kill":
            return cmd_kill(sys.argv[2:])
        return cmd_run(sys.argv[1], sys.argv[2:])
    except Failure as e:
        note(str(e))
        return 1


if __name__ == "__main__":
    sys.exit(main())
