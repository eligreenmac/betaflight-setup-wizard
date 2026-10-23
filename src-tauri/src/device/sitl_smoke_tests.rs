// SPDX-License-Identifier: GPL-3.0-or-later
//! Opt-in smoke test of the device read path (MSP identify / status / sensors, CLI probe and
//! exits) against a real Betaflight SITL. Ignored by default; run with
//!
//! ```text
//! tests/sitl/build-sitl.sh 2026.6.2 /tmp/bfsw-sitl     # prints the ELF path
//! BFSW_SITL_BIN=<elf> cargo test --lib device::sitl_smoke_tests -- --ignored --nocapture
//! ```
//!
//! Works for any line the wizard supports (4.4.3, 4.5.5, 2025.12.5, 2026.6.2); the
//! 2026.6-specific checks (API 1.48, MCU name via MSP2_MCU_INFO) apply when the SITL reports
//! API 1.48+. SITL's UART1 is TCP 5761 and serves MSP and the CLI; its "reboot" is the
//! process exiting. The port is fixed, so do not run this alongside another SITL test
//! (`config::tests_sitl_e2e`) on the same machine.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::device::cli::{CliOptions, CliSession};
use crate::device::error::{DeviceError, Result};
use crate::device::msp::{codes, MspClient, MCU_TYPE_ID_PROVIDED_BY_NAME};
use crate::device::serial::SerialLink;

const PORT: u16 = 5761;

/// `SerialLink` over the SITL TCP serial bridge.
struct TcpLink(TcpStream);

impl SerialLink for TcpLink {
    fn write_all(&mut self, data: &[u8]) -> Result<()> {
        self.0
            .write_all(data)
            .map_err(|e| DeviceError::Serial(e.to_string()))
    }
    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize> {
        self.0
            .set_read_timeout(Some(timeout.max(Duration::from_millis(1))))
            .map_err(|e| DeviceError::Serial(e.to_string()))?;
        match self.0.read(buf) {
            Ok(n) => Ok(n),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                Ok(0)
            }
            Err(e) => Err(DeviceError::Serial(e.to_string())),
        }
    }
    fn clear_input(&mut self) -> Result<()> {
        let mut b = [0u8; 4096];
        while self.read(&mut b, Duration::from_millis(20))? > 0 {}
        Ok(())
    }
    fn name(&self) -> &str {
        "sitl:5761"
    }
}

struct Sitl {
    child: Child,
}

impl Sitl {
    fn start(bin: &PathBuf, dir: &std::path::Path) -> (Sitl, TcpLink) {
        // SITL's port is fixed; on a shared machine another SITL test may hold it.
        assert!(
            TcpStream::connect(("127.0.0.1", PORT)).is_err(),
            "TCP {PORT} is already in use (another SITL running?); retry when it is free"
        );
        // Wrapped at once so Drop kills and reaps it on every path.
        let sitl = Sitl {
            child: Command::new(bin)
                .current_dir(dir)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("start SITL"),
        };
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(s) = TcpStream::connect(("127.0.0.1", PORT)) {
                std::thread::sleep(Duration::from_millis(500));
                return (sitl, TcpLink(s));
            }
            assert!(Instant::now() < end, "SITL did not open TCP {PORT}");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn exited_within(&mut self, t: Duration) -> bool {
        let end = Instant::now() + t;
        while Instant::now() < end {
            if self.child.try_wait().ok().flatten().is_some() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }
}

impl Drop for Sitl {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
#[ignore = "needs BFSW_SITL_BIN (a Betaflight SITL ELF, tests/sitl/build-sitl.sh)"]
fn sitl_msp_and_cli_read_path() {
    let Some(bin) = std::env::var_os("BFSW_SITL_BIN").map(PathBuf::from) else {
        eprintln!("BFSW_SITL_BIN not set; skipped");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (mut sitl, link) = Sitl::start(&bin, dir.path());

    // --- MSP identify (the connect path).
    let mut msp = MspClient::new(link);
    let id = msp.identify().expect("identify");
    eprintln!(
        "SITL: {} {} API {} board {:?} mcu id {:?} mcu name {:?} options {:?} hd {:?}",
        id.fc_variant,
        id.fc_version,
        id.api_version,
        id.board_info.board_name,
        id.board_info.mcu_type_id,
        id.mcu_name,
        id.build_options,
        id.hd_osd_supported
    );
    assert_eq!(id.fc_variant, "BTFL");
    assert!(id.is_supported_betaflight());
    let api = id.api_version;
    if api.at_least(1, 48) {
        assert!(id.fc_version.starts_with("2026."), "{}", id.fc_version);
        assert_eq!(
            id.board_info.mcu_type_id,
            Some(MCU_TYPE_ID_PROVIDED_BY_NAME)
        );
        // SITL's TARGET_MCU.
        assert_eq!(id.mcu_name.as_deref(), Some("SIMULATOR"));
        let info = crate::device::manager::fc_info(&id);
        assert_eq!(info.mcu.as_deref(), Some("SIMULATOR"));
    }
    if api.at_least(1, 47) {
        let ports = id.serial_config.as_ref().expect("serial config");
        assert_eq!(
            ports.scheme,
            crate::device::serial_ports::IdentifierScheme::UartBase50
        );
    }

    // --- Status / sensors (read-only MSP used by the calibration and motor screens).
    let st = crate::device::sensors::read_status(&mut msp).expect("status");
    eprintln!("status: {st:?}");
    let sensors = crate::device::sensors::read_sensors(&mut msp).expect("sensors");
    eprintln!("sensors: {sensors:?}");
    assert!(sensors.gyro.present, "SITL has a virtual gyro");
    assert!(crate::device::ram_state::msp_is_read_only(
        codes::MSP_STATUS_EX
    ));

    // --- CLI: version banner, probe, diff, exit noreboot keeps MSP alive.
    let link = msp.into_inner();
    let mut cli = CliSession::enter(link, CliOptions::default()).expect("enter CLI");
    let version = cli.run("version").unwrap();
    eprintln!("version: {version}");
    assert!(version.contains("# Betaflight / SITL"), "{version}");
    let probe = cli.probe_board().unwrap();
    assert!(probe.serial.contains("serial "), "{}", probe.serial);
    let diff = cli.run("diff all").unwrap();
    assert!(diff.contains("# version"), "{diff}");
    // A fresh SITL has no changed settings, so only the batch frame is guaranteed.
    assert!(diff.contains("# start the command batch"), "{diff}");
    if api.at_least(1, 46) {
        let link = cli.exit_no_reboot().unwrap();
        assert!(!sitl.exited_within(Duration::from_millis(500)));
        let mut msp = MspClient::new(link);
        assert_eq!(msp.api_version().unwrap(), api, "MSP after exit noreboot");
        let link = msp.into_inner();
        let cli = CliSession::enter(link, CliOptions::default()).unwrap();
        let (_link, rebooting) = cli.exit_reporting_reboot().unwrap();
        assert!(rebooting, "plain exit prints Rebooting");
    } else {
        let (_link, rebooting) = cli.exit_reporting_reboot().unwrap();
        assert!(rebooting);
    }
    assert!(
        sitl.exited_within(Duration::from_secs(5)),
        "SITL exits on its reboot"
    );
}
