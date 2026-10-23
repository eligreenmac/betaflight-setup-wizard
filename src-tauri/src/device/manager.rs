// SPDX-License-Identifier: GPL-3.0-or-later
//! The app-wide connection manager (ARCHITECTURE.md §4.1, §4.2).
//!
//! One [`ConnectionManager`] lives in Tauri managed state and owns the single open FC port.
//! Callers borrow the link with [`ConnectionManager::with_link`]; a second caller while the
//! link is borrowed (or lent to the motor-test / RC-stream worker) gets
//! `PORT_BUSY {heldBy: "APP"}`.

use super::cli::{BootloaderTarget, CliOptions, CliSession};
use super::error::{DeviceError, Result as DResult};
use super::motor::{
    self, EmergencyStop, MotorTest, MotorTestHandle, MotorTestRequest, StopReason, SystemClock,
};
use super::msp::{ApiVersion, FcIdentity, MspClient, RebootType, Uid};
use super::rc::{RcStopReason as DevRcStop, RcStream, RcStreamEvent, RcStreamOptions};
use super::serial::{SerialConfig as OpenConfig, SerialLink, SystemSerial};
use super::{usb_ids, Connection, FcLink, Reconnect};
use crate::error::{AppError, AppResult, PortHolder};
use crate::model::{
    FcInfo, FcSerialPort, HostPort, MotorOutputConfig, MotorTestLimits, MotorTestSession,
    MotorTestStopReason, MotorTestStopped, RcChannels, RcStopReason, RcStopped, RxSpiInfo,
};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

/// A serial link as stored by the manager.
pub type BoxLink = Box<dyn SerialLink>;
/// Opens a port by path. Replaced by a mock in tests.
pub type Opener = Arc<dyn Fn(&str) -> DResult<BoxLink> + Send + Sync>;
/// Lists the host's serial ports. Replaced by a fake in tests.
pub type Lister = Arc<dyn Fn() -> Vec<PortIdentity> + Send + Sync>;

/// Default for `device_identify`'s reconnect after `device_probe_board` on 4.4.
pub const DEFAULT_RECONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// API 1.46 (Betaflight 4.5) added `exit noreboot`.
const NOREBOOT_API: (u8, u8) = (1, 46);
/// Betaflight's USB VCP serial identifier.
const VCP_IDENTIFIER: i32 = 20;

/// Which physical device a port path belongs to (used to find it again after a reboot).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PortIdentity {
    pub path: String,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
    pub serial_number: Option<String>,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
}

impl PortIdentity {
    /// Same USB device: VID/PID plus serial number when both sides have one; without USB
    /// info, the same path.
    pub fn same_device(&self, other: &PortIdentity) -> bool {
        if self.vid.is_none() || other.vid.is_none() {
            return self.path == other.path;
        }
        if self.vid != other.vid || self.pid != other.pid {
            return false;
        }
        match (&self.serial_number, &other.serial_number) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        }
    }
}

pub fn system_opener() -> Opener {
    Arc::new(|path: &str| {
        let port = SystemSerial::open(path, OpenConfig::default())?;
        Ok(Box::new(port) as BoxLink)
    })
}

pub fn system_lister() -> Lister {
    Arc::new(|| {
        serialport::available_ports()
            .map(|ports| ports.into_iter().map(identity_of).collect())
            .unwrap_or_default()
    })
}

fn identity_of(p: serialport::SerialPortInfo) -> PortIdentity {
    let mut id = PortIdentity {
        path: p.port_name,
        ..Default::default()
    };
    if let serialport::SerialPortType::UsbPort(u) = p.port_type {
        id.vid = Some(u.vid);
        id.pid = Some(u.pid);
        id.serial_number = u.serial_number;
        id.manufacturer = u.manufacturer;
        id.product = u.product;
    }
    id
}

/// `device_list_ports`: host serial ports classified by VID/PID, FC VCPs first.
pub fn host_ports(lister: &Lister) -> Vec<HostPort> {
    let mut out: Vec<HostPort> = lister()
        .into_iter()
        .filter(|p| !matches!((p.vid, p.pid), (Some(v), Some(d)) if usb_ids::is_dfu(v, d)))
        .map(|p| HostPort {
            kind: match (p.vid, p.pid) {
                (Some(v), Some(d)) => usb_ids::classify(v, d),
                _ => crate::model::HostPortKind::Unknown,
            },
            path: p.path,
            vid: p.vid,
            pid: p.pid,
            manufacturer: p.manufacturer,
            product: p.product,
        })
        .collect();
    out.sort_by_key(|p| match p.kind {
        crate::model::HostPortKind::FcVcp => 0,
        crate::model::HostPortKind::UsbUartBridge => 1,
        crate::model::HostPortKind::Unknown => 2,
    });
    out
}

// ------------------------------------------------------------------------------ connection

enum Link {
    Msp(MspClient<BoxLink>),
    Cli(CliSession<BoxLink>),
    /// Closed (FC rebooting) or lent to a worker thread.
    Detached,
}

/// The open FC link. Implements [`Connection`] and [`Reconnect`], i.e. [`FcLink`].
pub struct FcConnection {
    port: PortIdentity,
    link: Link,
    api: Option<ApiVersion>,
    uid: Option<Uid>,
    opener: Opener,
    lister: Lister,
    cli_opts: CliOptions,
    poll: Duration,
}

