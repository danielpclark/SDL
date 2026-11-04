#!/bin/bash
# diag only: the OpenXR probe on the Linux runner, as the runner user.
# Usage: diag/linux.sh OUTDIR   (upstream SDL built in $UP/build)
set -u
O="$1"
D="$(cd "$(dirname "$0")" && pwd)"
mkdir -p "$O"
X="$RUNNER_TEMP/xdg"
rm -rf "$X"
mkdir -m 700 -p "$X"
LVP="$(ls /usr/share/vulkan/icd.d/lvp_icd*.json | head -1)"
echo "== whoami $(id)"
echo "== lavapipe ICD $LVP"
echo "== versions"
dpkg-query -W monado-service libopenxr1-monado libopenxr-loader1 libopenxr-dev openxr-layer-corevalidation vulkan-validationlayers mesa-vulkan-drivers libvulkan1 2>&1
echo "== active runtime"
ls -l /etc/xdg/openxr/1/ 2>&1
cat /etc/xdg/openxr/1/active_runtime.json 2>&1
echo "== layer manifests"
find /usr/share/openxr /etc/openxr /etc/xdg/openxr /usr/share/vulkan/explicit_layer.d /usr/share/vulkan/implicit_layer.d -name '*.json' 2>/dev/null
echo "== XDG_DATA_DIRS=${XDG_DATA_DIRS:-unset} XDG_CONFIG_DIRS=${XDG_CONFIG_DIRS:-unset}"

export XDG_RUNTIME_DIR="$X" VK_ICD_FILENAMES="$LVP" XRT_COMPOSITOR_NULL=1 IPC_EXIT_ON_DISCONNECT=0

echo "== monado-service with stdin /dev/null"
timeout 20 monado-service < /dev/null > "$O/monado-devnull.log" 2>&1
echo "exit $?"
tail -5 "$O/monado-devnull.log"
rm -f "$X"/monado_comp_ipc "$X"/monado.pid

start_monado() {
    rm -f "$X"/monado_comp_ipc "$X"/monado.pid
    # stdin: a pipe held open by a sleep the script kills afterwards
    (sleep 1800 | XRT_COMPOSITOR_LOG=info monado-service > "$O/monado-$1.log" 2>&1) &
    MON=$!
    for i in $(seq 1 200); do [ -S "$X/monado_comp_ipc" ] && break; sleep 0.1; done
    if [ -S "$X/monado_comp_ipc" ]; then echo "monado-$1: socket after $i polls"; else echo "monado-$1: NO SOCKET"; tail -30 "$O/monado-$1.log"; fi
}
stop_monado() {
    pkill -f "sleep 1800" 2>/dev/null
    for i in $(seq 1 50); do pgrep -x monado-service > /dev/null || break; sleep 0.1; done
    pkill -x monado-service 2>/dev/null
    wait "$MON" 2>/dev/null
    echo "monado-$1 stopped ($(grep -c . "$O/monado-$1.log") log lines)"
}

start_monado probe
grep -E "Null compositor|Simulated|llvmpipe|ERROR" "$O/monado-probe.log" | head -10
echo "== xrprobe (no layer)"
timeout 120 "$O/xrprobe" > "$O/xrprobe.log" 2>&1; echo "exit $?"
cat "$O/xrprobe.log"
echo "== xrprobe (core_validation, loader debug)"
XR_LOADER_DEBUG=all timeout 120 "$O/xrprobe" layer > "$O/xrprobe-layer.log" 2>&1; echo "exit $?"
grep -iE "search|looking|json|manifest|layer|validation|XR_APILAYER|runtime" "$O/xrprobe-layer.log" | head -80
grep -vE "^\[|^ " "$O/xrprobe-layer.log" | tail -40
echo "== xrprobe2 (two instances)"
timeout 120 "$O/xrprobe2" > "$O/xrprobe2.log" 2>&1; echo "exit $?"
grep -E "run|xrCreateInstance|xrDestroyInstance|^ok" "$O/xrprobe2.log"
stop_monado probe

