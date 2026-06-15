#!/bin/sh
# One-command Android release APK build.
#
# Assumes the toolchain lives under ~/android-toolchain (JDK + SDK + NDK) and
# cargo-apk is on PATH (~/.cargo/bin).  Override any of JAVA_HOME / ANDROID_HOME
# / ANDROID_NDK_ROOT in the environment to point elsewhere.  Extra args are
# forwarded to `cargo apk` (e.g. `./build-android.sh --quiet`).
set -e
cd "$(dirname "$0")"

export JAVA_HOME="${JAVA_HOME:-$HOME/android-toolchain/jdk}"
export ANDROID_HOME="${ANDROID_HOME:-$HOME/android-toolchain/sdk}"
export ANDROID_SDK_ROOT="$ANDROID_HOME"
# Pick the newest installed NDK unless one is already set.
if [ -z "$ANDROID_NDK_ROOT" ]; then
    ANDROID_NDK_ROOT="$(ls -d "$ANDROID_HOME"/ndk/* 2>/dev/null | sort -V | tail -1)"
fi
export ANDROID_NDK_ROOT
export ANDROID_NDK_HOME="$ANDROID_NDK_ROOT"
export PATH="$JAVA_HOME/bin:$HOME/.cargo/bin:$PATH"

echo "JDK : $JAVA_HOME"
echo "SDK : $ANDROID_HOME"
echo "NDK : $ANDROID_NDK_ROOT"
echo

cargo apk build --lib --release "$@"

APK="target/release/apk/zombiegame2.apk"
echo
echo "APK: $APK ($(du -h "$APK" 2>/dev/null | cut -f1))"
echo "Install on a connected device:"
echo "  $ANDROID_HOME/platform-tools/adb install -r $APK"
