// SPDX-License-Identifier: GPL-3.0-or-later
//! Bench motor test with a hard safety gate and a host-side watchdog.
//!
//! Wire behaviour follows Betaflight Configurator's motor tab (`useMotorTesting.ts`,
//! `useMotorStopValue.ts`, `MotorsTab.vue`, GPL-3.0-or-later): MSP_SET_MOTOR (214) carries
//! 8 x u16 LE values in the 1000-2000 range; "stop" is 1000 for DShot/ProShot, `mincommand`
//! for analog protocols; armed = bit 0 of the MSP_STATUS mode flags (BOXARM is always the
//! first box).
//!
//! What this module adds on top (the configurator relies on the user and a key-press handler):
//! * refuse unless the caller passes `props_off_confirmed = true`;
//! * refuse throttle above a low cap ([`DEFAULT_MAX_TEST_THROTTLE`], never above
//!   [`HARD_MAX_TEST_THROTTLE`]) or below the stop value; refuse 3D mode and disabled outputs;
//! * refuse when the FC reports ARMED, re-checked on every set and periodically while running;
//! * auto-stop (all motors to the stop value) on `stop()`, on drop, when the watchdog sees no
//!   set/keepalive within [`DEFAULT_WATCHDOG`], and when the worker's handle is dropped.
//!
//! Betaflight itself has no MSP motor timeout: if the USB link dies, the last MSP_SET_MOTOR value
//! stays applied until the FC is power-cycled. The test needs the battery connected (ESCs need
//! power), so the host can't fix that: the low cap, the disarmed check, the watchdog and the
//! emergency stop limit the risk, and the UI must confirm props removed, battery connected and
//! hands clear before passing `props_off_confirmed = true`. See docs/ARCHITECTURE.md §4.2.

use crate::device::error::{DeviceError, MotorTestRefusal, Result};
use crate::device::msp::{codes, MspClient, MspFrame, MspVersion, Reader};
use crate::device::serial::SerialLink;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, TryLockError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// MSP_SET_MOTOR always carries 8 motors.
pub const MAX_MOTORS: usize = 8;
/// Default throttle cap: 1150 is 15 % of the DShot range (1000 = stop, 2000 = full), enough to
/// see direction on any ESC, low enough to be survivable if something goes wrong.
pub const DEFAULT_MAX_TEST_THROTTLE: u16 = 1150;
/// Absolute ceiling; [`MotorTestLimits`] are clamped to it no matter what the caller passes.
pub const HARD_MAX_TEST_THROTTLE: u16 = 1250;
/// Stop all motors if no set/keepalive arrives within this window. The UI should send
/// `motor_test_keepalive` every [`RECOMMENDED_KEEPALIVE`] while a test runs.
pub const DEFAULT_WATCHDOG: Duration = Duration::from_millis(500);
pub const RECOMMENDED_KEEPALIVE: Duration = Duration::from_millis(200);
pub const MIN_WATCHDOG: Duration = Duration::from_millis(100);
pub const MAX_WATCHDOG: Duration = Duration::from_millis(2000);
/// While running, re-read MSP_STATUS this often and stop if the FC became armed.
pub const ARMED_RECHECK: Duration = Duration::from_millis(250);

/// Betaflight `motorPwmProtocolTypes_e` (MSP_ADVANCED_CONFIG byte 3).
pub mod protocol {
    pub const PWM: u8 = 0;
    pub const ONESHOT125: u8 = 1;
    pub const ONESHOT42: u8 = 2;
    pub const MULTISHOT: u8 = 3;
    pub const BRUSHED: u8 = 4;
    pub const DSHOT150: u8 = 5;
    pub const DSHOT300: u8 = 6;
    pub const DSHOT600: u8 = 7;
    pub const PROSHOT1000: u8 = 8;
    pub const DISABLED: u8 = 9;
}

/// Betaflight FEATURE_3D bit in MSP_FEATURE_CONFIG.
pub const FEATURE_3D_BIT: u32 = 12;

/// Monotonic time source (injectable for tests).
pub trait Clock: Send {
    fn now(&self) -> Duration;
}

/// Real monotonic clock.
pub struct SystemClock(Instant);

