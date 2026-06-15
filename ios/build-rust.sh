#!/bin/sh
# Compiles the Rust game as a static library for the iOS target Xcode is
# currently building, then drops it at ios/lib/libzombiegame2.a where the
# Xcode project links it.  Driven as a pre-build phase, but also runnable by
# hand (defaults to a release device build).
set -e

# Repo root, regardless of where Xcode invokes us from.
cd "$(dirname "$0")/.."

# Xcode runs build phases with a minimal PATH; make sure cargo/rustup resolve.
export PATH="$HOME/.cargo/bin:$HOME/.rustup/bin:$PATH"

: "${PLATFORM_NAME:=iphoneos}"
: "${CONFIGURATION:=Release}"

if [ "$PLATFORM_NAME" = "iphonesimulator" ]; then
    case "${ARCHS:-arm64}" in
        *x86_64*) TRIPLE="x86_64-apple-ios" ;;   # Intel Mac simulator
        *)        TRIPLE="aarch64-apple-ios-sim" ;; # Apple Silicon simulator
    esac
    FEATURES="--features ios_sim"
else
    TRIPLE="aarch64-apple-ios"                    # physical device
    FEATURES=""
fi

if [ "$CONFIGURATION" = "Debug" ]; then
    PROFILE_DIR="debug"; PROFILE_FLAG=""
else
    PROFILE_DIR="release"; PROFILE_FLAG="--release"
fi

rustup target add "$TRIPLE" >/dev/null 2>&1 || true

echo "cargo build --lib $PROFILE_FLAG --target $TRIPLE $FEATURES"
cargo build --lib $PROFILE_FLAG --target "$TRIPLE" $FEATURES

mkdir -p ios/lib
cp "target/$TRIPLE/$PROFILE_DIR/libzombiegame2.a" "ios/lib/libzombiegame2.a"
echo "Staged ios/lib/libzombiegame2.a  ($TRIPLE / $CONFIGURATION)"
