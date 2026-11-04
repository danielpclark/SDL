#!/usr/bin/env python3
"""Run a program that tests SDL's console input (the evdev reader of the
KMS/DRM driver and of the dummy driver's `evdev` variant) with its
requests answered: the input devices it reads made with uinput.

Usage:
  linux_console_run.py PROGRAM [ARGUMENTS...]

The program is run with this script's arguments after PROGRAM, its
stdout read by this script (and copied to this script's stdout), its
stderr this script's, its stdin a pipe this script answers on, and its exit
status this script's (128 + N for a signal N). It runs in a session of its
own (setsid), without a controlling terminal: SDL's console keyboard
finds no console (`/dev/tty`), and uses its built-in keymaps. The
variable SDL3_CONSOLE_RUNNER is set (to this script's path) for it, so it
knows it runs here. It is stopped (SIGKILL) after SDL3_CONSOLE_TIMEOUT
seconds (300 by default); the run fails then.

A program asks for what it needs with a line `sdl3-console-runner:
request WHAT [ARGUMENTS]` (copied, then acted on; ARGUMENTS are shell
words):
  uinput add NAME KIND
              a uinput device named NAME (bus USB, 1209:5d3b, version 1) of
              the KIND:
                keyboard     the keys KEY_ESC to KEY_MICMUTE (1 to 248)
                             and KEY_MAX - 1 (0x2fe), and EV_REP
                mouse        BTN_LEFT to BTN_TASK, REL_X, REL_Y,
                             REL_HWHEEL, REL_WHEEL, REL_WHEEL_HI_RES and
                             REL_HWHEEL_HI_RES
                tablet       an absolute pointer (as a virtual machine's):
                             BTN_LEFT, BTN_RIGHT and BTN_MIDDLE, ABS_X and
                             ABS_Y (0 to 32767)
                touchscreen  INPUT_PROP_DIRECT, BTN_TOUCH, ABS_X and ABS_Y
                             (0 to 1000), and 4 multitouch slots
                             (ABS_MT_SLOT 0 to 3), ABS_MT_POSITION_X and
                             ABS_MT_POSITION_Y (0 to 1000),
                             ABS_MT_TRACKING_ID (0 to 65535) and
                             ABS_MT_PRESSURE (0 to 255)
              once udev has its event node, with its classification
              (`ID_INPUT=1` in udev's database), within 10 s: the answer
              is `uinput: added NAME`; or `uinput: unreadable NAME NODE`,
              when the program can't open the node (as udev made it: its
              owner and mode)
  uinput events NAME TYPE CODE VALUE [TYPE CODE VALUE...]
              the device's events (the types and codes as numbers, or as
              <linux/input-event-codes.h> names: EV_KEY, KEY_A, SYN_REPORT,
              ...), written in one write(): the kernel has them in the
              readers' buffers when the answer, `uinput: sent N`, comes
  uinput remove NAME
              the device destroyed, once udev's database hasn't it (within
              10 s): `uinput: removed NAME`
The answer is written to the program's stdin once the request is done:
its lines (`sdl3-console-runner: reply LINE`), then `sdl3-console-runner:
done`. A request this script can't act on fails the run (its reply is
`error: WHY`). Every device left is destroyed once the program ends.

This needs a writable /dev/uinput (on Ubuntu: root's only; a udev rule
`KERNEL=="uinput", MODE="0666"` opens it), and the devices' event nodes
readable by the program (root's and the input group's: the rule
`SUBSYSTEM=="input", KERNEL=="event*", ATTRS{name}=="SDL3 *",
MODE="0666"` opens the ones of these devices only).
"""
import fcntl
import os
import shlex
import signal
import struct
import subprocess
import sys
import threading
import time

PREFIX = "sdl3-console-runner: "

# <asm-generic/ioctl.h> (x86, ARM, RISC-V; not MIPS, PowerPC, SPARC)
IOC_NONE, IOC_WRITE, IOC_READ = 0, 1, 2


def ioc(direction, kind, nr, size):
    return (direction << 30) | (size << 16) | (ord(kind) << 8) | nr


