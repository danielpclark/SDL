#!/bin/bash
# diag only: run as root (sudo -E). Every check is guarded; nothing stops the script.
D="$(cd "$(dirname "$0")" && pwd)"
B="${RUNNER_TEMP:-/tmp}/probe"
mkdir -p "$B"
U="${SUDO_USER:-runner}"
sec() { echo; echo "===== $*"; }

sec versions
uname -a
grep -E '^(PRETTY_NAME|VERSION_ID)=' /etc/os-release
dpkg-query -W -f '${Package} ${Version}\n' libdrm2 libdrm-tests libgbm1 libegl-mesa0 mesa-libgallium libgl1-mesa-dri mesa-vulkan-drivers libvulkan1 kmscube mesa-utils vulkan-tools udev kbd linux-libc-dev 2>&1
nproc; free -m | head -2; id "$U"

sec kernel config
CFG="/boot/config-$(uname -r)"
ls -l "$CFG" || true
grep -E '^(# )?CONFIG_(DRM|DRM_VKMS|DRM_VGEM|DRM_HYPERV|DRM_KMS_HELPER|DRM_FBDEV_EMULATION|DRM_SIMPLEDRM|DRM_BOCHS|DRM_VIRTIO_GPU|INPUT_UINPUT|INPUT_EVDEV|VT|VT_CONSOLE|VT_HW_CONSOLE_BINDING|HW_CONSOLE|FRAMEBUFFER_CONSOLE|DUMMY_CONSOLE|DEBUG_FS|CONFIGFS_FS|KVM|KVM_INTEL|KVM_AMD|FB|FB_EFI|HYPERV_KEYBOARD|SERIO_I8042|KEYBOARD_ATKBD|SYSFB_SIMPLEFB|DRM_PANEL_ORIENTATION_QUIRKS)[= ]' "$CFG" || echo "(no matches)"
find "/lib/modules/$(uname -r)" -name 'vkms*' -o -name 'uinput*' -o -name 'vgem*' 2>/dev/null
modinfo vkms 2>&1 | grep -E '^(filename|parm|depends)' || true
cat /proc/cmdline

sec devices before
lsmod | grep -E '^(drm|vkms|uinput|kvm|hyperv)' || echo "(none of drm/vkms/uinput/kvm loaded as modules)"
ls -l /dev/dri /dev/uinput /dev/kvm /dev/fb* /dev/console /dev/tty0 /dev/tty1 /dev/tty3 /dev/tty63 2>&1
ls /dev/tty[0-9]* 2>/dev/null | wc -l
cat /proc/consoles
echo "tty0 active: $(cat /sys/class/tty/tty0/active 2>&1)"; echo "console active: $(cat /sys/class/tty/console/active 2>&1)"
fgconsole 2>&1 || true
ps -eo pid,tty,args | grep -E 'getty|login' | grep -v grep || echo "(no getty)"
ls /sys/class/drm 2>&1

sec build the probes
CFLAGS="-O1 -Wall $(pkg-config --cflags libdrm gbm egl glesv2 vulkan)"
gcc $CFLAGS -o "$B/layout" "$D/layout.c" && echo layout ok
gcc $CFLAGS -o "$B/drmprobe" "$D/drmprobe.c" $(pkg-config --libs libdrm) && echo drmprobe ok
gcc $CFLAGS -o "$B/egl_gbm" "$D/egl_gbm.c" $(pkg-config --libs libdrm gbm egl glesv2) && echo egl_gbm ok
gcc $CFLAGS -o "$B/vkdisplay" "$D/vkdisplay.c" $(pkg-config --libs vulkan) && echo vkdisplay ok
gcc -O1 -Wall -o "$B/vtprobe" "$D/vtprobe.c" && echo vtprobe ok
grep -n 'DRM_EVENT_CONTEXT_VERSION' /usr/include/xf86drm.h

