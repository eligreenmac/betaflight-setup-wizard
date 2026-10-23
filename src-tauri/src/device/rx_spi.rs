// SPDX-License-Identifier: GPL-3.0-or-later
//! Onboard SPI receiver (e.g. ExpressLRS SX1280 on AIO boards) detection.
//!
//! MSP cannot answer "does this board have an SPI receiver": Betaflight Configurator only reads
//! `rxSpiProtocol` from `MSP_RX_CONFIG` (byte 16) and the `RX_SPI` feature bit (25), and a build
//! without `USE_RX_SPI` still sends 0 for the protocol, which is also a valid protocol index
//! (`V202_250K`). There is no RX_SPI build-option id either. So:
//! * **Authoritative:** CLI `resource` (configured pins) — an assigned `RX_SPI_CS` means the
//!   board has an SPI receiver wired. `resource show` (pins claimed at boot) is accepted too.
//! * **Firmware support:** `get rx_spi_protocol` prints `rx_spi_protocol = <NAME>` only when the
//!   build has `USE_RX_SPI`; otherwise the CLI reports an invalid name.
//! * **Hints only (MSP):** feature `RX_SPI` enabled, `rxSpiProtocol` index.
//!
//! Formats from firmware 2025.12.5 `cli.c`: `resource %s %d %c%02d` / `resource %s %d NONE`,
//! `resource show` lines `%c%02d: %s[ %d]`, `get` prints `%s = <value>`.

use crate::device::error::Result;
use crate::device::msp::{codes, Reader};
use serde::{Deserialize, Serialize};

/// `FEATURE_RX_SPI` bit in MSP_FEATURE_CONFIG.
pub const FEATURE_RX_SPI_BIT: u32 = 25;
/// Index of EXPRESSLRS in `lookupTableRxSpi` (configurator ReceiverTab checks `=== 19`).
pub const RX_SPI_PROTOCOL_EXPRESSLRS: u8 = 19;

/// `lookupTableRxSpi` (firmware `cli/settings.c`, `rx_spi_protocol_e` order).
pub const RX_SPI_PROTOCOLS: &[&str] = &[
    "V202_250K",
    "V202_1M",
    "SYMA_X",
    "SYMA_X5C",
    "CX10",
    "CX10A",
    "H8_3D",
    "INAV",
    "FRSKY_D",
    "FRSKY_X",
    "FLYSKY",
    "FLYSKY_2A",
    "KN",
    "SFHSS",
    "SPEKTRUM",
    "FRSKY_X_LBT",
    "REDPINE",
    "FRSKY_X_V2",
    "FRSKY_X_LBT_V2",
    "EXPRESSLRS",
];

/// One `resource` assignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceAssignment {
    /// Owner name as printed, e.g. `RX_SPI_CS`, `RX_SPI_EXPRESSLRS_BUSY`.
    pub owner: String,
    /// 1-based index as printed (`resource` always prints one; `resource show` only when > 0).
    pub index: Option<u8>,
    /// Pin like `A04`, `None` when `NONE`.
    pub pin: Option<String>,
}

/// `resource` / `diff` / `dump` lines: `resource OWNER INDEX PIN|NONE`.
pub fn parse_resource_list(text: &str) -> Vec<ResourceAssignment> {
    text.lines()
        .filter_map(|line| {
            let t: Vec<&str> = line.split_whitespace().collect();
            if t.len() != 4 || t[0] != "resource" {
                return None;
            }
            let index = t[2].parse::<u8>().ok()?;
            let pin = (!t[3].eq_ignore_ascii_case("NONE")).then(|| t[3].to_ascii_uppercase());
            Some(ResourceAssignment {
                owner: t[1].to_string(),
                index: Some(index),
                pin,
            })
        })
        .collect()
}

/// `resource show` lines: `A04: RX_SPI_CS` or `B00: MOTOR 1`.
pub fn parse_resource_show(text: &str) -> Vec<ResourceAssignment> {
    text.lines()
        .filter_map(|line| {
            let (pin, rest) = line.trim().split_once(':')?;
            let b = pin.as_bytes();
            if b.len() != 3 || !b[0].is_ascii_uppercase() || !b[1..].iter().all(u8::is_ascii_digit)
            {
                return None;
            }
            let mut t = rest.split_whitespace();
            let owner = t.next()?.to_string();
            let index = match t.next() {
                Some(s) => Some(s.parse::<u8>().ok()?),
                None => None,
            };
            (owner != "FREE").then(|| ResourceAssignment {
                owner,
                index,
                pin: Some(pin.to_string()),
            })
        })
        .collect()
}

