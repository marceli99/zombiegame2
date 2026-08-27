#!/bin/sh
# Launch the Android emulator with a visible, GPU-accelerated window.
#
# On a hybrid NVIDIA + AMD/Intel machine under Wayland the emulator's default
# GPU path selects the NVIDIA Vulkan device, which falls back to SwiftShader
# software rendering (seconds per frame) — the caveat documented in MOBILE.md.
# The fix: force the integrated AMD (radv) / Intel (anv) Vulkan ICD so gfxstream
# renders on the iGPU, and route the Qt window through XWayland (the native
# Wayland Qt path is flaky here).  Result: real hardware Vulkan, smooth window.
#
# Usage:  ./run-emulator.sh [avd-name] [extra emulator args...]
# Default AVD is `zombie`.  Override ANDROID_HOME if the SDK lives elsewhere.
set -e
cd "$(dirname "$0")"

export ANDROID_HOME="${ANDROID_HOME:-$HOME/android-toolchain/sdk}"
export ANDROID_SDK_ROOT="$ANDROID_HOME"
export ANDROID_AVD_HOME="${ANDROID_AVD_HOME:-$HOME/.android/avd}"

AVD="zombie"
if [ $# -gt 0 ]; then AVD="$1"; shift; fi

# Prefer a non-NVIDIA Vulkan ICD (AMD radv / Intel anv) so the emulator's
# gfxstream backend renders on the iGPU instead of NVIDIA (→ SwiftShader on
# Wayland).  If none is found we leave the loader on its default.
for icd in /usr/share/vulkan/icd.d/radeon_icd.json \
           /usr/share/vulkan/icd.d/radeon_icd.x86_64.json \
           /usr/share/vulkan/icd.d/intel_icd.x86_64.json; do
    if [ -f "$icd" ]; then export VK_ICD_FILENAMES="$icd"; break; fi
done

# Qt window via XWayland; DRI_PRIME=1 steers the GL path to the second GPU too.
export QT_QPA_PLATFORM=xcb
export DISPLAY="${DISPLAY:-:0}"
export DRI_PRIME=1

echo "AVD : $AVD"
echo "ICD : ${VK_ICD_FILENAMES:-(default Vulkan loader)}"
echo "DISP: $DISPLAY (Qt: $QT_QPA_PLATFORM)"
echo

exec "$ANDROID_HOME/emulator/emulator" -avd "$AVD" -gpu host \
    -no-snapshot -no-audio -no-boot-anim -no-metrics "$@"
