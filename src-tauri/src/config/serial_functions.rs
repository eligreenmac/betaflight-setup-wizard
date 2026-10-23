//! Serial port function bitmask, from betaflight src/main/io/serial.h
//! (stable across 4.4 .. 2026.6; 2026.6 adds LIDAR_NL and OSD_CUSTOM_TEXT, which the
//! wizard never manages: they are kept like every other unmanaged bit).

pub const MSP: u32 = 1 << 0;
pub const GPS: u32 = 1 << 1;
pub const TELEMETRY_FRSKY_HUB: u32 = 1 << 2;
pub const TELEMETRY_HOTT: u32 = 1 << 3;
pub const TELEMETRY_LTM: u32 = 1 << 4;
pub const TELEMETRY_SMARTPORT: u32 = 1 << 5;
pub const RX_SERIAL: u32 = 1 << 6;
pub const BLACKBOX: u32 = 1 << 7;
pub const TELEMETRY_MAVLINK: u32 = 1 << 9;
pub const ESC_SENSOR: u32 = 1 << 10;
pub const VTX_SMARTAUDIO: u32 = 1 << 11;
pub const TELEMETRY_IBUS: u32 = 1 << 12;
pub const VTX_TRAMP: u32 = 1 << 13;
pub const RCDEVICE: u32 = 1 << 14;
pub const LIDAR: u32 = 1 << 15;
pub const FRSKY_OSD: u32 = 1 << 16;
pub const VTX_MSP: u32 = 1 << 17;
pub const GIMBAL: u32 = 1 << 18;
/// 2026.6+ (io/serial.h FUNCTION_LIDAR_NL)
pub const LIDAR_NL: u32 = 1 << 19;
/// 2026.6+ (io/serial.h FUNCTION_OSD_CUSTOM_TEXT)
pub const OSD_CUSTOM_TEXT: u32 = 1 << 20;

/// Functions the wizard manages. Everything else on a port is never touched.
pub const MANAGED: u32 = RX_SERIAL | GPS | ESC_SENSOR | VTX_SMARTAUDIO | VTX_TRAMP | VTX_MSP;

/// betaflight io/serial.h TELEMETRY_SHAREABLE_PORT_FUNCTIONS_MASK
pub const TELEMETRY_SHAREABLE: u32 = TELEMETRY_FRSKY_HUB | TELEMETRY_LTM | TELEMETRY_MAVLINK;
/// TELEMETRY_PORT_FUNCTIONS_MASK
pub const TELEMETRY_PORT_FUNCTIONS: u32 =
    TELEMETRY_SHAREABLE | TELEMETRY_HOTT | TELEMETRY_SMARTPORT;
/// io/serial.c ALL_FUNCTIONS_SHARABLE_WITH_MSP (USE_TELEMETRY builds)
pub const SHARABLE_WITH_MSP: u32 = BLACKBOX | TELEMETRY_PORT_FUNCTIONS | VTX_MSP;
/// msp/msp_serial.h
pub const MAX_MSP_PORT_COUNT: usize = 3;

use super::firmware::Dialect;
use crate::model::{RxProtocol, SerialFunction};

pub fn function_bit(f: SerialFunction) -> u32 {
    match f {
        SerialFunction::RxSerial => RX_SERIAL,
        SerialFunction::Gps => GPS,
        SerialFunction::EscSensor => ESC_SENSOR,
        SerialFunction::VtxSmartaudio => VTX_SMARTAUDIO,
        SerialFunction::VtxTramp => VTX_TRAMP,
        SerialFunction::VtxMsp => VTX_MSP,
        SerialFunction::Msp => MSP,
    }
}

/// Managed functions present in `mask`, in a stable order (MSP last).
pub fn functions_in(mask: u32) -> Vec<SerialFunction> {
    [
        SerialFunction::RxSerial,
        SerialFunction::Gps,
        SerialFunction::EscSensor,
        SerialFunction::VtxSmartaudio,
        SerialFunction::VtxTramp,
        SerialFunction::VtxMsp,
        SerialFunction::Msp,
    ]
    .into_iter()
    .filter(|f| mask & function_bit(*f) != 0)
    .collect()
}

/// `serial` identifier for a port name in the given dialect.
/// 4.4/4.5: UART1=0..UART10=9; 2025.12+: UART1=51.. (PR #14094; 2026.6 adds
/// UART11..15 = 61..65). VCP=20, SOFTSERIAL1=30, LPUART1=40 in all.
pub fn port_identifier(name: &str, dialect: Dialect) -> Option<i32> {
    match name {
        "VCP" => return Some(20),
        "SOFTSERIAL1" => return Some(30),
        "SOFTSERIAL2" => return Some(31),
        "LPUART1" => return Some(40),
        _ => {}
    }
    let n: i32 = name.strip_prefix("UART")?.parse().ok()?;
    if !(1..=15).contains(&n) {
        return None;
    }
    Some(if dialect.uses_port_names() {
        50 + n
    } else {
        n - 1
    })
}

