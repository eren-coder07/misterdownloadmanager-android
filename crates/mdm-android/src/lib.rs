//! Mister Download Manager on Android.
//!
//! Runs the same app as the desktop program ([`mdm_app::app_android`]) through
//! eframe on a `NativeActivity` (GLES via eframe's glow renderer; the
//! storytold/anefzaoui eframe fork carries the `android_app` option). The
//! Gradle project in `android/` drives `cargo ndk`, which builds this crate
//! into `src/main/jniLibs` as `libmdm_android.so`; see `android/README.md`.
//!
//! A launch that never opens its window must say why where the user can read
//! it, on a device with no computer attached. Everything the app logs (the
//! `log` crate) goes to logcat under the tag "MDM" (`adb logcat -s MDM`) and
//! to `mdm-debug.log` in the app's storage — the external folder
//! (`Android/data/io.github.anefzaoui.mdm/files/`) a file manager can open,
//! beside the app's private one. A panic, a start that ends in an error, or a
//! native signal (SIGSEGV and friends) is written there too and shown as a
//! system notification, so the first line of the failure is on screen.

#[cfg(target_os = "android")]
mod android {
    use std::fmt::Write as _;
    use std::fs::File;
    use std::io::Write as _;
    use std::panic::{self, AssertUnwindSafe};
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::time::Instant;

    use winit::platform::android::activity::AndroidApp;

    /// The files every log line is mirrored to: the external files folder a
    /// file manager reaches (none there where external storage is not mounted)
    /// and the app's own, always-writable folder.
    static FILES: OnceLock<Vec<Mutex<File>>> = OnceLock::new();
    /// The start, for each line's `+ms`.
    static START: OnceLock<Instant> = OnceLock::new();
    /// The mirrored file opened for the signal handler (a raw fd, since the
    /// handler may run nothing that locks or allocates). -1: none.
    static SIGNAL_FD: AtomicI32 = AtomicI32::new(-1);

    /// The mirrored log files, opened once: one beside the app's public folder
    /// and one in its private one. Each launch's log is its own: the files are
    /// truncated here, so a report's tail (the launcher's gate reads them) is
    /// the launch that crashed, never one from before.
    fn open_files(app: &AndroidApp) -> Vec<Mutex<File>> {
        let mut files = Vec::new();
        for dir in [app.external_data_path(), app.internal_data_path()] {
            let Some(dir) = dir else { continue };
            match File::create(dir.join("mdm-debug.log")) {
                Ok(f) => files.push(Mutex::new(f)),
                Err(e) => log::warn!("debug log in {}: {e}", dir.display()),
            }
        }
        files
    }

    /// One line into the mirrored files. Failures are ignored: a full disk or
    /// a locked file must not take the logger down.
    fn mirror(line: &str) {
        if let Some(files) = FILES.get() {
            for f in files {
                if let Ok(mut f) = f.lock() {
                    let _ = f.write_all(line.as_bytes());
                }
            }
        }
    }

    /// One line into the mirrored files, stamped with the launch's `+ms`.
    fn debug_line(text: &str) {
        let ms = START.get().map(|s| s.elapsed().as_millis()).unwrap_or(0);
        mirror(&format!("[+{ms:08} ms] {text}\n"));
    }

