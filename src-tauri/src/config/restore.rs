//! `config_restore_after_flash` (ARCHITECTURE §5.2): replay a pre-flash `diff all` onto the
//! freshly flashed firmware, also across firmware lines, continuing past rejected lines.
//!
//! Split in two:
//! - [`plan_restore`] (pure, golden-tested): every backup line -> send (possibly mapped to the
//!   target dialect) or skip with a reason. Existence on the target is always taken from the
//!   target's live `dump all`; the generated tables in [`super::restore_tables`] only classify
//!   (removed on the line vs. compiled out of this build) and map renames.
//! - [`run`]: version + `dump all` -> plan -> `batch start` + lines -> `batch end` -> `save` ->
//!   reconnect -> read-back (warnings only).
//!
//! Secrets: no binding phrase is involved, but every line, error and warning that leaves this
//! module goes through [`mask_secrets`], progress never carries a line and nothing is logged.

use std::collections::{BTreeSet, HashMap};

use super::catalog::{beeper_cli_name, BEEPERS};
use super::firmware::Dialect;
use super::generate::serial_cmd;
use super::guard::{replay_lines, BOARD_IDENTITY_VERBS};
use super::restore_tables::{ADJUSTMENT_FUNCTIONS, FEATURES, SETTINGS, VALUES};
use super::serial_functions::{port_identifier, GIMBAL, LIDAR_NL, OSD_CUSTOM_TEXT};
use super::session::{discard, is_cli_error, read_version, Progress, REBOOT_TIMEOUT};
use super::snapshot::{parse_dump, parse_serial_line, Snapshot};
use crate::device::FcLink;
use crate::error::{AppError, AppResult, ConfigErrorReason};
use crate::model::{
    Backup, ConfigProgress, ConfigStage, RestoreFailure, RestoreReport, RestoreSkip,
    RestoreSkipReason,
};

use RestoreSkipReason as R;

// ------------------------------------------------------------------ tables

fn bit(d: Dialect) -> u8 {
    1 << Dialect::ALL.iter().position(|x| *x == d).unwrap_or(0)
}

fn line_label(d: Dialect) -> &'static str {
    d.as_str()
}

/// Lines (mask) that have setting `name`; None = not a setting on any supported line.
pub fn setting_lines(name: &str) -> Option<u8> {
    SETTINGS
        .binary_search_by(|(n, _)| (*n).cmp(name))
        .ok()
        .map(|i| SETTINGS[i].1)
}

/// Lines that have value `value` of `setting`; None = the value exists wherever the setting does.
fn value_lines(setting: &str, value: &str) -> Option<u8> {
    VALUES
        .iter()
        .find(|(s, v, _)| *s == setting && v.eq_ignore_ascii_case(value))
        .map(|(_, _, m)| *m)
}

fn feature_lines(name: &str) -> Option<u8> {
    FEATURES
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, m)| *m)
}

/// Same field, different CLI name per line: `(name, lines)` classes.
pub const SETTING_RENAMES: &[&[(&str, u8)]] = &[
    // pid.c PID_LEVEL P: 4.5 renamed (angle mode rework kept the field and range 0..200).
    &[("angle_level_strength", 0b0001), ("angle_p_gain", 0b1110)],
    // levelAngleLimit: 4.4 range 10..90, 4.5 10..85, 2025.12+ 10..80 (out of range -> FC rejects).
    &[("level_limit", 0b0001), ("angle_limit", 0b1110)],
    // motorConfig.digitalIdleOffsetValue (PR #13936).
    &[("dshot_idle_value", 0b0011), ("motor_idle", 0b1100)],
];

/// Same enum entry, different name per line: `(setting, [(value, lines)])`.
pub const VALUE_RENAMES: &[(&str, &[(&str, u8)])] = &[
    (
        "mag_hardware",
        &[("MAG_MPU925X_AK8963", 0b0001), ("MPU925X_AK8963", 0b1110)],
    ),
    ("mag_hardware", &[("QMC5883", 0b0111), ("QMC5883L", 0b1000)]),
    (
        "debug_mode",
        &[("DUAL_GYRO_RAW", 0b0011), ("MULTI_GYRO_RAW", 0b1100)],
    ),
    (
        "debug_mode",
        &[("DUAL_GYRO_DIFF", 0b0011), ("MULTI_GYRO_DIFF", 0b1100)],
    ),
    (
        "debug_mode",
        &[("DUAL_GYRO_SCALED", 0b0011), ("MULTI_GYRO_SCALED", 0b1100)],
    ),
    ("debug_mode", &[("D_MIN", 0b0011), ("D_MAX", 0b1100)]),
];