sec modprobe vkms
modprobe vkms enable_cursor=1 enable_overlay=1 enable_writeback=1; echo "modprobe vkms: $?"
sleep 1
for p in /sys/module/vkms/parameters/*; do echo "$p=$(cat "$p")"; done 2>/dev/null
dmesg | grep -iE 'vkms|drm' | tail -20
ls -l /dev/dri 2>&1
mountpoint -q /sys/kernel/debug || mount -t debugfs none /sys/kernel/debug
ls /sys/kernel/debug/dri 2>&1
for n in /sys/kernel/debug/dri/*/name; do echo "$n: $(cat "$n")"; done 2>/dev/null
CARD=""; DBG=""
for c in /dev/dri/card*; do
  m=$((16#$(stat -c %T "$c")))
  name=$(cat "/sys/kernel/debug/dri/$m/name" 2>/dev/null)
  echo "$c minor $m: $name; device $(readlink -f /sys/class/drm/$(basename "$c")/device)"
  case "$name" in vkms*) CARD="$c"; DBG="/sys/kernel/debug/dri/$m";; esac
done
if [ -z "$CARD" ]; then
  for d in /sys/kernel/debug/dri/*; do grep -q vkms "$d/name" 2>/dev/null && DBG="$d"; done
  CARD=$(ls /dev/dri/card* 2>/dev/null | tail -1)
fi
echo "vkms card: $CARD debugfs: $DBG"
ls "$DBG" "$DBG/crtc-0" "$DBG/crtc-0/crc" 2>&1

sec modetest
modetest -M vkms -c 2>&1 | head -60
modetest -M vkms -p 2>&1 | head -60

sec drmprobe
"$B/drmprobe" "$CARD" "$DBG" 2>&1

sec egl_gbm
"$B/egl_gbm" "$CARD" 2>&1 | fold -w 2000
if [ -n "$DBG" ]; then
  echo auto > "$DBG/crtc-0/crc/control" 2>&1
  timeout 3 head -3 "$DBG/crtc-0/crc/data" 2>&1 || true
fi

sec kmscube
kmscube --help 2>&1 | head -30
(timeout 25 kmscube -D "$CARD" -c 120 > "$B/kmscube.log" 2>&1; echo "kmscube status $?" >> "$B/kmscube.log") &
KP=$!
sleep 4
if [ -n "$DBG" ]; then
  echo auto > "$DBG/crtc-0/crc/control" 2>&1
  timeout 3 head -5 "$DBG/crtc-0/crc/data" 2>&1 || echo "(no crc lines)"
fi
wait $KP
head -40 "$B/kmscube.log"
(timeout 25 kmscube -A -D "$CARD" -c 60 > "$B/kmscube-atomic.log" 2>&1; echo "kmscube -A status $?" >> "$B/kmscube-atomic.log")
head -20 "$B/kmscube-atomic.log"

sec eglinfo
eglinfo --help 2>&1 | head -12
eglinfo -B -p gbm 2>&1 | head -40
eglinfo -p gbm 2>&1 | grep -oE 'EGL_ANDROID_native_fence_sync|EGL_KHR_fence_sync|EGL_KHR_platform_gbm|EGL_MESA_platform_gbm' | sort -u || true

sec vulkaninfo
vulkaninfo --summary 2>&1 | head -40
vulkaninfo 2>&1 | grep -iE 'VK_KHR_display|VK_EXT_direct_mode_display|VK_EXT_acquire_drm_display|VK_KHR_get_display_properties2' | sort -u || true
"$B/vkdisplay" 2>&1

sec vkms configfs
mountpoint -q /sys/kernel/config || mount -t configfs none /sys/kernel/config
ls -la /sys/kernel/config 2>&1
ls -la /sys/kernel/config/vkms 2>&1
if [ -d /sys/kernel/config/vkms ]; then
  V=/sys/kernel/config/vkms/probe
  mkdir "$V" && ls -la "$V"
  for i in 0 1; do
    mkdir "$V/planes/plane$i" "$V/crtcs/crtc$i" "$V/encoders/encoder$i" "$V/connectors/connector$i" 2>&1
    echo 1 > "$V/planes/plane$i/type" 2>&1
    ln -s "$V/crtcs/crtc$i" "$V/planes/plane$i/possible_crtcs/" 2>&1
    ln -s "$V/crtcs/crtc$i" "$V/encoders/encoder$i/possible_crtcs/" 2>&1
    ln -s "$V/encoders/encoder$i" "$V/connectors/connector$i/possible_encoders/" 2>&1
  done
  find "$V" -maxdepth 3 | head -40
  echo 1 > "$V/enabled"; echo "enable: $?"
  sleep 1
  ls -l /dev/dri; dmesg | tail -5
  for c in /dev/dri/card*; do [ "$c" != "$CARD" ] && modetest -D "$c" -c 2>&1 | grep -E 'connected|^[0-9]' | head -10; done
  echo 0 > "$V/enabled"
fi

sec uinput
modprobe uinput 2>&1; echo "modprobe uinput: $?"
ls -l /dev/uinput
sudo -u "$U" python3 -c "open('/dev/uinput', 'wb')" 2>&1 && echo "user can open /dev/uinput (before the rule)"
echo 'KERNEL=="uinput", SUBSYSTEM=="misc", MODE="0666", OPTIONS+="static_node=uinput"' > /etc/udev/rules.d/99-uinput.rules
udevadm control --reload-rules
udevadm trigger --name-match=uinput
udevadm settle -t 10
ls -l /dev/uinput
sudo -u "$U" python3 -c "open('/dev/uinput', 'wb')" 2>&1 && echo "user can open /dev/uinput (after the rule)"
python3 "$D/uinput_probe.py" 2>&1
echo "-- as $U:"
sudo -u "$U" python3 "$D/uinput_probe.py" 2>&1 | head -12
ls -l /dev/input/

sec VTs
"$B/vtprobe" /dev/tty3 2>&1
echo "-- setsid -c on tty3, then /dev/tty (upstream's open):"
setsid -c -w sh -c "$B/vtprobe /dev/tty > $B/vt-ctty.log 2>&1" < /dev/tty3; echo "setsid status $?"
cat "$B/vt-ctty.log"
echo "-- chvt:"
O=$(fgconsole 2>/dev/null || cat /sys/class/tty/tty0/active)
echo "fgconsole before: $O"
timeout 10 chvt 3; echo "chvt 3: $?, fgconsole $(fgconsole 2>&1)"
timeout 10 chvt "${O#tty}"; echo "chvt back: $?, fgconsole $(fgconsole 2>&1)"
echo "-- VT_PROCESS switching on tty3:"
timeout 30 "$B/vtprobe" /dev/tty3 3 2>&1
echo "fgconsole after: $(fgconsole 2>&1)"

sec layout
"$B/layout"

sec dmesg tail
dmesg | tail -30