impl FcConnection {
    fn open(port: PortIdentity, opener: Opener, lister: Lister, poll: Duration) -> DResult<Self> {
        let link = opener(&port.path)?;
        let mut c = FcConnection {
            port,
            link: Link::Msp(MspClient::new(link)),
            api: None,
            uid: None,
            opener,
            lister,
            cli_opts: CliOptions::default(),
            poll,
        };
        // Best effort: the FC may still be booting; identify() retries.
        if let Link::Msp(m) = &mut c.link {
            c.api = m.api_version().ok();
        }
        Ok(c)
    }

    pub fn path(&self) -> &str {
        &self.port.path
    }

    fn supports_noreboot(&self) -> bool {
        self.api
            .is_some_and(|a| a.at_least(NOREBOOT_API.0, NOREBOOT_API.1))
    }

    fn lost(&self) -> DeviceError {
        DeviceError::Serial(format!(
            "{} is not open (FC rebooting?); reconnect first",
            self.port.path
        ))
    }

    /// Make sure the link is in MSP mode (leaves the CLI with `exit noreboot` on 4.5+).
    fn msp(&mut self) -> DResult<&mut MspClient<BoxLink>> {
        if matches!(self.link, Link::Cli(_)) {
            if !self.supports_noreboot() {
                return Err(DeviceError::Cli(
                    "CLI is active and this firmware reboots when leaving it; call exit_cli first"
                        .into(),
                ));
            }
            let Link::Cli(s) = std::mem::replace(&mut self.link, Link::Detached) else {
                unreachable!()
            };
            self.link = Link::Msp(MspClient::new(s.exit_no_reboot()?));
        }
        let lost = self.lost();
        match &mut self.link {
            Link::Msp(m) => Ok(m),
            _ => Err(lost),
        }
    }

    fn cli(&mut self) -> DResult<&mut CliSession<BoxLink>> {
        if matches!(self.link, Link::Msp(_)) {
            let Link::Msp(m) = std::mem::replace(&mut self.link, Link::Detached) else {
                unreachable!()
            };
            match CliSession::enter(m.into_inner(), self.cli_opts) {
                Ok(s) => self.link = Link::Cli(s),
                Err(e) => {
                    // `enter` consumed the port; reopen it so the connection stays usable.
                    if let Ok(l) = (self.opener)(&self.port.path) {
                        self.link = Link::Msp(MspClient::new(l));
                    }
                    return Err(e);
                }
            }
        }
        let lost = self.lost();
        match &mut self.link {
            Link::Cli(s) => Ok(s),
            _ => Err(lost),
        }
    }

    /// Take the port out (MSP mode) for a worker thread.
    fn lend(&mut self) -> DResult<BoxLink> {
        self.msp()?;
        match std::mem::replace(&mut self.link, Link::Detached) {
            Link::Msp(m) => Ok(m.into_inner()),
            _ => Err(self.lost()),
        }
    }

    fn give_back(&mut self, link: BoxLink) {
        self.link = Link::Msp(MspClient::new(link));
    }

    /// MSP identity (API_VERSION, FC_VARIANT, FC_VERSION, BOARD_INFO, UID, serial config).
    pub fn identify(&mut self) -> DResult<FcIdentity> {
        let id = self.msp()?.identify()?;
        self.api = Some(id.api_version);
        if id.uid.is_some() {
            self.uid = id.uid;
        }
        Ok(id)
    }

    /// `identify` plus the read-only CLI probe (§4.1). On 4.4 leaving the CLI reboots the
    /// FC, so this reconnects before returning.
    pub fn probe_board(&mut self) -> DResult<FcIdentity> {
        let mut id = self.identify()?;
        let probe = self.cli()?.probe_board()?;
        id.apply_cli_probe(&probe);
        if self.supports_noreboot() {
            self.msp()?;
        } else {
            let Link::Cli(s) = std::mem::replace(&mut self.link, Link::Detached) else {
                return Err(self.lost());
            };
            drop(s.exit()?);
            self.reconnect_inner(DEFAULT_RECONNECT_TIMEOUT)?;
        }
        Ok(id)
    }

    /// Reboot into the ROM DFU bootloader: MSP_REBOOT, falling back to CLI `bl rom`.
    /// The port is gone afterwards.
    pub fn reboot_to_bootloader(&mut self) -> DResult<()> {
        let r = self.msp()?.reboot(RebootType::Bootloader);
        match r {
            Ok(_) => {}
            Err(DeviceError::MspUnsupported(_)) => {
                self.cli()?;
                let Link::Cli(s) = std::mem::replace(&mut self.link, Link::Detached) else {
                    return Err(self.lost());
                };
                s.reboot_to_bootloader(BootloaderTarget::Rom)?;
            }
            Err(e) => return Err(e),
        }
        self.link = Link::Detached;
        Ok(())
    }

    fn reconnect_inner(&mut self, timeout: Duration) -> DResult<()> {
        self.link = Link::Detached; // closes the port
        let start = Instant::now();
        let deadline = start + timeout;
        let present = |me: &Self| (me.lister)().into_iter().find(|p| me.port.same_device(p));
        // USB VCPs disappear while the FC reboots; UART bridges never do, so cap this wait.
        let gone_by = start + timeout.min(Duration::from_secs(3));
        while Instant::now() < gone_by && present(self).is_some() {
            std::thread::sleep(self.poll);
        }
        loop {
            if let Some(found) = present(self) {
                if let Ok(link) = (self.opener)(&found.path) {
                    let mut m = MspClient::new(link);
                    if let Ok(api) = m.api_version() {
                        let uid = match m.uid() {
                            Ok(u) => Some(u),
                            Err(DeviceError::MspUnsupported(_)) => None,
                            Err(_) => {
                                std::thread::sleep(self.poll);
                                continue;
                            }
                        };
                        if let (Some(old), Some(new)) = (self.uid, uid) {
                            if old != new {
                                return Err(DeviceError::Serial(format!(
                                    "a different flight controller came back on {} (uid {}, expected {})",
                                    found.path,
                                    new.device_identifier(),
                                    old.device_identifier()
                                )));
                            }
                        }
                        self.port = found;
                        self.api = Some(api);
                        self.uid = uid.or(self.uid);
                        self.link = Link::Msp(m);
                        return Ok(());
                    }
                }
            }
            if Instant::now() >= deadline {
                return Err(DeviceError::timeout(
                    format!("{} to come back after reboot", self.port.path),
                    timeout,
                ));
            }
            std::thread::sleep(self.poll);
        }
    }
}

