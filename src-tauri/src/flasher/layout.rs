// SPDX-License-Identifier: GPL-3.0-or-later
//! DfuSe memory layout strings, e.g. `@Internal Flash  /0x08000000/04*016Kg,01*064Kg,07*128Kg`.
//!
//! Ported from Betaflight Configurator `src/js/protocols/usbdfu.ts` `getChipInfo` /
//! `parseDescriptor` (GPL-3.0-or-later), including its fix-ups for known-bad bootloader strings
//! (SPRacing H7 EXST, AT32F43x option bytes, GD32H7 "InternalFlash"). Differences: the `M`
//! unit is 1024*1024 (the original multiplies by 1024 only), and the sector attribute letter is
//! kept (but, as in the original, not enforced).

use serde::Serialize;
use std::collections::BTreeMap;

/// A run of equally sized pages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sector {
    pub start_address: u32,
    pub num_pages: u32,
    pub page_size: u32,
    /// DfuSe attribute letter: a=readable, b=erasable, d=writeable, g=all (a|b|d), ...
    pub attributes: char,
}

impl Sector {
    pub fn total_size(&self) -> u64 {
        self.num_pages as u64 * self.page_size as u64
    }
    pub fn end(&self) -> u64 {
        self.start_address as u64 + self.total_size()
    }
    /// Start addresses of every page.
    pub fn pages(&self) -> impl Iterator<Item = u32> + '_ {
        (0..self.num_pages).map(move |i| self.start_address + i * self.page_size)
    }
}

/// One memory region of a DFU alt setting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRegion {
    /// As printed, e.g. "Internal Flash".
    pub name: String,
    pub start_address: u32,
    pub sectors: Vec<Sector>,
}

impl MemoryRegion {
    pub fn total_size(&self) -> u64 {
        self.sectors.iter().map(Sector::total_size).sum()
    }

    /// Lower-case key with spaces as underscores ("internal_flash").
    pub fn key(&self) -> String {
        self.name
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("_")
            .to_lowercase()
    }

    /// True if `[address, address+len)` lies entirely inside this region's sectors
    /// (usbdfu.ts `isBlockUsable`).
    pub fn contains(&self, address: u32, len: usize) -> bool {
        if len == 0 {
            return true;
        }
        let mut cur = address as u64;
        let end = address as u64 + len as u64;
        while cur < end {
            match self
                .sectors
                .iter()
                .find(|s| cur >= s.start_address as u64 && cur < s.end())
            {
                Some(s) => cur = s.end(),
                None => return false,
            }
        }
        true
    }

    /// Pages that overlap `[address, address+len)`.
    pub fn pages_overlapping(&self, address: u32, len: usize) -> Vec<(u32, u32)> {
        let lo = address as u64;
        let hi = lo + len as u64;
        let mut out = Vec::new();
        for s in &self.sectors {
            for p in s.pages() {
                let ps = p as u64;
                let pe = ps + s.page_size as u64;
                if ps < hi && lo < pe {
                    out.push((p, s.page_size));
                }
            }
        }
        out
    }

    /// Every page, for a full-chip erase.
    pub fn all_pages(&self) -> Vec<(u32, u32)> {
        self.sectors
            .iter()
            .flat_map(|s| s.pages().map(move |p| (p, s.page_size)))
            .collect()
    }
}

/// Memory regions keyed by [`MemoryRegion::key`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ChipInfo {
    pub regions: BTreeMap<String, MemoryRegion>,
}

impl ChipInfo {
    /// Build from the alt-setting strings; unparseable ones are skipped (like usbdfu.ts).
    pub fn from_descriptors<S: AsRef<str>>(descriptors: &[S]) -> Self {
        let mut regions = BTreeMap::new();
        for d in descriptors {
            match parse_memory_descriptor(d.as_ref()) {
                Some(r) => {
                    regions.insert(r.key(), r);
                }
                None => log::warn!(
                    "skipping unparseable DFU memory descriptor {:?}",
                    d.as_ref()
                ),
            }
        }
        ChipInfo { regions }
    }