    /// A panic's message, from its payload.
    fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
        if let Some(s) = payload.downcast_ref::<&str>() {
            (*s).to_owned()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "a panic with no message".to_owned()
        }
    }

    /// The mirrored file for the signal handler, opened the raw way (a signal
    /// handler may call `write` and nothing that locks): the app's own folder,
    /// which is always writable.
    fn open_signal_fd(internal: Option<&std::path::Path>) {
        let Some(dir) = internal else { return };
        let path = dir.join("mdm-debug.log");
        let Some(path) = path.to_str() else { return };
        let Ok(path) = std::ffi::CString::new(path) else { return };
        // SAFETY: `open` takes the path by pointer and nothing else holds it.
        let fd = unsafe {
            libc::open(path.as_ptr(), libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND, 0o600)
        };
        if fd >= 0 {
            SIGNAL_FD.store(fd, Ordering::Relaxed);
        }
    }

    /// Which native signal names what: the name, for the crash line.
    fn signal_name(sig: libc::c_int) -> &'static str {
        match sig {
            libc::SIGSEGV => "SIGSEGV (bad memory access)",
            libc::SIGBUS => "SIGBUS (bad memory access)",
            libc::SIGILL => "SIGILL (bad instruction)",
            libc::SIGFPE => "SIGFPE (arithmetic)",
            libc::SIGABRT => "SIGABRT (abort)",
            libc::SIGTRAP => "SIGTRAP",
            _ => "a signal",
        }
    }

    /// A native crash: one line into the mirrored file, then the system takes
    /// over as with any crashed app. The handler may only call the signal-safe
    /// few, so no locks, no allocation, no formatting: a ready line written by
    /// hand into a fixed buffer, the fault address included (where a bad memory
    /// access points — a null deref says one thing, a wild pointer another).
    #[allow(unsafe_code)]
    extern "C" fn on_native_signal(
        sig: libc::c_int,
        info: *mut libc::siginfo_t,
        _context: *mut libc::c_void,
    ) {
        let fd = SIGNAL_FD.load(Ordering::Relaxed);
        if fd >= 0 {
            let mut line = [0u8; 160];
            let mut n = 0;
            push_bytes(&mut line, &mut n, b"[+");
            let ms = START.get().map(|s| s.elapsed().as_millis() as u64).unwrap_or(0);
            push_num(&mut line, &mut n, ms);
            push_bytes(&mut line, &mut n, b" ms] NATIVE CRASH: ");
            push_bytes(&mut line, &mut n, signal_name(sig).as_bytes());
            // The fault address, where the signal carries one: si_code > 0
            // means the kernel generated it for that address; a raise or an
            // abort's si_addr is whoever's, and says nothing.
            if !info.is_null() {
                // SAFETY: the kernel hands the siginfo of this very signal.
                let si = unsafe { &*info };
                if si.si_code > 0 {
                    // SAFETY: `si_addr` reads the union for signal codes that
                    // carry an address (SEGV_MAPERR and friends).
                    let addr = unsafe { si.si_addr() } as usize;
                    push_bytes(&mut line, &mut n, b" at ");
                    push_hex(&mut line, &mut n, addr as u64);
                }
            }
            push_bytes(&mut line, &mut n, b"\n");
            // SAFETY: `write` takes a valid fd and a valid buffer; the handler
            // runs on the thread the signal hit, with nothing else owning fd.
            unsafe {
                let mut left = &line[..n];
                while !left.is_empty() {
                    let wrote = libc::write(fd, left.as_ptr().cast(), left.len());
                    if wrote <= 0 {
                        break;
                    }
                    left = &left[wrote as usize..];
                }
            }
        }
        // The system must see a crashed app (and Android's own report): the
        // default disposition takes over from here.
        // SAFETY: `signal` and `raise` take a signal number each.
        unsafe {
            libc::signal(sig, libc::SIG_DFL);
            libc::raise(sig);
        }
    }

    /// Bytes into a fixed buffer, at most what is left in it.
    fn push_bytes(buf: &mut [u8], at: &mut usize, bytes: &[u8]) {
        let room = buf.len().saturating_sub(*at);
        let n = bytes.len().min(room);
        buf[*at..*at + n].copy_from_slice(&bytes[..n]);
        *at += n;
    }

    /// A decimal number, hand-rolled (the handler may not format).
    fn push_num(buf: &mut [u8], at: &mut usize, mut v: u64) {
        let mut tmp = [0u8; 20];
        let mut i = tmp.len();
        if v == 0 {
            i -= 1;
            tmp[i] = b'0';
        } else {
            while v > 0 {
                i -= 1;
                tmp[i] = b'0' + (v % 10) as u8;
                v /= 10;
            }
        }
        push_bytes(buf, at, &tmp[i..]);
    }

    /// A hexadecimal number, hand-rolled, no `0x` (the caller writes it).
    fn push_hex(buf: &mut [u8], at: &mut usize, mut v: u64) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut tmp = [0u8; 16];
        let mut i = tmp.len();
        if v == 0 {
            i -= 1;
            tmp[i] = b'0';
        } else {
            while v > 0 {
                i -= 1;
                tmp[i] = HEX[(v & 0xf) as usize];
                v >>= 4;
            }
        }
        push_bytes(buf, at, &tmp[i..]);
    }

    /// The signals worth a line in the file, handled.
    #[allow(unsafe_code)]
    fn install_signal_handlers() {
        // SAFETY: `sigaction` takes a valid action; the handler only writes a
        // ready string and re-raises, and replaces the default's report with a
        // more useful one (a line saying what happened where).
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = on_native_signal as *const () as usize;
            action.sa_flags = libc::SA_SIGINFO | libc::SA_RESTART;
            libc::sigemptyset(&mut action.sa_mask);
            for sig in [libc::SIGSEGV, libc::SIGBUS, libc::SIGILL, libc::SIGFPE, libc::SIGABRT, libc::SIGTRAP] {
                libc::sigaction(sig, &action, std::ptr::null_mut());
            }
        }
    }

    /// `title` and `text` as a system notification, so a launch that cannot
    /// open its window says why on screen. JNI from the activity's thread,
    /// where there is no Looper: notifications need no one, dialogs would.
    #[allow(unsafe_code)]
    fn notify(app: &AndroidApp, title: &str, text: &str) {
        let result = notify_or_err(app, title, text);
        if let Err(e) = result {
            log::warn!("the failure notification didn't show: {e:?}");
            debug_line(&format!("notification didn't show: {e:?}"));
        }
    }

    /// The notification, through JNI: a channel, a builder, a post. The first
    /// error of the way back is the answer; a pending Java exception comes
    /// back as one (the attach call hands it over).
    #[allow(unsafe_code)]
    fn notify_or_err(app: &AndroidApp, title: &str, text: &str) -> Result<(), jni::errors::Error> {
        use jni::jni_sig;
        use jni::jni_str;
        use jni::objects::JObject;
        use jni::objects::JValue;

        // SAFETY: the pointer is the system's JavaVM, handed out for exactly this.
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) };
        vm.attach_current_thread(|env| {
            // SAFETY: the reference is the activity's, valid while the app
            // runs, and borrowed here (as_cast_raw never deletes it).
            let activity_raw = app.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe { env.as_cast_raw::<JObject>(&activity_raw)? };

            let service = env.new_string("notification")?;
            let manager = env
                .call_method(
                    &activity,
                    jni_str!("getSystemService"),
                    jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
                    &[JValue::Object(&service)],
                )?
                .l()?;

            let channel_class = env.find_class(jni_str!("android/app/NotificationChannel"))?;
            let id = env.new_string("mdm.debug")?;
            let name = env.new_string("MDM")?;
            let channel = env.new_object(
                &channel_class,
                jni_sig!("(Ljava/lang/String;Ljava/lang/CharSequence;I)V"),
                &[JValue::Object(&id), JValue::Object(&name), JValue::Int(3)],
            )?;
            env.call_method(
                &manager,
                jni_str!("createNotificationChannel"),
                jni_sig!("(Landroid/app/NotificationChannel;)V"),
                &[JValue::Object(&channel)],
            )?;

            let builder_class = env.find_class(jni_str!("android/app/Notification$Builder"))?;
            let id = env.new_string("mdm.debug")?;
            let builder = env.new_object(
                &builder_class,
                jni_sig!("(Landroid/content/Context;Ljava/lang/String;)V"),
                &[JValue::Object(&activity), JValue::Object(&id)],
            )?;
            let title = env.new_string(title)?;
            let text = env.new_string(text)?;
            env.call_method(
                &builder,
                jni_str!("setContentTitle"),
                jni_sig!("(Ljava/lang/CharSequence;)Landroid/app/Notification$Builder;"),
                &[JValue::Object(&title)],
            )?;
            env.call_method(
                &builder,
                jni_str!("setContentText"),
                jni_sig!("(Ljava/lang/CharSequence;)Landroid/app/Notification$Builder;"),
                &[JValue::Object(&text)],
            )?;
            // A long reason fits only a big-text style.
            let style_class = env.find_class(jni_str!("android/app/Notification$BigTextStyle"))?;
            let style = env.new_object(&style_class, jni_sig!("()V"), &[])?;
            let style = env
                .call_method(
                    &style,
                    jni_str!("bigText"),
                    jni_sig!("(Ljava/lang/CharSequence;)Landroid/app/Notification$BigTextStyle;"),
                    &[JValue::Object(&text)],
                )?
                .l()?;
            env.call_method(
                &builder,
                jni_str!("setStyle"),
                jni_sig!("(Landroid/app/Notification$Style;)Landroid/app/Notification$Builder;"),
                &[JValue::Object(&style)],
            )?;
            // The framework's alert icon: no icon ships with a NativeActivity app.
            let drawables = env.find_class(jni_str!("android/R$drawable"))?;
            let icon = env
                .get_static_field(&drawables, jni_str!("ic_dialog_alert"), jni_sig!("I"))?
                .i()?;
            env.call_method(
                &builder,
                jni_str!("setSmallIcon"),
                jni_sig!("(I)Landroid/app/Notification$Builder;"),
                &[JValue::Int(icon)],
            )?;
            let notification = env
                .call_method(&builder, jni_str!("build"), jni_sig!("()Landroid/app/Notification;"), &[])?
                .l()?;
            env.call_method(
                &manager,
                jni_str!("notify"),
                jni_sig!("(ILandroid/app/Notification;)V"),
                &[JValue::Int(0x4d44_4d44), JValue::Object(&notification)],
            )?;
            Ok(())
        })
    }

    /// The run-state marker the launcher's gate reads on the next launch:
    /// "starting" while the app runs, "exited" or "error" once it ends by
    /// itself. A crash leaves "starting" behind, and the gate reports it.
    fn set_run_state(internal: Option<&std::path::Path>, state: &str) {
        if let Some(dir) = internal {
            let _ = std::fs::write(dir.join("run-state"), state);
        }
    }

    pub fn run(app: AndroidApp) {
        START.set(Instant::now()).ok();
        let internal = app.internal_data_path();
        set_run_state(internal.as_deref(), "starting");

        // Every log line goes to logcat (tag MDM) and to the mirrored files at
        // once: logcat for a computer, the file for the launcher's gate and a
        // file manager. The files truncate here: each launch's log is its own.
        FILES.set(open_files(&app)).ok();
        // The system's version (a getprop read that would panic on a device
        // without it — that device has bigger problems, and the start goes on).
        let sdk = panic::catch_unwind(AssertUnwindSafe(AndroidApp::sdk_version)).unwrap_or(0);
        debug_line(&format!(
            "MDM {} starting ({}, {}, Android SDK {})",
            mdm_app::help::VERSION,
            std::env::consts::ARCH,
            if cfg!(target_pointer_width = "64") { "64-bit" } else { "32-bit" },
            sdk
        ));
        install_signal_handlers();

        // The app's own, writable folder: MDM's data dir (mdm_ipc::data_dir
        // honors MDM_DATA_DIR) and every HOME/XDG path other components follow.
        if let Some(dir) = &internal {
            open_signal_fd(Some(dir));
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
        } else {
            debug_line("no internal data path: the app's storage is not there");
        }

        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Debug)
                .with_tag("MDM")
                .format(|buf, record| {
                    let ms = START.get().map(|s| s.elapsed().as_millis()).unwrap_or(0);
                    let level = record.level();
                    let target = record.target();
                    write!(buf, "[+{ms:08} ms] {level:<5} [{target}] {}", record.args()).ok();
                    mirror(&format!("[+{ms:08} ms] {level:<5} [{target}] {}\n", record.args()));
                    Ok(())
                }),
        );

        // A panic anywhere: into the mirrored files (the app's own hook, set
        // once `app_android` runs, takes this one and adds its crash.log).
        let default_hook = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let thread = std::thread::current();
            let name = thread.name().unwrap_or("<unnamed>").to_owned();
            let message = panic_message(info.payload());
            debug_line(&format!("PANIC on thread {name}: {message}"));
            debug_line(&format!("backtrace:\n{}", std::backtrace::Backtrace::force_capture()));
            default_hook(info);
        }));

        // The app, contained: a panic during the start (before or inside the
        // window's loop) lands here instead of ending the process unwitnessed.
        let notifier = app.clone();
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| mdm_app::app_android(app)));
        match outcome {
            Ok(Ok(())) => set_run_state(internal.as_deref(), "exited"),
            Ok(Err(e)) => {
                set_run_state(internal.as_deref(), "error");
                let text = format!("Mister Download Manager could not start: {e:?}");
                debug_line(&text);
                notify(&notifier, "MDM could not start", &text);
            }
            Err(payload) => {
                // The marker stays "starting": the next launch reports the crash.
                let message = panic_message(payload.as_ref());
                let text =
                    format!("Mister Download Manager ended unexpectedly while starting: {message}");
                debug_line(&text);
                notify(&notifier, "MDM stopped unexpectedly", &text);
            }
        }
        debug_line("the activity ended; exiting");

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
