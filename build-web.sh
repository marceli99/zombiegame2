#!/bin/sh
# One-command browser build.  Produces a static site in `web/`:
#
#   web/index.html            shell page (checked in)
#   web/pkg/zombiegame2.js    wasm-bindgen glue
#   web/pkg/zombiegame2_bg.wasm
#   web/assets/               copy of ./assets (fonts) fetched by bevy_asset
#
# Serve `web/` from any static HTTP server (`./build-web.sh --serve` starts
# one on :8000).  Browsers refuse to run wasm from file://.
#
# Requirements (one-off):
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version <version pinned in Cargo.lock>
#   wasm-opt from binaryen on PATH (optional — skipped with a warning if absent)
#
# Flags:
#   --fast    skip wasm-opt (seconds instead of a minute; ~25 % bigger .wasm)
#   --serve   after building, run the signaling server (serves web/ + /ws)
#             on http://0.0.0.0:8000
set -e
cd "$(dirname "$0")"
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"

FAST=0; SERVE=0
for a in "$@"; do
    case "$a" in
        --fast)  FAST=1 ;;
        --serve) SERVE=1 ;;
        *) echo "unknown flag: $a" >&2; exit 2 ;;
    esac
done

WANT_BINDGEN="$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | sed -n 's/^version = "\(.*\)"/\1/p')"
HAVE_BINDGEN="$(wasm-bindgen --version 2>/dev/null | awk '{print $2}')"
if [ "$WANT_BINDGEN" != "$HAVE_BINDGEN" ]; then
    echo "wasm-bindgen-cli $WANT_BINDGEN required (have: ${HAVE_BINDGEN:-none})." >&2
    echo "  cargo install wasm-bindgen-cli --version $WANT_BINDGEN --locked" >&2
    exit 1
fi

cargo build --release --target wasm32-unknown-unknown --bin zombiegame2

rm -rf web/pkg
wasm-bindgen --target web --no-typescript --out-dir web/pkg --out-name zombiegame2 \
    target/wasm32-unknown-unknown/release/zombiegame2.wasm

WASM=web/pkg/zombiegame2_bg.wasm
if [ "$FAST" = 0 ]; then
    if command -v wasm-opt >/dev/null; then
        # -O3 over -Oz: the game is CPU-bound on a single thread, so we keep
        # the speed-tuned codegen and only take the free size wins.
        # Rust ≥1.82 emits these post-MVP features by default; wasm-opt must be
        # told they're allowed or it rejects the module.
        wasm-opt -O3 \
            --enable-bulk-memory --enable-reference-types --enable-sign-ext \
            --enable-mutable-globals --enable-nontrapping-float-to-int \
            "$WASM" -o "$WASM"
    else
        echo "wasm-opt not found — skipping size optimisation" >&2
    fi
fi

rm -rf web/assets
cp -r assets web/assets

echo
echo "web/pkg/zombiegame2_bg.wasm: $(du -h "$WASM" | cut -f1) (gzip: $(gzip -9 -c "$WASM" | wc -c | numfmt --to=iec))"

if [ "$SERVE" = 1 ]; then
    exec cargo run -p signaling -- --listen 0.0.0.0:8000 --web web
fi