impl Default for SystemClock {
    fn default() -> Self {
        SystemClock(Instant::now())
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

/// Hand-driven clock for tests. Clones share the same time.
#[derive(Clone, Default)]
pub struct ManualClock(Arc<AtomicU64>);

impl ManualClock {
    pub fn advance(&self, d: Duration) {
        self.0.fetch_add(d.as_millis() as u64, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}

/// MSP_STATUS / MSP_STATUS_EX, the parts we need.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FcStatus {
    pub cycle_time_us: u16,
    pub i2c_errors: u16,
    pub sensors: u16,
    /// Active-box bit set in MSP_BOXIDS order; bit 0 is BOXARM.
    pub mode_flags: u32,
    pub pid_profile: u8,
    /// STATUS_EX only.
    pub cpu_load: Option<u16>,
    pub arming_disable_flags: Option<u32>,
}

impl FcStatus {
    pub fn is_armed(&self) -> bool {
        self.mode_flags & 1 != 0
    }
}

pub fn parse_status(p: &[u8]) -> Result<FcStatus> {
    let mut r = Reader::new(codes::MSP_STATUS, p);
    Ok(FcStatus {
        cycle_time_us: r.u16()?,
        i2c_errors: r.u16()?,
        sensors: r.u16()?,
        mode_flags: r.u32()?,
        pid_profile: r.u8()?,
        cpu_load: None,
        arming_disable_flags: None,
    })
}

/// MSP_STATUS_EX (MSPHelper layout): STATUS fields, cpuload u16, numProfiles u8, rateProfile u8,
/// extra flight-mode byte count + bytes, arming-disable count u8 + flags u32, ...
pub fn parse_status_ex(p: &[u8]) -> Result<FcStatus> {
    let mut r = Reader::new(codes::MSP_STATUS_EX, p);
    let mut s = FcStatus {
        cycle_time_us: r.u16()?,
        i2c_errors: r.u16()?,
        sensors: r.u16()?,
        mode_flags: r.u32()?,
        pid_profile: r.u8()?,
        ..Default::default()
    };
    s.cpu_load = Some(r.u16()?);
    let _num_profiles = r.u8()?;
    let _rate_profile = r.u8()?;
    let extra = r.u8()? as usize;
    r.bytes(extra)?;
    if r.remaining() >= 5 {
        let _count = r.u8()?;
        s.arming_disable_flags = Some(r.u32()?);
    }
    Ok(s)
}

/// How the FC drives its motors (decides the stop value and whether we test at all).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MotorOutputConfig {
    pub protocol: u8,
    pub digital: bool,
    pub feature_3d: bool,
    pub mincommand: u16,
    pub motor_count: u8,
    /// Value that means "motor stopped" for this config.
    pub stop_value: u16,
}

impl MotorOutputConfig {
    /// From raw MSP_ADVANCED_CONFIG, MSP_FEATURE_CONFIG and MSP_MOTOR_CONFIG payloads.
    pub fn from_payloads(advanced: &[u8], features: &[u8], motor: &[u8]) -> Result<Self> {
        let mut a = Reader::new(codes::MSP_ADVANCED_CONFIG, advanced);
        a.bytes(3)?; // gyro_sync_denom, pid_process_denom, use_unsyncedPwm
        let protocol = a.u8()?;
        let mut f = Reader::new(codes::MSP_FEATURE_CONFIG, features);
        let mask = f.u32()?;
        let mut m = Reader::new(codes::MSP_MOTOR_CONFIG, motor);
        let _minthrottle = m.u16()?;
        let _maxthrottle = m.u16()?;
        let mincommand = m.u16()?;
        let motor_count = m.u8()?;
        Ok(Self::new(
            protocol,
            mask & (1 << FEATURE_3D_BIT) != 0,
            mincommand,
            motor_count,
        ))
    }

    pub fn new(protocol: u8, feature_3d: bool, mincommand: u16, motor_count: u8) -> Self {
        let digital = (protocol::DSHOT150..=protocol::PROSHOT1000).contains(&protocol);
        // computeZeroThrottleValue(): 3D -> 1500 (we refuse 3D anyway), digital -> 1000.
        let stop_value = if feature_3d {
            1500
        } else if digital {
            1000
        } else {
            mincommand
        };
        MotorOutputConfig {
            protocol,
            digital,
            feature_3d,
            mincommand,
            motor_count,
            stop_value,
        }
    }
}

/// Caller-tunable limits, always clamped to the hard bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MotorTestLimits {
    pub max_throttle: u16,
    #[serde(with = "millis")]
    pub watchdog: Duration,
}

impl Default for MotorTestLimits {
    fn default() -> Self {
        MotorTestLimits {
            max_throttle: DEFAULT_MAX_TEST_THROTTLE,
            watchdog: DEFAULT_WATCHDOG,
        }
    }
}

impl MotorTestLimits {
    pub fn clamped(self) -> Self {
        MotorTestLimits {
            max_throttle: self.max_throttle.min(HARD_MAX_TEST_THROTTLE),
            watchdog: self.watchdog.clamp(MIN_WATCHDOG, MAX_WATCHDOG),
        }
    }
}

mod millis {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;
    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_millis() as u64)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        Ok(Duration::from_millis(u64::deserialize(d)?))
    }
}

/// `motor_test_start` arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MotorTestRequest {
    /// 0-based motor; `None` = all motors.
    pub motor_index: Option<u8>,
    /// MSP motor value (1000-2000 scale, same as the configurator sliders).
    pub throttle: u16,
    pub props_off_confirmed: bool,
}

fn refuse(r: MotorTestRefusal) -> DeviceError {
    DeviceError::MotorTestRefused(r)
}

/// The safety gate, pure. Returns the 8 values to send.
pub fn check_gate(
    req: &MotorTestRequest,
    status: &FcStatus,
    out: &MotorOutputConfig,
    limits: &MotorTestLimits,
) -> Result<[u16; MAX_MOTORS]> {
    if !req.props_off_confirmed {
        return Err(refuse(MotorTestRefusal::PropsNotConfirmed));
    }
    if status.is_armed() {
        return Err(DeviceError::Armed);
    }
    if out.feature_3d {
        return Err(refuse(MotorTestRefusal::Mode3d));
    }
    if out.protocol == protocol::DISABLED || out.motor_count == 0 {
        return Err(refuse(MotorTestRefusal::MotorsDisabled));
    }
    let count = out.motor_count.min(MAX_MOTORS as u8);
    if let Some(i) = req.motor_index {
        if i >= count {
            return Err(refuse(MotorTestRefusal::InvalidMotor { index: i, count }));
        }
    }
    let max = limits.clamped().max_throttle;
    if req.throttle < out.stop_value {
        return Err(refuse(MotorTestRefusal::ThrottleBelowStop {
            requested: req.throttle,
            stop: out.stop_value,
        }));
    }
    if req.throttle > max {
        return Err(refuse(MotorTestRefusal::ThrottleAboveLimit {
            requested: req.throttle,
            max,
        }));
    }
    let mut v = [out.stop_value; MAX_MOTORS];
    match req.motor_index {
        Some(i) => v[i as usize] = req.throttle,
        None => v
            .iter_mut()
            .take(count as usize)
            .for_each(|x| *x = req.throttle),
    }
    Ok(v)
}

pub fn encode_set_motor(values: &[u16; MAX_MOTORS]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// Lock-free emergency stop for a running motor test (window close / app exit).
///
/// [`EmergencyStop::trigger`] never waits for the motor-test thread or the connection
/// manager: it sets the `halt` flag (checked by the worker before every MSP_SET_MOTOR, by
/// keepalive and by every loop iteration) and writes an all-stop MSP_SET_MOTOR frame
/// straight to a duplicated port handle. The worker then sends stop again through its own
/// link as soon as it gets control back, so if the direct write fails the motors stop
/// within one worker iteration after the current MSP exchange returns.
pub struct EmergencyStop {
    halt: AtomicBool,
    /// Set when the `MotorTest` is dropped (session over, link handed back or lost).
    finished: AtomicBool,
    /// Duplicated write handle; only ever touched by `trigger` (via `try_lock`).
    writer: Mutex<Option<Box<dyn std::io::Write + Send>>>,
    /// Pre-encoded MSP_SET_MOTOR with every motor at the stop value.
    frame: Vec<u8>,
}

/// What [`EmergencyStop::trigger`] managed to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmergencyOutcome {
    /// The stop frame was written through the duplicated handle.
    pub written: bool,
    /// The halt flag was already set (second trigger).
    pub already_halted: bool,
}

