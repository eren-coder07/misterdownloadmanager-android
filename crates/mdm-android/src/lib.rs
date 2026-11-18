//! Mister Download Manager on Android.
//!
//! Runs the same app as the desktop program ([`mdm_app::app_android`]) through
//! eframe on a `NativeActivity` (GLES via eframe's glow renderer; the
//! storytold/anefzaoui eframe fork carries the `android_app` option). The
//! Gradle project in `android/` drives `cargo ndk`, which builds this crate
//! into `src/main/jniLibs` as `libmdm_android.so`; see `android/README.md`.

#[cfg(target_os = "android")]
mod android {
    use winit::platform::android::activity::AndroidApp;

    pub fn run(app: AndroidApp) {
        // The app's own, writable folder: MDM's data dir (mdm_ipc::data_dir
        // honors MDM_DATA_DIR) and every HOME/XDG path other components follow.
        if let Some(dir) = app.internal_data_path() {
            // SAFETY: `set_var` is unsafe in edition 2024 because other threads
            // may read the environment while it changes — this runs on the
            // activity's own thread before anything else of the app's starts,
            // and nothing reads these variables until `app_android` runs.
            unsafe {
                std::env::set_var("MDM_DATA_DIR", &dir);
                std::env::set_var("HOME", &dir);
                std::env::set_var("XDG_DATA_HOME", dir.join("data"));
                std::env::set_var("XDG_CONFIG_HOME", dir.join("config"));
                std::env::set_var("XDG_CACHE_HOME", dir.join("cache"));
            }
        }
        // Logs reach logcat under the tag "MDM" (adb logcat -s MDM).
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Info)
                .with_tag("MDM"),
        );

        if let Err(e) = mdm_app::app_android(app) {
            log::error!("Mister Download Manager could not start: {e:?}");
        }
        // The system destroyed the activity, and the app has shut down. winit's
        // event loop can only be made once per process, so a new activity in
        // this process could never start: end the process, and the next launch
        // starts a fresh one.
        // SAFETY: `_exit` takes no pointers and never returns; it only ends the
        // process, without running exit-time destructors while the system's own
        // threads still use what they free.
        unsafe { libc::_exit(0) }
    }
}

/// The activity's entry point: android-activity's glue calls it by this name on
/// its own thread.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    android::run(app);
}
