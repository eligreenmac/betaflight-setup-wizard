//! Known USB VID/PIDs (from betaflight-configurator src/js/protocols/devices.ts).

use crate::model::HostPortKind;

pub const DFU_DEVICES: &[(u16, u16, &str)] =
    &[(0x0483, 0xDF11, "STM32 DFU"), (0x2E3C, 0xDF11, "AT32 DFU")];

pub const SERIAL_DEVICES: &[(u16, u16, HostPortKind, &str)] = &[
    (0x0483, 0x5740, HostPortKind::FcVcp, "STM32 VCP"),
    (0x2E3C, 0x5740, HostPortKind::FcVcp, "AT32 VCP"),
    (0x314B, 0x5740, HostPortKind::FcVcp, "APM32 VCP"),
    (0x28E9, 0x018A, HostPortKind::FcVcp, "GD32 VCP"),
    (0x2E8A, 0x0009, HostPortKind::FcVcp, "RP2040/RP2350 VCP"),
    (0x10C4, 0xEA60, HostPortKind::UsbUartBridge, "CP210x"),
    (0x1A86, 0x7522, HostPortKind::UsbUartBridge, "CH340"),
    (0x1A86, 0x7523, HostPortKind::UsbUartBridge, "CH340"),
    (0x0403, 0x6001, HostPortKind::UsbUartBridge, "FT232R"),
];

pub fn classify(vid: u16, pid: u16) -> HostPortKind {
    SERIAL_DEVICES
        .iter()
        .find(|(v, p, _, _)| *v == vid && *p == pid)
        .map(|(_, _, k, _)| *k)
        .unwrap_or(HostPortKind::Unknown)
}

pub fn is_dfu(vid: u16, pid: u16) -> bool {
    DFU_DEVICES.iter().any(|(v, p, _)| *v == vid && *p == pid)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classifies_stm32_vcp() {
        assert_eq!(classify(0x0483, 0x5740), HostPortKind::FcVcp);
        assert!(is_dfu(0x0483, 0xDF11));
        assert_eq!(classify(0xFFFF, 0x0001), HostPortKind::Unknown);
    }
}
