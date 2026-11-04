#!/usr/bin/env python3
"""Run a program that tests SDL's console input (the evdev reader of the
KMS/DRM driver and of the dummy driver's `evdev` variant) with its
requests answered: the input devices it reads made with uinput.

Usage:
  linux_console_run.py [--vt N] PROGRAM [ARGUMENTS...]

The program is run with this script's arguments after PROGRAM, its
stdout read by this script (and copied to this script's stdout), its
stderr this script's, its stdin a pipe this script answers on, and its exit
status this script's (128 + N for a signal N). It runs in a session of its
own (setsid), without a controlling terminal: SDL's console keyboard
finds no console (`/dev/tty`), and uses its built-in keymaps. The
variable SDL3_CONSOLE_RUNNER is set (to this script's path) for it, so it
knows it runs here. It is stopped (SIGKILL) after SDL3_CONSOLE_TIMEOUT
seconds (300 by default); the run fails then.

With --vt N (as root), the program runs on the virtual console ttyN
instead: ttyN is made the active console first, and the program's
controlling terminal and stdin (`setsid -c`, as a console program
started there), so SDL's console keyboard is ttyN's (its keymap, its
keyboard mode, its VT switches). The answers then come on another pipe,
whose descriptor is in the variable SDL3_CONSOLE_REPLY_FD, and
SDL3_CONSOLE_VT is N. The keyboard mode (KDGKBMODE) and the VT mode
(VT_GETMODE) ttyN had are checked once the program has ended, however it
ended (by a fatal signal too): a mode it left changed is restored and
fails the run. Then ttyN's input is flushed (keys the console wasn't muted
for are typed into it), and the console active before is made active
again.

A program asks for what it needs with a line `sdl3-console-runner:
request WHAT [ARGUMENTS]` (copied, then acted on; ARGUMENTS are shell
words):
  uinput add NAME KIND
              a uinput device named NAME (bus USB, 1209:5d3b, version 1) of
              the KIND:
                keyboard     the keys KEY_ESC to KEY_MICMUTE (1 to 248)
                             and KEY_MAX - 1 (0x2fe); no EV_REP (the
                             kernel's own repeats would be a matter of
                             timing)
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
With --vt N only:
  vt modes    ttyN's keyboard mode and VT mode: `vt: keyboard MODE mode
              VTMODE` (K_XLATE, K_UNICODE, K_OFF, ...; VT_AUTO or
              VT_PROCESS: the signals VT_PROCESS asks for aren't told)
  vt active   the active console: `vt: active M`
  vt activate M
              VT_ACTIVATE of ttyM, not waited for (a switch away from a
              VT_PROCESS console waits for its program's VT_RELDISP): `vt:
              activating M`
  vt switch M VT_ACTIVATE of ttyM, waited for (VT_GETSTATE, within 10 s):
              `vt: active M`, or the console still active then
  vt restore  ttyN's keyboard mode and VT mode set back to the ones it had
              before the program: `vt: restored keyboard OLD to MODE`,
              `vt: restored mode OLD to VTMODE`, or `vt: nothing to
              restore`
  vt flush    ttyN's input thrown away (TCIFLUSH): `vt: flushed`
  vt setkey TABLE KEY VALUE
              an entry of the console keymap (KDSKBENT; numbers: VALUE as
              KDGKBENT gives it, a Unicode keysym U+X as X ^ 0xf000, set
              while ttyN is in K_UNICODE), put back once the program has
              ended (the keymap is all the consoles'): `vt: key TABLE KEY
              set to VALUE`
As root, for the KMS/DRM driver's programs (on the card
/dev/dri/cardN, N the variable SDL_KMSDRM_DEVICE_INDEX, by default the
first card whose DRM driver is vkms: a card's index depends on the
machine's other cards, vkms's card0 on some runners, card1 after the
firmware framebuffer's simpledrm card0 on others; a descriptor of the card
opened for each request, without DRM master, as a second client; debugfs
mounted):
  drm state   what the card scans out: its first CRTC's mode, and the
              planes that have a framebuffer (their type, the
              framebuffer's size and format, their source and CRTC
              rectangles): `drm: crtc MODE (NAME) on|off`, `drm: plane
              TYPE fb WxH FOURCC src X,Y WxH crtc X,Y WxH`, or `drm: no
              planes on`
  drm crc     the CRC of the frame the first CRTC scans out (its debugfs
              crc/data: the composed frame of every vblank, with the
              cursor plane; the value once three in a row are the same,
              within 5 s): `drm: crc 0xC`, `drm: crc unstable`, or
              `drm: crc none` (no vblank in 1 s: the CRTC is off)
  drm pixels X Y [X Y...]
              pixels of the framebuffer on the first CRTC's primary plane
              (its GEM buffer mapped with MAP_DUMB: dumb buffers, as
              Mesa's kms_swrast and vkms have): `drm: pixel X,Y 0xV`
  drm master  whether another client has the card's DRM master: `drm: master
              taken`, or `drm: master free`
  drm versions
              the kernel's version, Mesa's (libgbm1's package) and
              libdrm's (libdrm2's): `versions: kernel K, Mesa M, libdrm L`
  drm master hold
              every card opened (/dev/dri/card*) and made this script's
              DRM master, kept until `drm master release` or the end:
              `drm: master held of every card`, or `drm: master held of
              cardN ... but not of cardM ...` (which cards a machine has
              varies, so they're named only when one's master can't be
              had)
  drm master release
              those cards closed: `drm: master released`
  drm card K [C]
              the card /dev/dri/cardK (and its CRTC C, 0 by default: the
              Cth of its resources' list) for the requests that follow
              (state, crc, pixels, master), in place of
              SDL_KMSDRM_DEVICE_INDEX's first CRTC: `drm: card K crtc C`
  drm card    SDL_KMSDRM_DEVICE_INDEX's card and first CRTC again: `drm:
              card default`
  drm info    what the hardware checks look at: the card's capabilities
              (`drm: cap NAME V`: DUMB_BUFFER, ASYNC_PAGE_FLIP,
              CURSOR_WIDTH, CURSOR_HEIGHT, ADDFB2_MODIFIERS,
              ATOMIC_ASYNC_PAGE_FLIP), its connectors (`drm: connector
              NAME STATUS, N modes, preferred WxH@HZ, vrr_capable V, panel
              orientation V`; NAME as the driver names displays, TYPE-ID),
              the CRTC's VRR_ENABLED (`drm: crtc VRR_ENABLED V`), and the
              planes with a framebuffer (`drm: plane TYPE fb WxH FOURCC
              modifier 0xM`, or `none` without one)
  vkms add NAME N
              a vkms device made through configfs (mounted on
              /sys/kernel/config if it isn't), NAME, of N pipelines, each a
              primary and a cursor plane, a CRTC, an encoder and a
              connected connector; enabled, once its card's node and its
              connectors are there (within 10 s): `vkms: added NAME cardK
              CONNECTOR...` (the connectors' names: Virtual-M, numbered
              across the machine's cards)
  vkms remove NAME
              the device disabled and its configfs directories removed,
              once its card is gone (within 10 s): `vkms: removed NAME`
With SDL_KMSDRM_DEVICE_INDEX set (as root), the card is opened and closed
once before the program: a client's last close makes the kernel's fbdev
emulation set its mode again, as it does after every program, so every
run starts from the same scanout.
The answer is written to the program's stdin (with --vt, the reply pipe)
once the request is done: its lines (`sdl3-console-runner: reply LINE`),
then `sdl3-console-runner: done`. The program's children may ask too
(their stdout and the reply pipe inherited), one process at a time. A request this script can't act on fails the run (its reply is
`error: WHY`). Every device left (uinput's, vkms's) is destroyed once the
program ends.

This needs a writable /dev/uinput (on Ubuntu: root's only; a udev rule
`KERNEL=="uinput", MODE="0666"` opens it), and the devices' event nodes
readable by the program (root's and the input group's: the rule
`SUBSYSTEM=="input", KERNEL=="event*", ATTRS{name}=="SDL3 *",
MODE="0666"` opens the ones of these devices only). --vt needs root, and
util-linux's setsid.
"""
import ctypes
import errno
import fcntl
import mmap
import os
import select
import shlex
import signal
import struct
import subprocess
import sys
import termios
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

