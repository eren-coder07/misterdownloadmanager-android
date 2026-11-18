# Mister Download Manager on Android

The same MDM as on the desktop, as an APK: a `NativeActivity` around
`libmdm_android.so`, which the `mdm-android` crate builds. This is a standalone
port repository of [anefzaoui/misterdownloadmanager](https://github.com/anefzaoui/misterdownloadmanager)
(upstream's desktop app untouched, GPL-3.0-or-later carried over) with the
Android port added on top.

## Build

Needs the Android SDK (platform 36, build-tools 36) and NDK 29.0.14206865, plus
Rust targets `aarch64-linux-android` and `armv7-linux-androideabi` (and/or
`x86_64-linux-android`) and `cargo-ndk`:

```sh
cargo install cargo-ndk
cd android
./gradlew assembleRelease   # universal APK (arm64-v8a + armeabi-v7a)
# or, per ABI:
./gradlew assembleRelease -Pmdm.abis=arm64-v8a
./gradlew assembleRelease -Pmdm.abis=armeabi-v7a
```

The APK lands in `app/build/outputs/apk/release/MDM-<version>-release.apk`.

* `-Pmdm.abis` — comma-separated ABIs (default: `arm64-v8a,armeabi-v7a`).
* `-Pmdm.portRevision` — a port-only release letter (`1` → `0.5.2a`).
* Release signing comes from the environment: `MDM_RELEASE_KEYSTORE`,
  `MDM_RELEASE_STORE_PASSWORD`, `MDM_RELEASE_KEY_ALIAS` and
  `MDM_RELEASE_KEY_PASSWORD`. Without them the build falls back to the debug key.
* The build passes `--no-default-features`: `browser-connection` (BoringSSL's C
  sources) stays out, as upstream's own cross-compile check does.

## How it works

`android/app/src/main/AndroidManifest.xml` declares a plain
`android.app.NativeActivity` (no Java code at all) loading
`libmdm_android.so`. That library is built from `crates/mdm-android/src/lib.rs`,
whose `android_main`:

* points `MDM_DATA_DIR` (mdm_ipc's own override), `HOME` and the XDG variables
  at the app's internal storage, so the download list, settings, torrent state
  and logs all land somewhere writable;
* initializes `android_logger` (logcat tag `MDM`);
* calls [`mdm_app::app_android`](../crates/app/src/lib.rs) — a variant of the
  desktop's `app()` without the desktop-only pieces (CLI dispatch,
  single-instance hand-over, update machinery, tray-start modes), keeping the
  remote control (#141) so a paired device drives this app over the network;
* starts eframe with `NativeOptions::android_app` set (the storytold/anefzaoui
  eframe fork carries it, the same way upstream's is `#[cfg(target_os =
  "android")]`) on the glow (GLES) renderer;
* ends the process when the activity ends (winit's event loop cannot restart
  in the same process).

The desktop `mdm-app` crate became a lib + bin: `src/lib.rs` holds the modules
and both entry points (`main_entry` for the desktop binary, `app_android` for
Android); `src/main.rs` is three lines calling the former, so the desktop
program is unchanged.

## Notes

* Downloads land in the app's own storage (`Android/data/io.github.anefzaoui.mdm/files`),
  which file managers can reach.
* The browser extension and the browser-like TLS connection (BoringSSL) are
  desktop-only; links reach MDM by pasting or through the remote control.
* The two folder pickers (video extractor, RSS rules) answer "none" on Android
  (no desktop file dialogs in a NativeActivity); the rest of the app is
  unaffected.
* The CI release workflow is
  [`.github/workflows/android-release.yml`](../.github/workflows/android-release.yml):
  it builds the universal APK plus per-ABI APKs, checksums them and publishes a
  GitHub release.
