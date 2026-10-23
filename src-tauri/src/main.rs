// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Elevated WinUSB install mode (the app relaunching itself via UAC): handled
    // before any Tauri/WebView code runs; see system/elevated_install.rs.
    if let Some(code) = bfsw_lib::system::elevated_install::dispatch(std::env::args()) {
        std::process::exit(code);
    }
    bfsw_lib::run()
}
