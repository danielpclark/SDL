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
  android_run.py --apk PROGRAM OUT.apk    write the app package (APK) of
                                          PROGRAM, an app's program or
                                          libmain.so
  android_run.py --boot                   make the emulator's virtual
                                          device when it doesn't exist
                                          yet, start the emulator and
                                          print its serial
  android_run.py --kill [SERIAL]          stop the emulator (this
                                          script's, or SERIAL's), and
                                          adb's server when no device
                                          is left

A program is spawned (unless it asks to run as an app, below): pushed to the device (`adb push`, to a directory of
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

A program runs as an app when it asks for it: a section `.sdl3_runner` in
its ELF file holding the word `app` (a `#[used] #[link_section =
".sdl3_runner"]` static of words separated by spaces or NULs, as
sdl3/tests/android_runner.rs has), which tools/android_link.py links as an
app's libmain.so (a shared library whose `SDL_main` is the program's
`main`, with sdl3's `JNI_OnLoad`). The program is put in an app package
(APK, as `--apk` writes it), with no Gradle: the SDK's build-tools
(SDL3_ANDROID_BUILD_TOOLS's version, by default 35.0.0, the one CI uses, or
the newest installed) link a manifest written here (upstream's
android-project's: its features and permissions, its theme, its activity
`org.libsdl.app.SDLActivity` and that activity's attributes, debuggable, and
the INTERNET permission, which the runner's sockets below need) against the
platform's android.jar (SDL3_ANDROID_PLATFORM's API level, by default 35,
or the newest installed) with `aapt2`, then the DEX file of the app's
classes (`classes.dex`, which sdl3 keeps in the program's section
`.sdl3_android_dex`) and the program (`lib/ABI/libmain.so`, uncompressed)
are added, the package aligned (`zipalign -P 16`, for 16 KB pages) and
signed (`apksigner`, with the debug key of SDL3_ANDROID_KEYSTORE, by
default ~/.android/debug.keystore, which is made with the JDK's `keytool`
when it doesn't exist: alias androiddebugkey, password android). Its
package name is org.libsdl.sdl3_rust.NAME (NAME the program's, without
Cargo's hash), or SDL3_ANDROID_PACKAGE.

It is installed (`adb install`) and launched (`am start -W`), with the
variables a spawned program gets but HOME, TMPDIR and CARGO_TARGET_TMPDIR
(an app has its own directories), as the manifest's meta-data
`SDL_ENV.NAME`, which upstream's SDLActivity sets as environment variables
(`getManifestEnvironmentVariables()`) when SDL first reads one. (aapt2
types a value that reads as a number, a boolean, a color or a resource
reference, so a variable whose value wouldn't come through as it is, as
`01`, `+1` or `#fff`, is left out, with a note.) An app's stdout and stderr
go nowhere, so the runner listens on two TCP ports, which the device
reaches through `adb reverse`, and passes them in SDL3_ANDROID_RUNNER_STDIO
(`OUT,ERR`): the program connects to them first thing and makes them its
fds 1 and 2 (sdl3/tests/android_support/). Its arguments are
SDL3_ANDROID_RUNNER_ARGS (`N:` then the N arguments, percent-encoded,
separated by commas): upstream gives an app's `SDL_main` the arguments
`app_process` and those of SDLActivity's `getArguments()`, none. adb
doesn't give an app's exit status, so the program ends by printing
`sdl3-android-runner: exit N` on a line of its own: N is this script's
exit status (the line isn't copied). The app is then stopped (`am
force-stop`) and uninstalled. Without that line (a crash, an uncaught
panic, which aborts in a native method, an app that returned from main and
stays up until the timeout) the run fails, and the end of the device's log
follows, then the reason once more. A process that ends before its launch
is complete is launched again by Android, which would run the program a
second time: the second connection is refused, and the run fails. So does
one whose package ART didn't verify (a `VerifyError`, a rejected class, a
failed dexopt in the device's log). With SDL3_ANDROID_VERBOSE=1 the
dexopt's result is noted.

A running app can ask for what only the host can do, with a line
`sdl3-android-runner: request WHAT [ARGUMENTS]` (copied, then acted on):
  background  the HOME key (`input keyevent KEYCODE_HOME`), which sends the
              app to the background
  foreground  the app's activity launched again (`am start`), which brings
              it back, the same process
  input ARGUMENTS
              `adb shell input ARGUMENTS` (shell words: `tap X Y`,
              `swipe X1 Y1 X2 Y2`, `keyevent CODE`, `text TEXT`, ...)
The app waits for what follows itself.

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
  SDL3_ANDROID_BUILD_TOOLS     the SDK's build-tools version an APK is
                               made with (default 35.0.0, else the newest)
  SDL3_ANDROID_PLATFORM        the API level of the android.jar an APK's
                               manifest is linked against (default 35,
                               else the newest)
  SDL3_ANDROID_KEYSTORE        the debug keystore APKs are signed with
                               (default ~/.android/debug.keystore)
  SDL3_ANDROID_PACKAGE         an app's package name (default
                               org.libsdl.sdl3_rust.NAME)
  ANDROID_HOME, ANDROID_SDK_ROOT
                               the SDK (adb and the emulator from it, else
                               from PATH; the build-tools and platforms from
                               it)
  JAVA_HOME                    the JDK of keytool and apksigner (else PATH's)
"""
import fcntl
import hashlib
import os
import re
import selectors
import shlex
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import time
import urllib.parse
import zipfile

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
    """What this script reads of an ELF file: its machine (the ABI), the
    words of its `.sdl3_runner` section (the way a program asks how it is
    run, as on the Apple platforms) and the DEX file of its
    `.sdl3_android_dex` section (sdl3's, the app's classes)."""

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
        self.dex = None
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
                    elif data[strtab + name:end] == b".sdl3_android_dex":
                        self.dex = data[offset:offset + size]
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


# ---------------------------------------------------------------------------
# Apps

SENTINEL = re.compile(r"sdl3-android-runner: exit (-?\d+)")
REQUEST = re.compile(r"sdl3-android-runner: request (\S+)(?: (.*))?")

# The app's activity: upstream's, which sdl3's DEX file declares.
ACTIVITY = "org.libsdl.app.SDLActivity"
PACKAGE_PREFIX = "org.libsdl.sdl3_rust."

# Upstream's minSdkVersion (android-project's), and the build-tools and
# platform CI makes the packages with.
MIN_SDK = 21
PREFERRED_BUILD_TOOLS = "35.0.0"
PREFERRED_PLATFORM = 35

# The variables an app doesn't get: it has its own directories.
NOT_FOR_APPS = ("HOME", "TMPDIR", "CARGO_TARGET_TMPDIR", "SDL3_TEST_SKIP_LOG")

# How long a launched app may take to connect to the runner's sockets
# while its process isn't there (it never started, or died at once).
CONNECT_TIMEOUT = 30

# The device's log lines that say ART didn't verify the app's classes.
VERIFY_FAILURE = re.compile(
    r"VerifyError|Rejecting class|[Vv]erification (of .*)?failed|[Vv]erification error|failed verification"
    r"|Dexopt result.*status=FAILED"
)


def version_key(name):
    return tuple(int(p) if p.isdigit() else -1 for p in re.split(r"[.\-]", name))


def build_tools():
    """The SDK's build-tools directory APKs are made with."""
    sdk = sdk_dir()
    if not sdk:
        raise Failure("no Android SDK (for the build-tools): set ANDROID_HOME")
    base = os.path.join(sdk, "build-tools")
    want = os.environ.get("SDL3_ANDROID_BUILD_TOOLS")
    if want:
        path = os.path.join(base, want)
        if not os.path.isfile(os.path.join(path, "aapt2")):
            raise Failure(f"no build-tools {want} in {base} (SDL3_ANDROID_BUILD_TOOLS): sdkmanager --install 'build-tools;{want}'")
        return path
    preferred = os.path.join(base, PREFERRED_BUILD_TOOLS)
    if os.path.isfile(os.path.join(preferred, "aapt2")):
        return preferred
    try:
        found = [n for n in os.listdir(base) if os.path.isfile(os.path.join(base, n, "aapt2"))]
    except OSError:
        found = []
    if not found:
        raise Failure(f"no build-tools in {base}: sdkmanager --install 'build-tools;{PREFERRED_BUILD_TOOLS}'")
    return os.path.join(base, max(found, key=version_key))


def platform_jar():
    """The platform's android.jar the manifest is linked against, and its
    API level."""
    sdk = sdk_dir()
    if not sdk:
        raise Failure("no Android SDK (for a platform's android.jar): set ANDROID_HOME")
    base = os.path.join(sdk, "platforms")
    found = {}
    try:
        for name in os.listdir(base):
            match = re.fullmatch(r"android-(\d+)", name)
            if match and os.path.isfile(os.path.join(base, name, "android.jar")):
                found[int(match.group(1))] = os.path.join(base, name, "android.jar")
    except OSError:
        pass
    want = os.environ.get("SDL3_ANDROID_PLATFORM")
    if want:
        if not want.isdigit() or int(want) not in found:
            raise Failure(f"no platform android-{want} in {base} (SDL3_ANDROID_PLATFORM): sdkmanager --install 'platforms;android-{want}'")
        return found[int(want)], int(want)
    if PREFERRED_PLATFORM in found:
        return found[PREFERRED_PLATFORM], PREFERRED_PLATFORM
    if not found:
        raise Failure(f"no platform in {base}: sdkmanager --install 'platforms;android-{PREFERRED_PLATFORM}'")
    api = max(found)
    return found[api], api


def jdk_tool(name):
    """A JDK tool (keytool): JAVA_HOME's, else PATH's."""
    home = os.environ.get("JAVA_HOME")
    if home and os.path.isfile(os.path.join(home, "bin", name)):
        return os.path.join(home, "bin", name)
    path = shutil.which(name)
    if not path:
        raise Failure(f"no {name}: install a JDK, and set JAVA_HOME")
    return path


def debug_keystore():
    """The debug keystore APKs are signed with (made when it doesn't exist,
    as the SDK's tools make it)."""
    path = os.environ.get("SDL3_ANDROID_KEYSTORE") or os.path.expanduser("~/.android/debug.keystore")
    with Lock("keystore"):
        if not os.path.isfile(path):
            os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
            run_tool([
                jdk_tool("keytool"), "-genkeypair", "-keystore", path, "-storepass", "android",
                "-keypass", "android", "-alias", "androiddebugkey", "-keyalg", "RSA", "-keysize", "2048",
                "-validity", "10000", "-dname", "CN=Android Debug,O=Android,C=US",
            ], timeout=120)
            note(f"made the debug keystore {path}")
    return path


def program_name(program):
    """The program's name, without Cargo's hash (`android_runner-0123456789abcdef`)."""
    name = os.path.basename(program)
    name = re.sub(r"\.so$", "", name)
    return re.sub(r"-[0-9a-f]{16}$", "", name)


def package_name(program):
    want = os.environ.get("SDL3_ANDROID_PACKAGE")
    if want:
        return want
    name = re.sub(r"[^A-Za-z0-9_]", "_", program_name(program))
    if not name or name[0].isdigit():
        name = "_" + name
    return PACKAGE_PREFIX + name


INTEGER = re.compile(r"[-+]?(?:\d+|0[xX][0-9a-fA-F]+)")
FLOATING = re.compile(
    r"[-+]?(?:(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?"
    r"|0[xX](?:[0-9a-fA-F]+\.?[0-9a-fA-F]*|\.[0-9a-fA-F]+)(?:[pP][-+]?\d+)?"
    r"|(?i:inf|infinity|nan))"
)


def manifest_value_problem(value):
    """Why `value` wouldn't come through a manifest's `android:value` as the
    same string (aapt2 types a value that reads as a reference, a color, a
    boolean, an integer or a floating-point number, whose `toString()` is
    what the app gets), or None."""
    if any(ord(c) < 0x20 and c not in "\t\n\r" for c in value):
        return "it has a control character, which XML can't have"
    if value != value.strip():
        return "it has whitespace at an end"
    if value[:1] in ("@", "?"):
        return "it reads as a resource reference"
    if re.fullmatch(r"#(?:[0-9a-fA-F]{3,4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})", value):
        return "it reads as a color"
    if value.lower() in ("true", "false"):
        return None if value in ("true", "false") else f"it reads as the boolean {value.lower()}"
    if INTEGER.fullmatch(value):
        digits = value.lstrip("+-")
        number = int(digits, 16) if digits[:2].lower() == "0x" else int(digits)
        number = -number if value.startswith("-") else number
        if not -2**31 <= number < 2**31:
            return "it reads as a floating-point number"
        return None if str(number) == value else f"it reads as the integer {number}"
    if FLOATING.fullmatch(value):
        return "it reads as a floating-point number"
    return None


def encode_arguments(args):
    """SDL3_ANDROID_RUNNER_ARGS: `N:` then the arguments, percent-encoded,
    separated by commas."""
    return f"{len(args)}:" + ",".join(urllib.parse.quote(a, safe="") for a in args)


def xml_attribute(value):
    return (
        value.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace('"', "&quot;")
        .replace("\n", "&#10;").replace("\r", "&#13;").replace("\t", "&#9;")
    )


def manifest_value(value):
    """A value as a manifest's `android:value`: aapt2 reads backslash
    escapes there (`\\\\` is a backslash), as in a resource string."""
    return xml_attribute(value.replace("\\", "\\\\"))


def manifest(package, env):
    """The app's AndroidManifest.xml: upstream's android-project's (its
    features and permissions, its application's and activity's
    attributes), with this package's name, the INTERNET permission (for the
    runner's sockets) and the variables as `SDL_ENV.NAME` meta-data."""
    meta = "".join(
        f'        <meta-data android:name="SDL_ENV.{xml_attribute(name)}" android:value="{manifest_value(value)}" />\n'
        for name, value in sorted(env.items())
    )
    return f'''<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    package="{package}"
    android:versionCode="1"
    android:versionName="1.0"
    android:installLocation="auto">
    <uses-feature android:glEsVersion="0x00020000" />
    <uses-feature android:name="android.hardware.touchscreen" android:required="false" />
    <uses-feature android:name="android.hardware.bluetooth" android:required="false" />
    <uses-feature android:name="android.hardware.gamepad" android:required="false" />
    <uses-feature android:name="android.hardware.usb.host" android:required="false" />
    <uses-feature android:name="android.hardware.type.pc" android:required="false" />
    <uses-permission android:name="android.permission.VIBRATE" />
    <uses-permission android:name="android.permission.INTERNET" />
    <application android:label="@string/app_name"
        android:allowBackup="true"
        android:theme="@style/AppTheme"
        android:enableOnBackInvokedCallback="true"
        android:hardwareAccelerated="true">
{meta}        <activity android:name="{ACTIVITY}"
            android:label="@string/app_name"
            android:alwaysRetainTaskState="true"
            android:launchMode="singleTop"
            android:configChanges="layoutDirection|locale|grammaticalGender|fontScale|fontWeightAdjustment|orientation|uiMode|screenLayout|screenSize|smallestScreenSize|keyboard|keyboardHidden|navigation"
            android:preferMinimalPostProcessing="true"
            android:exported="true">
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
            <intent-filter>
                <action android:name="android.hardware.usb.action.USB_DEVICE_ATTACHED" />
            </intent-filter>
        </activity>
    </application>
</manifest>
'''


def resources(name):
    """The app's resources: upstream's app name string (the program's name
    here) and its theme."""
    return {
        "values/strings.xml": f'''<?xml version="1.0" encoding="utf-8"?>
<resources>
    <string name="app_name">{xml_attribute(name)}</string>
</resources>
''',
        "values/styles.xml": '''<?xml version="1.0" encoding="utf-8"?>
<resources>
    <style name="AppTheme" parent="android:Theme.NoTitleBar">
        <item name="android:windowDrawsSystemBarBackgrounds">true</item>
        <item name="android:statusBarColor">@android:color/transparent</item>
        <item name="android:navigationBarColor">@android:color/transparent</item>
    </style>
</resources>
''',
    }


def write_apk(program, elf, out, env, temp):
    """Write PROGRAM's APK to OUT, with the variables `env` (see above): its
    package name."""
    abi, _ = elf.abi()
    if elf.dex is None:
        raise Failure(f"{program} has no DEX file of the app's classes (a section .sdl3_android_dex): does it link sdl3?")
    tools = build_tools()
    jar, api = platform_jar()
    package = package_name(program)
    start = time.monotonic()
    manifest_path = os.path.join(temp, "AndroidManifest.xml")
    with open(manifest_path, "w", encoding="utf-8") as f:
        f.write(manifest(package, env))
    compiled = os.path.join(temp, "compiled")
    os.makedirs(compiled, exist_ok=True)
    for relative, text in resources(program_name(program)).items():
        path = os.path.join(temp, "res", relative)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)
        run_tool([os.path.join(tools, "aapt2"), "compile", "-o", compiled, path], timeout=300)
    flats = sorted(os.path.join(compiled, n) for n in os.listdir(compiled))
    base = os.path.join(temp, "base.apk")
    run_tool([
        os.path.join(tools, "aapt2"), "link", "-o", base, "--manifest", manifest_path, "-I", jar,
        "--min-sdk-version", str(MIN_SDK), "--target-sdk-version", str(api), "--debug-mode", *flats,
    ], timeout=300)
    unsigned = os.path.join(temp, "unsigned.apk")
    with zipfile.ZipFile(base) as source, zipfile.ZipFile(unsigned, "w") as zip_out:
        for item in source.infolist():
            zip_out.writestr(item, source.read(item.filename))
        zip_out.writestr(zipfile.ZipInfo("classes.dex"), elf.dex, compress_type=zipfile.ZIP_DEFLATED)
        # (uncompressed, so that zipalign page-aligns it)
        zip_out.write(program, f"lib/{abi}/libmain.so", compress_type=zipfile.ZIP_STORED)
    aligned = os.path.join(temp, "aligned.apk")
    run_tool([os.path.join(tools, "zipalign"), "-P", "16", "-f", "4", unsigned, aligned], timeout=300)
    run_tool([
        os.path.join(tools, "apksigner"), "sign", "--ks", debug_keystore(), "--ks-pass", "pass:android",
        "--key-pass", "pass:android", "--ks-key-alias", "androiddebugkey", "--out", out, aligned,
    ], timeout=300)
    if VERBOSE:
        note(f"wrote {out} ({package}, {os.path.getsize(out)} bytes) in {time.monotonic() - start:.1f} s")
    return package


def cmd_apk(args):
    if len(args) != 2:
        raise Failure("usage: --apk PROGRAM OUT.apk")
    program, out = args
    if not os.path.isfile(program):
        raise Failure(f"no program {program}")
    elf = Elf(program)
    if elf.abi()[1] is None:
        raise Failure(f"{program} is built for {elf.abi()[0]}, which isn't one of Android's ABIs")
    with tempfile.TemporaryDirectory(prefix="sdl3-android-apk-") as temp:
        package = write_apk(os.path.abspath(program), elf, os.path.abspath(out), {}, temp)
    print(f"{out}: {package}")
    return 0


def app_environment(run, args, ports):
    """The app's variables: a spawned program's (with the test data copied),
    but those of its directories, plus the runner's sockets and the
    arguments; those that wouldn't come through the manifest as they are
    left out, with a note."""
    env, _ = run.environment()
    for name in NOT_FOR_APPS:
        env.pop(name, None)
    env["SDL3_ANDROID_RUNNER_STDIO"] = ",".join(str(p) for p in ports)
    env["SDL3_ANDROID_RUNNER_ARGS"] = encode_arguments(args)
    for name, value in sorted(env.items()):
        problem = manifest_value_problem(value)
        if problem:
            note(f"{name}={value!r} isn't passed on to the app: {problem}, which the manifest would change")
            del env[name]
    return env


class Stream:
    """One of the app's output streams, a connection to the runner's socket:
    copied to `out` by lines, the `sdl3-android-runner: exit N` line kept
    out (and noted) when `sentinel`, and the `sdl3-android-runner: request
    WHAT [ARGUMENTS]` lines passed to `on_request` (after they're copied)."""

    def __init__(self, name, listener, out, sentinel, on_request=None):
        self.name = name
        self.listener = listener
        self.out = out
        self.sentinel = sentinel
        self.on_request = on_request
        self.connection = None
        self.connections = 0
        self.closed = False
        self.pending = b""
        self.status = None
        self.tail = []

    def accept(self):
        connection, _ = self.listener.accept()
        self.connections += 1
        if self.connection is not None or self.closed:
            # (a second process: the first one's launch didn't complete)
            connection.close()
            return False
        connection.setblocking(False)
        self.connection = connection
        return True

    def read(self):
        """Read what's there: False once the stream has ended."""
        try:
            data = self.connection.recv(65536)
        except BlockingIOError:
            return True
        except OSError:
            data = b""
        if not data:
            self.close()
            return False
        self.lines(data)
        return True

    def lines(self, data, final=False):
        self.pending += data
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
            self.out.write(text + "\n")
            request = REQUEST.fullmatch(text) if self.on_request else None
            if request:
                self.out.flush()
                self.on_request(request.group(1), request.group(2))
        self.out.flush()

    def drain(self, deadline):
        """Read the rest (the app has been stopped)."""
        if self.connection is None:
            return
        self.connection.setblocking(True)
        while not self.closed:
            self.connection.settimeout(max(0.1, deadline - time.monotonic()))
            try:
                data = self.connection.recv(65536)
            except OSError:
                data = b""
            if not data:
                self.close()
            else:
                self.lines(data)

    def close(self):
        if not self.closed:
            self.lines(b"", final=True)
        self.closed = True
        if self.connection is not None:
            self.connection.close()


class AppRequests:
    """What a running app asks for (`sdl3-android-runner: request WHAT`)."""

    def __init__(self, serial, package):
        self.serial = serial
        self.package = package

    def handle(self, what, args=""):
        args = args or ""
        if what == "background":
            result = adb_shell(self.serial, "input keyevent KEYCODE_HOME", check=False)
        elif what == "foreground":
            result = adb_shell(self.serial, f"am start -n {self.package}/{ACTIVITY}", check=False)
        elif what == "input":
            try:
                words = shlex.split(args)
            except ValueError as e:
                note(f"request input: {e} (ignored)")
                return
            if not words:
                note("request input: no arguments (ignored)")
                return
            result = adb_shell(self.serial, "input " + shlex.join(words), check=False)
        else:
            note(f"unknown request {what!r} (ignored)")
            return
        if result.returncode != 0:
            note(f"request {what} {args}: failed ({result.returncode}): {(result.stderr or result.stdout).strip()}")


def app_pid(serial, package):
    result = adb_shell(serial, f"pidof {package}", check=False, timeout=30)
    return result.stdout.strip() or None


def install(serial, apk, package):
    result = adb(serial, "install", "-r", "-t", apk, check=False, timeout=600)
    output = (result.stdout + result.stderr).strip()
    if result.returncode != 0 and "INSTALL_FAILED_UPDATE_INCOMPATIBLE" in output:
        # (signed with another key: an older install of the package)
        adb(serial, "uninstall", package, check=False, timeout=120)
        result = adb(serial, "install", "-r", "-t", apk, check=False, timeout=600)
        output = (result.stdout + result.stderr).strip()
    if result.returncode != 0 or "Success" not in output:
        raise Failure(f"adb install {os.path.basename(apk)} failed ({result.returncode}): {output}")


def launch(serial, program, elf, args):
    with tempfile.TemporaryDirectory(prefix="sdl3-android-apk-") as temp:
        run = Run(serial, program, args)
        listeners = []
        try:
            for _ in range(2):
                listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
                listener.bind(("127.0.0.1", 0))
                listener.listen(4)
                listener.setblocking(False)
                listeners.append(listener)
            ports = [listener.getsockname()[1] for listener in listeners]
            env = app_environment(run, args, ports)
            apk = os.path.join(temp, "app.apk")
            package = write_apk(program, elf, apk, env, temp)
            # (one app at a time: each takes the screen)
            with Lock("app"):
                for port in ports:
                    adb(serial, "reverse", f"tcp:{port}", f"tcp:{port}")
                try:
                    adb(serial, "logcat", "-b", "all", "-c", check=False)
                    install(serial, apk, package)
                    try:
                        return run_app(serial, package, listeners)
                    finally:
                        adb(serial, "uninstall", package, check=False, timeout=120)
                finally:
                    for port in ports:
                        adb(serial, "reverse", "--remove", f"tcp:{port}", check=False)
        finally:
            for listener in listeners:
                listener.close()
            run.clean_up()


def device_log(serial):
    """The device's log since the app's install (the crash buffer too)."""
    result = adb(serial, "logcat", "-d", "-b", "main,system,crash", "-v", "brief", check=False, timeout=120)
    return result.stdout.splitlines()


def run_app(serial, package, listeners):
    timeout = number("SDL3_ANDROID_TIMEOUT", 1800)
    requests = AppRequests(serial, package)
    out = Stream("stdout", listeners[0], sys.stdout, True, requests.handle)
    err = Stream("stderr", listeners[1], sys.stderr, False)
    streams = [out, err]
    selector = selectors.DefaultSelector()
    for stream in streams:
        selector.register(stream.listener, selectors.EVENT_READ, (stream, "accept"))
    start = time.monotonic()
    result = adb_shell(serial, f"am start -W -n {package}/{ACTIVITY}", check=False, timeout=300)
    launched = result.returncode == 0 and "Error" not in result.stdout + result.stderr
    if VERBOSE:
        note(f"am start: {' | '.join(result.stdout.strip().splitlines())} ({time.monotonic() - start:.1f} s)")
    reason = None
    if not launched:
        reason = f"am start failed ({result.returncode}): {(result.stderr or result.stdout).strip()}"
    relaunched = False
    timed_out = False
    last_check = time.monotonic()
    while launched and out.status is None:
        if timeout and time.monotonic() - start > timeout:
            timed_out = True
            reason = f"the app ran for more than {timeout:g} s (SDL3_ANDROID_TIMEOUT)"
            note(f"{reason}: stopping it")
            break
        for key, _ in selector.select(timeout=0.5):
            stream, what = key.data
            if what == "accept":
                if stream.accept():
                    selector.register(stream.connection, selectors.EVENT_READ, (stream, "read"))
                else:
                    relaunched = True
            elif not stream.read():
                selector.unregister(key.fileobj)
        if relaunched:
            reason = (
                "the app's process ended before its launch was complete, and Android launched it again "
                "(the program would run a second time)"
            )
            break
        if all(s.closed for s in streams):
            reason = (
                "the app ended without printing `sdl3-android-runner: exit N` (it crashed, panicked in a "
                "native method, which aborts, or exited some other way)"
            )
            break
        if out.connection is None and time.monotonic() - last_check > 2:
            last_check = time.monotonic()
            if time.monotonic() - start > CONNECT_TIMEOUT and not app_pid(serial, package):
                reason = f"the app didn't connect to the runner in {CONNECT_TIMEOUT} s, and its process is gone"
                break
    # The status is known (or won't be): stop the app (which ends its
    # connections), then copy what's left of its output.
    adb_shell(serial, f"am force-stop {package}", check=False, timeout=60)
    deadline = time.monotonic() + 5
    for stream in streams:
        stream.drain(deadline)
    selector.close()
    log = device_log(serial)
    failures = [line for line in log if VERIFY_FAILURE.search(line) and ("org.libsdl" in line or "org/libsdl" in line or package in line)]
    if VERBOSE:
        for line in log:
            if "Dexopt result" in line and package in line:
                note(f"ART: {line.strip()}")
    if failures:
        note("ART didn't verify the app's classes:")
        for line in failures[-20:]:
            print(f"  | {line}", file=sys.stderr)
        note(f"{package} failed: ART's verification of its DEX file")
        return 1
    if out.status is not None and not relaunched and not timed_out:
        return out.status
    if reason is None:
        reason = "the app stopped"
    note(reason)
    for what, stream in (("output", out), ("error output", err)):
        if stream.tail:
            note(f"the end of its {what}:")
            for text in stream.tail[-20:]:
                print(f"  | {text}", file=sys.stderr)
    note("the end of the device's log:")
    for line in log[-200:]:
        print(f"  | {line}", file=sys.stderr)
    # (once more below the log, where a CI log's last lines show it)
    note(f"{package} failed: {reason}")
    return 1


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
        if "app" in elf.words:
            return launch(serial, program, elf, args)
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
        if sys.argv[1] == "--apk":
            return cmd_apk(sys.argv[2:])
        return cmd_run(sys.argv[1], sys.argv[2:])
    except Failure as e:
        note(str(e))
        return 1


if __name__ == "__main__":
    sys.exit(main())
