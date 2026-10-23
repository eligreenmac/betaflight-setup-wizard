// SPDX-License-Identifier: GPL-3.0-or-later
//! RAM state ("dirty RAM", ARCHITECTURE.md §4.1): the connection remembers when the FC's RAM
//! may hold changes that are not in EEPROM, and `device_acc_calibrate` refuses with
//! `PENDING_UNSAVED_CHANGES` until a confirmed reboot (app-issued + acknowledged + same board
//! re-verified; `device_reboot`), because the firmware saves its whole RAM config at the end
//! of an accelerometer calibration. A port drop / plain reconnect never clears it.
//!
//! The fake FC answers sensor MSP from `sensors::sim`, CLI from config's `MockFc` (real dumps),
//! and can be told to refuse MSP_REBOOT (error frame, no reboot), i.e. config's cleanup
//! reboot fails and its error is ignored or never confirmed.

use super::error::DeviceError;
use super::manager::{BoxLink, ConnectionManager, Lister, Opener, PortIdentity};
use super::motor::fake::{CliBackend, FakeFc, HookReply};
use super::msp::codes;
use super::sensors::sim::{self, Sim};
use crate::config::backup::BackupStore;
use crate::config::mock::{fixture, MockFc};
use crate::config::session;
use crate::device::Connection;
use crate::error::{AppError, UnsavedChangesReason};
use crate::model::DroneModel;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const PORT: &str = "COM7";
const RECONNECT: Duration = Duration::from_secs(2);

/// Config's MockFc as the fake FC's CLI, with an injectable rejected command. `save` answers
/// like Betaflight (`Rebooting`), so a successful apply's save is acknowledged.
struct Cfg {
    mock: MockFc,
    fail: Arc<Mutex<Option<String>>>,
}

impl CliBackend for Cfg {
    fn enter(&mut self) {
        let _ = self.mock.enter_cli();
    }
    fn exec(&mut self, line: &str) -> String {
        if self.fail.lock().unwrap().as_deref() == Some(line) {
            return format!("###ERROR IN {line}: INJECTED FAILURE###");
        }
        if line == "save" {
            let _ = self.mock.exit_cli(true);
            return "Saving...\nRebooting".into();
        }
        self.mock
            .cli_exec(line)
            .unwrap_or_else(|e| format!("###ERROR: {e}###"))
    }
    fn reboot(&mut self) {
        let _ = self.mock.exit_cli(false);
        let _ = self.mock.msp_request(session::MSP_SET_REBOOT, &[0]);
    }
}

#[derive(Clone, Copy, Debug)]
enum Port {
    /// USB VCP: the port disappears while the FC reboots.
    Usb,
    /// UART bridge: the port never goes away.
    Uart,
}

struct Rig {
    fc: FakeFc,
    sim: Arc<Mutex<Sim>>,
    /// MSP_REBOOT is answered with an error frame and the FC does not reboot.
    reboot_fails: Arc<AtomicBool>,
    fail: Arc<Mutex<Option<String>>>,
    /// The port is listed / can be opened (USB unplugged = false).
    present: Arc<AtomicBool>,
    m: ConnectionManager,
    port: Port,
}

fn rig(port: Port, api_minor: u8) -> Rig {
    rig_ex(port, api_minor, true)
}