impl EmergencyStop {
    pub fn new(writer: Option<Box<dyn std::io::Write + Send>>, stop_value: u16) -> Self {
        let frame = MspFrame::request(
            MspVersion::V1,
            codes::MSP_SET_MOTOR,
            &encode_set_motor(&[stop_value; MAX_MOTORS]),
        )
        .encode()
        .unwrap_or_default();
        EmergencyStop {
            halt: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            writer: Mutex::new(writer),
            frame,
        }
    }

    pub fn is_halted(&self) -> bool {
        self.halt.load(Ordering::SeqCst)
    }

    /// The motor-test session this belongs to has ended.
    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::SeqCst)
    }

    /// Session over: close the duplicated handle so the port can be reopened.
    fn release(&self) {
        self.finished.store(true, Ordering::SeqCst);
        let mut w = self.writer.lock().unwrap_or_else(|p| p.into_inner());
        *w = None;
    }

    /// Halt the test and write stop directly. Never blocks.
    pub fn trigger(&self) -> EmergencyOutcome {
        let already_halted = self.halt.swap(true, Ordering::SeqCst);
        let written = match self.writer.try_lock() {
            Ok(mut w) => Self::write(&mut w, &self.frame),
            Err(TryLockError::Poisoned(p)) => Self::write(&mut p.into_inner(), &self.frame),
            // Another trigger is writing right now; it sends the same frame.
            Err(TryLockError::WouldBlock) => false,
        };
        EmergencyOutcome {
            written,
            already_halted,
        }
    }

    fn write(w: &mut Option<Box<dyn std::io::Write + Send>>, frame: &[u8]) -> bool {
        let Some(w) = w.as_mut() else { return false };
        // Twice, like `MotorTest::stop`: MSP_SET_MOTOR is fire-and-forget.
        let r = w
            .write_all(frame)
            .and_then(|_| w.write_all(frame))
            .and_then(|_| w.flush());
        if let Err(e) = &r {
            log::warn!("emergency stop: direct MSP_SET_MOTOR write failed: {e}");
        }
        r.is_ok()
    }
}

/// A motor-test session owning the MSP link. Motors are stopped on `stop()`, on watchdog
/// expiry (`tick()`), on `into_inner()` and on drop.
pub struct MotorTest<S: SerialLink, C: Clock = SystemClock> {
    client: Option<MspClient<S>>,
    clock: C,
    limits: MotorTestLimits,
    output: MotorOutputConfig,
    values: [u16; MAX_MOTORS],
    active: bool,
    last_refresh: Duration,
    last_status_check: Duration,
    emergency: Arc<EmergencyStop>,
}

impl<S: SerialLink, C: Clock> MotorTest<S, C> {
    /// Read the FC's motor config and make sure motors are at stop. Nothing spins yet.
    /// On failure the link is handed back with the error, so the caller keeps the port.
    pub fn new(
        link: S,
        clock: C,
        limits: MotorTestLimits,
    ) -> std::result::Result<Self, (DeviceError, S)> {
        let writer = link.try_clone_writer();
        let mut client = MspClient::new(link);
        let read = |client: &mut MspClient<S>| -> Result<MotorOutputConfig> {
            let adv = client.request(codes::MSP_ADVANCED_CONFIG, &[])?;
            let feat = client.request(codes::MSP_FEATURE_CONFIG, &[])?;
            let mot = client.request(codes::MSP_MOTOR_CONFIG, &[])?;
            MotorOutputConfig::from_payloads(&adv, &feat, &mot)
        };
        let output = match read(&mut client) {
            Ok(o) => o,
            Err(e) => return Err((e, client.into_inner())),
        };
        let now = clock.now();
        let emergency = Arc::new(EmergencyStop::new(writer, output.stop_value));
        let mut t = MotorTest {
            client: Some(client),
            clock,
            limits: limits.clamped(),
            output: output.clone(),
            values: [output.stop_value; MAX_MOTORS],
            active: false,
            last_refresh: now,
            last_status_check: now,
            emergency,
        };
        if let Err(e) = t.send_values(t.values) {
            return Err((e, t.into_inner()));
        }
        Ok(t)
    }

    fn client(&mut self) -> &mut MspClient<S> {
        self.client
            .as_mut()
            .expect("client present until into_inner")
    }

    fn send_values(&mut self, v: [u16; MAX_MOTORS]) -> Result<()> {
        let stop = [self.output.stop_value; MAX_MOTORS];
        let v = if self.emergency.is_halted() { stop } else { v };
        let r = self
            .client()
            .send(codes::MSP_SET_MOTOR, &encode_set_motor(&v));
        if r.is_err() {
            self.active = false;
        }
        r
    }

    pub fn read_status(&mut self) -> Result<FcStatus> {
        match self.client().request(codes::MSP_STATUS_EX, &[]) {
            Ok(p) => parse_status_ex(&p),
            Err(DeviceError::MspUnsupported(_)) => {
                parse_status(&self.client().request(codes::MSP_STATUS, &[])?)
            }
            Err(e) => Err(e),
        }
    }

    pub fn output_config(&self) -> &MotorOutputConfig {
        &self.output
    }
    pub fn limits(&self) -> &MotorTestLimits {
        &self.limits
    }
    pub fn values(&self) -> [u16; MAX_MOTORS] {
        self.values
    }
    pub fn is_active(&self) -> bool {
        self.active
    }
    /// Shared emergency-stop handle (see [`EmergencyStop`]).
    pub fn emergency(&self) -> Arc<EmergencyStop> {
        self.emergency.clone()
    }

