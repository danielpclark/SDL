#!/bin/bash
# The spike jobs' common setup: KVM, the SDK packages, Rust. Prints TIMING
# lines. Env: SPIKE_IMAGE, SPIKE_NDK_VERSION, SPIKE_BUILD_TOOLS, RUST_TOOLCHAIN.
set -e
echo "::group::KVM"
ls -l /dev/kvm || true
echo 'KERNEL=="kvm", GROUP="kvm", MODE="0666", OPTIONS+="static_node=kvm"' | sudo tee /etc/udev/rules.d/99-kvm4all.rules
sudo udevadm control --reload-rules
sudo udevadm trigger --name-match=kvm
ls -l /dev/kvm
echo "::endgroup::"
SDKM=$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager
pk=(emulator platform-tools "$SPIKE_IMAGE" "platforms;android-35" "build-tools;$SPIKE_BUILD_TOOLS" "ndk;$SPIKE_NDK_VERSION")
s=$(date +%s)
yes | "$SDKM" --install "${pk[@]}" >"$RUNNER_TEMP/sdkmanager.log" 2>&1 || { tail -30 "$RUNNER_TEMP/sdkmanager.log"; exit 1; }
echo "TIMING sdkmanager --install ${pk[*]}: $(( $(date +%s) - s ))s"
grep -iE "download|install|unzip" "$RUNNER_TEMP/sdkmanager.log" | grep -v "^\[" | tail -12 || true
du -sh "$ANDROID_HOME/emulator" "$ANDROID_HOME/system-images" 2>/dev/null || true
df -h / | tail -1
{
  echo "ANDROID_NDK_ROOT=$ANDROID_HOME/ndk/$SPIKE_NDK_VERSION"
  echo "ANDROID_NDK_HOME=$ANDROID_HOME/ndk/$SPIKE_NDK_VERSION"
} >>"$GITHUB_ENV"
{
  echo "$ANDROID_HOME/emulator"
  echo "$ANDROID_HOME/platform-tools"
  echo "$ANDROID_HOME/build-tools/$SPIKE_BUILD_TOOLS"
} >>"$GITHUB_PATH"
echo "--- the emulator's missing host libraries (before apt)"
ldd "$ANDROID_HOME/emulator/qemu/linux-x86_64/qemu-system-x86_64" | grep "not found" || echo "(none)"
s=$(date +%s)
sudo apt-get install -y -q libpulse0 >/dev/null 2>&1 || { sudo apt-get update -q >/dev/null && sudo apt-get install -y -q libpulse0 >/dev/null; }
echo "TIMING apt-get install libpulse0: $(( $(date +%s) - s ))s"
ldd "$ANDROID_HOME/emulator/qemu/linux-x86_64/qemu-system-x86_64" | grep "not found" || echo "(no missing libraries now)"
mkdir -p "$HOME/.android/avd"
"$ANDROID_HOME/emulator/emulator" -version | head -1
"$ANDROID_HOME/emulator/emulator" -accel-check || true
s=$(date +%s)
rustup toolchain install "$RUST_TOOLCHAIN" --profile minimal --target x86_64-linux-android,aarch64-linux-android >/dev/null 2>&1
rustup default "$RUST_TOOLCHAIN" >/dev/null
echo "TIMING rustup toolchain + 2 android targets: $(( $(date +%s) - s ))s ($(rustc --version))"