impl Connection for FcConnection {
    fn msp_request(&mut self, code: u16, payload: &[u8]) -> AppResult<Vec<u8>> {
        Ok(self.msp()?.request(code, payload)?)
    }

    fn enter_cli(&mut self) -> AppResult<()> {
        self.cli()?;
        Ok(())
    }

    fn cli_exec(&mut self, line: &str) -> AppResult<String> {
        Ok(self.cli()?.run(line)?)
    }

    /// `save=true`: sends `save` (FC reboots) → call `reconnect`. `save=false`:
    /// `exit noreboot` on 4.5+; on 4.4 `exit` reboots the FC → call `reconnect`.
    fn exit_cli(&mut self, save: bool) -> AppResult<()> {
        if !matches!(self.link, Link::Cli(_)) {
            return Ok(());
        }
        let Link::Cli(s) = std::mem::replace(&mut self.link, Link::Detached) else {
            unreachable!()
        };
        if save {
            s.save()?;
        } else if self.supports_noreboot() {
            self.link = Link::Msp(MspClient::new(s.exit_no_reboot()?));
        } else {
            drop(s.exit()?);
        }
        Ok(())
    }
}

impl Reconnect for FcConnection {
    fn reconnect(&mut self, timeout: Duration) -> AppResult<()> {
        Ok(self.reconnect_inner(timeout)?)
    }
}

// ------------------------------------------------------------------------------ manager

struct MotorSlot {
    conn: FcConnection,
    handle: MotorTestHandle<BoxLink>,
    session: MotorTestSession,
}

struct RcSlot {
    conn: FcConnection,
    stream: RcStream<BoxLink>,
}

enum Slot {
    Empty,
    Idle(FcConnection),
    MotorTest(MotorSlot),
    Rc(RcSlot),
}

/// The single owner of the FC port. Put it in Tauri state:
/// `app.manage(device::ConnectionManager::default())`.
pub struct ConnectionManager {
    slot: Mutex<Slot>,
    /// Path of the open port, readable while `slot` is locked (for PORT_BUSY details).
    port: Mutex<Option<String>>,
    /// Emergency-stop handle of the current/last motor test. Kept outside `slot` so window
    /// close / app exit can stop the motors while a command holds `slot`. Never held
    /// across I/O.
    motor_emergency: Mutex<Option<Arc<EmergencyStop>>>,
    opener: Opener,
    lister: Lister,
    poll: Duration,
}

impl Default for ConnectionManager {
    fn default() -> Self {
        Self::new(system_opener(), system_lister())
    }
}

impl ConnectionManager {
    pub fn new(opener: Opener, lister: Lister) -> Self {
        ConnectionManager {
            slot: Mutex::new(Slot::Empty),
            port: Mutex::new(None),
            motor_emergency: Mutex::new(None),
            opener,
            lister,
            poll: Duration::from_millis(200),
        }
    }

    /// Poll interval used while waiting for a rebooting FC (tests use a short one).
    pub fn with_poll_interval(mut self, poll: Duration) -> Self {
        self.poll = poll;
        self
    }

    pub fn lister(&self) -> &Lister {
        &self.lister
    }

    fn port_name(&self) -> String {
        self.port
            .lock()
            .map(|p| p.clone().unwrap_or_default())
            .unwrap_or_default()
    }

    fn busy(&self) -> AppError {
        AppError::PortBusy {
            port: self.port_name(),
            held_by: PortHolder::App,
        }
    }

