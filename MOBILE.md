# Mobile (Android / iOS)

> **Legacy (2026-09):** the game moved to the browser (see README) and the
> native Android / iOS packaging below is no longer maintained. Phones now play
> the web build — the touch controls and mobile render profile described here
> are what it uses, so this document stays as background on those systems.

The game is structured so mobile is a first-class target: the whole game is a
library (`src/lib.rs`) that builds as a desktop binary, an Android `cdylib`, and
an iOS `staticlib` from the same code. Touch + gamepad controls and a
mobile-tuned render path are already wired in.

## What's already done for mobile

- **One codebase, all targets** — `[lib] crate-type = ["rlib", "cdylib",
  "staticlib"]`; `#[bevy_main]` provides the Android `android_main` and the iOS
  `main_rs` entry points; `src/main.rs` is just the desktop shim.
- **Controls** (`src/mobile.rs`) — on-screen twin-stick touch controls (left =
  move, right = aim + autofire) plus colour-coded action buttons (USE / reload /
  grenade / weapon slots), and full gamepad support (works on desktop too).
  Menus are driven by a gamepad→synthetic-key bridge and on-screen nav buttons,
  so no menu code had to be rewritten; main-menu entries are also directly
  tappable (bevy_ui `Interaction`). Two things make the synthetic key bridge
  actually reliable: the injectors run in `PreUpdate` after `InputSystem` (so
  the press is visible to every `Update` menu handler regardless of order), and
  every injected key is released in `Last` (`InjectedKeys` + `release_injected_keys`).
  Without the release a synthetic `press()` stuck in `pressed` forever, and a
  second `press()` of an already-pressed key does *not* re-arm `just_pressed` —
  so menus confirmed only on the very first tap. Touch UI activates on Android
  (or with `ZG_FORCE_TOUCH=1` on desktop to preview).
- **Aim assist** (`apply_aim_assist`) — the right stick snaps fire onto the
  nearest zombie inside a ~50° cone (`AIM_ASSIST_*`), so thumb aiming is
  forgiving without becoming full auto-target. Touch/pad only; mouse aim is
  untouched.
- **Mobile render profile** (`setup_camera`) — Android/iOS skip HDR, bloom and
  the LUT tonemapper (a lean SDR path that runs on tiled mobile GPUs and is
  easier on the render adapter). Desktop keeps the full glow look. Phones also
  zoom the camera in (`MOBILE_VIEW_H` vs desktop `FIXED_VIEW_H`) so the action
  reads bigger on a small screen.
- **Launcher icon** — `icon-src.svg` rasterised to `res/mipmap-*/ic_launcher.png`
  by `tools/gen-icon.sh`; wired through `resources` + `application.icon` in
  `Cargo.toml`.
- **Platform hygiene** — `wayland` feature is Linux-only; `Msaa::Off` on Android
  (some drivers crash with MSAA); landscape-locked; `INTERNET` permission for
  LAN; assets bundled into the app.

## Build Android

Prereqs (installed under `~/android-toolchain` in this setup): JDK 17, Android
SDK + platform-34 + build-tools + NDK, plus the Rust targets and `cargo-apk`:

```sh
rustup target add aarch64-linux-android x86_64-linux-android
cargo install cargo-apk
```

**One-time: create the signing keystore.** Release APKs are signed with
`release.keystore` in the repo root, which is git-ignored — on a fresh clone it
does not exist and `cargo apk build --release` (what `build-android.sh` runs)
**fails trying to open it**. Generate the throwaway self-signed key once
(`keytool` ships with the JDK, e.g. `$JAVA_HOME/bin/keytool`):

```sh
keytool -genkeypair -keystore release.keystore -alias zombie -keyalg RSA \
  -keysize 2048 -validity 10000 -storepass zombiegame -keypass zombiegame \
  -dname "CN=Zombiegame, O=Zombiegame, C=PL"
```

That key is fine for sideloading; for a Google Play upload replace it with a
properly managed upload key (see the `[package.metadata.android.signing.release]`
comment in `Cargo.toml`). Never commit a keystore.

Then just:

```sh
./build-android.sh
```

Output: `target/release/apk/zombiegame2.apk` (universal arm64 + x86_64).

Install on a connected device (USB debugging on):

```sh
~/android-toolchain/sdk/platform-tools/adb install -r target/release/apk/zombiegame2.apk
```

The APK ships both ABIs so it installs on real phones (arm64) **and** the
x86_64 emulator. To slim a real release, drop `x86_64-linux-android` from
`build_targets` in `Cargo.toml`.

## Build iOS

Requires macOS + Xcode (the iOS toolchain is macOS-only). Everything is staged
in `ios/` — see `ios/README.md`. Short version on a Mac:

```sh
cd ios && ./build-rust.sh && xcodegen generate && open ZombieGame.xcodeproj
```

You can test on your own iPhone with a **free** Apple ID (7-day signing) or in
the Simulator (no account). A paid Developer account is only needed for
TestFlight / App Store.

## Testing on a Linux PC — emulator GPU caveat (+ fix)

On an **NVIDIA + Wayland** desktop the emulator's default GPU path picks the
NVIDIA Vulkan device and falls back to **SwiftShader software rendering**
(seconds per frame — Bevy even logs error `B0006`). This is an emulation/driver
limitation, not the game: headless runs still render the menu and gameplay
correctly, just slowly.

**Fix on a hybrid box (NVIDIA discrete + AMD/Intel iGPU):** force the integrated
GPU's Vulkan ICD and route the emulator's Qt window through XWayland. The
helper script does this for you:

```sh
./run-emulator.sh          # boots the `zombie` AVD, windowed, on the iGPU
```

Under the hood it exports `VK_ICD_FILENAMES=<radv/anv ICD>` (hides NVIDIA so
gfxstream selects the iGPU), `QT_QPA_PLATFORM=xcb` and runs `emulator -gpu host`.
Verified working: Bevy then reports the AMD `RADV` adapter (hardware Vulkan)
instead of `SwiftShader`, and the menu + live gameplay render in a real window.

Then install + launch:

```sh
~/android-toolchain/sdk/platform-tools/adb install -r target/release/apk/zombiegame2.apk
~/android-toolchain/sdk/platform-tools/adb shell monkey -p org.zombiegame2.game -c android.intent.category.LAUNCHER 1
```

For the most faithful test still prefer a **real phone** (full GPU speed, real
touch). A pure-NVIDIA machine with no second GPU stays stuck on SwiftShader —
use a device there.

## Controls reference

| Action | Touch | Gamepad |
|--------|-------|---------|
| Move | left-half stick (floating) | left stick |
| Aim + fire | right-half stick (autofire + aim assist) | right stick / RT |
| Interact (use / unlock / pickup) | `USE` button | South (A) |
| Reload | `R` button | West (X) |
| Grenade | `G` button | North (Y) |
| Weapon slot 1/2/3 | `1`/`2`/`3` buttons | D-pad ←/↑/→ |
| Menu navigate | on-screen ↑↓←→ | D-pad / left stick |
| Confirm / Back | `OK` / `X` | A / B |
