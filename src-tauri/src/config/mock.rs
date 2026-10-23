//! C1: `MockFc`, a stateful Betaflight CLI simulator implementing `device::Connection`,
//! seeded from dump-format fixtures in `src-tauri/tests/fixtures/<firmware>/`.
//!
//! The fixtures are synthetic (hand-written from the 4.4.3 / 4.5.x / 2025.12.x CLI
//! sources), not recordings from hardware. The simulator models what the config
//! workstream depends on: `version`, `serial`, `feature`, `set`/`get`, `aux`, `resource`,
//! `batch start|end`, `defaults nosave`, `dump all`, `diff all`, save/exit semantics
//! (unsaved changes are lost on `exit`; `save` is refused after a batch error) and
//! error injection.

use std::collections::BTreeMap;

use super::firmware::Dialect;
use super::generate::serial_cmd;
use super::session::Reconnect;
use super::snapshot::{parse_dump, parse_serial_line, AuxSlot, Snapshot};
use crate::device::Connection;
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
struct State {
    ports: Vec<super::snapshot::PortState>,
    features: BTreeMap<String, bool>,
    settings: BTreeMap<String, String>,
    aux: BTreeMap<u8, AuxSlot>,
    resources: BTreeMap<String, String>,
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
        }
    }
}

pub struct MockFc {
    pub version: String,
    pub board_name: String,
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

impl MockFc {
    pub fn from_dump(dump: &str) -> MockFc {
        let snap = parse_dump(dump);
        let version = snap
            .version
            .clone()
            .expect("fixture needs a version banner");
        let dialect = Dialect::from_version(&version).unwrap_or(Dialect::V2025_12);
        let st = State::from_snapshot(&snap);
        MockFc {
            board_name: snap.board_name.clone().unwrap_or_default(),
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
        format!(
            "# Betaflight / STM32F405 (S405) {} Jan  1 2026 / 00:00:00 (mock) MSP API: 1.46\r\n# board_name: {}\r\n",
            self.version, self.board_name
        )
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
            return err(line.split(' ').next().unwrap_or(line), "INJECTED FAILURE");
        }
        let r = self.exec_inner(line);
        if r.contains("###ERROR") {
            self.batch_error = true;
        }
        r
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
                "end" => "Command batch ended\r\n".into(),
                _ => err("batch", "INVALID ARGUMENT"),
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
                    if let Some(q) = self.live.ports.iter_mut().find(|q| q.name == p.name) {
                        *q = p;
                    }
                    String::new()
                }
                _ => err("serial", "INVALID PORT"),
            },
            "feature" => {
                let (name, on) = match args.strip_prefix('-') {
                    Some(n) => (n, false),
                    None => (args, true),
                };
                match self.live.features.get_mut(name) {
                    Some(v) => {
                        *v = on;
                        format!("{} {name}\r\n", if on { "Enabled" } else { "Disabled" })
                    }
                    None => err("feature", "INVALID NAME"),
                }
            }
            "set" => {
                let Some((k, v)) = args.split_once('=') else {
                    return err("set", "INVALID ARGUMENT");
                };
                let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_string());
                match self.live.settings.get_mut(&k) {
                    Some(slot) => {
                        *slot = v.clone();
                        format!("{k} set to {v}\r\n")
                    }
                    None => err("set", "INVALID NAME"),
                }
            }
            "get" => match self.live.settings.get(args) {
                Some(v) => format!("{args} = {v}\r\n"),
                None => err("get", "INVALID NAME"),
            },
            "aux" => {
                let n: Vec<u16> = args
                    .split_whitespace()
                    .filter_map(|x| x.parse().ok())
                    .collect();
                if n.len() < 5 || n[0] >= u16::from(AuxSlot::COUNT) {
                    return err("aux", "INVALID ARGUMENT");
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
            _ => err(cmd, "UNKNOWN COMMAND"),
        }
    }
}

impl Connection for MockFc {
    fn msp_request(&mut self, _code: u16, _payload: &[u8]) -> AppResult<Vec<u8>> {
        Err(AppError::NotImplemented("MockFc::msp_request"))
    }

    fn enter_cli(&mut self) -> AppResult<()> {
        self.in_cli = true;
        Ok(())
    }

    fn cli_exec(&mut self, line: &str) -> AppResult<String> {
        if !self.in_cli {
            return Err(AppError::Cli("not in CLI mode".into()));
        }
        self.log.push(line.to_string());
        Ok(self.exec(line))
    }

    fn exit_cli(&mut self, save: bool) -> AppResult<()> {
        if !self.in_cli {
            return Err(AppError::Cli("not in CLI mode".into()));
        }
        if save {
            self.log.push("save".into());
            if self.batch_error {
                return Err(AppError::Cli(
                    "save refused: PLEASE FIX ERRORS THEN 'SAVE'".into(),
                ));
            }
            self.saved = self.live.clone();
        } else {
            self.log.push("exit".into());
            self.live = self.saved.clone();
        }
        self.in_cli = false;
        self.batch_error = false;
        self.reboots += 1;
        Ok(())
    }
}

impl Reconnect for MockFc {
    fn reconnect(&mut self) -> AppResult<()> {
        Ok(())
    }
}