/// `exit_reboots = false`: plain `exit` leaves the CLI without rebooting or printing
/// `Rebooting` (unknown firmware / lost output).
fn rig_ex(port: Port, api_minor: u8, exit_reboots: bool) -> Rig {
    let (mut fc, sim) = sim::fake(Sim::default());
    fc.api_minor = api_minor;
    fc.exit_reboots = exit_reboots;
    let fw = if api_minor >= 46 { "4.5" } else { "4.4" };
    let fail = Arc::new(Mutex::new(None));
    *fc.cli_backend.lock().unwrap() = Some(Box::new(Cfg {
        mock: MockFc::from_dump(&fixture(fw, "speedybeef405v4")),
        fail: fail.clone(),
    }));
    let reboot_fails = Arc::new(AtomicBool::new(false));
    let mut inner = fc.msp_hook.lock().unwrap().take().expect("sim hook");
    let rf = reboot_fails.clone();
    *fc.msp_hook.lock().unwrap() = Some(Box::new(move |code, payload| {
        if code == codes::MSP_REBOOT && rf.load(Ordering::SeqCst) {
            return Some(HookReply::Error);
        }
        inner(code, payload)
    }));
    let present = Arc::new(AtomicBool::new(true));
    if matches!(port, Port::Usb) {
        *fc.unplug_on_reboot.lock().unwrap() = Some(present.clone());
    }
    let f2 = fc.clone();
    let p2 = present.clone();
    let p3 = present.clone();
    let opener: Opener = Arc::new(move |_path: &str| {
        if !p2.load(Ordering::SeqCst) {
            return Err(DeviceError::NotFound(PORT.into()));
        }
        Ok(Box::new(f2.serial()) as BoxLink)
    });
    let lister: Lister = Arc::new(move || {
        if p3.load(Ordering::SeqCst) {
            vec![PortIdentity {
                path: PORT.into(),
                vid: Some(0x0483),
                pid: Some(0x5740),
                serial_number: Some("0x8000000".into()),
                ..Default::default()
            }]
        } else {
            vec![]
        }
    });
    let m = ConnectionManager::new(opener, lister)
        .with_poll_interval(Duration::from_millis(5))
        .with_msp_timeout(Duration::from_millis(100))
        .with_acc_calibration(sim::fast());
    m.connect(PORT).unwrap();
    Rig {
        fc,
        sim,
        reboot_fails,
        fail,
        present,
        m,
        port,
    }
}

fn model() -> DroneModel {
    let p = format!(
        "{}/tests/fixtures/models/freestyle-5in-analog.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

/// A backup store in a fresh temp dir. Keep the returned `TempDir` alive for as long as the
/// store is used; dropping it removes the directory (also when the test panics).
fn store(tag: &str) -> (tempfile::TempDir, BackupStore) {
    let dir = tempfile::Builder::new()
        .prefix(&format!("bfsw-dirty-{tag}-"))
        .tempdir()
        .unwrap();
    let store = BackupStore::new(dir.path());
    (dir, store)
}

impl Rig {
    fn pending(&self) -> Option<UnsavedChangesReason> {
        self.m.pending_unsaved_changes().unwrap()
    }
    fn msp_reboots(&self) -> usize {
        self.fc.msp_reboots.lock().unwrap().len()
    }
    fn calibration_requests(&self) -> u32 {
        self.sim.lock().unwrap().calibration_requests
    }
    /// Config's apply (plan first); `reject` = the plan's first command is refused by the FC.
    fn apply(&self, reject: bool, tag: &str) -> Result<crate::model::ApplyResult, AppError> {
        let model = model();
        let script = self.m.with_link(|l| session::plan(l, &model)).unwrap();
        assert!(!script.commands.is_empty());
        if reject {
            *self.fail.lock().unwrap() = Some(script.commands[0].clone());
        }
        let (_dir, store) = store(tag);
        let r = self
            .m
            .with_link(|l| session::apply(l, &script, &store, "UID", &mut |_| {}));
        *self.fail.lock().unwrap() = None;
        r
    }
    /// A failed apply whose cleanup reboot (MSP_REBOOT) fails: the FC keeps the rejected
    /// batch's partial changes in RAM.
    fn failed_apply_without_reboot(&self, tag: &str) {
        self.reboot_fails.store(true, Ordering::SeqCst);
        let e = self.apply(true, tag).expect_err("cleanup reboot failed");
        assert_ne!(e.code(), "PENDING_UNSAVED_CHANGES");
        self.reboot_fails.store(false, Ordering::SeqCst);
    }
    fn assert_refused(&self, reason: &str) {
        // Armed board: the dirty-RAM check comes first.
        self.sim.lock().unwrap().armed = true;
        let e = self.m.acc_calibrate().expect_err("refused");
        self.sim.lock().unwrap().armed = false;
        assert_eq!(e.code(), "PENDING_UNSAVED_CHANGES", "{:?}: {e}", self.port);
        assert_eq!(
            e.details().unwrap(),
            serde_json::json!({ "reason": reason })
        );
        assert!(e.to_string().contains("reboot the board first"), "{e}");
        assert_eq!(
            self.calibration_requests(),
            0,
            "MSP_ACC_CALIBRATION not sent"
        );
    }
    /// The user pulls the USB cable (battery connected: RAM survives) and plugs it back in
    /// while the link reconnects. No reboot is issued.
    fn replug_and_reconnect(&self) {
        self.present.store(false, Ordering::SeqCst);
        let p = self.present.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(40));
            p.store(true, Ordering::SeqCst);
        });
        self.m.with_link(|l| l.reconnect(RECONNECT)).unwrap();
        t.join().unwrap();
    }
    fn assert_calibrates(&self) {
        let before = self.calibration_requests();
        self.m
            .acc_calibrate()
            .unwrap_or_else(|e| panic!("{:?}: {e}", self.port));
        assert_eq!(self.calibration_requests(), before + 1);
    }
}