/// Settings whose meaning changes at `boundary` (the first line with the new meaning): skipped
/// whenever the backup and the target are on different sides of it.
pub const SEMANTICS_CHANGED: &[(&[&str], Dialect, &str)] = &[
    (
        &[
            "d_roll",
            "d_pitch",
            "d_yaw",
            "d_min_roll",
            "d_min_pitch",
            "d_min_yaw",
            "d_max_roll",
            "d_max_pitch",
            "d_max_yaw",
            "simplified_dmax_gain",
            "simplified_d_max_gain",
        ],
        Dialect::V2025_12,
        "D-term changed meaning in 2025.12 (4.x: d_* is the maximum and d_min_* the minimum; \
         2025.12+: d_* is the base and d_max_* the maximum); re-tune D",
    ),
    (
        &["horizon_transition", "horizon_limit_sticks"],
        Dialect::V4_5,
        "horizon mode was reworked in 4.5",
    ),
];

/// 2026.6 per-battery-profile settings (`PROFILE_BATTERY_VALUE`) that are master settings on
/// older lines (`battery_profile_name` is 2026.6-only).
pub const BATTERY_SETTINGS: &[&str] = &[
    "bat_capacity",
    "vbat_max_cell_voltage",
    "vbat_full_cell_voltage",
    "vbat_min_cell_voltage",
    "vbat_warning_cell_voltage",
    "cbat_alert_percent",
    "force_battery_cell_count",
];

/// Permanent box ids (`msp/msp_box.c`) and the lines that have them. Ids never move; 3 and 11
/// were unused (commented out) before 2025.12 reused them for ALTHOLD / POS HOLD.
pub const AUX_MODES: &[(u8, &str, u8)] = &[
    (0, "ARM", 15),
    (1, "ANGLE", 15),
    (2, "HORIZON", 15),
    (3, "ALTHOLD", 12),
    (4, "ANTI GRAVITY", 15),
    (5, "MAG", 15),
    (6, "HEADFREE", 15),
    (7, "HEADADJ", 15),
    (8, "CAMSTAB", 15),
    (11, "POS HOLD", 12),
    (12, "PASSTHRU", 15),
    (13, "BEEPER", 15),
    (15, "LEDLOW", 15),
    (17, "CALIB", 15),
    (19, "OSD DISABLE", 15),
    (20, "TELEMETRY", 15),
    (23, "SERVO1", 15),
    (24, "SERVO2", 15),
    (25, "SERVO3", 15),
    (26, "BLACKBOX", 15),
    (27, "FAILSAFE", 15),
    (28, "AIR MODE", 15),
    (29, "3D DISABLE", 15),
    (30, "FPV ANGLE MIX", 15),
    (31, "BLACKBOX ERASE", 15),
    (32, "CAMERA CONTROL 1", 15),
    (33, "CAMERA CONTROL 2", 15),
    (34, "CAMERA CONTROL 3", 15),
    (35, "FLIP OVER AFTER CRASH", 15),
    (36, "PREARM", 15),
    (37, "GPS BEEP SATELLITE COUNT", 15),
    (39, "VTX PIT MODE", 15),
    (40, "USER1", 15),
    (41, "USER2", 15),
    (42, "USER3", 15),
    (43, "USER4", 15),
    (44, "PID AUDIO", 15),
    (45, "PARALYZE", 15),
    (46, "GPS RESCUE", 15),
    (47, "ACRO TRAINER", 15),
    (48, "VTX CONTROL DISABLE", 15),
    (49, "LAUNCH CONTROL", 15),
    (50, "MSP OVERRIDE", 15),
    (51, "STICK COMMANDS DISABLE", 15),
    (52, "BEEPER MUTE", 15),
    (53, "READY", 15),
    (54, "LAP TIMER RESET", 14),
    (55, "CHIRP", 12),
    (56, "AUTOPILOT", 8),
    (57, "WP CAPTURE", 8),
];

fn aux_mode(id: u8) -> Option<(&'static str, u8)> {
    AUX_MODES
        .iter()
        .find(|(i, _, _)| *i == id)
        .map(|(_, n, m)| (*n, *m))
}

/// Serial function bits that exist on `d` (`io/serial.h`).
fn serial_bits(d: Dialect) -> u32 {
    let mut all = (1u32 << 18) - 1; // MSP .. VTX_MSP: every line
    if d >= Dialect::V2025_12 {
        all |= GIMBAL;
    }
    if d >= Dialect::V2026_6 {
        all |= LIDAR_NL | OSD_CUSTOM_TEXT;
    }
    all
}

// ------------------------------------------------------------------ secrets

const SECRET_SETTINGS: &[&str] = &["expresslrs_uid"];

/// Replace the value after any secret setting name (`expresslrs_uid`) with `***`, up to the end
/// of the line or the closing `###` of a CLI error. Everything leaving this module goes
/// through here.
pub fn mask_secrets(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let lower = line.to_ascii_lowercase();
        let hit = SECRET_SETTINGS
            .iter()
            .filter_map(|s| lower.find(s).map(|p| p + s.len()))
            .min();
        match hit {
            Some(end) => {
                out.push_str(&line[..end]);
                out.push_str(" = ***");
                if let Some(close) = line[end..].find("###") {
                    out.push_str(&line[end + close..]);
                }
            }
            None => out.push_str(line),
        }
    }
    out
}

