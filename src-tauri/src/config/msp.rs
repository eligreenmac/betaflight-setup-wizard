//! MSP identification messages the config workstream relies on (C1 / #12): message ids,
//! encoders used by `MockFc`, and pure decoders into [`FcInfo`] fields.
//!
//! `device::` owns the transport (`Connection::msp_request`) and does not define MSP ids
//! yet; these are offered for reuse there. Layouts follow Betaflight `msp.c` and are
//! checked against real SITL replies in `tests/fixtures/sitl/<version>/msp.txt`
//! (4.4.3, 4.5.5, 2025.12.5).

use crate::model::FcInfo;

pub const MSP_API_VERSION: u16 = 1;
pub const MSP_FC_VARIANT: u16 = 2;
pub const MSP_FC_VERSION: u16 = 3;
pub const MSP_BOARD_INFO: u16 = 4;
pub const MSP_BUILD_INFO: u16 = 5;
pub const MSP_UID: u16 = 160;

/// MSP_BOARD_INFO, the part the wizard uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardInfo {
    /// 4-char board identifier, e.g. "S405" (SITL: "SITL").
    pub identifier: String,
    pub hardware_revision: u16,
    pub target_name: String,
    /// Empty on builds without a board config (SITL).
    pub board_name: String,
    pub manufacturer_id: String,
    pub mcu_type_id: Option<u8>,
}

/// `mcuTypeId_e` (4.4+), the ids the supported boards use.
pub fn mcu_name(id: u8) -> Option<&'static str> {
    Some(match id {
        0 => "SIMULATOR",
        1 => "STM32F405",
        2 => "STM32F411",
        3 => "STM32F446",
        4 => "STM32F722",
        5 => "STM32F745",
        6 => "STM32F746",
        7 => "STM32F765",
        _ => return None,
    })
}

pub fn mcu_type_id(mcu: &str) -> u8 {
    (0..=7).find(|i| mcu_name(*i) == Some(mcu)).unwrap_or(0xff)
}

pub fn decode_api_version(p: &[u8]) -> Option<String> {
    (p.len() >= 3).then(|| format!("{}.{}", p[1], p[2]))
}

pub fn decode_fc_variant(p: &[u8]) -> Option<String> {
    (p.len() >= 4).then(|| String::from_utf8_lossy(&p[..4]).into_owned())
}

/// 4.x replies `major minor patch`. 2025.12+ replies `yy month patch` followed by a
/// length-prefixed version string ("2025.12.5"); the string is authoritative there.
pub fn decode_fc_version(p: &[u8]) -> Option<String> {
    if p.len() < 3 {
        return None;
    }
    if p.len() > 4 {
        let n = p[3] as usize;
        if let Some(s) = p.get(4..4 + n) {
            return Some(String::from_utf8_lossy(s).into_owned());
        }
    }
    Some(format!("{}.{}.{}", p[0], p[1], p[2]))
}

fn pstr(p: &[u8], i: &mut usize) -> Option<String> {
    let n = *p.get(*i)? as usize;
    let s = p.get(*i + 1..*i + 1 + n)?;
    *i += 1 + n;
    Some(String::from_utf8_lossy(s).into_owned())
}

pub fn decode_board_info(p: &[u8]) -> Option<BoardInfo> {
    let identifier = String::from_utf8_lossy(p.get(..4)?).into_owned();
    let hardware_revision = u16::from_le_bytes([*p.get(4)?, *p.get(5)?]);
    let mut i = 8; // + OSD/max7456 type, target capabilities
    let target_name = pstr(p, &mut i)?;
    let board_name = pstr(p, &mut i)?;
    let manufacturer_id = pstr(p, &mut i)?;
    let mcu_type_id = p.get(i + 32).copied(); // after the 32-byte signature
    Some(BoardInfo {
        identifier,
        hardware_revision,
        target_name,
        board_name,
        manufacturer_id,
        mcu_type_id,
    })
}

/// MSP_UID is three little-endian u32 words; rendered like the CLI `mcu_id` line
/// (`%08x%08x%08x`), so the UID matches what a `dump` shows.
pub fn decode_uid(p: &[u8]) -> Option<String> {
    if p.len() < 12 {
        return None;
    }
    Some(
        p[..12]
            .chunks(4)
            .map(|w| format!("{:08x}", u32::from_le_bytes([w[0], w[1], w[2], w[3]])))
            .collect(),
    )
}

/// Inverse of [`decode_uid`]; `mcu_id` is the 24-hex-digit string from the CLI.
pub fn encode_uid(mcu_id: &str) -> Option<Vec<u8>> {
    if mcu_id.len() != 24 {
        return None;
    }
    let mut out = Vec::with_capacity(12);
    for i in 0..3 {
        let w = u32::from_str_radix(mcu_id.get(i * 8..i * 8 + 8)?, 16).ok()?;
        out.extend_from_slice(&w.to_le_bytes());
    }
    Some(out)
}

/// Build the identification part of [`FcInfo`] from raw MSP replies (pure; the caller
/// performs the requests). `build_options` is left empty: MSP_BUILD_INFO only carries
/// numeric option ids on 4.5+.
pub fn fc_info(
    api: &[u8],
    variant: &[u8],
    version: &[u8],
    board: &[u8],
    uid: Option<&[u8]>,
) -> Option<FcInfo> {
    let b = decode_board_info(board)?;
    Some(FcInfo {
        fc_variant: decode_fc_variant(variant)?,
        firmware_version: decode_fc_version(version)?,
        api_version: decode_api_version(api)?,
        board_name: if b.board_name.is_empty() {
            b.target_name.clone()
        } else {
            b.board_name.clone()
        },
        manufacturer_id: (!b.manufacturer_id.is_empty()).then(|| b.manufacturer_id.clone()),
        mcu: b.mcu_type_id.and_then(mcu_name).map(str::to_string),
        uid: uid.and_then(decode_uid),
        build_options: Vec::new(),
        serial_ports: Vec::new(),
        rx_spi: Default::default(),
    })
}