# <linux/uinput.h>
UI_DEV_CREATE = ioc(IOC_NONE, "U", 1, 0)
UI_DEV_DESTROY = ioc(IOC_NONE, "U", 2, 0)
UI_DEV_SETUP = ioc(IOC_WRITE, "U", 3, 92)  # struct uinput_setup
UI_ABS_SETUP = ioc(IOC_WRITE, "U", 4, 28)  # struct uinput_abs_setup
UI_SET_EVBIT = ioc(IOC_WRITE, "U", 100, 4)
UI_SET_KEYBIT = ioc(IOC_WRITE, "U", 101, 4)
UI_SET_RELBIT = ioc(IOC_WRITE, "U", 102, 4)
UI_SET_ABSBIT = ioc(IOC_WRITE, "U", 103, 4)
UI_SET_PROPBIT = ioc(IOC_WRITE, "U", 110, 4)


def UI_GET_SYSNAME(length):
    return ioc(IOC_READ, "U", 44, length)


BUS_USB = 0x03

# <linux/input-event-codes.h>: the names the requests may use
CODES = {
    "EV_SYN": 0x00, "EV_KEY": 0x01, "EV_REL": 0x02, "EV_ABS": 0x03, "EV_MSC": 0x04, "EV_REP": 0x14,
    "SYN_REPORT": 0, "SYN_CONFIG": 1, "SYN_MT_REPORT": 2, "SYN_DROPPED": 3,
    "KEY_ESC": 1, "KEY_1": 2, "KEY_2": 3, "KEY_0": 11, "KEY_MINUS": 12, "KEY_BACKSPACE": 14, "KEY_TAB": 15,
    "KEY_Q": 16, "KEY_W": 17, "KEY_E": 18, "KEY_R": 19, "KEY_T": 20, "KEY_Y": 21, "KEY_U": 22, "KEY_I": 23,
    "KEY_O": 24, "KEY_P": 25, "KEY_ENTER": 28, "KEY_LEFTCTRL": 29, "KEY_A": 30, "KEY_S": 31, "KEY_D": 32,
    "KEY_F": 33, "KEY_G": 34, "KEY_H": 35, "KEY_J": 36, "KEY_K": 37, "KEY_L": 38, "KEY_SEMICOLON": 39,
    "KEY_APOSTROPHE": 40, "KEY_GRAVE": 41, "KEY_LEFTSHIFT": 42, "KEY_BACKSLASH": 43, "KEY_Z": 44, "KEY_X": 45,
    "KEY_C": 46, "KEY_V": 47, "KEY_B": 48, "KEY_N": 49, "KEY_M": 50, "KEY_COMMA": 51, "KEY_DOT": 52,
    "KEY_SLASH": 53, "KEY_RIGHTSHIFT": 54, "KEY_KPASTERISK": 55, "KEY_LEFTALT": 56, "KEY_SPACE": 57,
    "KEY_CAPSLOCK": 58, "KEY_F1": 59, "KEY_NUMLOCK": 69, "KEY_SCROLLLOCK": 70, "KEY_KP7": 71, "KEY_KP8": 72,
    "KEY_KP9": 73, "KEY_KPMINUS": 74, "KEY_KP4": 75, "KEY_KP5": 76, "KEY_KP6": 77, "KEY_KPPLUS": 78,
    "KEY_KP1": 79, "KEY_KP2": 80, "KEY_KP3": 81, "KEY_KP0": 82, "KEY_KPDOT": 83, "KEY_102ND": 86,
    "KEY_KPENTER": 96, "KEY_RIGHTCTRL": 97, "KEY_RIGHTALT": 100, "KEY_HOME": 102, "KEY_UP": 103,
    "KEY_LEFT": 105, "KEY_RIGHT": 106, "KEY_DOWN": 108, "KEY_DELETE": 111, "KEY_MUTE": 113,
    "KEY_LEFTMETA": 125, "KEY_COMPOSE": 127, "KEY_PLAYPAUSE": 164, "KEY_MICMUTE": 248, "KEY_MAX": 0x2ff,
    "BTN_LEFT": 0x110, "BTN_RIGHT": 0x111, "BTN_MIDDLE": 0x112, "BTN_SIDE": 0x113, "BTN_EXTRA": 0x114,
    "BTN_FORWARD": 0x115, "BTN_BACK": 0x116, "BTN_TASK": 0x117, "BTN_TOUCH": 0x14a,
    "REL_X": 0x00, "REL_Y": 0x01, "REL_HWHEEL": 0x06, "REL_WHEEL": 0x08, "REL_WHEEL_HI_RES": 0x0b,
    "REL_HWHEEL_HI_RES": 0x0c,
    "ABS_X": 0x00, "ABS_Y": 0x01, "ABS_MT_SLOT": 0x2f, "ABS_MT_POSITION_X": 0x35, "ABS_MT_POSITION_Y": 0x36,
    "ABS_MT_TRACKING_ID": 0x39, "ABS_MT_PRESSURE": 0x3a,
    "INPUT_PROP_DIRECT": 0x01,
}

