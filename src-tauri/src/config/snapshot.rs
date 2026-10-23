//! Parsers for CLI output: `serial`, `dump all` / `diff all` (also backup files).

use std::collections::BTreeMap;

use super::firmware::{parse_version, Dialect};
use super::serial_functions::{normalize_port_name, port_identifier, port_name};
use super::vtx_table::VtxTableState;
use crate::model::FcSerialPort;

/// One `serial` line, with the three baud fields kept verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortState {
    pub name: String,
    pub identifier: i32,
    pub function_mask: u32,
    /// msp, gps, telemetry, blackbox bauds as printed (e.g. "115200", "57600", "0", "115200").
    pub bauds: [String; 4],
}

impl PortState {
    pub const DEFAULT_BAUDS: [&'static str; 4] = ["115200", "57600", "0", "115200"];

    pub fn from_fc_port(p: &FcSerialPort) -> PortState {
        PortState {
            name: p.name.clone(),
            identifier: p.identifier,
            function_mask: p.function_mask,
            bauds: Self::DEFAULT_BAUDS.map(String::from),
        }
    }

    pub fn to_fc_port(&self) -> FcSerialPort {
        FcSerialPort {
            name: self.name.clone(),
            identifier: self.identifier,
            function_mask: self.function_mask,
            is_connection_port: self.name == "VCP",
        }
    }
}

/// An `aux` condition: `aux <slot> <permanentId> <auxIndex> <min> <max> <logic> <linkedTo>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuxSlot {
    pub slot: u8,
    pub mode_id: u8,
    pub aux_channel: u8,
    pub range_min: u16,
    pub range_max: u16,
    pub logic: u8,
    pub linked_to: u8,
}

impl AuxSlot {
    pub const COUNT: u8 = 20; // MAX_MODE_ACTIVATION_CONDITION_COUNT

    /// Firmware treats a condition as unused when its range is empty (isRangeUsable).
    pub fn is_used(&self) -> bool {
        self.range_min < self.range_max
    }

    pub fn empty(slot: u8) -> AuxSlot {
        AuxSlot {
            slot,
            mode_id: 0,
            aux_channel: 0,
            range_min: 900,
            range_max: 900,
            logic: 0,
            linked_to: 0,
        }
    }

    pub fn to_cli(self) -> String {
        format!(
            "aux {} {} {} {} {} {} {}",
            self.slot,
            self.mode_id,
            self.aux_channel,
            self.range_min,
            self.range_max,
            self.logic,
            self.linked_to
        )
    }
}

/// FC state as read from `dump all` (or `diff all`).
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub version: Option<String>,
    pub board_name: Option<String>,
    pub ports: Vec<PortState>,
    /// feature name -> enabled
    pub features: BTreeMap<String, bool>,
    /// setting name (lowercase) -> value as printed
    pub settings: BTreeMap<String, String>,
    pub aux: Vec<AuxSlot>,
    /// "RX_SPI_CS 1" -> "A15" / "NONE"
    pub resources: BTreeMap<String, String>,
    /// The `vtxtable` block; None when the dump has none (VTX table compiled out).
    pub vtxtable: Option<VtxTableState>,
    /// `beeper [-]NAME` lines in dump order (name, enabled); None = the dump has none
    /// (no beeper command in this build, e.g. SITL).
    pub beepers: Option<Vec<(String, bool)>>,
    /// `beacon [-]NAME` lines (DShot beacon); None = none in the dump (no USE_DSHOT).
    pub beacons: Option<Vec<(String, bool)>>,
}

impl Snapshot {
    pub fn dialect(&self) -> Option<Dialect> {
        self.version
            .as_deref()
            .and_then(|v| Dialect::from_version(v).ok())
    }

    pub fn resource_assigned(&self, name: &str) -> bool {
        self.resources.iter().any(|(k, v)| {
            k.split_whitespace().next() == Some(name) && !v.eq_ignore_ascii_case("NONE")
        })
    }

    pub fn setting(&self, name: &str) -> Option<&str> {
        self.settings.get(name).map(String::as_str)
    }

    pub fn has_setting(&self, name: &str) -> bool {
        self.settings.contains_key(name)
    }
}

/// Parse one `serial` line. Accepts numeric (4.4/4.5, legacy) or named (2025.12+) ports.
pub fn parse_serial_line(line: &str, dialect: Dialect) -> Option<PortState> {
    let mut t = line.split_whitespace();
    if t.next()? != "serial" {
        return None;
    }
    let id_tok = t.next()?;
    let (name, identifier) = match id_tok.parse::<i32>() {
        Ok(id) => (port_name(id, dialect)?, id),
        Err(_) => {
            let name = normalize_port_name(id_tok);
            let id = port_identifier(&name, dialect)?;
            (name, id)
        }
    };
    let function_mask = t.next()?.parse().ok()?;
    let rest: Vec<String> = t.take(4).map(String::from).collect();
    let bauds = if rest.len() == 4 {
        [
            rest[0].clone(),
            rest[1].clone(),
            rest[2].clone(),
            rest[3].clone(),
        ]
    } else {
        PortState::DEFAULT_BAUDS.map(String::from)
    };
    Some(PortState {
        name,
        identifier,
        function_mask,
        bauds,
    })
}

