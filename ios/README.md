# iOS build

The whole game compiles into a Rust **static library** (`crate-type =
["staticlib", ...]`) with a C entry point (`#[bevy_main]` → `main_rs`), and this
folder wraps it in a minimal iOS app. Same gameplay code as desktop/Android —
the touch + gamepad controls already work here.

> ⚠️ Requires **macOS + Xcode**. The iOS toolchain (clang/codesign/Simulator)
> is macOS-only — it cannot run on Linux/Windows. You do **not** need a paid
> Apple Developer account to test (see below); you only need a Mac (a cloud Mac
> works too).

## Testing without a paid Developer account ($99/yr)

- **iOS Simulator** — no account at all. Pick a simulator in Xcode and Run.
- **Your own iPhone (free Apple ID)** — sign in Xcode with a free Apple ID
  ("Personal Team"). Xcode auto-creates a free provisioning profile. Caveats:
  the build **expires after 7 days** (just re-run from Xcode to refresh), and
  you can only have a few sideloaded apps at once.
- **TestFlight / distributing to others** does require the paid program.

## One-time setup (on the Mac)

```sh
# Xcode from the App Store, then its command-line tools:
xcode-select --install

# Rust + iOS targets:
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
# (Intel Macs also: rustup target add x86_64-apple-ios)

# XcodeGen, to turn project.yml into an .xcodeproj:
brew install xcodegen
```

## Build & run

```sh
cd ios
./build-rust.sh          # compiles the static lib once (slow first time)
xcodegen generate        # creates ZombieGame.xcodeproj
open ZombieGame.xcodeproj
```

In Xcode:

1. Select the **ZombieGame** scheme.
2. Choose a target:
   - **Simulator** (e.g. "iPhone 15") — Run (⌘R), no signing needed.
   - **Your iPhone** (plugged in) — open *Signing & Capabilities*, set **Team**
     to your Apple ID (Personal Team). If the bundle id
     `org.zombiegame2.game` is taken, change it to something unique. Then Run.
3. First run on a device: on the phone, *Settings → General → VPN & Device
   Management → trust your developer certificate*.

The pre-build script `build-rust.sh` re-compiles the Rust lib automatically on
every Xcode build for whichever target (device vs simulator, debug vs release)
is selected, so you normally don't run it by hand after the first time.

## No Mac at all?

Compile on a cloud Mac (MacinCloud / MacStadium / AWS EC2 mac) or CI (GitHub
Actions `macos` runner, Codemagic). Run the Simulator there, or build an `.ipa`
and sideload it onto your iPhone from any OS with **AltStore**/**Sideloadly** +
a free Apple ID (same 7-day expiry).

## Troubleshooting

- **`library not found for -lzombiegame2`** — run `./build-rust.sh` once, then
  build again (the `.a` must exist before the link step on a fresh checkout).
- **Build script "Operation not permitted"** — ensure
  `ENABLE_USER_SCRIPT_SANDBOXING = NO` survived (it's set in `project.yml`).
- **cargo not found in the build phase** — Xcode uses a minimal PATH;
  `build-rust.sh` already adds `~/.cargo/bin`, but adjust if rustup lives
  elsewhere.
