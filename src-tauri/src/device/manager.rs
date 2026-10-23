// SPDX-License-Identifier: GPL-3.0-or-later
//! The app-wide connection manager (ARCHITECTURE.md §4.1, §4.2).
//!
//! One [`ConnectionManager`] lives in Tauri managed state and owns the single open FC port.
//! Callers borrow the link with [`ConnectionManager::with_link`]; a second caller while the
//! link is borrowed (or lent to the motor-test / RC-stream / attitude-stream worker) gets
//! `PORT_BUSY {heldBy: "APP"}`.

use super::attitude::{self, AttitudeStream, AttitudeStreamEvent, AttitudeStreamOptions};
use super::cli::{BootloaderTarget, CliOptions, CliSession};
use super::error::{DeviceError, Result as DResult};
use super::motor::{
    self, EmergencyStop, MotorTest, MotorTestHandle, MotorTestRequest, StopReason, SystemClock,
};
use super::msp::{codes, ApiVersion, FcIdentity, MspClient, RebootType, Uid};
use super::ram_state;
use super::rc::{RcStopReason as DevRcStop, RcStream, RcStreamEvent, RcStreamOptions};
use super::sensors::{self, AccCalibrationOptions};
use super::serial::{SerialConfig as OpenConfig, SerialLink, SystemSerial};
use super::{usb_ids, Connection, FcLink, Reconnect};
use crate::error::{AppError, AppResult, PortHolder, UnsavedChangesReason};
use crate::model::{
    AccCalibrationResult, Attitude, AttitudeStopped, FcInfo, FcSerialPort, HostPort,
    MotorOutputConfig, MotorTestLimits, MotorTestSession, MotorTestStopReason, MotorTestStopped,
    RcChannels, RcStopReason, RcStopped, RxSpiInfo, SensorStatus,
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
/// What a stale CLI session gets to leave it. NEVER `save`: that would persist whatever
/// half-typed changes the previous session left behind. The leading CR/LF terminates any
/// partial line (e.g. the echoed MSP request) so `exit` is parsed on its own line.
const STALE_CLI_EXIT: &[u8] = b"\r\nexit\r\n";
/// Printed by Betaflight's `cliRebootEx` right before `systemReset()` (4.4, 4.5, 2025.12,
/// master). Plain `exit` reboots on all of them, which throws away unsaved RAM changes and
/// any open batch; `exit noreboot` (4.5+) prints "leaving CLI mode, no reboot" instead.
const CLI_REBOOT_MARKER: &[u8] = b"Rebooting";

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

/// A board whose connection was closed while its RAM state was dirty (§4.1 "RAM state").
#[derive(Debug, Clone)]
struct CarriedDirty {
    uid: Option<Uid>,
    path: String,
    reason: UnsavedChangesReason,
    /// Closed by an (acknowledged) reboot into the DFU bootloader: a successful flash
    /// afterwards proves the RAM is gone ([`ConnectionManager::forget_ram_state_after_flash`]).
    to_bootloader: bool,
}

/// Shared between the manager and its connections: filled when a dirty connection is
/// dropped, consumed by the next `connect` to the same board.
type DirtyCarry = Arc<Mutex<Vec<CarriedDirty>>>;

/// The open FC link. Implements [`Connection`] and [`Reconnect`], i.e. [`FcLink`].
///
/// **RAM state** (ARCHITECTURE.md §4.1): `ram_dirty` says the FC's RAM may hold changes
/// that are not in EEPROM. It is set by [`mark_ram_dirty`](Self::mark_ram_dirty) (non-read-only
/// CLI line, `exit noreboot`, MSP write; all in the `Connection` impl) and cleared only by
/// [`confirm_reboot`](Self::confirm_reboot), i.e. after a reboot the app issued in the same
/// sequence and the FC acknowledged (MSP_REBOOT reply, `Rebooting` after our `exit`/`save`),
/// followed by a reconnect that re-verified the same board. A port drop or a reconnect alone
/// never clears it: a battery-powered FC keeps its RAM over a USB unplug/replug.
pub struct FcConnection {
    port: PortIdentity,
    link: Link,
    api: Option<ApiVersion>,
    uid: Option<Uid>,
    opener: Opener,
    lister: Lister,
    cli_opts: CliOptions,
    poll: Duration,
    msp_timeout: Duration,
    /// `Some(first reason)` while RAM may hold unsaved changes.
    ram_dirty: Option<UnsavedChangesReason>,
    /// The FC acknowledged an app-issued reboot (MSP_REBOOT reply, `Rebooting` after our
    /// `exit`/`save`) since the last change. Consumed by the next reconnect: if that
    /// re-verifies the same board, the reboot is confirmed.
    reboot_acked: bool,
    /// Confirmed reboots on this connection (including a stale-CLI recovery on open).
    confirmed_reboots: u32,
    /// `reboot_to_bootloader` succeeded: the port is gone and the next step is a DFU flash.
    entered_bootloader: bool,
    carry: DirtyCarry,
}

impl Drop for FcConnection {
    fn drop(&mut self) {
        if let Some(reason) = self.ram_dirty {
            log::warn!(
                "{}: closing the port while the FC may hold unsaved changes ({reason}); the next \
                 connect to this board reboots it first",
                self.port.path
            );
            let mut carry = self.carry.lock().unwrap_or_else(|p| p.into_inner());
            carry.retain(|c| !self.same_board(c));
            carry.push(CarriedDirty {
                uid: self.uid,
                path: self.port.path.clone(),
                reason,
                to_bootloader: self.entered_bootloader,
            });
        }
    }
}

/// "The FC did not answer MSP at all" (as opposed to a closed port or an MSP-level error).
fn no_msp_answer(e: &DeviceError) -> bool {
    matches!(
        e,
        DeviceError::Timeout { .. } | DeviceError::MspMalformed(_)
    )
}

impl FcConnection {
    fn open(
        port: PortIdentity,
        opener: Opener,
        lister: Lister,
        poll: Duration,
        msp_timeout: Duration,
        carry: DirtyCarry,
    ) -> DResult<Self> {
        let link = opener(&port.path)?;
        let mut c = FcConnection {
            port,
            link: Link::Detached,
            api: None,
            uid: None,
            opener,
            lister,
            cli_opts: CliOptions::default(),
            poll,
            msp_timeout,
            ram_dirty: None,
            reboot_acked: false,
            confirmed_reboots: 0,
            entered_bootloader: false,
            carry,
        };
        let mut m = c.client(link);
        match m.api_version() {
            Ok(api) => {
                c.api = Some(api);
                // Cache the board identity so a later reboot (CLI exit, save, MSP reboot)
                // can be checked against it, and `Connection::uid` needs no round-trip.
                c.uid = m.uid().ok();
                c.link = Link::Msp(m);
            }
            // No MSP answer at all: most likely a CLI session left open (crashed app,
            // Configurator closed mid-CLI). Leave it once with `exit` and retry once.
            Err(e) if no_msp_answer(&e) => {
                let budget = DEFAULT_RECONNECT_TIMEOUT;
                c.recover_stale_cli(m.into_inner(), Instant::now() + budget, budget)?;
            }
            // Anything else (e.g. the FC is still booting and garbled the reply): keep the
            // port; identify() reports the error if it persists.
            Err(e) => {
                log::debug!("{}: MSP_API_VERSION on open failed: {e}", c.port.path);
                c.link = Link::Msp(m);
            }
        }
        Ok(c)
    }

    fn client(&self, link: BoxLink) -> MspClient<BoxLink> {
        let mut m = MspClient::new(link);
        m.timeout = self.msp_timeout;
        m
    }

    // ------------------------------------------------------------------ RAM state

    /// Why the FC's RAM may hold unsaved changes; `None` = clean.
    pub fn ram_dirty(&self) -> Option<UnsavedChangesReason> {
        self.ram_dirty
    }

    /// RAM may now hold unsaved changes. Keeps the first reason; voids an earlier reboot
    /// acknowledgement (the change came after it).
    fn mark_ram_dirty(&mut self, reason: UnsavedChangesReason, what: &str) {
        if self.ram_dirty.is_none() {
            log::info!(
                "{}: FC RAM may now hold unsaved changes ({what})",
                self.port.path
            );
            self.ram_dirty = Some(reason);
        }
        self.reboot_acked = false;
    }

    /// The FC verifiably rebooted and the same board answered again: RAM = EEPROM.
    fn confirm_reboot(&mut self, how: &str) {
        if let Some(r) = self.ram_dirty.take() {
            log::info!(
                "{}: confirmed reboot ({how}); pending RAM changes ({r}) discarded",
                self.port.path
            );
        }
        self.reboot_acked = false;
        self.confirmed_reboots += 1;
    }

    /// Same physical board as `c` (MSP_UID if both known, else the port path).
    fn same_board(&self, c: &CarriedDirty) -> bool {
        match (self.uid, c.uid) {
            (Some(a), Some(b)) => a == b,
            _ => self.port.path == c.path,
        }
    }

    /// Fresh connect: if this board was closed while dirty (and has not been rebooted by this
    /// connection yet, e.g. by the stale-CLI recovery), reboot it once so RAM is discarded.
    fn resolve_carried_ram_state(&mut self) -> DResult<()> {
        let carried = {
            let mut carry = self.carry.lock().unwrap_or_else(|p| p.into_inner());
            let i = carry.iter().position(|c| self.same_board(c));
            i.map(|i| carry.remove(i))
        };
        let Some(c) = carried else {
            return Ok(());
        };
        if self.confirmed_reboots > 0 {
            return Ok(());
        }
        self.ram_dirty = Some(c.reason);
        log::warn!(
            "{}: this board was disconnected with unsaved RAM changes ({}); rebooting it to \
             discard them",
            self.port.path,
            c.reason
        );
        self.reboot_confirmed()
    }

    /// `device_reboot` ("Reboot board"), also the connect-time reboot of a board closed dirty:
    /// an app-issued firmware reboot that only succeeds once it is confirmed.
    ///
    /// Our CLI open: plain `exit` (never `save`), see
    /// [`exit_cli_and_reconnect`](Self::exit_cli_and_reconnect). Otherwise (or if that exit
    /// was not confirmed): MSP_REBOOT `[0]`, which Betaflight answers before resetting
    /// (msp.c `*mspPostProcessFn = mspRebootFn`; msp_serial.c waits for TX before running
    /// it), then wait for the port to drop and return, re-verify API version + UID.
    ///
    /// Not confirmed (no reply / error frame) → `PENDING_UNSAVED_CHANGES {reason}` if RAM is
    /// dirty (it stays dirty), else [`DeviceError::RebootNotConfirmed`] (`MSP`). The board did
    /// not come back / a different board came back → the reconnect's `SERIAL` error.
    pub fn reboot_confirmed(&mut self) -> DResult<()> {
        let before = self.confirmed_reboots;
        if matches!(self.link, Link::Cli(_)) {
            self.leave_cli_for_msp()?;
            if self.confirmed_reboots > before {
                return Ok(());
            }
        }
        let Link::Msp(mut m) = std::mem::replace(&mut self.link, Link::Detached) else {
            return Err(self.lost());
        };
        let acked = match m.reboot(RebootType::Firmware) {
            Ok(a) => a,
            // Error frame: the FC refused, nothing rebooted; the link is still good.
            Err(e @ (DeviceError::MspUnsupported(_) | DeviceError::MspMalformed(_))) => {
                self.link = Link::Msp(m);
                return Err(self.reboot_not_confirmed(&e.to_string()));
            }
            Err(e) => {
                self.link = Link::Msp(m);
                return Err(e);
            }
        };
        drop(m);
        self.reboot_acked = acked;
        self.reconnect_inner(DEFAULT_RECONNECT_TIMEOUT)?;
        if self.confirmed_reboots > before {
            Ok(())
        } else {
            Err(self.reboot_not_confirmed("no reply to MSP_REBOOT"))
        }
    }

    fn reboot_not_confirmed(&self, why: &str) -> DeviceError {
        log::warn!("{}: reboot not confirmed: {why}", self.port.path);
        match self.ram_dirty {
            Some(r) => DeviceError::PendingUnsavedChanges(r),
            None => DeviceError::RebootNotConfirmed(format!("{}: {why}", self.port.path)),
        }
    }

    /// How long to wait for a rebooting USB VCP to disappear (UART bridges never do).
    /// 3 s with the default 200 ms poll.
    fn gone_wait(&self, timeout: Duration) -> Duration {
        timeout.min(self.poll * 15)
    }

    /// Settle time after the port is back before talking to the FC (1 s by default).
    fn settle(&self) -> Duration {
        self.poll * 5
    }

    fn present(&self) -> Option<PortIdentity> {
        (self.lister)()
            .into_iter()
            .find(|p| self.port.same_device(p))
    }

    /// Wait (capped by [`gone_wait`](Self::gone_wait)) for a rebooting USB VCP to disappear.
    /// Returns whether it did; UART bridges never do.
    fn wait_gone(&self, timeout: Duration) -> bool {
        let gone_by = Instant::now() + self.gone_wait(timeout);
        loop {
            if self.present().is_none() {
                return true;
            }
            if Instant::now() >= gone_by {
                return false;
            }
            std::thread::sleep(self.poll);
        }
    }

    /// Wait until the port is listed and opens, let the FC settle, drain boot noise and ask
    /// for MSP_API_VERSION exactly once.
    fn wait_back(
        &self,
        deadline: Instant,
        timeout: Duration,
        what: &str,
    ) -> DResult<(PortIdentity, MspClient<BoxLink>, ApiVersion)> {
        let (found, mut link) = loop {
            if let Some(found) = self.present() {
                if let Ok(link) = (self.opener)(&found.path) {
                    break (found, link);
                }
            }
            if Instant::now() >= deadline {
                return Err(DeviceError::timeout(
                    format!("{} to come back {what}", self.port.path),
                    timeout,
                ));
            }
            std::thread::sleep(self.poll);
        };
        std::thread::sleep(self.settle());
        drain(&mut link, self.poll);
        let mut m = self.client(link);
        match m.api_version() {
            Ok(api) => Ok((found, m, api)),
            Err(e) => {
                log::warn!("{}: no MSP answer {what}: {e}", found.path);
                Err(e)
            }
        }
    }

    /// The FC did not answer MSP: assume a stale CLI session (left by another app or a
    /// crash) and leave it with [`exit_cli_and_reconnect`](Self::exit_cli_and_reconnect).
    fn recover_stale_cli(
        &mut self,
        link: BoxLink,
        deadline: Instant,
        timeout: Duration,
    ) -> DResult<()> {
        log::warn!(
            "{}: no MSP answer; assuming a stale CLI session, sending `exit`",
            self.port.path
        );
        // Whoever left the CLI open may have changed settings in it.
        self.mark_ram_dirty(UnsavedChangesReason::CliCommand, "stale CLI session");
        self.exit_cli_and_reconnect(link, deadline, timeout, "a stale CLI")
    }

    /// Our own CLI session is open (config's read-only commands leave it open: on 4.4 the
    /// only way out reboots) and an MSP call needs the link: leave it the same way.
    fn leave_cli_for_msp(&mut self) -> DResult<()> {
        let Link::Cli(s) = std::mem::replace(&mut self.link, Link::Detached) else {
            return Ok(());
        };
        log::info!(
            "{}: CLI still open before an MSP call; `exit` (FC reboots) and reconnecting",
            self.port.path
        );
        let budget = DEFAULT_RECONNECT_TIMEOUT;
        self.exit_cli_and_reconnect(s.into_link(), Instant::now() + budget, budget, "the CLI")
    }

    /// Leave the CLI so that nothing typed in it stays live, then reconnect in MSP mode.
    /// The link is `Detached` from the first byte on, so a failure leaves no CLI state
    /// behind (later calls get "not open; reconnect first", never a second `exit`).
    ///
    /// 1. Send `exit` (never `save`). In Betaflight 4.4, 4.5, 2025.12 and master plain `exit`
    ///    reboots the FC (`cliRebootEx`, which prints `Rebooting`), dropping unsaved RAM
    ///    changes and any open batch. Only `exit noreboot` (4.5+) keeps them.
    /// 2. Follow the reboot like `reconnect`: a USB VCP disappears and re-enumerates, a UART
    ///    bridge stays. Settle, drain, retry MSP exactly once; still silent → error.
    /// 3. `Rebooting` printed: confirmed reboot (RAM state cleared) once the board is
    ///    re-verified. A port drop without it is no proof (a battery-powered FC keeps RAM over
    ///    a USB replug); it is accepted only while the RAM state is clean (nothing to discard).
    ///    Otherwise reboot over MSP (MSP_REBOOT 68, `[0]` = firmware, like
    ///    `config::session::discard` on 4.5+) once, wait for the FC again; its reply confirms.
    /// 4. [`adopt`](Self::adopt) re-verifies MSP_UID: a different board is an error.
    fn exit_cli_and_reconnect(
        &mut self,
        mut link: BoxLink,
        deadline: Instant,
        timeout: Duration,
        from: &str,
    ) -> DResult<()> {
        self.link = Link::Detached;
        let seen = match link.write_all(STALE_CLI_EXIT) {
            Ok(()) => saw_reboot(&mut link, self.settle()),
            Err(DeviceError::Disconnected) => ExitSeen::PortDropped,
            Err(e) => return Err(e),
        };
        drop(link); // close the port so a re-enumerated VCP can be reopened
        let gone = self.wait_gone(timeout);
        let after_exit = format!("after leaving {from}");
        let (found, m, api) = self.wait_back(deadline, timeout, &after_exit)?;
        if seen == ExitSeen::Rebooting {
            self.adopt(found, m, api)?;
            self.confirm_reboot("CLI `exit` printed `Rebooting`");
            return Ok(());
        }
        let dropped = gone || seen == ExitSeen::PortDropped;
        if dropped && self.ram_dirty.is_none() {
            // Nothing to discard; the drop is consistent with `exit`'s reboot. Not counted as
            // a confirmed reboot.
            return self.adopt(found, m, api);
        }
        log::warn!(
            "{}: `exit` did not print `Rebooting`{}; rebooting over MSP to discard RAM changes",
            found.path,
            if dropped {
                " (a port drop alone is no proof)"
            } else {
                ""
            }
        );
        let mut m = m;
        let acked = m.reboot(RebootType::Firmware)?;
        drop(m);
        self.wait_gone(timeout);
        let (found, m, api) = self.wait_back(deadline, timeout, "after MSP_REBOOT")?;
        self.adopt(found, m, api)?;
        if acked {
            self.confirm_reboot("MSP_REBOOT after `exit` acknowledged");
        } else if let Some(r) = self.ram_dirty {
            log::warn!(
                "{}: MSP_REBOOT after `exit` not acknowledged; RAM may still hold unsaved \
                 changes ({r})",
                self.port.path
            );
        }
        Ok(())
    }

    /// Accept a reopened port after checking it is the same board (MSP_UID).
    fn adopt(
        &mut self,
        found: PortIdentity,
        mut m: MspClient<BoxLink>,
        api: ApiVersion,
    ) -> DResult<()> {
        let uid = match m.uid() {
            Ok(u) => Some(u),
            Err(DeviceError::MspUnsupported(_)) => None,
            Err(e) => return Err(e),
        };
        if let (Some(old), Some(new)) = (self.uid, uid) {
            if old != new {
                return Err(DeviceError::Serial(format!(
                    "a different flight controller came back on {} (uid {}, expected {})",
                    found.path,
                    new.mcu_id(),
                    old.mcu_id()
                )));
            }
        }
        self.port = found;
        self.api = Some(api);
        self.uid = uid.or(self.uid);
        self.link = Link::Msp(m);
        Ok(())
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

    /// Make sure the link is in MSP mode. If our CLI session is still open, leave it with
    /// plain `exit` + reconnect ([`leave_cli_for_msp`](Self::leave_cli_for_msp)) on every
    /// firmware: `exit noreboot` would keep whatever the session changed live in RAM.
    fn msp(&mut self) -> DResult<&mut MspClient<BoxLink>> {
        if matches!(self.link, Link::Cli(_)) {
            self.leave_cli_for_msp()?;
        }
        let lost = self.lost();
        match &mut self.link {
            Link::Msp(m) => Ok(m),
            _ => Err(lost),
        }
    }

    /// Is our CLI session open (entered via the CLI path, not yet left)?
    pub fn in_cli(&self) -> bool {
        matches!(self.link, Link::Cli(_))
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
                        self.link = Link::Msp(self.client(l));
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
        self.link = Link::Msp(self.client(link));
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
            // Fresh session, read-only commands only: nothing to discard, skip the reboot.
            let Link::Cli(s) = std::mem::replace(&mut self.link, Link::Detached) else {
                return Err(self.lost());
            };
            self.link = Link::Msp(self.client(s.exit_no_reboot()?));
        } else {
            self.leave_cli_for_msp()?;
        }
        Ok(id)
    }

    /// Reboot into the ROM DFU bootloader: MSP_REBOOT, falling back to CLI `bl rom`.
    /// The port is gone afterwards.
    pub fn reboot_to_bootloader(&mut self) -> DResult<()> {
        if let Link::Cli(_) = self.link {
            // Already in the CLI: `bl rom` directly instead of exit + reboot + MSP_REBOOT.
            let Link::Cli(s) = std::mem::replace(&mut self.link, Link::Detached) else {
                unreachable!()
            };
            s.reboot_to_bootloader(BootloaderTarget::Rom)?;
            self.entered_bootloader = true;
            return Ok(());
        }
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
        self.entered_bootloader = true;
        Ok(())
    }

    fn reconnect_inner(&mut self, timeout: Duration) -> DResult<()> {
        self.link = Link::Detached; // closes the port
        let start = Instant::now();
        let deadline = start + timeout;
        // Only an acknowledged, app-issued reboot (consumed here) can be confirmed by this
        // reconnect; a port drop alone is no proof.
        let acked = std::mem::take(&mut self.reboot_acked);
        // USB VCPs disappear while the FC reboots; UART bridges never do, so cap this wait.
        let gone = self.wait_gone(timeout);
        loop {
            if let Some(found) = self.present() {
                if let Ok(link) = (self.opener)(&found.path) {
                    let mut m = self.client(link);
                    match m.api_version() {
                        Ok(api) => match self.adopt(found, m, api) {
                            Ok(()) => {
                                if acked {
                                    self.confirm_reboot("acknowledged reboot, board re-verified");
                                } else if let Some(r) = self.ram_dirty {
                                    log::warn!(
                                        "{}: reconnected (port dropped: {gone}) without an \
                                         acknowledged reboot; RAM may still hold unsaved \
                                         changes ({r})",
                                        self.port.path
                                    );
                                }
                                return Ok(());
                            }
                            Err(e @ DeviceError::Serial(_)) => return Err(e), // other board
                            Err(_) => {}                                      // retry
                        },
                        // Port is back but the FC does not speak MSP: stale CLI. One
                        // `exit` + one MSP retry, then give up.
                        Err(e) if no_msp_answer(&e) => {
                            return self.recover_stale_cli(m.into_inner(), deadline, timeout);
                        }
                        Err(_) => {}
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

/// What the FC showed after `exit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitSeen {
    /// `Rebooting` printed (`cliRebootEx`): the FC acknowledged the reboot.
    Rebooting,
    /// The port dropped (FC reset, or the cable was pulled: no proof).
    PortDropped,
    Nothing,
}

/// Read the FC's answer to `exit` for up to `window`.
fn saw_reboot(link: &mut BoxLink, window: Duration) -> ExitSeen {
    let mut seen = Vec::new();
    let mut buf = [0u8; 256];
    let until = Instant::now() + window;
    while Instant::now() < until {
        match link.read(&mut buf, Duration::from_millis(50)) {
            Ok(n) => {
                seen.extend_from_slice(&buf[..n]);
                if seen
                    .windows(CLI_REBOOT_MARKER.len())
                    .any(|w| w == CLI_REBOOT_MARKER)
                {
                    return ExitSeen::Rebooting;
                }
            }
            Err(DeviceError::Disconnected) => return ExitSeen::PortDropped,
            Err(_) => return ExitSeen::Nothing,
        }
    }
    ExitSeen::Nothing
}

/// Discard whatever the FC sent (CLI banner, "Rebooting...", boot noise).
fn drain(link: &mut BoxLink, quiet: Duration) {
    let _ = link.clear_input();
    let mut buf = [0u8; 256];
    let until = Instant::now() + Duration::from_secs(1);
    while Instant::now() < until {
        match link.read(&mut buf, quiet.min(Duration::from_millis(50))) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

impl Connection for FcConnection {
    /// RAM state: MSP_REBOOT answered → reboot acknowledged; any code that is not a known
    /// read → dirty (set before sending, after leaving an open CLI).
    fn msp_request(&mut self, code: u16, payload: &[u8]) -> AppResult<Vec<u8>> {
        self.msp()?;
        if code == codes::MSP_REBOOT {
            let r = self.msp()?.request(code, payload);
            if r.is_ok() {
                self.reboot_acked = true;
            }
            return Ok(r?);
        }
        if !ram_state::msp_is_read_only(code) {
            self.mark_ram_dirty(UnsavedChangesReason::MspWrite, &format!("MSP {code}"));
        }
        Ok(self.msp()?.request(code, payload)?)
    }

    fn enter_cli(&mut self) -> AppResult<()> {
        self.cli()?;
        Ok(())
    }

    /// RAM state: a line that is not read-only marks RAM dirty (set before sending).
    fn cli_exec(&mut self, line: &str) -> AppResult<String> {
        self.cli()?;
        if !ram_state::cli_line_is_read_only(line) {
            self.mark_ram_dirty(UnsavedChangesReason::CliCommand, line);
        }
        Ok(self.cli()?.run(line)?)
    }

    /// `save=true`: sends `save` (FC reboots) → call `reconnect`. `save=false`:
    /// `exit noreboot` on 4.5+ (link back in MSP mode, RAM changes stay live: config's
    /// `discard` follows it with MSP_REBOOT); on 4.4 `exit` reboots the FC → call
    /// `reconnect`. Either way the CLI state is cleared before anything is written.
    fn exit_cli(&mut self, save: bool) -> AppResult<()> {
        if !matches!(self.link, Link::Cli(_)) {
            return Ok(());
        }
        let Link::Cli(s) = std::mem::replace(&mut self.link, Link::Detached) else {
            unreachable!()
        };
        if save {
            // `Rebooting` printed = the FC acknowledged; the reconnect confirms it.
            if s.save()?.contains("Rebooting") {
                self.reboot_acked = true;
            }
        } else if self.supports_noreboot() {
            self.mark_ram_dirty(UnsavedChangesReason::ExitNoReboot, "exit noreboot");
            self.link = Link::Msp(self.client(s.exit_no_reboot()?));
        } else {
            // 4.4: plain `exit` reboots; `Rebooting` printed = acknowledged, the reconnect
            // confirms it.
            let (link, rebooting) = s.exit_reporting_reboot()?;
            drop(link);
            if rebooting {
                self.reboot_acked = true;
            }
        }
        Ok(())
    }

    fn uid(&self) -> Option<String> {
        self.uid.map(|u| u.mcu_id())
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

struct AttitudeSlot {
    conn: FcConnection,
    stream: AttitudeStream<BoxLink>,
}

enum Slot {
    Empty,
    Idle(FcConnection),
    MotorTest(MotorSlot),
    Rc(RcSlot),
    Attitude(AttitudeSlot),
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
    msp_timeout: Duration,
    acc_calibration: AccCalibrationOptions,
    /// Boards closed with unsaved RAM changes (see [`FcConnection`]'s RAM state).
    dirty_carry: DirtyCarry,
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
            msp_timeout: Duration::from_secs(1),
            acc_calibration: AccCalibrationOptions::default(),
            dirty_carry: DirtyCarry::default(),
        }
    }

    /// Poll interval used while waiting for a rebooting FC (tests use a short one).
    pub fn with_poll_interval(mut self, poll: Duration) -> Self {
        self.poll = poll;
        self
    }

    /// MSP response timeout for the connection's own requests (tests use a short one).
    pub fn with_msp_timeout(mut self, t: Duration) -> Self {
        self.msp_timeout = t;
        self
    }

    /// Timing of `acc_calibrate` (tests use short waits).
    pub fn with_acc_calibration(mut self, o: AccCalibrationOptions) -> Self {
        self.acc_calibration = o;
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
            Slot::MotorTest(_) | Slot::Rc(_) | Slot::Attitude(_) => Err(self.busy()),
        }
    }

    fn with_conn<T>(&self, f: impl FnOnce(&mut FcConnection) -> DResult<T>) -> AppResult<T> {
        let mut g = self.lock()?;
        match &mut *g {
            Slot::Empty => Err(AppError::NotConnected),
            Slot::Idle(c) => Ok(f(c)?),
            Slot::MotorTest(_) | Slot::Rc(_) | Slot::Attitude(_) => Err(self.busy()),
        }
    }

    pub fn is_connected(&self) -> bool {
        self.slot
            .try_lock()
            .map(|g| !matches!(*g, Slot::Empty))
            .unwrap_or(true)
    }

    /// Open `path` as the active connection (closing a previous one first). Already open
    /// and clean: no-op. A board closed (or still open) with unsaved RAM changes is rebooted
    /// once on (re)connect (§4.1 "RAM state").
    pub fn connect(&self, path: &str) -> AppResult<()> {
        let mut g = self.lock()?;
        if let Slot::Idle(c) = &*g {
            if c.path() == path && c.ram_dirty().is_none() {
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
        let mut conn = FcConnection::open(
            ident,
            self.opener.clone(),
            self.lister.clone(),
            self.poll,
            self.msp_timeout,
            self.dirty_carry.clone(),
        )?;
        // On failure `conn` is dropped still dirty, so the board stays marked.
        conn.resolve_carried_ram_state()?;
        *g = Slot::Idle(conn);
        self.set_port(Some(path.to_string()));
        Ok(())
    }

    /// Stop any motor test / RC stream / attitude stream, then close the port.
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

    /// `device_reboot` ("Reboot board"): app-issued firmware reboot, resolved once confirmed;
    /// clears the RAM state. See [`FcConnection::reboot_confirmed`] for the errors.
    pub fn reboot(&self) -> AppResult<()> {
        self.with_conn(|c| c.reboot_confirmed())
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

    /// A DFU flash (or read-protection removal) succeeded: the MCU was reset and its flash
    /// rewritten, so a board that was rebooted into the bootloader while its RAM was dirty
    /// no longer holds those changes. Drops those marks so the next `connect` does not reboot
    /// the freshly flashed board. A board closed dirty in any other way stays marked.
    pub fn forget_ram_state_after_flash(&self) {
        let mut carry = self.dirty_carry.lock().unwrap_or_else(|p| p.into_inner());
        carry.retain(|c| {
            if c.to_bootloader {
                log::info!(
                    "{}: flashed after a reboot to DFU; pending RAM changes ({}) are gone",
                    c.path,
                    c.reason
                );
            }
            !c.to_bootloader
        });
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
            Slot::Rc(_) | Slot::Attitude(_) => Err(self.busy()),
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

    /// Safety net for window close / app exit: stops a running motor test and the RC /
    /// attitude stream. Returns whether a motor-test session was open. Never blocks:
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

    /// Ends a motor test / RC stream / attitude stream and puts the connection back to Idle (or Empty if
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
            Slot::Attitude(a) => {
                let AttitudeSlot { mut conn, stream } = a;
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
            Slot::MotorTest(_) | Slot::Attitude(_) => return Err(self.busy()),
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

impl ConnectionManager {
    // -------------------------------------------------------------------- sensors

    /// Sensor presence / enable state, hardware names, acc calibration state and the
    /// alignment readout. Read-only.
    pub fn get_sensors(&self) -> AppResult<SensorStatus> {
        self.with_conn(|c| sensors::read_sensors(c.msp()?))
    }

    /// One MSP_ATTITUDE sample (`seq` = 0, `timestampMs` = 0).
    pub fn get_attitude(&self) -> AppResult<Attitude> {
        self.with_conn(|c| sensors::read_attitude(c.msp()?))
    }

    /// Calibrate the accelerometer and return once the FC is done (holds the port for
    /// the whole calibration, ~0.5-2 s). See [`sensors::acc_calibrate`].
    ///
    /// Refused with `PENDING_UNSAVED_CHANGES` (before the armed / acc / moving checks, nothing
    /// sent) while the RAM state is dirty: the FC saves its whole RAM config at the end of the
    /// calibration. An open CLI is left first (plain `exit` = confirmed reboot).
    pub fn acc_calibrate(&self) -> AppResult<AccCalibrationResult> {
        let opts = self.acc_calibration;
        self.with_conn(|c| {
            c.msp()?;
            if let Some(reason) = c.ram_dirty() {
                return Err(DeviceError::PendingUnsavedChanges(reason));
            }
            sensors::acc_calibrate(c.msp()?, &opts)
        })
    }

    /// The open connection's RAM state (`None` = clean). `NOT_CONNECTED` / `PORT_BUSY` like
    /// every other call.
    pub fn pending_unsaved_changes(&self) -> AppResult<Option<UnsavedChangesReason>> {
        self.with_conn(|c| Ok(c.ram_dirty()))
    }

    // -------------------------------------------------------------- attitude stream

    /// Poll MSP_ATTITUDE at `rate_hz` (default 10, clamped 1..=30) on a worker thread.
    /// Already running: Ok (rate unchanged). Motor test / RC stream running: PORT_BUSY.
    pub fn attitude_stream_start(
        &self,
        rate_hz: Option<u32>,
        on_attitude: impl Fn(Attitude) + Send + 'static,
        on_stopped: impl Fn(AttitudeStopped) + Send + 'static,
    ) -> AppResult<()> {
        let mut g = self.lock()?;
        match &*g {
            Slot::Empty => return Err(AppError::NotConnected),
            Slot::Attitude(_) => return Ok(()),
            Slot::MotorTest(_) | Slot::Rc(_) => return Err(self.busy()),
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
        let opts = AttitudeStreamOptions {
            rate_hz: attitude::effective_rate(rate_hz),
            ..Default::default()
        };
        let stream = AttitudeStream::start(link, opts, move |ev: AttitudeStreamEvent| match ev {
            AttitudeStreamEvent::Attitude(a) => on_attitude(a),
            AttitudeStreamEvent::Stopped { reason, error } => on_stopped(AttitudeStopped {
                reason: match reason {
                    DevRcStop::Requested => RcStopReason::Requested,
                    DevRcStop::Disconnected => RcStopReason::Disconnected,
                    DevRcStop::Error => RcStopReason::Error,
                },
                error,
            }),
        });
        *g = Slot::Attitude(AttitudeSlot { conn, stream });
        Ok(())
    }

    /// Idempotent.
    pub fn attitude_stream_stop(&self) -> AppResult<()> {
        let mut g = self.lock()?;
        if matches!(*g, Slot::Attitude(_)) {
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

/// `motor_pwm_protocol` names (Betaflight `motorProtocolTypes_e`, 4.4 - 2026.6; DRONECAN is 2026.6+).
pub fn motor_protocol_name(p: u8) -> String {
    const NAMES: [&str; 11] = [
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
        "DRONECAN", // 2026.6
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
/// 2026.6 sends [`MCU_TYPE_ID_PROVIDED_BY_NAME`](crate::device::msp::MCU_TYPE_ID_PROVIDED_BY_NAME)
/// (254, no entry here) and the name in MSP2_MCU_INFO (`FcIdentity::mcu_name`).
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
        mcu: b
            .mcu_type_id
            .and_then(mcu_name)
            .map(str::to_string)
            .or_else(|| id.mcu_name.clone()),
        uid: id.uid.map(|u| u.mcu_id()),
        build_options: id.build_options.clone(),
        hd_osd_supported: id.hd_osd_supported,
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
        ConnectionManager::new(opener, lister)
            .with_poll_interval(Duration::from_millis(5))
            .with_msp_timeout(Duration::from_millis(100))
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

    fn sent(fc: &FakeFc) -> String {
        String::from_utf8_lossy(&fc.rx_log.lock().unwrap()).into_owned()
    }

    fn exits(fc: &FakeFc) -> usize {
        sent(fc).matches("exit").count()
    }

    fn api_requests(fc: &FakeFc) -> usize {
        let frame = [b'$', b'M', b'<', 0, codes::MSP_API_VERSION as u8];
        fc.rx_log
            .lock()
            .unwrap()
            .windows(frame.len())
            .filter(|w| *w == frame)
            .count()
    }

    fn msp_reboots(fc: &FakeFc) -> Vec<Vec<u8>> {
        fc.msp_reboots.lock().unwrap().clone()
    }

    /// A previous session left the FC in the CLI with unsaved changes (e.g. a failed batch).
    fn stale_cli(fc: &FakeFc) {
        fc.cli.store(true, Ordering::SeqCst);
        fc.ram_dirty.store(true, Ordering::SeqCst);
    }

    /// Recovered, RAM discarded, exactly one `exit`, never `save`, MSP works.
    fn assert_recovered_clean(fc: &FakeFc, m: &ConnectionManager) {
        assert!(!fc.cli.load(Ordering::SeqCst), "left the CLI");
        assert!(
            !fc.ram_dirty.load(Ordering::SeqCst),
            "stale RAM changes discarded"
        );
        assert_eq!(exits(fc), 1, "{}", sent(fc));
        assert!(!sent(fc).contains("save"), "never save: {}", sent(fc));
        assert_eq!(
            m.with_link(|l| l.msp_request(codes::MSP_API_VERSION, &[]))
                .unwrap(),
            vec![0, 1, 46]
        );
    }

    #[test]
    fn healthy_fc_never_gets_exit() {
        let fc = FakeFc::default();
        let m = manager(&fc, Arc::new(AtomicBool::new(true)));
        m.connect(PORT).unwrap();
        m.identify_uid_for_test();
        m.with_link(|l| l.reconnect(Duration::from_millis(300)))
            .unwrap();
        assert_eq!(exits(&fc), 0, "{}", sent(&fc));
        assert!(fc.cli_lines.lock().unwrap().is_empty());
        assert!(
            msp_reboots(&fc).is_empty(),
            "no MSP_REBOOT for a healthy FC"
        );
    }

    /// 4.4 `cliExit` (and plain `exit` on 4.5 / 2025.12 / 2026.6) reboots; a USB VCP re-enumerates.
    #[test]
    fn stale_cli_exit_reboots_usb_vcp() {
        let fc = FakeFc::default();
        let present = Arc::new(AtomicBool::new(true));
        stale_cli(&fc);
        *fc.unplug_on_reboot.lock().unwrap() = Some(present.clone());
        let m = manager(&fc, present);
        m.connect(PORT).unwrap();
        assert_recovered_clean(&fc, &m);
        assert!(msp_reboots(&fc).is_empty(), "exit already rebooted");
        assert_eq!(
            api_requests(&fc),
            2 + 1,
            "probe + one retry (+ the check above)"
        );
    }

    /// Same reboot behind a UART bridge: the port never drops, `Rebooting` is the evidence.
    #[test]
    fn stale_cli_exit_reboots_uart_bridge() {
        let fc = FakeFc::default();
        stale_cli(&fc);
        let m = manager(&fc, Arc::new(AtomicBool::new(true)));
        m.connect(PORT).unwrap();
        assert_recovered_clean(&fc, &m);
        assert!(msp_reboots(&fc).is_empty(), "`Rebooting` was seen");
    }

    /// 4.5+/2025.12 `exit noreboot`-style: the CLI is left but RAM keeps the stale changes,
    /// so recovery reboots over MSP (68, payload 0 = firmware) exactly once.
    #[test]
    fn stale_cli_exit_without_reboot_gets_one_msp_reboot() {
        let fc = FakeFc {
            exit_reboots: false,
            ..FakeFc::default()
        };
        let present = Arc::new(AtomicBool::new(true));
        stale_cli(&fc);
        *fc.unplug_on_reboot.lock().unwrap() = Some(present.clone());
        let m = manager(&fc, present);
        m.connect(PORT).unwrap();
        assert_eq!(msp_reboots(&fc), vec![vec![RebootType::Firmware as u8]]);
        assert_eq!(codes::MSP_REBOOT, 68);
        assert_recovered_clean(&fc, &m);
    }

    #[test]
    fn stale_cli_exit_without_reboot_on_uart_bridge() {
        let fc = FakeFc {
            exit_reboots: false,
            ..FakeFc::default()
        };
        stale_cli(&fc);
        let m = manager(&fc, Arc::new(AtomicBool::new(true)));
        m.connect(PORT).unwrap();
        assert_eq!(msp_reboots(&fc).len(), 1);
        assert_recovered_clean(&fc, &m);
    }

    #[test]
    fn connect_gives_up_after_one_exit_and_one_retry() {
        let fc = FakeFc {
            cli_sticky: true,
            ..FakeFc::default()
        };
        stale_cli(&fc);
        let m = manager(&fc, Arc::new(AtomicBool::new(true)));
        let e = m.connect(PORT).unwrap_err();
        assert_eq!(e.code(), "SERIAL", "{e}");
        assert!(e.to_string().contains("timed out"), "{e}");
        assert_eq!(exits(&fc), 1, "exactly one exit: {}", sent(&fc));
        assert_eq!(api_requests(&fc), 2, "initial probe + exactly one retry");
        assert!(msp_reboots(&fc).is_empty());
        assert!(!sent(&fc).contains("save"));
        assert!(!m.is_connected());
    }

    #[test]
    fn reconnect_recovers_a_stale_cli_session() {
        let fc = FakeFc::default();
        let present = Arc::new(AtomicBool::new(true));
        let m = manager(&fc, present.clone());
        m.connect(PORT).unwrap();
        m.identify_uid_for_test();
        // The FC comes back from a reboot sitting in the CLI (e.g. another tool typed `#`).
        stale_cli(&fc);
        *fc.unplug_on_reboot.lock().unwrap() = Some(present);
        m.with_link(|l| l.reconnect(Duration::from_secs(2)))
            .unwrap();
        assert_recovered_clean(&fc, &m);
        assert!(msp_reboots(&fc).is_empty());
    }

    #[test]
    fn reconnect_stale_cli_exit_without_reboot_gets_one_msp_reboot() {
        let fc = FakeFc {
            exit_reboots: false,
            ..FakeFc::default()
        };
        let present = Arc::new(AtomicBool::new(true));
        let m = manager(&fc, present.clone());
        m.connect(PORT).unwrap();
        m.identify_uid_for_test();
        stale_cli(&fc);
        *fc.unplug_on_reboot.lock().unwrap() = Some(present);
        m.with_link(|l| l.reconnect(Duration::from_secs(2)))
            .unwrap();
        assert_eq!(msp_reboots(&fc), vec![vec![0]]);
        assert_recovered_clean(&fc, &m);
    }

    #[test]
    fn reconnect_gives_up_after_one_exit() {
        let fc = FakeFc {
            cli_sticky: true,
            ..FakeFc::default()
        };
        let m = manager(&fc, Arc::new(AtomicBool::new(true)));
        m.connect(PORT).unwrap();
        stale_cli(&fc);
        let e = m
            .with_link(|l| l.reconnect(Duration::from_secs(2)))
            .unwrap_err();
        assert_eq!(e.code(), "SERIAL", "{e}");
        assert_eq!(exits(&fc), 1);
        assert!(msp_reboots(&fc).is_empty());
        assert!(!sent(&fc).contains("save"));
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
    fn fc_info_uid_is_the_cli_mcu_id() {
        // Three little-endian u32, each %08x, concatenated (CLI `mcu_id`).
        let p = [
            0x78, 0x56, 0x34, 0x12, 0x0f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80,
        ];
        let uid = parse_uid(&p).unwrap();
        assert_eq!(uid.mcu_id(), "123456780000000f80000000");
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
            build_options: vec!["USE_DSHOT".into(), "USE_OSD_HD".into()],
            hd_osd_supported: Some(true),
            mcu_name: None,
        };
        let info = fc_info(&id);
        assert_eq!(info.build_options, ["USE_DSHOT", "USE_OSD_HD"]);
        assert_eq!(info.hd_osd_supported, Some(true));
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["hdOsdSupported"], true);
        assert_eq!(json["buildOptions"][1], "USE_OSD_HD");
        assert_eq!(info.uid.as_deref(), Some("123456780000000f80000000"));
        assert_eq!(info.uid.as_ref().unwrap().len(), 24);
        assert_eq!(info.api_version, "1.46");
        assert_eq!(info.board_name, "SPEEDYBEEF405V4");
        assert_eq!(info.mcu.as_deref(), Some("F40X"));
        assert_eq!(info.rx_spi.present, None);
    }

    #[test]
    fn fc_info_mcu_2026_6_comes_from_mcu_info() {
        // 2026.6 MSP_BOARD_INFO always carries MCU_TYPE_ID_PROVIDED_BY_NAME (254).
        let mut id = FcIdentity {
            api_version: ApiVersion {
                protocol: 0,
                major: 1,
                minor: 48,
            },
            fc_variant: "BTFL".into(),
            fc_version: "2026.6.2".into(),
            board_info: BoardInfo {
                board_identifier: "S405".into(),
                board_version: 0,
                board_type: 2,
                target_capabilities: 0,
                target_name: "STM32F405".into(),
                board_name: "SPEEDYBEEF405V4".into(),
                manufacturer_id: "SPBE".into(),
                signature: vec![],
                mcu_type_id: Some(crate::device::msp::MCU_TYPE_ID_PROVIDED_BY_NAME),
                configuration_state: None,
                sample_rate_hz: None,
                configuration_problems: None,
            },
            uid: None,
            serial_config: None,
            rx_spi: DevRxSpi::from_msp(None, None),
            build_options: vec![],
            hd_osd_supported: None,
            mcu_name: Some("STM32F405".into()),
        };
        assert_eq!(fc_info(&id).mcu.as_deref(), Some("STM32F405"));
        assert_eq!(fc_info(&id).api_version, "1.48");
        id.mcu_name = None;
        assert_eq!(fc_info(&id).mcu, None);
        // A known table id still wins (2025.12 and older never need MSP2_MCU_INFO).
        id.board_info.mcu_type_id = Some(17);
        id.mcu_name = Some("ignored".into());
        assert_eq!(fc_info(&id).mcu.as_deref(), Some("AT32F435"));
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

    // ------------------------------------------------------------ sensors / attitude

    fn sensor_manager(sim: crate::device::sensors::sim::Sim) -> (ConnectionManager, FakeFc) {
        let (fc, _) = crate::device::sensors::sim::fake(sim);
        let m = manager(&fc, Arc::new(AtomicBool::new(true)))
            .with_acc_calibration(crate::device::sensors::sim::fast());
        m.connect(PORT).unwrap();
        (m, fc)
    }

    #[test]
    fn attitude_stream_borrows_the_port_exclusively() {
        let (m, fc) = sensor_manager(Default::default());
        let (tx, rx) = mpsc::channel();
        let (stx, srx) = mpsc::channel();
        let (tx, stx) = (Mutex::new(tx), Mutex::new(stx));
        m.attitude_stream_start(
            Some(500),
            move |a| {
                let _ = tx.lock().unwrap().send(a);
            },
            move |s| {
                let _ = stx.lock().unwrap().send(s.reason);
            },
        )
        .unwrap();
        let a = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!((a.seq, a.roll, a.pitch, a.yaw), (0, -12.5, 4.7, 271.0));
        // Idempotent start.
        m.attitude_stream_start(None, |_| {}, |_| {}).unwrap();

        // Everyone else gets PORT_BUSY {APP}.
        let busy = |e: AppError| {
            assert_eq!(e.code(), "PORT_BUSY");
            assert_eq!(e.details().unwrap()["heldBy"], "APP");
        };
        busy(m.rc_stream_start(None, |_| {}, |_| {}).unwrap_err());
        busy(
            m.motor_test_start(req(Some(1), 1100), None, |_| {})
                .unwrap_err(),
        );
        busy(m.get_sensors().unwrap_err());
        busy(m.acc_calibrate().unwrap_err());
        busy(m.with_link(|_| Ok(())).unwrap_err());
        assert!(
            fc.motors.lock().unwrap().is_empty(),
            "no motor command sent"
        );

        m.attitude_stream_stop().unwrap();
        assert_eq!(
            srx.recv_timeout(Duration::from_secs(1)).unwrap(),
            RcStopReason::Requested
        );
        m.attitude_stream_stop().unwrap(); // idempotent
        assert_eq!(m.get_attitude().unwrap().yaw, 271.0);

        // And the other way round.
        m.rc_stream_start(None, |_| {}, |_| {}).unwrap();
        busy(m.attitude_stream_start(None, |_| {}, |_| {}).unwrap_err());
        m.rc_stream_stop().unwrap();
        m.motor_test_start(req(Some(1), 1100), None, |_| {})
            .unwrap();
        busy(m.attitude_stream_start(None, |_| {}, |_| {}).unwrap_err());
        m.motor_test_stop().unwrap();
        m.attitude_stream_start(None, |_| {}, |_| {}).unwrap();
        m.attitude_stream_stop().unwrap();
    }

    #[test]
    fn attitude_stream_stops_on_disconnect_and_emergency_stop() {
        let (m, _fc) = sensor_manager(Default::default());
        let (stx, srx) = mpsc::channel();
        let stx = Mutex::new(stx);
        let start = |m: &ConnectionManager| {
            let stx = Mutex::new(stx.lock().unwrap().clone());
            m.attitude_stream_start(
                None,
                |_| {},
                move |s| {
                    let _ = stx.lock().unwrap().send(s.reason);
                },
            )
            .unwrap();
        };
        start(&m);
        assert!(!m.emergency_stop_all(), "no motor test was running");
        assert_eq!(
            srx.recv_timeout(Duration::from_secs(1)).unwrap(),
            RcStopReason::Requested
        );
        assert!(
            m.with_link(|_| Ok(())).is_ok(),
            "port back with the manager"
        );
        start(&m);
        m.disconnect().unwrap();
        assert_eq!(
            srx.recv_timeout(Duration::from_secs(1)).unwrap(),
            RcStopReason::Requested
        );
        assert!(!m.is_connected());
    }

    #[test]
    fn acc_calibration_through_the_manager() {
        let (m, _) = sensor_manager(Default::default());
        let s = m.get_sensors().unwrap();
        assert_eq!(s.acc_calibrated, Some(false));
        let r = m.acc_calibrate().unwrap();
        assert_eq!(
            r.completion,
            crate::model::AccCalibrationCompletion::CalibratingFlag
        );
        assert_eq!(m.get_sensors().unwrap().acc_calibrated, Some(true));

        let (m, _) = sensor_manager(crate::device::sensors::sim::Sim {
            armed: true,
            ..Default::default()
        });
        assert_eq!(m.acc_calibrate().unwrap_err().code(), "BOARD_ARMED");
        let (m, _) = sensor_manager(crate::device::sensors::sim::Sim {
            sensors: crate::device::sensors::sensor_bits::GYRO,
            ..Default::default()
        });
        assert_eq!(m.acc_calibrate().unwrap_err().code(), "ACC_NOT_AVAILABLE");
    }
}