/// `get <name>` output: the value on the `name = value` line, `None` if the setting is absent
/// (CLI prints an "Invalid name" error instead).
pub fn parse_get_value(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == name).then(|| v.trim().to_string())
    })
}

/// `rxSpiProtocol` from an MSP_RX_CONFIG reply (byte 16, per MSPHelper.ts).
pub fn parse_rx_config_spi_protocol(p: &[u8]) -> Result<u8> {
    let mut r = Reader::new(codes::MSP_RX_CONFIG, p);
    r.bytes(16)?;
    r.u8()
}

/// What the wizard knows about an onboard SPI receiver. Serialised in the board info as `rxSpi`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RxSpiInfo {
    /// MVP field: board has an SPI RX wired (`RX_SPI_CS` assigned). `None` = not determined yet
    /// (CLI not probed).
    pub has_rx_spi: Option<bool>,
    /// Build includes `USE_RX_SPI` (`get rx_spi_protocol` answered). `None` = not asked.
    pub firmware_supports_rx_spi: Option<bool>,
    /// Assigned `RX_SPI_*` resources as printed.
    pub resources: Vec<ResourceAssignment>,
    /// ExpressLRS-specific pins (`RX_SPI_EXPRESSLRS_RESET` / `_BUSY`) are assigned.
    pub has_elrs_pins: bool,
    /// `rx_spi_protocol` from the CLI, verbatim (e.g. `EXPRESSLRS`).
    pub protocol: Option<String>,
    /// Hint: `rxSpiProtocol` index from MSP_RX_CONFIG (meaningless if the build lacks RX_SPI).
    pub msp_protocol_index: Option<u8>,
    /// Hint: `RX_SPI` feature enabled (MSP_FEATURE_CONFIG bit 25).
    pub feature_enabled: Option<bool>,
}

impl RxSpiInfo {
    /// MSP hints only; `has_rx_spi` stays undetermined.
    pub fn from_msp(features: Option<u32>, rx_spi_protocol: Option<u8>) -> Self {
        RxSpiInfo {
            feature_enabled: features.map(|f| f & (1 << FEATURE_RX_SPI_BIT) != 0),
            msp_protocol_index: rx_spi_protocol,
            ..Default::default()
        }
    }

    /// Fold in CLI output. `resource_text` may be `resource`, `resource show`, `diff all` or
    /// `dump` output; `get_text` is the output of `get rx_spi_protocol` (if run).
    pub fn apply_cli(&mut self, resource_text: &str, get_text: Option<&str>) {
        let mut list = parse_resource_list(resource_text);
        if list.is_empty() {
            list = parse_resource_show(resource_text);
        }
        self.resources = list
            .into_iter()
            .filter(|r| r.owner.starts_with("RX_SPI_") && r.pin.is_some())
            .collect();
        let assigned = |o: &str| self.resources.iter().any(|r| r.owner == o);
        self.has_rx_spi = Some(assigned("RX_SPI_CS"));
        self.has_elrs_pins =
            assigned("RX_SPI_EXPRESSLRS_RESET") && assigned("RX_SPI_EXPRESSLRS_BUSY");
        if let Some(g) = get_text {
            self.protocol = parse_get_value(g, "rx_spi_protocol");
            self.firmware_supports_rx_spi = Some(self.protocol.is_some());
        }
        if self.has_rx_spi == Some(true) {
            // Pins can only be assigned on a USE_RX_SPI build.
            self.firmware_supports_rx_spi = Some(true);
        }
    }

