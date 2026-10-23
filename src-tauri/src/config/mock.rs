//! C1: `MockFc`, a stateful Betaflight CLI simulator implementing `device::Connection`,
//! seeded from dump-format fixtures in `src-tauri/tests/fixtures/<firmware>/`.
//!
//! The fixtures are composed from real public sources (Betaflight SITL `dump all` plus
//! the board's unified-target / config.h; see `tests/fixtures/README.md`). Responses
//! follow what Betaflight 4.4.3 / 4.5.5 / 2025.12.5 SITL actually print
//! (`tests/fixtures/sitl/<version>/transcript.txt`): every line is echoed, then
//! `serialrx_provider set to CRSF`, `Enabled RX_MSP`, `###ERROR IN set: INVALID NAME: foo = 1###`,
//! `###ERROR IN input: UNKNOWN COMMAND, TRY 'HELP'###`, and so on.
//!
//! Modelled: `version`, `serial`, `feature`, `set`/`get`, `aux`, `resource`,
//! `batch start|end`, `defaults nosave`, `dump all`, `diff all`, save/exit semantics
//! (unsaved changes are lost on `exit`; `save` persists even after a batch error, as on
//! real firmware, so the engine must never send it then), compiled-out features
//! (`unavailable`), error injection, and the MSP identification messages
//! (API_VERSION, FC_VARIANT, FC_VERSION, BOARD_INFO, BUILD_INFO, UID).

use std::collections::BTreeMap;

use super::firmware::Dialect;
use super::generate::serial_cmd;
use super::msp;
use super::snapshot::{parse_dump, parse_serial_line, AuxSlot, Snapshot};
use super::vtx_table::VtxTableState;
use crate::device::{Connection, Reconnect};
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
struct State {
    ports: Vec<super::snapshot::PortState>,
    features: BTreeMap<String, bool>,
    settings: BTreeMap<String, String>,
    aux: BTreeMap<u8, AuxSlot>,
    resources: BTreeMap<String, String>,
    vtxtable: VtxTableState,
}

impl State {
    fn from_snapshot(s: &Snapshot) -> State {
        let mut aux: BTreeMap<u8, AuxSlot> = (0..AuxSlot::COUNT)
            .map(|i| (i, AuxSlot::empty(i)))
            .collect();
        for a in &s.aux {
            aux.insert(a.slot, *a);
        }
        State {
            ports: s.ports.clone(),
            features: s.features.clone(),
            settings: s.settings.clone(),
            aux,
            resources: s.resources.clone(),
            vtxtable: s.vtxtable.clone().unwrap_or_default(),
        }
    }
}

/// Link-level failure injected by [`MockFc::link_fault`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkFault {
    /// The line gets no reply (read timeout); the link still works afterwards.
    Timeout,
    /// The cable is pulled while sending the line: every later call fails and the FC
    /// power-cycles (unsaved changes are lost).
    Unplug,
}

pub struct MockFc {
    pub version: String,
    pub board_name: String,
    /// `# Betaflight / STM32F405 (S405) 4.5.5 ...` line from the fixture.
    banner_line: String,
    /// "STM32F405", "S405" from the banner.
    pub mcu: String,
    pub board_identifier: String,
    pub manufacturer_id: String,
    /// CLI `mcu_id` (24 hex digits); MSP_UID returns it as three LE u32 words.
    pub mcu_id: String,
    /// Features compiled out of this build: `feature X` answers "unavailable".
    pub unavailable_features: Vec<String>,
    /// Number of `save` commands accepted while the batch had errors (real firmware saves).
    pub saves_after_error: usize,
    dialect: Dialect,
    defaults: State,
    saved: State,
    live: State,
    in_cli: bool,
    batch_error: bool,
    /// Commands (exact or prefix) the FC will reject.
    pub fail_on: Vec<String>,
    /// Every line received, in order.
    pub log: Vec<String>,
    pub reboots: usize,
    /// Fail the link when a line starting with this prefix is sent.
    pub link_fault: Option<(String, LinkFault)>,
    unplugged: bool,
}

pub fn fixture(firmware: &str, board: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/{firmware}/{board}.dump.txt",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn err(cmd: &str, reason: &str) -> String {
    format!("###ERROR IN {cmd}: {reason}###\r\n")
}

fn unknown_command() -> String {
    err("input", "UNKNOWN COMMAND, TRY 'HELP'")
}

