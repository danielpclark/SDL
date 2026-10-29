#!/bin/bash
# The emulated device: ABIs and ARM translation, graphics, sensors, camera,
# vibrator, input and IME (questions 6-10, host side).
set +e
h() { echo; echo "===== $*"; }
h "properties"
adb shell getprop | grep -E "ro.product.cpu.abi|native.bridge|ro.build.version.(sdk|release)|ro.build.fingerprint|ro.kernel.qemu|ro.hardware|egl|vulkan|opengles|gfxstream|ro.boot.qemu|ro.debuggable|ro.secure|ro.product.model|ro.boot.hardware|ro.hardware.audio|ro.soc|page_size|ro.zygote" | head -60
adb shell getconf PAGE_SIZE
h "ARM translation"
adb shell ls -la /system/lib64/libndk_translation.so /system/lib64/arm64 /system/bin/arm64 2>&1 | head -8
adb shell ls /proc/sys/fs/binfmt_misc 2>&1 | head
adb shell 'cat /proc/sys/fs/binfmt_misc/* 2>/dev/null | head -20'
h "display, window manager"
adb shell wm size; adb shell wm density
adb shell dumpsys SurfaceFlinger | grep -iE "GLES|EGL|vulkan|Display .*:|refresh" | head -12
adb shell dumpsys gpu 2>&1 | head -20
h "sensors"
adb shell dumpsys sensorservice | grep -E "^[0-9]+\) |Sensor List|handle" | head -30
h "camera"
adb shell dumpsys media.camera | grep -iE "Number of camera|Camera [0-9]|Facing|Device version|Resource cost" | head -20
h "vibrator"
adb shell dumpsys vibrator_manager | head -20
adb shell cmd vibrator_manager list 2>&1 | head
h "IME"
adb shell ime list -a -s
adb shell settings get secure default_input_method
adb shell pm list packages | grep -iE "inputmethod|latin|keyboard|gboard"
h "input devices"
adb shell dumpsys input | grep -E "^  [0-9]+: |Sources:|KeyboardType|Classes:" | head -40
h "audio"
adb shell dumpsys media.audio_flinger | grep -iE "Output thread|Input thread|Sample rate|HAL format|Standby|Output devices|Input device" | head -20
adb shell dumpsys audio | grep -iE "devices|stream_music" | head -10
h "packages and storage"
adb shell df -h /data | tail -1
adb shell pm list packages | wc -l
