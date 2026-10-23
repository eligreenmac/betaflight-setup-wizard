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
    let flags = st.arming_disable_flags.expect("STATUS_EX arming flags");
    let (names, unknown) = crate::device::arming::decode(flags, api);
    eprintln!("arming disabled: {names:?}");
    assert_eq!(
        unknown, 0,
        "arming flags outside this line's table: {flags:#x}"
    );
    assert!(names.contains(&"RXLOSS"), "SITL has no RX: {names:?}");
    let sensors = crate::device::sensors::read_sensors(&mut msp).expect("sensors");
    eprintln!("sensors: {sensors:?}");
    assert!(sensors.gyro.present, "SITL has a virtual gyro");
    if api.at_least(1, 46) {
        // Catches a wrong per-line table: SITL's gyro/acc/baro are the VIRTUAL drivers.
        assert_eq!(sensors.gyro.detected_hardware.as_deref(), Some("VIRTUAL"));
        assert_eq!(sensors.acc.detected_hardware.as_deref(), Some("VIRTUAL"));
        if sensors.baro.present {
            assert_eq!(sensors.baro.detected_hardware.as_deref(), Some("VIRTUAL"));
        }
    }
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

/// MSP_ANALOG (not otherwise used by the wizard), part of a typical status poll.
const MSP_ANALOG: u16 = 110;

fn rate(n: usize, t: Duration) -> f64 {
    n as f64 / t.as_secs_f64()
}

fn percentile(v: &mut [Duration], p: f64) -> Duration {
    v.sort_unstable();
    v[((v.len() - 1) as f64 * p) as usize]
}

/// Measures what the attitude stream can get out of an FC (numbers behind the 50 Hz cap
/// and 30 Hz default in docs/ARCHITECTURE.md §4.3). Uses TCP 5761 like the smoke test, so
/// it asserts the port is free first. Ignored like the smoke test:
///
/// ```text
/// BFSW_SITL_BIN=<elf> cargo test --lib device::sitl_smoke_tests::sitl_attitude_throughput \
///     -- --ignored --nocapture
/// ```
#[test]
#[ignore = "needs BFSW_SITL_BIN (a Betaflight SITL ELF, tests/sitl/build-sitl.sh)"]
fn sitl_attitude_throughput() {
    use crate::device::attitude::{AttitudeStream, AttitudeStreamEvent, AttitudeStreamOptions};
    use crate::device::sensors;
    use std::sync::mpsc;

    let Some(bin) = std::env::var_os("BFSW_SITL_BIN").map(PathBuf::from) else {
        eprintln!("BFSW_SITL_BIN not set; skipped");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (_sitl, link) = Sitl::start(&bin, dir.path());
    let mut msp = MspClient::new(link);
    let id = msp.identify().expect("identify");
    eprintln!("SITL {} API {}", id.fc_version, id.api_version);
    let window = Duration::from_secs(3);

    // 1. MSP_ATTITUDE alone, back to back.
    let mut lat = Vec::new();
    let t0 = Instant::now();
    while t0.elapsed() < window {
        let t = Instant::now();
        msp.request(codes::MSP_ATTITUDE, &[]).expect("attitude");
        lat.push(t.elapsed());
    }
    let alone = rate(lat.len(), t0.elapsed());
    eprintln!(
        "attitude alone: {alone:.0} Hz, latency p50 {:?} p99 {:?}",
        percentile(&mut lat, 0.5),
        percentile(&mut lat, 0.99)
    );

    // 2. Back to back, with a status poll (STATUS_EX + RC + ANALOG) after every 3rd.
    let (mut n, mut other) = (0usize, 0usize);
    let t0 = Instant::now();
    while t0.elapsed() < window {
        msp.request(codes::MSP_ATTITUDE, &[]).expect("attitude");
        n += 1;
        if n % 3 == 0 {
            for code in [codes::MSP_STATUS_EX, codes::MSP_RC, MSP_ANALOG] {
                msp.request(code, &[]).expect("status poll");
                other += 1;
            }
        }
    }
    let el = t0.elapsed();
    eprintln!(
        "attitude with status polling: {:.0} Hz attitude + {:.0} Hz other",
        rate(n, el),
        rate(other, el)
    );

    // 3. The stream alone, then with get_sensors-style reads borrowing the link at 5 Hz
    // (each borrow pauses the stream for its ~7 requests).
    for (rate_hz, sensors_hz) in [(30, 0), (50, 0), (30, 5), (50, 5)] {
        let link = msp.into_inner();
        let (tx, rx) = mpsc::channel();
        let stream = AttitudeStream::start(
            link,
            AttitudeStreamOptions {
                rate_hz,
                ..Default::default()
            },
            move |ev: AttitudeStreamEvent| {
                let _ = tx.send(ev);
            },
        );
        let mut borrow = Vec::new();
        let t0 = Instant::now();
        while t0.elapsed() < window {
            if sensors_hz == 0 {
                std::thread::sleep(window);
                continue;
            }
            std::thread::sleep(Duration::from_millis(1000 / sensors_hz));
            let t = Instant::now();
            let link = stream.take_link().expect("link");
            let mut c = MspClient::new(link);
            sensors::read_sensors(&mut c).expect("sensors while streaming");
            stream.return_link(c.into_inner());
            borrow.push(t.elapsed());
        }
        let el = t0.elapsed();
        msp = MspClient::new(stream.stop().expect("link back"));
        let got = rate(
            rx.try_iter()
                .filter(|e| matches!(e, AttitudeStreamEvent::Attitude(_)))
                .count(),
            el,
        );
        if sensors_hz == 0 {
            eprintln!("stream at {rate_hz} Hz alone: {got:.1} Hz delivered");
            // Never above what the FC answers back to back (older SITLs answer slower).
            let can = (rate_hz as f64).min(alone);
            assert!(
                got >= can * 0.7,
                "stream fell short at {rate_hz} Hz: {got:.1} Hz"
            );
        } else {
            eprintln!(
                "stream at {rate_hz} Hz + sensors at {sensors_hz} Hz: {got:.1} Hz delivered, \
                 sensors call p50 {:?} max {:?}",
                percentile(&mut borrow, 0.5),
                borrow.iter().max().unwrap()
            );
            assert!(*borrow.iter().max().unwrap() < Duration::from_millis(500));
        }
    }
    // Betaflight answers one MSP command per 100 Hz serial task pass (fewer on the 4.4/4.5
    // SITLs, whose scheduler timing differs from hardware).
    assert!(alone > 0.0);
}