/// Lookup-table values MockFc enforces (`###ERROR IN set: INVALID VALUE###`). Only
/// tables captured from SITL (`serialrx_provider`) or identical in the settings.c of all
/// three releases (`gps_sbas_mode`) are listed, in firmware order; booleans are ON/OFF;
/// other settings accept any value.
fn allowed_values(
    dialect: Dialect,
    name: &str,
    current: Option<&String>,
) -> Option<Vec<&'static str>> {
    const RX: [&str; 14] = [
        "SPEK2048",
        "SBUS",
        "SUMD",
        "SUMH",
        "XB-B",
        "XB-B-RJ01",
        "IBUS",
        "JETIEXBUS",
        "CRSF",
        "SRXL",
        "CUSTOM",
        "FPORT",
        "SRXL2",
        "GHST",
    ];
    match (name, current.map(String::as_str)) {
        ("serialrx_provider", _) => Some(match dialect {
            Dialect::V4_4 => [&["SPEK1024"][..], &RX].concat(),
            Dialect::V4_5 => [&["NONE"][..], &RX, &["SPEK1024"]].concat(),
            Dialect::V2025_12 => [&["NONE"][..], &RX, &["SPEK1024", "MAVLINK"]].concat(),
        }),
        ("gps_sbas_mode", _) => Some(vec!["AUTO", "EGNOS", "WAAS", "MSAS", "GAGAN", "NONE"]),
        (_, Some("ON" | "OFF")) => Some(vec!["OFF", "ON"]),
        _ => None,
    }
}

/// Value of a `name value` line in a dump, e.g. `mcu_id`.
fn dump_value<'a>(dump: &'a str, key: &str) -> Option<&'a str> {
    dump.lines()
        .filter_map(|l| l.trim().strip_prefix(key))
        .find(|r| r.starts_with(' '))
        .map(str::trim)
}

impl MockFc {
    pub fn from_dump(dump: &str) -> MockFc {
        let snap = parse_dump(dump);
        let version = snap
            .version
            .clone()
            .expect("fixture needs a version banner");
        let dialect = Dialect::from_version(&version).unwrap_or(Dialect::V2025_12);
        let st = State::from_snapshot(&snap);
        let banner_line = dump
            .lines()
            .find(|l| l.starts_with("# Betaflight /"))
            .unwrap_or("# Betaflight / STM32F405 (S405)")
            .trim()
            .to_string();
        // "# Betaflight / STM32F405 (S405) 4.5.5 ..."
        let mut w = banner_line.split_whitespace().skip(3);
        let mcu = w.next().unwrap_or("STM32F405").to_string();
        let board_identifier = w
            .next()
            .unwrap_or("(S405)")
            .trim_matches(|c| c == '(' || c == ')')
            .to_string();
        let manufacturer_id = dump_value(dump, "manufacturer_id")
            .unwrap_or_default()
            .to_string();
        MockFc {
            board_name: snap.board_name.clone().unwrap_or_default(),
            banner_line,
            mcu,
            board_identifier,
            manufacturer_id,
            mcu_id: dump_value(dump, "mcu_id")
                .unwrap_or("000000000000000100000002")
                .to_string(),
            unavailable_features: Vec::new(),
            saves_after_error: 0,
            version,
            dialect,
            defaults: st.clone(),
            saved: st.clone(),
            live: st,
            in_cli: false,
            batch_error: false,
            fail_on: Vec::new(),
            log: Vec::new(),
            reboots: 0,
            link_fault: None,
            unplugged: false,
        }
    }

    pub fn load(firmware: &str, board: &str) -> MockFc {
        MockFc::from_dump(&fixture(firmware, board))
    }

    /// Change the saved (pre-existing) state without logging, e.g. a user's old config.
    pub fn preset(&mut self, lines: &[&str]) {
        for l in lines {
            let r = self.exec(l);
            assert!(!r.contains("###ERROR"), "preset {l}: {r}");
        }
        self.saved = self.live.clone();
    }

    pub fn snapshot(&self) -> Snapshot {
        parse_dump(&self.render(false))
    }

    fn banner(&self) -> String {
        let mut b = format!("{}\r\n", self.banner_line);
        if !self.board_name.is_empty() {
            b += &format!("# board_name: {}\r\n", self.board_name);
            if !self.manufacturer_id.is_empty() {
                b += &format!("# manufacturer_id: {}\r\n", self.manufacturer_id);
            }
        }
        b
    }

