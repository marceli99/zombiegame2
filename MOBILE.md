# Mobile (Android / iOS)

The game is structured so mobile is a first-class target: the whole game is a
library (`src/lib.rs`) that builds as a desktop binary, an Android `cdylib`, and
an iOS `staticlib` from the same code. Touch + gamepad controls and a
mobile-tuned render path are already wired in.

## What's already done for mobile

- **One codebase, all targets** — `[lib] crate-type = ["rlib", "cdylib",
  "staticlib"]`; `#[bevy_main]` provides the Android `android_main` and the iOS
  `main_rs` entry points; `src/main.rs` is just the desktop shim.
- **Controls** (`src/mobile.rs`) — on-screen twin-stick touch controls (left =
  move, right = aim + autofire) plus action buttons (reload / grenade /
  interact / weapon slots), and full gamepad support (works on desktop too).
  Menus are driven by a gamepad→synthetic-key bridge and on-screen nav buttons,
  so no menu code had to be rewritten. Touch UI activates on Android (or with
  `ZG_FORCE_TOUCH=1` on desktop to preview the layout).
- **Mobile render profile** (`setup_camera`) — Android/iOS skip HDR, bloom and
  the LUT tonemapper (a lean SDR path that runs on tiled mobile GPUs and is
  easier on the render adapter). Desktop keeps the full glow look.
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

## Testing on a Linux PC — caveat

Running the APK in an **Android emulator or Waydroid on an NVIDIA + Wayland**
desktop is unreliable: the emulator's host-GL window path falls back to software
(seconds per frame), and Waydroid/NVIDIA often can't provide a Vulkan adapter.
This is an emulation/driver limitation, not the game — headless emulator runs
render the menu and gameplay correctly.

For actual play-testing, use a **real phone** (full GPU speed, real touch). If
you must test on the PC, an x86_64 emulator on an Intel/AMD GPU, or a non-NVIDIA
Waydroid setup, tends to work.

## Controls reference

| Action | Touch | Gamepad |
|--------|-------|---------|
| Move | left-half stick (floating) | left stick |
| Aim + fire | right-half stick (autofire) | right stick / RT |
| Reload | `R` button | West (X) |
| Grenade | `N` button | North (Y) |
| Interact | `E` button | South (A) |
| Weapon slot 1/2/3 | `1`/`2`/`3` buttons | D-pad ←/↑/→ |
| Menu navigate | on-screen ↑↓←→ | D-pad / left stick |
| Confirm / Back | `OK` / `X` | A / B |
