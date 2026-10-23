// SPDX-License-Identifier: GPL-3.0-or-later
//! MSP (MultiWii Serial Protocol) v1 and v2: framing, a resynchronising stream parser, a
//! request/response client, and typed helpers for the identification commands.
//!
//! Framing and checksums are ported from Betaflight Configurator `src/js/msp.ts`
//! (`encode_message_v1`, `encode_message_v2`, `crc8_dvb_s2`, the byte decoder); command codes
//! from `src/js/msp/MSPCodes.ts`; payload layouts from `src/js/msp/MSPHelper.ts`
//! (GPL-3.0-or-later). Unlike the configurator's byte state machine, the parser here keeps a
//! buffer and, on a bad header or checksum, drops only the leading `$` and rescans, so a real
//! frame that started inside garbage is still found.
//!
//! Frame layouts:
//! ```text
//! v1:       '$' 'M' dir len:u8 code:u8 payload[len] xor(len,code,payload)
//! v1 jumbo: '$' 'M' dir 0xFF code:u8 len:u16le payload[len] xor(0xFF,code,len_lo,len_hi,payload)
//! v2:       '$' 'X' dir flag:u8 code:u16le len:u16le payload[len] crc8_dvb_s2(flag..payload)
//! dir: '<' host->FC, '>' FC->host, '!' error / unsupported
//! ```

use crate::device::build_info;
use crate::device::error::{DeviceError, Result};
use crate::device::rx_spi::{parse_rx_config_spi_protocol, RxSpiInfo};
use crate::device::serial::SerialLink;
use crate::device::serial_ports::{
    parse_msp1_serial_config, parse_msp2_serial_config, serial_config_from_cli,
    serial_config_from_msp, SerialConfig, SerialConfigSource,
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// MSP command codes used by the wizard (values from MSPCodes.ts).
pub mod codes {
    pub const MSP_API_VERSION: u16 = 1;
    pub const MSP_FC_VARIANT: u16 = 2;
    pub const MSP_FC_VERSION: u16 = 3;
    pub const MSP_BOARD_INFO: u16 = 4;
    pub const MSP_BUILD_INFO: u16 = 5;
    pub const MSP_NAME: u16 = 10;
    pub const MSP_REBOOT: u16 = 68;
    pub const MSP_FEATURE_CONFIG: u16 = 36;
    pub const MSP_BOARD_ALIGNMENT_CONFIG: u16 = 38;
    pub const MSP_RX_CONFIG: u16 = 44;
    pub const MSP_CF_SERIAL_CONFIG: u16 = 54;
    pub const MSP_ADVANCED_CONFIG: u16 = 90;
    pub const MSP_SENSOR_CONFIG: u16 = 96;
    pub const MSP_SET_ARMING_DISABLED: u16 = 99;
    pub const MSP_STATUS: u16 = 101;
    pub const MSP_RAW_IMU: u16 = 102;
    pub const MSP_MOTOR: u16 = 104;
    pub const MSP_RC: u16 = 105;
    pub const MSP_ATTITUDE: u16 = 108;
    /// `canvas_cols, canvas_rows` on OSD builds, an empty reply without `USE_OSD`.
    pub const MSP_OSD_CANVAS: u16 = 189;
    pub const MSP_SENSOR_ALIGNMENT: u16 = 126;
    pub const MSP_MOTOR_CONFIG: u16 = 131;
    pub const MSP_STATUS_EX: u16 = 150;
    pub const MSP_ACC_CALIBRATION: u16 = 205;
    pub const MSP_SET_MOTOR: u16 = 214;
    pub const MSP_UID: u16 = 160;
    pub const MSP2_COMMON_SERIAL_CONFIG: u16 = 0x1009;
    pub const MSP2_GET_TEXT: u16 = 0x3006;
    pub const MSP2_SENSOR_CONFIG_ACTIVE: u16 = 0x300A;
    pub const MSP2_MCU_INFO: u16 = 0x300C;
}

/// `targetCapabilities` bits from MSP_BOARD_INFO (fc.ts `TARGET_CAPABILITIES_FLAGS`).
pub mod capabilities {
    pub const HAS_VCP: u8 = 0;
    pub const HAS_SOFTSERIAL: u8 = 1;
    pub const IS_UNIFIED: u8 = 2;
    pub const HAS_FLASH_BOOTLOADER: u8 = 3;
    pub const SUPPORTS_CUSTOM_DEFAULTS: u8 = 4;
    pub const HAS_CUSTOM_DEFAULTS: u8 = 5;
    pub const SUPPORTS_RX_BIND: u8 = 6;
}

const JUMBO_FRAME_MIN_SIZE: usize = 255;
/// Default cap on payload length; a garbage header claiming more is treated as noise.
pub const DEFAULT_MAX_PAYLOAD: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MspVersion {
    V1,
    V2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    /// `<` host to FC.
    Request,
    /// `>` FC to host.
    Response,
    /// `!` the FC did not recognise / could not run the command.
    Error,
}

impl Direction {
    fn byte(self) -> u8 {
        match self {
            Direction::Request => b'<',
            Direction::Response => b'>',
            Direction::Error => b'!',
        }
    }
    fn from_byte(b: u8) -> Option<Self> {
        match b {
            b'<' => Some(Direction::Request),
            b'>' => Some(Direction::Response),
            b'!' => Some(Direction::Error),
            _ => None,
        }
    }
}

/// One MSP frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MspFrame {
    pub version: MspVersion,
    pub direction: Direction,
    /// v2 flag byte (0 for v1).
    pub flag: u8,
    pub code: u16,
    pub payload: Vec<u8>,
}

impl MspFrame {
    pub fn request(version: MspVersion, code: u16, payload: &[u8]) -> Self {
        MspFrame {
            version,
            direction: Direction::Request,
            flag: 0,
            code,
            payload: payload.to_vec(),
        }
    }

    pub fn response(version: MspVersion, code: u16, payload: &[u8]) -> Self {
        MspFrame {
            version,
            direction: Direction::Response,
            flag: 0,
            code,
            payload: payload.to_vec(),
        }
    }

    /// Serialise to wire bytes.
    pub fn encode(&self) -> Result<Vec<u8>> {
        match self.version {
            MspVersion::V1 => {
                let code =
                    u8::try_from(self.code).map_err(|_| DeviceError::MspCodeTooLarge(self.code))?;
                Ok(encode_v1(self.direction, code, &self.payload))
            }
            MspVersion::V2 => Ok(encode_v2(
                self.direction,
                self.flag,
                self.code,
                &self.payload,
            )),
        }
    }
}

/// CRC-8/DVB-S2 (poly 0xD5), one byte step. Port of `MSP.crc8_dvb_s2`.
pub fn crc8_dvb_s2(mut crc: u8, byte: u8) -> u8 {
    crc ^= byte;
    for _ in 0..8 {
        crc = if crc & 0x80 != 0 {
            (crc << 1) ^ 0xD5
        } else {
            crc << 1
        };
    }
    crc
}

pub fn crc8_dvb_s2_slice(data: &[u8]) -> u8 {
    data.iter().fold(0, |crc, &b| crc8_dvb_s2(crc, b))
}

fn xor_slice(data: &[u8]) -> u8 {
    data.iter().fold(0, |acc, &b| acc ^ b)
}