echo "== vulkan layers (vulkaninfo)"
vulkaninfo --summary 2>&1 | sed -n '/Instance Layers/,/Devices/p' | head -20
VK_LOADER_DEBUG=layer vulkaninfo --summary 2>&1 | grep -iE "searching|validation" | head -20

echo "== SDL probe (debug mode; XR core_validation and VK_LAYER_KHRONOS_validation if found)"
start_monado sdl
SDL_VIDEO_DRIVER=offscreen timeout 120 "$O/sdlxr" > "$O/sdlxr.log" 2>&1; echo "exit $?"
cat "$O/sdlxr.log" | head -300
stop_monado sdl

echo "== SDL probe, loader debug"
start_monado sdl2
SDL_VIDEO_DRIVER=offscreen XR_LOADER_DEBUG=warn timeout 120 "$O/sdlxr" > "$O/sdlxr2.log" 2>&1; echo "exit $?"
grep -vE "^frame|^state" "$O/sdlxr2.log" | head -120
stop_monado sdl2

echo "== testgpu_spinning_cube_xr (offscreen, 20 s)"
start_monado cube
SDL_VIDEO_DRIVER=offscreen timeout 20 "$UP/build/test/testgpu_spinning_cube_xr" > "$O/cube.log" 2>&1; echo "exit $? (124: the timeout)"
grep -c "Validation\|VUID" "$O/cube.log"
sort "$O/cube.log" | uniq -c | sort -rn | head -60
stop_monado cube

echo "== no service: xrCreateInstance"
timeout 60 "$O/xrprobe" > "$O/xrprobe-noservice.log" 2>&1; echo "exit $?"
tail -5 "$O/xrprobe-noservice.log"
echo "== no runtime"
XR_RUNTIME_JSON=/nonexistent.json timeout 60 "$O/xrprobe" > "$O/xrprobe-noruntime.log" 2>&1; echo "exit $?"
tail -5 "$O/xrprobe-noruntime.log"

echo "== SUMMARY"
echo "-- versions"
dpkg-query -W -f '${Package} ${Version}\n' monado-service libopenxr1-monado libopenxr-loader1 libopenxr-dev openxr-layer-corevalidation vulkan-validationlayers mesa-vulkan-drivers libvulkan1 2>&1
echo "-- manifests"
find /usr/share/openxr /etc/openxr /etc/xdg/openxr /usr/share/vulkan/explicit_layer.d -name '*.json' 2>/dev/null
readlink -f /etc/xdg/openxr/1/active_runtime.json
echo "-- XR loader's layer search (xrprobe layer, XR_LOADER_DEBUG=all)"
{ grep -iE "api_layers|explicit|implicit|layer.*json|core_validation" "$O/xrprobe-layer.log" || true; } | sort | uniq -c | head -20
{ grep -E "^layer:|xrCreateInstance" "$O/xrprobe-layer.log" || true; } | head
echo "-- Vulkan layers"
{ vulkaninfo --summary 2>/dev/null | grep -iE "VK_LAYER" || true; } | head
echo "-- sdlxr: validation messages"
{ grep -oE "VUID-[A-Za-z0-9_-]+" "$O/sdlxr.log" || true; } | sort | uniq -c
{ grep -iE "validation|error" "$O/sdlxr.log" || true; } | grep -v VUID | cut -c1-250 | sort | uniq -c | head -20
{ grep -E "VUID" "$O/sdlxr.log" || true; } | cut -c1-400 | sort -u | head -8
echo "-- sdlxr: XR loader messages"
{ grep -E "OpenXR-Loader" "$O/sdlxr.log" || true; } | cut -c1-250 | sort | uniq -c
echo "-- sdlxr: the destroys"
{ grep -E "Destroy|SDL_Destroy|xrDestroy|frames|^ok" "$O/sdlxr.log" || true; }
echo "-- cube: validation messages"
{ grep -oE "VUID-[A-Za-z0-9_-]+" "$O/cube.log" || true; } | sort | uniq -c
{ grep -E "OpenXR-Loader|api layer" "$O/cube.log" || true; } | cut -c1-250 | sort | uniq -c
echo "-- monado banner"
{ grep -iE "null compositor|Simulated|llvmpipe" "$O/monado-sdl.log" || true; } | cut -c1-200 | head -5
exit 0