# <linux/kd.h>, <linux/vt.h>
KDGKBMODE, KDSKBMODE = 0x4B44, 0x4B45
KDGKBENT, KDSKBENT = 0x4B46, 0x4B47
# struct kbentry: unsigned char kb_table, kb_index; unsigned short kb_value
KBENTRY = struct.Struct("@BBH")
KB_MODES = {0: "K_RAW", 1: "K_XLATE", 2: "K_MEDIUMRAW", 3: "K_UNICODE", 4: "K_OFF"}
VT_GETMODE, VT_SETMODE, VT_GETSTATE, VT_ACTIVATE = 0x5601, 0x5602, 0x5603, 0x5606
VT_MODES = {0: "VT_AUTO", 1: "VT_PROCESS", 2: "VT_ACKACQ"}
# struct vt_mode: char mode, waitv; short relsig, acqsig, frsig
VT_MODE = struct.Struct("@bbhhh")
# struct vt_stat: unsigned short v_active, v_signal, v_state
VT_STAT = struct.Struct("@HHH")

# <drm/drm.h>, <drm/drm_mode.h> (64-bit layouts)
DRM_IOCTL_VERSION = ioc(IOC_READ | IOC_WRITE, "d", 0x00, 64)  # struct drm_version
DRM_IOCTL_SET_MASTER = ioc(IOC_NONE, "d", 0x1E, 0)
DRM_IOCTL_DROP_MASTER = ioc(IOC_NONE, "d", 0x1F, 0)
DRM_IOCTL_GEM_CLOSE = ioc(IOC_WRITE, "d", 0x09, 8)  # struct drm_gem_close
DRM_IOCTL_SET_CLIENT_CAP = ioc(IOC_WRITE, "d", 0x0D, 16)  # struct drm_set_client_cap
DRM_IOCTL_MODE_GETRESOURCES = ioc(IOC_READ | IOC_WRITE, "d", 0xA0, 64)  # struct drm_mode_card_res
DRM_IOCTL_MODE_GETCRTC = ioc(IOC_READ | IOC_WRITE, "d", 0xA1, 104)  # struct drm_mode_crtc
DRM_IOCTL_MODE_GETPROPERTY = ioc(IOC_READ | IOC_WRITE, "d", 0xAA, 64)  # struct drm_mode_get_property
DRM_IOCTL_MODE_MAP_DUMB = ioc(IOC_READ | IOC_WRITE, "d", 0xB3, 16)  # struct drm_mode_map_dumb
DRM_IOCTL_MODE_GETPLANERESOURCES = ioc(IOC_READ | IOC_WRITE, "d", 0xB5, 16)  # struct drm_mode_get_plane_res
DRM_IOCTL_MODE_GETPLANE = ioc(IOC_READ | IOC_WRITE, "d", 0xB6, 32)  # struct drm_mode_get_plane
DRM_IOCTL_MODE_OBJ_GETPROPERTIES = ioc(IOC_READ | IOC_WRITE, "d", 0xB9, 32)  # struct drm_mode_obj_get_properties
DRM_IOCTL_MODE_GETFB2 = ioc(IOC_READ | IOC_WRITE, "d", 0xCE, 104)  # struct drm_mode_fb_cmd2
DRM_CLIENT_CAP_UNIVERSAL_PLANES, DRM_CLIENT_CAP_ATOMIC = 2, 3
DRM_MODE_OBJECT_PLANE = 0xEEEEEEEE
DRM_MODE_OBJECT_CRTC = 0xCCCCCCCC
DRM_MODE_OBJECT_CONNECTOR = 0xC0C0C0C0
DRM_IOCTL_GET_CAP = ioc(IOC_READ | IOC_WRITE, "d", 0x0C, 16)  # struct drm_get_cap
DRM_IOCTL_MODE_GETCONNECTOR = ioc(IOC_READ | IOC_WRITE, "d", 0xA7, 80)  # struct drm_mode_get_connector
# <drm/drm.h>'s DRM_CAP_*
DRM_CAPS = (("DUMB_BUFFER", 0x1), ("ASYNC_PAGE_FLIP", 0x7), ("CURSOR_WIDTH", 0x8), ("CURSOR_HEIGHT", 0x9),
            ("ADDFB2_MODIFIERS", 0x10), ("ATOMIC_ASYNC_PAGE_FLIP", 0x15))