/// Encode a v1 frame; payloads of 255+ bytes use the jumbo layout.
pub fn encode_v1(direction: Direction, code: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 8);
    out.extend_from_slice(&[b'$', b'M', direction.byte()]);
    if payload.len() >= JUMBO_FRAME_MIN_SIZE {
        let len = u16::try_from(payload.len()).unwrap_or(u16::MAX);
        out.extend_from_slice(&[0xFF, code]);
        out.extend_from_slice(&len.to_le_bytes());
    } else {
        out.extend_from_slice(&[payload.len() as u8, code]);
    }
    out.extend_from_slice(payload);
    let checksum = xor_slice(&out[3..]);
    out.push(checksum);
    out
}

/// Encode a v2 frame.
pub fn encode_v2(direction: Direction, flag: u8, code: u16, payload: &[u8]) -> Vec<u8> {
    let len = u16::try_from(payload.len()).unwrap_or(u16::MAX);
    let mut out = Vec::with_capacity(payload.len() + 9);
    out.extend_from_slice(&[b'$', b'X', direction.byte(), flag]);
    out.extend_from_slice(&code.to_le_bytes());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(payload);
    let crc = crc8_dvb_s2_slice(&out[3..]);
    out.push(crc);
    out
}

/// What the parser produced from a chunk of input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MspEvent {
    Frame(MspFrame),
    /// A well-formed header whose checksum did not match; the parser has resynchronised.
    ChecksumError {
        version: MspVersion,
        code: u16,
    },
}

enum Step {
    NeedMore,
    Skip(usize),
    Frame(MspFrame, usize),
    BadChecksum(MspVersion, u16),
}

/// Streaming MSP parser, tolerant of garbage and of frames split across reads.
#[derive(Debug, Clone)]
pub struct MspParser {
    buf: Vec<u8>,
    max_payload: usize,
    garbage: u64,
}

impl Default for MspParser {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_PAYLOAD)
    }
}

impl MspParser {
    pub fn new(max_payload: usize) -> Self {
        MspParser {
            buf: Vec::new(),
            max_payload,
            garbage: 0,
        }
    }

    /// Bytes discarded as noise so far.
    pub fn garbage_bytes(&self) -> u64 {
        self.garbage
    }

    /// Bytes buffered waiting for the rest of a frame.
    pub fn pending(&self) -> usize {
        self.buf.len()
    }

    pub fn reset(&mut self) {
        self.buf.clear();
    }

    /// Feed bytes; returns every complete frame (and checksum error) found.
    pub fn feed(&mut self, data: &[u8]) -> Vec<MspEvent> {
        self.buf.extend_from_slice(data);
        let mut events = Vec::new();
        loop {
            match self.step() {
                Step::NeedMore => break,
                Step::Skip(n) => {
                    self.garbage += n as u64;
                    self.buf.drain(..n);
                }
                Step::Frame(frame, used) => {
                    self.buf.drain(..used);
                    events.push(MspEvent::Frame(frame));
                }
                Step::BadChecksum(version, code) => {
                    self.garbage += 1;
                    self.buf.drain(..1);
                    events.push(MspEvent::ChecksumError { version, code });
                }
            }
        }
        events
    }

    fn step(&self) -> Step {
        let b = &self.buf;
        if b.is_empty() {
            return Step::NeedMore;
        }
        let Some(start) = b.iter().position(|&x| x == b'$') else {
            return Step::Skip(b.len());
        };
        if start > 0 {
            return Step::Skip(start);
        }
        if b.len() < 3 {
            return Step::NeedMore;
        }
        let version = match b[1] {
            b'M' => MspVersion::V1,
            b'X' => MspVersion::V2,
            _ => return Step::Skip(1),
        };
        let Some(direction) = Direction::from_byte(b[2]) else {
            return Step::Skip(1);
        };
        match version {
            MspVersion::V1 => {
                if b.len() < 5 {
                    return Step::NeedMore;
                }
                let code = b[4] as u16;
                let (len, data_start) = if b[3] as usize == JUMBO_FRAME_MIN_SIZE {
                    if b.len() < 7 {
                        return Step::NeedMore;
                    }
                    (u16::from_le_bytes([b[5], b[6]]) as usize, 7)
                } else {
                    (b[3] as usize, 5)
                };
                if len > self.max_payload {
                    return Step::Skip(1);
                }
                let total = data_start + len + 1;
                if b.len() < total {
                    return Step::NeedMore;
                }
                if xor_slice(&b[3..total - 1]) != b[total - 1] {
                    return Step::BadChecksum(version, code);
                }
                let payload = b[data_start..total - 1].to_vec();
                Step::Frame(
                    MspFrame {
                        version,
                        direction,
                        flag: 0,
                        code,
                        payload,
                    },
                    total,
                )
            }
            MspVersion::V2 => {
                if b.len() < 8 {
                    return Step::NeedMore;
                }
                let flag = b[3];
                let code = u16::from_le_bytes([b[4], b[5]]);
                let len = u16::from_le_bytes([b[6], b[7]]) as usize;
                if len > self.max_payload {
                    return Step::Skip(1);
                }
                let total = 8 + len + 1;
                if b.len() < total {
                    return Step::NeedMore;
                }
                if crc8_dvb_s2_slice(&b[3..total - 1]) != b[total - 1] {
                    return Step::BadChecksum(version, code);
                }
                let payload = b[8..total - 1].to_vec();
                Step::Frame(
                    MspFrame {
                        version,
                        direction,
                        flag,
                        code,
                        payload,
                    },
                    total,
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Typed payloads
// ---------------------------------------------------------------------------------------------

/// MSP_REBOOT types (MSPHelper.ts `REBOOT_TYPES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[repr(u8)]
pub enum RebootType {
    Firmware = 0,
    /// ROM (system memory) bootloader: the STM32/AT32 DFU used by the MVP.
    Bootloader = 1,
    Msc = 2,
    MscUtc = 3,
    /// Flash bootloader, on boards with `HAS_FLASH_BOOTLOADER`.
    BootloaderFlash = 4,
}

/// MSP_API_VERSION. Betaflight 4.4 reports 1.45, 4.5 1.46, 2025.12 1.47, 2026.6 1.48.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiVersion {
    pub protocol: u8,
    pub major: u8,
    pub minor: u8,
}

impl ApiVersion {
    pub fn at_least(&self, major: u8, minor: u8) -> bool {
        (self.major, self.minor) >= (major, minor)
    }
}

impl std::fmt::Display for ApiVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.0", self.major, self.minor)
    }
}

/// MSP_BOARD_INFO. Fields after `mcu_type_id` depend on the API version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardInfo {
    pub board_identifier: String,
    pub board_version: u16,
    pub board_type: u8,
    pub target_capabilities: u8,
    pub target_name: String,
    pub board_name: String,
    pub manufacturer_id: String,
    pub signature: Vec<u8>,
    pub mcu_type_id: Option<u8>,
    /// API 1.42+
    pub configuration_state: Option<u8>,
    /// API 1.43+
    pub sample_rate_hz: Option<u16>,
    /// API 1.43+
    pub configuration_problems: Option<u32>,
}

impl BoardInfo {
    pub fn has_capability(&self, bit: u8) -> bool {
        self.target_capabilities & (1 << bit) != 0
    }

    pub fn has_flash_bootloader(&self) -> bool {
        self.has_capability(capabilities::HAS_FLASH_BOOTLOADER)
    }

