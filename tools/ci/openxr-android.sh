#!/bin/bash
# The OpenXR loader of the Android CI job, for sdl3/tests/android_openxr_loader.rs
# (capability openxr, see docs/HARDWARE_TESTING.md). Run from the
# repository's root.
#
#   tools/ci/openxr-android.sh
#     Khronos's OpenXR loader for Android (the Maven Central package
#     org.khronos.openxr:openxr_loader_for_android, OPENXR_LOADER_ANDROID_VERSION's
#     AAR, Apache-2.0, checked against OPENXR_LOADER_ANDROID_SHA256 and
#     Maven's SHA-1, kept in $RUNNER_TEMP/openxr-android-cache, never
#     committed): its libopenxr_loader.so of each ABI the job runs (which
#     needs no libc++_shared.so) in $RUNNER_TEMP/openxr-android/ABI/, a
#     directory of SDL3_ANDROID_LIBRARY_PATH (in GITHUB_ENV), where
#     tools/android_run.py finds the libraries of an app's libs= word.
set -euo pipefail

: "${OPENXR_LOADER_ANDROID_VERSION:?}" "${OPENXR_LOADER_ANDROID_SHA256:?}"
V="$OPENXR_LOADER_ANDROID_VERSION"
C="$RUNNER_TEMP/openxr-android-cache"
X="$RUNNER_TEMP/openxr-android"
M="https://repo1.maven.org/maven2/org/khronos/openxr/openxr_loader_for_android/$V"
aar="$C/openxr_loader_for_android-$V.aar"
mkdir -p "$C"
if [ ! -f "$aar" ]; then
  curl -fsSL --retry 5 --retry-delay 10 --retry-all-errors -o "$aar.part" "$M/openxr_loader_for_android-$V.aar"
  sha1="$(curl -fsSL --retry 5 --retry-delay 10 --retry-all-errors "$M/openxr_loader_for_android-$V.aar.sha1")"
  echo "${sha1%% *}  $aar.part" | sha1sum -c
  mv "$aar.part" "$aar"
fi
echo "$OPENXR_LOADER_ANDROID_SHA256  $aar" | sha256sum -c
rm -rf "$X"
for abi in x86_64 arm64-v8a; do
  mkdir -p "$X/$abi"
  unzip -q -j -o "$aar" "prefab/modules/openxr_loader/libs/android.$abi/libopenxr_loader.so" -d "$X/$abi"
done
ls -l "$X"/*/libopenxr_loader.so
echo "SDL3_ANDROID_LIBRARY_PATH=$X" >> "$GITHUB_ENV"