// ------------------------------------------------------------------ target view

/// `set` values per `dump all` section, for the read-back.
#[derive(Debug, Default, Clone)]
pub struct Sections {
    master: HashMap<String, String>,
    profile: HashMap<u8, HashMap<String, String>>,
    rate: HashMap<u8, HashMap<String, String>>,
    battery: HashMap<u8, HashMap<String, String>>,
}

impl Sections {
    pub fn parse(text: &str) -> Sections {
        enum Cur {
            Master,
            Profile(u8),
            Rate(u8),
            Battery(u8),
        }
        let mut s = Sections::default();
        let mut cur = Cur::Master;
        for raw in text.lines() {
            let line = raw.trim();
            let mut t = line.split_whitespace();
            let verb = t.next().unwrap_or("");
            let idx = t.next().and_then(|x| x.parse::<u8>().ok());
            match (verb, idx) {
                ("profile", Some(i)) => cur = Cur::Profile(i),
                ("rateprofile", Some(i)) => cur = Cur::Rate(i),
                ("battery_profile", Some(i)) => cur = Cur::Battery(i),
                ("set", _) => {
                    if let Some((k, v)) = line[3..].split_once('=') {
                        let map = match cur {
                            Cur::Master => &mut s.master,
                            Cur::Profile(i) => s.profile.entry(i).or_default(),
                            Cur::Rate(i) => s.rate.entry(i).or_default(),
                            Cur::Battery(i) => s.battery.entry(i).or_default(),
                        };
                        map.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
                    }
                }
                _ => {}
            }
        }
        s
    }

    fn get(&self, name: &str, ctx: &Ctx) -> Option<&String> {
        self.master
            .get(name)
            .or_else(|| self.profile.get(&ctx.pid).and_then(|m| m.get(name)))
            .or_else(|| self.rate.get(&ctx.rate).and_then(|m| m.get(name)))
            .or_else(|| self.battery.get(&ctx.batt).and_then(|m| m.get(name)))
    }
}

/// What the plan needs from the target: its `dump all`.
#[derive(Debug, Clone)]
pub struct Target {
    pub dialect: Dialect,
    pub snapshot: Snapshot,
    settings: BTreeSet<String>,
}

impl Target {
    pub fn from_dump(dialect: Dialect, dump: &str) -> Target {
        let snapshot = parse_dump(dump);
        let settings = Sections::parse(dump);
        let mut names: BTreeSet<String> = settings.master.keys().cloned().collect();
        for m in settings
            .profile
            .values()
            .chain(settings.rate.values())
            .chain(settings.battery.values())
        {
            names.extend(m.keys().cloned());
        }
        Target {
            dialect,
            snapshot,
            settings: names,
        }
    }
}

// ------------------------------------------------------------------ plan

/// Profile selection while replaying (for the read-back lookup).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ctx {
    pub pid: u8,
    pub rate: u8,
    pub batt: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    None,
    Set {
        name: String,
        value: String,
        ctx: Ctx,
    },
    Feature {
        name: String,
        on: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Send `cmd` (== `line` unless mapped).
    Send {
        line: String,
        cmd: String,
        check: Check,
    },
    Skip {
        line: String,
        reason: RestoreSkipReason,
        detail: String,
    },
}

#[derive(Debug, Clone, Default)]
pub struct Plan {
    /// One step per non-empty backup body line, in order.
    pub steps: Vec<Step>,
    pub warnings: Vec<String>,
}

impl Plan {
    /// Human-readable form for the goldens (secrets masked).
    pub fn render(&self) -> String {
        let mut out = String::new();
        for s in &self.steps {
            match s {
                Step::Send { line, cmd, .. } if line == cmd => {
                    out += &format!("SEND {}\n", mask_secrets(cmd))
                }
                Step::Send { line, cmd, .. } => {
                    out += &format!("SEND {}    <- {}\n", mask_secrets(cmd), mask_secrets(line))
                }
                Step::Skip {
                    line,
                    reason,
                    detail,
                } => {
                    let r = serde_json::to_value(reason)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_string))
                        .unwrap_or_default();
                    out += &format!("SKIP {r} {}", mask_secrets(line));
                    if !detail.is_empty() {
                        out += &format!("    ({detail})");
                    }
                    out.push('\n');
                }
            }
        }
        for w in &self.warnings {
            out += &format!("WARN {}\n", mask_secrets(w));
        }
        out
    }
}

fn skip(line: &str, reason: RestoreSkipReason, detail: impl Into<String>) -> Step {
    Step::Skip {
        line: line.to_string(),
        reason,
        detail: detail.into(),
    }
}