    fn lock(&self) -> AppResult<MutexGuard<'_, Slot>> {
        match self.slot.try_lock() {
            Ok(g) => Ok(g),
            Err(TryLockError::WouldBlock) => Err(self.busy()),
            Err(TryLockError::Poisoned(p)) => Ok(p.into_inner()),
        }
    }

    fn set_port(&self, p: Option<String>) {
        if let Ok(mut g) = self.port.lock() {
            *g = p;
        }
    }

    /// Borrow the open link for the duration of `f` (config/flasher entry point).
    /// `NOT_CONNECTED` without a port; `PORT_BUSY {heldBy: APP}` while someone else has it.
    pub fn with_link<T>(&self, f: impl FnOnce(&mut dyn FcLink) -> AppResult<T>) -> AppResult<T> {
        let mut g = self.lock()?;
        match &mut *g {
            Slot::Empty => Err(AppError::NotConnected),
            Slot::Idle(c) => f(c),
            Slot::MotorTest(_) | Slot::Rc(_) => Err(self.busy()),
        }
    }

    fn with_conn<T>(&self, f: impl FnOnce(&mut FcConnection) -> DResult<T>) -> AppResult<T> {
        let mut g = self.lock()?;
        match &mut *g {
            Slot::Empty => Err(AppError::NotConnected),
            Slot::Idle(c) => Ok(f(c)?),
            Slot::MotorTest(_) | Slot::Rc(_) => Err(self.busy()),
        }
    }

    pub fn is_connected(&self) -> bool {
        self.slot
            .try_lock()
            .map(|g| !matches!(*g, Slot::Empty))
            .unwrap_or(true)
    }

    /// Open `path` as the active connection (closing a previous one first).
    pub fn connect(&self, path: &str) -> AppResult<()> {
        let mut g = self.lock()?;
        if let Slot::Idle(c) = &*g {
            if c.path() == path {
                return Ok(());
            }
        }
        Self::stop_workers(&mut g);
        *g = Slot::Empty;
        self.set_port(None);
        let ident = (self.lister)()
            .into_iter()
            .find(|p| p.path == path)
            .unwrap_or_else(|| PortIdentity {
                path: path.to_string(),
                ..Default::default()
            });
        let conn = FcConnection::open(ident, self.opener.clone(), self.lister.clone(), self.poll)?;
        *g = Slot::Idle(conn);
        self.set_port(Some(path.to_string()));
        Ok(())
    }

    /// Stop any motor test / RC stream, then close the port.
    pub fn disconnect(&self) -> AppResult<()> {
        let mut g = self.lock()?;
        Self::stop_workers(&mut g);
        *g = Slot::Empty;
        self.set_port(None);
        Ok(())
    }

    pub fn identify(&self) -> AppResult<FcInfo> {
        self.with_conn(|c| c.identify()).map(|id| fc_info(&id))
    }

    pub fn probe_board(&self) -> AppResult<FcInfo> {
        self.with_conn(|c| c.probe_board()).map(|id| fc_info(&id))
    }

    pub fn cli_exec(&self, line: &str) -> AppResult<String> {
        self.with_link(|l| l.cli_exec(line))
    }

    /// Reboot to DFU; the connection is closed afterwards (the VCP disappears).
    pub fn reboot_to_bootloader(&self) -> AppResult<()> {
        let mut g = self.lock()?;
        let r = match &mut *g {
            Slot::Empty => return Err(AppError::NotConnected),
            Slot::Idle(c) => c.reboot_to_bootloader(),
            _ => return Err(self.busy()),
        };
        r?;
        *g = Slot::Empty;
        self.set_port(None);
        Ok(())
    }

    // -------------------------------------------------------------------- motor test

    /// Start (first call) or update (later calls) the motor test. `on_stopped` receives
    /// every end of a running test (`device://motor-test-stopped`).
    pub fn motor_test_start(
        &self,
        req: MotorTestRequest,
        limits: Option<MotorTestLimits>,
        on_stopped: impl Fn(MotorTestStopped) + Send + Sync + 'static,
    ) -> AppResult<MotorTestSession> {
        let mut g = self.lock()?;
        match &mut *g {
            Slot::Empty => Err(AppError::NotConnected),
            Slot::Rc(_) => Err(self.busy()),
            Slot::MotorTest(m) => {
                let r = m.handle.set(req);
                let session = m.session.clone();
                if matches!(r, Err(DeviceError::Disconnected)) {
                    *g = Slot::Empty;
                    self.set_port(None);
                }
                r?;
                Ok(session)
            }
            Slot::Idle(_) => {
                let Slot::Idle(mut conn) = std::mem::replace(&mut *g, Slot::Empty) else {
                    unreachable!()
                };
                let link = match conn.lend() {
                    Ok(l) => l,
                    Err(e) => {
                        *g = Slot::Idle(conn);
                        return Err(e.into());
                    }
                };
                let dev_limits = limits.map(dev_limits).unwrap_or_default();
                let test = match MotorTest::new(link, SystemClock::default(), dev_limits) {
                    Ok(t) => t,
                    Err((e, link)) => {
                        conn.give_back(link);
                        *g = Slot::Idle(conn);
                        return Err(e.into());
                    }
                };
                let session = MotorTestSession {
                    output_config: output_config(test.output_config()),
                    limits: model_limits(test.limits()),
                };
                let emergency = test.emergency();
                let flag = emergency.clone();
                let handle = MotorTestHandle::spawn(test, move |ev: motor::MotorTestStopped| {
                    let reason = match ev.reason {
                        StopReason::Requested if flag.is_halted() => {
                            MotorTestStopReason::AppClosing
                        }
                        StopReason::Requested => MotorTestStopReason::Requested,
                        StopReason::Watchdog => MotorTestStopReason::Watchdog,
                        StopReason::Armed => MotorTestStopReason::Armed,
                        StopReason::Disconnected => MotorTestStopReason::Disconnected,
                        StopReason::Error => MotorTestStopReason::Error,
                    };
                    on_stopped(MotorTestStopped {
                        reason,
                        error: ev.error,
                    });
                });
                if let Err(e) = handle.set(req) {
                    // Refused (motors at stop) or lost: end the session, keep the port.
                    match handle.finish() {
                        Some(link) => {
                            conn.give_back(link);
                            *g = Slot::Idle(conn);
                        }
                        None => self.set_port(None),
                    }
                    return Err(e.into());
                }
                *self
                    .motor_emergency
                    .lock()
                    .unwrap_or_else(|p| p.into_inner()) = Some(emergency);
                *g = Slot::MotorTest(MotorSlot {
                    conn,
                    handle,
                    session: session.clone(),
                });
                Ok(session)
            }
        }
    }

    pub fn motor_test_keepalive(&self) -> AppResult<()> {
        let g = self.lock()?;
        match &*g {
            Slot::MotorTest(m) => Ok(m.handle.keepalive()?),
            _ => Err(AppError::MotorTestNotActive),
        }
    }

    /// Stop all motors and return the port to the manager. Idempotent.
    pub fn motor_test_stop(&self) -> AppResult<()> {
        let mut g = self.lock()?;
        if matches!(*g, Slot::MotorTest(_)) {
            Self::stop_workers(&mut g);
            if matches!(*g, Slot::Empty) {
                self.set_port(None);
            }
        }
        Ok(())
    }

    /// Safety net for window close / app exit: stops a running motor test and the RC
    /// stream. Returns whether a motor-test session was open. Never blocks:
    ///
    /// 1. Lock-free: sets the motor test's halt flag (checked by the worker before every
    ///    MSP_SET_MOTOR, by keepalive and every ~20 ms) and writes an all-stop
    ///    MSP_SET_MOTOR directly to a duplicated port handle, without waiting for the worker
    ///    or the manager lock.
    /// 2. One `try_lock` on the manager for cleanup (port back to Idle, RC stream stopped).
    ///    If a command holds it (e.g. a keepalive waiting on the worker), cleanup is left to
    ///    the worker, which stops the motors again as soon as it gets control back.
    pub fn emergency_stop_all(&self) -> bool {
        let em = self
            .motor_emergency
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .filter(|e| !e.is_finished());
        let outcome = em.as_ref().map(|e| e.trigger());
        let running = em.is_some();
        let direct = match outcome {
            Some(o) if o.written => "stop frame written directly",
            Some(_) => "direct stop write unavailable, worker will send stop",
            None => "",
        };
        let mut g = match self.slot.try_lock() {
            Ok(g) => g,
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
            Err(TryLockError::WouldBlock) => {
                if running {
                    log::warn!(
                        "emergency stop: motor test running; halt flag set, {direct}; manager \
                         busy (command in flight), worker stops motors and keeps the port \
                         until it returns"
                    );
                } else {
                    log::warn!(
                        "emergency stop: no motor test session; manager busy (command in \
                         flight), RC stream (if any) not stopped"
                    );
                }
                return running;
            }
        };
        if running {
            log::warn!("emergency stop: stopping motor test; halt flag set, {direct}");
        }
        Self::stop_workers(&mut g);
        running
    }

    /// Ends a motor test / RC stream and puts the connection back to Idle (or Empty if
    /// the port is gone).
    fn stop_workers(g: &mut MutexGuard<'_, Slot>) {
        match std::mem::replace(&mut **g, Slot::Empty) {
            Slot::MotorTest(m) => {
                let MotorSlot {
                    mut conn, handle, ..
                } = m;
                let _ = handle.stop();
                if let Some(link) = handle.finish() {
                    conn.give_back(link);
                    **g = Slot::Idle(conn);
                }
            }
            Slot::Rc(r) => {
                let RcSlot { mut conn, stream } = r;
                if let Some(link) = stream.stop() {
                    conn.give_back(link);
                    **g = Slot::Idle(conn);
                }
            }
            other => **g = other,
        }
    }

    // -------------------------------------------------------------------- RC stream

    /// Poll MSP_RC at `rate_hz` (default 15, clamped 1..=50) on a worker thread.
    pub fn rc_stream_start(
        &self,
        rate_hz: Option<u32>,
        on_channels: impl Fn(RcChannels) + Send + 'static,
        on_stopped: impl Fn(RcStopped) + Send + 'static,
    ) -> AppResult<()> {
        let mut g = self.lock()?;
        match &*g {
            Slot::Empty => return Err(AppError::NotConnected),
            Slot::Rc(_) => return Ok(()),
            Slot::MotorTest(_) => return Err(self.busy()),
            Slot::Idle(_) => {}
        }
        let Slot::Idle(mut conn) = std::mem::replace(&mut *g, Slot::Empty) else {
            unreachable!()
        };
        let link = match conn.lend() {
            Ok(l) => l,
            Err(e) => {
                *g = Slot::Idle(conn);
                return Err(e.into());
            }
        };
        let opts = RcStreamOptions {
            rate_hz: rate_hz
                .unwrap_or(super::rc::DEFAULT_RATE_HZ)
                .clamp(super::rc::MIN_RATE_HZ, super::rc::MAX_RATE_HZ),
            ..Default::default()
        };
        let stream = RcStream::start(link, opts, move |ev: RcStreamEvent| match ev {
            RcStreamEvent::Channels(f) => on_channels(RcChannels {
                seq: f.seq,
                timestamp_ms: f.timestamp_ms,
                channels: f.channels,
            }),
            RcStreamEvent::Stopped { reason, error } => on_stopped(RcStopped {
                reason: match reason {
                    DevRcStop::Requested => RcStopReason::Requested,
                    DevRcStop::Disconnected => RcStopReason::Disconnected,
                    DevRcStop::Error => RcStopReason::Error,
                },
                error,
            }),
        });
        *g = Slot::Rc(RcSlot { conn, stream });
        Ok(())
    }

    /// Idempotent.
    pub fn rc_stream_stop(&self) -> AppResult<()> {
        let mut g = self.lock()?;
        if matches!(*g, Slot::Rc(_)) {
            Self::stop_workers(&mut g);
            if matches!(*g, Slot::Empty) {
                self.set_port(None);
            }
        }
        Ok(())
    }
}

