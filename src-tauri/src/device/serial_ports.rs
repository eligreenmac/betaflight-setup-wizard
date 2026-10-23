// SPDX-License-Identifier: GPL-3.0-or-later
//! The board's serial port list, read without assumptions about which ports exist.
//!
//! Sources, in order of preference:
//! 1. `MSP2_COMMON_SERIAL_CONFIG` (0x1009): what Betaflight Configurator uses
//!    (`MSPHelper.ts`, `loadSerialConfig`). Layout: `count:u8`, then per port
//!    `identifier:u8 functionMask:u32 msp:u8 gps:u8 telemetry:u8 blackbox:u8` (baud *indices*);
//!    per-port size is `remaining / count` so newer firmware can append fields we skip.
//! 2. `MSP_CF_SERIAL_CONFIG` (54): legacy v1, 7 bytes per port with a u16 function mask.
//! 3. CLI `serial` output (or the `serial` lines of a `diff`/`dump`), as a fallback.
//!
//! Identifier schemes (firmware `src/main/io/serial.h`, `serialPortIdentifier_e`):
//! * 4.4 / 4.5 ("legacy"): UART1..UART10 = 0..9, VCP = 20, SOFT1/2 = 30/31, LPUART1 = 40.
//!   CLI prints the number: `serial 0 64 115200 57600 0 115200`.
//! * 2025.12 ("uart-base-50"): VCP/SOFT/LPUART unchanged, UART0 = 50, UART1..UART10 = 51..60,
//!   PIOUART0..9 = 70..79. CLI prints the *name*: `serial UART1 64 115200 57600 0 115200`,
//!   and accepts a legacy number 0..19 by adding 51.
//!
//! The identifier is reported exactly as the board sent it and is the only key a write may use.
//! `name` is filled only when the board itself printed it (CLI on 2025.12). `label` is a
//! display hint derived from the firmware enum for the detected scheme; never write it back.
//! No port is ever assumed to exist: the list is whatever the board returned.

use crate::device::error::{DeviceError, Result};
use crate::device::msp::{codes, ApiVersion, Reader};
use serde::{Deserialize, Serialize};

/// `baudRates[]` / configurator `BAUD_RATES`; index 0 is AUTO (printed as 0 by the CLI).
pub const BAUD_RATES: [u32; 16] = [
    0, 9600, 19200, 38400, 57600, 115200, 230400, 250000, 400000, 460800, 500000, 921600, 1000000,
    1500000, 2000000, 2470000,
];

/// `serialPortFunction_e` bit names (configurator `SERIAL_PORT_FUNCTIONS`). Bit 8 is unused.
pub const SERIAL_FUNCTIONS: &[(u8, &str)] = &[
    (0, "MSP"),
    (1, "GPS"),
    (2, "TELEMETRY_FRSKY"),
    (3, "TELEMETRY_HOTT"),
    (4, "TELEMETRY_LTM"),
    (5, "TELEMETRY_SMARTPORT"),
    (6, "RX_SERIAL"),
    (7, "BLACKBOX"),
    (9, "TELEMETRY_MAVLINK"),
    (10, "ESC_SENSOR"),
    (11, "TBS_SMARTAUDIO"),
    (12, "TELEMETRY_IBUS"),
    (13, "IRC_TRAMP"),
    (14, "RUNCAM_DEVICE_CONTROL"),
    (15, "LIDAR_TF"),
    (16, "FRSKY_OSD"),
    (17, "VTX_MSP"),
    (18, "GIMBAL"),
    (19, "OSD_CUSTOM_TEXT"),
];

pub const FUNCTION_MSP: u32 = 1 << 0;
pub const FUNCTION_RX_SERIAL: u32 = 1 << 6;

/// Where the port list came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SerialConfigSource {
    Msp2,
    Msp1,
    Cli,
}

/// Which `serialPortIdentifier_e` numbering the board uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IdentifierScheme {
    /// 4.4 / 4.5: UART1 = 0.
    Legacy,
    /// 2025.12+: UART0 = 50, UART1 = 51, PIOUART0 = 70.
    #[serde(rename = "uart-base-50")]
    UartBase50,
    /// Only scheme-neutral ports (VCP 20, SOFT 30/31, LPUART 40) seen and no version hint,
    /// or both UART blocks present (should not happen).
    Unknown,
}

/// One baud setting: the raw index (MSP) and the rate (0 = AUTO). `None` = not representable
/// (index out of table from MSP, or a CLI rate not in the table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Baud {
    pub index: Option<u8>,
    pub rate: Option<u32>,
}