    /// Board has an SPI RX and it is (or can be set to) ExpressLRS.
    pub fn is_expresslrs(&self) -> Option<bool> {
        match (&self.protocol, self.has_rx_spi) {
            (Some(p), _) => Some(p == "EXPRESSLRS"),
            (None, Some(false)) => Some(false),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic samples built from the 2025.12.5 cli.c formats (not captured from hardware).
    /// `resource` on an AIO F411 with onboard SPI ELRS (HIDE_UNUSED: no NONE lines).
    const RESOURCE_SPI_ELRS: &str = "resource BEEPER 1 C15\r\nresource MOTOR 1 B10\r\nresource MOTOR 2 B06\r\n\
resource SERIAL_TX 1 A09\r\nresource SERIAL_RX 1 A10\r\nresource SPI_SCK 3 B03\r\nresource SPI_MISO 3 B04\r\n\
resource SPI_MOSI 3 B05\r\nresource RX_SPI_CS 1 A15\r\nresource RX_SPI_EXTI 1 C14\r\nresource RX_SPI_BIND 1 B02\r\n\
resource RX_SPI_LED 1 B09\r\nresource RX_SPI_EXPRESSLRS_RESET 1 A08\r\nresource RX_SPI_EXPRESSLRS_BUSY 1 A13\r\n";
    /// `resource` on a UART-receiver F405.
    const RESOURCE_UART_RX: &str =
        "resource BEEPER 1 B04\r\nresource MOTOR 1 C06\r\nresource SERIAL_TX 1 A09\r\n\
resource SERIAL_RX 1 A10\r\nresource RX_SPI_CS 1 NONE\r\n";
    const RESOURCE_SHOW: &str = "Currently active IO resource assignments:\r\n(reboot to update)\r\n--------------------\r\n\
A08: RX_SPI_EXPRESSLRS_RESET\r\nA13: RX_SPI_EXPRESSLRS_BUSY\r\nA15: RX_SPI_CS\r\nB06: MOTOR 2\r\nB10: MOTOR 1\r\nC13: FREE\r\nC14: RX_SPI_EXTI\r\n";
    const GET_ELRS: &str = "rx_spi_protocol = EXPRESSLRS\r\nAllowed values: V202_250K, V202_1M, SYMA_X, SYMA_X5C, CX10, CX10A, H8_3D, INAV, FRSKY_D, FRSKY_X, FLYSKY, FLYSKY_2A, KN, SFHSS, SPEKTRUM, FRSKY_X_LBT, REDPINE, FRSKY_X_V2, FRSKY_X_LBT_V2, EXPRESSLRS\r\nDefault value: EXPRESSLRS\r\n";
    const GET_MISSING: &str = "###ERROR IN get: INVALID NAME: rx_spi_protocol###\r\n";

    #[test]
    fn spi_elrs_board() {
        let mut info = RxSpiInfo::from_msp(
            Some(1 << FEATURE_RX_SPI_BIT),
            Some(RX_SPI_PROTOCOL_EXPRESSLRS),
        );
        info.apply_cli(RESOURCE_SPI_ELRS, Some(GET_ELRS));
        assert_eq!(info.has_rx_spi, Some(true));
        assert!(info.has_elrs_pins);
        assert_eq!(info.resources.len(), 6);
        assert_eq!(
            info.resources[0],
            ResourceAssignment {
                owner: "RX_SPI_CS".into(),
                index: Some(1),
                pin: Some("A15".into())
            }
        );
        assert_eq!(info.protocol.as_deref(), Some("EXPRESSLRS"));
        assert_eq!(info.firmware_supports_rx_spi, Some(true));
        assert_eq!(info.feature_enabled, Some(true));
        assert_eq!(info.is_expresslrs(), Some(true));
    }

    #[test]
    fn uart_rx_board_and_missing_setting() {
        let mut info = RxSpiInfo::default();
        info.apply_cli(RESOURCE_UART_RX, Some(GET_MISSING));
        assert_eq!(info.has_rx_spi, Some(false));
        assert!(info.resources.is_empty());
        assert_eq!(info.firmware_supports_rx_spi, Some(false));
        assert_eq!(info.is_expresslrs(), Some(false));
    }

    #[test]
    fn resource_show_fallback() {
        let mut info = RxSpiInfo::default();
        info.apply_cli(RESOURCE_SHOW, None);
        assert_eq!(info.has_rx_spi, Some(true));
        assert!(info.has_elrs_pins);
        assert_eq!(info.firmware_supports_rx_spi, Some(true));
        assert_eq!(info.protocol, None);
        let show = parse_resource_show(RESOURCE_SHOW);
        assert_eq!(
            show.iter().find(|r| r.owner == "MOTOR").unwrap().index,
            Some(2)
        );
        assert!(show.iter().all(|r| r.owner != "FREE"));
    }

    #[test]
    fn msp_hints_do_not_decide() {
        let info = RxSpiInfo::from_msp(Some(0), Some(0));
        assert_eq!(info.has_rx_spi, None);
        assert_eq!(info.feature_enabled, Some(false));
        assert_eq!(info.is_expresslrs(), None);
        let mut rx = vec![0u8; 16];
        rx.extend([19, 0, 0, 0, 0, 0]);
        assert_eq!(parse_rx_config_spi_protocol(&rx).unwrap(), 19);
        assert!(parse_rx_config_spi_protocol(&rx[..16]).is_err());
        assert_eq!(
            RX_SPI_PROTOCOLS[RX_SPI_PROTOCOL_EXPRESSLRS as usize],
            "EXPRESSLRS"
        );
        assert_eq!(
            parse_get_value(
                "a = 1\r\nrx_spi_protocol =  FRSKY_X \r\n",
                "rx_spi_protocol"
            )
            .as_deref(),
            Some("FRSKY_X")
        );
    }
}