/// Parse the output of the `serial` command.
pub fn parse_serial(text: &str, dialect: Dialect) -> Vec<PortState> {
    text.lines()
        .filter_map(|l| parse_serial_line(l.trim(), dialect))
        .collect()
}

/// Parse `dump all` / `diff all` / a backup file.
pub fn parse_dump(text: &str) -> Snapshot {
    let mut s = Snapshot::default();
    if let Some(v) = parse_version(text) {
        s.version = Some(v.version);
        s.board_name = v.board_name;
    }
    let dialect = s.dialect().unwrap_or(Dialect::V4_5);
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut t = line.split_whitespace();
        match t.next() {
            Some("board_name") => {
                if let Some(b) = t.next() {
                    s.board_name = Some(b.to_string());
                }
            }
            Some("serial") => {
                if let Some(p) = parse_serial_line(line, dialect) {
                    s.ports.retain(|q| q.name != p.name);
                    s.ports.push(p);
                }
            }
            Some("feature") => {
                if let Some(f) = t.next() {
                    match f.strip_prefix('-') {
                        Some(n) => s.features.insert(n.to_string(), false),
                        None => s.features.insert(f.to_string(), true),
                    };
                }
            }
            Some("set") => {
                if let Some((k, v)) = line[3..].split_once('=') {
                    s.settings
                        .insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
                }
            }
            Some("aux") => {
                let n: Vec<u16> = t.filter_map(|x| x.parse().ok()).collect();
                if n.len() >= 5 {
                    let slot = AuxSlot {
                        slot: n[0] as u8,
                        mode_id: n[1] as u8,
                        aux_channel: n[2] as u8,
                        range_min: n[3],
                        range_max: n[4],
                        logic: n.get(5).copied().unwrap_or(0) as u8,
                        linked_to: n.get(6).copied().unwrap_or(0) as u8,
                    };
                    s.aux.retain(|a| a.slot != slot.slot);
                    s.aux.push(slot);
                }
            }
            Some("vtxtable") => {
                let rest = line["vtxtable".len()..].trim();
                let _ = s.vtxtable.get_or_insert_with(Default::default).apply(rest);
            }
            Some(verb @ ("beeper" | "beacon")) => {
                if let Some(f) = t.next() {
                    let list = if verb == "beeper" {
                        &mut s.beepers
                    } else {
                        &mut s.beacons
                    };
                    let (name, on) = match f.strip_prefix('-') {
                        Some(n) => (n, false),
                        None => (f, true),
                    };
                    let list = list.get_or_insert_with(Vec::new);
                    list.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
                    list.push((name.to_ascii_uppercase(), on));
                }
            }
            Some("resource") => {
                let parts: Vec<&str> = t.collect();
                if parts.len() >= 3 {
                    s.resources
                        .insert(format!("{} {}", parts[0], parts[1]), parts[2].to_string());
                }
            }
            _ => {}
        }
    }
    s.aux.sort_by_key(|a| a.slot);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_numeric_and_named() {
        let p = parse_serial_line("serial 1 64 115200 57600 0 115200", Dialect::V4_5).unwrap();
        assert_eq!(
            (p.name.as_str(), p.identifier, p.function_mask),
            ("UART2", 1, 64)
        );
        let p =
            parse_serial_line("serial UART2 64 115200 57600 0 115200", Dialect::V2025_12).unwrap();
        assert_eq!((p.name.as_str(), p.identifier), ("UART2", 52));
        let p =
            parse_serial_line("serial USART3 2 115200 57600 0 115200", Dialect::V2025_12).unwrap();
        assert_eq!(p.name, "UART3");
        let p = parse_serial_line("serial 20 1 115200 57600 0 115200", Dialect::V2025_12).unwrap();
        assert_eq!(p.name, "VCP");
    }

    #[test]
    fn dump_parsing() {
        let t = "# Betaflight / STM32F411 (S411) 4.5.2 Jun 1 2025 / 00:00:00 (x) MSP API: 1.46\nboard_name CRAZYBEEF4SX1280\nresource RX_SPI_CS 1 A15\nresource RX_SPI_BIND 1 NONE\nfeature -RX_SPI\nfeature RX_SERIAL\nset expresslrs_uid = 0,0,0,0,0,0\naux 0 0 0 1700 2100 0 0\naux 1 0 0 900 900 0 0\n";
        let s = parse_dump(t);
        assert_eq!(s.version.as_deref(), Some("4.5.2"));
        assert_eq!(s.board_name.as_deref(), Some("CRAZYBEEF4SX1280"));
        assert!(s.resource_assigned("RX_SPI_CS"));
        assert!(!s.resource_assigned("RX_SPI_BIND"));
        assert_eq!(s.features.get("RX_SPI"), Some(&false));
        assert_eq!(s.setting("expresslrs_uid"), Some("0,0,0,0,0,0"));
        assert!(s.aux[0].is_used() && !s.aux[1].is_used());
        assert!(s.beepers.is_none() && s.beacons.is_none());
        let s = parse_dump("beeper GYRO_CALIBRATED\nbeeper -RX_LOST\nbeacon -RX_SET\n");
        assert_eq!(
            s.beepers,
            Some(vec![
                ("GYRO_CALIBRATED".into(), true),
                ("RX_LOST".into(), false)
            ])
        );
        assert_eq!(s.beacons, Some(vec![("RX_SET".into(), false)]));
    }
}
