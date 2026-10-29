#!/usr/bin/env python3
"""Package an APK without Gradle (the prototype of android_run.py --apk).

apk.py OUT.apk ABI LIBMAIN [EXTRA_LIB=NAME ...]

1. aapt2 link a generated manifest against android.jar;
2. classes.dex: pulled out of LIBMAIN's .sdl3_android_dex section with
   llvm-objcopy (and checked against gen/classes.dex);
3. lib/ABI/libmain.so (+ extra libs) added uncompressed;
4. zipalign -P 16 -f 4; apksigner sign with a debug key (made once with
   keytool); apksigner verify.
Env: ANDROID_HOME, ANDROID_NDK_ROOT, SPIKE_BUILD_TOOLS (e.g. 35.0.0).
"""

import hashlib
import os
import subprocess
import sys
import time
import zipfile

out, abi, libmain = sys.argv[1], sys.argv[2], sys.argv[3]
extras = [a.split("=", 1) for a in sys.argv[4:]]
sdk = os.environ["ANDROID_HOME"]
bt = os.path.join(sdk, "build-tools", os.environ.get("SPIKE_BUILD_TOOLS", "35.0.0"))
jar = os.path.join(sdk, "platforms", "android-35", "android.jar")
ndkbin = os.path.join(os.environ["ANDROID_NDK_ROOT"], "toolchains/llvm/prebuilt/linux-x86_64/bin")
here = os.path.dirname(os.path.abspath(__file__))
work = out + ".d"
os.makedirs(work, exist_ok=True)
pkg = os.environ.get("SPIKE_PKG", "org.libsdl.sdl3_rust.spike")

MANIFEST = """<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="%s"
    android:versionCode="1" android:versionName="1.0">
  <uses-permission android:name="android.permission.INTERNET"/>
  <uses-permission android:name="android.permission.CAMERA"/>
  <uses-permission android:name="android.permission.RECORD_AUDIO"/>
  <uses-permission android:name="android.permission.VIBRATE"/>
  <uses-feature android:glEsVersion="0x00020000"/>
  <application android:label="spike" android:hasCode="true" android:extractNativeLibs="true">
    <activity android:name="org.libsdl.app.SDLActivity" android:exported="true"
        android:configChanges="layoutDirection|locale|grammaticalGender|fontScale|fontWeightAdjustment|orientation|uiMode|screenLayout|screenSize|smallestScreenSize|keyboard|keyboardHidden|navigation">
      <intent-filter>
        <action android:name="android.intent.action.MAIN"/>
        <category android:name="android.intent.category.LAUNCHER"/>
      </intent-filter>
    </activity>
  </application>
</manifest>
""" % pkg


def step(label, cmd):
    t = time.time()
    r = subprocess.run(cmd, capture_output=True, text=True)
    print("apk: %s rc=%d %.2fs %s" % (label, r.returncode, time.time() - t, (r.stdout + r.stderr).strip()[-800:]))
    if r.returncode:
        sys.exit("apk: %s failed" % label)


t0 = time.time()
man = os.path.join(work, "AndroidManifest.xml")
with open(man, "w") as f:
    f.write(MANIFEST)
base = os.path.join(work, "base.apk")
step("aapt2 link", [os.path.join(bt, "aapt2"), "link", "-o", base, "--manifest", man, "-I", jar,
                    "--min-sdk-version", "21", "--target-sdk-version", "35", "--debug-mode"])
dex = os.path.join(work, "classes.dex")
step("llvm-objcopy --dump-section", [os.path.join(ndkbin, "llvm-objcopy"), "--dump-section",
                                      ".sdl3_android_dex=" + dex, libmain, os.path.join(work, "discard.so")])
got = open(dex, "rb").read()
want = open(os.path.join(here, "gen", "classes.dex"), "rb").read()
print("apk: classes.dex from the ELF section: %d bytes, sha256 %s, identical to gen/classes.dex: %s"
      % (len(got), hashlib.sha256(got).hexdigest()[:16], got == want))
unsigned = os.path.join(work, "unsigned.apk")
with zipfile.ZipFile(base) as zi, zipfile.ZipFile(unsigned, "w") as zo:
    for item in zi.infolist():
        zo.writestr(item, zi.read(item.filename))
    zo.writestr(zipfile.ZipInfo("classes.dex"), got, compress_type=zipfile.ZIP_DEFLATED)
    for src, name in [(libmain, "libmain.so")] + [(a, b) for a, b in extras]:
        zo.write(src, "lib/%s/%s" % (abi, name), compress_type=zipfile.ZIP_STORED)
aligned = os.path.join(work, "aligned.apk")
step("zipalign", [os.path.join(bt, "zipalign"), "-P", "16", "-f", "4", unsigned, aligned])
ks = os.path.expanduser("~/.android/spike-debug.keystore")
if not os.path.exists(ks):
    step("keytool", ["keytool", "-genkeypair", "-keystore", ks, "-storepass", "android", "-keypass", "android",
                     "-alias", "androiddebugkey", "-keyalg", "RSA", "-keysize", "2048", "-validity", "10000",
                     "-dname", "CN=Android Debug,O=Android,C=US"])
step("apksigner sign", [os.path.join(bt, "apksigner"), "sign", "--ks", ks, "--ks-pass", "pass:android",
                        "--key-pass", "pass:android", "--out", out, aligned])
step("apksigner verify", [os.path.join(bt, "apksigner"), "verify", "--print-certs", out])
step("zipalign -c", [os.path.join(bt, "zipalign"), "-c", "-P", "16", "-v", "4", out])
print("apk: %s %d bytes, total %.2fs" % (out, os.path.getsize(out), time.time() - t0))
with zipfile.ZipFile(out) as z:
    for i in z.infolist():
        print("apk:   %-40s %9d %s" % (i.filename, i.file_size, "stored" if i.compress_type == 0 else "deflated"))
