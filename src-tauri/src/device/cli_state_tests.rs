// SPDX-License-Identifier: GPL-3.0-or-later
//! CLI state on the device link: config's read-only commands (read ports, plan, verify,
//! backup) leave the CLI open, because on 4.4 the only way out reboots the FC. The next MSP
//! call (identify, motor test, RC stream) must leave it transparently: plain `exit`, follow
//! the reboot (or MSP_REBOOT if none was seen), re-verify MSP_UID. `save` is never sent.
//!
//! The fake FC answers CLI commands with config's `MockFc` (real 4.4 / 4.5 / 2025.12 dumps)
//! and MSP with `motor::fake::FakeFc`.

use super::error::DeviceError;
use super::manager::{BoxLink, ConnectionManager, Lister, Opener, PortIdentity};
use super::motor::fake::{CliBackend, FakeFc};
use super::motor::MotorTestRequest;
use super::msp::codes;
use crate::config::mock::{fixture, MockFc};
use crate::config::session;
use crate::device::Connection;
use crate::model::DroneModel;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const PORT: &str = "COM7";

/// Betaflight line the fake FC behaves like.
#[derive(Clone, Copy, Debug)]
enum Fw {
    /// API 1.45: `exit` reboots, no `exit noreboot`.
    V4_4,
    /// API 1.46: plain `exit` reboots too (`cliExitCmd` without `noreboot`).
    V4_5,
    /// API 1.47 (2025.12), same CLI exit code as 4.5.
    V2025_12,
    /// 4.5 board whose `exit` leaves the CLI without rebooting (what `exit noreboot`
    /// does): recovery must reboot it over MSP so RAM is discarded.
    V4_5NoReboot,
}

impl Fw {
    fn dump(self) -> String {
        let fw = match self {
            Fw::V4_4 => "4.4",
            Fw::V4_5 | Fw::V4_5NoReboot => "4.5",
            Fw::V2025_12 => "2025.12",
        };
        fixture(fw, "speedybeef405v4")
    }
    fn api_minor(self) -> u8 {
        match self {
            Fw::V4_4 => 45,
            Fw::V4_5 | Fw::V4_5NoReboot => 46,
            Fw::V2025_12 => 47,
        }
    }
    const ALL: [Fw; 4] = [Fw::V4_4, Fw::V4_5, Fw::V2025_12, Fw::V4_5NoReboot];
}

/// Config's MockFc as the fake FC's CLI.
struct Cfg(MockFc);

impl CliBackend for Cfg {
    fn enter(&mut self) {
        let _ = self.0.enter_cli();
    }
    fn exec(&mut self, line: &str) -> String {
        self.0
            .cli_exec(line)
            .unwrap_or_else(|e| format!("###ERROR: {e}###"))
    }
    fn reboot(&mut self) {
        // Leave the CLI (if in it) and power-cycle: live config = saved config.
        let _ = self.0.exit_cli(false);
        let _ = self.0.msp_request(session::MSP_SET_REBOOT, &[0]);
    }
}

struct Rig {
    fc: FakeFc,
    present: Arc<AtomicBool>,
    /// Port gone for good (independent of the reboot's unplug/replug).
    dead: Arc<AtomicBool>,
    m: ConnectionManager,
}