    /// Gate + spin. A refusal leaves (or puts) every motor at stop.
    pub fn set(&mut self, req: &MotorTestRequest) -> Result<()> {
        if self.emergency.is_halted() {
            let _ = self.stop();
            return Err(DeviceError::MotorTestNotActive);
        }
        let status = match self.read_status() {
            Ok(s) => s,
            Err(e) => {
                let _ = self.stop();
                return Err(e);
            }
        };
        let values = match check_gate(req, &status, &self.output, &self.limits) {
            Ok(v) => v,
            Err(e) => {
                let _ = self.stop();
                return Err(e);
            }
        };
        self.send_values(values)?;
        self.values = values;
        self.active = values.iter().any(|&v| v != self.output.stop_value);
        let now = self.clock.now();
        self.last_refresh = now;
        self.last_status_check = now;
        Ok(())
    }

    /// Feed the watchdog and re-send the current values.
    pub fn keepalive(&mut self) -> Result<()> {
        if self.emergency.is_halted() {
            if self.active {
                let _ = self.stop();
            }
            return Err(DeviceError::MotorTestNotActive);
        }
        if !self.active {
            return Err(DeviceError::MotorTestNotActive);
        }
        self.send_values(self.values)?;
        self.last_refresh = self.clock.now();
        Ok(())
    }

    /// All motors to the stop value (sent twice; MSP_SET_MOTOR is fire-and-forget).
    pub fn stop(&mut self) -> Result<()> {
        self.active = false;
        let stop = [self.output.stop_value; MAX_MOTORS];
        self.values = stop;
        let r1 = self.send_values(stop);
        let r2 = self.send_values(stop);
        r1.and(r2)
    }

    /// Call periodically. Stops on watchdog expiry or if the FC became armed.
    pub fn tick(&mut self) -> Result<Option<StopReason>> {
        if !self.active {
            return Ok(None);
        }
        let now = self.clock.now();
        if now.saturating_sub(self.last_refresh) > self.limits.watchdog {
            log::warn!("motor test watchdog expired; stopping motors");
            self.stop()?;
            return Ok(Some(StopReason::Watchdog));
        }
        if now.saturating_sub(self.last_status_check) >= ARMED_RECHECK {
            self.last_status_check = now;
            match self.read_status() {
                Ok(s) if s.is_armed() => {
                    self.stop()?;
                    return Ok(Some(StopReason::Armed));
                }
                Ok(_) => {}
                Err(e @ DeviceError::Disconnected) => {
                    self.active = false;
                    return Err(e);
                }
                Err(e) => {
                    let _ = self.stop();
                    return Err(e);
                }
            }
        }
        Ok(None)
    }

    /// Stop motors (best effort) and give the link back.
    pub fn into_inner(mut self) -> S {
        let _ = self.stop();
        self.client.take().expect("client present").into_inner()
    }
}

impl<S: SerialLink, C: Clock> Drop for MotorTest<S, C> {
    fn drop(&mut self) {
        if self.client.is_some() {
            let _ = self.stop();
        }
        self.emergency.release();
    }
}

/// Why a running test stopped (payload of the `device://motor-test-stopped` event).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    Requested,
    Watchdog,
    Armed,
    Disconnected,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MotorTestStopped {
    pub reason: StopReason,
    pub error: Option<serde_json::Value>,
}

enum Cmd {
    Set(MotorTestRequest, mpsc::Sender<Result<()>>),
    Keepalive(mpsc::Sender<Result<()>>),
    Stop(mpsc::Sender<Result<()>>),
}

/// Runs a [`MotorTest`] on its own thread so the watchdog fires even if the UI stalls.
/// Dropping the handle stops the motors.
pub struct MotorTestHandle<S: SerialLink + 'static> {
    tx: Option<mpsc::Sender<Cmd>>,
    join: Option<JoinHandle<Option<S>>>,
    emergency: Arc<EmergencyStop>,
}

impl<S: SerialLink + 'static> MotorTestHandle<S> {
    pub fn spawn<C: Clock + 'static>(
        test: MotorTest<S, C>,
        mut on_stopped: impl FnMut(MotorTestStopped) + Send + 'static,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<Cmd>();
        let emergency = test.emergency();
        let halt = emergency.clone();
        // Poll at least every 20 ms so an emergency halt is acted on promptly even when no
        // commands arrive; `tick()` is time-based, so polling more often is harmless.
        let tick = (test.limits.watchdog / 5)
            .max(Duration::from_millis(10))
            .min(Duration::from_millis(20));
        let join = std::thread::Builder::new()
            .name("motor-test".into())
            .spawn(move || {
                let mut test = test;
                let mut halt_handled = false;
                loop {
                    let active_before = test.is_active();
                    let mut emitted = false;
                    let mut emit = |ev: MotorTestStopped, emitted: &mut bool| {
                        *emitted = true;
                        on_stopped(ev);
                    };
                    match rx.recv_timeout(tick) {
                        Ok(cmd) => {
                            let (r, reply, requested_stop) = match cmd {
                                Cmd::Set(req, reply) => (test.set(&req), reply, false),
                                Cmd::Keepalive(reply) => (test.keepalive(), reply, false),
                                Cmd::Stop(reply) => {
                                    let was = test.is_active();
                                    (test.stop(), reply, was)
                                }
                            };
                            let fatal = matches!(r, Err(DeviceError::Disconnected));
                            let _ = reply.send(r);
                            if fatal {
                                emit(
                                    MotorTestStopped {
                                        reason: StopReason::Disconnected,
                                        error: None,
                                    },
                                    &mut emitted,
                                );
                                return None;
                            }
                            if requested_stop {
                                emit(
                                    MotorTestStopped {
                                        reason: StopReason::Requested,
                                        error: None,
                                    },
                                    &mut emitted,
                                );
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => match test.tick() {
                            Ok(Some(reason)) => emit(
                                MotorTestStopped {
                                    reason,
                                    error: None,
                                },
                                &mut emitted,
                            ),
                            Ok(None) => {}
                            Err(DeviceError::Disconnected) => {
                                emit(
                                    MotorTestStopped {
                                        reason: StopReason::Disconnected,
                                        error: None,
                                    },
                                    &mut emitted,
                                );
                                return None;
                            }
                            Err(e) => emit(
                                MotorTestStopped {
                                    reason: StopReason::Error,
                                    error: Some(crate::error::AppError::from(e).to_json()),
                                },
                                &mut emitted,
                            ),
                        },
                        // Handle dropped or finish(): stop and hand the link back.
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            return Some(test.into_inner())
                        }
                    }
                    // Emergency halt: stop through our own link too (the direct write is
                    // best effort) and report it once.
                    if !halt_handled && halt.is_halted() {
                        halt_handled = true;
                        // Active now, or deactivated by a halted keepalive this iteration.
                        let was_active = active_before || test.is_active();
                        let r = test.stop();
                        if was_active && !emitted {
                            emit(
                                MotorTestStopped {
                                    reason: StopReason::Requested,
                                    error: None,
                                },
                                &mut emitted,
                            );
                        }
                        if matches!(r, Err(DeviceError::Disconnected)) {
                            return None;
                        }
                    }
                }
            })
            .expect("spawn motor-test thread");
        MotorTestHandle {
            tx: Some(tx),
            join: Some(join),
            emergency,
        }
    }

    /// Lock-free emergency-stop handle for this test; usable without the handle itself.
    pub fn emergency(&self) -> Arc<EmergencyStop> {
        self.emergency.clone()
    }

    fn call(&self, make: impl FnOnce(mpsc::Sender<Result<()>>) -> Cmd) -> Result<()> {
        let (rtx, rrx) = mpsc::channel();
        let tx = self.tx.as_ref().ok_or(DeviceError::MotorTestNotActive)?;
        tx.send(make(rtx)).map_err(|_| DeviceError::Disconnected)?;
        rrx.recv().map_err(|_| DeviceError::Disconnected)?
    }

    pub fn set(&self, req: MotorTestRequest) -> Result<()> {
        self.call(|r| Cmd::Set(req, r))
    }
    pub fn keepalive(&self) -> Result<()> {
        self.call(Cmd::Keepalive)
    }
    pub fn stop(&self) -> Result<()> {
        self.call(Cmd::Stop)
    }

    /// Stop motors, end the worker and return the link (None if it disconnected).
    pub fn finish(mut self) -> Option<S> {
        self.tx.take();
        self.join.take().and_then(|j| j.join().ok().flatten())
    }
}

