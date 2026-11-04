#!/usr/bin/env python3
"""diag only: uinput devices made with the standard library (as the console
runner will), and how udev classifies them."""
import fcntl
import os
import struct
import subprocess
import sys
import time

IOC_WRITE, IOC_READ = 1, 2


def ioc(d, t, nr, size):
    return (d << 30) | (size << 16) | (ord(t) << 8) | nr


UI_DEV_CREATE = ioc(0, "U", 1, 0)
UI_DEV_DESTROY = ioc(0, "U", 2, 0)
UI_DEV_SETUP = ioc(IOC_WRITE, "U", 3, 92)
UI_ABS_SETUP = ioc(IOC_WRITE, "U", 4, 28)
UI_SET_EVBIT = ioc(IOC_WRITE, "U", 100, 4)
UI_SET_KEYBIT = ioc(IOC_WRITE, "U", 101, 4)
UI_SET_RELBIT = ioc(IOC_WRITE, "U", 102, 4)
UI_SET_ABSBIT = ioc(IOC_WRITE, "U", 103, 4)
UI_SET_PROPBIT = ioc(IOC_WRITE, "U", 110, 4)


def UI_GET_SYSNAME(n):
    return ioc(IOC_READ, "U", 44, n)


EV_SYN, EV_KEY, EV_REL, EV_ABS = 0, 1, 2, 3


def make(name, evs, keys=(), rels=(), abss=(), props=()):
    fd = os.open("/dev/uinput", os.O_WRONLY | os.O_NONBLOCK)
    for e in evs:
        fcntl.ioctl(fd, UI_SET_EVBIT, e)
    for k in keys:
        fcntl.ioctl(fd, UI_SET_KEYBIT, k)
    for r in rels:
        fcntl.ioctl(fd, UI_SET_RELBIT, r)
    for p in props:
        fcntl.ioctl(fd, UI_SET_PROPBIT, p)
    for code, lo, hi in abss:
        fcntl.ioctl(fd, UI_SET_ABSBIT, code)
        fcntl.ioctl(fd, UI_ABS_SETUP, struct.pack("H2x6i", code, 0, lo, hi, 0, 0, 0))
    fcntl.ioctl(fd, UI_DEV_SETUP, struct.pack("HHHH80sI", 3, 0x1234, 0x5678, 1, name.encode(), 0))
    fcntl.ioctl(fd, UI_DEV_CREATE)
    buf = bytearray(64)
    fcntl.ioctl(fd, UI_GET_SYSNAME(64), buf)
    return fd, buf.split(b"\0")[0].decode()


def main():
    print("UI_DEV_SETUP 0x%x UI_ABS_SETUP 0x%x UI_SET_EVBIT 0x%x UI_GET_SYSNAME(64) 0x%x" % (UI_DEV_SETUP, UI_ABS_SETUP, UI_SET_EVBIT, UI_GET_SYSNAME(64)))
    devs = []
    try:
        devs.append(make("probe keyboard", [EV_KEY], keys=range(1, 128)))
        devs.append(make("probe mouse", [EV_KEY, EV_REL], keys=range(0x110, 0x118), rels=[0, 1, 6, 8, 11, 12]))
        devs.append(make("probe abs pointer", [EV_KEY, EV_ABS], keys=[0x110, 0x111, 0x112], abss=[(0, 0, 32767), (1, 0, 32767)]))
        devs.append(make("probe touchscreen", [EV_KEY, EV_ABS], keys=[0x14a], props=[1],
                         abss=[(0, 0, 1000), (1, 0, 1000), (0x2f, 0, 9), (0x35, 0, 1000), (0x36, 0, 1000), (0x39, 0, 65535), (0x3a, 0, 255)]))
    except OSError as e:
        print("uinput failed:", e)
    t0 = time.monotonic()
    subprocess.run(["udevadm", "settle", "-t", "10"])
    print("udevadm settle took %.2fs" % (time.monotonic() - t0))
    for fd, sysname in devs:
        sys_dir = "/sys/devices/virtual/input/" + sysname
        events = [n for n in os.listdir(sys_dir) if n.startswith("event")]
        print("==", sysname, open(sys_dir + "/name").read().strip(), events)
        for ev in events:
            node = "/dev/input/" + ev
            st = os.stat(node)
            print("  %s mode %o uid %d gid %d" % (node, st.st_mode & 0o7777, st.st_uid, st.st_gid))
            out = subprocess.run(["udevadm", "info", "-q", "property", "-n", node], capture_output=True, text=True).stdout
            print("  " + " ".join(line for line in out.split() if line.startswith("ID_INPUT")))
        # one key press and release on the keyboard, to see the events flow
        if sysname == devs[0][1]:
            evfd = os.open("/dev/input/" + events[0], os.O_RDONLY | os.O_NONBLOCK)
            for t, c, v in [(EV_KEY, 30, 1), (EV_SYN, 0, 0), (EV_KEY, 30, 0), (EV_SYN, 0, 0)]:
                os.write(fd, struct.pack("llHHi", 0, 0, t, c, v))
            time.sleep(0.2)
            data = os.read(evfd, 24 * 16)
            print("  read %d bytes: %s" % (len(data), [struct.unpack("llHHi", data[i:i + 24])[2:] for i in range(0, len(data), 24)]))
            os.close(evfd)
    for fd, _ in devs:
        fcntl.ioctl(fd, UI_DEV_DESTROY)
        os.close(fd)


main()
