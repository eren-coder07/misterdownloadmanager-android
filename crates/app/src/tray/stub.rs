//! The tray's inert Android stand-in: a NativeActivity's app has no notification
//! area, so there is no tray here. The types keep the desktop's shapes
//! ([`TrayCmd`], [`Icon`], [`OnClose`]), `available()` is false, `Tray::new`
//! never makes anything, and every setter is a no-op — the app compiles
//! unchanged and behaves as on a desktop machine without any tray host.

/// The tray icon as the window holds it: the shell's last answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    /// The shell holds the icon, or gives it a place: it can bring a hidden window back.
    There,
    /// No answer, or none recent: nothing is concluded either way.
    Unknown,
    /// The shell said it has no icon of MDM's, or there is no taskbar.
    Gone,
}

/// What the tray asks the app to do. Never sent here (there is no tray), but the
/// app's handlers compile against it as on the desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCmd {
    Show,
    Toggle,
    AddUrl,
    Batch,
    BatchClipboard,
    LimiterOn,
    LimiterOff,
    Scheduler,
    StartQueue,
    StopQueue,
    Options,
    Exit,
    ShowDownload(u64),
}

/// What closing the main window does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnClose {
    Hide,
    Minimize,
    Quit,
}

/// Whether a tray shows MDM's icon at all: Android has none.
pub fn available() -> bool {
    false
}

/// What closing the main window does — the same decision as the desktop's: with
/// no tray here, `tray` being false means closing quits, unless the dock answer
/// says otherwise (macOS only; never on Android).
pub fn on_close(
    quitting: bool,
    keep_running: bool,
    tray: bool,
    icon: Icon,
    taskbar: bool,
    dock: bool,
) -> OnClose {
    if quitting || !keep_running {
        OnClose::Quit
    } else if dock {
        OnClose::Hide
    } else if !tray {
        OnClose::Quit
    } else if icon != Icon::Gone {
        OnClose::Hide
    } else if taskbar {
        OnClose::Minimize
    } else {
        OnClose::Quit
    }
}

/// The notification-area icon: never made on Android ([`Tray::new`] answers
/// [`None`], and `available()` already keeps the window from trying).
pub struct Tray;

impl Tray {
    pub fn new(_ctx: &egui::Context, _limiter: bool) -> Option<Tray> {
        None
    }

    pub fn icon(&self) -> Icon {
        Icon::Gone
    }

    pub fn icon_now(&self) -> Icon {
        Icon::Gone
    }

    pub fn rescue(&self) -> bool {
        false
    }

    pub fn poll(&self) -> Vec<TrayCmd> {
        Vec::new()
    }

    pub fn relabel(&self) {}

    pub fn set_queue(&self, _running: bool) {}

    pub fn set_tooltip(&self, _tip: &str) {}

    pub fn set_downloads(&self, _running: &[(u64, String)]) {}

    pub fn set_limiter(&self, _on: bool) {}
}
