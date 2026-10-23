// SPDX-License-Identifier: GPL-3.0-or-later
//! Sensor status, accelerometer calibration and attitude (read-only MSP; no config writes).
//!
//! Payload layouts follow Betaflight `src/main/msp/msp.c` (4.4.3, 4.5.5, 2025.12.5, 2026.6.2) and
//! Configurator `MSPHelper.ts`; see ARCHITECTURE.md §4.3 for the behaviour and evidence.

use crate::device::error::{DeviceError, Result};
use crate::device::motor::{parse_status, parse_status_ex, FcStatus};
use crate::device::msp::{codes, parse_board_info, MspClient, Reader};
use crate::device::serial::SerialLink;
use crate::model::{
    AccCalibrationCompletion, AccCalibrationResult, Attitude, SensorAlignment, SensorState,
    SensorStatus,
};
use std::time::{Duration, Instant};

/// MSP_STATUS `sensors` bits (`sensors_e`, sensors.h): set = detected and running.
pub mod sensor_bits {
    pub const ACC: u16 = 1 << 0;
    pub const BARO: u16 = 1 << 1;
    pub const MAG: u16 = 1 << 2;
    pub const GYRO: u16 = 1 << 5;
}

/// `ARMING_DISABLED_CALIBRATING` (runtime_config.h): set while any sensor calibrates.
pub const ARMING_DISABLED_CALIBRATING: u32 = 1 << 12;
/// MSP_BOARD_INFO configuration problem `PROBLEM_ACC_NEEDS_CALIBRATION`.
pub const PROBLEM_ACC_NEEDS_CALIBRATION: u32 = 1 << 0;
/// `ACC_NONE` / `BARO_NONE` / `MAG_NONE`: the sensor is disabled in the config.
pub const HARDWARE_NONE: u8 = 1;
/// MSP2_SENSOR_CONFIG_ACTIVE: no driver compiled in.
pub const SENSOR_NOT_AVAILABLE: u8 = 0xFF;

// ------------------------------------------------------------------------- payloads

/// MSP_ATTITUDE (108): roll, pitch int16 decidegrees; yaw int16 whole degrees
/// (msp.c 4.5.5:1324-1328). Returns (roll, pitch, yaw) in degrees.
pub fn parse_attitude(p: &[u8]) -> Result<(f32, f32, f32)> {
    let mut r = Reader::new(codes::MSP_ATTITUDE, p);
    let roll = r.i16()? as f32 / 10.0;
    let pitch = r.i16()? as f32 / 10.0;
    let yaw = r.i16()? as f32;
    Ok((roll, pitch, yaw))
}

/// MSP_RAW_IMU (102): acc[3], gyro[3], mag[3] int16; gyro in whole deg/s
/// (`gyroRateDps`, msp.c 4.5.5:1131). Returns the gyro rates.
pub fn parse_raw_imu_gyro(p: &[u8]) -> Result<[i16; 3]> {
    let mut r = Reader::new(codes::MSP_RAW_IMU, p);
    r.bytes(6)?;
    Ok([r.i16()?, r.i16()?, r.i16()?])
}

/// MSP_SENSOR_CONFIG (96): acc_hardware, baro_hardware, mag_hardware (u8 each, then
/// rangefinder / optical flow on newer firmware, ignored).
pub fn parse_sensor_config(p: &[u8]) -> Result<[u8; 3]> {
    let mut r = Reader::new(codes::MSP_SENSOR_CONFIG, p);
    Ok([r.u8()?, r.u8()?, r.u8()?])
}

/// MSP2_SENSOR_CONFIG_ACTIVE (0x300A, API 1.46+): detected gyro, acc, baro, mag hardware.
pub fn parse_sensor_config_active(p: &[u8]) -> Result<[u8; 4]> {
    let mut r = Reader::new(codes::MSP2_SENSOR_CONFIG_ACTIVE, p);
    Ok([r.u8()?, r.u8()?, r.u8()?, r.u8()?])
}

/// MSP_BOARD_ALIGNMENT_CONFIG (38): roll, pitch, yaw int16 degrees.
pub fn parse_board_alignment(p: &[u8]) -> Result<[i16; 3]> {
    let mut r = Reader::new(codes::MSP_BOARD_ALIGNMENT_CONFIG, p);
    Ok([r.i16()?, r.i16()?, r.i16()?])
}

/// MSP_SENSOR_ALIGNMENT (126): first byte is the active gyro's alignment.
pub fn parse_sensor_alignment(p: &[u8]) -> Result<u8> {
    Reader::new(codes::MSP_SENSOR_ALIGNMENT, p).u8()
}

// ------------------------------------------------------------------------ names

/// `lookupTableAlignment` (settings.c), same in 4.4 - 2026.6.
const ALIGNMENT: &[&str] = &[
    "DEFAULT",
    "CW0",
    "CW90",
    "CW180",
    "CW270",
    "CW0FLIP",
    "CW90FLIP",
    "CW180FLIP",
    "CW270FLIP",
    "CUSTOM",
];

pub fn alignment_name(v: u8) -> String {
    ALIGNMENT
        .get(v as usize)
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("UNKNOWN_{v}"))
}