# <drm/drm_mode.h>'s DRM_MODE_CONNECTOR_*, as libdrm's drmModeGetConnectorTypeName() names them
DRM_CONNECTOR_TYPES = ["Unknown", "VGA", "DVI-I", "DVI-D", "DVI-A", "Composite", "SVIDEO", "LVDS",
                       "Component", "DIN", "DP", "HDMI-A", "HDMI-B", "TV", "eDP", "Virtual", "DSI", "DPI",
                       "Writeback", "SPI", "USB"]
DRM_MODE_FB_MODIFIERS = 2
DRM_MODE_TYPE_PREFERRED = 1 << 3
DRM_PLANE_TYPES = {0: "overlay", 1: "primary", 2: "cursor"}
# struct drm_version: int version_major, version_minor,
# version_patchlevel; size_t name_len; char *name; size_t date_len; char
# *date; size_t desc_len; char *desc
VERSION = struct.Struct("@iiiQQQQQQ")
# struct drm_mode_modeinfo
MODEINFO = "IHHHHHHHHHHIII32s"
# struct drm_mode_crtc: __u64 set_connectors_ptr; __u32 count_connectors,
# crtc_id, fb_id, x, y, gamma_size, mode_valid; struct drm_mode_modeinfo
CRTC = struct.Struct("@QIIIIIII" + MODEINFO)
# struct drm_mode_card_res
CARD_RES = struct.Struct("@QQQQIIIIIIII")
# struct drm_mode_get_plane_res
PLANE_RES = struct.Struct("@QI4x")
# struct drm_mode_get_plane
GET_PLANE = struct.Struct("@IIIIIIQ")
# struct drm_mode_obj_get_properties
OBJ_PROPS = struct.Struct("@QQIII4x")
# struct drm_mode_get_property
GET_PROPERTY = struct.Struct("@QQII32sII")
# struct drm_mode_fb_cmd2
FB_CMD2 = struct.Struct("@IIIII4I4I4I4x4Q")
# struct drm_mode_map_dumb
MAP_DUMB = struct.Struct("@IIQ")
# struct drm_get_cap
GET_CAP = struct.Struct("@QQ")
# struct drm_mode_get_connector: __u64 encoders_ptr, modes_ptr, props_ptr,
# prop_values_ptr; __u32 count_modes, count_props, count_encoders,
# encoder_id, connector_id, connector_type, connector_type_id, connection,
# mm_width, mm_height, subpixel, pad
GET_CONNECTOR = struct.Struct("@QQQQIIIIIIIIIIII")