fn send(line: &str, cmd: String, check: Check) -> Step {
    Step::Send {
        line: line.to_string(),
        cmd,
        check,
    }
}

/// The body of a backup file: everything after the `# bfsw-backup` header block.
pub fn backup_body(text: &str) -> &str {
    if text.starts_with("# bfsw-backup") {
        if let Some(i) = text.find("\n\n") {
            return &text[i + 2..];
        }
        if let Some(i) = text.find("\r\n\r\n") {
            return &text[i + 4..];
        }
    }
    text
}

struct Planner<'a> {
    from: Dialect,
    to: Dialect,
    target: &'a Target,
    ctx: Ctx,
    /// 2026.6 source: the battery profile section being read (None outside one).
    batt_section: Option<u8>,
    /// 2026.6 -> older: the selected battery profile (the last `battery_profile N`).
    selected_batt: u8,
    battery_into_profile: Vec<String>,
    dropped_bits: Vec<String>,
    other_batt: bool,
    d_term: bool,
    warnings: Vec<String>,
}

impl Planner<'_> {
    fn changed(&self) -> bool {
        self.from != self.to
    }

    fn set(&mut self, line: &str) -> Step {
        let Some((k, v)) = line[3..].split_once('=') else {
            return send(line, line.to_string(), Check::None);
        };
        let name = k.trim().to_ascii_lowercase();
        let value = v.trim().to_string();
        let (from, to) = (self.from, self.to);

        // 2026.6 -> older: only the selected battery profile can be restored.
        if let Some(n) = self.batt_section {
            if !to.has_battery_profiles() && n != self.selected_batt {
                self.other_batt = true;
                return skip(
                    line,
                    R::OtherBatteryProfile,
                    format!(
                        "battery profile {n}; Betaflight {} has one set of battery settings \
                         (profile {} is restored)",
                        line_label(to),
                        self.selected_batt
                    ),
                );
            }
        }

        if self.changed() {
            for (names, boundary, why) in SEMANTICS_CHANGED {
                if names.contains(&name.as_str()) && ((from < *boundary) != (to < *boundary)) {
                    if *boundary == Dialect::V2025_12 {
                        self.d_term = true;
                    }
                    return skip(line, R::SemanticsChanged, *why);
                }
            }
        }

        let mut target_name = name.clone();
        if self.changed() {
            for class in SETTING_RENAMES {
                if class.iter().any(|(n, m)| *n == name && m & bit(from) != 0) {
                    if let Some((n, _)) = class.iter().find(|(_, m)| m & bit(to) != 0) {
                        target_name = n.to_string();
                    }
                }
            }
        }

        if !self.target.settings.contains(&target_name) {
            return match setting_lines(&target_name) {
                Some(m) if m & bit(to) != 0 => skip(
                    line,
                    R::NotInBuild,
                    format!(
                        "{target_name} exists on Betaflight {} but not in this build",
                        line_label(to)
                    ),
                ),
                _ => skip(
                    line,
                    R::RemovedOnTarget,
                    format!(
                        "setting {target_name} does not exist on Betaflight {}",
                        line_label(to)
                    ),
                ),
            };
        }

        let mut target_value = value.clone();
        if self.changed() {
            for (s, class) in VALUE_RENAMES {
                if *s != target_name && *s != name {
                    continue;
                }
                if class
                    .iter()
                    .any(|(v, m)| v.eq_ignore_ascii_case(&value) && m & bit(from) != 0)
                {
                    if let Some((v, _)) = class.iter().find(|(_, m)| m & bit(to) != 0) {
                        target_value = v.to_string();
                    }
                }
            }
            if let Some(m) = value_lines(&target_name, &target_value) {
                if m & bit(to) == 0 {
                    return skip(
                        line,
                        R::RemovedOnTarget,
                        format!(
                            "value {target_value} of {target_name} does not exist on Betaflight {}",
                            line_label(to)
                        ),
                    );
                }
            }
            if !from.has_battery_profiles()
                && to.has_battery_profiles()
                && BATTERY_SETTINGS.contains(&target_name.as_str())
            {
                self.battery_into_profile.push(target_name.clone());
            }
        }

        let cmd = if target_name == name && target_value == value {
            line.to_string()
        } else {
            format!("set {target_name} = {target_value}")
        };
        send(
            line,
            cmd,
            Check::Set {
                name: target_name,
                value: target_value,
                ctx: self.ctx,
            },
        )
    }

    fn feature(&mut self, line: &str, arg: &str) -> Step {
        let (name, on) = match arg.strip_prefix('-') {
            Some(n) => (n.to_ascii_uppercase(), false),
            None => (arg.to_ascii_uppercase(), true),
        };
        let to = self.to;
        if let Some(m) = feature_lines(&name) {
            if m & bit(to) == 0 {
                return skip(
                    line,
                    R::RemovedOnTarget,
                    format!(
                        "feature {name} has no CLI name on Betaflight {}",
                        line_label(to)
                    ),
                );
            }
        }
        let features = &self.target.snapshot.features;
        if !features.is_empty() && !features.contains_key(&name) {
            return skip(
                line,
                R::NotInBuild,
                format!("feature {name} is not in this build"),
            );
        }
        send(line, line.to_string(), Check::Feature { name, on })
    }

    fn beeper(&mut self, line: &str, arg: &str) -> Step {
        if !self.changed() {
            return send(line, line.to_string(), Check::None);
        }
        let (name, off) = match arg.strip_prefix('-') {
            Some(n) => (n, "-"),
            None => (arg, ""),
        };
        let Some(b) = BEEPERS.iter().find(|b| {
            beeper_cli_name(**b, self.from).is_some_and(|n| n.eq_ignore_ascii_case(name))
        }) else {
            return send(line, line.to_string(), Check::None);
        };
        match beeper_cli_name(*b, self.to) {
            Some(n) if n.eq_ignore_ascii_case(name) => send(line, line.to_string(), Check::None),
            Some(n) => send(line, format!("beeper {off}{n}"), Check::None),
            None => skip(
                line,
                R::RemovedOnTarget,
                format!(
                    "beeper {} does not exist on Betaflight {}",
                    name.to_ascii_uppercase(),
                    line_label(self.to)
                ),
            ),
        }
    }

    fn serial(&mut self, line: &str) -> Step {
        let (from, to) = (self.from, self.to);
        let Some(mut p) = parse_serial_line(line, from) else {
            return skip(
                line,
                R::PortNotAvailable,
                format!("unknown serial port for Betaflight {}", line_label(from)),
            );
        };
        if !self.target.snapshot.ports.is_empty()
            && !self.target.snapshot.ports.iter().any(|q| q.name == p.name)
        {
            return skip(
                line,
                R::PortNotAvailable,
                format!("{} does not exist on this board / firmware", p.name),
            );
        }
        if !self.changed() {
            return send(line, line.to_string(), Check::None);
        }
        let dropped = p.function_mask & !serial_bits(to);
        if dropped != 0 {
            self.dropped_bits
                .push(format!("{} (mask {dropped})", p.name));
        }
        let Some(id) = port_identifier(&p.name, to) else {
            return skip(
                line,
                R::PortNotAvailable,
                format!(
                    "{} has no identifier on Betaflight {}",
                    p.name,
                    line_label(to)
                ),
            );
        };
        p.identifier = id;
        let mask = p.function_mask & serial_bits(to);
        send(line, serial_cmd(&p, mask, to), Check::None)
    }

    fn aux(&mut self, line: &str) -> Step {
        if !self.changed() {
            return send(line, line.to_string(), Check::None);
        }
        let t: Vec<&str> = line.split_whitespace().collect();
        let to = self.to;
        let check = |tok: Option<&&str>, linked: bool| -> Result<(), String> {
            let Some(id) = tok.and_then(|x| x.parse::<u8>().ok()) else {
                return Ok(());
            };
            if linked && id == 0 {
                return Ok(());
            }
            match aux_mode(id) {
                Some((_, m)) if m & bit(to) != 0 => Ok(()),
                Some((n, _)) => Err(format!(
                    "mode {n} ({id}) does not exist on Betaflight {}",
                    line_label(to)
                )),
                None => Err(format!("mode id {id} is unknown")),
            }
        };
        // aux <slot> <modeId> <ch> <start> <end> <logic> <linkedTo>
        if let Err(e) = check(t.get(2), false) {
            return skip(line, R::NoEquivalentMode, e);
        }
        if let Err(e) = check(t.get(7), true) {
            return skip(line, R::NoEquivalentMode, format!("linked {e}"));
        }
        send(line, line.to_string(), Check::None)
    }

    fn adjrange(&mut self, line: &str) -> Step {
        if !self.changed() {
            return send(line, line.to_string(), Check::None);
        }
        // adjrange <slot> 0 <ch> <start> <end> <function> <switchCh> <center> <scale>
        let mut t: Vec<String> = line.split_whitespace().map(String::from).collect();
        let Some(f) = t.get(6).and_then(|x| x.parse::<usize>().ok()) else {
            return send(line, line.to_string(), Check::None);
        };
        let src = ADJUSTMENT_FUNCTIONS[Dialect::ALL.iter().position(|d| *d == self.from).unwrap()];
        let dst = ADJUSTMENT_FUNCTIONS[Dialect::ALL.iter().position(|d| *d == self.to).unwrap()];
        let Some(name) = src.get(f) else {
            return skip(
                line,
                R::NoEquivalentMode,
                format!("adjustment function {f} is unknown"),
            );
        };
        match dst.iter().position(|n| n == name) {
            Some(g) if g == f => send(line, line.to_string(), Check::None),
            Some(g) => {
                t[6] = g.to_string();
                send(line, t.join(" "), Check::None)
            }
            None => skip(
                line,
                R::NoEquivalentMode,
                format!(
                    "adjustment {name} does not exist on Betaflight {}",
                    line_label(self.to)
                ),
            ),
        }
    }

    fn line(&mut self, line: &str, first_command: bool) -> Step {
        let verb = line
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let arg = line[verb.len().min(line.len())..].trim();
        let (from, to) = (self.from, self.to);
        if line.starts_with('#') {
            return skip(line, R::Comment, "");
        }
        if first_command && line.eq_ignore_ascii_case("diff all")
            || ["batch start", "batch end", "save"]
                .iter()
                .any(|c| line.eq_ignore_ascii_case(c))
        {
            return skip(line, R::SessionControl, "the restore sends its own");
        }
        if BOARD_IDENTITY_VERBS.contains(&verb.as_str()) {
            return skip(
                line,
                R::BoardIdentity,
                "identity of the hardware, never replayed",
            );
        }
        match verb.as_str() {
            "resource" | "timer" | "dma" if self.changed() => skip(
                line,
                R::HardwareLineChanged,
                format!(
                    "pin maps differ between Betaflight {} and {}; the new firmware's board \
                     defaults are kept",
                    line_label(from),
                    line_label(to)
                ),
            ),
            "set" => self.set(line),
            "feature" => self.feature(line, arg),
            "beeper" => self.beeper(line, arg),
            "serial" => self.serial(line),
            "aux" => self.aux(line),
            "adjrange" => self.adjrange(line),
            "profile" | "rateprofile" => {
                self.batt_section = None;
                if let Ok(i) = arg.parse::<u8>() {
                    if verb == "profile" {
                        self.ctx.pid = i;
                    } else {
                        self.ctx.rate = i;
                    }
                }
                send(line, line.to_string(), Check::None)
            }
            "battery_profile" => {
                let i = arg.parse::<u8>().ok();
                self.batt_section = i;
                if !to.has_battery_profiles() {
                    return skip(
                        line,
                        R::RemovedOnTarget,
                        format!(
                            "battery profiles are new in 2026.6; Betaflight {} has one set of \
                             battery settings",
                            line_label(to)
                        ),
                    );
                }
                if let Some(i) = i {
                    self.ctx.batt = i;
                }
                send(line, line.to_string(), Check::None)
            }
            "waypoint" if !to.has_battery_profiles() => skip(
                line,
                R::RemovedOnTarget,
                format!(
                    "flight plans are new in 2026.6, not on Betaflight {}",
                    line_label(to)
                ),
            ),
            _ => send(line, line.to_string(), Check::None),
        }
    }
}

