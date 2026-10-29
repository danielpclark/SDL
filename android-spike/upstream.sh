#!/bin/bash
# Build upstream's android-project (SDL 3.5.0 at $UPSTREAM_COMMIT) with
# Gradle and c/main.c as the app (question 11). Prints TIMING lines.
# Env: UPSTREAM_COMMIT, RUNNER_TEMP, ANDROID_HOME, SPIKE_NDK_VERSION.
set -e
here=$(cd "$(dirname "$0")" && pwd)
up=$RUNNER_TEMP/upstream
proj=$RUNNER_TEMP/cref
s=$(date +%s)
git init -q "$up"
git -C "$up" fetch -q --depth 1 https://github.com/libsdl-org/SDL "$UPSTREAM_COMMIT"
git -C "$up" checkout -q FETCH_HEAD
echo "TIMING upstream fetch $(( $(date +%s) - s ))s: $(git -C "$up" log -1 --format='%H %s')"
rm -rf "$proj"
cp -r "$up/android-project" "$proj"
ln -s "$up" "$proj/app/jni/SDL"
cp "$here/c/main.c" "$proj/app/jni/src/main.c"
sed -i 's/YourSourceHere.c/main.c/' "$proj/app/jni/src/Android.mk"
sed -i "s/abiFilters 'arm64-v8a'/abiFilters 'x86_64'/" "$proj/app/build.gradle"
grep -n 'ndkVersion' "$proj/app/build.gradle"
if [ ! -d "$ANDROID_HOME/ndk/28.2.13676358" ]; then
  echo "NDK 28.2.13676358 (the template's) is not installed; using $SPIKE_NDK_VERSION"
  sed -i "s/ndkVersion = \"28.2.13676358\"/ndkVersion = \"$SPIKE_NDK_VERSION\"/" "$proj/app/build.gradle"
fi
grep -n 'ndkVersion\|abiFilters' "$proj/app/build.gradle"
cd "$proj"
java -version 2>&1 | head -1
s=$(date +%s)
./gradlew --no-daemon -q assembleDebug 2>&1 | tail -40
echo "TIMING gradle assembleDebug (first, with the Gradle and AGP downloads) $(( $(date +%s) - s ))s"
s=$(date +%s)
touch app/jni/src/main.c
./gradlew --no-daemon -q assembleDebug 2>&1 | tail -10
echo "TIMING gradle assembleDebug (again, main.c touched) $(( $(date +%s) - s ))s"
apk=app/build/outputs/apk/debug/app-debug.apk
ls -la "$apk"
unzip -l "$apk" | tail -20
cp "$apk" "$RUNNER_TEMP/cref.apk"