# What each kind of device has: (event types, keys, relative axes,
# absolute axes as (code, minimum, maximum), properties)
KINDS = {
    "keyboard": ([0x01, 0x14], list(range(1, 249)) + [0x2fe], [], [], []),
    "mouse": ([0x01, 0x02], list(range(0x110, 0x118)), [0x00, 0x01, 0x06, 0x08, 0x0b, 0x0c], [], []),
    "tablet": ([0x01, 0x03], [0x110, 0x111, 0x112], [], [(0x00, 0, 32767), (0x01, 0, 32767)], []),
    "touchscreen": ([0x01, 0x03], [0x14a], [],
                    [(0x00, 0, 1000), (0x01, 0, 1000), (0x2f, 0, 3), (0x35, 0, 1000), (0x36, 0, 1000),
                     (0x39, 0, 65535), (0x3a, 0, 255)], [0x01]),
}

# struct input_event (two longs of time, then __u16 type, __u16 code,
# __s32 value)
EVENT = struct.Struct("@llHHi")


class RequestError(Exception):
    pass


def number(word):
    if word in CODES:
        return CODES[word]
    try:
        return int(word, 0)
    except ValueError:
        raise RequestError(f"unknown code {word!r}")


class Device:
    def __init__(self, name, kind):
        if kind not in KINDS:
            raise RequestError(f"unknown kind of device {kind!r}")
        types, keys, rels, abss, props = KINDS[kind]
        self.name = name
        self.fd = os.open("/dev/uinput", os.O_WRONLY | os.O_NONBLOCK | os.O_CLOEXEC)
        try:
            for t in types:
                fcntl.ioctl(self.fd, UI_SET_EVBIT, t)
            for k in keys:
                fcntl.ioctl(self.fd, UI_SET_KEYBIT, k)
            for r in rels:
                fcntl.ioctl(self.fd, UI_SET_RELBIT, r)
            for p in props:
                fcntl.ioctl(self.fd, UI_SET_PROPBIT, p)
            for code, lo, hi in abss:
                fcntl.ioctl(self.fd, UI_SET_ABSBIT, code)
                # struct uinput_abs_setup: __u16 code, then struct input_absinfo
                fcntl.ioctl(self.fd, UI_ABS_SETUP, struct.pack("@H2x6i", code, 0, lo, hi, 0, 0, 0))
            # struct uinput_setup: struct input_id, char name[80], __u32 ff_effects_max
            fcntl.ioctl(self.fd, UI_DEV_SETUP, struct.pack("@HHHH80sI", BUS_USB, 0x1209, 0x5d3b, 1,
                                                           name.encode()[:79], 0))
            fcntl.ioctl(self.fd, UI_DEV_CREATE)
            buf = bytearray(64)
            fcntl.ioctl(self.fd, UI_GET_SYSNAME(len(buf)), buf)
            self.sysname = buf.split(b"\0")[0].decode()
        except OSError:
            os.close(self.fd)
            raise
        self.node = None
        self.db = None

    def wait_added(self, timeout=10.0):
        """The event node, once udev has classified it."""
        end = time.monotonic() + timeout
        sys_dir = f"/sys/devices/virtual/input/{self.sysname}"
        while time.monotonic() < end:
            events = [n for n in os.listdir(sys_dir) if n.startswith("event")] if os.path.isdir(sys_dir) else []
            if events:
                minor = int(open(f"{sys_dir}/{events[0]}/dev").read().split(":")[1])
                self.node = f"/dev/input/{events[0]}"
                self.db = f"/run/udev/data/c13:{minor}"
                try:
                    if "E:ID_INPUT=1" in open(self.db).read().split("\n") and os.path.exists(self.node):
                        return
                except OSError:
                    pass
            time.sleep(0.01)
        raise RequestError(f"udev didn't add {self.name}'s event node in {timeout} s")

    def send(self, events):
        data = b"".join(EVENT.pack(0, 0, t, c, v) for t, c, v in events)
        written = os.write(self.fd, data)
        if written != len(data):
            raise RequestError(f"wrote {written} of {len(data)} bytes")

    def destroy(self, timeout=10.0):
        fcntl.ioctl(self.fd, UI_DEV_DESTROY)
        os.close(self.fd)
        end = time.monotonic() + timeout
        while self.db and os.path.exists(self.db) and time.monotonic() < end:
            time.sleep(0.01)
        if self.db and os.path.exists(self.db):
            raise RequestError(f"udev didn't remove {self.name} in {timeout} s")


