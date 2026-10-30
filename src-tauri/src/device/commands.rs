// SPDX-License-Identifier: GPL-3.0-or-later
//! Tauri commands for workstream `device`. Signatures are a contract with the UI
//! (src/api/commands.ts) - do not change without a `shared` PR.
//!
//! Every command runs on a blocking thread against the app-wide
//! [`ConnectionManager`] in Tauri managed state.

use super::manager::{host_ports, ConnectionManager};
use super::motor::MotorTestRequest;
use super::{
    ATTITUDE_EVENT, ATTITUDE_STOPPED_EVENT, MOTOR_TEST_STOPPED_EVENT, RC_CHANNELS_EVENT,
    RC_STOPPED_EVENT,
};
use crate::error::{AppError, AppResult};
use crate::model::{
    AccCalibrationResult, Attitude, FcInfo, HostPort, MotorTestLimits, MotorTestSession,
    SensorStatus,
};
use tauri::{AppHandle, Emitter, Manager};

async fn blocking<T: Send + 'static>(
    app: AppHandle,
    f: impl FnOnce(&ConnectionManager, &AppHandle) -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(move || {
        let mgr = app.state::<ConnectionManager>();
        f(&mgr, &app)
    })
    .await
    .map_err(|e| AppError::Serial(format!("device worker failed: {e}")))?
}

/// List host serial ports, classified by VID/PID (FC VCPs first; DFU devices excluded).
#[tauri::command]
pub async fn device_list_ports(app: AppHandle) -> AppResult<Vec<HostPort>> {
    blocking(app, |m, _| Ok(host_ports(m.lister()))).await
}

/// Open `path` (e.g. "COM5") at 115200 8N1 and keep it as the active connection.
#[tauri::command]
pub async fn device_connect(app: AppHandle, path: String) -> AppResult<()> {
    blocking(app, move |m, _| m.connect(&path)).await
}

/// Stops a running motor test / RC stream first, then closes the port.
#[tauri::command]
pub async fn device_disconnect(app: AppHandle) -> AppResult<()> {
    blocking(app, |m, _| m.disconnect()).await
}

/// Read FC identity via MSP (API_VERSION, FC_VARIANT, FC_VERSION, BOARD_INFO, UID,
/// serial config, RX hints). `uid` is the MSP_UID device identifier.
#[tauri::command]
pub async fn device_identify(app: AppHandle) -> AppResult<FcInfo> {
    blocking(app, |m, _| m.identify()).await
}

/// Run one raw CLI line (enters CLI mode if needed). For diagnostics/advanced use.
#[tauri::command]
pub async fn device_cli_exec(app: AppHandle, line: String) -> AppResult<String> {
    blocking(app, move |m, _| m.cli_exec(&line)).await
}

/// "Reboot board": reboot the connected FC (plain `exit` if our CLI is open, never `save`;
/// else MSP_REBOOT `[0]`), wait for it to come back, re-verify API version + UID. Resolves
/// once the reboot is confirmed and clears `PENDING_UNSAVED_CHANGES` (ARCHITECTURE §4.1).
/// Not confirmed: `PENDING_UNSAVED_CHANGES` (RAM still dirty) or `MSP`; did not come back /
/// another board: `SERIAL`.
#[tauri::command]
pub async fn device_reboot(app: AppHandle) -> AppResult<()> {
    blocking(app, |m, _| m.reboot()).await
}

/// Reboot the connected FC into its ROM DFU bootloader (MSP_REBOOT, CLI `bl rom` fallback).
/// The connection is closed afterwards.
#[tauri::command]
pub async fn device_reboot_to_bootloader(app: AppHandle) -> AppResult<()> {
    blocking(app, |m, _| m.reboot_to_bootloader()).await
}

/// `device_identify` plus a read-only CLI probe (`serial`, `resource`,
/// `get rx_spi_protocol`, `get displayport_msp_col_adjust`) that fills `rxSpi.present` and
/// can settle a null `hdOsdSupported` to false. Leaves the CLI with
/// `exit noreboot` on 4.5+; on 4.4 leaving the CLI reboots the FC and this reconnects.
#[tauri::command]
pub async fn device_probe_board(app: AppHandle) -> AppResult<FcInfo> {
    blocking(app, |m, _| m.probe_board()).await
}