impl Baud {
    pub fn from_index(i: u8) -> Self {
        Baud {
            index: Some(i),
            rate: BAUD_RATES.get(i as usize).copied(),
        }
    }
    pub fn from_rate(rate: u32) -> Self {
        Baud {
            index: BAUD_RATES.iter().position(|&r| r == rate).map(|i| i as u8),
            rate: Some(rate),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialPortConfig {
    /// Raw `serialPortIdentifier_e` as the board sent it. `None` only for CLI output from
    /// firmware that prints names (2025.12), where the number is not shown.
    pub identifier: Option<u8>,
    /// Port name exactly as the board printed it (CLI, 2025.12+). Never derived.
    pub name: Option<String>,
    /// Display hint derived from the identifier and detected scheme ("UART1", "VCP"), or `None`
    /// if unknown. Not authoritative; never send it to the board.
    pub label: Option<String>,
    pub function_mask: u32,
    /// Known function names for set bits.
    pub functions: Vec<String>,
    /// Set bits we have no name for (newer firmware); kept so a round trip loses nothing.
    pub unknown_function_bits: u32,
    pub msp_baud: Baud,
    pub gps_baud: Baud,
    pub telemetry_baud: Baud,
    pub blackbox_baud: Baud,
}

impl SerialPortConfig {
    fn new(identifier: Option<u8>, name: Option<String>, mask: u32, bauds: [Baud; 4]) -> Self {
        let (functions, unknown) = decode_function_mask(mask);
        SerialPortConfig {
            identifier,
            name,
            label: None,
            function_mask: mask,
            functions,
            unknown_function_bits: unknown,
            msp_baud: bauds[0],
            gps_baud: bauds[1],
            telemetry_baud: bauds[2],
            blackbox_baud: bauds[3],
        }
    }

    pub fn has_function(&self, bit_mask: u32) -> bool {
        self.function_mask & bit_mask != 0
    }

    /// VCP is 20 in both schemes; by name on CLI-only data.
    pub fn is_vcp(&self) -> bool {
        self.identifier == Some(20) || self.name.as_deref() == Some("VCP")
    }
}

/// The board's serial configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialConfig {
    pub source: SerialConfigSource,
    pub scheme: IdentifierScheme,
    pub ports: Vec<SerialPortConfig>,
}

impl SerialConfig {
    fn finish(
        source: SerialConfigSource,
        mut ports: Vec<SerialPortConfig>,
        api: Option<ApiVersion>,
    ) -> Self {
        let scheme = if source == SerialConfigSource::Cli {
            scheme_from_cli(&ports, api)
        } else {
            detect_scheme(ports.iter().filter_map(|p| p.identifier), api)
        };
        for p in &mut ports {
            p.label = match (&p.name, p.identifier) {
                (Some(n), _) => Some(n.clone()),
                (None, Some(id)) => label_for(id, scheme).map(str::to_string),
                _ => None,
            };
        }
        SerialConfig {
            source,
            scheme,
            ports,
        }
    }

    /// Port carrying serial RX, if any.
    pub fn rx_serial_port(&self) -> Option<&SerialPortConfig> {
        self.ports
            .iter()
            .find(|p| p.has_function(FUNCTION_RX_SERIAL))
    }
}

pub fn decode_function_mask(mask: u32) -> (Vec<String>, u32) {
    let mut known = 0u32;
    let mut names = Vec::new();
    for &(bit, name) in SERIAL_FUNCTIONS {
        if mask & (1 << bit) != 0 {
            names.push(name.to_string());
            known |= 1 << bit;
        }
    }
    (names, mask & !known)
}

/// Scheme from the identifiers themselves; `api` breaks the tie when only scheme-neutral ports
/// are present (API 1.47 = Betaflight 2025.12; see DESIGN.md open questions).
pub fn detect_scheme(
    ids: impl IntoIterator<Item = u8>,
    api: Option<ApiVersion>,
) -> IdentifierScheme {
    let (mut legacy, mut modern) = (false, false);
    for id in ids {
        match id {
            0..=19 => legacy = true,
            50..=79 => modern = true,
            _ => {}
        }
    }
    match (legacy, modern) {
        (true, false) => IdentifierScheme::Legacy,
        (false, true) => IdentifierScheme::UartBase50,
        (true, true) => IdentifierScheme::Unknown,
        (false, false) => match api {
            Some(a) if a.at_least(1, 47) => IdentifierScheme::UartBase50,
            Some(_) => IdentifierScheme::Legacy,
            None => IdentifierScheme::Unknown,
        },
    }
}

fn scheme_from_cli(ports: &[SerialPortConfig], api: Option<ApiVersion>) -> IdentifierScheme {
    // 2025.12 prints names, 4.x prints numbers. Mixed output should not happen.
    let named = ports.iter().any(|p| p.name.is_some());
    let numbered = ports.iter().any(|p| p.identifier.is_some());
    match (named, numbered) {
        (true, false) => IdentifierScheme::UartBase50,
        (false, true) => detect_scheme(ports.iter().filter_map(|p| p.identifier), api),
        _ => IdentifierScheme::Unknown,
    }
}

/// Display label for an identifier under a scheme (firmware enum, not a board fact).
pub fn label_for(id: u8, scheme: IdentifierScheme) -> Option<&'static str> {
    const UART: [&str; 16] = [
        "UART0", "UART1", "UART2", "UART3", "UART4", "UART5", "UART6", "UART7", "UART8", "UART9",
        "UART10", "UART11", "UART12", "UART13", "UART14", "UART15",
    ];
    const PIO: [&str; 10] = [
        "PIOUART0", "PIOUART1", "PIOUART2", "PIOUART3", "PIOUART4", "PIOUART5", "PIOUART6",
        "PIOUART7", "PIOUART8", "PIOUART9",
    ];
    match (id, scheme) {
        (20, _) => Some("VCP"),
        (30, _) => Some("SOFT1"),
        (31, _) => Some("SOFT2"),
        (40, _) => Some("LPUART1"),
        (0..=9, IdentifierScheme::Legacy) => Some(UART[id as usize + 1]),
        (50..=65, IdentifierScheme::UartBase50) => Some(UART[(id - 50) as usize]),
        (70..=79, IdentifierScheme::UartBase50) => Some(PIO[(id - 70) as usize]),
        _ => None,
    }
}

/// MSP2_COMMON_SERIAL_CONFIG reply.
pub fn parse_msp2_serial_config(p: &[u8]) -> Result<Vec<SerialPortConfig>> {
    let mut r = Reader::new(codes::MSP2_COMMON_SERIAL_CONFIG, p);
    let count = r.u8()? as usize;
    if count == 0 {
        return Ok(Vec::new());
    }
    let size = r.remaining() / count;
    if size < 9 {
        return Err(DeviceError::MspPayloadTooShort {
            code: codes::MSP2_COMMON_SERIAL_CONFIG,
            needed: 1 + 9 * count,
            got: p.len(),
        });
    }
    let mut ports = Vec::with_capacity(count);
    for _ in 0..count {
        let rec = r.bytes(size)?;
        let mut pr = Reader::new(codes::MSP2_COMMON_SERIAL_CONFIG, rec);
        let id = pr.u8()?;
        let mask = pr.u32()?;
        let bauds = [pr.u8()?, pr.u8()?, pr.u8()?, pr.u8()?].map(Baud::from_index);
        ports.push(SerialPortConfig::new(Some(id), None, mask, bauds));
    }
    Ok(ports)
}

/// MSP_CF_SERIAL_CONFIG reply (7 bytes per port).
pub fn parse_msp1_serial_config(p: &[u8]) -> Result<Vec<SerialPortConfig>> {
    if !p.len().is_multiple_of(7) {
        return Err(DeviceError::MspPayloadTooShort {
            code: codes::MSP_CF_SERIAL_CONFIG,
            needed: p.len().div_ceil(7) * 7,
            got: p.len(),
        });
    }
    let mut r = Reader::new(codes::MSP_CF_SERIAL_CONFIG, p);
    let mut ports = Vec::with_capacity(p.len() / 7);
    while r.remaining() > 0 {
        let id = r.u8()?;
        let mask = r.u16()? as u32;
        let bauds = [r.u8()?, r.u8()?, r.u8()?, r.u8()?].map(Baud::from_index);
        ports.push(SerialPortConfig::new(Some(id), None, mask, bauds));
    }
    Ok(ports)
}

/// `serial` lines from CLI output (`serial`, `diff`, `dump`). Comment lines and anything that
/// does not have the 6-field shape are ignored.
pub fn parse_cli_serial(text: &str) -> Vec<SerialPortConfig> {
    let mut ports = Vec::new();
    for line in text.lines() {
        let mut t = line.split_whitespace();
        if t.next() != Some("serial") {
            continue;
        }
        let f: Vec<&str> = t.collect();
        if f.len() != 6 {
            continue;
        }
        let (Ok(mask), Some(rates)) = (
            f[1].parse::<u32>(),
            f[2..6]
                .iter()
                .map(|s| s.parse::<u32>().ok())
                .collect::<Option<Vec<u32>>>(),
        ) else {
            continue;
        };
        let bauds = [rates[0], rates[1], rates[2], rates[3]].map(Baud::from_rate);
        let (id, name) = match f[0].parse::<u8>() {
            Ok(n) => (Some(n), None),
            Err(_) => (None, Some(f[0].to_string())),
        };
        ports.push(SerialPortConfig::new(id, name, mask, bauds));
    }
    ports
}

/// Build a [`SerialConfig`] from CLI text.
pub fn serial_config_from_cli(text: &str, api: Option<ApiVersion>) -> Option<SerialConfig> {
    let ports = parse_cli_serial(text);
    (!ports.is_empty()).then(|| SerialConfig::finish(SerialConfigSource::Cli, ports, api))
}

pub(crate) fn serial_config_from_msp(
    source: SerialConfigSource,
    ports: Vec<SerialPortConfig>,
    api: Option<ApiVersion>,
) -> SerialConfig {
    SerialConfig::finish(source, ports, api)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api(minor: u8) -> Option<ApiVersion> {
        Some(ApiVersion {
            protocol: 0,
            major: 1,
            minor,
        })
    }

    fn msp2(ports: &[(u8, u32, [u8; 4])], extra: usize) -> Vec<u8> {
        let mut p = vec![ports.len() as u8];
        for (id, mask, b) in ports {
            p.push(*id);
            p.extend(mask.to_le_bytes());
            p.extend(b);
            p.extend(std::iter::repeat_n(0xEE, extra));
        }
        p
    }

    /// Recorded shape of a 4.5 F405 reply: VCP(MSP), UART1(RX_SERIAL), UART3, UART6.
    fn sample_45() -> Vec<u8> {
        msp2(
            &[
                (20, 1, [5, 4, 0, 5]),
                (0, 64, [5, 4, 0, 5]),
                (2, 0, [5, 4, 0, 5]),
                (5, 0, [5, 4, 0, 5]),
            ],
            0,
        )
    }

    /// Same board on 2025.12: identifiers moved to 51+ (UART1 = 51 etc.).
    fn sample_2025_12() -> Vec<u8> {
        msp2(
            &[
                (20, 1, [5, 4, 0, 5]),
                (51, 64, [5, 4, 0, 5]),
                (53, 0, [5, 4, 0, 5]),
                (56, 1 << 17, [5, 4, 0, 5]),
            ],
            0,
        )
    }

    #[test]
    fn msp2_legacy_ids_kept_raw() {
        let ports = parse_msp2_serial_config(&sample_45()).unwrap();
        let cfg = serial_config_from_msp(SerialConfigSource::Msp2, ports, api(46));
        assert_eq!(cfg.scheme, IdentifierScheme::Legacy);
        let ids: Vec<_> = cfg.ports.iter().map(|p| p.identifier.unwrap()).collect();
        assert_eq!(ids, [20, 0, 2, 5]);
        assert_eq!(cfg.ports[1].label.as_deref(), Some("UART1"));
        assert_eq!(cfg.ports[1].functions, ["RX_SERIAL"]);
        assert_eq!(
            cfg.ports[1].msp_baud,
            Baud {
                index: Some(5),
                rate: Some(115200)
            }
        );
        assert_eq!(cfg.ports[1].telemetry_baud.rate, Some(0)); // AUTO
        assert!(cfg.ports[0].is_vcp());
        assert_eq!(cfg.rx_serial_port().unwrap().identifier, Some(0));
    }

    #[test]
    fn msp2_2025_12_ids_kept_raw() {
        let ports = parse_msp2_serial_config(&sample_2025_12()).unwrap();
        // Scheme comes from the ids, not the API version.
        let cfg = serial_config_from_msp(SerialConfigSource::Msp2, ports, None);
        assert_eq!(cfg.scheme, IdentifierScheme::UartBase50);
        assert_eq!(cfg.ports[1].identifier, Some(51));
        assert_eq!(cfg.ports[1].label.as_deref(), Some("UART1"));
        assert_eq!(cfg.ports[3].label.as_deref(), Some("UART6"));
        assert_eq!(cfg.ports[3].functions, ["VTX_MSP"]);
        assert!(cfg.ports.iter().all(|p| p.name.is_none()));
    }

    #[test]
    fn msp2_skips_appended_fields_and_keeps_unknown_bits() {
        let p = msp2(
            &[(20, 1 | (1 << 25), [5, 4, 0, 5]), (50, 0, [15, 99, 0, 0])],
            3,
        );
        let ports = parse_msp2_serial_config(&p).unwrap();
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0].unknown_function_bits, 1 << 25);
        assert_eq!(ports[1].identifier, Some(50));
        assert_eq!(ports[1].msp_baud.rate, Some(2_470_000));
        assert_eq!(
            ports[1].gps_baud,
            Baud {
                index: Some(99),
                rate: None
            }
        );
        let cfg = serial_config_from_msp(SerialConfigSource::Msp2, ports, None);
        assert_eq!(cfg.ports[1].label.as_deref(), Some("UART0"));
    }

    #[test]
    fn msp2_edge_cases() {
        assert!(parse_msp2_serial_config(&[0]).unwrap().is_empty());
        assert!(parse_msp2_serial_config(&[2, 1, 2, 3]).is_err());
        assert!(parse_msp2_serial_config(&[]).is_err());
    }

    #[test]
    fn msp1_layout() {
        let p = [20, 1, 0, 5, 4, 0, 5, 0, 64, 0, 5, 4, 0, 5];
        let ports = parse_msp1_serial_config(&p).unwrap();
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[1].identifier, Some(0));
        assert_eq!(ports[1].function_mask, 64);
        assert!(parse_msp1_serial_config(&p[..10]).is_err());
    }

    #[test]
    fn scheme_ambiguity_uses_api_hint() {
        assert_eq!(detect_scheme([20, 30], None), IdentifierScheme::Unknown);
        assert_eq!(
            detect_scheme([20, 30], api(47)),
            IdentifierScheme::UartBase50
        );
        assert_eq!(detect_scheme([20], api(45)), IdentifierScheme::Legacy);
        assert_eq!(detect_scheme([0, 51], api(47)), IdentifierScheme::Unknown);
        assert_eq!(label_for(5, IdentifierScheme::UartBase50), None);
        assert_eq!(label_for(51, IdentifierScheme::Legacy), None);
    }

    // CLI samples follow the firmware printf formats: 4.4.3/4.5.5 `"serial %d %d %ld %ld %ld %ld"`,
    // 2025.12.5 `"serial %s %d %ld %ld %ld %ld"` (name from serialPortNames[]).
    const CLI_45: &str = "serial 20 1 115200 57600 0 115200\r\nserial 0 64 115200 57600 0 115200\r\nserial 1 0 115200 57600 0 115200\r\nserial 5 2048 115200 57600 0 115200\r\n";
    const CLI_2025_12_DIFF: &str = "# diff\r\n# version\r\n# Betaflight / STM32H743 (SH74) 2025.12.5\r\n\r\n# serial\r\nserial UART1 64 115200 57600 0 115200\r\nserial UART2 1 115200 57600 0 115200\r\nserial SOFT1 0 115200 57600 0 115200\r\n\r\n# feature\r\nfeature -RX_PARALLEL_PWM\r\n";

    #[test]
    fn cli_legacy_numbers() {
        let cfg = serial_config_from_cli(CLI_45, api(46)).unwrap();
        assert_eq!(cfg.source, SerialConfigSource::Cli);
        assert_eq!(cfg.scheme, IdentifierScheme::Legacy);
        assert_eq!(cfg.ports.len(), 4);
        assert_eq!(cfg.ports[3].identifier, Some(5));
        assert_eq!(cfg.ports[3].functions, ["TBS_SMARTAUDIO"]);
        assert_eq!(
            cfg.ports[3].gps_baud,
            Baud {
                index: Some(4),
                rate: Some(57600)
            }
        );
        assert_eq!(cfg.ports[3].label.as_deref(), Some("UART6"));
    }

    #[test]
    fn cli_2025_12_names_verbatim() {
        let cfg = serial_config_from_cli(CLI_2025_12_DIFF, None).unwrap();
        assert_eq!(cfg.scheme, IdentifierScheme::UartBase50);
        let names: Vec<_> = cfg
            .ports
            .iter()
            .map(|p| p.name.as_deref().unwrap())
            .collect();
        assert_eq!(names, ["UART1", "UART2", "SOFT1"]);
        assert!(cfg.ports.iter().all(|p| p.identifier.is_none()));
        assert_eq!(cfg.ports[0].functions, ["RX_SERIAL"]);
        assert!(
            serial_config_from_cli("serial\r\nserial 1 2\r\n# serial 0 1 0 0 0 0", None).is_none()
        );
    }

    #[test]
    fn serialises_camel_case() {
        let ports = parse_msp2_serial_config(&sample_2025_12()).unwrap();
        let v = serde_json::to_value(serial_config_from_msp(
            SerialConfigSource::Msp2,
            ports,
            None,
        ))
        .unwrap();
        assert_eq!(v["source"], "msp2");
        assert_eq!(v["scheme"], "uart-base-50");
        assert_eq!(v["ports"][1]["identifier"], 51);
        assert_eq!(v["ports"][1]["mspBaud"]["rate"], 115200);
    }
}
