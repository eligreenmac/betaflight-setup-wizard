//! Opt-in end-to-end test against a running Betaflight SITL (C1 / #12).
//!
//! Set `BFSW_SITL_BIN` to a `betaflight_SITL.elf` (build one with
//! `tests/sitl/build-sitl.sh <tag>`); without it the test is skipped. The SITL's UART1
//! listens on TCP 5761 and serves both MSP and the CLI. `save`/`exit` make the process
//! exit (its "reboot"); `Reconnect` starts it again in the same directory, so
//! `eeprom.bin` persists like flash.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::backup::BackupStore;
use super::firmware::Dialect;
use super::msp;
use super::session;
use super::vtx_table::VtxTableState;
use crate::device::cli::{has_prompt_at_end, CliOptions};
use crate::device::{Connection, Reconnect};
use crate::error::{AppError, AppResult};
use crate::model::CliScript;

const PORT: u16 = 5761;

/// Something already listens on TCP [`PORT`].
fn port_busy() -> bool {
    TcpStream::connect_timeout(&([127, 0, 0, 1], PORT).into(), Duration::from_millis(200)).is_ok()
}

/// Wait up to 3 s for [`PORT`] to be free (our own SITL may still be exiting). Another
/// process holding it (e.g. Device's SITL smoke test on the shared box) is an error: the
/// test would otherwise talk to the wrong simulator. Never run two SITL tests at once.
fn ensure_port_free() -> AppResult<()> {
    for _ in 0..30 {
        if !port_busy() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(AppError::Cli(format!(
        "SITL: TCP {PORT} is already in use (another SITL running?); stop it first"
    )))
}

struct SitlFc {
    bin: PathBuf,
    dir: PathBuf,
    child: Option<Child>,
    sock: Option<TcpStream>,
    in_cli: bool,
    /// The last `save`/`exit` made the process exit on its own (Betaflight's reboot).
    rebooted_itself: bool,
    /// 4.5+: `exit_cli(false)` sends `exit noreboot` (device contract, ARCHITECTURE §4.1).
    noreboot_exit: bool,
}

fn io(e: std::io::Error) -> AppError {
    AppError::Cli(format!("SITL: {e}"))
}

/// Read one CLI response: up to a prompt (`# ` at the start of a line) followed by the
/// app's quiet period (`CliOptions::quiet`, 100 ms) of silence, like `device::cli`'s
/// reader. `dump all` has comment lines (`# feature` ...), so a TCP read can end in
/// `\r\n# ` mid-dump; stopping there truncated the dump. Returns what arrived by `timeout`
/// (or EOF).
fn read_cli_response(s: &mut TcpStream, timeout: Duration) -> AppResult<Vec<u8>> {
    let quiet = CliOptions::default().quiet;
    s.set_read_timeout(Some(Duration::from_millis(20)))
        .map_err(io)?;
    let (mut buf, end) = (Vec::new(), Instant::now() + timeout);
    let mut prompt_since: Option<Instant> = None;
    let mut chunk = [0u8; 65536];
    while Instant::now() < end {
        if prompt_since.is_some_and(|t| t.elapsed() >= quiet) {
            break;
        }
        match s.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                prompt_since = has_prompt_at_end(&buf).then(Instant::now);
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
    }
    Ok(buf)
}

impl SitlFc {
    fn start(bin: PathBuf, dir: PathBuf) -> AppResult<SitlFc> {
        let mut fc = SitlFc {
            bin,
            dir,
            child: None,
            sock: None,
            in_cli: false,
            rebooted_itself: false,
            noreboot_exit: false,
        };
        fc.reconnect(session::REBOOT_TIMEOUT)?;
        Ok(fc)
    }

    fn stop(&mut self) {
        self.sock = None;
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    fn sock(&mut self) -> AppResult<&mut TcpStream> {
        self.sock
            .as_mut()
            .ok_or_else(|| AppError::Cli("SITL: not connected".into()))
    }

    /// Read whatever arrives within `timeout` (or until EOF). MSP frames and the bytes after
    /// `exit` / `save`; CLI responses use [`read_cli_response`].
    fn read(&mut self, timeout: Duration) -> AppResult<Vec<u8>> {
        let s = self.sock()?;
        s.set_read_timeout(Some(Duration::from_millis(100)))
            .map_err(io)?;
        let (mut buf, end) = (Vec::new(), Instant::now() + timeout);
        let mut chunk = [0u8; 65536];
        while Instant::now() < end {
            match s.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => break,
            }
        }
        Ok(buf)
    }
}

impl SitlFc {
    /// The SITL "reboots" by exiting; record whether it did so on its own, then clean up.
    fn wait_for_exit(&mut self) -> AppResult<()> {
        self.rebooted_itself = false;
        if let Some(c) = self.child.as_mut() {
            let end = Instant::now() + Duration::from_secs(5);
            while Instant::now() < end {
                if c.try_wait().map_err(io)?.is_some() {
                    self.rebooted_itself = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        self.stop();
        Ok(())
    }
}

impl Drop for SitlFc {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Connection for SitlFc {
    fn msp_request(&mut self, code: u16, payload: &[u8]) -> AppResult<Vec<u8>> {
        let (code8, n) = (code as u8, payload.len() as u8);
        let mut frame = vec![b'$', b'M', b'<', n, code8];
        frame.extend_from_slice(payload);
        frame.push(payload.iter().fold(n ^ code8, |c, b| c ^ b));
        self.sock()?.write_all(&frame).map_err(io)?;
        let mut raw = Vec::new();
        let end = Instant::now() + Duration::from_secs(2);
        while Instant::now() < end {
            raw.extend(self.read(Duration::from_millis(200))?);
            if raw.len() >= 6 && raw.len() >= 6 + raw[3] as usize {
                break;
            }
        }
        if raw.len() < 6 || &raw[..3] != b"$M>" || raw[4] != code8 {
            return Err(AppError::Cli(format!("MSP {code}: bad reply {raw:02x?}")));
        }
        if code == session::MSP_SET_REBOOT {
            self.wait_for_exit()?;
        }
        Ok(raw[5..5 + raw[3] as usize].to_vec())
    }

    fn enter_cli(&mut self) -> AppResult<()> {
        if self.in_cli {
            return Ok(());
        }
        for _ in 0..40 {
            self.sock()?.write_all(b"#").map_err(io)?;
            let r = read_cli_response(self.sock()?, Duration::from_millis(500))?;
            if r.windows(12).any(|w| w == b"Entering CLI") {
                if !has_prompt_at_end(&r) {
                    read_cli_response(self.sock()?, Duration::from_millis(300))?;
                }
                self.in_cli = true;
                return Ok(());
            }
        }
        Err(AppError::Cli("SITL: no CLI prompt".into()))
    }

    fn cli_exec(&mut self, line: &str) -> AppResult<String> {
        self.sock()?
            .write_all(format!("{line}\n").as_bytes())
            .map_err(io)?;
        let r = read_cli_response(self.sock()?, Duration::from_secs(10))?;
        let mut s = String::from_utf8_lossy(&r).replace('\r', "");
        if let Some(t) = s.strip_suffix("\n# ") {
            s = t.to_string();
        }
        Ok(s)
    }

    fn exit_cli(&mut self, save: bool) -> AppResult<()> {
        if !save && self.noreboot_exit {
            self.sock()?.write_all(b"exit noreboot\n").map_err(io)?;
            // "# leaving CLI mode, no reboot"; the port is back to MSP afterwards.
            self.read(Duration::from_millis(700))?;
            self.in_cli = false;
            return Ok(());
        }
        let cmd: &[u8] = if save { b"save\n" } else { b"exit\n" };
        self.sock()?.write_all(cmd).map_err(io)?;
        self.read(Duration::from_secs(3))?; // "Rebooting", then the process exits
        self.in_cli = false;
        self.wait_for_exit()
    }
}

impl Reconnect for SitlFc {
    /// Restart the process and wait until it answers MSP. A fresh SITL occasionally
    /// accepts the TCP connection before its serial layer is up; retry the start then.
    fn reconnect(&mut self, _timeout: Duration) -> AppResult<()> {
        let mut last = AppError::Cli(format!("SITL did not open TCP {PORT}"));
        for _ in 0..3 {
            self.stop();
            ensure_port_free()?;
            let log = std::fs::File::create(self.dir.join("sitl.log")).map_err(io)?;
            self.child = Some(
                Command::new(&self.bin)
                    .current_dir(&self.dir)
                    .stdout(log)
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(io)?,
            );
            for _ in 0..100 {
                if let Ok(s) = TcpStream::connect(("127.0.0.1", PORT)) {
                    self.sock = Some(s);
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            if self.sock.is_none() {
                continue;
            }
            for _ in 0..10 {
                std::thread::sleep(Duration::from_millis(300));
                match self.msp_request(msp::MSP_API_VERSION, &[]) {
                    Ok(_) => {
                        let v = self.msp_request(msp::MSP_FC_VERSION, &[])?;
                        let v = msp::decode_fc_version(&v).unwrap_or_default();
                        self.noreboot_exit = Dialect::from_version(&v)
                            .map(|d| d != Dialect::V4_4)
                            .unwrap_or(true);
                        return Ok(());
                    }
                    Err(e) => last = e,
                }
            }
        }
        Err(last)
    }
}

/// Wraps the SITL link and fails it when a line starting with `at` is sent: `Timeout`
/// sends the line but reports no reply; `Drop` closes the socket before sending.
struct Faulty<'a> {
    fc: &'a mut SitlFc,
    at: &'static str,
    drop_link: bool,
    sent: Vec<String>,
}

impl Connection for Faulty<'_> {
    fn msp_request(&mut self, code: u16, payload: &[u8]) -> AppResult<Vec<u8>> {
        if code == session::MSP_SET_REBOOT {
            self.sent.push("MSP_SET_REBOOT".into());
        }
        self.fc.msp_request(code, payload)
    }
    fn enter_cli(&mut self) -> AppResult<()> {
        self.fc.enter_cli()
    }
    fn cli_exec(&mut self, line: &str) -> AppResult<String> {
        if line.starts_with(self.at) {
            if self.drop_link {
                self.fc.sock = None;
                return Err(AppError::Serial("device disconnected".into()));
            }
            self.fc
                .sock()?
                .write_all(format!("{line}\n").as_bytes())
                .map_err(io)?;
            return Err(AppError::Serial(format!("timeout waiting for `{line}`")));
        }
        self.sent.push(line.to_string());
        self.fc.cli_exec(line)
    }
    fn exit_cli(&mut self, save: bool) -> AppResult<()> {
        self.sent.push(if save { "save" } else { "exit" }.into());
        self.fc.exit_cli(save)
    }
}

impl Reconnect for Faulty<'_> {
    fn reconnect(&mut self, timeout: Duration) -> AppResult<()> {
        self.fc.reconnect(timeout)
    }
}

fn get(fc: &mut SitlFc, name: &str) -> String {
    fc.enter_cli().unwrap();
    let r = fc.cli_exec(&format!("get {name}")).unwrap();
    let v = r
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{name} = ")))
        .unwrap_or_else(|| panic!("get {name}: {r}"))
        .trim()
        .to_string();
    fc.exit_cli(false).unwrap();
    fc.reconnect(session::REBOOT_TIMEOUT).unwrap();
    v
}

#[test]
fn sitl_end_to_end_apply_error_and_rollback() {
    let Some(bin) = std::env::var_os("BFSW_SITL_BIN").map(PathBuf::from) else {
        eprintln!("BFSW_SITL_BIN not set; skipping the SITL end-to-end test");
        return;
    };
    let dir = std::env::temp_dir().join(format!("bfsw-sitl-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = BackupStore::new(dir.join("backups"));
    let mut fc = SitlFc::start(bin, dir.clone()).unwrap();

    // Identification over MSP (UART1 serves MSP until '#').
    let q = |fc: &mut SitlFc, c| fc.msp_request(c, &[]).unwrap();
    let (api, var, ver, board, uid) = (
        q(&mut fc, msp::MSP_API_VERSION),
        q(&mut fc, msp::MSP_FC_VARIANT),
        q(&mut fc, msp::MSP_FC_VERSION),
        q(&mut fc, msp::MSP_BOARD_INFO),
        q(&mut fc, msp::MSP_UID),
    );
    let info = msp::fc_info(&api, &var, &ver, &board, Some(&uid)).unwrap();
    assert_eq!(info.fc_variant, "BTFL");
    let uid = info.uid.clone().unwrap();

    fc.enter_cli().unwrap();
    let (vi, dialect) = session::read_version(&mut fc).unwrap();
    assert_eq!(vi.version, info.firmware_version);
    let original = {
        let r = fc.cli_exec("get serialrx_provider").unwrap();
        r.lines()
            .find_map(|l| l.strip_prefix("serialrx_provider = "))
            .unwrap()
            .trim()
            .to_string()
    };
    let target = if original == "SBUS" { "CRSF" } else { "SBUS" };
    let script = |cmds: &[&str]| CliScript {
        commands: cmds.iter().map(|c| c.to_string()).collect(),
        warnings: Vec::new(),
        dialect: dialect.as_str().into(),
        cleared: Vec::new(),
    };

    // 1. Successful apply: saved, rebooted (process restarted), value persisted.
    let ok = script(&[
        &format!("set serialrx_provider = {target}"),
        "set craft_name = BFSW",
    ]);
    let r = session::apply(&mut fc, &ok, &store, &uid, &mut |_| {}).unwrap();
    assert!(r.saved && r.errors.is_empty(), "{r:?}");
    let first_backup = r.backup.clone().unwrap();
    assert_eq!(get(&mut fc, "serialrx_provider"), target);
    assert_eq!(get(&mut fc, "craft_name"), "BFSW");

    // 2. A rejected command aborts the batch, `exit` discards it, nothing is saved.
    fc.enter_cli().unwrap();
    let bad = script(&[
        &format!("set serialrx_provider = {original}"),
        "set foo = 1",
    ]);
    let r = session::apply(&mut fc, &bad, &store, &uid, &mut |_| {}).unwrap();
    assert!(!r.saved);
    assert!(r.errors[0].response.contains("INVALID NAME"), "{r:?}");
    assert_eq!(
        get(&mut fc, "serialrx_provider"),
        target,
        "failed batch must not persist"
    );

    // 3. Rollback to the backup taken before step 1 restores the original value.
    fc.enter_cli().unwrap();
    let r = session::rollback(&mut fc, &first_backup, &store, &uid, &mut |_| {}).unwrap();
    assert!(r.saved && r.errors.is_empty(), "{r:?}");
    assert_eq!(get(&mut fc, "serialrx_provider"), original);
    assert_eq!(get(&mut fc, "craft_name"), "-");
    // The real backup went through the guard: identity lines dropped, defaults first.
    let replayed =
        super::guard::replay_lines(&std::fs::read_to_string(&first_backup.path).unwrap()).unwrap();
    assert_eq!(replayed[0], "defaults nosave");
    assert!(replayed
        .iter()
        .all(|l| !l.starts_with("mcu_id") && !l.starts_with("signature") && l != "save"));

    // 4. Guard: a script with `save` is refused before anything reaches the FC.
    fc.enter_cli().unwrap();
    let mut f = Faulty {
        fc: &mut fc,
        at: "\u{0}never",
        drop_link: false,
        sent: Vec::new(),
    };
    let e = session::apply(
        &mut f,
        &script(&["set craft_name = X", "save"]),
        &store,
        &uid,
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(e.code(), "CONFIG");
    assert!(f.sent.is_empty(), "{:?}", f.sent);
    f.fc.exit_cli(false).unwrap();
    f.fc.reconnect(session::REBOOT_TIMEOUT).unwrap();

    // 5. Link timeout mid-batch: best-effort `exit` reaches the FC (it reboots on its
    //    own), `save` is never sent, the half-applied batch is gone after reboot.
    fc.enter_cli().unwrap();
    let mut f = Faulty {
        fc: &mut fc,
        at: "set craft_name",
        drop_link: false,
        sent: Vec::new(),
    };
    let e = session::apply(
        &mut f,
        &script(&[
            &format!("set serialrx_provider = {target}"),
            "set craft_name = HALF",
        ]),
        &store,
        &uid,
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(e.code(), "SERIAL", "original error returned: {e}");
    let discard: &[&str] = if dialect == Dialect::V4_4 {
        &["exit"]
    } else {
        &["exit", "MSP_SET_REBOOT"]
    };
    assert!(
        f.sent
            .ends_with(&discard.iter().map(|x| x.to_string()).collect::<Vec<_>>()),
        "{:?}",
        f.sent
    );
    assert!(!f.sent.iter().any(|l| l == "save"));
    assert!(
        f.fc.rebooted_itself,
        "SITL rebooted (exit / MSP_SET_REBOOT)"
    );
    f.fc.reconnect(session::REBOOT_TIMEOUT).unwrap();
    assert_eq!(get(&mut fc, "serialrx_provider"), original);
    assert_eq!(get(&mut fc, "craft_name"), "-");

    // 6. Link dropped mid-batch: the `exit` attempt fails (ignored), the original error
    //    is returned and nothing was saved.
    fc.enter_cli().unwrap();
    let mut f = Faulty {
        fc: &mut fc,
        at: "set craft_name",
        drop_link: true,
        sent: Vec::new(),
    };
    let e = session::apply(
        &mut f,
        &script(&[
            &format!("set serialrx_provider = {target}"),
            "set craft_name = HALF",
        ]),
        &store,
        &uid,
        &mut |_| {},
    )
    .unwrap_err();
    assert!(e.to_string().contains("disconnected"), "{e}");
    assert!(
        f.sent
            .ends_with(&discard.iter().map(|x| x.to_string()).collect::<Vec<_>>()),
        "{:?}",
        f.sent
    );
    assert!(!f.sent.iter().any(|l| l == "save"));
    // Observation (not a requirement): does the FC stay in CLI after the link drops?
    // Reconnect to the same process without restarting it and probe MSP.
    f.fc.sock = TcpStream::connect(("127.0.0.1", PORT)).ok();
    f.fc.in_cli = false;
    let msp_alive = f.fc.msp_request(msp::MSP_API_VERSION, &[]).is_ok();
    eprintln!("after link drop mid-batch the SITL answers MSP on reconnect: {msp_alive}");
    f.fc.reconnect(session::REBOOT_TIMEOUT).unwrap(); // restart: unsaved changes are lost
    assert_eq!(get(&mut fc, "serialrx_provider"), original);

    // 7. Why config reboots over MSP on 4.5+: `exit noreboot` keeps unsaved changes live.
    if dialect != Dialect::V4_4 {
        fc.enter_cli().unwrap();
        fc.cli_exec("set craft_name = LIVE").unwrap();
        fc.exit_cli(false).unwrap(); // exit noreboot
        fc.enter_cli().unwrap();
        let r = fc.cli_exec("get craft_name").unwrap();
        assert!(r.contains("craft_name = LIVE"), "{r}");
        fc.exit_cli(false).unwrap();
        fc.msp_request(session::MSP_SET_REBOOT, &[0]).unwrap();
        assert!(fc.rebooted_itself);
        fc.reconnect(session::REBOOT_TIMEOUT).unwrap();
        assert_eq!(get(&mut fc, "craft_name"), "-");
    }

    // 8. The model-ext lines (shared/config-model-ext) are accepted by the real CLI and
    //    read back as generated. GPS and DShot are not compiled into SITL, so only the
    //    VTX table, vtx_*, craft_name, motor_output_reordering and (2025.12) motor_idle.
    let m: crate::model::DroneModel = serde_json::from_str(
        &std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/models/longrange-7in-analog.json"),
        )
        .unwrap(),
    )
    .unwrap();
    // SITL quirk (4.4.3/4.5.5/2025.12.5 x86 builds): an 8-character band name reads back
    // with its last character replaced by a space ("BOSCAM_A" -> "BOSCAM_"), although
    // VTX_TABLE_BAND_NAME_LENGTH is 8 and 8-character names (BOSCAM_A, RACEBAND,
    // FATSHARK) are standard in real FC dumps. Use 7-character names here so the test
    // checks the protocol rather than the SITL quirk; the goldens keep the real names.
    let mut table = m.vtx.table.clone().unwrap();
    for b in &mut table.bands {
        b.name.truncate(7);
    }
    let table = &table;
    let mut lines = VtxTableState::commands_for(table);
    lines.extend([
        "set vtx_band = 2".to_string(),
        "set vtx_channel = 1".into(),
        "set vtx_power = 2".into(),
        "set craft_name = LR7 Chimera".into(),
        "set motor_output_reordering = 1,0,3,2,4,5,6,7".into(),
    ]);
    if dialect.idle_setting() == "motor_idle" {
        lines.push("set motor_idle = 450".into());
    }
    fc.enter_cli().unwrap();
    for l in &lines {
        let r = fc.cli_exec(l).unwrap();
        assert!(!r.contains("###ERROR"), "{l}: {r}");
    }
    let dump = fc.cli_exec("dump all").unwrap();
    let snap = super::snapshot::parse_dump(&dump);
    assert_eq!(
        snap.vtxtable.as_ref().map(VtxTableState::render),
        Some(VtxTableState::from_model(table).render())
    );
    assert_eq!(snap.setting("craft_name"), Some("LR7 Chimera"));
    assert_eq!(snap.setting("vtx_power"), Some("2"));
    assert_eq!(
        snap.setting("motor_output_reordering"),
        Some("1,0,3,2,4,5,6,7")
    );
    if dialect.idle_setting() == "motor_idle" {
        assert_eq!(snap.setting("motor_idle"), Some("450"));
    }
    let r = fc.cli_exec("set craft_name = -").unwrap();
    assert!(!r.contains("###ERROR"), "{r}");
    // discard step 8 (unsaved) before step 9
    fc.exit_cli(false).unwrap();
    if dialect != Dialect::V4_4 {
        fc.msp_request(session::MSP_SET_REBOOT, &[0]).unwrap();
    }
    fc.reconnect(session::REBOOT_TIMEOUT).unwrap();

    // 9. Batch 2 through plan -> apply -> verify on the real CLI: airmode, optional modes
    //    and removeModes, sensors, board alignment. SITL has no beeper / beacon commands
    //    (warnings) and no OSD (CONFIG error); serial / DShot / GPS are not part of this
    //    step (SITL lacks them), so only the batch-2 lines of the plan are applied.
    let mut m: crate::model::DroneModel = serde_json::from_str(
        &std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/models/batch2-freestyle-hd.json"),
        )
        .unwrap(),
    )
    .unwrap();
    m.receiver.kind = crate::model::ReceiverKind::None;
    m.vtx = crate::model::Vtx {
        kind: crate::model::VtxKind::None,
        protocol: None,
        port: None,
        digital_system: None,
        region: None,
        table: None,
        band: None,
        channel: None,
        power: None,
    };
    m.esc.telemetry_port = None;
    fc.enter_cli().unwrap();
    // an old PREARM range (removeModes) and an unmanaged MAG range (kept)
    for l in ["aux 10 36 4 1700 2100 0 0", "aux 11 5 5 1700 2100 0 0"] {
        let r = fc.cli_exec(l).unwrap();
        assert!(!r.contains("###ERROR"), "{l}: {r}");
    }
    let osd_err = session::plan(&mut fc, &m).unwrap_err();
    assert_eq!(osd_err.code(), "CONFIG");
    assert!(osd_err.to_string().contains("no OSD"), "{osd_err}");
    m.osd = None;
    let plan = session::plan(&mut fc, &m).unwrap();
    assert!(
        plan.warnings
            .iter()
            .any(|w| w.contains("no beeper command"))
            && plan.warnings.iter().any(|w| w.contains("no DShot beacon")),
        "{:?}",
        plan.warnings
    );
    assert!(plan.warnings.iter().any(|w| w.contains("kept 1 aux")));
    let batch2 = |c: &str| {
        c.ends_with("AIRMODE")
            || c.starts_with("aux ")
            || c.starts_with("set acc_hardware")
            || c.starts_with("set baro_hardware")
            || c.starts_with("set mag_hardware")
            || c.starts_with("set align_board_")
            || c.starts_with("beeper")
            || c.starts_with("beacon")
    };
    let cmds: Vec<&str> = plan
        .commands
        .iter()
        .map(String::as_str)
        .filter(|c| batch2(c))
        .collect();
    assert!(cmds.contains(&"feature -AIRMODE"), "{cmds:?}");
    assert!(cmds.contains(&"aux 10 0 0 900 900 0 0"), "{cmds:?}");
    assert!(cmds.contains(&"set align_board_yaw = 90"), "{cmds:?}");
    let r = session::apply(&mut fc, &script(&cmds), &store, &uid, &mut |_| {}).unwrap();
    assert!(r.saved && r.errors.is_empty(), "{r:?}");
    fc.enter_cli().unwrap();
    let v = session::verify(&mut fc, &m).unwrap();
    let b2: Vec<_> = v
        .mismatches
        .iter()
        .filter(|x| {
            let k = x.setting.as_str();
            k.starts_with("aux")
                || k.contains("AIRMODE")
                || k.contains("_hardware")
                || k.contains("align_board")
        })
        .collect();
    assert!(b2.is_empty(), "{b2:?}");
    let dump = fc.cli_exec("dump all").unwrap();
    let snap = super::snapshot::parse_dump(&dump);
    assert_eq!(
        snap.features.get("AIRMODE"),
        Some(&false),
        "dump ({} lines), feature lines: {:?}",
        dump.lines().count(),
        dump.lines()
            .filter(|l| l.starts_with("feature"))
            .collect::<Vec<_>>()
    );
    assert_eq!(snap.setting("align_board_yaw"), Some("90"));
    assert_eq!(snap.setting("baro_hardware"), Some("NONE"));
    let used: Vec<(u8, u8)> = snap
        .aux
        .iter()
        .filter(|a| a.is_used())
        .map(|a| (a.slot, a.mode_id))
        .collect();
    assert!(
        used.contains(&(1, 28)) && used.contains(&(11, 5)),
        "{used:?}"
    );
    assert!(!used.iter().any(|u| u.1 == 36), "{used:?}");
    assert!(snap.beepers.is_none() && snap.beacons.is_none());
    fc.exit_cli(false).unwrap();
    if dialect != Dialect::V4_4 {
        fc.msp_request(session::MSP_SET_REBOOT, &[0]).unwrap();
    }

    drop(fc);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The CLI reader does not stop at a `# ` comment line that happens to end a TCP read
/// (`dump all`'s `# feature`), only at a prompt followed by the quiet period.
#[test]
fn sitl_cli_reader_waits_out_comment_lines() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut c, _) = listener.accept().unwrap();
        c.write_all(b"dump all\r\n\r\n# version\r\n# ").unwrap();
        std::thread::sleep(Duration::from_millis(40));
        c.write_all(b"Betaflight\r\n\r\n# feature\r\n# ").unwrap();
        std::thread::sleep(Duration::from_millis(60));
        c.write_all(b"\r\nfeature -AIRMODE\r\n\r\n# ").unwrap();
        // keep the connection open past the quiet period
        std::thread::sleep(Duration::from_millis(400));
    });
    let mut s = TcpStream::connect(addr).unwrap();
    let t0 = Instant::now();
    let r = read_cli_response(&mut s, Duration::from_secs(2)).unwrap();
    let text = String::from_utf8_lossy(&r);
    assert!(text.ends_with("feature -AIRMODE\r\n\r\n# "), "{text:?}");
    assert!(
        t0.elapsed() < Duration::from_millis(400),
        "{:?}",
        t0.elapsed()
    );
    server.join().unwrap();
}

/// `get name` inside PID profile `profile` (no save; the SITL restarts afterwards).
fn get_in_profile(fc: &mut SitlFc, profile: u8, name: &str) -> String {
    fc.enter_cli().unwrap();
    fc.cli_exec(&format!("profile {profile}")).unwrap();
    let r = fc.cli_exec(&format!("get {name}")).unwrap();
    let v = r
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{name} = ")))
        .unwrap_or_else(|| panic!("get {name}: {r}"))
        .trim()
        .to_string();
    fc.exit_cli(false).unwrap();
    fc.reconnect(session::REBOOT_TIMEOUT).unwrap();
    v
}

/// config_restore_after_flash across a simulated flash: configure the
/// `BFSW_SITL_FROM_BIN` SITL, take the `diff all` backup like `config_backup`, "flash" by
/// starting `BFSW_SITL_BIN` in a fresh directory (empty eeprom = full erase, board at
/// defaults), restore, and check the values on the new firmware. Both may be the same
/// line. Skipped unless both are set.
#[test]
fn sitl_restore_after_flash() {
    use super::restore;
    use crate::error::ConfigErrorReason;
    let (Some(from_bin), Some(to_bin)) = (
        std::env::var_os("BFSW_SITL_FROM_BIN").map(PathBuf::from),
        std::env::var_os("BFSW_SITL_BIN").map(PathBuf::from),
    ) else {
        eprintln!("BFSW_SITL_FROM_BIN / BFSW_SITL_BIN not set; skipping the SITL restore test");
        return;
    };
    let root = std::env::temp_dir().join(format!("bfsw-sitl-restore-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for d in ["from", "to"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    let store = BackupStore::new(root.join("backups"));

    // 1. Old firmware: configure a few settings that are renamed between lines.
    let mut fc = SitlFc::start(from_bin, root.join("from")).unwrap();
    let uid = {
        let q = |fc: &mut SitlFc, c| fc.msp_request(c, &[]).unwrap();
        let (api, var, ver, board, uid) = (
            q(&mut fc, msp::MSP_API_VERSION),
            q(&mut fc, msp::MSP_FC_VARIANT),
            q(&mut fc, msp::MSP_FC_VERSION),
            q(&mut fc, msp::MSP_BOARD_INFO),
            q(&mut fc, msp::MSP_UID),
        );
        let info = msp::fc_info(&api, &var, &ver, &board, Some(&uid)).unwrap();
        info.uid.unwrap_or_else(|| "unknown".into())
    };
    let (vi, from) = session::read_version(&mut fc).unwrap();
    let idle = |d: Dialect| {
        if d >= Dialect::V2025_12 {
            "motor_idle"
        } else {
            "dshot_idle_value"
        }
    };
    let angle = |d: Dialect| {
        if d == Dialect::V4_4 {
            "angle_level_strength"
        } else {
            "angle_p_gain"
        }
    };
    let dmax = |d: Dialect| {
        if d >= Dialect::V2025_12 {
            "D_MAX"
        } else {
            "D_MIN"
        }
    };
    let port = if from.uses_port_names() { "UART3" } else { "2" };
    // The idle setting is DShot's on 4.x, which the 4.x SITL does not compile in.
    let has = |fc: &mut SitlFc, name: &str| {
        !fc.cli_exec(&format!("get {name}"))
            .unwrap()
            .contains("###ERROR")
    };
    let from_idle = has(&mut fc, idle(from));
    let mut setup = vec![
        "set craft_name = RESTORE".into(),
        format!("set debug_mode = {}", dmax(from)),
        format!("serial {port} 64 115200 57600 0 115200"),
        "aux 0 0 0 1700 2100 0 0".into(),
        "profile 1".into(),
        "set p_roll = 55".into(),
        format!("set {} = 60", angle(from)),
        "profile 0".into(),
    ];
    if from_idle {
        setup.push(format!("set {} = 450", idle(from)));
    }
    for l in &setup {
        let r = fc.cli_exec(l).unwrap();
        assert!(!r.contains("###ERROR"), "{l}: {r}");
    }
    fc.exit_cli(true).unwrap();
    fc.reconnect(session::REBOOT_TIMEOUT).unwrap();
    let backup = session::backup(&mut fc, &store, &uid).unwrap();
    fc.exit_cli(false).unwrap();
    fc.stop();
    drop(fc);

    // 2. "Flash": the new firmware starts with an empty eeprom (defaults).
    let mut fc = SitlFc::start(to_bin, root.join("to")).unwrap();
    let (to_vi, to) = session::read_version(&mut fc).unwrap();
    let to_idle = has(&mut fc, idle(to));
    fc.exit_cli(false).unwrap();
    fc.reconnect(session::REBOOT_TIMEOUT).unwrap();
    assert_ne!(
        get(&mut fc, "craft_name"),
        "RESTORE",
        "target must start at defaults"
    );

    let prepared = restore::prepare(&backup, std::fs::read_to_string(&backup.path)).unwrap();

    // A backup of another board is refused before anything is sent.
    let e = restore::run(&mut fc, &prepared, "0123456789ABCDEF01234567", &mut |_| {}).unwrap_err();
    match &e {
        AppError::ConfigRejected { reason, .. } => {
            assert_eq!(*reason, ConfigErrorReason::BoardUidMismatch, "{e:?}")
        }
        _ if uid == "unknown" => {}
        _ => panic!("{e:?}"),
    }
    fc.exit_cli(false).unwrap();
    fc.reconnect(session::REBOOT_TIMEOUT).unwrap();

    let mut stages = Vec::new();
    let r = restore::run(&mut fc, &prepared, &uid, &mut |p| stages.push(p.stage)).unwrap();
    eprintln!(
        "SITL restore {} -> {}: applied {}, failed {:?}, skipped {}, warnings {:#?}",
        vi.version,
        to_vi.version,
        r.applied,
        r.failed,
        r.skipped.len(),
        r.warnings
    );
    for s in &r.skipped {
        eprintln!("  skip {:?} {}", s.reason, s.line);
    }
    assert!(r.saved);
    assert_eq!(r.from_version, vi.version);
    assert_eq!(r.to_version, to_vi.version);
    assert_eq!(r.line_changed, from != to);
    assert!(
        r.failed.is_empty(),
        "SITL rejected restored lines: {:?}",
        r.failed
    );
    assert!(fc.rebooted_itself, "save rebooted the SITL");
    assert!(stages.contains(&crate::model::ConfigStage::Reconnected));

    // 3. The values survived the reboot under the new firmware's names.
    fc.exit_cli(false).unwrap();
    fc.reconnect(session::REBOOT_TIMEOUT).unwrap();
    let idle_line = format!("set {} = 450", idle(from));
    match (from_idle, to_idle) {
        (true, true) => assert_eq!(get(&mut fc, idle(to)), "450"),
        (true, false) => {
            assert!(
                r.skipped.iter().any(|s| s.line == idle_line
                    && s.reason == crate::model::RestoreSkipReason::NotInBuild),
                "{:?}",
                r.skipped
            )
        }
        _ => {}
    }
    assert_eq!(get(&mut fc, "craft_name"), "RESTORE");
    assert_eq!(get(&mut fc, "debug_mode"), dmax(to));
    assert_eq!(get_in_profile(&mut fc, 1, "p_roll"), "55");
    assert_eq!(get_in_profile(&mut fc, 1, angle(to)), "60");
    assert_eq!(
        get_in_profile(&mut fc, 0, "p_roll"),
        get_in_profile(&mut fc, 2, "p_roll")
    );
    fc.enter_cli().unwrap();
    let serial = fc.cli_exec("serial").unwrap();
    let want = if to.uses_port_names() {
        "serial UART3 64 "
    } else {
        "serial 2 64 "
    };
    assert!(serial.contains(want), "{serial}");
    let aux = fc.cli_exec("aux").unwrap();
    assert!(aux.contains("aux 0 0 0 1700 2100 0 0"), "{aux}");
    fc.exit_cli(false).unwrap();
    fc.stop();
    drop(fc);
    assert!(
        !r.warnings.iter().any(|w| w.starts_with("Read back")),
        "{:?}",
        r.warnings
    );
    let _ = std::fs::remove_dir_all(&root);
}