// ------------------------------------------------------------------------------ mapping

fn dev_limits(l: MotorTestLimits) -> motor::MotorTestLimits {
    motor::MotorTestLimits {
        max_throttle: l.max_throttle,
        watchdog: Duration::from_millis(l.watchdog_ms as u64),
    }
    .clamped()
}

fn model_limits(l: &motor::MotorTestLimits) -> MotorTestLimits {
    MotorTestLimits {
        max_throttle: l.max_throttle,
        watchdog_ms: l.watchdog.as_millis() as u32,
    }
}

/// `motor_pwm_protocol` names (Betaflight `motorProtocolTypes_e`, 4.4 - 2025.12).
pub fn motor_protocol_name(p: u8) -> String {
    const NAMES: [&str; 10] = [
        "PWM",
        "ONESHOT125",
        "ONESHOT42",
        "MULTISHOT",
        "BRUSHED",
        "DSHOT150",
        "DSHOT300",
        "DSHOT600",
        "PROSHOT1000",
        "DISABLED",
    ];
    NAMES
        .get(p as usize)
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("UNKNOWN_{p}"))
}

fn output_config(o: &motor::MotorOutputConfig) -> MotorOutputConfig {
    MotorOutputConfig {
        protocol: motor_protocol_name(o.protocol),
        digital: o.digital,
        feature3d: o.feature_3d,
        mincommand: o.mincommand,
        motor_count: o.motor_count,
        stop_value: o.stop_value,
    }
}