/// Pure: classify and map every non-empty line of a `diff all` body (backup header removed)
/// taken on `from` for the target described by `target`.
pub fn plan_restore(body: &str, from: Dialect, target: &Target) -> Plan {
    let to = target.dialect;
    let lines: Vec<&str> = body
        .lines()
        .map(|l| l.trim_end_matches('\r').trim_matches(' '))
        .filter(|l| !l.is_empty())
        .collect();
    let selected_batt = lines
        .iter()
        .rev()
        .find_map(|l| l.strip_prefix("battery_profile "))
        .and_then(|x| x.trim().parse().ok())
        .unwrap_or(0);
    let mut p = Planner {
        from,
        to,
        target,
        ctx: Ctx::default(),
        batt_section: None,
        selected_batt,
        battery_into_profile: Vec::new(),
        dropped_bits: Vec::new(),
        other_batt: false,
        d_term: false,
        warnings: Vec::new(),
    };
    let mut steps = Vec::with_capacity(lines.len());
    let mut seen_command = false;
    for l in lines {
        let first = !seen_command && !l.starts_with('#');
        if !l.starts_with('#') {
            seen_command = true;
        }
        steps.push(p.line(l, first));
    }
    let mut warnings = std::mem::take(&mut p.warnings);
    if from != to {
        warnings.push(format!(
            "Settings were converted from Betaflight {} to {}; check the skipped lines.",
            line_label(from),
            line_label(to)
        ));
    }
    if p.d_term {
        warnings.push(
            "D-term settings were not restored: their meaning changed in 2025.12. Re-tune D \
             (or re-apply your tune) before flying."
                .into(),
        );
    }
    if !p.battery_into_profile.is_empty() {
        warnings.push(format!(
            "Battery settings ({}) were restored into battery profile 0, the selected one; \
             battery profiles 1 and 2 keep their defaults.",
            p.battery_into_profile.join(", ")
        ));
    }
    if p.other_batt {
        warnings.push(format!(
            "Only battery profile {} was restored: Betaflight {} has one set of battery settings.",
            p.selected_batt,
            line_label(to)
        ));
    }
    if !p.dropped_bits.is_empty() {
        warnings.push(format!(
            "Serial functions that do not exist on Betaflight {} were removed: {}.",
            line_label(to),
            p.dropped_bits.join(", ")
        ));
    }
    Plan { steps, warnings }
}