#[test]
fn clean_connection_calibrates() {
    for port in [Port::Usb, Port::Uart] {
        let r = rig(port, 46);
        assert_eq!(r.pending(), None);
        // Reads (MSP and CLI) do not mark RAM.
        r.m.identify().unwrap();
        r.m.get_sensors().unwrap();
        r.m.with_link(|l| session::verify(l, &model())).unwrap();
        assert_eq!(r.pending(), None);
        r.assert_calibrates();
        assert_eq!(r.msp_reboots(), 0, "{port:?}: no reboot needed");
    }
}

#[test]
fn failed_apply_with_unconfirmed_reboot_refuses_calibration() {
    for port in [Port::Usb, Port::Uart] {
        let r = rig(port, 46);
        r.failed_apply_without_reboot(&format!("refuse-{port:?}"));
        assert_eq!(r.pending(), Some(UnsavedChangesReason::CliCommand));
        r.assert_refused("CLI_COMMAND");
        // Still refused on retry (nothing cleared it).
        r.assert_refused("CLI_COMMAND");
    }
}

#[test]
fn ignored_reboot_error_does_not_clear() {
    for port in [Port::Usb, Port::Uart] {
        let r = rig(port, 46);
        r.failed_apply_without_reboot(&format!("ignored-{port:?}"));
        r.reboot_fails.store(true, Ordering::SeqCst);
        r.m.with_link(|l| {
            let _ = l.msp_request(codes::MSP_REBOOT, &[0]); // error ignored
            l.reconnect(RECONNECT) // port never dropped: reopens the same FC
        })
        .unwrap();
        assert_eq!(
            r.pending(),
            Some(UnsavedChangesReason::CliCommand),
            "{port:?}"
        );
        r.assert_refused("CLI_COMMAND");
    }
}

#[test]
fn confirmed_reboot_clears() {
    for port in [Port::Usb, Port::Uart] {
        let r = rig(port, 46);
        r.failed_apply_without_reboot(&format!("reboot-{port:?}"));
        r.m.with_link(|l| {
            l.msp_request(codes::MSP_REBOOT, &[0])?;
            l.reconnect(RECONNECT)
        })
        .unwrap();
        assert_eq!(r.pending(), None, "{port:?}");
        r.assert_calibrates();
    }
}

#[test]
fn reconnect_reboots_a_dirty_board_and_clears() {
    for port in [Port::Usb, Port::Uart] {
        // disconnect + connect
        let r = rig(port, 46);
        r.failed_apply_without_reboot(&format!("reconnect-{port:?}"));
        let reboots = r.msp_reboots();
        r.m.disconnect().unwrap();
        r.m.connect(PORT).unwrap();
        assert_eq!(r.msp_reboots(), reboots + 1, "{port:?}: rebooted once");
        assert_eq!(r.pending(), None);
        r.assert_calibrates();
        // A clean board is not rebooted on connect.
        r.m.disconnect().unwrap();
        r.m.connect(PORT).unwrap();
        assert_eq!(r.msp_reboots(), reboots + 1);

        // connect to the already-open port while dirty reopens it (no early return)
        let r = rig(port, 46);
        r.failed_apply_without_reboot(&format!("reopen-{port:?}"));
        r.m.connect(PORT).unwrap();
        assert_eq!(r.pending(), None);
        r.assert_calibrates();
    }
}