class Runner:
    def __init__(self):
        self.devices = {}

    def request(self, words):
        if len(words) < 3 or words[0] != "uinput":
            raise RequestError(f"unknown request {shlex.join(words)!r}")
        what, name = words[1], words[2]
        if what == "add":
            if len(words) != 4:
                raise RequestError("uinput add NAME KIND")
            if name in self.devices:
                raise RequestError(f"{name} exists")
            device = Device(name, words[3])
            self.devices[name] = device
            device.wait_added()
            if not os.access(device.node, os.R_OK):
                st = os.stat(device.node)
                return [f"uinput: unreadable {name} {device.node} (uid {st.st_uid} gid {st.st_gid} "
                        f"mode {st.st_mode & 0o7777:o})"]
            return [f"uinput: added {name}"]
        device = self.devices.get(name)
        if device is None:
            raise RequestError(f"no device {name}")
        if what == "events":
            values = [number(w) for w in words[3:]]
            if not values or len(values) % 3:
                raise RequestError("uinput events NAME TYPE CODE VALUE [...]")
            device.send([tuple(values[i:i + 3]) for i in range(0, len(values), 3)])
            return [f"uinput: sent {len(values) // 3}"]
        if what == "remove":
            del self.devices[name]
            device.destroy()
            return [f"uinput: removed {name}"]
        raise RequestError(f"unknown request {shlex.join(words)!r}")

    def clean_up(self):
        for device in self.devices.values():
            try:
                device.destroy()
            except (OSError, RequestError) as e:
                print(f"linux_console_run.py: {device.name}: {e}", file=sys.stderr)
        self.devices.clear()


def main():
    if len(sys.argv) < 2 or sys.argv[1] in ("-h", "--help"):
        print(__doc__)
        return 0 if len(sys.argv) > 1 else 2
    timeout = float(os.environ.get("SDL3_CONSOLE_TIMEOUT", "300"))
    env = dict(os.environ, SDL3_CONSOLE_RUNNER=os.path.abspath(__file__))
    proc = subprocess.Popen(sys.argv[1:], stdin=subprocess.PIPE, stdout=subprocess.PIPE, env=env,
                            start_new_session=True)
    timed_out = threading.Event()

    def stop():
        timed_out.set()
        proc.kill()

    timer = threading.Timer(timeout, stop)
    timer.start()
    runner = Runner()
    failed = False
    try:
        for raw in proc.stdout:
            line = raw.decode("utf-8", "replace").rstrip("\n")
            print(line, flush=True)
            if not line.startswith(PREFIX + "request "):
                continue
            try:
                replies = runner.request(shlex.split(line[len(PREFIX + "request "):]))
            except (RequestError, OSError, ValueError) as e:
                replies = [f"error: {e}"]
                failed = True
                print(f"linux_console_run.py: {line}: {e}", file=sys.stderr, flush=True)
            answer = "".join(f"{PREFIX}reply {r}\n" for r in replies) + f"{PREFIX}done\n"
            try:
                proc.stdin.write(answer.encode())
                proc.stdin.flush()
            except BrokenPipeError:
                pass
        status = proc.wait()
    finally:
        timer.cancel()
        if proc.poll() is None:
            proc.kill()
            proc.wait()
        runner.clean_up()
    if timed_out.is_set():
        print(f"linux_console_run.py: {sys.argv[1]} didn't end in {timeout:g} s", file=sys.stderr)
        return 1
    if status < 0:
        status = 128 - status
    if failed and status == 0:
        return 1
    return status


if __name__ == "__main__":
    sys.exit(main())
