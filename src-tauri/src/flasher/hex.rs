// SPDX-License-Identifier: GPL-3.0-or-later
//! Intel HEX parsing into contiguous memory segments.
//!
//! Behaviour follows Betaflight Configurator `src/js/workers/hex_parser.ts` (GPL-3.0-or-later):
//! data records are merged while addresses stay contiguous; types 04 (extended linear) and 05
//! (start linear) are honoured. Unlike the original, record checksums are verified, type 02
//! (extended segment) is supported, and an address jump backwards or a gap starts a new segment.

use crate::flasher::error::{FlashError, Result};
use serde::Serialize;

/// A contiguous run of bytes at an absolute address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Segment {
    pub address: u32,
    pub data: Vec<u8>,
}

impl Segment {
    pub fn end(&self) -> u64 {
        self.address as u64 + self.data.len() as u64
    }
}

/// A parsed firmware image.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct HexImage {
    pub segments: Vec<Segment>,
    pub start_linear_address: Option<u32>,
}

impl HexImage {
    pub fn bytes_total(&self) -> usize {
        self.segments.iter().map(|s| s.data.len()).sum()
    }

    /// A raw binary loaded at `address` (for `.bin` files).
    pub fn from_bin(address: u32, data: Vec<u8>) -> Self {
        HexImage {
            segments: vec![Segment { address, data }],
            start_linear_address: None,
        }
    }

    /// First address written, used to leave DFU.
    pub fn base_address(&self) -> Option<u32> {
        self.segments.iter().map(|s| s.address).min()
    }
}

fn err(line: usize, reason: impl Into<String>) -> FlashError {
    FlashError::Hex {
        line,
        reason: reason.into(),
    }
}

fn parse_hex_bytes(s: &str, line: usize) -> Result<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return Err(err(line, "odd number of hex digits"));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| err(line, "invalid hex digit")))
        .collect()
}

/// Parse Intel HEX text.
pub fn parse_intel_hex(text: &str) -> Result<HexImage> {
    let mut image = HexImage::default();
    let mut base: u32 = 0;
    let mut saw_eof = false;

    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if saw_eof {
            return Err(err(line_no, "data after end-of-file record"));
        }
        let body = line
            .strip_prefix(':')
            .ok_or_else(|| err(line_no, "missing ':'"))?;
        let bytes = parse_hex_bytes(body, line_no)?;
        if bytes.len() < 5 {
            return Err(err(line_no, "record too short"));
        }
        let count = bytes[0] as usize;
        if bytes.len() != count + 5 {
            return Err(err(line_no, "byte count does not match record length"));
        }
        let sum = bytes.iter().fold(0u8, |a, &b| a.wrapping_add(b));
        if sum != 0 {
            return Err(err(line_no, "checksum mismatch"));
        }
        let offset = u16::from_be_bytes([bytes[1], bytes[2]]) as u32;
        let rtype = bytes[3];
        let data = &bytes[4..4 + count];
        match rtype {
            0x00 => {
                let addr = base.wrapping_add(offset);
                match image.segments.last_mut() {
                    Some(seg) if seg.end() == addr as u64 => seg.data.extend_from_slice(data),
                    _ => image.segments.push(Segment {
                        address: addr,
                        data: data.to_vec(),
                    }),
                }
            }
            0x01 => saw_eof = true,
            0x02 => {
                if count != 2 {
                    return Err(err(line_no, "bad extended segment address record"));
                }
                base = (u16::from_be_bytes([data[0], data[1]]) as u32) << 4;
            }
            0x03 => {} // CS:IP start address, meaningless for ARM
            0x04 => {
                if count != 2 {
                    return Err(err(line_no, "bad extended linear address record"));
                }
                base = (u16::from_be_bytes([data[0], data[1]]) as u32) << 16;
            }
            0x05 => {
                if count != 4 {
                    return Err(err(line_no, "bad start linear address record"));
                }
                image.start_linear_address =
                    Some(u32::from_be_bytes([data[0], data[1], data[2], data[3]]));
            }
            t => return Err(err(line_no, format!("unknown record type {t:#04x}"))),
        }
    }
    if !saw_eof {
        return Err(err(text.lines().count(), "missing end-of-file record"));
    }
    image.segments.retain(|s| !s.data.is_empty());
    if image.segments.is_empty() {
        return Err(FlashError::EmptyImage);
    }
    Ok(image)
}