    pub fn internal_flash(&self) -> Option<&MemoryRegion> {
        self.regions.get("internal_flash")
    }
    pub fn external_flash(&self) -> Option<&MemoryRegion> {
        self.regions.get("external_flash")
    }
    pub fn option_bytes(&self) -> Option<&MemoryRegion> {
        self.regions.get("option_bytes")
    }
    /// The flash to program: internal, else external.
    pub fn flash(&self) -> Option<&MemoryRegion> {
        self.internal_flash().or_else(|| self.external_flash())
    }
}

/// Leading decimal digits (JS `parseInt` semantics on "016Kg" -> 16).
fn leading_int(s: &str) -> Option<u64> {
    let s = s.trim_start();
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn parse_address(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        let hex: String = h.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        u32::from_str_radix(&hex, 16).ok()
    } else {
        leading_int(s).and_then(|v| u32::try_from(v).ok())
    }
}

/// Parse one DfuSe memory descriptor string.
pub fn parse_memory_descriptor(input: &str) -> Option<MemoryRegion> {
    let mut s = input.to_string();
    // Known-bad bootloader strings (from usbdfu.ts).
    if s == "@External Flash /0x90000000/1001*128Kg,3*128Kg,20*128Ka" {
        s = "@External Flash /0x90000000/998*128Kg,1*128Kg,4*128Kg,21*128Ka".into();
    }
    if s == "@Option byte   /0x1FFFC000/01*4096 g" {
        s = "@Option bytes   /0x1FFFC000/01*4096 g".into();
    }
    if s == "@Option byte   /0x1FFFC000/01*512 g" {
        s = "@Option bytes   /0x1FFFC000/01*512 g".into();
    }
    if let Some(rest) = s.strip_prefix("@InternalFlash") {
        s = format!("@Internal Flash{rest}");
    }
    let s: String = s.chars().filter(|c| (' '..='~').contains(c)).collect();
    let parts: Vec<&str> = s.split('/').collect();
    // G474 lists two option byte banks; keep the first (as usbdfu.ts does).
    if parts.len() < 3 || !parts[0].starts_with('@') {
        return None;
    }
    let name = parts[0].trim().trim_start_matches('@').trim().to_string();
    let start_address = parse_address(parts[1])?;

    let mut sectors = Vec::new();
    let mut offset: u64 = 0;
    for spec in parts[2].split(',') {
        let (count, size) = spec.split_once('*')?;
        let num_pages = u32::try_from(leading_int(count)?).ok()?;
        let base = leading_int(size)?;
        if base == 0 {
            return None;
        }
        // The unit is the second-to-last char ("016Kg" -> 'K', "4096 g" -> ' ').
        let chars: Vec<char> = size.trim_end().chars().collect();
        let unit = if chars.len() >= 2 {
            chars[chars.len() - 2]
        } else {
            ' '
        };
        let attributes = chars.last().copied().unwrap_or(' ');
        let mult = match unit {
            'K' => 1024,
            'M' => 1024 * 1024,
            _ => 1,
        };
        let page_size = u32::try_from(base * mult).ok()?;
        let start = u32::try_from(start_address as u64 + offset).ok()?;
        sectors.push(Sector {
            start_address: start,
            num_pages,
            page_size,
            attributes,
        });
        offset += num_pages as u64 * page_size as u64;
    }
    if sectors.is_empty() {
        return None;
    }
    Some(MemoryRegion {
        name,
        start_address,
        sectors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f405_layout() {
        let r = parse_memory_descriptor("@Internal Flash  /0x08000000/04*016Kg,01*064Kg,07*128Kg")
            .unwrap();
        assert_eq!(r.key(), "internal_flash");
        assert_eq!(r.start_address, 0x0800_0000);
        assert_eq!(r.sectors.len(), 3);
        assert_eq!(
            r.sectors[0],
            Sector {
                start_address: 0x0800_0000,
                num_pages: 4,
                page_size: 16384,
                attributes: 'g'
            }
        );
        assert_eq!(r.sectors[1].start_address, 0x0801_0000);
        assert_eq!(r.sectors[2].start_address, 0x0802_0000);
        assert_eq!(r.total_size(), 1024 * 1024);
    }

    #[test]
    fn other_known_strings() {
        let f3 = parse_memory_descriptor("@Internal Flash  /0x08000000/128*0002Kg").unwrap();
        assert_eq!(f3.total_size(), 256 * 1024);
        let f74 =
            parse_memory_descriptor("@Internal Flash  /0x08000000/04*032Kg,01*128Kg,03*256Kg")
                .unwrap();
        assert_eq!(f74.total_size(), 1024 * 1024);
        let at32 =
            parse_memory_descriptor("@Internal Flash   /0x08000000/08*04Ka,1000*04Kg").unwrap();
        assert_eq!(at32.sectors[0].attributes, 'a');
        assert_eq!(at32.sectors[1].start_address, 0x0800_8000);
        assert_eq!(at32.total_size(), 1008 * 4096);
        let ob = parse_memory_descriptor("@Option byte   /0x1FFFC000/01*4096 g").unwrap();
        assert_eq!(ob.key(), "option_bytes");
        assert_eq!(ob.sectors[0].page_size, 4096);
        let g4 =
            parse_memory_descriptor("@Option Bytes   /0x1FFF7800/01*048 e/0x1FFFF800/01*048 e")
                .unwrap();
        assert_eq!(g4.start_address, 0x1FFF_7800);
        assert_eq!(g4.total_size(), 48);
        let h7 = parse_memory_descriptor("@External Flash /0x90000000/1001*128Kg,3*128Kg,20*128Ka")
            .unwrap();
        assert_eq!(h7.sectors[0].num_pages, 998);
        let gd = parse_memory_descriptor("@InternalFlash /0x08000000/512*4Kg").unwrap();
        assert_eq!(gd.key(), "internal_flash");
        let mb = parse_memory_descriptor("@Big /0x00000000/2*1Mg").unwrap();
        assert_eq!(mb.total_size(), 2 * 1024 * 1024);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_memory_descriptor("@Truncated /0x1FFF0000").is_none());
        assert!(parse_memory_descriptor("Internal Flash /0x08000000/4*16Kg").is_none());
        assert!(parse_memory_descriptor("@X /0x08000000/4x16Kg").is_none());
        assert!(parse_memory_descriptor("@X /0x08000000/4*0Kg").is_none());
        assert!(parse_memory_descriptor("").is_none());
    }

    #[test]
    fn chip_info_and_ranges() {
        let chip = ChipInfo::from_descriptors(&[
            "@Internal Flash  /0x08000000/04*016Kg,01*064Kg,07*128Kg",
            "@Option Bytes  /0x1FFFC000/01*016 e",
            "@garbage",
        ]);
        assert_eq!(chip.regions.len(), 2);
        let f = chip.flash().unwrap();
        assert!(chip.option_bytes().is_some());
        assert!(f.contains(0x0800_0000, 1024 * 1024));
        assert!(f.contains(0x0800_FFF0, 0x20)); // spans sectors
        assert!(!f.contains(0x0800_0000, 1024 * 1024 + 1));
        assert!(!f.contains(0x0700_0000, 4));
        assert_eq!(
            f.pages_overlapping(0x0800_FFF0, 0x20),
            vec![(0x0800_C000, 16384), (0x0801_0000, 65536)]
        );
        assert_eq!(
            f.pages_overlapping(0x0800_3FF0, 0x20),
            vec![(0x0800_0000, 16384), (0x0800_4000, 16384)]
        );
        assert_eq!(f.all_pages().len(), 12);
    }
}
