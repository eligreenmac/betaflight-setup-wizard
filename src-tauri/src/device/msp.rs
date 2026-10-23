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
    pub const MSP_RX_CONFIG: u16 = 44;
    pub const MSP_CF_SERIAL_CONFIG: u16 = 54;
    pub const MSP_ADVANCED_CONFIG: u16 = 90;
    pub const MSP_SET_ARMING_DISABLED: u16 = 99;
    pub const MSP_STATUS: u16 = 101;
    pub const MSP_MOTOR: u16 = 104;
    pub const MSP_RC: u16 = 105;
    pub const MSP_MOTOR_CONFIG: u16 = 131;
    pub const MSP_STATUS_EX: u16 = 150;
    pub const MSP_SET_MOTOR: u16 = 214;
    pub const MSP_UID: u16 = 160;
    pub const MSP2_COMMON_SERIAL_CONFIG: u16 = 0x1009;
    pub const MSP2_GET_TEXT: u16 = 0x3006;
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

/// MSP_API_VERSION. Betaflight 4.4 reports 1.45, 4.5 reports 1.46.
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

/// MSP_UID: the MCU's 96-bit unique id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Uid(pub [u32; 3]);

impl Uid {
    /// The configurator's `deviceIdentifier`: the three words in unpadded hex, concatenated.
    pub fn device_identifier(&self) -> String {
        format!("{:x}{:x}{:x}", self.0[0], self.0[1], self.0[2])
    }

    /// Fixed-width hex, safer as a key.
    pub fn to_hex(&self) -> String {
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
}

impl FcIdentity {
    /// MVP gate: Betaflight 4.4+ (API 1.45+).
    pub fn is_supported_betaflight(&self) -> bool {
        self.fc_variant == "BTFL" && self.api_version.at_least(1, 45)
    }

    /// Merge a CLI probe: RX_SPI becomes authoritative; the CLI serial list is used only when
    /// MSP gave none (MSP carries the raw identifiers, the 2025.12 CLI only names).
    pub fn apply_cli_probe(&mut self, probe: &crate::device::cli::CliBoardProbe) {
        self.rx_spi
            .apply_cli(&probe.resource, probe.rx_spi_protocol.as_deref());
        if self.serial_config.is_none() {
            self.serial_config = serial_config_from_cli(&probe.serial, Some(self.api_version));
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
        Ok(FcIdentity {
            api_version,
            fc_variant,
            fc_version,
            board_info,
            uid,
            serial_config,
            rx_spi,
        })
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
        assert_eq!(uid.device_identifier(), "1deadbeef10");
        assert_eq!(uid.to_hex(), "00000001deadbeef00000010");
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
        assert_eq!(id.uid.unwrap().device_identifier(), "123");
        assert!(id.is_supported_betaflight());
        // FcIdentity serialises for the Tauri layer.
        let json = serde_json::to_value(&id).unwrap();
        assert_eq!(json["boardInfo"]["boardName"], "MATEKF405");
        // Firmware without the serial/rx commands: fields degrade, identify still works.
        assert_eq!(id.serial_config, None);
        assert_eq!(id.rx_spi.has_rx_spi, None);
        assert_eq!(json["rxSpi"]["hasRxSpi"], serde_json::Value::Null);
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