    /// The MSP_REBOOT type that reaches the bootloader on this board (as webstm32.ts picks it).
    pub fn bootloader_reboot_type(&self) -> RebootType {
        if self.has_flash_bootloader() {
            RebootType::BootloaderFlash
        } else {
            RebootType::Bootloader
        }
    }
}

/// MSP_UID: the MCU's 96-bit unique id as the three words `U_ID_0, U_ID_1, U_ID_2`
/// (msp.c writes each with `sbufWriteU32`, i.e. little-endian).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Uid(pub [u32; 3]);

impl Uid {
    /// Exactly what the CLI `mcu_id` command prints (cli.c:
    /// `"%08x%08x%08x", U_ID_0, U_ID_1, U_ID_2`): always 24 lowercase hex chars.
    /// This is `FcInfo.uid` and the backup folder name.
    pub fn mcu_id(&self) -> String {
        format!("{:08x}{:08x}{:08x}", self.0[0], self.0[1], self.0[2])
    }
}

/// What the wizard learns from a firmware-mode FC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FcIdentity {
    pub api_version: ApiVersion,
    pub fc_variant: String,
    pub fc_version: String,
    pub board_info: BoardInfo,
    pub uid: Option<Uid>,
    /// The board's serial ports exactly as reported (MSP2, else MSP v1; CLI can fill it later).
    /// `None` if the firmware answered neither command.
    pub serial_config: Option<SerialConfig>,
    /// Onboard SPI receiver. From `identify()` only the MSP hints are set (`hasRxSpi: null`);
    /// [`FcIdentity::apply_cli_probe`] makes it authoritative.
    pub rx_spi: RxSpiInfo,
    /// Build API defines decoded from MSP_BUILD_INFO (4.5+); empty on 4.4 / unknown.
    #[serde(default)]
    pub build_options: Vec<String>,
    /// `USE_OSD_HD` compiled in; `None` = undetermined (see `device::build_info`).
    #[serde(default)]
    pub hd_osd_supported: Option<bool>,
    /// MCU name from MSP2_MCU_INFO (e.g. "STM32F405"). Only requested when MSP_BOARD_INFO's
    /// MCU type id has no name in our table: 2026.6 always sends
    /// [`MCU_TYPE_ID_PROVIDED_BY_NAME`] there.
    #[serde(default)]
    pub mcu_name: Option<String>,
}

impl FcIdentity {
    /// Betaflight (API 1.45+) on a supported release line. The lines come from
    /// `config::firmware::SUPPORTED_LINES` via [`crate::config::firmware::is_supported`]
    /// (one source of truth). 2026.12.x and unparseable versions are not supported.
    pub fn is_supported_betaflight(&self) -> bool {
        self.fc_variant == "BTFL"
            && self.api_version.at_least(1, 45)
            && crate::config::firmware::is_supported(&self.fc_version)
    }

    /// Merge a CLI probe: RX_SPI becomes authoritative; the CLI serial list is used only when
    /// MSP gave none (MSP carries the raw identifiers, the 2025.12 CLI only names).
    pub fn apply_cli_probe(&mut self, probe: &crate::device::cli::CliBoardProbe) {
        self.rx_spi
            .apply_cli(&probe.resource, probe.rx_spi_protocol.as_deref());
        if self.serial_config.is_none() {
            self.serial_config = serial_config_from_cli(&probe.serial, Some(self.api_version));
        }
        if self.hd_osd_supported.is_none() {
            self.hd_osd_supported = probe
                .displayport_msp_col_adjust
                .as_deref()
                .and_then(build_info::hd_osd_from_cli_get);
        }
    }
}

pub(crate) struct Reader<'a> {
    code: u16,
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(code: u16, data: &'a [u8]) -> Self {
        Reader { code, data, pos: 0 }
    }
    pub(crate) fn short(&self, n: usize) -> DeviceError {
        DeviceError::MspPayloadTooShort {
            code: self.code,
            needed: self.pos + n,
            got: self.data.len(),
        }
    }
    pub(crate) fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let s = self
            .data
            .get(self.pos..self.pos + n)
            .ok_or_else(|| self.short(n))?;
        self.pos += n;
        Ok(s)
    }
    pub(crate) fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }
    pub(crate) fn u16(&mut self) -> Result<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    pub(crate) fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }
    pub(crate) fn u32(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    pub(crate) fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }
    /// Length-prefixed string (MSPHelper `getText`).
    pub(crate) fn text(&mut self) -> Result<String> {
        let n = self.u8()? as usize;
        Ok(latin1(self.bytes(n)?))
    }
}

