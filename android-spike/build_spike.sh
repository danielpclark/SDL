#!/bin/bash
# Build the spike crate for x86_64 and aarch64 through link.py; check the
# ELF results; generate and check the DEX; package the APKs.
set -e
cd "$(dirname "$0")"
here=$PWD
export CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER=$here/link-x86_64-linux-android
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$here/link-aarch64-linux-android
export SPIKE_LINK_LOG=$RUNNER_TEMP/link.log
chmod +x link-* *.py *.sh
B=$ANDROID_NDK_ROOT/toolchains/llvm/prebuilt/linux-x86_64/bin
for t in x86_64-linux-android aarch64-linux-android; do
  s=$(date +%s)
  cargo build --target $t --bins 2>&1 | tail -3
  echo "TIMING spike build $t: $(( $(date +%s) - s ))s"
done
echo "--- link log"; cat "$SPIKE_LINK_LOG"
for t in x86_64-linux-android aarch64-linux-android; do
  d=target/$t/debug
  ls -la $d/probe $d/spike_app $d/uinput_pad
  ls -la $d/deps/probe-*.pie_exp $d/deps/probe-*.shared 2>/dev/null || true
  file $d/probe $d/spike_app
  echo "--- $t spike_app: ELF type, sections, dynamic symbols"
  $B/llvm-readelf -h $d/spike_app | grep -E "Type:|Machine:"
  $B/llvm-readelf -S $d/spike_app | grep -E "sdl3|Name" || true
  $B/llvm-readelf -p .sdl3_runner $d/spike_app || true
  $B/llvm-readelf --dyn-syms $d/spike_app | grep -E " (JNI_OnLoad|SDL_main|main)$" || true
  echo "dynamic symbols: $($B/llvm-readelf --dyn-syms $d/spike_app | wc -l)"
  $B/llvm-readelf -d $d/spike_app | grep -E "NEEDED|SONAME|FLAGS" || true
  $B/llvm-readelf -l $d/spike_app | grep -E "LOAD" | head -3
  echo "--- $t probe variants"
  for v in $d/probe $d/deps/probe-*.pie_exp $d/deps/probe-*.shared; do
    [ -f "$v" ] || continue
    echo "$(basename $v): $($B/llvm-readelf -h $v | grep Type: | xargs) main/SDL_main in dynsym: $($B/llvm-readelf --dyn-syms $v | grep -cE ' (main|SDL_main)$') flags: $($B/llvm-readelf -d $v | grep FLAGS_1 | xargs)"
  done
done

set +e
echo "===== DEX"
python3 dexgen.py "$RUNNER_TEMP/gen"
cmp gen/classes.dex "$RUNNER_TEMP/gen/classes.dex" && cmp gen/inmem.dex "$RUNNER_TEMP/gen/inmem.dex" && echo "committed gen/*.dex are current"
BT=$ANDROID_HOME/build-tools/$SPIKE_BUILD_TOOLS
JAR=$ANDROID_HOME/platforms/android-35/android.jar
echo "--- dexdump -d gen/classes.dex"
$BT/dexdump -d gen/classes.dex
echo "--- dexdump -d gen/inmem.dex"
$BT/dexdump -d gen/inmem.dex | grep -vE "^\s*$" | head -60
echo "--- d8 re-dexing gen/classes.dex (a second reader's check)"
mkdir -p "$RUNNER_TEMP/redex"
$BT/d8 --min-api 21 --lib "$JAR" --output "$RUNNER_TEMP/redex" gen/classes.dex && echo "d8 accepted it ($(stat -c %s "$RUNNER_TEMP/redex/classes.dex") bytes out)"
echo "--- the javac + d8 reference (java/)"
mkdir -p "$RUNNER_TEMP/jref/classes" "$RUNNER_TEMP/jref/dex"
javac --release 8 -cp "$JAR" -d "$RUNNER_TEMP/jref/classes" $(find java -name '*.java') 2>&1 | grep -v "warning" || true
$BT/d8 --min-api 21 --lib "$JAR" --output "$RUNNER_TEMP/jref/dex" $(find "$RUNNER_TEMP/jref/classes/org/libsdl/app" -name '*.class')
ls -la "$RUNNER_TEMP/jref/dex"
$BT/dexdump -d "$RUNNER_TEMP/jref/dex/classes.dex" > "$RUNNER_TEMP/jref/dump.txt"
$BT/dexdump -d gen/classes.dex > "$RUNNER_TEMP/gen/dump.txt"
echo "--- diff of the method/class listings (reference d8 vs dexgen.py), insns and offsets dropped"
norm() { grep -E "Class descriptor|Superclass|Interfaces|#[0-9]+ +:|name +:|type +:|access +:|registers|ins +:|outs +:|insns size|invoke|const-string|return" "$1" | sed -E 's/^[0-9a-f]+: [0-9a-f ]+\|[0-9a-f]+: //; s/ \/\/ (method|string)@[0-9a-f]+//; s/0x[0-9a-f]+ //' ; }
diff <(norm "$RUNNER_TEMP/jref/dump.txt") <(norm "$RUNNER_TEMP/gen/dump.txt") && echo "(no differences)"
$BT/dexdump -f gen/classes.dex | head -40

echo "===== APKs"
x=target/x86_64-linux-android/debug
a=target/aarch64-linux-android/debug
python3 apk.py "$RUNNER_TEMP/spike.apk" x86_64 $x/spike_app \
  "$x/probe=libprobe_pie.so" \
  "$(ls $x/deps/probe-*.pie_exp | head -1)=libprobe_pie_exp.so" \
  "$(ls $x/deps/probe-*.shared | head -1)=libprobe_shared.so"
SPIKE_PKG=org.libsdl.sdl3_rust.spike python3 apk.py "$RUNNER_TEMP/spike-arm64.apk" arm64-v8a $a/spike_app \
  "$a/probe=libprobe_pie.so" | tail -6
$BT/aapt2 dump badging "$RUNNER_TEMP/spike.apk" | head -12