// ------------------------------------------------------------------ run

/// A backup checked and ready to restore (before the port is borrowed).
#[derive(Debug, Clone)]
pub struct Prepared {
    pub backup: Backup,
    pub body: String,
    pub from: Dialect,
}

fn reject(
    reason: ConfigErrorReason,
    msg: impl Into<String>,
    bk: &str,
    path: Option<&str>,
    stage: Option<ConfigStage>,
) -> AppError {
    AppError::restore_rejected(reason, msg, bk, path.map(str::to_string), stage)
}

/// Read and check the backup file: readable, from a supported line, a `diff all` whose lines
/// are all on the replay allowlist (`guard::replay_lines`). Nothing is sent.
pub fn prepare(backup: &Backup, text: std::io::Result<String>) -> AppResult<Prepared> {
    let id = backup.id.as_str();
    let path = Some(backup.path.as_str());
    let text = text.map_err(|e| {
        reject(
            ConfigErrorReason::BackupUnreadable,
            format!("backup {id} could not be read: {e}"),
            id,
            path,
            None,
        )
    })?;
    let from = Dialect::from_version(&backup.firmware_version).map_err(|_| {
        reject(
            ConfigErrorReason::BackupUnreadable,
            format!(
                "backup {id} was taken on Betaflight '{}', which is not a supported line",
                backup.firmware_version
            ),
            id,
            path,
            None,
        )
    })?;
    let body = backup_body(&text);
    replay_lines(body).map_err(|e| {
        reject(
            ConfigErrorReason::BackupNotDiffAll,
            format!(
                "backup {id} is not a restorable `diff all`: {}",
                mask_secrets(&e.to_string())
            ),
            id,
            path,
            None,
        )
    })?;
    Ok(Prepared {
        backup: backup.clone(),
        body: body.to_string(),
        from,
    })
}