/// Optional extras in `identify()`: an MSP error frame or a malformed payload means "unknown",
/// transport errors (timeout, disconnect) still fail the call.
fn optional<T>(r: Result<T>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(DeviceError::MspUnsupported(_)) | Err(DeviceError::MspPayloadTooShort { .. }) => {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// Like [`optional`], but a timeout is also "no answer": for requests that only fill
/// optional identity fields, so they can never fail `identify`.
fn best_effort<T>(r: Result<T>) -> Result<Option<T>> {
    match r {
        Err(DeviceError::Timeout { .. }) | Err(DeviceError::MspMalformed(_)) => Ok(None),
        r => optional(r),
    }
}

fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

pub fn parse_api_version(p: &[u8]) -> Result<ApiVersion> {
    let mut r = Reader::new(codes::MSP_API_VERSION, p);
    Ok(ApiVersion {
        protocol: r.u8()?,
        major: r.u8()?,
        minor: r.u8()?,
    })
}

pub fn parse_fc_variant(p: &[u8]) -> Result<String> {
    let mut r = Reader::new(codes::MSP_FC_VARIANT, p);
    Ok(latin1(r.bytes(4)?))
}

pub fn parse_fc_version(p: &[u8]) -> Result<String> {
    let mut r = Reader::new(codes::MSP_FC_VERSION, p);
    let major = r.u8()?;
    if major < 10 {
        Ok(format!("{}.{}.{}", major, r.u8()?, r.u8()?))
    } else {
        // Newer firmware: two bytes to discard, then the version as text.
        r.u16()?;
        r.text()
    }
}

/// `MCU_TYPE_ID_PROVIDED_BY_NAME` (2026.6 `build/build_config.h`, 254): the MCU type id that
/// MSP_BOARD_INFO / MSP2_MCU_INFO send instead of a `mcuTypeId_e` value; the name is in
/// MSP2_MCU_INFO. Defined once in `config::msp`.
pub use crate::config::msp::MCU_TYPE_ID_PROVIDED_BY_NAME;

pub fn parse_mcu_info(p: &[u8]) -> Result<(u8, String)> {
    let mut r = Reader::new(codes::MSP2_MCU_INFO, p);
    let id = r.u8()?;
    let len = r.u8()? as usize;
    let name = latin1(r.bytes(len)?);
    Ok((id, name))
}

pub fn parse_board_info(p: &[u8]) -> Result<BoardInfo> {
    let mut r = Reader::new(codes::MSP_BOARD_INFO, p);
    let board_identifier = latin1(r.bytes(4)?);
    let board_version = r.u16()?;
    let board_type = r.u8()?;
    let target_capabilities = r.u8()?;
    let target_name = r.text()?;
    let board_name = r.text()?;
    let manufacturer_id = r.text()?;
    let signature = r.bytes(32)?.to_vec();
    let mcu_type_id = if r.remaining() >= 1 {
        Some(r.u8()?)
    } else {
        None
    };
    let configuration_state = if r.remaining() >= 1 {
        Some(r.u8()?)
    } else {
        None
    };
    let (sample_rate_hz, configuration_problems) = if r.remaining() >= 6 {
        (Some(r.u16()?), Some(r.u32()?))
    } else {
        (None, None)
    };
    Ok(BoardInfo {
        board_identifier,
        board_version,
        board_type,
        target_capabilities,
        target_name,
        board_name,
        manufacturer_id,
        signature,
        mcu_type_id,
        configuration_state,
        sample_rate_hz,
        configuration_problems,
    })
}

pub fn parse_uid(p: &[u8]) -> Result<Uid> {
    let mut r = Reader::new(codes::MSP_UID, p);
    Ok(Uid([r.u32()?, r.u32()?, r.u32()?]))
}

// ---------------------------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------------------------

/// Request/response MSP over a [`SerialLink`]. Single outstanding request; responses for other
/// codes (late replies, telemetry) are skipped.
pub struct MspClient<S: SerialLink> {
    link: S,
    parser: MspParser,
    /// Version for codes <= 255. Codes above 255 always go out as v2.
    pub version: MspVersion,
    pub timeout: Duration,
}

impl<S: SerialLink> MspClient<S> {
    pub fn new(link: S) -> Self {
        MspClient {
            link,
            parser: MspParser::default(),
            version: MspVersion::V1,
            timeout: Duration::from_secs(1),
        }
    }

    pub fn link_mut(&mut self) -> &mut S {
        &mut self.link
    }

    pub fn into_inner(self) -> S {
        self.link
    }

    fn version_for(&self, code: u16) -> MspVersion {
        if code > 255 {
            MspVersion::V2
        } else {
            self.version
        }
    }

    /// Send a command without waiting.
    pub fn send(&mut self, code: u16, payload: &[u8]) -> Result<()> {
        let bytes = MspFrame::request(self.version_for(code), code, payload).encode()?;
        self.link.write_all(&bytes)
    }

    /// Send a command and wait for its response payload.
    pub fn request(&mut self, code: u16, payload: &[u8]) -> Result<Vec<u8>> {
        self.send(code, payload)?;
        self.wait_for(code, self.timeout)
    }

    fn wait_for(&mut self, code: u16, timeout: Duration) -> Result<Vec<u8>> {
        let deadline = Instant::now() + timeout;
        let mut buf = [0u8; 512];
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(DeviceError::timeout(
                    format!("MSP response to command {code}"),
                    timeout,
                ));
            }
            let n = self
                .link
                .read(&mut buf, (deadline - now).min(Duration::from_millis(50)))?;
            for ev in self.parser.feed(&buf[..n]) {
                match ev {
                    MspEvent::Frame(f) if f.code == code && f.direction == Direction::Response => {
                        return Ok(f.payload)
                    }
                    MspEvent::Frame(f) if f.code == code && f.direction == Direction::Error => {
                        return Err(DeviceError::MspUnsupported(code))
                    }
                    MspEvent::Frame(f) => {
                        log::debug!("MSP: skipping unrelated frame code {}", f.code)
                    }
                    MspEvent::ChecksumError { code: c, .. } => {
                        log::warn!("MSP: checksum error (code {c})")
                    }
                }
            }
        }
    }

    pub fn api_version(&mut self) -> Result<ApiVersion> {
        parse_api_version(&self.request(codes::MSP_API_VERSION, &[])?)
    }

    pub fn fc_variant(&mut self) -> Result<String> {
        parse_fc_variant(&self.request(codes::MSP_FC_VARIANT, &[])?)
    }

    pub fn fc_version(&mut self) -> Result<String> {
        parse_fc_version(&self.request(codes::MSP_FC_VERSION, &[])?)
    }

    pub fn board_info(&mut self) -> Result<BoardInfo> {
        parse_board_info(&self.request(codes::MSP_BOARD_INFO, &[])?)
    }

    pub fn uid(&mut self) -> Result<Uid> {
        parse_uid(&self.request(codes::MSP_UID, &[])?)
    }

    /// The handshake the configurator runs on connect, in the same order.
    pub fn identify(&mut self) -> Result<FcIdentity> {
        let api_version = self.api_version()?;
        let fc_variant = self.fc_variant()?;
        let fc_version = self.fc_version()?;
        let board_info = self.board_info()?;
        let mcu_name = if board_info
            .mcu_type_id
            .and_then(crate::device::manager::mcu_name)
            .is_some()
        {
            None
        } else {
            best_effort(
                self.request(codes::MSP2_MCU_INFO, &[])
                    .and_then(|p| parse_mcu_info(&p)),
            )?
            .map(|(_, name)| name)
            .filter(|n| !n.is_empty())
        };
        let uid = match self.uid() {
            Ok(u) => Some(u),
            Err(DeviceError::MspUnsupported(_)) => None,
            Err(e) => return Err(e),
        };
        let serial_config = optional(self.serial_config(Some(api_version)))?;
        let features = optional(
            self.request(codes::MSP_FEATURE_CONFIG, &[])
                .and_then(|p| Reader::new(codes::MSP_FEATURE_CONFIG, &p).u32()),
        )?;
        let rx_proto = optional(
            self.request(codes::MSP_RX_CONFIG, &[])
                .and_then(|p| parse_rx_config_spi_protocol(&p)),
        )?;
        let rx_spi = RxSpiInfo::from_msp(features, rx_proto);
        let (build_options, hd_osd_supported) = self.build_options_and_hd_osd(api_version)?;
        Ok(FcIdentity {
            api_version,
            fc_variant,
            fc_version,
            board_info,
            uid,
            serial_config,
            rx_spi,
            build_options,
            hd_osd_supported,
            mcu_name,
        })
    }

    /// MSP_BUILD_INFO options (4.5+) and the MSP part of the HD OSD decision: build options
    /// first, then an empty MSP_OSD_CANVAS reply (no OSD at all). ARCHITECTURE §4.1.
    fn build_options_and_hd_osd(&mut self, api: ApiVersion) -> Result<(Vec<String>, Option<bool>)> {
        let info = best_effort(
            self.request(codes::MSP_BUILD_INFO, &[])
                .and_then(|p| build_info::parse_build_info(&p, api)),
        )?;
        let names = info.as_ref().map(|b| b.option_names()).unwrap_or_default();
        let mut hd = build_info::hd_osd_from_build_options(
            info.as_ref().and_then(|b| b.option_ids.as_deref()),
        );
        if hd.is_none() {
            if let Some(p) = best_effort(self.request(codes::MSP_OSD_CANVAS, &[]))? {
                hd = build_info::hd_osd_from_osd_canvas(&p);
            }
        }
        Ok((names, hd))
    }

    /// Serial port list: MSP2_COMMON_SERIAL_CONFIG (what the configurator uses), falling back to
    /// MSP_CF_SERIAL_CONFIG if the firmware rejects the v2 command. Identifiers are kept raw.
    pub fn serial_config(&mut self, api: Option<ApiVersion>) -> Result<SerialConfig> {
        match self.request(codes::MSP2_COMMON_SERIAL_CONFIG, &[]) {
            Ok(p) => Ok(serial_config_from_msp(
                SerialConfigSource::Msp2,
                parse_msp2_serial_config(&p)?,
                api,
            )),
            Err(DeviceError::MspUnsupported(_)) => {
                let p = self.request(codes::MSP_CF_SERIAL_CONFIG, &[])?;
                Ok(serial_config_from_msp(
                    SerialConfigSource::Msp1,
                    parse_msp1_serial_config(&p)?,
                    api,
                ))
            }
            Err(e) => Err(e),
        }
    }

    /// Ask the FC to reboot. Firmware often drops the VCP before replying, so a missing reply or
    /// a disconnect counts as success. Returns whether the FC acknowledged.
    pub fn reboot(&mut self, kind: RebootType) -> Result<bool> {
        self.send(codes::MSP_REBOOT, &[kind as u8])?;
        match self.wait_for(codes::MSP_REBOOT, Duration::from_millis(500)) {
            Ok(_) => Ok(true),
            Err(DeviceError::Timeout { .. }) | Err(DeviceError::Disconnected) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::mock::MockSerial;

    #[test]
    fn crc8_check_value() {
        // CRC-8/DVB-S2 catalogue check value.
        assert_eq!(crc8_dvb_s2_slice(b"123456789"), 0xBC);
    }

    #[test]
    fn v1_known_vector() {
        // MSP_API_VERSION request: $M< 00 01 01
        assert_eq!(encode_v1(Direction::Request, 1, &[]), b"$M<\x00\x01\x01");
        // MSP_REBOOT to bootloader: len 1, code 68, payload 1 -> 1^68^1 = 68
        assert_eq!(
            encode_v1(Direction::Request, 68, &[1]),
            vec![b'$', b'M', b'<', 1, 68, 1, 68]
        );
    }

    #[test]
    fn v2_known_vector() {
        // MSP_IDENT (100) v2 request, the INAV-documented vector: $X< 00 64 00 00 00 8F
        assert_eq!(
            encode_v2(Direction::Request, 0, 100, &[]),
            vec![b'$', b'X', b'<', 0, 0x64, 0, 0, 0, 0x8F]
        );
    }

    #[test]
    fn v1_code_too_large() {
        let f = MspFrame::request(MspVersion::V1, 0x3006, &[]);
        assert!(matches!(
            f.encode(),
            Err(DeviceError::MspCodeTooLarge(0x3006))
        ));
    }

    fn roundtrip(frame: MspFrame) {
        let bytes = frame.encode().unwrap();
        let mut p = MspParser::default();
        assert_eq!(p.feed(&bytes), vec![MspEvent::Frame(frame)]);
        assert_eq!(p.pending(), 0);
        assert_eq!(p.garbage_bytes(), 0);
    }

    #[test]
    fn roundtrips() {
        roundtrip(MspFrame::request(MspVersion::V1, 1, &[]));
        roundtrip(MspFrame::response(MspVersion::V1, 4, &[1, 2, 3, 4, 5]));
        roundtrip(MspFrame::response(
            MspVersion::V1,
            200,
            &(0..=254u8).collect::<Vec<_>>(),
        )); // jumbo edge
        roundtrip(MspFrame::response(MspVersion::V1, 5, &vec![0xAA; 1000])); // jumbo
        roundtrip(MspFrame::request(MspVersion::V2, 0x3006, &[3]));
        roundtrip(MspFrame::response(MspVersion::V2, 0x300C, &vec![0x24; 600]));
        roundtrip(MspFrame {
            version: MspVersion::V2,
            direction: Direction::Error,
            flag: 1,
            code: 7,
            payload: vec![],
        });
    }

    #[test]
    fn jumbo_header_layout() {
        let bytes = encode_v1(Direction::Response, 9, &[0u8; 300]);
        assert_eq!(&bytes[..7], &[b'$', b'M', b'>', 0xFF, 9, 0x2C, 0x01]);
        assert_eq!(bytes.len(), 7 + 300 + 1);
    }

    #[test]
    fn parser_handles_byte_by_byte_and_garbage() {
        let a = MspFrame::response(MspVersion::V1, 1, &[0, 1, 46]);
        let b = MspFrame::response(MspVersion::V2, 0x3006, b"hello");
        let mut stream = b"\r\n# garbage $$ $Q $M".to_vec(); // includes fake sync bytes
        stream.extend(a.encode().unwrap());
        stream.extend(b"\x00\xFF noise");
        stream.extend(b.encode().unwrap());

        let mut p = MspParser::default();
        let mut frames = Vec::new();
        for byte in &stream {
            for ev in p.feed(std::slice::from_ref(byte)) {
                if let MspEvent::Frame(f) = ev {
                    frames.push(f);
                }
            }
        }
        assert_eq!(frames, vec![a, b]);
        assert!(p.garbage_bytes() > 0);
    }

    #[test]
    fn parser_resyncs_after_bad_checksum_inside_frame() {
        let good = MspFrame::response(MspVersion::V1, 2, b"BTFL");
        let mut bad = encode_v1(Direction::Response, 3, &[4, 5, 0]);
        *bad.last_mut().unwrap() ^= 0xFF; // corrupt checksum
        let mut stream = bad.clone();
        stream.extend(good.encode().unwrap());
        let mut p = MspParser::default();
        let evs = p.feed(&stream);
        assert_eq!(
            evs[0],
            MspEvent::ChecksumError {
                version: MspVersion::V1,
                code: 3
            }
        );
        assert_eq!(evs[1], MspEvent::Frame(good));
    }

    #[test]
    fn parser_finds_frame_starting_inside_false_header() {
        // A '$M>' with a huge (bogus) length that would swallow the real frame if trusted.
        let good = MspFrame::response(MspVersion::V2, 1, &[0, 1, 46]);
        let mut stream = vec![b'$', b'X', b'>', 0, 1, 0, 0xFF, 0xFF];
        stream.extend(good.encode().unwrap());
        let mut p = MspParser::default();
        assert_eq!(p.feed(&stream), vec![MspEvent::Frame(good)]);
    }

    #[test]
    fn parser_waits_for_partial_frame() {
        let f = MspFrame::response(MspVersion::V2, 0x1234, &[9; 10]);
        let bytes = f.encode().unwrap();
        let mut p = MspParser::default();
        assert!(p.feed(&bytes[..5]).is_empty());
        assert!(p.feed(&bytes[5..12]).is_empty());
        assert_eq!(p.feed(&bytes[12..]), vec![MspEvent::Frame(f)]);
    }

    fn board_info_payload() -> Vec<u8> {
        let mut p = b"S405".to_vec();
        p.extend(0u16.to_le_bytes());
        p.push(2); // board type
        p.push(0b0000_1101); // VCP, unified, flash bootloader
        for s in ["STM32F405", "MATEKF405", "MTKS"] {
            p.push(s.len() as u8);
            p.extend(s.as_bytes());
        }
        p.extend([0u8; 32]);
        p.push(1); // mcu type
        p.push(2); // configuration state
        p.extend(8000u16.to_le_bytes());
        p.extend(5u32.to_le_bytes());
        p
    }

    #[test]
    fn parses_board_info() {
        let b = parse_board_info(&board_info_payload()).unwrap();
        assert_eq!(b.board_identifier, "S405");
        assert_eq!(b.target_name, "STM32F405");
        assert_eq!(b.board_name, "MATEKF405");
        assert_eq!(b.manufacturer_id, "MTKS");
        assert_eq!(b.sample_rate_hz, Some(8000));
        assert_eq!(b.configuration_problems, Some(5));
        assert!(b.has_flash_bootloader());
        assert_eq!(b.bootloader_reboot_type(), RebootType::BootloaderFlash);
        // Old firmware without the trailing fields still parses.
        let short = &board_info_payload()[..board_info_payload().len() - 6];
        let b = parse_board_info(short).unwrap();
        assert_eq!(b.mcu_type_id, Some(1));
        assert_eq!(b.sample_rate_hz, None);
        assert!(parse_board_info(&[1, 2]).is_err());
    }

    #[test]
    fn uid_matches_cli_mcu_id() {
        // Known vector: an MSP_UID payload and the `mcu_id` line the same FC prints.
        // U_ID_0 = 0x0033002b, U_ID_1 = 0x3138510b, U_ID_2 = 0x32383733, each LE on the wire.
        let payload = [
            0x2b, 0x00, 0x33, 0x00, 0x0b, 0x51, 0x38, 0x31, 0x33, 0x37, 0x38, 0x32,
        ];
        let cli = "mcu_id 0033002b3138510b32383733";
        let uid = parse_uid(&payload).unwrap();
        assert_eq!(uid.0, [0x0033_002b, 0x3138_510b, 0x3238_3733]);
        assert_eq!(uid.mcu_id(), cli.strip_prefix("mcu_id ").unwrap());

        // Padding: words with leading zeros keep all 8 digits (the unpadded form would be
        // "1f0abc", ambiguous with other UIDs).
        let uid = parse_uid(&[1, 0, 0, 0, 0x0f, 0, 0, 0, 0xbc, 0x0a, 0, 0]).unwrap();
        assert_eq!(uid.mcu_id(), "000000010000000f00000abc");
        assert_eq!(Uid([0; 3]).mcu_id(), "000000000000000000000000");
        assert_eq!(Uid([u32::MAX; 3]).mcu_id(), "ffffffffffffffffffffffff");

        // Always 24 lowercase hex chars.
        for w in [
            [0, 0, 0],
            [1, 0x10, 0x100],
            [0xABCD_EF01, 7, 0x8000_0000],
            [u32::MAX; 3],
        ] {
            let s = Uid(w).mcu_id();
            assert_eq!(s.len(), 24, "{s}");
            assert!(
                s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "{s}"
            );
        }
        assert!(parse_uid(&payload[..11]).is_err());
    }

    #[test]
    fn parses_versions_and_uid() {
        assert_eq!(
            parse_api_version(&[0, 1, 46]).unwrap().to_string(),
            "1.46.0"
        );
        assert_eq!(parse_fc_version(&[4, 5, 1]).unwrap(), "4.5.1");
        // Major >= 10: major byte, two discarded bytes, then length-prefixed text.
        let mut p = vec![25u8, 12, 0];
        p.push(7);
        p.extend(b"2025.12");
        assert_eq!(parse_fc_version(&p).unwrap(), "2025.12");
        let uid = parse_uid(&[0x01, 0, 0, 0, 0xEF, 0xBE, 0xAD, 0xDE, 0x10, 0, 0, 0]).unwrap();
        assert_eq!(uid.mcu_id(), "00000001deadbeef00000010");
        assert!(ApiVersion {
            protocol: 0,
            major: 1,
            minor: 45
        }
        .at_least(1, 45));
        assert!(!ApiVersion {
            protocol: 0,
            major: 1,
            minor: 44
        }
        .at_least(1, 45));
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Serial {
        None,
        V1,
        V2,
    }

    /// A fake Betaflight that answers identification requests, replying in small chunks.
    fn fake_fc() -> MockSerial {
        fake_fc_with(Serial::None)
    }

    fn fake_fc_with(serial: Serial) -> MockSerial {
        let mut parser = MspParser::default();
        let board = board_info_payload();
        let mut m = MockSerial::new(move |w| {
            let mut out = b"\x00junk".to_vec();
            for ev in parser.feed(w) {
                let MspEvent::Frame(f) = ev else { continue };
                let reply = |p: &[u8]| MspFrame::response(f.version, f.code, p).encode().unwrap();
                out.extend(match f.code {
                    codes::MSP_API_VERSION => reply(&[0, 1, 46]),
                    codes::MSP_FC_VARIANT => reply(b"BTFL"),
                    codes::MSP_FC_VERSION => reply(&[4, 5, 2]),
                    codes::MSP_BOARD_INFO => reply(&board),
                    codes::MSP_UID => reply(&[1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0]),
                    codes::MSP2_COMMON_SERIAL_CONFIG if serial == Serial::V2 => {
                        // 2025.12-style ids: VCP, UART1 (=51, serial RX), UART2 (=52).
                        reply(&[
                            3, 20, 1, 0, 0, 0, 5, 4, 0, 5, 51, 64, 0, 0, 0, 5, 4, 0, 5, 52, 0, 0,
                            0, 0, 5, 4, 0, 5,
                        ])
                    }
                    codes::MSP_CF_SERIAL_CONFIG if serial != Serial::None => {
                        // 4.4-style ids: VCP, UART1 (=0).
                        reply(&[20, 1, 0, 5, 4, 0, 5, 0, 64, 0, 5, 4, 0, 5])
                    }
                    codes::MSP_FEATURE_CONFIG if serial != Serial::None => {
                        reply(&(1u32 << 25).to_le_bytes())
                    }
                    codes::MSP_RX_CONFIG if serial != Serial::None => {
                        let mut p = vec![0u8; 16];
                        p.extend([19, 0, 0, 0, 0, 0, 0]);
                        reply(&p)
                    }
                    codes::MSP_REBOOT => reply(&f.payload),
                    c => MspFrame {
                        version: f.version,
                        direction: Direction::Error,
                        flag: 0,
                        code: c,
                        payload: vec![],
                    }
                    .encode()
                    .unwrap(),
                });
            }
            out
        });
        m.chunk = 3;
        m
    }

    #[test]
    fn client_identify_against_fake_fc() {
        let mut c = MspClient::new(fake_fc());
        let id = c.identify().unwrap();
        assert_eq!(id.fc_variant, "BTFL");
        assert_eq!(id.fc_version, "4.5.2");
        assert_eq!(id.api_version.to_string(), "1.46.0");
        assert_eq!(id.uid.unwrap().mcu_id(), "000000010000000200000003");
        assert!(id.is_supported_betaflight());
        // FcIdentity serialises for the Tauri layer.
        let json = serde_json::to_value(&id).unwrap();
        assert_eq!(json["boardInfo"]["boardName"], "MATEKF405");
        // Firmware without the serial/rx commands: fields degrade, identify still works.
        assert_eq!(id.serial_config, None);
        assert_eq!(id.rx_spi.has_rx_spi, None);
        assert_eq!(json["rxSpi"]["hasRxSpi"], serde_json::Value::Null);
    }

    /// Fake FC for the HD OSD decision: `api_minor` 45 = 4.4, 46 = 4.5, 47 = 2025.12,
    /// 48 = 2026.6.
    /// `None` replies are MSP errors (command not compiled in).
    fn fake_fc_osd(
        api_minor: u8,
        build_info: Option<Vec<u8>>,
        canvas: Option<Vec<u8>>,
    ) -> MockSerial {
        let mut parser = MspParser::default();
        let board = board_info_payload();
        MockSerial::new(move |w| {
            let mut out = Vec::new();
            for ev in parser.feed(w) {
                let MspEvent::Frame(f) = ev else { continue };
                let reply = |p: &[u8]| MspFrame::response(f.version, f.code, p).encode().unwrap();
                let err = || {
                    MspFrame {
                        version: f.version,
                        direction: Direction::Error,
                        flag: 0,
                        code: f.code,
                        payload: vec![],
                    }
                    .encode()
                    .unwrap()
                };
                out.extend(match f.code {
                    codes::MSP_API_VERSION => reply(&[0, 1, api_minor]),
                    codes::MSP_FC_VARIANT => reply(b"BTFL"),
                    codes::MSP_FC_VERSION => reply(&[4, 5, 2]),
                    codes::MSP_BOARD_INFO => reply(&board),
                    codes::MSP_BUILD_INFO => build_info.as_deref().map_or_else(err, reply),
                    codes::MSP_OSD_CANVAS => canvas.as_deref().map_or_else(err, reply),
                    _ => err(),
                });
            }
            out
        })
    }

    fn build_info_reply(ids: &[u16]) -> Vec<u8> {
        let mut p = b"Jan  1 202500:00:00abc1234".to_vec();
        for id in ids {
            p.extend(id.to_le_bytes());
        }
        p
    }

    #[test]
    fn identify_hd_osd_per_firmware_line() {
        use crate::device::build_info::{BUILD_OPTION_OSD_HD as HD, BUILD_OPTION_OSD_SD as SD};
        let id = |minor, bi: Option<Vec<u8>>, canvas: Option<Vec<u8>>| {
            MspClient::new(fake_fc_osd(minor, bi, canvas))
                .identify()
                .unwrap()
        };
        let hd_build = build_info_reply(&[4097, 8231, SD, HD]);
        let sd_build = build_info_reply(&[4097, 8231, SD]);
        for minor in [46, 47, 48] {
            // 4.5 / 2025.12 / 2026.6: the build option list decides; canvas not needed.
            let i = id(minor, Some(hd_build.clone()), None);
            assert_eq!(i.hd_osd_supported, Some(true));
            assert_eq!(
                i.build_options,
                ["USE_SERIALRX_CRSF", "USE_DSHOT", "USE_OSD_SD", "USE_OSD_HD"]
            );
            let i = id(minor, Some(sd_build.clone()), Some(vec![30, 16]));
            assert_eq!(i.hd_osd_supported, Some(false));
            // MSP_BUILD_INFO failing: canvas evidence only.
            let i = id(minor, None, Some(vec![]));
            assert_eq!(i.hd_osd_supported, Some(false));
            assert!(i.build_options.is_empty());
            assert_eq!(id(minor, None, Some(vec![53, 20])).hd_osd_supported, None);
        }
        // 4.4: no options in MSP_BUILD_INFO.
        let i = id(45, Some(build_info_reply(&[])), Some(vec![53, 20]));
        assert_eq!(i.hd_osd_supported, None);
        assert!(i.build_options.is_empty());
        let i = id(45, Some(build_info_reply(&[])), Some(vec![]));
        assert_eq!(i.hd_osd_supported, Some(false));
        assert_eq!(id(45, None, None).hd_osd_supported, None);

        // CLI probe: "INVALID NAME" settles a null to false; an existing setting doesn't.
        let mut i = id(45, Some(build_info_reply(&[])), Some(vec![30, 16]));
        let mut probe = crate::device::cli::CliBoardProbe {
            displayport_msp_col_adjust: Some("displayport_msp_col_adjust = 0".into()),
            ..Default::default()
        };
        i.apply_cli_probe(&probe);
        assert_eq!(i.hd_osd_supported, None);
        probe.displayport_msp_col_adjust =
            Some("###ERROR IN get: INVALID NAME: displayport_msp_col_adjust###".into());
        i.apply_cli_probe(&probe);
        assert_eq!(i.hd_osd_supported, Some(false));
        // ...and never overrides a decision from build options.
        let mut i = id(46, Some(hd_build), None);
        i.apply_cli_probe(&probe);
        assert_eq!(i.hd_osd_supported, Some(true));
    }

    /// Replays the 2026.6.2 SITL capture (`tests/fixtures/device/2026.6.2/msp.txt`), plus
    /// an MSP2_MCU_INFO reply carrying `mcu`, which the capture does not include.
    fn fake_fc_2026_6_capture(mcu: Option<&'static [u8]>) -> MockSerial {
        let capture = include_str!("../../tests/fixtures/device/2026.6.2/msp.txt");
        let replies: std::collections::HashMap<u16, Vec<u8>> = capture
            .lines()
            .filter_map(|l| {
                let mut f = l.split_whitespace();
                let _name = f.next()?;
                let code = f.next()?.parse().ok()?;
                let hex = f.next().unwrap_or("");
                let bytes = (0..hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                    .collect();
                Some((code, bytes))
            })
            .collect();
        let mut parser = MspParser::default();
        let mut m = MockSerial::new(move |w| {
            let mut out = Vec::new();
            for ev in parser.feed(w) {
                let MspEvent::Frame(f) = ev else { continue };
                let payload = match f.code {
                    codes::MSP2_MCU_INFO => mcu.map(|n| {
                        let mut p = vec![MCU_TYPE_ID_PROVIDED_BY_NAME, n.len() as u8];
                        p.extend(n);
                        p
                    }),
                    c => replies.get(&c).cloned(),
                };
                out.extend(match payload {
                    Some(p) => MspFrame::response(f.version, f.code, &p).encode().unwrap(),
                    None => MspFrame {
                        version: f.version,
                        direction: Direction::Error,
                        flag: 0,
                        code: f.code,
                        payload: vec![],
                    }
                    .encode()
                    .unwrap(),
                });
            }
            out
        });
        m.chunk = 5;
        m
    }

    #[test]
    fn identify_2026_6_2_sitl_capture() {
        let id = MspClient::new(fake_fc_2026_6_capture(Some(b"SIMULATOR")))
            .identify()
            .unwrap();
        assert_eq!(id.fc_variant, "BTFL");
        assert_eq!(id.fc_version, "2026.6.2");
        assert_eq!(id.api_version.to_string(), "1.48.0");
        assert!(id.api_version.at_least(1, 46));
        assert!(id.is_supported_betaflight());
        assert_eq!(id.board_info.board_identifier, "SITL");
        assert_eq!(
            id.board_info.mcu_type_id,
            Some(MCU_TYPE_ID_PROVIDED_BY_NAME)
        );
        assert_eq!(id.mcu_name.as_deref(), Some("SIMULATOR"));
        assert_eq!(
            crate::device::manager::fc_info(&id).mcu.as_deref(),
            Some("SIMULATOR")
        );
        // 16427 (FLIGHT_PLAN) is sent twice by 2026.6.2; listed once, order kept.
        assert_eq!(
            id.build_options,
            [
                "USE_SERIALRX_CRSF",
                "USE_SERIALRX_FPORT",
                "USE_SERIALRX_GHST",
                "USE_SERIALRX_IBUS",
                "USE_SERIALRX_SBUS",
                "USE_SERIALRX_SPEKTRUM",
                "USE_TELEMETRY_SMARTPORT",
                "USE_ALTITUDE_HOLD",
                "USE_FLIGHT_PLAN",
                "USE_GPS",
                "USE_MAG",
                "USE_POSITION_HOLD",
                "USE_PWM_OUTPUT",
            ]
        );
        // No USE_OSD_HD option: a definite "no HD OSD" from the option list.
        assert_eq!(id.hd_osd_supported, Some(false));
        assert_eq!(id.uid.unwrap().mcu_id(), "000000000000000100000002");
    }

    #[test]
    fn supported_gate_uses_the_config_lines() {
        let mut id = MspClient::new(fake_fc_2026_6_capture(Some(b"SIMULATOR")))
            .identify()
            .unwrap();
        assert!(id.is_supported_betaflight());
        for (v, ok) in [
            ("4.4.3", true),
            ("4.5.5", true),
            ("2025.12.5", true),
            ("2026.6.2", true),
            ("2026.12.0", false),
            ("2026.12.0-alpha", false),
            ("4.3.2", false),
            ("", false),
            ("garbage", false),
        ] {
            id.fc_version = v.into();
            assert_eq!(id.is_supported_betaflight(), ok, "{v}");
        }
        id.fc_version = "2026.6.2".into();
        id.fc_variant = "INAV".into();
        assert!(!id.is_supported_betaflight());
    }

    #[test]
    fn identify_2026_6_without_mcu_info_keeps_mcu_unknown() {
        // MSP2_MCU_INFO is best effort: an error or an empty name leaves the MCU unknown
        // and identify still succeeds.
        for mcu in [None, Some(&b""[..])] {
            let id = MspClient::new(fake_fc_2026_6_capture(mcu))
                .identify()
                .unwrap();
            assert_eq!(id.fc_version, "2026.6.2");
            assert_eq!(id.mcu_name, None);
            assert_eq!(crate::device::manager::fc_info(&id).mcu, None);
        }
    }

    #[test]
    fn parse_2026_6_version_and_mcu_info() {
        let fc = hex_bytes("1a060208323032362e362e32");
        assert_eq!(parse_fc_version(&fc).unwrap(), "2026.6.2");
        assert_eq!(parse_fc_version(&[4, 5, 2]).unwrap(), "4.5.2");
        const BY_NAME: u8 = MCU_TYPE_ID_PROVIDED_BY_NAME;
        assert_eq!(BY_NAME, 254);
        let mut p = vec![BY_NAME, 9];
        p.extend(b"STM32F405");
        assert_eq!(
            parse_mcu_info(&p).unwrap(),
            (BY_NAME, "STM32F405".to_string())
        );
        assert_eq!(
            parse_mcu_info(&[BY_NAME, 0]).unwrap(),
            (BY_NAME, String::new())
        );
        assert!(parse_mcu_info(&[BY_NAME, 9, b'S']).is_err());
        assert!(parse_mcu_info(&[]).is_err());
    }

    fn hex_bytes(h: &str) -> Vec<u8> {
        (0..h.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn identify_survives_silent_build_info() {
        // A firmware that never answers the optional requests still identifies (after the
        // per-request timeout), with the fields left undetermined.
        let mut parser = MspParser::default();
        let board = board_info_payload();
        let m = MockSerial::new(move |w| {
            let mut out = Vec::new();
            for ev in parser.feed(w) {
                let MspEvent::Frame(f) = ev else { continue };
                let reply = |p: &[u8]| MspFrame::response(f.version, f.code, p).encode().unwrap();
                out.extend(match f.code {
                    codes::MSP_API_VERSION => reply(&[0, 1, 46]),
                    codes::MSP_FC_VARIANT => reply(b"BTFL"),
                    codes::MSP_FC_VERSION => reply(&[4, 5, 2]),
                    codes::MSP_BOARD_INFO => reply(&board),
                    codes::MSP_BUILD_INFO | codes::MSP_OSD_CANVAS => vec![],
                    c => MspFrame {
                        version: f.version,
                        direction: Direction::Error,
                        flag: 0,
                        code: c,
                        payload: vec![],
                    }
                    .encode()
                    .unwrap(),
                });
            }
            out
        });
        let mut c = MspClient::new(m);
        c.timeout = Duration::from_millis(30);
        let i = c.identify().unwrap();
        assert_eq!(i.hd_osd_supported, None);
        assert!(i.build_options.is_empty());
    }

    #[test]
    fn identify_reads_serial_ports_msp2_raw_ids() {
        let id = MspClient::new(fake_fc_with(Serial::V2)).identify().unwrap();
        let cfg = id.serial_config.as_ref().unwrap();
        assert_eq!(cfg.source, SerialConfigSource::Msp2);
        assert_eq!(
            cfg.scheme,
            crate::device::serial_ports::IdentifierScheme::UartBase50
        );
        assert_eq!(
            cfg.ports
                .iter()
                .map(|p| p.identifier.unwrap())
                .collect::<Vec<_>>(),
            [20, 51, 52]
        );
        assert_eq!(id.rx_spi.feature_enabled, Some(true));
        assert_eq!(id.rx_spi.msp_protocol_index, Some(19));
        assert_eq!(id.rx_spi.has_rx_spi, None);
        let json = serde_json::to_value(&id).unwrap();
        assert_eq!(json["serialConfig"]["ports"][1]["identifier"], 51);
    }

    #[test]
    fn identify_falls_back_to_msp1_serial_config() {
        let mut id = MspClient::new(fake_fc_with(Serial::V1)).identify().unwrap();
        let cfg = id.serial_config.as_ref().unwrap();
        assert_eq!(cfg.source, SerialConfigSource::Msp1);
        assert_eq!(
            cfg.scheme,
            crate::device::serial_ports::IdentifierScheme::Legacy
        );
        assert_eq!(cfg.ports[1].identifier, Some(0));
        // CLI probe makes RX_SPI authoritative but keeps the MSP port list.
        let probe = crate::device::cli::CliBoardProbe {
            serial: "serial UART1 64 115200 57600 0 115200".into(),
            resource: "resource RX_SPI_CS 1 NONE\r\n".into(),
            rx_spi_protocol: None,
            displayport_msp_col_adjust: None,
        };
        id.apply_cli_probe(&probe);
        assert_eq!(id.rx_spi.has_rx_spi, Some(false));
        assert_eq!(
            id.serial_config.as_ref().unwrap().source,
            SerialConfigSource::Msp1
        );
        // With no MSP list, the CLI list is used.
        id.serial_config = None;
        id.apply_cli_probe(&probe);
        assert_eq!(
            id.serial_config.unwrap().ports[0].name.as_deref(),
            Some("UART1")
        );
    }

    #[test]
    fn client_reports_unsupported_and_timeout() {
        let mut c = MspClient::new(fake_fc());
        assert!(matches!(
            c.request(codes::MSP_STATUS, &[]),
            Err(DeviceError::MspUnsupported(101))
        ));
        let mut silent = MspClient::new(MockSerial::silent());
        silent.timeout = Duration::from_millis(30);
        assert!(matches!(
            silent.api_version(),
            Err(DeviceError::Timeout { .. })
        ));
    }

    #[test]
    fn reboot_tolerates_disconnect() {
        let mut c = MspClient::new(fake_fc());
        assert!(c.reboot(RebootType::Bootloader).unwrap());
        let w = &c.link_mut().written;
        assert_eq!(&w[w.len() - 7..], &[b'$', b'M', b'<', 1, 68, 1, 68]);

        let mut m = MockSerial::silent();
        m.disconnect_after_write_containing = Some(vec![68]);
        let mut c = MspClient::new(m);
        assert!(!c.reboot(RebootType::Bootloader).unwrap());
    }
}