/// `mcuTypeNames` from Betaflight cli.c (4.5.5; ids are stable across 4.4 - 2025.12).
pub fn mcu_name(id: u8) -> Option<&'static str> {
    const MCU: [&str; 18] = [
        "SIMULATOR",
        "F40X",
        "F411",
        "F446",
        "F722",
        "F745",
        "F746",
        "F765",
        "H750",
        "H743 (Rev Unknown)",
        "H743 (Rev.Y)",
        "H743 (Rev.X)",
        "H743 (Rev.V)",
        "H7A3",
        "H723/H725",
        "G474",
        "H730",
        "AT32F435",
    ];
    MCU.get(id as usize).copied()
}

/// MSP identity → the shared `FcInfo`.
pub fn fc_info(id: &FcIdentity) -> FcInfo {
    let b = &id.board_info;
    let non_empty = |s: &str| (!s.is_empty()).then(|| s.to_string());
    let serial_ports = id
        .serial_config
        .as_ref()
        .map(|cfg| {
            cfg.ports
                .iter()
                .filter_map(|p| {
                    let identifier = p.identifier? as i32;
                    let name = p
                        .name
                        .clone()
                        .or_else(|| p.label.clone())
                        .unwrap_or_else(|| format!("PORT{identifier}"));
                    Some(FcSerialPort {
                        name,
                        identifier,
                        function_mask: p.function_mask,
                        is_connection_port: identifier == VCP_IDENTIFIER,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let r = &id.rx_spi;
    FcInfo {
        fc_variant: id.fc_variant.clone(),
        firmware_version: id.fc_version.clone(),
        api_version: format!("{}.{}", id.api_version.major, id.api_version.minor),
        board_name: non_empty(&b.board_name)
            .or_else(|| non_empty(&b.target_name))
            .unwrap_or_else(|| b.board_identifier.clone()),
        manufacturer_id: non_empty(&b.manufacturer_id),
        mcu: b.mcu_type_id.and_then(mcu_name).map(str::to_string),
        uid: id.uid.map(|u| u.device_identifier()),
        build_options: Vec::new(),
        serial_ports,
        rx_spi: RxSpiInfo {
            present: r.has_rx_spi,
            expresslrs_pins: r.has_elrs_pins,
            protocol: r.protocol.clone(),
            feature_enabled: r.feature_enabled,
            resources: r
                .resources
                .iter()
                .map(|a| {
                    format!(
                        "{} {} {}",
                        a.owner,
                        a.index.unwrap_or(1),
                        a.pin.as_deref().unwrap_or("NONE")
                    )
                })
                .collect(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::motor::fake::FakeFc;
    use crate::device::msp::codes;
    use crate::device::msp::{parse_uid, BoardInfo};
    use crate::device::rx_spi::RxSpiInfo as DevRxSpi;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;

    const PORT: &str = "COM7";

    fn ident() -> PortIdentity {
        PortIdentity {
            path: PORT.into(),
            vid: Some(0x0483),
            pid: Some(0x5740),
            serial_number: Some("0x8000000".into()),
            ..Default::default()
        }
    }

    /// Manager wired to a fake FC; `present` controls whether the port is listed.
    fn manager(fc: &FakeFc, present: Arc<AtomicBool>) -> ConnectionManager {
        let fc = fc.clone();
        let p2 = present.clone();
        let opener: Opener = Arc::new(move |_path: &str| {
            if !p2.load(Ordering::SeqCst) {
                return Err(DeviceError::NotFound(PORT.into()));
            }
            Ok(Box::new(fc.serial()) as BoxLink)
        });
        let lister: Lister = Arc::new(move || {
            if present.load(Ordering::SeqCst) {
                vec![ident()]
            } else {
                vec![]
            }
        });
        ConnectionManager::new(opener, lister).with_poll_interval(Duration::from_millis(5))
    }

    fn req(i: Option<u8>, t: u16) -> MotorTestRequest {
        MotorTestRequest {
            motor_index: i,
            throttle: t,
            props_off_confirmed: true,
        }
    }

    #[test]
    fn with_link_needs_a_connection() {
        let m = manager(&FakeFc::default(), Arc::new(AtomicBool::new(true)));
        let e = m.with_link(|_| Ok(())).unwrap_err();
        assert_eq!(e.code(), "NOT_CONNECTED");
        m.connect(PORT).unwrap();
        let api = m
            .with_link(|l| l.msp_request(codes::MSP_API_VERSION, &[]))
            .unwrap();
        assert_eq!(api, vec![0, 1, 46]);
    }

    #[test]
    fn emergency_stop_all_stops_running_motor_test() {
        let fc = FakeFc::default();
        let m = manager(&fc, Arc::new(AtomicBool::new(true)));
        m.connect(PORT).unwrap();
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let s = m
            .motor_test_start(req(Some(1), 1100), None, move |ev| {
                let _ = tx.lock().unwrap().send(ev.reason);
            })
            .unwrap();
        assert_eq!(s.output_config.protocol, "DSHOT600");
        assert_eq!(s.output_config.stop_value, 1000);
        assert_eq!(s.limits.max_throttle, 1150);
        assert_eq!(fc.last(), [1000, 1100, 1000, 1000, 1000, 1000, 1000, 1000]);

        // The port is lent to the motor-test worker: everyone else gets PORT_BUSY {APP}.
        let busy = m.with_link(|_| Ok(())).unwrap_err();
        assert_eq!(busy.code(), "PORT_BUSY");
        assert_eq!(busy.details().unwrap()["heldBy"], "APP");
        assert_eq!(busy.details().unwrap()["port"], PORT);

        assert!(m.emergency_stop_all());
        assert_eq!(fc.last(), [1000; 8], "all motors at stop");
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            MotorTestStopReason::AppClosing
        );
        // Port is back with the manager; nothing left to stop.
        assert!(m.with_link(|_| Ok(())).is_ok());
        assert!(!m.emergency_stop_all());
        assert_eq!(
            m.motor_test_keepalive().unwrap_err().code(),
            "MOTOR_TEST_NOT_ACTIVE"
        );
    }

    /// Start a motor test spinning motor 1 at 1100; stop events go to the returned channel.
    fn spinning(fc: &FakeFc) -> (Arc<ConnectionManager>, mpsc::Receiver<MotorTestStopReason>) {
        let m = Arc::new(manager(fc, Arc::new(AtomicBool::new(true))));
        m.connect(PORT).unwrap();
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        m.motor_test_start(req(Some(1), 1100), None, move |ev| {
            let _ = tx.lock().unwrap().send(ev.reason);
        })
        .unwrap();
        assert_eq!(fc.last(), [1000, 1100, 1000, 1000, 1000, 1000, 1000, 1000]);
        (m, rx)
    }

    #[test]
    fn emergency_stop_does_not_wait_for_a_held_manager_lock() {
        let fc = FakeFc::default();
        let (m, rx) = spinning(&fc);

        // Something (a keepalive, say) holds the manager lock and doesn't let go.
        let guard = m.slot.lock().unwrap();
        let t0 = Instant::now();
        assert!(m.emergency_stop_all(), "a motor test was running");
        assert!(
            t0.elapsed() < Duration::from_millis(100),
            "returned in {:?}",
            t0.elapsed()
        );
        // Direct write through the duplicated handle: motors already at stop.
        assert_eq!(fc.last(), [1000; 8], "all motors at stop");
        // The worker sees the halt flag on its own and reports it.
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            MotorTestStopReason::AppClosing
        );
        drop(guard);

        // Halted for good: no more spinning, keepalive refused; stop cleans up the port.
        assert_eq!(
            m.motor_test_keepalive().unwrap_err().code(),
            "MOTOR_TEST_NOT_ACTIVE"
        );
        assert_eq!(fc.last(), [1000; 8]);
        m.motor_test_stop().unwrap();
        assert!(m.with_link(|_| Ok(())).is_ok());
        assert!(!m.emergency_stop_all(), "session over");
        assert!(
            rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "exactly one stopped event"
        );
    }

    #[test]
    fn emergency_stop_while_keepalive_holds_the_lock_and_worker_is_stuck() {
        let fc = FakeFc::default();
        let (m, rx) = spinning(&fc);

        // The worker gets stuck writing the keepalive's SET_MOTOR, so the keepalive call
        // holds the manager lock while it waits for the worker's reply.
        fc.wedge.store(true, Ordering::SeqCst);
        let m2 = m.clone();
        let keepalive = std::thread::spawn(move || m2.motor_test_keepalive());
        let t = Instant::now();
        while m.slot.try_lock().is_ok() {
            assert!(
                t.elapsed() < Duration::from_secs(1),
                "keepalive took the lock"
            );
            std::thread::sleep(Duration::from_millis(1));
        }

        let t0 = Instant::now();
        assert!(m.emergency_stop_all(), "a motor test was running");
        assert!(
            t0.elapsed() < Duration::from_millis(100),
            "returned in {:?}",
            t0.elapsed()
        );
        assert_eq!(fc.last(), [1000; 8], "stopped by the direct write");
        assert!(m.slot.try_lock().is_err(), "keepalive still holds the lock");

        // Worker unsticks: the write already in flight lands, then the worker sees the halt,
        // sends stop through its own link and reports it; nothing but stop after that.
        fc.wedge.store(false, Ordering::SeqCst);
        let _ = keepalive.join().unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            MotorTestStopReason::AppClosing
        );
        let sent = fc.motors.lock().unwrap().clone();
        let after_stop = sent.iter().rposition(|v| v[1] == 1100).unwrap() + 1;
        assert!(
            sent[after_stop..].iter().all(|v| *v == [1000; 8]) && after_stop < sent.len(),
            "nothing but stop after the emergency stop: {sent:?}"
        );
        assert_eq!(
            m.motor_test_keepalive().unwrap_err().code(),
            "MOTOR_TEST_NOT_ACTIVE"
        );
        assert_eq!(
            m.motor_test_start(req(Some(1), 1100), None, |_| {})
                .unwrap_err()
                .code(),
            "MOTOR_TEST_NOT_ACTIVE"
        );
        assert_eq!(fc.last(), [1000; 8]);
        // Next emergency stop can take the lock and hand the port back.
        assert!(m.emergency_stop_all());
        assert!(m.with_link(|_| Ok(())).is_ok());
    }

    #[test]
    fn refused_start_keeps_the_port_and_motors_stopped() {
        let fc = FakeFc::default();
        let m = manager(&fc, Arc::new(AtomicBool::new(true)));
        m.connect(PORT).unwrap();
        let e = m
            .motor_test_start(req(Some(0), 1300), None, |_| {})
            .unwrap_err();
        assert_eq!(e.code(), "MOTOR_TEST_REFUSED");
        assert_eq!(fc.last(), [1000; 8]);
        fc.armed.store(true, Ordering::SeqCst);
        let e = m
            .motor_test_start(req(Some(0), 1100), None, |_| {})
            .unwrap_err();
        assert_eq!(e.code(), "BOARD_ARMED");
        assert!(m.with_link(|_| Ok(())).is_ok());
    }

    #[test]
    fn disconnect_stops_motors_first() {
        let fc = FakeFc::default();
        let m = manager(&fc, Arc::new(AtomicBool::new(true)));
        m.connect(PORT).unwrap();
        m.motor_test_start(req(None, 1050), None, |_| {}).unwrap();
        assert_eq!(fc.last()[..4], [1050; 4]);
        m.disconnect().unwrap();
        assert_eq!(fc.last(), [1000; 8]);
        assert_eq!(m.with_link(|_| Ok(())).unwrap_err().code(), "NOT_CONNECTED");
    }

    #[test]
    fn reconnect_waits_for_the_same_board() {
        let fc = FakeFc::default();
        let present = Arc::new(AtomicBool::new(true));
        let m = manager(&fc, present.clone());
        m.connect(PORT).unwrap();
        m.identify_uid_for_test();
        let p = present.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            p.store(false, Ordering::SeqCst); // VCP drops while rebooting
            std::thread::sleep(Duration::from_millis(60));
            p.store(true, Ordering::SeqCst);
        });
        m.with_link(|l| l.reconnect(Duration::from_secs(2)))
            .unwrap();
        t.join().unwrap();
        assert!(m
            .with_link(|l| l.msp_request(codes::MSP_API_VERSION, &[]))
            .is_ok());

        // A different board on the same port is rejected.
        *fc.uid.lock().unwrap() = [9, 0, 0, 0, 9, 0, 0, 0, 9, 0, 0, 0];
        let e = m
            .with_link(|l| l.reconnect(Duration::from_millis(300)))
            .unwrap_err();
        assert!(e.to_string().contains("different flight controller"), "{e}");
    }

    #[test]
    fn reconnect_times_out() {
        let fc = FakeFc::default();
        let present = Arc::new(AtomicBool::new(true));
        let m = manager(&fc, present.clone());
        m.connect(PORT).unwrap();
        present.store(false, Ordering::SeqCst);
        let e = m
            .with_link(|l| l.reconnect(Duration::from_millis(50)))
            .unwrap_err();
        assert_eq!(e.code(), "SERIAL");
    }

    impl ConnectionManager {
        /// Read MSP_UID like identify() does, without needing BOARD_INFO in the fake.
        fn identify_uid_for_test(&self) {
            self.with_conn(|c| {
                let uid = c.msp()?.uid()?;
                c.uid = Some(uid);
                Ok(())
            })
            .unwrap();
        }
    }

    #[test]
    fn uid_is_formatted_like_the_configurator() {
        // Three little-endian u32, unpadded lowercase hex, concatenated.
        let p = [
            0x78, 0x56, 0x34, 0x12, 0x0f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80,
        ];
        let uid = parse_uid(&p).unwrap();
        assert_eq!(uid.device_identifier(), "12345678f80000000");
        assert!(parse_uid(&p[..11]).is_err());

        let id = FcIdentity {
            api_version: ApiVersion {
                protocol: 0,
                major: 1,
                minor: 46,
            },
            fc_variant: "BTFL".into(),
            fc_version: "4.5.2".into(),
            board_info: BoardInfo {
                board_identifier: "S405".into(),
                board_version: 0,
                board_type: 2,
                target_capabilities: 0,
                target_name: "STM32F405".into(),
                board_name: "SPEEDYBEEF405V4".into(),
                manufacturer_id: "SPBE".into(),
                signature: vec![],
                mcu_type_id: Some(1),
                configuration_state: None,
                sample_rate_hz: None,
                configuration_problems: None,
            },
            uid: Some(uid),
            serial_config: None,
            rx_spi: DevRxSpi::from_msp(None, None),
        };
        let info = fc_info(&id);
        assert_eq!(info.uid.as_deref(), Some("12345678f80000000"));
        assert_eq!(info.api_version, "1.46");
        assert_eq!(info.board_name, "SPEEDYBEEF405V4");
        assert_eq!(info.mcu.as_deref(), Some("F40X"));
        assert_eq!(info.rx_spi.present, None);
    }

    #[test]
    fn same_device_matching() {
        let a = ident();
        let mut b = ident();
        b.path = "COM9".into();
        assert!(a.same_device(&b), "Windows may renumber the COM port");
        b.serial_number = Some("other".into());
        assert!(!a.same_device(&b));
        let bridge = PortIdentity {
            path: "/dev/ttyUSB0".into(),
            ..Default::default()
        };
        assert!(bridge.same_device(&bridge.clone()));
    }
}