fn known(u: &str) -> bool {
    !u.is_empty() && !u.eq_ignore_ascii_case("unknown")
}

fn is_link_error(e: &AppError) -> bool {
    matches!(
        e,
        AppError::Serial(_)
            | AppError::Msp(_)
            | AppError::Cli(_)
            | AppError::BoardNotFound(_)
            | AppError::NotConnected
            | AppError::Io(_)
    )
}

fn emit(cb: &mut dyn FnMut(ConfigProgress), stage: ConfigStage, fraction: f32, msg: String) {
    cb(ConfigProgress {
        stage,
        fraction,
        message: Some(msg),
        line: None,
        total: None,
    });
}

/// First line of the FC's error answer (`###ERROR IN set: INVALID NAME: ...###`), masked.
fn error_text(resp: &str) -> String {
    let l = resp
        .lines()
        .map(str::trim)
        .find(|l| l.contains("###ERROR") || l.to_ascii_lowercase().contains("unavailable"))
        .unwrap_or("rejected");
    mask_secrets(l)
}

/// Run the restore on the connected FC. `fc_uid` is the link's cached MSP_UID ("unknown" if
/// none).
pub fn run(
    conn: &mut dyn FcLink,
    src: &Prepared,
    fc_uid: &str,
    cb: Progress,
) -> AppResult<RestoreReport> {
    let bk = &src.backup;
    let id = bk.id.as_str();
    let lost = |stage: Option<ConfigStage>, e: &AppError| {
        reject(
            ConfigErrorReason::BoardNotReconnected,
            format!(
                "lost the connection to the FC: {}",
                mask_secrets(&e.to_string())
            ),
            id,
            None,
            stage,
        )
    };
    let start = |e: AppError| if is_link_error(&e) { lost(None, &e) } else { e };

    let (info, to) = read_version(conn).map_err(start)?;
    let dump = conn.cli_exec("dump all").map_err(start)?;
    let target = Target::from_dump(to, &dump);
    let fc_board = info
        .board_name
        .clone()
        .or_else(|| target.snapshot.board_name.clone())
        .unwrap_or_default();

    let mut warnings = Vec::new();
    if known(fc_uid) && known(&bk.board_uid) {
        if !fc_uid.eq_ignore_ascii_case(&bk.board_uid) {
            return Err(reject(
                ConfigErrorReason::BoardUidMismatch,
                format!(
                    "backup {id} was taken on another board (UID {}), the connected FC is {fc_uid}; nothing was changed",
                    bk.board_uid
                ),
                id,
                None,
                None,
            ));
        }
    } else if !fc_board.is_empty() && fc_board.eq_ignore_ascii_case(&bk.board_name) {
        warnings.push(format!(
            "The board's unique id is unknown; the backup was matched by board name ({fc_board}) only."
        ));
    } else {
        return Err(reject(
            ConfigErrorReason::BoardUidMismatch,
            format!(
                "the board's unique id is unknown and its name ({}) differs from the backup's ({}); nothing was changed",
                if fc_board.is_empty() { "unknown" } else { &fc_board },
                bk.board_name
            ),
            id,
            None,
            None,
        ));
    }

    let plan = plan_restore(&src.body, src.from, &target);
    warnings.extend(plan.warnings.iter().cloned());
    let total = plan.steps.len();
    let mut report = RestoreReport {
        applied: 0,
        failed: Vec::new(),
        skipped: Vec::new(),
        warnings: Vec::new(),
        saved: false,
        line_changed: src.from != to,
        from_version: bk.firmware_version.clone(),
        to_version: info.version.clone(),
    };

    let restoring = |cb: &mut dyn FnMut(ConfigProgress), n: usize| {
        cb(ConfigProgress {
            stage: ConfigStage::Restoring,
            fraction: if total == 0 {
                1.0
            } else {
                n as f32 / total as f32
            },
            message: Some(format!("line {n}/{total}")),
            line: Some(n),
            total: Some(total),
        });
    };
    restoring(cb, 0);

    let drop = |conn: &mut dyn FcLink, e: AppError| -> AppError {
        let _ = discard(conn, to);
        lost(Some(ConfigStage::Restoring), &e)
    };
    if let Err(e) = conn.cli_exec("batch start") {
        return Err(drop(conn, e));
    }
    let mut checks = Vec::new();
    for (i, step) in plan.steps.iter().enumerate() {
        match step {
            Step::Skip {
                line,
                reason,
                detail,
            } => report.skipped.push(RestoreSkip {
                line: mask_secrets(line),
                reason: *reason,
                detail: detail.clone(),
            }),
            Step::Send { line, cmd, check } => match conn.cli_exec(cmd) {
                Err(e) => return Err(drop(conn, e)),
                Ok(resp) if is_cli_error(cmd, &resp) => {
                    let mut error = error_text(&resp);
                    if cmd != line {
                        error += &format!(" (sent as `{}`)", mask_secrets(cmd));
                    }
                    report.failed.push(RestoreFailure {
                        line: mask_secrets(line),
                        error,
                    });
                }
                Ok(_) => {
                    report.applied += 1;
                    if *check != Check::None {
                        checks.push(check.clone());
                    }
                }
            },
        }
        restoring(cb, i + 1);
    }
    // `save` is refused while a batch with errors is active (cli.c prepareSave): end it first.
    if let Err(e) = conn.cli_exec("batch end") {
        return Err(drop(conn, e));
    }

    emit(cb, ConfigStage::Saving, 0.0, "save".into());
    conn.exit_cli(true)
        .map_err(|e| lost(Some(ConfigStage::Saving), &e))?;
    report.saved = true;
    emit(cb, ConfigStage::Rebooting, 0.0, "waiting for the FC".into());
    conn.reconnect(REBOOT_TIMEOUT)
        .map_err(|e| lost(Some(ConfigStage::Rebooting), &e))?;
    emit(cb, ConfigStage::Reconnected, 1.0, "reading back".into());

    match read_back(conn, &checks) {
        Ok(w) => warnings.extend(w),
        Err(e) => warnings.push(format!(
            "The restored settings could not be read back: {}",
            mask_secrets(&e.to_string())
        )),
    }
    report.warnings = warnings.iter().map(|w| mask_secrets(w)).collect();
    Ok(report)
}