impl<S: SerialLink + 'static> Drop for MotorTestHandle<S> {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! A fake FC answering the motor-test MSP commands.
    use super::*;
    use crate::device::mock::MockSerial;
    use crate::device::msp::{MspEvent, MspFrame, MspParser};
    use std::sync::atomic::AtomicBool;
    use std::sync::Mutex;

    /// What a CLI command does on the fake FC (e.g. config's `MockFc`, which answers
    /// `version` / `dump all` / `diff all` from real dumps). Without one, every command just
    /// marks RAM dirty and prints a prompt.
    pub trait CliBackend: Send {
        /// `#` received: the CLI was entered.
        fn enter(&mut self) {}
        /// Output of one command (without echo and prompt).
        fn exec(&mut self, line: &str) -> String;
        /// The FC reset: unsaved changes are gone.
        fn reboot(&mut self) {}
    }

    /// What an [`MspHook`] answers.
    pub enum HookReply {
        /// A normal response with this payload.
        Ok(Vec<u8>),
        /// An MSP error frame (command unknown to / refused by the firmware).
        Error,
        /// No reply at all (the request times out).
        Silent,
    }

    /// Extra MSP behaviour for one test: called with (code, payload) before the built-in
    /// handlers; `None` falls through to them.
    pub type MspHook = Box<dyn FnMut(u16, &[u8]) -> Option<HookReply> + Send>;

    #[derive(Clone)]
    pub struct FakeFc {
        pub armed: Arc<AtomicBool>,
        pub motors: Arc<Mutex<Vec<[u16; 8]>>>,
        pub protocol: u8,
        pub features: u32,
        pub mincommand: u16,
        pub motor_count: u8,
        /// MSP_UID payload (12 bytes).
        pub uid: Arc<Mutex<[u8; 12]>>,
        /// While set, writes on links from `serial()` block (see `MockSerial::wedge`).
        pub wedge: Arc<AtomicBool>,
        /// A CLI session is active: input is echoed and parsed as CLI lines, MSP is ignored.
        pub cli: Arc<AtomicBool>,
        /// Plain `exit` reboots, as in every Betaflight release (4.4 `cliExit`, 4.5+
        /// `cliExitCmd` without `noreboot`). `false` models an exit that leaves the CLI but
        /// keeps RAM (what `exit noreboot` does on 4.5+ / 2025.12).
        pub exit_reboots: bool,
        /// Unsaved changes live in RAM (any CLI command other than `exit`); a reboot clears it.
        pub ram_dirty: Arc<AtomicBool>,
        /// MSP_REBOOT requests received (payloads).
        pub msp_reboots: Arc<Mutex<Vec<Vec<u8>>>>,
        /// `exit` does not leave the CLI (a board that never recovers).
        pub cli_sticky: bool,
        /// Every byte written to the FC.
        pub rx_log: Arc<Mutex<Vec<u8>>>,
        /// CLI lines received while in CLI mode.
        pub cli_lines: Arc<Mutex<Vec<String>>>,
        /// On every reboot: unplug this "present" flag for ~30 ms (USB VCP re-enumerating).
        /// `None` = UART bridge, the port never goes away.
        pub unplug_on_reboot: Arc<Mutex<Option<Arc<AtomicBool>>>>,
        /// How long a reboot keeps the USB port unplugged.
        pub reboot_ms: u64,
        /// MSP_API_VERSION minor: 45 = Betaflight 4.4 (no `exit noreboot`), 46 = 4.5,
        /// 47 = 2025.12.
        pub api_minor: u8,
        /// `#` enters the CLI and commands go here (see [`CliBackend`]).
        pub cli_backend: Arc<Mutex<Option<Box<dyn CliBackend>>>>,
        /// Reboots seen (`exit`, MSP_REBOOT, `bl`).
        pub reboots: Arc<std::sync::atomic::AtomicUsize>,
        /// See [`MspHook`].
        pub msp_hook: Arc<Mutex<Option<MspHook>>>,
    }

    impl Default for FakeFc {
        fn default() -> Self {
            FakeFc {
                armed: Arc::default(),
                motors: Arc::default(),
                protocol: protocol::DSHOT600,
                features: 0,
                mincommand: 1000,
                motor_count: 4,
                uid: Arc::new(Mutex::new([1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0])),
                wedge: Arc::default(),
                cli: Arc::default(),
                exit_reboots: true,
                ram_dirty: Arc::default(),
                msp_reboots: Arc::default(),
                cli_sticky: false,
                rx_log: Arc::default(),
                cli_lines: Arc::default(),
                unplug_on_reboot: Arc::default(),
                reboot_ms: 30,
                api_minor: 46,
                cli_backend: Arc::default(),
                reboots: Arc::default(),
                msp_hook: Arc::default(),
            }
        }
    }

    pub fn status_ex(armed: bool) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend(125u16.to_le_bytes());
        p.extend(0u16.to_le_bytes());
        p.extend(0x23u16.to_le_bytes());
        p.extend((armed as u32).to_le_bytes());
        p.push(0); // profile
        p.extend(12u16.to_le_bytes());
        p.extend([4, 0]); // num profiles, rate profile
        p.extend([1, 0]); // 1 extra flight-mode byte
        p.push(25);
        p.extend((if armed { 0 } else { 1u32 << 21 }).to_le_bytes());
        p.push(0);
        p.extend(40u16.to_le_bytes());
        p
    }

    impl FakeFc {
        /// FC reset: RAM changes are gone, the CLI is closed, a USB VCP re-enumerates.
        fn reboot(&self) {
            self.reboots.fetch_add(1, Ordering::SeqCst);
            self.ram_dirty.store(false, Ordering::SeqCst);
            self.cli.store(false, Ordering::SeqCst);
            if let Some(b) = self.cli_backend.lock().unwrap().as_mut() {
                b.reboot();
            }
            if let Some(p) = self.unplug_on_reboot.lock().unwrap().clone() {
                p.store(false, Ordering::SeqCst);
                let ms = self.reboot_ms;
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(ms));
                    p.store(true, Ordering::SeqCst);
                });
            }
        }

        pub fn serial(&self) -> MockSerial {
            let me = self.clone();
            let mut parser = MspParser::default();
            let mut line = Vec::<u8>::new();
            let mut link = MockSerial::new(move |w| {
                me.rx_log.lock().unwrap().extend_from_slice(w);
                let mut out = Vec::new();
                if me.cli.load(Ordering::SeqCst) {
                    // Betaflight CLI: echo, run complete lines, never answer MSP.
                    out.extend_from_slice(w);
                    for &b in w {
                        if b != b'\r' && b != b'\n' {
                            line.push(b);
                            continue;
                        }
                        if line.is_empty() {
                            continue;
                        }
                        let cmd = String::from_utf8_lossy(&line).trim().to_string();
                        line.clear();
                        me.cli_lines.lock().unwrap().push(cmd.clone());
                        match cmd.as_str() {
                            // Betaflight's output (cliExitCmd + cliRebootEx).
                            "exit" if !me.cli_sticky && me.exit_reboots => {
                                out.extend_from_slice(
                                    b"\r\n# leaving CLI mode, unsaved changes lost\r\n\r\nRebooting",
                                );
                                me.reboot();
                            }
                            "exit" if !me.cli_sticky => {
                                out.extend_from_slice(b"\r\n# leaving CLI mode, no reboot\r\n");
                                me.cli.store(false, Ordering::SeqCst);
                            }
                            "exit" => out.extend_from_slice(b"\r\n# "),
                            // 4.5+ `cliExitCmd`; 4.4 ignores the argument and reboots.
                            "exit noreboot" if me.api_minor >= 46 => {
                                out.extend_from_slice(b"\r\n# leaving CLI mode, no reboot\r\n");
                                me.cli.store(false, Ordering::SeqCst);
                            }
                            "exit noreboot" => {
                                out.extend_from_slice(
                                    b"\r\n# leaving CLI mode, unsaved changes lost\r\n\r\nRebooting",
                                );
                                me.reboot();
                            }
                            "bl" | "bl rom" => {
                                out.extend_from_slice(b"\r\nRebooting into bootloader");
                                me.reboot();
                            }
                            _ => {
                                let mut backend = me.cli_backend.lock().unwrap();
                                match backend.as_mut() {
                                    Some(b) => {
                                        let r = b.exec(&cmd);
                                        out.extend_from_slice(b"\r\n");
                                        out.extend_from_slice(r.replace('\n', "\r\n").as_bytes());
                                        out.extend_from_slice(b"\r\n\r\n# ");
                                    }
                                    None => {
                                        me.ram_dirty.store(true, Ordering::SeqCst);
                                        out.extend_from_slice(b"\r\n# ");
                                    }
                                }
                            }
                        }
                    }
                    return out;
                }
                if w == b"#" {
                    // MSP-mode FC receiving `#` outside a frame: enter the CLI (cliEnter).
                    me.cli.store(true, Ordering::SeqCst);
                    if let Some(b) = me.cli_backend.lock().unwrap().as_mut() {
                        b.enter();
                    }
                    out.extend_from_slice(
                        b"\r\nEntering CLI Mode, type 'exit' to reboot, or 'help'\r\n\r\n# ",
                    );
                    return out;
                }
                for ev in parser.feed(w) {
                    let MspEvent::Frame(f) = ev else { continue };
                    let hooked = me
                        .msp_hook
                        .lock()
                        .unwrap()
                        .as_mut()
                        .and_then(|h| h(f.code, &f.payload));
                    match hooked {
                        Some(HookReply::Ok(p)) => {
                            out.extend(MspFrame::response(f.version, f.code, &p).encode().unwrap());
                            continue;
                        }
                        Some(HookReply::Error) => {
                            let mut e = MspFrame::response(f.version, f.code, &[]);
                            e.direction = crate::device::msp::Direction::Error;
                            out.extend(e.encode().unwrap());
                            continue;
                        }
                        Some(HookReply::Silent) => continue,
                        None => {}
                    }
                    let reply: Option<Vec<u8>> = match f.code {
                        codes::MSP_API_VERSION => Some(vec![0, 1, me.api_minor]),
                        codes::MSP_FC_VARIANT => Some(b"BTFL".to_vec()),
                        codes::MSP_FC_VERSION => Some(vec![4, 5, 5]),
                        codes::MSP_BOARD_INFO => {
                            let mut p = b"S405".to_vec();
                            p.extend([0, 0, 0, 0]); // version, type, capabilities
                            p.extend([0, 0, 0]); // target, board, manufacturer (empty)
                            p.extend([0u8; 32]); // signature
                            p.push(1); // F40X
                            Some(p)
                        }
                        // identify() extras: a too-short payload reads as "unknown".
                        codes::MSP2_COMMON_SERIAL_CONFIG | codes::MSP_RX_CONFIG => Some(vec![]),
                        codes::MSP_RC => {
                            Some([1500u16; 8].iter().flat_map(|v| v.to_le_bytes()).collect())
                        }
                        codes::MSP_REBOOT => {
                            me.msp_reboots.lock().unwrap().push(f.payload.clone());
                            me.reboot();
                            Some(vec![f.payload.first().copied().unwrap_or(0)])
                        }
                        codes::MSP_UID => Some(me.uid.lock().unwrap().to_vec()),
                        codes::MSP_ADVANCED_CONFIG => Some(vec![1, 1, 0, me.protocol, 0, 0]),
                        codes::MSP_FEATURE_CONFIG => Some(me.features.to_le_bytes().to_vec()),
                        codes::MSP_MOTOR_CONFIG => {
                            let mut p = Vec::new();
                            p.extend(1070u16.to_le_bytes());
                            p.extend(2000u16.to_le_bytes());
                            p.extend(me.mincommand.to_le_bytes());
                            p.extend([me.motor_count, 14, 1, 0]);
                            Some(p)
                        }
                        codes::MSP_STATUS_EX => Some(status_ex(me.armed.load(Ordering::SeqCst))),
                        codes::MSP_SET_MOTOR => {
                            let mut v = [0u16; 8];
                            for (i, c) in f.payload.chunks(2).enumerate().take(8) {
                                v[i] = u16::from_le_bytes([c[0], c[1]]);
                            }
                            me.motors.lock().unwrap().push(v);
                            Some(vec![])
                        }
                        _ => None,
                    };
                    if let Some(p) = reply {
                        out.extend(MspFrame::response(f.version, f.code, &p).encode().unwrap());
                    }
                }
                out
            });
            link.wedge = Some(self.wedge.clone());
            link
        }

        pub fn last(&self) -> [u16; 8] {
            *self
                .motors
                .lock()
                .unwrap()
                .last()
                .expect("a SET_MOTOR was sent")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::*;
    use super::*;

    fn req(i: Option<u8>, t: u16, ok: bool) -> MotorTestRequest {
        MotorTestRequest {
            motor_index: i,
            throttle: t,
            props_off_confirmed: ok,
        }
    }

    fn out() -> MotorOutputConfig {
        MotorOutputConfig::new(protocol::DSHOT600, false, 1000, 4)
    }

    fn code(r: Result<[u16; 8]>) -> &'static str {
        crate::error::AppError::from(r.unwrap_err()).code()
    }

    #[test]
    fn gate_rules() {
        let st = FcStatus::default();
        let lim = MotorTestLimits::default();
        assert_eq!(
            check_gate(&req(Some(2), 1100, true), &st, &out(), &lim).unwrap(),
            [1000, 1000, 1100, 1000, 1000, 1000, 1000, 1000]
        );
        assert_eq!(
            check_gate(&req(None, 1050, true), &st, &out(), &lim).unwrap()[..5],
            [1050, 1050, 1050, 1050, 1000]
        );
        assert_eq!(
            check_gate(&req(Some(0), 1150, true), &st, &out(), &lim).unwrap()[0],
            1150
        );

        assert!(matches!(
            check_gate(&req(Some(0), 1100, false), &st, &out(), &lim),
            Err(DeviceError::MotorTestRefused(
                MotorTestRefusal::PropsNotConfirmed
            ))
        ));
        assert!(matches!(
            check_gate(&req(Some(0), 1151, true), &st, &out(), &lim),
            Err(DeviceError::MotorTestRefused(
                MotorTestRefusal::ThrottleAboveLimit {
                    requested: 1151,
                    max: 1150
                }
            ))
        ));
        assert!(matches!(
            check_gate(&req(Some(0), 999, true), &st, &out(), &lim),
            Err(DeviceError::MotorTestRefused(
                MotorTestRefusal::ThrottleBelowStop { .. }
            ))
        ));
        assert!(matches!(
            check_gate(&req(Some(4), 1100, true), &st, &out(), &lim),
            Err(DeviceError::MotorTestRefused(
                MotorTestRefusal::InvalidMotor { index: 4, count: 4 }
            ))
        ));
        let armed = FcStatus {
            mode_flags: 1,
            ..Default::default()
        };
        assert_eq!(
            code(check_gate(&req(Some(0), 1100, true), &armed, &out(), &lim)),
            "BOARD_ARMED"
        );
        // Props check comes first, even when armed.
        assert_eq!(
            code(check_gate(&req(Some(0), 1100, false), &armed, &out(), &lim)),
            "MOTOR_TEST_REFUSED"
        );
        let three_d = MotorOutputConfig::new(protocol::DSHOT600, true, 1000, 4);
        assert_eq!(
            code(check_gate(&req(Some(0), 1100, true), &st, &three_d, &lim)),
            "MOTOR_TEST_REFUSED"
        );
        let disabled = MotorOutputConfig::new(protocol::DISABLED, false, 1000, 4);
        assert_eq!(
            code(check_gate(&req(Some(0), 1100, true), &st, &disabled, &lim)),
            "MOTOR_TEST_REFUSED"
        );
    }

    #[test]
    fn limits_cannot_exceed_hard_cap() {
        let lim = MotorTestLimits {
            max_throttle: 2000,
            watchdog: Duration::from_secs(60),
        };
        let c = lim.clamped();
        assert_eq!(c.max_throttle, HARD_MAX_TEST_THROTTLE);
        assert_eq!(c.watchdog, MAX_WATCHDOG);
        let r = check_gate(
            &req(Some(0), 1300, true),
            &FcStatus::default(),
            &out(),
            &lim,
        );
        assert!(matches!(
            r,
            Err(DeviceError::MotorTestRefused(
                MotorTestRefusal::ThrottleAboveLimit { max: 1250, .. }
            ))
        ));
    }

    #[test]
    fn analog_stop_value_is_mincommand() {
        let c = MotorOutputConfig::new(protocol::ONESHOT125, false, 1020, 4);
        assert_eq!(c.stop_value, 1020);
        assert!(!c.digital);
        let r = check_gate(
            &req(Some(0), 1010, true),
            &FcStatus::default(),
            &c,
            &MotorTestLimits::default(),
        );
        assert!(matches!(
            r,
            Err(DeviceError::MotorTestRefused(
                MotorTestRefusal::ThrottleBelowStop { stop: 1020, .. }
            ))
        ));
    }

    #[test]
    fn status_parsing() {
        let s = parse_status_ex(&status_ex(true)).unwrap();
        assert!(s.is_armed());
        assert_eq!(s.cpu_load, Some(12));
        assert_eq!(s.arming_disable_flags, Some(0));
        let s = parse_status_ex(&status_ex(false)).unwrap();
        assert!(!s.is_armed());
        assert_eq!(s.arming_disable_flags, Some(1 << 21));
        assert!(parse_status(&[0; 5]).is_err());
    }

    #[test]
    fn session_set_keepalive_watchdog() {
        let fc = FakeFc::default();
        let clock = ManualClock::default();
        let mut t = MotorTest::new(fc.serial(), clock.clone(), MotorTestLimits::default())
            .map_err(|(e, _)| e)
            .unwrap();
        assert_eq!(fc.last(), [1000; 8]); // stop sent on open
        t.set(&req(Some(1), 1120, true)).unwrap();
        assert_eq!(fc.last()[..4], [1000, 1120, 1000, 1000]);
        assert!(t.is_active());

        clock.advance(Duration::from_millis(400));
        t.keepalive().unwrap();
        clock.advance(Duration::from_millis(400));
        assert_eq!(t.tick().unwrap(), None); // 400 ms since keepalive < 500 ms
        assert_eq!(fc.last()[1], 1120);
        clock.advance(Duration::from_millis(101));
        assert_eq!(t.tick().unwrap(), Some(StopReason::Watchdog));
        assert_eq!(fc.last(), [1000; 8]);
        assert!(!t.is_active());
        assert!(matches!(
            t.keepalive(),
            Err(DeviceError::MotorTestNotActive)
        ));
    }

    #[test]
    fn stops_when_fc_becomes_armed() {
        let fc = FakeFc::default();
        let clock = ManualClock::default();
        let mut t = MotorTest::new(fc.serial(), clock.clone(), MotorTestLimits::default())
            .map_err(|(e, _)| e)
            .unwrap();
        t.set(&req(None, 1100, true)).unwrap();
        fc.armed.store(true, Ordering::SeqCst);
        clock.advance(ARMED_RECHECK);
        t.keepalive().unwrap();
        assert_eq!(t.tick().unwrap(), Some(StopReason::Armed));
        assert_eq!(fc.last(), [1000; 8]);
        // And a new start is refused while armed.
        assert!(matches!(
            t.set(&req(None, 1100, true)),
            Err(DeviceError::Armed)
        ));
        assert_eq!(fc.last(), [1000; 8]);
    }

    #[test]
    fn refusal_and_drop_stop_motors() {
        let fc = FakeFc::default();
        let mut t = MotorTest::new(
            fc.serial(),
            ManualClock::default(),
            MotorTestLimits::default(),
        )
        .map_err(|(e, _)| e)
        .unwrap();
        t.set(&req(Some(0), 1100, true)).unwrap();
        assert!(t.set(&req(Some(0), 1200, true)).is_err()); // over cap -> stop
        assert_eq!(fc.last(), [1000; 8]);
        t.set(&req(Some(0), 1100, true)).unwrap();
        drop(t);
        assert_eq!(fc.last(), [1000; 8]);
    }

    #[test]
    fn worker_watchdog_and_drop() {
        let fc = FakeFc::default();
        let limits = MotorTestLimits {
            watchdog: MIN_WATCHDOG,
            ..Default::default()
        };
        let t = MotorTest::new(fc.serial(), SystemClock::default(), limits)
            .map_err(|(e, _)| e)
            .unwrap();
        let (etx, erx) = mpsc::channel();
        let h = MotorTestHandle::spawn(t, move |e| {
            let _ = etx.send(e);
        });
        h.set(req(Some(3), 1100, true)).unwrap();
        assert_eq!(fc.last()[3], 1100);
        let ev = erx
            .recv_timeout(Duration::from_secs(2))
            .expect("watchdog event");
        assert_eq!(ev.reason, StopReason::Watchdog);
        assert_eq!(fc.last(), [1000; 8]);

        h.set(req(Some(3), 1100, true)).unwrap();
        let link = h.finish().expect("link back");
        assert_eq!(fc.last(), [1000; 8]);
        drop(link);
    }

    #[test]
    fn worker_reports_disconnect() {
        let fc = FakeFc::default();
        let unplug = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let t = MotorTest::new(
            fc.serial().with_disconnect_switch(unplug.clone()),
            SystemClock::default(),
            MotorTestLimits::default(),
        )
        .map_err(|(e, _)| e)
        .unwrap();
        let (etx, erx) = mpsc::channel();
        let h = MotorTestHandle::spawn(t, move |e| {
            let _ = etx.send(e);
        });
        h.set(req(Some(0), 1100, true)).unwrap();
        unplug.store(true, Ordering::SeqCst);
        // Keepalives keep the watchdog quiet, so the armed re-check is what notices.
        let ev = loop {
            let _ = h.keepalive();
            if let Ok(ev) = erx.recv_timeout(Duration::from_millis(50)) {
                break ev;
            }
        };
        assert_eq!(ev.reason, StopReason::Disconnected);
        assert!(h.finish().is_none());
    }
}