    fn api_version(&self) -> (u8, u8) {
        self.banner_line
            .split("MSP API: ")
            .nth(1)
            .and_then(|v| {
                let (a, b) = v.trim().split_once('.')?;
                Some((a.parse().ok()?, b.parse().ok()?))
            })
            .unwrap_or((1, 46))
    }

    /// Reply payload for an MSP request, laid out like Betaflight's `msp.c`.
    pub fn msp_reply(&self, code: u16) -> Option<Vec<u8>> {
        let pstr = |o: &mut Vec<u8>, s: &str| {
            o.push(s.len() as u8);
            o.extend_from_slice(s.as_bytes());
        };
        Some(match code {
            msp::MSP_API_VERSION => {
                let (a, b) = self.api_version();
                vec![0, a, b]
            }
            msp::MSP_FC_VARIANT => b"BTFL".to_vec(),
            msp::MSP_FC_VERSION => {
                let n: Vec<u16> = self
                    .version
                    .split('.')
                    .filter_map(|x| x.parse().ok())
                    .collect();
                let (a, b, c) = (
                    n[0],
                    n.get(1).copied().unwrap_or(0),
                    n.get(2).copied().unwrap_or(0),
                );
                if a >= 2025 {
                    let mut o = vec![(a % 100) as u8, b as u8, c as u8];
                    pstr(&mut o, &self.version);
                    o
                } else {
                    vec![a as u8, b as u8, c as u8]
                }
            }
            msp::MSP_BOARD_INFO => {
                let mut o = self.board_identifier.as_bytes().to_vec();
                o.resize(4, b' ');
                o.extend_from_slice(&0u16.to_le_bytes()); // hardware revision
                o.push(2); // onboard max7456
                o.push(0); // target capabilities
                pstr(&mut o, &self.mcu);
                pstr(&mut o, &self.board_name);
                pstr(&mut o, &self.manufacturer_id);
                o.extend_from_slice(&[0; 32]); // signature
                o.push(msp::mcu_type_id(&self.mcu));
                o.push(2); // configuration state: CONFIGURED
                o.extend_from_slice(&8000u16.to_le_bytes()); // gyro sample rate
                o.extend_from_slice(&0u32.to_le_bytes()); // configuration problems
                o.extend_from_slice(&[0, 0]); // SPI / I2C device counts
                o
            }
            msp::MSP_BUILD_INFO => {
                // "Mmm dd yyyy" + "hh:mm:ss" + 7-char revision, as printed in the banner.
                let mut o = b"Jan  1 202600:00:00".to_vec();
                o.extend_from_slice(b"mockrev");
                o
            }
            msp::MSP_UID => msp::encode_uid(&self.mcu_id)?,
            _ => return None,
        })
    }

    fn render(&self, diff: bool) -> String {
        let d = &self.defaults;
        let s = &self.live;
        let mut o = self.banner();
        o += "\r\n# start the command batch\r\nbatch start\r\n";
        if diff {
            o += "\r\n# reset configuration to default settings\r\ndefaults nosave\r\n";
        }
        o += &format!("\r\nboard_name {}\r\n", self.board_name);
        for (k, v) in &s.resources {
            if !diff || d.resources.get(k) != Some(v) {
                o += &format!("resource {k} {v}\r\n");
            }
        }
        for (f, on) in &s.features {
            if !diff || d.features.get(f) != Some(on) {
                o += &format!("feature {}{f}\r\n", if *on { "" } else { "-" });
            }
        }
        for p in &s.ports {
            let def = d.ports.iter().find(|q| q.name == p.name);
            if !diff || def != Some(p) {
                o += &serial_cmd(p, p.function_mask, self.dialect);
                o += "\r\n";
            }
        }
        for a in s.aux.values() {
            if !diff || d.aux.get(&a.slot) != Some(a) {
                o += &a.to_cli();
                o += "\r\n";
            }
        }
        if !diff || d.vtxtable != s.vtxtable {
            o += "\r\n# vtxtable\r\n";
            for l in s.vtxtable.render() {
                o += &l;
                o += "\r\n";
            }
        }
        o += "\r\n# master\r\n";
        for (k, v) in &s.settings {
            if !diff || d.settings.get(k) != Some(v) {
                o += &format!("set {k} = {v}\r\n");
            }
        }
        o += "\r\n# end the command batch\r\nbatch end\r\n";
        o
    }