/// `dump all` after the reboot: every applied `set` / `feature` must read back as sent.
fn read_back(conn: &mut dyn FcLink, checks: &[Check]) -> AppResult<Vec<String>> {
    let (_, _) = read_version(conn)?;
    let dump = conn.cli_exec("dump all")?;
    let sections = Sections::parse(&dump);
    let snap = parse_dump(&dump);
    let mut out = Vec::new();
    for c in checks {
        match c {
            Check::Set { name, value, ctx } => match sections.get(name, ctx) {
                Some(v) if v.eq_ignore_ascii_case(value) => {}
                Some(v) => out.push(format!("Read back: {name} is {v}, restored {value}")),
                None => out.push(format!("Read back: {name} is missing")),
            },
            Check::Feature { name, on } => {
                if let Some(v) = snap.features.get(name) {
                    if v != on {
                        out.push(format!(
                            "Read back: feature {name} is {}, restored {}",
                            if *v { "on" } else { "off" },
                            if *on { "on" } else { "off" }
                        ));
                    }
                }
            }
            Check::None => {}
        }
    }
    // A secret's value never appears: compare above, report masked.
    Ok(out
        .into_iter()
        .map(|w| {
            if SECRET_SETTINGS.iter().any(|s| w.contains(s)) {
                w.split(" is ").next().unwrap_or("").to_string() + " differs from the backup"
            } else {
                w
            }
        })
        .collect())
}
