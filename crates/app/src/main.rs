//! The desktop binary: everything lives in the library (`lib.rs`), so the
//! Android port's cdylib (`mdm-android`) can reuse the same modules.

#![windows_subsystem = "windows"]

use mdm_app::main_entry;

fn main() -> std::process::ExitCode {
    main_entry()
}