# <linux/input-event-codes.h>: the names the requests may use
CODES = {
    "EV_SYN": 0x00, "EV_KEY": 0x01, "EV_REL": 0x02, "EV_ABS": 0x03, "EV_MSC": 0x04, "EV_REP": 0x14,
    "SYN_REPORT": 0, "SYN_CONFIG": 1, "SYN_MT_REPORT": 2, "SYN_DROPPED": 3,
    "KEY_ESC": 1, "KEY_1": 2, "KEY_2": 3, "KEY_3": 4, "KEY_4": 5, "KEY_5": 6, "KEY_6": 7,
    "KEY_7": 8, "KEY_8": 9, "KEY_9": 10, "KEY_0": 11, "KEY_MINUS": 12, "KEY_BACKSPACE": 14, "KEY_TAB": 15,
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
    # (no EV_REP: the kernel would repeat a key held down on its own)
    "keyboard": ([0x01], list(range(1, 249)) + [0x2fe], [], [], []),
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


class Console:
    """The virtual console ttyN the program runs on (--vt N)."""

    def __init__(self, number):
        self.number = number
        self.path = f"/dev/tty{number}"
        self.fd = os.open(self.path, os.O_RDWR | os.O_NOCTTY | os.O_CLOEXEC)
        self.kb_mode = self.keyboard_mode()
        self.vt_mode = self.mode()
        self.active_before = self.active()
        # The keymap's entries set, as they were: (table, key) -> value
        self.keys_before = {}

    def keyboard_mode(self):
        buf = bytearray(4)
        fcntl.ioctl(self.fd, KDGKBMODE, buf)
        return struct.unpack("@i", buf)[0]

    def mode(self):
        buf = bytearray(VT_MODE.size)
        fcntl.ioctl(self.fd, VT_GETMODE, buf)
        return VT_MODE.unpack(buf)

    def active(self):
        buf = bytearray(VT_STAT.size)
        fcntl.ioctl(self.fd, VT_GETSTATE, buf)
        return VT_STAT.unpack(buf)[0]

    def switch(self, number, timeout=10.0):
        """VT_ACTIVATE, then the switch waited for (VT_WAITACTIVE can't time
        out): the console active then."""
        fcntl.ioctl(self.fd, VT_ACTIVATE, number)
        end = time.monotonic() + timeout
        while self.active() != number and time.monotonic() < end:
            time.sleep(0.01)
        return self.active()

    def modes(self):
        kb = self.keyboard_mode()
        return f"vt: keyboard {KB_MODES.get(kb, kb)} mode {VT_MODES.get(self.mode()[0], self.mode()[0])}"

    def restore(self):
        """The modes set back to the ones before the program: what was
        restored, (what, old, new) for each."""
        restored = []
        kb = self.keyboard_mode()
        if kb != self.kb_mode:
            fcntl.ioctl(self.fd, KDSKBMODE, self.kb_mode)
            restored.append(("keyboard", KB_MODES.get(kb, kb), KB_MODES.get(self.kb_mode, self.kb_mode)))
        mode = self.mode()
        if mode[0] != self.vt_mode[0]:
            fcntl.ioctl(self.fd, VT_SETMODE, VT_MODE.pack(*self.vt_mode))
            restored.append(("mode", VT_MODES.get(mode[0], mode[0]),
                             VT_MODES.get(self.vt_mode[0], self.vt_mode[0])))
        return restored

    def request(self, words):
        what = words[1] if len(words) > 1 else ""
        if what == "modes" and len(words) == 2:
            return [self.modes()]
        if what == "active" and len(words) == 2:
            return [f"vt: active {self.active()}"]
        if what == "activate" and len(words) == 3:
            fcntl.ioctl(self.fd, VT_ACTIVATE, int(words[2]))
            return [f"vt: activating {int(words[2])}"]
        if what == "switch" and len(words) == 3:
            return [f"vt: active {self.switch(int(words[2]))}"]
        if what == "restore" and len(words) == 2:
            return [f"vt: restored {what} {old} to {new}" for what, old, new in self.restore()] or \
                ["vt: nothing to restore"]
        if what == "flush" and len(words) == 2:
            termios.tcflush(self.fd, termios.TCIFLUSH)
            return ["vt: flushed"]
        if what == "setkey" and len(words) == 5:
            table, key, value = (int(w, 0) for w in words[2:])
            if (table, key) not in self.keys_before:
                buf = bytearray(KBENTRY.pack(table, key, 0))
                fcntl.ioctl(self.fd, KDGKBENT, buf)
                self.keys_before[(table, key)] = KBENTRY.unpack(buf)[2]
            fcntl.ioctl(self.fd, KDSKBENT, KBENTRY.pack(table, key, value))
            return [f"vt: key {table} {key} set to {value:#06x}"]
        raise RequestError(f"unknown request {shlex.join(words)!r}")

    def finish(self):
        """After the program: the modes checked (and restored), the input
        flushed, the console active before made active again. Whether the
        modes were the ones before."""
        # (opened again: the program's end, as its session's leader, hung
        # the console up, which this descriptor is then)
        os.close(self.fd)
        self.fd = os.open(self.path, os.O_RDWR | os.O_NOCTTY | os.O_CLOEXEC)
        restored = self.restore()
        for what, old, new in restored:
            what = "VT mode" if what == "mode" else "keyboard mode"
            print(f"linux_console_run.py: {self.path}: the program left the {what} {old}, not {new} as it "
                  "was: restored", file=sys.stderr)
        # (K_UNICODE again, for the Unicode keysyms)
        for (table, key), value in self.keys_before.items():
            fcntl.ioctl(self.fd, KDSKBENT, KBENTRY.pack(table, key, value))
        termios.tcflush(self.fd, termios.TCIFLUSH)
        if self.active() != self.active_before:
            self.switch(self.active_before)
        os.close(self.fd)
        return not restored


def fourcc(value):
    return "".join(chr((value >> s) & 0xFF) for s in (0, 8, 16, 24))


def signed(value):
    return value - (1 << 64) if value >= 1 << 63 else value


def drm_driver(index):
    """The name of the DRM driver of /dev/dri/cardINDEX (DRM_IOCTL_VERSION's:
    `vkms` for every vkms device, whatever its own name), or None."""
    try:
        fd = os.open(f"/dev/dri/card{index}", os.O_RDONLY | os.O_CLOEXEC)
    except OSError:
        return None
    try:
        name = ctypes.create_string_buffer(64)
        buf = bytearray(VERSION.pack(0, 0, 0, len(name), ctypes.addressof(name), 0, 0, 0, 0))
        fcntl.ioctl(fd, DRM_IOCTL_VERSION, buf)
        return name.raw[:min(VERSION.unpack(buf)[3], len(name))].decode(errors="replace")
    except OSError:
        return None
    finally:
        os.close(fd)


def vkms_card():
    """The index of the first card (/dev/dri/cardN, by N) whose DRM driver is
    vkms: SDL_KMSDRM_DEVICE_INDEX's default."""
    cards = sorted(int(name[4:]) for name in os.listdir("/dev/dri")
                   if name.startswith("card") and name[4:].isdigit())
    for index in cards:
        if drm_driver(index) == "vkms":
            return index
    raise OSError(errno.ENODEV, "no vkms card (modprobe vkms)")


class Card:
    """A descriptor of the DRM card the KMS/DRM driver uses, opened for a
    request (as a second client: without DRM master, which the first open
    of a card without a master gets, dropped at once)."""

    def __init__(self, index=None, crtc_index=0):
        if index is None:
            index = os.environ.get("SDL_KMSDRM_DEVICE_INDEX", "")
            index = int(index) if index else vkms_card()
        self.index = index
        self.crtc_index = crtc_index
        self.path = f"/dev/dri/card{self.index}"
        self.fd = os.open(self.path, os.O_RDWR | os.O_CLOEXEC)
        try:
            fcntl.ioctl(self.fd, DRM_IOCTL_DROP_MASTER)
        except OSError:
            pass
        for cap in (DRM_CLIENT_CAP_UNIVERSAL_PLANES, DRM_CLIENT_CAP_ATOMIC):
            fcntl.ioctl(self.fd, DRM_IOCTL_SET_CLIENT_CAP, struct.pack("@QQ", cap, 1))
        self.minor = os.minor(os.fstat(self.fd).st_rdev)

    def close(self):
        os.close(self.fd)

    def ioctl(self, request, layout, *values):
        buf = bytearray(layout.pack(*values))
        fcntl.ioctl(self.fd, request, buf)
        return layout.unpack(buf)

    @staticmethod
    def array(kind, count):
        a = (kind * max(count, 1))()
        return a, ctypes.addressof(a)

    def crtcs(self):
        r = self.ioctl(DRM_IOCTL_MODE_GETRESOURCES, CARD_RES, *([0] * 12))
        fbs, fbp = self.array(ctypes.c_uint32, r[4])
        crtcs, crtcp = self.array(ctypes.c_uint32, r[5])
        conns, connp = self.array(ctypes.c_uint32, r[6])
        encs, encp = self.array(ctypes.c_uint32, r[7])
        self.ioctl(DRM_IOCTL_MODE_GETRESOURCES, CARD_RES, fbp, crtcp, connp, encp, r[4], r[5], r[6], r[7],
                   0, 0, 0, 0)
        return list(crtcs)[:r[5]]

    def crtc(self, crtc_id):
        return self.ioctl(DRM_IOCTL_MODE_GETCRTC, CRTC, *([0, 0, crtc_id] + [0] * 19 + [b""]))

    def props(self, obj_id, obj_type):
        r = self.ioctl(DRM_IOCTL_MODE_OBJ_GETPROPERTIES, OBJ_PROPS, 0, 0, 0, obj_id, obj_type)
        ids, idp = self.array(ctypes.c_uint32, r[2])
        values, valp = self.array(ctypes.c_uint64, r[2])
        r = self.ioctl(DRM_IOCTL_MODE_OBJ_GETPROPERTIES, OBJ_PROPS, idp, valp, r[2], obj_id, obj_type)
        props = {}
        for i in range(r[2]):
            p = self.ioctl(DRM_IOCTL_MODE_GETPROPERTY, GET_PROPERTY, 0, 0, ids[i], 0, b"", 0, 0)
            props[p[4].split(b"\0")[0].decode()] = values[i]
        return props

    def planes(self):
        r = self.ioctl(DRM_IOCTL_MODE_GETPLANERESOURCES, PLANE_RES, 0, 0)
        ids, idp = self.array(ctypes.c_uint32, r[1])
        r = self.ioctl(DRM_IOCTL_MODE_GETPLANERESOURCES, PLANE_RES, idp, r[1])
        planes = []
        for plane_id in list(ids)[:r[1]]:
            p = self.ioctl(DRM_IOCTL_MODE_GETPLANE, GET_PLANE, plane_id, 0, 0, 0, 0, 0, 0)
            planes.append((plane_id, p[1], p[2], self.props(plane_id, DRM_MODE_OBJECT_PLANE)))
        return planes

    def fb(self, fb_id):
        """GETFB2 of a framebuffer: its GEM handles (this descriptor's) are
        to be closed with close_handles()."""
        return self.ioctl(DRM_IOCTL_MODE_GETFB2, FB_CMD2, *([fb_id] + [0] * 20))

    def close_handles(self, fb):
        for h in set(fb[5:9]):
            if h:
                fcntl.ioctl(self.fd, DRM_IOCTL_GEM_CLOSE, struct.pack("@II", h, 0))

    def state(self):
        lines = []
        crtcs = self.crtcs()
        first = crtcs[self.crtc_index]
        crtc = self.crtc(first)
        mode = crtc[8:]
        name = mode[14].split(b"\0")[0].decode()
        lines.append(f"drm: crtc {mode[1]}x{mode[6]} ({name}) {'on' if crtc[7] else 'off'}")
        on = [p for p in self.planes() if p[2]]
        for plane_id, crtc_id, fb_id, props in sorted(on, key=lambda p: (p[3].get("type", 9), p[0])):
            fb = self.fb(fb_id)
            self.close_handles(fb)
            ptype = DRM_PLANE_TYPES.get(props.get("type"), props.get("type"))
            lines.append(f"drm: plane {ptype} fb {fb[1]}x{fb[2]} {fourcc(fb[3])} "
                         f"src {props.get('SRC_X', 0) >> 16},{props.get('SRC_Y', 0) >> 16} "
                         f"{props.get('SRC_W', 0) >> 16}x{props.get('SRC_H', 0) >> 16} "
                         f"crtc {signed(props.get('CRTC_X', 0))},{signed(props.get('CRTC_Y', 0))} "
                         f"{props.get('CRTC_W', 0)}x{props.get('CRTC_H', 0)}"
                         + ("" if crtc_id == first else f" (crtc {crtcs.index(crtc_id) if crtc_id in crtcs else crtc_id})"))
        if not on:
            lines.append("drm: no planes on")
        return lines

    def pixels(self, points):
        crtcs = self.crtcs()
        primary = [p for p in self.planes() if p[3].get("type") == 1 and p[2] and p[1] == crtcs[self.crtc_index]]
        if not primary:
            return ["drm: no primary plane on"]
        fb = self.fb(primary[0][2])
        width, height, handle, pitch, offset = fb[1], fb[2], fb[5], fb[9], fb[13]
        lines = []
        try:
            m = self.ioctl(DRM_IOCTL_MODE_MAP_DUMB, MAP_DUMB, handle, 0, 0)
            size = pitch * height + offset
            with mmap.mmap(self.fd, size, mmap.MAP_SHARED, mmap.PROT_READ, offset=m[2]) as data:
                for x, y in points:
                    if not (0 <= x < width and 0 <= y < height):
                        lines.append(f"drm: pixel {x},{y} outside")
                        continue
                    at = offset + y * pitch + x * 4
                    lines.append(f"drm: pixel {x},{y} {struct.unpack_from('<I', data, at)[0]:#010x}")
        finally:
            self.close_handles(fb)
        return lines

    def info(self):
        lines = []
        for name, cap in DRM_CAPS:
            try:
                value = self.ioctl(DRM_IOCTL_GET_CAP, GET_CAP, cap, 0)[1]
                lines.append(f"drm: cap {name} {value}")
            except OSError as e:
                lines.append(f"drm: cap {name} error {e.errno}")
        r = self.ioctl(DRM_IOCTL_MODE_GETRESOURCES, CARD_RES, *([0] * 12))
        fbs, fbp = self.array(ctypes.c_uint32, r[4])
        crtcs, crtcp = self.array(ctypes.c_uint32, r[5])
        conns, connp = self.array(ctypes.c_uint32, r[6])
        encs, encp = self.array(ctypes.c_uint32, r[7])
        self.ioctl(DRM_IOCTL_MODE_GETRESOURCES, CARD_RES, fbp, crtcp, connp, encp, r[4], r[5], r[6], r[7],
                   0, 0, 0, 0)
        size = struct.calcsize("@" + MODEINFO)
        for connector_id in list(conns)[:r[6]]:
            c = self.ioctl(DRM_IOCTL_MODE_GETCONNECTOR, GET_CONNECTOR, *([0] * 8 + [connector_id] + [0] * 7))
            count = c[4]
            modes = (ctypes.c_char * (size * max(count, 1)))()
            c = self.ioctl(DRM_IOCTL_MODE_GETCONNECTOR, GET_CONNECTOR, 0, ctypes.addressof(modes), 0, 0,
                           count, 0, 0, 0, connector_id, 0, 0, 0, 0, 0, 0, 0)
            count = min(count, c[4])
            infos = [struct.unpack_from("@" + MODEINFO, modes, size * i) for i in range(count)]
            # (clock, hdisplay, ..., vrefresh, flags, type, name)
            preferred = [m for m in infos if m[13] & DRM_MODE_TYPE_PREFERRED] or infos[:1]
            mode = f"{preferred[0][1]}x{preferred[0][6]}@{preferred[0][11]}" if preferred else "none"
            ctype = DRM_CONNECTOR_TYPES[c[5]] if c[5] < len(DRM_CONNECTOR_TYPES) else str(c[5])
            status = {1: "connected", 2: "disconnected"}.get(c[7], "unknown")
            props = self.props(connector_id, DRM_MODE_OBJECT_CONNECTOR)
            lines.append(f"drm: connector {ctype}-{c[6]} {status}, {count} modes, preferred {mode}, "
                         f"vrr_capable {props.get('vrr_capable', 'none')}, "
                         f"panel orientation {props.get('panel orientation', 'none')}")
        crtc_props = self.props(list(crtcs)[self.crtc_index], DRM_MODE_OBJECT_CRTC)
        lines.append(f"drm: crtc VRR_ENABLED {crtc_props.get('VRR_ENABLED', 'none')}")
        for plane_id, crtc_id, fb_id, props in self.planes():
            if not fb_id:
                continue
            fb = self.fb(fb_id)
            self.close_handles(fb)
            ptype = DRM_PLANE_TYPES.get(props.get("type"), props.get("type"))
            modifier = f"0x{fb[17]:x}" if fb[4] & DRM_MODE_FB_MODIFIERS else "none"
            lines.append(f"drm: plane {ptype} fb {fb[1]}x{fb[2]} {fourcc(fb[3])} modifier {modifier}")
        return lines

    def crc(self):
        base = f"/sys/kernel/debug/dri/{self.minor}/crtc-{self.crtc_index}/crc"
        with open(f"{base}/control", "w") as f:
            f.write("auto")
        fd = os.open(f"{base}/data", os.O_RDONLY | os.O_NONBLOCK | os.O_CLOEXEC)
        try:
            values = []
            pending = b""
            end = time.monotonic() + 5.0
            while time.monotonic() < end:
                ready, _, _ = select.select([fd], [], [], 1.0)
                if not ready:
                    return ["drm: crc none"]
                try:
                    pending += os.read(fd, 4096)
                except BlockingIOError:
                    continue
                *lines, pending = pending.split(b"\n")
                for line in lines:
                    words = line.split()
                    if len(words) >= 2:
                        values.append(int(words[1], 16))
                # (the first two may be of frames composed before)
                if len(values) >= 5 and values[-1] == values[-2] == values[-3]:
                    return [f"drm: crc {values[-1]:#010x}"]
            return ["drm: crc unstable"]
        finally:
            os.close(fd)


VKMS_CONFIGFS = "/sys/kernel/config/vkms"


class Vkms:
    """A vkms device made through configfs: N pipelines of a primary and a
    cursor plane, a CRTC, an encoder and a connected connector each."""

    def __init__(self, name, count):
        if not os.path.isdir(VKMS_CONFIGFS):
            if not os.path.ismount("/sys/kernel/config"):
                subprocess.run(["mount", "-t", "configfs", "none", "/sys/kernel/config"], check=False)
            if not os.path.isdir(VKMS_CONFIGFS):
                raise RequestError(f"no {VKMS_CONFIGFS} (modprobe vkms; a kernel with vkms's configfs)")
        self.name = name
        self.path = f"{VKMS_CONFIGFS}/{name}"
        self.count = count
        self.links = []
        self.dirs = []
        os.mkdir(self.path)
        try:
            for i in range(count):
                for kind, item in (("crtcs", f"crtc{i}"), ("encoders", f"encoder{i}"),
                                   ("connectors", f"connector{i}")):
                    self.mkdir(f"{kind}/{item}")
                for plane, ptype in ((f"primary{i}", 1), (f"cursor{i}", 2)):
                    self.mkdir(f"planes/{plane}")
                    self.write(f"planes/{plane}/type", ptype)
                    self.link(f"crtcs/crtc{i}", f"planes/{plane}/possible_crtcs/crtc{i}")
                self.link(f"crtcs/crtc{i}", f"encoders/encoder{i}/possible_crtcs/crtc{i}")
                self.link(f"encoders/encoder{i}", f"connectors/connector{i}/possible_encoders/encoder{i}")
                self.write(f"connectors/connector{i}/status", 1)
            self.write("enabled", 1)
            self.card = None
            drm = f"/sys/devices/faux/{name}/drm"
            end = time.monotonic() + 10.0
            while time.monotonic() < end:
                cards = [c for c in (os.listdir(drm) if os.path.isdir(drm) else []) if c.startswith("card")]
                if cards and os.path.exists(f"/dev/dri/{cards[0]}"):
                    connectors = sorted(c[len(cards[0]) + 1:] for c in os.listdir(f"{drm}/{cards[0]}")
                                        if c.startswith(cards[0] + "-"))
                    if len(connectors) == count:
                        self.card = cards[0]
                        self.connectors = sorted(connectors, key=lambda c: (len(c), c))
                        break
                time.sleep(0.05)
            if self.card is None:
                raise RequestError(f"vkms {name}: no card with {count} connectors after 10 s")
            # (the card opened and closed: the kernel's fbdev emulation sets
            # its mode on every CRTC at a client's last close, as for the
            # card the programs use, so every run starts from that scanout)
            index = int(self.card[len("card"):])
            Card(index).close()
            end = time.monotonic() + 10.0
            while True:
                card = Card(index)
                try:
                    on = all(card.crtc(c)[7] for c in card.crtcs())
                finally:
                    card.close()
                if on:
                    break
                if time.monotonic() >= end:
                    raise RequestError(f"vkms {name}: its CRTCs aren't all on after 10 s")
                time.sleep(0.05)
        except BaseException:
            self.destroy()
            raise

    def mkdir(self, rel):
        os.mkdir(f"{self.path}/{rel}")
        self.dirs.append(f"{self.path}/{rel}")

    def link(self, target, rel):
        os.symlink(f"{self.path}/{target}", f"{self.path}/{rel}")
        self.links.append(f"{self.path}/{rel}")

    def write(self, rel, value):
        with open(f"{self.path}/{rel}", "w") as f:
            f.write(f"{value}\n")

    def destroy(self):
        try:
            with open(f"{self.path}/enabled") as f:
                enabled = f.read().strip() == "1"
            if enabled:
                self.write("enabled", 0)
        except OSError:
            pass
        for link in reversed(self.links):
            os.unlink(link)
        self.links.clear()
        for d in reversed(self.dirs):
            os.rmdir(d)
        self.dirs.clear()
        os.rmdir(self.path)
        # (the card's node goes once the device is gone)
        card = getattr(self, "card", None)
        end = time.monotonic() + 10.0
        while card and os.path.exists(f"/dev/dri/{card}") and time.monotonic() < end:
            time.sleep(0.05)


class Runner:
    def __init__(self, console=None):
        self.devices = {}
        self.console = console
        # The cards held by `drm master hold`
        self.masters = []
        # The vkms devices of `vkms add`
        self.vkms = {}
        # The card and CRTC of `drm card` (None: SDL_KMSDRM_DEVICE_INDEX's)
        self.card = None
        self.crtc_index = 0

    def new_card(self):
        return Card(self.card, self.crtc_index)

    def vkms_request(self, words):
        if len(words) == 4 and words[1] == "add":
            name, count = words[2], int(words[3])
            if name in self.vkms:
                raise RequestError(f"vkms {name} exists")
            device = Vkms(name, count)
            self.vkms[name] = device
            return [f"vkms: added {name} {device.card} {' '.join(device.connectors)}"]
        if len(words) == 3 and words[1] == "remove":
            device = self.vkms.pop(words[2], None)
            if device is None:
                raise RequestError(f"no vkms {words[2]}")
            device.destroy()
            return [f"vkms: removed {words[2]}"]
        raise RequestError(f"unknown request {shlex.join(words)!r}")

    def drm(self, words):
        what = shlex.join(words[1:])
        if what == "master hold":
            held, not_held = [], []
            for name in sorted(os.listdir("/dev/dri")):
                if not name.startswith("card"):
                    continue
                fd = os.open(f"/dev/dri/{name}", os.O_RDWR | os.O_CLOEXEC)
                self.masters.append(fd)
                try:
                    fcntl.ioctl(fd, DRM_IOCTL_SET_MASTER)
                    held.append(name)
                except OSError:
                    not_held.append(name)
            # (which cards there are, and their numbers, vary by machine:
            # the answer names them only when one's master can't be had)
            if not not_held:
                return ["drm: master held of every card"]
            return [f"drm: master held of {' '.join(held) or 'no card'} but not of {' '.join(not_held)}"]
        if what == "versions":
            def package(name):
                r = subprocess.run(["dpkg-query", "-W", "-f", "${Version}", name], capture_output=True,
                                   text=True)
                return r.stdout if r.returncode == 0 else "unknown"
            return [f"versions: kernel {os.uname().release}, Mesa {package('libgbm1')}, "
                    f"libdrm {package('libdrm2')}"]
        if len(words) == 2 and words[1] == "card":
            self.card = None
            self.crtc_index = 0
            return ["drm: card default"]
        if len(words) in (3, 4) and words[1] == "card":
            self.card = int(words[2])
            self.crtc_index = int(words[3]) if len(words) == 4 else 0
            return [f"drm: card {self.card} crtc {self.crtc_index}"]
        if what == "master":
            # (a descriptor that gets DRM master: none else has it)
            card = self.new_card()
            try:
                fcntl.ioctl(card.fd, DRM_IOCTL_SET_MASTER)
                fcntl.ioctl(card.fd, DRM_IOCTL_DROP_MASTER)
                return ["drm: master free"]
            except OSError:
                return ["drm: master taken"]
            finally:
                card.close()
        if what == "master release":
            for fd in self.masters:
                os.close(fd)
            self.masters.clear()
            return ["drm: master released"]
        card = self.new_card()
        try:
            if what == "state":
                return card.state()
            if what == "crc":
                return card.crc()
            if what == "info":
                return card.info()
            if len(words) >= 4 and words[1] == "pixels" and len(words) % 2 == 0:
                values = [int(w, 0) for w in words[2:]]
                return card.pixels(list(zip(values[::2], values[1::2])))
        finally:
            card.close()
        raise RequestError(f"unknown request {shlex.join(words)!r}")

    def request(self, words):
        if words and words[0] == "vt":
            if self.console is None:
                raise RequestError("vt requests need --vt")
            return self.console.request(words)
        if words and words[0] == "drm":
            return self.drm(words)
        if words and words[0] == "vkms":
            return self.vkms_request(words)
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
        for fd in self.masters:
            os.close(fd)
        self.masters.clear()
        for device in self.devices.values():
            try:
                device.destroy()
            except (OSError, RequestError) as e:
                print(f"linux_console_run.py: {device.name}: {e}", file=sys.stderr)
        self.devices.clear()
        for device in self.vkms.values():
            try:
                device.destroy()
            except OSError as e:
                print(f"linux_console_run.py: vkms {device.name}: {e}", file=sys.stderr)
        self.vkms.clear()


def main():
    args = sys.argv[1:]
    if not args or args[0] in ("-h", "--help"):
        print(__doc__)
        return 0 if args else 2
    console = None
    if args[0] == "--vt":
        if len(args) < 3:
            print(__doc__)
            return 2
        console = Console(int(args[1]))
        args = args[2:]
    timeout = float(os.environ.get("SDL3_CONSOLE_TIMEOUT", "300"))
    env = dict(os.environ, SDL3_CONSOLE_RUNNER=os.path.abspath(__file__))
    if "SDL_KMSDRM_DEVICE_INDEX" in os.environ and os.geteuid() == 0:
        # The card opened and closed once: the last client's close makes the
        # kernel's fbdev emulation set its own mode again, as after every
        # program (and request), so the first run starts from the scanout
        # the later ones do
        try:
            Card().close()
        except OSError as e:
            print(f"linux_console_run.py: the DRM card: {e}", file=sys.stderr)
    if console is None:
        proc = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE, env=env,
                                start_new_session=True)
        replies = proc.stdin
    else:
        if console.switch(console.number) != console.number:
            print(f"linux_console_run.py: {console.path} isn't active after 10 s", file=sys.stderr)
            console.finish()
            return 1
        reply_read, reply_write = os.pipe()
        env.update(SDL3_CONSOLE_REPLY_FD=str(reply_read), SDL3_CONSOLE_VT=str(console.number))
        # (setsid -c: a session of its own, whose controlling terminal is its
        # stdin, the console; not forking, as this child of the script leads
        # no process group)
        tty = os.open(console.path, os.O_RDWR | os.O_NOCTTY | os.O_CLOEXEC)
        try:
            proc = subprocess.Popen(["setsid", "-c"] + args, stdin=tty, stdout=subprocess.PIPE, env=env,
                                    pass_fds=(reply_read,))
        finally:
            os.close(tty)
            os.close(reply_read)
        replies = os.fdopen(reply_write, "wb")
    timed_out = threading.Event()

    def stop():
        timed_out.set()
        proc.kill()

    timer = threading.Timer(timeout, stop)
    timer.start()
    runner = Runner(console)
    failed = False
    try:
        for raw in proc.stdout:
            line = raw.decode("utf-8", "replace").rstrip("\n")
            print(line, flush=True)
            if not line.startswith(PREFIX + "request "):
                continue
            try:
                lines = runner.request(shlex.split(line[len(PREFIX + "request "):]))
            except (RequestError, OSError, ValueError) as e:
                lines = [f"error: {e}"]
                failed = True
                print(f"linux_console_run.py: {line}: {e}", file=sys.stderr, flush=True)
            answer = "".join(f"{PREFIX}reply {r}\n" for r in lines) + f"{PREFIX}done\n"
            try:
                replies.write(answer.encode())
                replies.flush()
            except BrokenPipeError:
                pass
        proc.wait()
    finally:
        timer.cancel()
        if proc.poll() is None:
            proc.kill()
            proc.wait()
        runner.clean_up()
        if console is not None:
            try:
                replies.close()
            except BrokenPipeError:
                pass
            # (however the program ended)
            if not console.finish():
                failed = True
    if timed_out.is_set():
        print(f"linux_console_run.py: {args[0]} didn't end in {timeout:g} s", file=sys.stderr)
        return 1
    status = proc.returncode
    if status < 0:
        status = 128 - status
    if failed and status == 0:
        return 1
    return status

if __name__ == "__main__":
    sys.exit(main())