/// Inverse of [`port_identifier`]. 2025.12+ also accepts legacy 0..=19 as UART1..
pub fn port_name(identifier: i32, dialect: Dialect) -> Option<String> {
    match identifier {
        20 => Some("VCP".into()),
        30 => Some("SOFTSERIAL1".into()),
        31 => Some("SOFTSERIAL2".into()),
        40 => Some("LPUART1".into()),
        0..=14 if !dialect.uses_port_names() => Some(format!("UART{}", identifier + 1)),
        0..=19 => Some(format!("UART{}", identifier + 1)),
        51..=65 if dialect.uses_port_names() => Some(format!("UART{}", identifier - 50)),
        _ => None,
    }
}

/// Normalize names printed by 2025.12 (`USART1`, `UART1`) to `UART1`.
pub fn normalize_port_name(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("USART") {
        format!("UART{rest}")
    } else {
        s.to_string()
    }
}

/// Mirror of betaflight io/serial.c `isSerialConfigValid` (identical rules in
/// 4.4.3, 4.5.5, 2025.12.5 and 2026.6.2 apart from the soft-serial sanitising, and soft
/// serial is out of MVP scope). If the firmware finds the config invalid at boot
/// it resets the WHOLE serial config to defaults, so the planner must refuse first.
/// Returns a human-readable reason on failure.
pub fn check_serial_config(
    ports: &[(String, u32)],
    serialrx: Option<RxProtocol>,
) -> Result<(), String> {
    let mut msp_ports = 0usize;
    for (name, mask) in ports {
        let mask = *mask;
        if mask & MSP != 0 {
            msp_ports += 1;
        }
        if name == "VCP" && mask & MSP == 0 {
            return Err("the USB VCP port must keep MSP".into());
        }
        let bits = mask.count_ones();
        if mask & VTX_MSP != 0 && bits == 1 {
            return Err(format!("{name}: VTX (MSP) must be shared with MSP"));
        }
        if bits > 1 {
            if bits > (MSP | SHARABLE_WITH_MSP).count_ones() {
                return Err(format!("{name}: too many functions on one port"));
            }
            let msp_share = mask & MSP != 0 && mask & SHARABLE_WITH_MSP != 0;
            let rx_tlm_share = mask & RX_SERIAL != 0
                && mask & TELEMETRY_SHAREABLE != 0
                && matches!(
                    serialrx,
                    Some(RxProtocol::Spek1024)
                        | Some(RxProtocol::Spek2048)
                        | Some(RxProtocol::Sbus)
                        | Some(RxProtocol::Sumd)
                        | Some(RxProtocol::Ibus)
                );
            let ibus_share =
                mask == RX_SERIAL | TELEMETRY_IBUS && matches!(serialrx, Some(RxProtocol::Ibus));
            if !(msp_share || rx_tlm_share || ibus_share) {
                return Err(format!(
                    "{name}: functions {:#x} cannot share one port (firmware would reset all ports to defaults)",
                    mask
                ));
            }
        }
    }
    if msp_ports == 0 || msp_ports > MAX_MSP_PORT_COUNT {
        return Err(format!(
            "{msp_ports} MSP ports; Betaflight requires 1..={MAX_MSP_PORT_COUNT}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_per_dialect() {
        assert_eq!(port_identifier("UART1", Dialect::V4_5), Some(0));
        assert_eq!(port_identifier("UART6", Dialect::V4_4), Some(5));
        assert_eq!(port_identifier("UART1", Dialect::V2025_12), Some(51));
        assert_eq!(port_identifier("VCP", Dialect::V2025_12), Some(20));
        assert_eq!(port_name(5, Dialect::V4_5).as_deref(), Some("UART6"));
        assert_eq!(port_name(56, Dialect::V2025_12).as_deref(), Some("UART6"));
        assert_eq!(port_name(0, Dialect::V2025_12).as_deref(), Some("UART1"));
    }

    #[test]
    fn validity_rules() {
        let vcp = ("VCP".to_string(), MSP);
        let ok = vec![
            vcp.clone(),
            ("UART1".into(), MSP | VTX_MSP),
            ("UART2".into(), RX_SERIAL),
        ];
        assert!(check_serial_config(&ok, Some(RxProtocol::Crsf)).is_ok());
        let lone_vtx_msp = vec![vcp.clone(), ("UART1".into(), VTX_MSP)];
        assert!(check_serial_config(&lone_vtx_msp, None).is_err());
        let rx_gps = vec![vcp.clone(), ("UART1".into(), RX_SERIAL | GPS)];
        assert!(check_serial_config(&rx_gps, Some(RxProtocol::Crsf)).is_err());
        let sbus_ltm = vec![vcp.clone(), ("UART1".into(), RX_SERIAL | TELEMETRY_LTM)];
        assert!(check_serial_config(&sbus_ltm, Some(RxProtocol::Sbus)).is_ok());
        assert!(check_serial_config(&sbus_ltm, Some(RxProtocol::Crsf)).is_err());
        let no_msp_vcp = vec![("VCP".into(), 0)];
        assert!(check_serial_config(&no_msp_vcp, None).is_err());
        let four_msp = vec![
            vcp,
            ("UART1".into(), MSP),
            ("UART2".into(), MSP),
            ("UART3".into(), MSP),
        ];
        assert!(check_serial_config(&four_msp, None).is_err());
    }
}