    fn exec(&mut self, line: &str) -> String {
        let line = line.trim();
        if self
            .fail_on
            .iter()
            .any(|f| line == f || line.starts_with(f.as_str()))
        {
            self.batch_error = true;
            return format!(
                "{line}\r\n{}",
                err(line.split(' ').next().unwrap_or(line), "INJECTED FAILURE")
            );
        }
        let r = self.exec_inner(line);
        if r.contains("###ERROR") {
            self.batch_error = true;
        }
        // The CLI echoes every line before answering.
        format!("{line}\r\n{r}")
    }

    fn exec_inner(&mut self, line: &str) -> String {
        let (cmd, args) = line.split_once(' ').unwrap_or((line, ""));
        let args = args.trim();
        match cmd {
            "version" => self.banner(),
            "batch" => match args {
                "start" => {
                    self.batch_error = false;
                    "Command batch started\r\n".into()
                }
                "end" if self.batch_error => err(
                    "batch",
                    "ERRORS WERE DETECTED - PLEASE REVIEW BEFORE CONTINUING",
                ),
                "end" => "Command batch ended\r\n".into(),
                _ => unknown_command(),
            },
            "defaults" if args == "nosave" => {
                self.live = self.defaults.clone();
                String::new()
            }
            "board_name" => String::new(),
            "dump" => self.render(false),
            "diff" => self.render(true),
            "serial" if args.is_empty() => self
                .live
                .ports
                .iter()
                .map(|p| serial_cmd(p, p.function_mask, self.dialect) + "\r\n")
                .collect(),
            "serial" => match parse_serial_line(line, self.dialect) {
                Some(p) if self.live.ports.iter().any(|q| q.name == p.name) => {
                    let echo = serial_cmd(&p, p.function_mask, self.dialect) + "\r\n";
                    if let Some(q) = self.live.ports.iter_mut().find(|q| q.name == p.name) {
                        *q = p;
                    }
                    echo
                }
                // Real firmware: 2025.12 cannot parse an unknown port name; 4.x rejects an
                // unknown numeric identifier as an argument-count error.
                _ if self.dialect == Dialect::V2025_12 => err("serial", "PARSING FAILED"),
                _ => err("serial", "INVALID ARGUMENT COUNT"),
            },
            "feature" => {
                let (name, on) = match args.strip_prefix('-') {
                    Some(n) => (n, false),
                    None => (args, true),
                };
                if self.unavailable_features.iter().any(|f| f == name) {
                    return if self.dialect == Dialect::V2025_12 {
                        format!("Unavailable {name}\r\n")
                    } else {
                        "unavailable\r\n".into()
                    };
                }
                match self.live.features.get_mut(name) {
                    Some(v) => {
                        *v = on;
                        format!("{} {name}\r\n", if on { "Enabled" } else { "Disabled" })
                    }
                    None => err("feature", &format!("INVALID NAME: {name}")),
                }
            }
            "set" => {
                let Some((k, v)) = args.split_once('=') else {
                    return match self.live.settings.get(args) {
                        Some(v) => format!("{args} = {v}\r\n"),
                        None => err("set", &format!("INVALID NAME: {args}")),
                    };
                };
                let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_string());
                // MODE_STRING (craft_name, pilot_name): 1..=16 chars, "-" = empty.
                if k.ends_with("_name") && (v.is_empty() || v.len() > 16) {
                    return err("set", "STRING MUST BE 1-16 CHARACTERS OR '-' FOR EMPTY");
                }
                if let Some(allowed) = allowed_values(self.dialect, &k, self.live.settings.get(&k))
                {
                    if !allowed.iter().any(|x| x.eq_ignore_ascii_case(&v)) {
                        return format!(
                            "{}Allowed values: {}\r\n",
                            err("set", "INVALID VALUE"),
                            allowed.join(", ")
                        );
                    }
                }
                match self.live.settings.get_mut(&k) {
                    Some(slot) => {
                        *slot = v.clone();
                        format!("{k} set to {v}\r\n")
                    }
                    None => err("set", &format!("INVALID NAME: {args}")),
                }
            }
            "get" => match self.live.settings.get(args) {
                Some(v) => format!("{args} = {v}\r\n"),
                None => err("get", &format!("INVALID NAME: {args}")),
            },
            "aux" => {
                let n: Vec<u16> = args
                    .split_whitespace()
                    .filter_map(|x| x.parse().ok())
                    .collect();
                if n.first().is_some_and(|i| *i >= u16::from(AuxSlot::COUNT)) {
                    return err(
                        "aux",
                        &format!("INDEX NOT BETWEEN 0 AND {}", AuxSlot::COUNT - 1),
                    );
                }
                if n.len() < 5 {
                    return err("aux", "INVALID ARGUMENT COUNT");
                }
                let a = AuxSlot {
                    slot: n[0] as u8,
                    mode_id: n[1] as u8,
                    aux_channel: n[2] as u8,
                    range_min: n[3],
                    range_max: n[4],
                    logic: n.get(5).copied().unwrap_or(0) as u8,
                    linked_to: n.get(6).copied().unwrap_or(0) as u8,
                };
                self.live.aux.insert(a.slot, a);
                a.to_cli() + "\r\n"
            }
            "vtxtable" if args.is_empty() => self
                .live
                .vtxtable
                .render()
                .into_iter()
                .map(|l| l + "\r\n")
                .collect(),
            "vtxtable" => match self.live.vtxtable.apply(args) {
                Ok(()) => String::new(),
                Err(e) => err("vtxtable", &e),
            },
            "resource" => {
                let p: Vec<&str> = args.split_whitespace().collect();
                if p.len() == 3 {
                    self.live
                        .resources
                        .insert(format!("{} {}", p[0], p[1]), p[2].to_string());
                    String::new()
                } else {
                    err("resource", "INVALID ARGUMENT")
                }
            }
            _ => unknown_command(),
        }
    }
}