fn rig(fw: Fw) -> Rig {
    let fc = FakeFc {
        api_minor: fw.api_minor(),
        exit_reboots: !matches!(fw, Fw::V4_5NoReboot),
        ..FakeFc::default()
    };
    *fc.cli_backend.lock().unwrap() = Some(Box::new(Cfg(MockFc::from_dump(&fw.dump()))));
    let present = Arc::new(AtomicBool::new(true));
    *fc.unplug_on_reboot.lock().unwrap() = Some(present.clone());
    let dead = Arc::new(AtomicBool::new(false));
    let f2 = fc.clone();
    let p2 = present.clone();
    let d2 = dead.clone();
    let opener: Opener = Arc::new(move |_path: &str| {
        if !p2.load(Ordering::SeqCst) || d2.load(Ordering::SeqCst) {
            return Err(DeviceError::NotFound(PORT.into()));
        }
        Ok(Box::new(f2.serial()) as BoxLink)
    });
    let p3 = present.clone();
    let d3 = dead.clone();
    let lister: Lister = Arc::new(move || {
        if p3.load(Ordering::SeqCst) && !d3.load(Ordering::SeqCst) {
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
        .with_msp_timeout(Duration::from_millis(100));
    m.connect(PORT).unwrap();
    Rig {
        fc,
        present,
        dead,
        m,
    }
}

fn model() -> DroneModel {
    let p = format!(
        "{}/tests/fixtures/models/freestyle-5in-analog.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

impl Rig {
    fn sent(&self) -> String {
        String::from_utf8_lossy(&self.fc.rx_log.lock().unwrap()).into_owned()
    }
    /// Plain `exit` lines the FC received in CLI mode.
    fn exits(&self) -> usize {
        self.fc
            .cli_lines
            .lock()
            .unwrap()
            .iter()
            .filter(|l| *l == "exit")
            .count()
    }
    fn msp_reboots(&self) -> Vec<Vec<u8>> {
        self.fc.msp_reboots.lock().unwrap().clone()
    }
    fn in_cli(&self) -> bool {
        self.fc.cli.load(Ordering::SeqCst)
    }
    /// Config's verify: enters the CLI, reads `version` + `dump all`, leaves it open.
    fn verify(&self) {
        let model = model();
        self.m
            .with_link(|l| session::verify(l, &model))
            .expect("verify");
        assert!(self.in_cli(), "verify leaves the CLI open");
    }
    /// An MSP_UID request was sent after the last `exit` (board re-verified).
    fn uid_checked_after_exit(&self) -> bool {
        let log = self.fc.rx_log.lock().unwrap().clone();
        let Some(at) = log.windows(4).rposition(|w| w == b"exit") else {
            return false;
        };
        let uid = [b'$', b'M', b'<', 0, codes::MSP_UID as u8];
        log[at..].windows(uid.len()).any(|w| w == uid)
    }
    /// One clean transition: exactly one `exit`, no `save`, MSP_REBOOT only when the exit
    /// did not reboot, back in MSP mode with the board re-verified.
    fn assert_one_clean_transition(&self, fw: Fw) {
        assert_eq!(self.exits(), 1, "{fw:?}: one exit: {}", self.sent());
        assert!(!self.sent().contains("save"), "{fw:?}: never save");
        assert!(!self.in_cli(), "{fw:?}: CLI left");
        let want: Vec<Vec<u8>> = match fw {
            Fw::V4_5NoReboot => vec![vec![0]],
            _ => vec![],
        };
        assert_eq!(
            self.msp_reboots(),
            want,
            "{fw:?}: MSP_REBOOT [0] only if needed"
        );
        assert!(self.uid_checked_after_exit(), "{fw:?}: MSP_UID re-verified");
        assert_eq!(
            self.fc.reboots.load(Ordering::SeqCst),
            1,
            "{fw:?}: one reboot"
        );
    }
}

fn motor_req() -> MotorTestRequest {
    MotorTestRequest {
        motor_index: Some(0),
        throttle: 1100,
        props_off_confirmed: true,
    }
}

#[test]
fn verify_then_motor_test() {
    for fw in Fw::ALL {
        let r = rig(fw);
        r.verify();
        let s =
            r.m.motor_test_start(motor_req(), None, |_| {})
                .unwrap_or_else(|e| panic!("{fw:?}: {e}"));
        assert_eq!(s.output_config.motor_count, 4);
        r.m.motor_test_stop().unwrap();
        r.assert_one_clean_transition(fw);
    }
}

#[test]
fn verify_then_rc_stream() {
    for fw in Fw::ALL {
        let r = rig(fw);
        r.verify();
        let (tx, rx) = std::sync::mpsc::channel();
        r.m.rc_stream_start(
            Some(50),
            move |c| {
                let _ = tx.send(c);
            },
            |_| {},
        )
        .unwrap_or_else(|e| panic!("{fw:?}: {e}"));
        let ch = rx.recv_timeout(Duration::from_secs(2)).expect("RC frame");
        assert_eq!(ch.channels[0], 1500);
        r.m.rc_stream_stop().unwrap();
        r.assert_one_clean_transition(fw);
    }
}

#[test]
fn verify_then_identify() {
    for fw in Fw::ALL {
        let r = rig(fw);
        r.verify();
        let info = r.m.identify().unwrap_or_else(|e| panic!("{fw:?}: {e}"));
        assert_eq!(info.uid.as_deref(), Some("000000010000000200000003"));
        r.assert_one_clean_transition(fw);
        // Already in MSP mode: no second exit.
        r.m.identify().unwrap();
        assert_eq!(r.exits(), 1);
    }
}

#[test]
fn one_exit_per_transition() {
    let r = rig(Fw::V4_4);
    r.verify();
    r.m.identify().unwrap();
    r.verify();
    r.verify(); // CLI already open: no exit, no re-enter
    r.m.identify().unwrap();
    r.m.identify().unwrap();
    assert_eq!(r.exits(), 2, "{}", r.sent());
    assert_eq!(r.fc.reboots.load(Ordering::SeqCst), 2);
    assert!(!r.sent().contains("save"));
}

#[test]
fn msp_without_cli_sends_no_exit() {
    for fw in Fw::ALL {
        let r = rig(fw);
        r.m.identify().unwrap();
        r.m.motor_test_start(motor_req(), None, |_| {}).unwrap();
        r.m.motor_test_stop().unwrap();
        r.m.rc_stream_start(Some(50), |_| {}, |_| {}).unwrap();
        r.m.rc_stream_stop().unwrap();
        r.m.with_link(|l| l.msp_request(codes::MSP_API_VERSION, &[]))
            .unwrap();
        assert_eq!(r.exits(), 0, "{fw:?}");
        assert!(!r.sent().contains("exit"), "{fw:?}");
        assert!(r.msp_reboots().is_empty(), "{fw:?}");
        assert_eq!(r.fc.reboots.load(Ordering::SeqCst), 0, "{fw:?}");
    }
}

/// Config's own exits clear the CLI state: nothing extra is sent afterwards.
#[test]
fn config_exit_and_discard_clear_cli_state() {
    // 4.4: exit_cli(false) = `exit` (reboot), then reconnect.
    let r = rig(Fw::V4_4);
    r.verify();
    r.m.with_link(|l| {
        l.exit_cli(false)?;
        l.reconnect(Duration::from_secs(2))
    })
    .unwrap();
    r.m.identify().unwrap();
    assert_eq!(r.exits(), 1, "only config's own exit");
    assert!(r.msp_reboots().is_empty());

    // 4.5+: discard = `exit noreboot` + MSP_SET_REBOOT, then reconnect.
    for fw in [Fw::V4_5, Fw::V2025_12] {
        let r = rig(fw);
        r.verify();
        r.m.with_link(|l| {
            l.exit_cli(false)?;
            l.msp_request(session::MSP_SET_REBOOT, &[0])?;
            l.reconnect(Duration::from_secs(2))
        })
        .unwrap();
        r.m.identify().unwrap();
        assert_eq!(r.exits(), 0, "{fw:?}: no implicit exit");
        assert!(r.sent().contains("exit noreboot"));
        assert_eq!(
            r.msp_reboots(),
            vec![vec![0]],
            "{fw:?}: only config's reboot"
        );
        assert!(!r.sent().contains("save"));
    }
}

/// A board that never leaves the CLI: one exit, the right error, and no CLI state left
/// behind (the next call does not send a second exit).
#[test]
fn failed_exit_is_a_serial_error_and_leaves_no_cli_state() {
    let r = rig(Fw::V4_4);
    r.verify();
    // The board never comes back after the exit (unplugged / wedged bootloader).
    r.dead.store(true, Ordering::SeqCst);
    let t = Instant::now();
    let e = r.m.identify().unwrap_err();
    assert_eq!(e.code(), "SERIAL", "{e}");
    assert!(e.to_string().contains("after leaving the CLI"), "{e}");
    assert!(
        t.elapsed() < Duration::from_secs(20),
        "bounded by the reconnect timeout"
    );
    assert_eq!(r.exits(), 1);
    // Link is detached, not "in CLI": no second exit, a reconnect hint instead.
    r.dead.store(false, Ordering::SeqCst);
    let e2 = r.m.identify().unwrap_err();
    assert!(e2.to_string().contains("reconnect"), "{e2}");
    assert_eq!(r.exits(), 1);
    // And an explicit reconnect brings it back.
    r.m.with_link(|l| l.reconnect(Duration::from_secs(2)))
        .unwrap();
    r.m.identify().unwrap();
    assert!(!r.sent().contains("save"));
}

#[test]
fn sticky_cli_gives_one_exit_then_a_timeout() {
    let fc = FakeFc {
        api_minor: 45,
        cli_sticky: true,
        ..FakeFc::default()
    };
    *fc.cli_backend.lock().unwrap() = Some(Box::new(Cfg(MockFc::from_dump(&Fw::V4_4.dump()))));
    let present = Arc::new(AtomicBool::new(true));
    let f2 = fc.clone();
    let opener: Opener = Arc::new(move |_p: &str| Ok(Box::new(f2.serial()) as BoxLink));
    let lister: Lister = Arc::new(move || {
        vec![PortIdentity {
            path: PORT.into(),
            ..Default::default()
        }]
    });
    let m = ConnectionManager::new(opener, lister)
        .with_poll_interval(Duration::from_millis(5))
        .with_msp_timeout(Duration::from_millis(100));
    m.connect(PORT).unwrap();
    let model = model();
    m.with_link(|l| session::verify(l, &model)).unwrap();
    let e = m.identify().unwrap_err();
    assert_eq!(e.code(), "SERIAL", "{e}");
    assert!(e.to_string().contains("timed out"), "{e}");
    let exits = fc
        .cli_lines
        .lock()
        .unwrap()
        .iter()
        .filter(|l| *l == "exit")
        .count();
    assert_eq!(exits, 1);
    assert!(fc.msp_reboots.lock().unwrap().is_empty());
    let sent = String::from_utf8_lossy(&fc.rx_log.lock().unwrap()).into_owned();
    assert!(!sent.contains("save"));
    drop(present);
}

/// A different board on the port after the reboot: refused (SERIAL), UID re-verified.
#[test]
fn different_board_after_cli_exit_is_refused() {
    for fw in [Fw::V4_4, Fw::V4_5] {
        let r = rig(fw);
        r.verify();
        *r.fc.uid.lock().unwrap() = [9, 0, 0, 0, 9, 0, 0, 0, 9, 0, 0, 0];
        let e = r.m.identify().unwrap_err();
        assert_eq!(e.code(), "SERIAL", "{fw:?}: {e}");
        assert!(e.to_string().contains("different flight controller"), "{e}");
        assert!(e.to_string().contains("000000010000000200000003"), "{e}");
        assert_eq!(r.exits(), 1);
    }
}

/// The emergency stop never waits for a CLI exit + reboot in progress.
#[test]
fn emergency_stop_does_not_wait_for_a_cli_exit() {
    let mut r = rig(Fw::V4_4);
    r.fc.reboot_ms = 400; // slow re-enumeration
                          // Rebuild with the slower FakeFc so its reboots keep the port away for 400 ms.
    let fc = r.fc.clone();
    r = Rig {
        m: {
            let f2 = fc.clone();
            let p2 = r.present.clone();
            let p3 = r.present.clone();
            let opener: Opener = Arc::new(move |_p: &str| {
                if !p2.load(Ordering::SeqCst) {
                    return Err(DeviceError::NotFound(PORT.into()));
                }
                Ok(Box::new(f2.serial()) as BoxLink)
            });
            let lister: Lister = Arc::new(move || {
                if p3.load(Ordering::SeqCst) {
                    vec![PortIdentity {
                        path: PORT.into(),
                        ..Default::default()
                    }]
                } else {
                    vec![]
                }
            });
            let m = ConnectionManager::new(opener, lister)
                .with_poll_interval(Duration::from_millis(5))
                .with_msp_timeout(Duration::from_millis(100));
            m.connect(PORT).unwrap();
            m
        },
        fc,
        present: r.present,
        dead: r.dead,
    };
    r.verify();
    let m = Arc::new(r.m);
    let m2 = m.clone();
    let worker = std::thread::spawn(move || m2.identify());
    // Wait until the transition has started (port unplugged by the reboot).
    let t0 = Instant::now();
    while r.present.load(Ordering::SeqCst) && t0.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!r.present.load(Ordering::SeqCst), "exit rebooted the FC");
    let t = Instant::now();
    let had_motor_test = m.emergency_stop_all();
    assert!(
        t.elapsed() < Duration::from_millis(50),
        "emergency stop waited {:?}",
        t.elapsed()
    );
    assert!(!had_motor_test);
    worker.join().unwrap().unwrap();
}

/// `bl rom` straight from the CLI: no exit/reboot/MSP round first.
#[test]
fn bootloader_from_open_cli_uses_bl_without_exit() {
    let r = rig(Fw::V4_4);
    r.verify();
    r.m.reboot_to_bootloader().unwrap();
    assert_eq!(r.exits(), 0);
    assert!(r.fc.cli_lines.lock().unwrap().iter().any(|l| l == "bl rom"));
    assert!(!r.m.is_connected());
}

/// `Connection::uid` is the cached `FcInfo.uid`: no MSP round-trip, works in CLI mode.
#[test]
fn link_uid_is_cached() {
    let r = rig(Fw::V4_4);
    let before = r.fc.rx_log.lock().unwrap().len();
    let uid = r.m.with_link(|l| Ok(l.uid())).unwrap();
    assert_eq!(uid.as_deref(), Some("000000010000000200000003"));
    assert_eq!(r.fc.rx_log.lock().unwrap().len(), before, "no I/O");
    r.verify();
    let uid = r.m.with_link(|l| Ok(l.uid())).unwrap();
    assert_eq!(uid.as_deref(), Some("000000010000000200000003"));
    assert!(r.in_cli(), "reading the uid does not leave the CLI");
    assert_eq!(r.exits(), 0);
}

/// `FcInfo.hdOsdSupported` end to end (identify + CLI probe) on each firmware line, with the
/// real SpeedyBee F405 V4 dumps as the CLI. The fake FC's MSP_BUILD_INFO carries
/// DSHOT/OSD_SD/OSD_HD on 4.5+ and nothing on 4.4; its MSP_OSD_CANVAS answers 53x20.
#[test]
fn probe_board_reports_hd_osd_per_version() {
    for (fw, want) in [
        (Fw::V4_4, None),
        (Fw::V4_5, Some(true)),
        (Fw::V2025_12, Some(true)),
    ] {
        let r = rig(fw);
        let info = r.m.identify().unwrap();
        assert_eq!(info.hd_osd_supported, want, "{fw:?} identify");
        let info = r.m.probe_board().unwrap();
        // 4.4: the dump has displayport_msp_col_adjust, which SD-only builds have too → null.
        assert_eq!(info.hd_osd_supported, want, "{fw:?} probe");
        assert!(
            r.fc.cli_lines
                .lock()
                .unwrap()
                .iter()
                .any(|l| l == "get displayport_msp_col_adjust"),
            "{fw:?}"
        );
        let names = if want.is_some() {
            vec!["USE_DSHOT", "USE_OSD_SD", "USE_OSD_HD"]
        } else {
            vec![]
        };
        assert_eq!(info.build_options, names, "{fw:?}");
        assert!(!r.sent().contains("save"), "{fw:?}");
    }

    // 4.4 build without MSP DisplayPort (no OSD): the CLI rejects the name → false.
    let r = rig(Fw::V4_4);
    let dump: String = Fw::V4_4
        .dump()
        .lines()
        .filter(|l| !l.contains("displayport_msp_col_adjust"))
        .map(|l| format!("{l}\n"))
        .collect();
    *r.fc.cli_backend.lock().unwrap() = Some(Box::new(Cfg(MockFc::from_dump(&dump))));
    assert_eq!(r.m.identify().unwrap().hd_osd_supported, None);
    assert_eq!(r.m.probe_board().unwrap().hd_osd_supported, Some(false));
}
