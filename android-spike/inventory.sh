#!/bin/bash
# What the runner image has before any install (question 1).
set +e
head -4 /etc/os-release; uname -a; echo "ImageOS=$ImageOS ImageVersion=$ImageVersion"
echo "cpus=$(nproc) vmx/svm flags=$(grep -cE 'vmx|svm' /proc/cpuinfo)"; free -h | head -2; df -h / /mnt 2>/dev/null
echo "ANDROID_HOME=$ANDROID_HOME ANDROID_SDK_ROOT=$ANDROID_SDK_ROOT"
echo "ANDROID_NDK=$ANDROID_NDK ANDROID_NDK_HOME=$ANDROID_NDK_HOME ANDROID_NDK_ROOT=$ANDROID_NDK_ROOT ANDROID_NDK_LATEST_HOME=$ANDROID_NDK_LATEST_HOME"
ls "$ANDROID_HOME"
for d in ndk build-tools platforms system-images emulator cmdline-tools platform-tools; do echo "$d: $(ls "$ANDROID_HOME/$d" 2>&1 | tr '\n' ' ')"; done
cat "$ANDROID_HOME/platform-tools/source.properties" 2>/dev/null | grep Revision
cat "$ANDROID_HOME/cmdline-tools/latest/source.properties" 2>/dev/null | grep Revision
du -sh "$ANDROID_HOME" 2>/dev/null
java -version 2>&1 | head -1; echo "JAVA_HOME=$JAVA_HOME"; ls /usr/lib/jvm
which gradle; gradle --version 2>/dev/null | grep Gradle
rustup --version 2>&1 | head -1; rustup toolchain list; cargo --version
python3 --version
ls -l /dev/kvm; id
