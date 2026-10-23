//! Betaflight Setup Wizard - Rust core.
//!
//! Module ownership (see docs/ARCHITECTURE.md):
//!   device/   -> workstream `device`
//!   flasher/  -> workstream `flasher`
//!   config/   -> workstream `config`
//!   system/   -> workstream `packaging` (drivers, environment checks)
//!   model.rs, error.rs, this file -> shared; change only via a PR labelled `shared`.
//!
//! All Tauri commands are registered here ONCE so that workstreams never
//! need to edit this file: they implement the functions in their own module.

pub mod config;
pub mod device;
pub mod error;
pub mod flasher;
pub mod model;
pub mod system;

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            // device
            device::commands::device_list_ports,
            device::commands::device_connect,
            device::commands::device_disconnect,
            device::commands::device_identify,
            device::commands::device_cli_exec,
            device::commands::device_reboot_to_bootloader,
            device::commands::device_probe_board,
            device::commands::device_motor_test_start,
            device::commands::device_motor_test_keepalive,
            device::commands::device_motor_test_stop,
            device::commands::device_rc_stream_start,
            device::commands::device_rc_stream_stop,
            // flasher
            flasher::commands::flasher_list_targets,
            flasher::commands::flasher_list_releases,
            flasher::commands::flasher_list_dfu_devices,
            flasher::commands::flasher_fetch_firmware,
            flasher::commands::flasher_list_cache,
            flasher::commands::flasher_flash,
            flasher::commands::flasher_remove_read_protection,
            // config
            config::commands::config_read_ports,
            config::commands::config_backup,
            config::commands::config_generate,
            config::commands::config_elrs_uid,
            config::commands::config_plan,
            config::commands::config_apply,
            config::commands::config_verify,
            config::commands::config_rollback,
            config::commands::config_list_backups,
            config::commands::config_backups_dir,
            config::commands::config_restore,
            // system (packaging)
            system::commands::system_driver_status,
            system::commands::system_install_dfu_driver,
            system::commands::system_app_info,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Betaflight Setup Wizard");
}