#[test]
fn failed_reboot_on_connect_keeps_the_board_marked() {
    let r = rig(Port::Uart, 46);
    r.failed_apply_without_reboot("connect-fail");
    r.m.disconnect().unwrap();
    r.reboot_fails.store(true, Ordering::SeqCst);
    assert!(r.m.connect(PORT).is_err());
    assert_eq!(
        r.m.pending_unsaved_changes().unwrap_err().code(),
        "NOT_CONNECTED"
    );
    r.reboot_fails.store(false, Ordering::SeqCst);
    r.m.connect(PORT).unwrap();
    assert_eq!(r.pending(), None);
    r.assert_calibrates();
}

#[test]
fn successful_apply_ends_clean() {
    for port in [Port::Usb, Port::Uart] {
        let r = rig(port, 46);
        let res = r.apply(false, &format!("ok-{port:?}")).unwrap();
        assert!(res.saved);
        assert_eq!(r.pending(), None, "{port:?}: acknowledged save + reconnect");
        r.assert_calibrates();
    }
}

#[test]
fn failed_apply_on_4_4() {
    // config's discard is plain `exit`, which prints `Rebooting`: acknowledged, and the
    // reconnect re-verifies the board → confirmed on USB and UART alike.
    for port in [Port::Usb, Port::Uart] {
        let r = rig(port, 45);
        let res = r.apply(true, &format!("44-{port:?}")).unwrap();
        assert!(!res.saved);
        assert_eq!(r.pending(), None, "{port:?}");
        r.assert_calibrates();
    }
}

#[test]
fn usb_replug_without_an_app_reboot_keeps_the_flag() {
    let r = rig(Port::Usb, 46);
    r.failed_apply_without_reboot("replug");
    r.replug_and_reconnect();
    assert_eq!(r.pending(), Some(UnsavedChangesReason::CliCommand));
    r.assert_refused("CLI_COMMAND");
    assert_eq!(
        r.fc.reboots.load(Ordering::SeqCst),
        0,
        "the FC never rebooted"
    );
}

/// Decision: an MSP_REBOOT answered with an error frame is not an acknowledgement, so a port
/// drop after it (whatever caused it) does not confirm anything.
#[test]
fn msp_reboot_error_plus_port_drop_does_not_clear() {
    let r = rig(Port::Usb, 46);
    r.failed_apply_without_reboot("err-drop");
    r.reboot_fails.store(true, Ordering::SeqCst);
    assert!(r
        .m
        .with_link(|l| l.msp_request(codes::MSP_REBOOT, &[0]))
        .is_err());
    r.replug_and_reconnect();
    assert_eq!(r.pending(), Some(UnsavedChangesReason::CliCommand));
    r.assert_refused("CLI_COMMAND");
}

#[test]
fn device_reboot_clears() {
    for port in [Port::Usb, Port::Uart] {
        let r = rig(port, 46);
        r.failed_apply_without_reboot(&format!("devreboot-{port:?}"));
        let reboots = r.msp_reboots();
        r.m.reboot().unwrap_or_else(|e| panic!("{port:?}: {e}"));
        assert_eq!(r.msp_reboots(), reboots + 1, "{port:?}: MSP_REBOOT [0]");
        assert_eq!(r.fc.msp_reboots.lock().unwrap().last().unwrap(), &vec![0u8]);
        assert_eq!(r.pending(), None, "{port:?}");
        r.assert_calibrates();
        // A clean board reboots too (and stays clean).
        r.m.reboot().unwrap();
        assert_eq!(r.msp_reboots(), reboots + 2);
    }
}

#[test]
fn device_reboot_from_an_open_cli_sends_exit_never_save() {
    for api in [45, 46, 47] {
        let r = rig(Port::Uart, api);
        r.m.with_link(|l| l.cli_exec("set gyro_lpf1_static_hz = 250"))
            .unwrap();
        assert_eq!(r.pending(), Some(UnsavedChangesReason::CliCommand));
        r.m.reboot().unwrap_or_else(|e| panic!("api {api}: {e}"));
        assert_eq!(r.pending(), None, "api {api}");
        let lines = r.fc.cli_lines.lock().unwrap().clone();
        assert!(lines.iter().any(|l| l == "exit"), "api {api}: {lines:?}");
        assert!(!lines.iter().any(|l| l.starts_with("save")), "api {api}");
        assert_eq!(r.msp_reboots(), 0, "api {api}: `Rebooting` confirmed it");
    }
}