/// Encode an image back to Intel HEX (used by tests and for exporting).
pub fn to_intel_hex(image: &HexImage) -> String {
    fn record(out: &mut String, rtype: u8, offset: u16, data: &[u8]) {
        let mut bytes = vec![data.len() as u8, (offset >> 8) as u8, offset as u8, rtype];
        bytes.extend_from_slice(data);
        let sum = bytes.iter().fold(0u8, |a, &b| a.wrapping_add(b));
        bytes.push(sum.wrapping_neg());
        out.push(':');
        for b in bytes {
            out.push_str(&format!("{b:02X}"));
        }
        out.push('\n');
    }
    let mut out = String::new();
    let mut upper: Option<u16> = None;
    for seg in &image.segments {
        let mut addr = seg.address;
        for chunk in seg.data.chunks(16) {
            // Split chunks that would cross a 64 KiB boundary.
            let mut rest = chunk;
            while !rest.is_empty() {
                let hi = (addr >> 16) as u16;
                if upper != Some(hi) {
                    record(&mut out, 0x04, 0, &hi.to_be_bytes());
                    upper = Some(hi);
                }
                let room = (0x1_0000 - (addr & 0xFFFF)) as usize;
                let n = rest.len().min(room);
                record(&mut out, 0x00, addr as u16, &rest[..n]);
                addr = addr.wrapping_add(n as u32);
                rest = &rest[n..];
            }
        }
    }
    if let Some(start) = image.start_linear_address {
        record(&mut out, 0x05, 0, &start.to_be_bytes());
    }
    record(&mut out, 0x01, 0, &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = ":020000040800F2
:10000000000002200D010008F1010008F3010008C2
:10001000F5010008F7010008F901000800000000E0
:040000050800018965
:00000001FF
";

    #[test]
    fn parses_sample() {
        let img = parse_intel_hex(SAMPLE).unwrap();
        assert_eq!(img.segments.len(), 1);
        assert_eq!(img.segments[0].address, 0x0800_0000);
        assert_eq!(img.bytes_total(), 32);
        assert_eq!(&img.segments[0].data[..4], &[0x00, 0x00, 0x02, 0x20]);
        assert_eq!(img.start_linear_address, Some(0x0800_0189));
        assert_eq!(img.base_address(), Some(0x0800_0000));
    }

    #[test]
    fn gaps_make_new_segments() {
        let img = HexImage {
            segments: vec![
                Segment {
                    address: 0x0800_0000,
                    data: (0..40).collect(),
                },
                Segment {
                    address: 0x0800_C000,
                    data: vec![0xAB; 20],
                },
                Segment {
                    address: 0x0801_FFF8,
                    data: (0..16).collect(),
                }, // crosses 64K
            ],
            start_linear_address: None,
        };
        let text = to_intel_hex(&img);
        let back = parse_intel_hex(&text).unwrap();
        assert_eq!(back, img);
    }

    #[test]
    fn rejects_bad_input() {
        let bad_sum = SAMPLE.replace(
            ":10000000000002200D010008F1010008F3010008C2",
            ":10000000000002200D010008F1010008F3010008C3",
        );
        assert!(matches!(
            parse_intel_hex(&bad_sum),
            Err(FlashError::Hex { line: 2, .. })
        ));
        assert!(parse_intel_hex(":020000040800F2\n").is_err()); // no EOF
        assert!(parse_intel_hex("garbage\n").is_err());
        assert!(matches!(
            parse_intel_hex(":00000001FF\n"),
            Err(FlashError::EmptyImage)
        ));
        assert!(parse_intel_hex(":0200000000FE\n:00000001FF\n").is_err()); // length mismatch
    }

    #[test]
    fn extended_segment_address() {
        // base 0x1000 << 4 = 0x10000
        let text = ":020000021000EC\n:0100000042BD\n:00000001FF\n";
        let img = parse_intel_hex(text).unwrap();
        assert_eq!(img.segments[0].address, 0x10000);
        assert_eq!(img.segments[0].data, vec![0x42]);
    }

    #[test]
    fn crlf_and_blank_lines() {
        let img = parse_intel_hex(&SAMPLE.replace('\n', "\r\n\r\n")).unwrap();
        assert_eq!(img.bytes_total(), 32);
    }
}
