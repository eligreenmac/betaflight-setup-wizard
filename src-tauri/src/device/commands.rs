//! Tauri commands for workstream `device`. Signatures are a contract with the UI
//! (src/api/commands.ts) - do not change without a `shared` PR.

use crate::error::{AppError, AppResult};
use crate::model::{FcInfo, HostPort, MotorTestLimits, MotorTestSession};

/// List host serial ports, classified by VID/PID.
#[tauri::command]
pub async fn device_list_ports() -> AppResult<Vec<HostPort>> {
    Err(AppError::NotImplemented("device_list_ports"))
}

/// Open `path` (e.g. "COM5") at 115200 8N1 and keep it as the active connection.
#[tauri::command]
pub async fn device_connect(path: String) -> AppResult<()> {
    let _ = path;
    Err(AppError::NotImplemented("device_connect"))
}

#[tauri::command]
pub async fn device_disconnect() -> AppResult<()> {
    Err(AppError::NotImplemented("device_disconnect"))
}

/// Read FC identity via MSP (API_VERSION, FC_VARIANT, FC_VERSION, BOARD_INFO, UID, BUILD_INFO).
#[tauri::command]
pub async fn device_identify() -> AppResult<FcInfo> {
    Err(AppError::NotImplemented("device_identify"))
}

/// Run one raw CLI line (enters CLI mode if needed). For diagnostics/advanced use.
#[tauri::command]
pub async fn device_cli_exec(line: String) -> AppResult<String> {
    let _ = line;
    Err(AppError::NotImplemented("device_cli_exec"))
}

/// Reboot the connected FC into its ROM DFU bootloader (MSP_SET_REBOOT / CLI `bl`).
#[tauri::command]
pub async fn device_reboot_to_bootloader() -> AppResult<()> {
    Err(AppError::NotImplemented("device_reboot_to_bootloader"))
}

/// `device_identify` plus a read-only CLI probe (`serial`, `resource`,
/// `get rx_spi_protocol`) that fills `rxSpi.present`. Leaves the CLI with
/// `exit noreboot` on 4.5+; on 4.4 leaving the CLI reboots the FC (reconnect needed).
#[tauri::command]
pub async fn device_probe_board() -> AppResult<FcInfo> {
    Err(AppError::NotImplemented("device_probe_board"))
}

/// Start (first call) or update (later calls) a motor test on the active connection.
/// Safety gate: `props_off_confirmed`, FC disarmed, no 3D, valid motor, stop <= throttle
/// <= cap. Feeds the watchdog. Emits `device://motor-test-stopped` when the test ends.
#[tauri::command]
pub async fn device_motor_test_start(
    motor_index: Option<u8>,
    throttle: u16,
    props_off_confirmed: bool,
    limits: Option<MotorTestLimits>,
) -> AppResult<MotorTestSession> {
    let _ = (motor_index, throttle, props_off_confirmed, limits);
    Err(AppError::NotImplemented("device_motor_test_start"))
}

/// Feed the motor-test watchdog (UI: every 200 ms). `MOTOR_TEST_NOT_ACTIVE` once stopped.
#[tauri::command]
pub async fn device_motor_test_keepalive() -> AppResult<()> {
    Err(AppError::NotImplemented("device_motor_test_keepalive"))
}

/// Stop all motors and hand the port back to the connection manager.
#[tauri::command]
pub async fn device_motor_test_stop() -> AppResult<()> {
    Err(AppError::NotImplemented("device_motor_test_stop"))
}

/// Stream MSP_RC on `device://rc-channels` (`rate_hz` default 15, clamped 1..=50).
#[tauri::command]
pub async fn device_rc_stream_start(rate_hz: Option<u32>) -> AppResult<()> {
    let _ = rate_hz;
    Err(AppError::NotImplemented("device_rc_stream_start"))
}

#[tauri::command]
pub async fn device_rc_stream_stop() -> AppResult<()> {
    Err(AppError::NotImplemented("device_rc_stream_stop"))
}