/// Hardware lookup tables from settings.c (2026.6: sensors/sensors.c), by firmware (MSP API
/// minor: 45 = 4.4, 46 = 4.5, 47 = 2025.12, 48+ = 2026.6). Spellings are the firmware's (4.4 gyro "LSM6SDO" included).
mod tables {
    pub const GYRO_44: &[&str] = &[
        "AUTO",
        "NONE",
        "MPU6050",
        "L3G4200D",
        "MPU3050",
        "L3GD20",
        "MPU6000",
        "MPU6500",
        "MPU9250",
        "ICM20601",
        "ICM20602",
        "ICM20608G",
        "ICM20649",
        "ICM20689",
        "ICM42605",
        "ICM42688P",
        "BMI160",
        "BMI270",
        "LSM6SDO",
        "FAKE",
    ];
    pub const ACC_44: &[&str] = &[
        "AUTO",
        "NONE",
        "ADXL345",
        "MPU6050",
        "MMA8452",
        "BMA280",
        "LSM303DLHC",
        "MPU6000",
        "MPU6500",
        "MPU9250",
        "ICM20601",
        "ICM20602",
        "ICM20608G",
        "ICM20649",
        "ICM20689",
        "ICM42605",
        "ICM42688P",
        "BMI160",
        "BMI270",
        "LSM6DSO",
        "FAKE",
    ];
    pub const BARO_44: &[&str] = &[
        "AUTO",
        "NONE",
        "BMP085",
        "MS5611",
        "BMP280",
        "LPS",
        "QMP6988",
        "BMP388",
        "DPS310",
        "2SMPB_02B",
    ];
    pub const MAG_44: &[&str] = &[
        "AUTO",
        "NONE",
        "HMC5883",
        "AK8975",
        "AK8963",
        "QMC5883",
        "LIS3MDL",
        "MAG_MPU925X_AK8963",
    ];
    pub const GYRO_45: &[&str] = &[
        "AUTO",
        "NONE",
        "MPU6050",
        "L3G4200D",
        "MPU3050",
        "L3GD20",
        "MPU6000",
        "MPU6500",
        "MPU9250",
        "ICM20601",
        "ICM20602",
        "ICM20608G",
        "ICM20649",
        "ICM20689",
        "ICM42605",
        "ICM42688P",
        "BMI160",
        "BMI270",
        "LSM6DSO",
        "LSM6DSV16X",
        "VIRTUAL",
    ];
    pub const ACC_45: &[&str] = &[
        "AUTO",
        "NONE",
        "ADXL345",
        "MPU6050",
        "MMA8452",
        "BMA280",
        "LSM303DLHC",
        "MPU6000",
        "MPU6500",
        "MPU9250",
        "ICM20601",
        "ICM20602",
        "ICM20608G",
        "ICM20649",
        "ICM20689",
        "ICM42605",
        "ICM42688P",
        "BMI160",
        "BMI270",
        "LSM6DSO",
        "LSM6DSV16X",
        "VIRTUAL",
    ];
    pub const BARO_45: &[&str] = &[
        "AUTO",
        "NONE",
        "BMP085",
        "MS5611",
        "BMP280",
        "LPS",
        "QMP6988",
        "BMP388",
        "DPS310",
        "2SMPB_02B",
        "LPS22DF",
        "VIRTUAL",
    ];
    pub const MAG_45: &[&str] = &[
        "AUTO",
        "NONE",
        "HMC5883",
        "AK8975",
        "AK8963",
        "QMC5883",
        "LIS2MDL",
        "LIS3MDL",
        "MPU925X_AK8963",
        "IST8310",
    ];
    pub const GYRO_2025: &[&str] = &[
        "AUTO",
        "NONE",
        "MPU6050",
        "L3GD20",
        "MPU6000",
        "MPU6500",
        "MPU9250",
        "ICM20601",
        "ICM20602",
        "ICM20608G",
        "ICM20649",
        "ICM20689",
        "ICM42605",
        "ICM42688P",
        "BMI160",
        "BMI270",
        "LSM6DSO",
        "LSM6DSV16X",
        "IIM42653",
        "ICM45605",
        "ICM45686",
        "ICM40609D",
        "IIM42652",
        "VIRTUAL",
    ];
    pub const ACC_2025: &[&str] = &[
        "AUTO",
        "NONE",
        "MPU6050",
        "MPU6000",
        "MPU6500",
        "MPU9250",
        "ICM20601",
        "ICM20602",
        "ICM20608G",
        "ICM20649",
        "ICM20689",
        "ICM42605",
        "ICM42688P",
        "BMI160",
        "BMI270",
        "LSM6DSO",
        "LSM6DSV16X",
        "IIM42653",
        "ICM45605",
        "ICM45686",
        "ICM40609D",
        "IIM42652",
        "VIRTUAL",
    ];
    /// 2026.6.2 `sensors/sensors.c` `lookupTableGyroHardware` (`gyroHardware_e`: NONE = 0,
    /// DEFAULT = 1; the gyro is the one table that starts with NONE).
    pub const GYRO_2026: &[&str] = &[
        "NONE",
        "AUTO",
        "MPU6050",
        "L3GD20",
        "MPU6000",
        "MPU6500",
        "MPU9250",
        "ICM20601",
        "ICM20602",
        "ICM20608G",
        "ICM20649",
        "ICM20689",
        "ICM42605",
        "ICM42688P",
        "BMI160",
        "BMI270",
        "LSM6DSO",
        "LSM6DSV16X",
        "IIM42653",
        "ICM45605",
        "ICM45686",
        "ICM40609D",
        "IIM42652",
        "LSM6DSK320X",
        "ICM42622P",
        "ICM42686P",
        "ICM56686",
        "VIRTUAL",
    ];
    /// 2026.6.2 `lookupTableAccHardware`: adds LSM6DSK320X..ICM56686 before VIRTUAL.
    pub const ACC_2026: &[&str] = &[
        "AUTO",
        "NONE",
        "MPU6050",
        "MPU6000",
        "MPU6500",
        "MPU9250",
        "ICM20601",
        "ICM20602",
        "ICM20608G",
        "ICM20649",
        "ICM20689",
        "ICM42605",
        "ICM42688P",
        "BMI160",
        "BMI270",
        "LSM6DSO",
        "LSM6DSV16X",
        "IIM42653",
        "ICM45605",
        "ICM45686",
        "ICM40609D",
        "IIM42652",
        "LSM6DSK320X",
        "ICM42622P",
        "ICM42686P",
        "ICM56686",
        "VIRTUAL",
    ];
    /// 2026.6.2 `lookupTableBaroHardware`: BMP580/BMP581 insert at 11, VIRTUAL moves 11 -> 13.
    pub const BARO_2026: &[&str] = &[
        "AUTO",
        "NONE",
        "BMP085",
        "MS5611",
        "BMP280",
        "LPS",
        "QMP6988",
        "BMP388",
        "DPS310",
        "2SMPB_02B",
        "LPS22DF",
        "BMP580",
        "BMP581",
        "VIRTUAL",
        "SPA06_003",
    ];
    /// 2026.6.2 `lookupTableMagHardware`: QMC5883 is renamed QMC5883L; DRONECAN only with
    /// `ENABLE_DRONECAN`.
    pub const MAG_2026: &[&str] = &[
        "AUTO",
        "NONE",
        "HMC5883",
        "AK8975",
        "AK8963",
        "QMC5883L",
        "LIS2MDL",
        "LIS3MDL",
        "MPU925X_AK8963",
        "IST8310",
        "MMC560X",
        "QMC5883P",
        "DRONECAN",
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SensorKind {
    Gyro,
    Acc,
    Baro,
    Mag,
}

/// CLI name of a hardware id (`gyro_hardware`-style enum) for the given MSP API minor.
pub fn hardware_name(kind: SensorKind, api_minor: u8, id: u8) -> Option<String> {
    use tables::*;
    let t: &[&str] = match (kind, api_minor) {
        (SensorKind::Gyro, ..=45) => GYRO_44,
        (SensorKind::Acc, ..=45) => ACC_44,
        (SensorKind::Baro, ..=45) => BARO_44,
        (SensorKind::Mag, ..=45) => MAG_44,
        (SensorKind::Gyro, 46) => GYRO_45,
        (SensorKind::Acc, 46) => ACC_45,
        (SensorKind::Gyro, 47) => GYRO_2025,
        (SensorKind::Acc, 47) => ACC_2025,
        (SensorKind::Baro, 47) => BARO_45,
        (SensorKind::Mag, 47) => MAG_45,
        (SensorKind::Gyro, _) => GYRO_2026,
        (SensorKind::Acc, _) => ACC_2026,
        (SensorKind::Baro, _) => BARO_2026,
        (SensorKind::Mag, _) => MAG_2026,
    };
    Some(
        t.get(id as usize)
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("UNKNOWN_{id}")),
    )
}

/// A detected id worth reporting: not NONE (nothing found), not 0xFF (not compiled in).
fn detected_name(kind: SensorKind, api_minor: u8, id: u8) -> Option<String> {
    if id == HARDWARE_NONE || id == SENSOR_NOT_AVAILABLE || id == 0 {
        None
    } else {
        hardware_name(kind, api_minor, id)
    }
}

// ----------------------------------------------------------------------- reading

/// MSP error frame / malformed payload = "unknown"; transport errors propagate.
fn optional<T>(r: Result<T>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(DeviceError::MspUnsupported(_)) | Err(DeviceError::MspPayloadTooShort { .. }) => {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// MSP_STATUS_EX, falling back to MSP_STATUS (same as the motor test).
pub fn read_status<S: SerialLink>(c: &mut MspClient<S>) -> Result<FcStatus> {
    match c.request(codes::MSP_STATUS_EX, &[]) {
        Ok(p) => parse_status_ex(&p),
        Err(DeviceError::MspUnsupported(_)) => parse_status(&c.request(codes::MSP_STATUS, &[])?),
        Err(e) => Err(e),
    }
}

fn api_minor<S: SerialLink>(c: &mut MspClient<S>) -> Result<u8> {
    let p = c.request(codes::MSP_API_VERSION, &[])?;
    Ok(p.get(2).copied().unwrap_or(0))
}

/// `[acc, baro, mag]` configured hardware, `None` if the FC does not answer.
fn read_sensor_config<S: SerialLink>(c: &mut MspClient<S>) -> Result<Option<[u8; 3]>> {
    optional(
        c.request(codes::MSP_SENSOR_CONFIG, &[])
            .and_then(|p| parse_sensor_config(&p)),
    )
}

fn acc_needs_calibration<S: SerialLink>(c: &mut MspClient<S>) -> Result<Option<bool>> {
    let info = optional(
        c.request(codes::MSP_BOARD_INFO, &[])
            .and_then(|p| parse_board_info(&p)),
    )?;
    Ok(info
        .and_then(|i| i.configuration_problems)
        .map(|p| p & PROBLEM_ACC_NEEDS_CALIBRATION != 0))
}

/// Read everything `device_get_sensors` returns. Read-only.
pub fn read_sensors<S: SerialLink>(c: &mut MspClient<S>) -> Result<SensorStatus> {
    let api = api_minor(c)?;
    let status = read_status(c)?;
    let config = read_sensor_config(c)?;
    let active = optional(
        c.request(codes::MSP2_SENSOR_CONFIG_ACTIVE, &[])
            .and_then(|p| parse_sensor_config_active(&p)),
    )?;
    let board = optional(
        c.request(codes::MSP_BOARD_ALIGNMENT_CONFIG, &[])
            .and_then(|p| parse_board_alignment(&p)),
    )?;
    let gyro_align = optional(
        c.request(codes::MSP_SENSOR_ALIGNMENT, &[])
            .and_then(|p| parse_sensor_alignment(&p)),
    )?;
    let present = |bit: u16| status.sensors & bit != 0;
    let state = |kind: SensorKind, bit: u16, cfg: Option<u8>, det: Option<u8>| {
        let present = present(bit);
        SensorState {
            present,
            // Without MSP_SENSOR_CONFIG: a present sensor is certainly enabled; an absent
            // one is reported as enabled (= "not known to be disabled").
            enabled: cfg.is_none_or(|h| h != HARDWARE_NONE),
            configured_hardware: cfg.and_then(|h| hardware_name(kind, api, h)),
            detected_hardware: det.and_then(|d| detected_name(kind, api, d)),
        }
    };
    let det = |i: usize| active.map(|a| a[i]);
    let cfg = |i: usize| config.map(|c| c[i]);
    let acc = state(SensorKind::Acc, sensor_bits::ACC, cfg(0), det(1));
    let acc_calibrated = if acc.present {
        acc_needs_calibration(c)?.map(|n| !n)
    } else {
        None
    };
    Ok(SensorStatus {
        gyro: SensorState {
            present: present(sensor_bits::GYRO),
            enabled: true,
            configured_hardware: None,
            detected_hardware: det(0).and_then(|d| detected_name(SensorKind::Gyro, api, d)),
        },
        acc,
        baro: state(SensorKind::Baro, sensor_bits::BARO, cfg(1), det(2)),
        mag: state(SensorKind::Mag, sensor_bits::MAG, cfg(2), det(3)),
        acc_calibrated,
        alignment: match (board, gyro_align) {
            (Some(b), Some(g)) => Some(SensorAlignment {
                board_roll_deg: b[0],
                board_pitch_deg: b[1],
                board_yaw_deg: b[2],
                gyro_alignment: alignment_name(g),
            }),
            _ => None,
        },
    })
}

/// One MSP_ATTITUDE read (`device_get_attitude`).
pub fn read_attitude<S: SerialLink>(c: &mut MspClient<S>) -> Result<Attitude> {
    let (roll, pitch, yaw) = parse_attitude(&c.request(codes::MSP_ATTITUDE, &[])?)?;
    Ok(Attitude {
        seq: 0,
        timestamp_ms: 0,
        roll,
        pitch,
        yaw,
    })
}

// ------------------------------------------------------------------- calibration

/// Timing of [`acc_calibrate`]. Defaults are for real hardware; tests shrink them.
#[derive(Debug, Clone, Copy)]
pub struct AccCalibrationOptions {
    /// Gyro samples taken before calibrating (motion check).
    pub motion_samples: u32,
    pub motion_sample_interval: Duration,
    /// Max gyro rate (deg/s, any axis) that still counts as "still".
    pub motion_limit_dps: u16,
    /// STATUS_EX / RAW_IMU poll interval while calibrating.
    pub poll_interval: Duration,
    /// If the CALIBRATING flag has not been seen by then, stop looking for it.
    pub flag_window: Duration,
    /// Wait used when the flag cannot be observed (Configurator's
    /// `ACC_CALIBRATION_TIMEOUT_MS`).
    pub fixed_wait: Duration,
    /// Give up waiting for the flag to clear after this long.
    pub max_wait: Duration,
}

impl Default for AccCalibrationOptions {
    fn default() -> Self {
        AccCalibrationOptions {
            motion_samples: 5,
            motion_sample_interval: Duration::from_millis(40),
            motion_limit_dps: 8,
            poll_interval: Duration::from_millis(50),
            flag_window: Duration::from_millis(1000),
            fixed_wait: Duration::from_millis(2000),
            max_wait: Duration::from_millis(4000),
        }
    }
}

fn max_rate(g: [i16; 3]) -> u16 {
    g.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0)
}

/// Gyro rate sample; `None` if RAW_IMU is not answered (motion check is skipped then).
fn gyro_rate<S: SerialLink>(c: &mut MspClient<S>) -> Result<Option<u16>> {
    match c.request(codes::MSP_RAW_IMU, &[]) {
        Ok(p) => Ok(parse_raw_imu_gyro(&p).ok().map(max_rate)),
        Err(DeviceError::MspUnsupported(_)) | Err(DeviceError::Timeout { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Calibrate the accelerometer (MSP_ACC_CALIBRATION) and wait until the FC is done.
///
/// Refuses (nothing sent) when armed (`Armed`; the FC itself would silently ignore the
/// request, msp.c 4.5.5:3285-3288), when the acc is absent or disabled
/// (`AccNotAvailable`) and when the gyro shows the board moving (`BoardMoving`,
/// `during: false`). The FC saves the result to EEPROM itself (acceleration_init.c
/// `performAcclerationCalibration` -> `saveConfigAndNotify()`).
pub fn acc_calibrate<S: SerialLink>(
    c: &mut MspClient<S>,
    opts: &AccCalibrationOptions,
) -> Result<AccCalibrationResult> {
    let status = read_status(c)?;
    if status.is_armed() {
        return Err(DeviceError::Armed);
    }
    let present = status.sensors & sensor_bits::ACC != 0;
    let enabled = read_sensor_config(c)?.is_none_or(|h| h[0] != HARDWARE_NONE);
    if !present || !enabled {
        return Err(DeviceError::AccNotAvailable { present, enabled });
    }

    // Motion pre-check.
    let mut before = 0u16;
    for i in 0..opts.motion_samples {
        if i > 0 {
            std::thread::sleep(opts.motion_sample_interval);
        }
        match gyro_rate(c)? {
            Some(r) => before = before.max(r),
            None => break,
        }
    }
    if before > opts.motion_limit_dps {
        return Err(DeviceError::BoardMoving {
            max_rate_dps: before,
            limit_dps: opts.motion_limit_dps,
            during: false,
        });
    }

    // The flag is only a usable completion signal if it is clear now (nothing else, e.g.
    // a baro, is calibrating) and STATUS_EX reports it at all.
    let flag_usable = status
        .arming_disable_flags
        .is_some_and(|f| f & ARMING_DISABLED_CALIBRATING == 0);

    match c.request(codes::MSP_ACC_CALIBRATION, &[]) {
        Ok(_) => {}
        Err(DeviceError::MspUnsupported(_)) => {
            // Built without USE_ACC.
            return Err(DeviceError::AccNotAvailable {
                present: false,
                enabled,
            });
        }
        Err(e) => return Err(e),
    }
    let started = Instant::now();

    let mut seen = false;
    let mut during = 0u16;
    let completion = loop {
        std::thread::sleep(opts.poll_interval);
        let elapsed = started.elapsed();
        if flag_usable {
            match read_status(c) {
                Ok(s) => {
                    let calibrating = s
                        .arming_disable_flags
                        .is_some_and(|f| f & ARMING_DISABLED_CALIBRATING != 0);
                    if calibrating {
                        seen = true;
                    } else if seen {
                        break AccCalibrationCompletion::CalibratingFlag;
                    }
                }
                // The FC is busy writing the EEPROM at the end of the calibration.
                Err(DeviceError::Timeout { .. }) => {}
                Err(e) => return Err(e),
            }
        }
        if let Some(r) = gyro_rate(c)? {
            during = during.max(r);
        }
        if (!seen || !flag_usable) && elapsed >= opts.flag_window.max(opts.fixed_wait) {
            break AccCalibrationCompletion::FixedWait;
        }
        if seen && elapsed >= opts.max_wait {
            return Err(DeviceError::AccCalibrationFailed(format!(
                "the flight controller still reports calibrating after {} ms",
                opts.max_wait.as_millis()
            )));
        }
    };
    let duration_ms = started.elapsed().as_millis() as u32;

    if during > opts.motion_limit_dps {
        return Err(DeviceError::BoardMoving {
            max_rate_dps: during,
            limit_dps: opts.motion_limit_dps,
            during: true,
        });
    }
    if acc_needs_calibration(c)? == Some(true) {
        return Err(DeviceError::AccCalibrationFailed(
            "the flight controller still reports that the accelerometer needs calibration".into(),
        ));
    }
    Ok(AccCalibrationResult {
        duration_ms,
        completion,
        max_gyro_rate_dps: before.max(during),
    })
}

/// A simulated FC sensor stack on top of the motor-test [`FakeFc`](super::motor::fake::FakeFc).
#[cfg(test)]
pub(crate) mod sim {
    use super::*;
    use crate::device::motor::fake::{FakeFc, HookReply};
    use std::sync::{Arc, Mutex};

    pub struct Sim {
        pub armed: bool,
        /// MSP_STATUS sensor bits.
        pub sensors: u16,
        /// MSP_SENSOR_CONFIG `[acc, baro, mag]`.
        pub config: [u8; 3],
        /// MSP2_SENSOR_CONFIG_ACTIVE; `None` = error frame (Betaflight 4.4).
        pub active: Option<[u8; 4]>,
        /// STATUS_EX answered (else error frame, STATUS fallback).
        pub status_ex: bool,
        /// Gyro rates (deg/s) reported by MSP_RAW_IMU.
        pub gyro: [i16; 3],
        /// Gyro rates while calibrating (if different).
        pub gyro_while_calibrating: Option<[i16; 3]>,
        /// How long a calibration keeps the CALIBRATING flag set.
        pub calibration: Duration,
        /// Something else (a baro) keeps the CALIBRATING flag set all the time.
        pub other_calibrating: bool,
        /// Whether a finished calibration clears "acc needs calibration".
        pub calibration_succeeds: bool,
        /// MSP_ACC_CALIBRATION answered with an error frame (no USE_ACC).
        pub acc_cal_unsupported: bool,
        pub acc_calibrated: bool,
        pub calibrating_since: Option<Instant>,
        pub calibration_requests: u32,
        /// While true, STATUS_EX is not answered (EEPROM write).
        pub status_silent: bool,
    }

    impl Default for Sim {
        fn default() -> Self {
            Sim {
                armed: false,
                sensors: sensor_bits::GYRO | sensor_bits::ACC | sensor_bits::BARO,
                config: [0, 0, 1],
                active: Some([17, 18, 8, 1]), // BMI270 / BMI270 / DPS310 / none (4.5)
                status_ex: true,
                gyro: [1, -2, 0],
                gyro_while_calibrating: None,
                calibration: Duration::from_millis(60),
                other_calibrating: false,
                calibration_succeeds: true,
                acc_cal_unsupported: false,
                acc_calibrated: false,
                calibrating_since: None,
                calibration_requests: 0,
                status_silent: false,
            }
        }
    }

    impl Sim {
        fn calibrating(&mut self) -> bool {
            if let Some(t) = self.calibrating_since {
                if t.elapsed() < self.calibration {
                    return true;
                }
                self.calibrating_since = None;
                if self.calibration_succeeds {
                    self.acc_calibrated = true;
                }
            }
            false
        }

        fn status(&mut self, ex: bool) -> Vec<u8> {
            let calibrating = self.calibrating() || self.other_calibrating;
            let mut p = Vec::new();
            p.extend(125u16.to_le_bytes());
            p.extend(0u16.to_le_bytes());
            p.extend(self.sensors.to_le_bytes());
            p.extend((self.armed as u32).to_le_bytes());
            p.push(0);
            if !ex {
                return p;
            }
            p.extend(12u16.to_le_bytes());
            p.extend([4, 0, 0]); // profiles, rate profile, no extra mode bytes
            p.push(25);
            let mut flags = if self.armed { 0 } else { 1u32 << 21 };
            if calibrating {
                flags |= ARMING_DISABLED_CALIBRATING;
            }
            p.extend(flags.to_le_bytes());
            p.push(0);
            p
        }

        fn board_info(&mut self) -> Vec<u8> {
            self.calibrating();
            let mut p = b"S405".to_vec();
            p.extend([0, 0, 0, 0, 0, 0, 0]);
            p.extend([0u8; 32]);
            p.extend([1, 0]); // mcu, configuration state
            p.extend(1000u16.to_le_bytes());
            p.extend((!self.acc_calibrated as u32).to_le_bytes());
            p
        }

        fn raw_imu(&mut self) -> Vec<u8> {
            let g = match self.gyro_while_calibrating {
                Some(g) if self.calibrating_since.is_some() => g,
                _ => self.gyro,
            };
            let mut p = vec![0u8; 6];
            for v in g {
                p.extend(v.to_le_bytes());
            }
            p.extend([0u8; 6]);
            p
        }

        fn handle(&mut self, code: u16) -> Option<HookReply> {
            let ok = |p: Vec<u8>| Some(HookReply::Ok(p));
            match code {
                codes::MSP_STATUS_EX if self.status_silent => Some(HookReply::Silent),
                codes::MSP_STATUS_EX if !self.status_ex => Some(HookReply::Error),
                codes::MSP_STATUS_EX => ok(self.status(true)),
                codes::MSP_STATUS => ok(self.status(false)),
                codes::MSP_SENSOR_CONFIG => ok(self.config.to_vec()),
                codes::MSP2_SENSOR_CONFIG_ACTIVE => match self.active {
                    Some(a) => ok([a.as_slice(), &[0xFF, 0xFF]].concat()),
                    None => Some(HookReply::Error),
                },
                codes::MSP_BOARD_ALIGNMENT_CONFIG => {
                    ok([0i16, 0, 45].iter().flat_map(|v| v.to_le_bytes()).collect())
                }
                codes::MSP_SENSOR_ALIGNMENT => ok(vec![3, 3, 0]),
                codes::MSP_BOARD_INFO => ok(self.board_info()),
                codes::MSP_RAW_IMU => ok(self.raw_imu()),
                codes::MSP_ATTITUDE => ok([-125i16, 47, 271]
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect()),
                codes::MSP_ACC_CALIBRATION if self.acc_cal_unsupported => Some(HookReply::Error),
                codes::MSP_ACC_CALIBRATION => {
                    self.calibration_requests += 1;
                    // Betaflight ignores the request while armed (msp.c: `if (!ARMING_FLAG(ARMED))`).
                    if !self.armed {
                        self.calibrating_since = Some(Instant::now());
                    }
                    ok(vec![])
                }
                _ => None,
            }
        }
    }

    /// A FakeFc whose sensor commands are answered from the returned [`Sim`].
    pub fn fake(sim: Sim) -> (FakeFc, Arc<Mutex<Sim>>) {
        let fc = FakeFc::default();
        let sim = Arc::new(Mutex::new(sim));
        let s2 = sim.clone();
        *fc.msp_hook.lock().unwrap() =
            Some(Box::new(move |code, _| s2.lock().unwrap().handle(code)));
        (fc, sim)
    }

    /// Short waits for tests.
    pub fn fast() -> AccCalibrationOptions {
        AccCalibrationOptions {
            motion_samples: 3,
            motion_sample_interval: Duration::from_millis(2),
            motion_limit_dps: 8,
            poll_interval: Duration::from_millis(10),
            flag_window: Duration::from_millis(150),
            fixed_wait: Duration::from_millis(200),
            max_wait: Duration::from_millis(400),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::sim::*;
    use super::*;
    use crate::device::mock::MockSerial;
    use crate::error::AppError;

    fn client(fc: &crate::device::motor::fake::FakeFc) -> MspClient<MockSerial> {
        let mut c = MspClient::new(fc.serial());
        c.timeout = Duration::from_millis(50);
        c
    }

    fn le(v: &[i16]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    #[test]
    fn parses_payloads() {
        assert_eq!(
            parse_attitude(&le(&[-1795, 899, 359])).unwrap(),
            (-179.5, 89.9, 359.0)
        );
        assert!(parse_attitude(&[0; 5]).is_err());
        let mut imu = le(&[512, 0, 512]);
        imu.extend(le(&[-300, 7, 2]));
        imu.extend(le(&[0, 0, 0]));
        assert_eq!(parse_raw_imu_gyro(&imu).unwrap(), [-300, 7, 2]);
        assert_eq!(max_rate([-300, 7, 2]), 300);
        assert_eq!(max_rate([i16::MIN, 0, 0]), 32768);
        assert_eq!(parse_sensor_config(&[0, 1, 5, 0, 0]).unwrap(), [0, 1, 5]);
        assert!(parse_sensor_config(&[0, 1]).is_err());
        assert_eq!(
            parse_sensor_config_active(&[17, 18, 8, 0xFF, 0xFF, 0xFF]).unwrap(),
            [17, 18, 8, 0xFF]
        );
        assert_eq!(
            parse_board_alignment(&le(&[-10, 0, 180])).unwrap(),
            [-10, 0, 180]
        );
        assert_eq!(parse_sensor_alignment(&[8, 8, 0]).unwrap(), 8);
    }

    #[test]
    fn names_follow_the_firmware_tables() {
        use SensorKind::*;
        let n = |k, api, id| hardware_name(k, api, id).unwrap();
        assert_eq!(n(Acc, 45, 18), "BMI270");
        assert_eq!(n(Acc, 46, 18), "BMI270");
        assert_eq!(n(Acc, 47, 14), "BMI270"); // 2025.12 dropped the old I2C accs
        assert_eq!(n(Gyro, 45, 17), "BMI270");
        assert_eq!(n(Gyro, 47, 15), "BMI270");
        assert_eq!(n(Gyro, 47, 20), "ICM45686");
        assert_eq!(n(Gyro, 45, 18), "LSM6SDO");
        assert_eq!(n(Baro, 45, 8), "DPS310");
        assert_eq!(n(Baro, 47, 10), "LPS22DF");
        assert_eq!(n(Mag, 45, 6), "LIS3MDL");
        assert_eq!(n(Mag, 46, 6), "LIS2MDL");
        assert_eq!(n(Mag, 47, 1), "NONE");
        assert_eq!(n(Mag, 47, 99), "UNKNOWN_99");
        // 2026.6 (API 1.48): new parts appended before VIRTUAL, which therefore moved.
        assert_eq!(n(Gyro, 48, 15), "BMI270");
        assert_eq!(n(Gyro, 48, 26), "ICM56686");
        assert_eq!(n(Gyro, 48, 27), "VIRTUAL");
        assert_eq!(n(Acc, 48, 25), "ICM56686");
        assert_eq!(n(Acc, 48, 26), "VIRTUAL");
        assert_eq!(n(Baro, 48, 13), "VIRTUAL");
        assert_eq!(n(Baro, 48, 14), "SPA06_003");
        assert_eq!(n(Mag, 48, 5), "QMC5883L");
        assert_eq!(n(Mag, 48, 11), "QMC5883P");
        assert_eq!(n(Mag, 48, 12), "DRONECAN");
        assert_eq!(n(Mag, 49, 12), "DRONECAN"); // newer API: latest table
        assert_eq!(detected_name(Mag, 46, 1), None);
        assert_eq!(detected_name(Mag, 46, 0xFF), None);
        assert_eq!(alignment_name(3), "CW180");
        assert_eq!(alignment_name(9), "CUSTOM");
        assert_eq!(alignment_name(42), "UNKNOWN_42");
    }

    #[test]
    fn reads_sensor_status() {
        let (fc, _) = fake(Sim::default());
        let s = read_sensors(&mut client(&fc)).unwrap();
        assert_eq!(
            s.gyro,
            SensorState {
                present: true,
                enabled: true,
                configured_hardware: None,
                detected_hardware: Some("BMI270".into()),
            }
        );
        assert_eq!(
            s.acc,
            SensorState {
                present: true,
                enabled: true,
                configured_hardware: Some("AUTO".into()),
                detected_hardware: Some("BMI270".into()),
            }
        );
        assert!(s.baro.present && s.baro.enabled);
        assert_eq!(s.baro.detected_hardware.as_deref(), Some("DPS310"));
        assert_eq!(
            s.mag,
            SensorState {
                present: false,
                enabled: false,
                configured_hardware: Some("NONE".into()),
                detected_hardware: None,
            }
        );
        assert_eq!(s.acc_calibrated, Some(false));
        assert_eq!(
            s.alignment,
            Some(SensorAlignment {
                board_roll_deg: 0,
                board_pitch_deg: 0,
                board_yaw_deg: 45,
                gyro_alignment: "CW180".into(),
            })
        );
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["acc"]["configuredHardware"], "AUTO");
        assert_eq!(json["alignment"]["boardYawDeg"], 45);
        assert_eq!(json["accCalibrated"], false);
    }

    #[test]
    fn sensor_status_on_4_4_without_status_ex_or_active_ids() {
        let (fc, _) = fake(Sim {
            status_ex: false,
            active: None,
            sensors: sensor_bits::GYRO,
            config: [1, 0, 0],
            ..Default::default()
        });
        let s = read_sensors(&mut client(&fc)).unwrap();
        assert!(s.gyro.present);
        assert_eq!(s.gyro.detected_hardware, None);
        assert!(!s.acc.present && !s.acc.enabled);
        assert_eq!(s.acc.configured_hardware.as_deref(), Some("NONE"));
        assert!(!s.baro.present && s.baro.enabled);
        assert_eq!(s.acc_calibrated, None);
    }

    fn calibrate(
        sim: Sim,
    ) -> (
        Result<AccCalibrationResult>,
        std::sync::Arc<std::sync::Mutex<Sim>>,
    ) {
        let (fc, sim) = fake(sim);
        let r = acc_calibrate(&mut client(&fc), &fast());
        (r, sim)
    }

    #[test]
    fn calibration_completes_on_the_calibrating_flag() {
        let (r, sim) = calibrate(Sim::default());
        let r = r.unwrap();
        assert_eq!(r.completion, AccCalibrationCompletion::CalibratingFlag);
        assert!(r.duration_ms >= 60, "{r:?}");
        assert_eq!(r.max_gyro_rate_dps, 2);
        let s = sim.lock().unwrap();
        assert_eq!(s.calibration_requests, 1);
        assert!(s.acc_calibrated);
    }

    #[test]
    fn calibration_falls_back_to_a_fixed_wait() {
        // No STATUS_EX (no arming-disable flags) ...
        let (r, _) = calibrate(Sim {
            status_ex: false,
            ..Default::default()
        });
        let r = r.unwrap();
        assert_eq!(r.completion, AccCalibrationCompletion::FixedWait);
        assert!(r.duration_ms >= 200, "{r:?}");
        // ... or the flag already set by another sensor.
        let (r, _) = calibrate(Sim {
            other_calibrating: true,
            ..Default::default()
        });
        assert_eq!(r.unwrap().completion, AccCalibrationCompletion::FixedWait);
        // ... or the flag never seen (too short to catch).
        let (r, _) = calibrate(Sim {
            calibration: Duration::ZERO,
            ..Default::default()
        });
        assert_eq!(r.unwrap().completion, AccCalibrationCompletion::FixedWait);
    }

    #[test]
    fn calibration_tolerates_an_unanswered_status_poll() {
        let (fc, sim) = fake(Sim {
            calibration: Duration::from_millis(80),
            ..Default::default()
        });
        let s2 = sim.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            s2.lock().unwrap().status_silent = true;
            std::thread::sleep(Duration::from_millis(60));
            s2.lock().unwrap().status_silent = false;
        });
        let r = acc_calibrate(&mut client(&fc), &fast()).unwrap();
        t.join().unwrap();
        assert_eq!(r.completion, AccCalibrationCompletion::CalibratingFlag);
    }

    #[test]
    fn calibration_refuses_when_armed() {
        let (r, sim) = calibrate(Sim {
            armed: true,
            ..Default::default()
        });
        assert_eq!(AppError::from(r.unwrap_err()).code(), "BOARD_ARMED");
        assert_eq!(sim.lock().unwrap().calibration_requests, 0, "nothing sent");
    }

    #[test]
    fn calibration_refuses_without_a_usable_acc() {
        let code = |sim| {
            let (r, s) = calibrate(sim);
            assert_eq!(s.lock().unwrap().calibration_requests, 0, "nothing sent");
            AppError::from(r.unwrap_err())
        };
        let e = code(Sim {
            sensors: sensor_bits::GYRO,
            ..Default::default()
        });
        assert_eq!(e.code(), "ACC_NOT_AVAILABLE");
        assert_eq!(
            e.details().unwrap(),
            serde_json::json!({"present": false, "enabled": true})
        );
        let e = code(Sim {
            sensors: sensor_bits::GYRO,
            config: [1, 0, 0],
            ..Default::default()
        });
        assert_eq!(
            e.details().unwrap(),
            serde_json::json!({"present": false, "enabled": false})
        );
        // Built without USE_ACC: the FC rejects the command.
        let (r, _) = calibrate(Sim {
            acc_cal_unsupported: true,
            ..Default::default()
        });
        assert_eq!(AppError::from(r.unwrap_err()).code(), "ACC_NOT_AVAILABLE");
    }

    #[test]
    fn calibration_refuses_a_moving_board() {
        let (r, sim) = calibrate(Sim {
            gyro: [3, 40, -1],
            ..Default::default()
        });
        let e = AppError::from(r.unwrap_err());
        assert_eq!(e.code(), "BOARD_MOVING");
        assert_eq!(
            e.details().unwrap(),
            serde_json::json!({"maxRateDps": 40, "limitDps": 8, "during": false})
        );
        assert_eq!(sim.lock().unwrap().calibration_requests, 0);

        let (r, sim) = calibrate(Sim {
            gyro_while_calibrating: Some([0, 0, -25]),
            calibration: Duration::from_millis(80),
            ..Default::default()
        });
        let e = AppError::from(r.unwrap_err());
        assert_eq!(
            e.details().unwrap(),
            serde_json::json!({"maxRateDps": 25, "limitDps": 8, "during": true})
        );
        assert_eq!(sim.lock().unwrap().calibration_requests, 1);
    }

    #[test]
    fn calibration_reports_a_failed_result() {
        let (r, _) = calibrate(Sim {
            calibration_succeeds: false,
            ..Default::default()
        });
        assert_eq!(
            AppError::from(r.unwrap_err()).code(),
            "ACC_CALIBRATION_FAILED"
        );
        let (r, _) = calibrate(Sim {
            calibration: Duration::from_secs(5),
            ..Default::default()
        });
        let e = AppError::from(r.unwrap_err());
        assert_eq!(e.code(), "ACC_CALIBRATION_FAILED");
        assert!(e.to_string().contains("still reports calibrating"));
    }

    #[test]
    fn one_shot_attitude() {
        let (fc, _) = fake(Sim::default());
        let a = read_attitude(&mut client(&fc)).unwrap();
        assert_eq!((a.seq, a.roll, a.pitch, a.yaw), (0, -12.5, 4.7, 271.0));
    }
}
