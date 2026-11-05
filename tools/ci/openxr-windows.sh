#!/bin/bash
# The OpenXR pieces of the Windows CI job, for sdl3/tests/gpu_openxr.rs's
# test runtime scenarios (capability openxr-runtime, see
# docs/HARDWARE_TESTING.md). Run from the repository's root, in Git Bash.
#
#   tools/ci/openxr-windows.sh install
#     Khronos's OpenXR loader for Windows (OPENXR_LOADER_VERSION's release
#     zip, checked against OPENXR_LOADER_SHA256, kept in
#     $RUNNER_TEMP/gpu-cache with the GPU drivers): its openxr_loader.dll
#     beside the test programs (target/{debug,release}/deps, where the
#     sdl3 crate's tests load it by name, as upstream does), and its core
#     validation API layer registered (the loader ignores
#     XR_API_LAYER_PATH in an elevated process, as the runner's).
#   tools/ci/openxr-windows.sh runtime PROFILE
#     The repository's test runtime of a cargo profile
#     (target/PROFILE/deps/sdl3_openxr_test_runtime.dll, which cargo test
#     builds there) as the loader's active runtime: a manifest of it in
#     $RUNNER_TEMP/openxr, named by
#     HKLM\SOFTWARE\Khronos\OpenXR\1\ActiveRuntime (the loader ignores
#     XR_RUNTIME_JSON in an elevated process).
set -euo pipefail

X="$RUNNER_TEMP/openxr"
mkdir -p "$X"
case "${1:-}" in
install)
  : "${OPENXR_LOADER_VERSION:?}" "${OPENXR_LOADER_SHA256:?}"
  C="$RUNNER_TEMP/gpu-cache"
  mkdir -p "$C"
  zip="$C/openxr_loader_windows-$OPENXR_LOADER_VERSION.zip"
  if [ ! -f "$zip" ]; then
    curl -fsSL --retry 5 --retry-delay 10 -o "$zip" \
      "https://github.com/KhronosGroup/OpenXR-SDK-Source/releases/download/release-$OPENXR_LOADER_VERSION/openxr_loader_windows-$OPENXR_LOADER_VERSION.zip"
  fi
  echo "$OPENXR_LOADER_SHA256  $zip" | sha256sum -c
  7z x -y -o"$X" "$zip" x64/bin/openxr_loader.dll x64/bin/api_layers > /dev/null
  for d in target/debug/deps target/release/deps; do
    mkdir -p "$d"
    cp "$X/x64/bin/openxr_loader.dll" "$d/"
  done
  MSYS_NO_PATHCONV=1 reg add 'HKLM\SOFTWARE\Khronos\OpenXR\1\ApiLayers\Explicit' \
    /v "$(cygpath -w "$X/x64/bin/api_layers/XrApiLayer_core_validation.json")" /t REG_DWORD /d 0 /f
  ;;
runtime)
  profile=${2:?a cargo profile}
  library="$(cygpath -w "$PWD/target/$profile/deps/sdl3_openxr_test_runtime.dll")"
  manifest="$X/sdl3_openxr_test_runtime.json"
  printf '{\n    "file_format_version": "1.0.0",\n    "runtime": {\n        "name": "sdl3-openxr-test-runtime",\n        "library_path": "%s"\n    }\n}\n' \
    "${library//\\/\\\\}" > "$manifest"
  cat "$manifest"
  MSYS_NO_PATHCONV=1 reg add 'HKLM\SOFTWARE\Khronos\OpenXR\1' \
    /v ActiveRuntime /t REG_SZ /d "$(cygpath -w "$manifest")" /f
  ;;
*)
  echo "usage: $0 install | runtime PROFILE" >&2
  exit 2
  ;;
esac