impl Connection for MockFc {
    /// MSP_SET_REBOOT reboots, dropping unsaved RAM changes; identification codes are
    /// answered from `msp_reply`.
    fn msp_request(&mut self, code: u16, _payload: &[u8]) -> AppResult<Vec<u8>> {
        if self.unplugged {
            return Err(AppError::Serial("port closed".into()));
        }
        if self.in_cli {
            return Err(AppError::Cli("MSP request while in CLI mode".into()));
        }
        if code == super::session::MSP_SET_REBOOT {
            self.log.push("MSP_SET_REBOOT".into());
            self.live = self.saved.clone();
            self.reboots += 1;
            return Ok(vec![0]);
        }
        self.msp_reply(code)
            .ok_or_else(|| AppError::Cli(format!("MSP {code}: unsupported by MockFc")))
    }

    fn enter_cli(&mut self) -> AppResult<()> {
        self.in_cli = true;
        Ok(())
    }

    fn cli_exec(&mut self, line: &str) -> AppResult<String> {
        if self.unplugged {
            return Err(AppError::Serial("port closed".into()));
        }
        if !self.in_cli {
            return Err(AppError::Cli("not in CLI mode".into()));
        }
        self.log.push(line.to_string());
        if let Some((prefix, fault)) = self.link_fault.clone() {
            if line.starts_with(prefix.as_str()) {
                self.link_fault = None;
                if fault == LinkFault::Unplug {
                    self.unplugged = true;
                    self.in_cli = false;
                    self.live = self.saved.clone();
                    return Err(AppError::Serial("device disconnected".into()));
                }
                return Err(AppError::Serial(format!(
                    "timeout waiting for reply to `{line}`"
                )));
            }
        }
        Ok(self.exec(line))
    }

    fn exit_cli(&mut self, save: bool) -> AppResult<()> {
        if self.unplugged {
            let exit = if self.dialect == Dialect::V4_4 {
                "exit"
            } else {
                "exit noreboot"
            };
            self.log.push(if save { "save" } else { exit }.into());
            return Err(AppError::Serial("port closed".into()));
        }
        if !self.in_cli {
            return Err(AppError::Cli("not in CLI mode".into()));
        }
        if save {
            self.log.push("save".into());
            if self.batch_error {
                // Verified on 4.4.3 / 4.5.5 / 2025.12.5 SITL: the failed batch is saved anyway.
                self.saves_after_error += 1;
            }
            self.saved = self.live.clone();
        } else if self.dialect == Dialect::V4_4 {
            self.log.push("exit".into());
            self.live = self.saved.clone();
        } else {
            // device::Connection contract: `exit noreboot` on 4.5+ - leaves the CLI, keeps
            // the unsaved changes active, no reboot.
            self.log.push("exit noreboot".into());
            self.in_cli = false;
            self.batch_error = false;
            return Ok(());
        }
        self.in_cli = false;
        self.batch_error = false;
        self.reboots += 1;
        Ok(())
    }
}

impl Reconnect for MockFc {
    fn reconnect(&mut self, _timeout: std::time::Duration) -> AppResult<()> {
        Ok(())
    }
}