/// Start (first call) or update (later calls) a motor test on the active connection.
/// Safety gate: `props_off_confirmed`, FC disarmed, no 3D, valid motor, stop <= throttle
/// <= cap. Feeds the watchdog. Emits `device://motor-test-stopped` when the test ends.
#[tauri::command]
pub async fn device_motor_test_start(
    app: AppHandle,
    motor_index: Option<u8>,
    throttle: u16,
    props_off_confirmed: bool,
    limits: Option<MotorTestLimits>,
) -> AppResult<MotorTestSession> {
    let req = MotorTestRequest {
        motor_index,
        throttle,
        props_off_confirmed,
    };
    blocking(app, move |m, app| {
        let app = app.clone();
        m.motor_test_start(req, limits, move |ev| {
            let _ = app.emit(MOTOR_TEST_STOPPED_EVENT, ev);
        })
    })
    .await
}

/// Feed the motor-test watchdog (UI: every 200 ms). `MOTOR_TEST_NOT_ACTIVE` once stopped.
#[tauri::command]
pub async fn device_motor_test_keepalive(app: AppHandle) -> AppResult<()> {
    blocking(app, |m, _| m.motor_test_keepalive()).await
}

/// Stop all motors and hand the port back to the connection manager. Idempotent.
#[tauri::command]
pub async fn device_motor_test_stop(app: AppHandle) -> AppResult<()> {
    blocking(app, |m, _| m.motor_test_stop()).await
}

/// Stream MSP_RC on `device://rc-channels` (`rate_hz` default 15, clamped 1..=50).
#[tauri::command]
pub async fn device_rc_stream_start(app: AppHandle, rate_hz: Option<u32>) -> AppResult<()> {
    blocking(app, move |m, app| {
        let a1 = app.clone();
        let a2 = app.clone();
        m.rc_stream_start(
            rate_hz,
            move |ev| {
                let _ = a1.emit(RC_CHANNELS_EVENT, ev);
            },
            move |ev| {
                let _ = a2.emit(RC_STOPPED_EVENT, ev);
            },
        )
    })
    .await
}

/// Idempotent.
#[tauri::command]
pub async fn device_rc_stream_stop(app: AppHandle) -> AppResult<()> {
    blocking(app, |m, _| m.rc_stream_stop()).await
}

/// Sensor status: per sensor (gyro, acc, baro, mag) `present` / `enabled` / configured and
/// detected hardware, plus `accCalibrated` and a read-only alignment readout. No writes.
#[tauri::command]
pub async fn device_sensors(app: AppHandle) -> AppResult<SensorStatus> {
    blocking(app, |m, _| m.get_sensors()).await
}

/// Calibrate the accelerometer (MSP_ACC_CALIBRATION) with the board level and still;
/// resolves when the FC is done. The FC saves the result to EEPROM by itself. Errors:
/// `BOARD_ARMED`, `ACC_NOT_AVAILABLE`, `BOARD_MOVING`, `ACC_CALIBRATION_FAILED`.
#[tauri::command]
pub async fn device_acc_calibrate(app: AppHandle) -> AppResult<AccCalibrationResult> {
    blocking(app, |m, _| m.acc_calibrate()).await
}

/// One MSP_ATTITUDE sample (`seq` and `timestampMs` are 0).
#[tauri::command]
pub async fn device_get_attitude(app: AppHandle) -> AppResult<Attitude> {
    blocking(app, |m, _| m.get_attitude()).await
}

/// Stream MSP_ATTITUDE on `device://attitude` (`rate_hz` default 30, clamped 1..=50), latest
/// sample only (a slow listener sees gaps in `seq`, never a backlog). Exclusive with the
/// motor test and the RC stream (`PORT_BUSY {heldBy: APP}`); `device_identify`,
/// `device_sensors`, `device_get_attitude` and the unsaved-changes check share the port
/// with it (the stream pauses for them), other port commands get `PORT_BUSY`.
#[tauri::command]
pub async fn device_attitude_stream_start(app: AppHandle, rate_hz: Option<u32>) -> AppResult<()> {
    blocking(app, move |m, app| {
        let a1 = app.clone();
        let a2 = app.clone();
        m.attitude_stream_start(
            rate_hz,
            move |ev| {
                let _ = a1.emit(ATTITUDE_EVENT, ev);
            },
            move |ev| {
                let _ = a2.emit(ATTITUDE_STOPPED_EVENT, ev);
            },
        )
    })
    .await
}

/// Idempotent.
#[tauri::command]
pub async fn device_attitude_stream_stop(app: AppHandle) -> AppResult<()> {
    blocking(app, |m, _| m.attitude_stream_stop()).await
}