#[test]
fn unconfirmed_device_reboot_errors() {
    // Dirty: PENDING_UNSAVED_CHANGES, still dirty.
    let r = rig(Port::Usb, 46);
    r.failed_apply_without_reboot("unconfirmed");
    r.reboot_fails.store(true, Ordering::SeqCst);
    let e = r.m.reboot().unwrap_err();
    assert_eq!(e.code(), "PENDING_UNSAVED_CHANGES");
    assert_eq!(
        e.details().unwrap(),
        serde_json::json!({ "reason": "CLI_COMMAND" })
    );
    assert_eq!(r.pending(), Some(UnsavedChangesReason::CliCommand));
    // The link is still usable; a later confirmed reboot clears it.
    r.reboot_fails.store(false, Ordering::SeqCst);
    r.m.reboot().unwrap();
    r.assert_calibrates();

    // Clean: MSP.
    let r = rig(Port::Uart, 46);
    r.reboot_fails.store(true, Ordering::SeqCst);
    assert_eq!(r.m.reboot().unwrap_err().code(), "MSP");
    assert_eq!(r.pending(), None);
}

#[test]
fn msp_write_and_exit_noreboot_mark_ram() {
    let r = rig(Port::Uart, 46);
    // MSP_SET_FEATURE_CONFIG (unknown to the fake: times out, but it was sent).
    let _ = r.m.with_link(|l| l.msp_request(37, &[0, 0, 0, 0]));
    assert_eq!(r.pending(), Some(UnsavedChangesReason::MspWrite));
    r.assert_refused("MSP_WRITE");

    let r = rig(Port::Uart, 46);
    r.m.with_link(|l| {
        l.enter_cli()?;
        l.exit_cli(false)
    })
    .unwrap();
    assert_eq!(r.pending(), Some(UnsavedChangesReason::ExitNoReboot));
    r.assert_refused("EXIT_NOREBOOT");
}

#[test]
fn open_cli_with_changes_is_left_by_rebooting_before_calibration() {
    // A non-read-only CLI line left in an open CLI: calibrate leaves the CLI with plain
    // `exit` (a confirmed reboot), which discards the change, then calibrates.
    let r = rig(Port::Usb, 46);
    r.m.with_link(|l| l.cli_exec("set gyro_lpf1_static_hz = 250"))
        .unwrap();
    assert_eq!(r.pending(), Some(UnsavedChangesReason::CliCommand));
    r.assert_calibrates();
    assert_eq!(r.pending(), None);
}

/// After our `exit` a port drop without `Rebooting` is no proof: the link reboots over MSP
/// and only that reply confirms.
#[test]
fn exit_with_a_port_drop_but_no_rebooting_line_is_not_a_confirmed_reboot() {
    for reboot_ok in [false, true] {
        let r = rig_ex(Port::Usb, 46, false);
        r.m.with_link(|l| l.cli_exec("set gyro_lpf1_static_hz = 250"))
            .unwrap();
        r.reboot_fails.store(!reboot_ok, Ordering::SeqCst);
        // The cable is pulled and replugged right as the link leaves the CLI.
        r.present.store(false, Ordering::SeqCst);
        let p = r.present.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(40));
            p.store(true, Ordering::SeqCst);
        });
        let res = r.m.identify(); // leaves the CLI first
        t.join().unwrap();
        if reboot_ok {
            res.unwrap();
            assert_eq!(r.pending(), None, "acknowledged MSP_REBOOT after `exit`");
            r.assert_calibrates();
        } else {
            // MSP_REBOOT error frame: the fallback fails, the link is detached, and the
            // board stays marked (carried to the next connect).
            assert!(res.is_err());
            r.reboot_fails.store(false, Ordering::SeqCst);
            let before = r.msp_reboots();
            r.m.connect(PORT).unwrap();
            assert_eq!(
                r.msp_reboots(),
                before + 1,
                "connect reboots the marked board"
            );
            assert_eq!(r.pending(), None);
        }
    }
}
