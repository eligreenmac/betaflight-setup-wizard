// SPDX-License-Identifier: GPL-3.0-or-later
//! STM32 / AT32 DfuSe flashing over an abstract USB transport.
//!
//! Ported from Betaflight Configurator `src/js/protocols/usbdfu.ts` (GPL-3.0-or-later):
//! the request/state/status tables, `clearStatus` (spec-correct ABORT vs CLRSTATUS, plus the
//! H743 Rev.V "wedged in dfuDNBUSY" CLRSTATUS pair), `loadAddress`, page erase with the same
//! busy handling, `wBlockNum` starting at 2, the option-byte read-protection probe, verify by
//! UPLOAD, and leaving DFU with a zero-length DNLOAD. The callback chain of the original is a
//! straight-line blocking sequence here; sleeps go through an injectable function so tests run
//! instantly against a simulated device.

use crate::flasher::error::{FlashError, Result};
use crate::flasher::hex::HexImage;
use crate::flasher::layout::{ChipInfo, MemoryRegion};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// DFU class requests.
pub mod request {
    pub const DETACH: u8 = 0x00;
    pub const DNLOAD: u8 = 0x01;
    pub const UPLOAD: u8 = 0x02;
    pub const GETSTATUS: u8 = 0x03;
    pub const CLRSTATUS: u8 = 0x04;
    pub const GETSTATE: u8 = 0x05;
    pub const ABORT: u8 = 0x06;
}

/// DFU device states (bState).
pub mod state {
    pub const APP_IDLE: u8 = 0;
    pub const APP_DETACH: u8 = 1;
    pub const DFU_IDLE: u8 = 2;
    pub const DFU_DNLOAD_SYNC: u8 = 3;
    pub const DFU_DNBUSY: u8 = 4;
    pub const DFU_DNLOAD_IDLE: u8 = 5;
    pub const DFU_MANIFEST_SYNC: u8 = 6;
    pub const DFU_MANIFEST: u8 = 7;
    pub const DFU_MANIFEST_WAIT_RESET: u8 = 8;
    pub const DFU_UPLOAD_IDLE: u8 = 9;
    pub const DFU_ERROR: u8 = 10;
}

/// DFU status codes (bStatus).
pub mod status {
    pub const OK: u8 = 0x00;
    pub const ERR_TARGET: u8 = 0x01;
    pub const ERR_FILE: u8 = 0x02;
    pub const ERR_WRITE: u8 = 0x03;
    pub const ERR_ERASE: u8 = 0x04;
    pub const ERR_CHECK_ERASED: u8 = 0x05;
    pub const ERR_PROG: u8 = 0x06;
    pub const ERR_VERIFY: u8 = 0x07;
    pub const ERR_ADDRESS: u8 = 0x08;
    pub const ERR_NOTDONE: u8 = 0x09;
    pub const ERR_FIRMWARE: u8 = 0x0A;
    pub const ERR_VENDOR: u8 = 0x0B;
    pub const ERR_USBR: u8 = 0x0C;
    pub const ERR_POR: u8 = 0x0D;
    pub const ERR_UNKNOWN: u8 = 0x0E;
    pub const ERR_STALLEDPKT: u8 = 0x0F;
}

/// DfuSe commands sent as DNLOAD block 0.
pub mod cmd {
    pub const SET_ADDRESS: u8 = 0x21;
    pub const ERASE: u8 = 0x41;
    pub const READ_UNPROTECT: u8 = 0x92;
}

/// Default when the functional descriptor gives no wTransferSize (F3/F4/F7).
pub const DEFAULT_TRANSFER_SIZE: usize = 2048;

/// The USB side, one claimed DFU interface. Requests are class/interface control transfers on
/// that interface. Implemented over rusb by `usb::RusbDfuTransport` and by a simulator in tests.
pub trait DfuTransport {
    fn control_in(&mut self, request: u8, value: u16, length: usize) -> Result<Vec<u8>>;
    fn control_out(&mut self, request: u8, value: u16, data: &[u8]) -> Result<()>;
    /// The iInterface strings of the DFU interface's alt settings (memory layouts).
    fn memory_descriptors(&mut self) -> Result<Vec<String>>;
    /// wTransferSize from the DFU functional descriptor, if present.
    fn transfer_size(&mut self) -> Result<Option<u16>>;
}

impl<T: DfuTransport + ?Sized> DfuTransport for Box<T> {
    fn control_in(&mut self, request: u8, value: u16, length: usize) -> Result<Vec<u8>> {
        (**self).control_in(request, value, length)
    }
    fn control_out(&mut self, request: u8, value: u16, data: &[u8]) -> Result<()> {
        (**self).control_out(request, value, data)
    }
    fn memory_descriptors(&mut self) -> Result<Vec<String>> {
        (**self).memory_descriptors()
    }
    fn transfer_size(&mut self) -> Result<Option<u16>> {
        (**self).transfer_size()
    }
}

/// A DFU_GETSTATUS reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DfuStatus {
    pub status: u8,
    pub poll_timeout_ms: u32,
    pub state: u8,
    pub istring: u8,
}

impl DfuStatus {
    pub fn parse(b: &[u8]) -> Result<Self> {
        if b.len() < 6 {
            return Err(FlashError::Usb(format!(
                "GETSTATUS returned {} bytes",
                b.len()
            )));
        }
        Ok(DfuStatus {
            status: b[0],
            poll_timeout_ms: u32::from_le_bytes([b[1], b[2], b[3], 0]),
            state: b[4],
            istring: b[5],
        })
    }
}

/// Which pages to erase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EraseMode {
    /// Only pages the firmware touches (keeps settings outside the image, e.g. on boards whose
    /// config lives in a separate flash sector).
    #[default]
    Pages,
    /// Every page of the flash region ("Full chip erase" in the configurator).
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DfuOptions {
    pub erase: EraseMode,
    pub verify: bool,
    /// Leave DFU and start the firmware when done.
    pub leave: bool,
    /// Probe the option bytes for read protection first; a protected chip fails with
    /// `ReadProtected` (never unprotected here; see [`DfuSe::remove_read_protection`]).
    pub check_read_protection: bool,
}

impl Default for DfuOptions {
    fn default() -> Self {
        DfuOptions {
            erase: EraseMode::Pages,
            verify: true,
            leave: true,
            check_read_protection: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DfuPhase {
    Detect,
    Erase,
    Write,
    Verify,
    Leave,
    Done,
}

/// Progress event payload (mapped to `model::FlashProgress` on `flasher://progress`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DfuProgress {
    pub phase: DfuPhase,
    /// Overall 0..=100.
    pub percent: f32,
    /// Work done/total within the phase (pages for erase, bytes for write/verify).
    pub done: u64,
    pub total: u64,
}

/// What a successful flash did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlashReport {
    pub flash_region: String,
    pub flash_size: u64,
    pub transfer_size: usize,
    pub pages_erased: usize,
    pub bytes_written: usize,
    pub verified: bool,
    pub duration_ms: u64,
}

/// A DfuSe session on an open transport.
pub struct DfuSe<T: DfuTransport> {
    transport: T,
    sleep: Box<dyn FnMut(Duration) + Send>,
    transfer_size: usize,
    /// Bound on the clear-status loop (usbdfu.ts uses 100).
    pub max_clear_attempts: u32,
}

impl<T: DfuTransport> DfuSe<T> {
    pub fn new(transport: T) -> Self {
        DfuSe {
            transport,
            sleep: Box::new(std::thread::sleep),
            transfer_size: DEFAULT_TRANSFER_SIZE,
            max_clear_attempts: 100,
        }
    }

    /// Replace the sleep function (tests pass a no-op).
    pub fn with_sleep(mut self, sleep: impl FnMut(Duration) + Send + 'static) -> Self {
        self.sleep = Box::new(sleep);
        self
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    pub fn into_transport(self) -> T {
        self.transport
    }

    fn wait_ms(&mut self, ms: u32) {
        if ms > 0 {
            (self.sleep)(Duration::from_millis(ms as u64));
        }
    }

    fn fail(step: &str, s: DfuStatus) -> FlashError {
        FlashError::Dfu {
            step: step.to_string(),
            status: s.status,
            state: s.state,
        }
    }

    pub fn get_status(&mut self) -> Result<DfuStatus> {
        let b = self.transport.control_in(request::GETSTATUS, 0, 6)?;
        DfuStatus::parse(&b)
    }

    /// Bring the device back to dfuIDLE (usbdfu.ts `clearStatus`).
    /// `busy_is_stuck`: the caller already waited out the poll timeout, so dfuDNBUSY means wedged.
    pub fn clear_status(&mut self, busy_is_stuck: bool) -> Result<()> {
        let mut last = 0;
        for _ in 0..self.max_clear_attempts {
            let s = self.get_status()?;
            last = s.state;
            match s.state {
                state::DFU_IDLE => return Ok(()),
                state::DFU_DNLOAD_IDLE | state::DFU_UPLOAD_IDLE => {
                    self.wait_ms(s.poll_timeout_ms);
                    self.transport.control_out(request::ABORT, 0, &[])?;
                }
                state::DFU_ERROR => {
                    self.wait_ms(s.poll_timeout_ms);
                    self.transport.control_out(request::CLRSTATUS, 0, &[])?;
                }
                state::DFU_DNBUSY if busy_is_stuck => {
                    self.wait_ms(s.poll_timeout_ms);
                    self.transport.control_out(request::CLRSTATUS, 0, &[])?;
                }
                _ => self.wait_ms(s.poll_timeout_ms.max(1)),
            }
        }
        Err(FlashError::DfuStuck(self.max_clear_attempts, last))
    }

    /// DNLOAD, then GETSTATUS which must report dfuDNBUSY; wait the poll timeout and return the
    /// next GETSTATUS.
    fn download_and_poll(&mut self, block: u16, data: &[u8], step: &str) -> Result<DfuStatus> {
        self.transport.control_out(request::DNLOAD, block, data)?;
        let s = self.get_status()?;
        if s.state != state::DFU_DNBUSY {
            return Err(Self::fail(step, s));
        }
        self.wait_ms(s.poll_timeout_ms);
        self.get_status()
    }

    /// Set the address pointer, returning the final status unchecked (the read-protection probe
    /// needs to see dfuERROR/errVENDOR).
    pub fn try_set_address(&mut self, address: u32) -> Result<DfuStatus> {
        let a = address.to_le_bytes();
        self.download_and_poll(
            0,
            &[cmd::SET_ADDRESS, a[0], a[1], a[2], a[3]],
            "set address",
        )
    }

    pub fn set_address(&mut self, address: u32) -> Result<()> {
        let s = self.try_set_address(address)?;
        if s.state != state::DFU_DNLOAD_IDLE {
            return Err(Self::fail(&format!("set address {address:#010x}"), s));
        }
        Ok(())
    }

    /// Erase the page at `address`.
    pub fn erase_page(&mut self, address: u32) -> Result<()> {
        let a = address.to_le_bytes();
        let step = format!("erase page {address:#010x}");
        let s = self.download_and_poll(0, &[cmd::ERASE, a[0], a[1], a[2], a[3]], &step)?;
        self.finish_erase(s, &step)
    }

    /// DfuSe mass erase (0x41 with no address). Not used by the default flow, which erases page
    /// by page like the configurator.
    pub fn mass_erase(&mut self) -> Result<()> {
        let s = self.download_and_poll(0, &[cmd::ERASE], "mass erase")?;
        self.finish_erase(s, "mass erase")
    }

    fn finish_erase(&mut self, s: DfuStatus, step: &str) -> Result<()> {
        match s.state {
            state::DFU_DNLOAD_IDLE => Ok(()),
            state::DFU_DNBUSY => {
                // H743 Rev.V stays in dfuDNBUSY past the reported delay; the erase is done.
                self.clear_status(true)?;
                let s = self.get_status()?;
                if s.state == state::DFU_IDLE {
                    Ok(())
                } else {
                    Err(Self::fail(step, s))
                }
            }
            _ => Err(Self::fail(step, s)),
        }
    }

    /// Write one block (`block` >= 2) at pointer + (block-2) * transfer size.
    pub fn download_block(&mut self, block: u16, data: &[u8]) -> Result<()> {
        let s = self.download_and_poll(block, data, "write")?;
        if s.state != state::DFU_DNLOAD_IDLE {
            return Err(Self::fail("write", s));
        }
        Ok(())
    }

    /// Read one block (`block` >= 2) from pointer + (block-2) * transfer size.
    pub fn upload_block(&mut self, block: u16, length: usize) -> Result<Vec<u8>> {
        self.transport.control_in(request::UPLOAD, block, length)
    }

    pub fn read_chip_info(&mut self) -> Result<ChipInfo> {
        let d = self.transport.memory_descriptors()?;
        log::info!("DFU memory descriptors: {d:?}");
        Ok(ChipInfo::from_descriptors(&d))
    }

    /// Leave DFU and jump to the firmware at `address` (zero-length DNLOAD + GETSTATUS).
    pub fn leave(&mut self, address: u32) -> Result<()> {
        self.clear_status(false)?;
        self.set_address(address)?;
        self.transport.control_out(request::DNLOAD, 0, &[])?;
        // The device usually resets during or right after this; errors are expected.
        let _ = self.get_status();
        Ok(())
    }

    /// Whether the chip is read protected (option-byte probe, as the configurator does).
    pub fn is_read_protected(&mut self) -> Result<bool> {
        let chip = self.read_chip_info()?;
        match chip.option_bytes().cloned() {
            Some(ob) => self.probe_read_protection(&ob),
            None => Ok(false),
        }
    }

    fn probe_read_protection(&mut self, option_bytes: &MemoryRegion) -> Result<bool> {
        self.clear_status(false)?;
        let s = self.try_set_address(option_bytes.start_address)?;
        let protected = if s.state == state::DFU_ERROR && s.status == status::ERR_VENDOR {
            true
        } else if s.state == state::DFU_DNLOAD_IDLE {
            self.clear_status(false)?;
            let len = option_bytes.total_size() as usize;
            let data = self.upload_block(2, len)?;
            let s = self.get_status()?;
            !(s.state == state::DFU_UPLOAD_IDLE && data.len() == len)
        } else {
            return Err(Self::fail("option bytes address", s));
        };
        self.clear_status(false)?;
        Ok(protected)
    }

    /// Remove read protection. **Mass-erases the chip** (firmware, settings, everything). Only
    /// call after the UI told the user so and got an explicit confirmation; without
    /// `confirm_erase` nothing is sent and `EraseNotConfirmed` is returned. On success the
    /// device resets: the user must unplug/replug (or re-enter DFU) and flash again.
    pub fn remove_read_protection(&mut self, confirm_erase: bool) -> Result<()> {
        if !confirm_erase {
            return Err(FlashError::EraseNotConfirmed);
        }
        self.clear_status(false)?;
        self.unprotect_unchecked()
    }

    /// Send READ_UNPROTECT. The chip mass-erases and resets; waits ~20 s like the configurator.
    fn unprotect_unchecked(&mut self) -> Result<()> {
        self.transport
            .control_out(request::DNLOAD, 0, &[cmd::READ_UNPROTECT])?;
        let s = self.get_status()?;
        if s.state != state::DFU_DNBUSY {
            return Err(Self::fail("read unprotect", s));
        }
        self.wait_ms(s.poll_timeout_ms + 20_000);
        // Expected to fail: the device resets.
        let _ = self.get_status();
        Ok(())
    }

    /// Full flash: detect layout, check fit, (read-protection probe), erase, write, verify, leave.
    pub fn flash(
        &mut self,
        image: &HexImage,
        opts: &DfuOptions,
        progress: &mut dyn FnMut(DfuProgress),
        cancel: &AtomicBool,
    ) -> Result<FlashReport> {
        let started = Instant::now();
        if image.segments.is_empty() || image.bytes_total() == 0 {
            return Err(FlashError::EmptyImage);
        }
        let check_cancel = || {
            if cancel.load(Ordering::Relaxed) {
                Err(FlashError::Cancelled)
            } else {
                Ok(())
            }
        };
        progress(DfuProgress {
            phase: DfuPhase::Detect,
            percent: 0.0,
            done: 0,
            total: 0,
        });

        let chip = self.read_chip_info()?;
        let region = chip.flash().cloned().ok_or(FlashError::NoFlashLayout)?;
        for seg in &image.segments {
            if !region.contains(seg.address, seg.data.len()) {
                return Err(FlashError::OutOfRange {
                    address: seg.address,
                    len: seg.data.len(),
                });
            }
        }
        self.transfer_size = match self.transport.transfer_size() {
            Ok(Some(n)) if n > 0 => n as usize,
            _ => DEFAULT_TRANSFER_SIZE,
        };
        self.clear_status(false)?;

        if opts.check_read_protection && region.key() == "internal_flash" {
            if let Some(ob) = chip.option_bytes().cloned() {
                if self.probe_read_protection(&ob)? {
                    return Err(FlashError::ReadProtected);
                }
            }
        }

        let w = Weights::new(opts);
        // Erase
        let mut pages: Vec<(u32, u32)> = match opts.erase {
            EraseMode::Full => region.all_pages(),
            EraseMode::Pages => image
                .segments
                .iter()
                .flat_map(|s| region.pages_overlapping(s.address, s.data.len()))
                .collect(),
        };
        pages.sort_unstable();
        pages.dedup();
        let total_pages = pages.len() as u64;
        for (i, (addr, _size)) in pages.iter().enumerate() {
            check_cancel()?;
            self.erase_page(*addr)?;
            let done = i as u64 + 1;
            progress(DfuProgress {
                phase: DfuPhase::Erase,
                percent: w.at(w.erase, done, total_pages),
                done,
                total: total_pages,
            });
        }

        // Write
        let total = image.bytes_total() as u64;
        let ts = self.transfer_size;
        let mut written = 0u64;
        for seg in &image.segments {
            check_cancel()?;
            self.set_address(seg.address)?;
            for (i, chunk) in seg.data.chunks(ts).enumerate() {
                check_cancel()?;
                self.download_block(block_number(i)?, chunk)?;
                written += chunk.len() as u64;
                progress(DfuProgress {
                    phase: DfuPhase::Write,
                    percent: w.at(w.write, written, total),
                    done: written,
                    total,
                });
            }
        }

        // Verify
        if opts.verify {
            let mut read = 0u64;
            for seg in &image.segments {
                check_cancel()?;
                self.clear_status(false)?;
                self.set_address(seg.address)?;
                self.clear_status(false)?;
                for (i, chunk) in seg.data.chunks(ts).enumerate() {
                    check_cancel()?;
                    let got = self.upload_block(block_number(i)?, chunk.len())?;
                    let base = seg.address + (i * ts) as u32;
                    if let Some(j) = (0..chunk.len()).find(|&j| got.get(j) != Some(&chunk[j])) {
                        return Err(FlashError::VerifyMismatch {
                            address: base + j as u32,
                            expected: chunk[j],
                            actual: got.get(j).copied().unwrap_or(0),
                        });
                    }
                    read += chunk.len() as u64;
                    progress(DfuProgress {
                        phase: DfuPhase::Verify,
                        percent: w.at(w.verify, read, total),
                        done: read,
                        total,
                    });
                }
            }
        }

        if opts.leave {
            progress(DfuProgress {
                phase: DfuPhase::Leave,
                percent: 100.0,
                done: 0,
                total: 0,
            });
            let start = image.base_address().unwrap_or(region.start_address);
            self.leave(start)?;
        }
        progress(DfuProgress {
            phase: DfuPhase::Done,
            percent: 100.0,
            done: total,
            total,
        });
        Ok(FlashReport {
            flash_region: region.name.clone(),
            flash_size: region.total_size(),
            transfer_size: ts,
            pages_erased: pages.len(),
            bytes_written: image.bytes_total(),
            verified: opts.verify,
            duration_ms: started.elapsed().as_millis() as u64,
        })
    }
}

fn block_number(i: usize) -> Result<u16> {
    u16::try_from(i + 2)
        .map_err(|_| FlashError::Usb("segment too large for DFU block numbering".into()))
}

/// Progress bar split (usbdfu.ts): full erase 0-33/33-66/66-100, page erase 0-10/10-50/50-100.
struct Weights {
    erase: (f32, f32),
    write: (f32, f32),
    verify: (f32, f32),
}

impl Weights {
    fn new(o: &DfuOptions) -> Self {
        let (e, w) = if o.erase == EraseMode::Full {
            (33.0, 66.0)
        } else {
            (10.0, 50.0)
        };
        if o.verify {
            Weights {
                erase: (0.0, e),
                write: (e, w),
                verify: (w, 100.0),
            }
        } else {
            Weights {
                erase: (0.0, e),
                write: (e, 100.0),
                verify: (100.0, 100.0),
            }
        }
    }
    fn at(&self, range: (f32, f32), done: u64, total: u64) -> f32 {
        if total == 0 {
            return range.1;
        }
        range.0 + (range.1 - range.0) * (done as f32 / total as f32)
    }
}

#[cfg(test)]
pub(crate) mod sim {
    //! A simulated STM32-style DfuSe bootloader.
    use super::*;
    use std::collections::BTreeMap;

    enum Pending {
        SetAddress(u32),
        Erase(Option<u32>),
        Unprotect,
        Write(u16, Vec<u8>),
    }

    pub struct SimDfu {
        pub descriptors: Vec<String>,
        pub transfer: Option<u16>,
        pub flash_base: u32,
        pub flash: Vec<u8>,
        pub read_protected: bool,
        /// Stay in dfuDNBUSY after the next erase, as the H743 Rev.V does.
        pub stuck_busy_after_erase: bool,
        /// Flip a bit of the byte written at this address.
        pub corrupt_write_at: Option<u32>,
        pub log: Vec<String>,
        pub left: bool,
        state: u8,
        status: u8,
        addr: u32,
        pending: Option<Pending>,
        stuck: bool,
        erased: BTreeMap<u32, ()>,
    }

    impl SimDfu {
        /// F405-like: 4x16K, 1x64K, 7x128K at 0x08000000, plus option bytes.
        pub fn f405() -> Self {
            SimDfu {
                descriptors: vec![
                    "@Internal Flash  /0x08000000/04*016Kg,01*064Kg,07*128Kg".into(),
                    "@Option Bytes  /0x1FFFC000/01*016 e".into(),
                ],
                transfer: Some(2048),
                flash_base: 0x0800_0000,
                flash: vec![0x00; 1024 * 1024], // not erased
                read_protected: false,
                stuck_busy_after_erase: false,
                corrupt_write_at: None,
                log: Vec::new(),
                left: false,
                state: state::DFU_IDLE,
                status: status::OK,
                addr: 0,
                pending: None,
                stuck: false,
                erased: BTreeMap::new(),
            }
        }

        pub fn erased_pages(&self) -> Vec<u32> {
            self.erased.keys().copied().collect()
        }

        fn page_len(&self, addr: u32) -> u32 {
            let off = addr - self.flash_base;
            match off {
                0..=0xFFFF => 16 * 1024,
                0x1_0000..=0x1_FFFF => 64 * 1024,
                _ => 128 * 1024,
            }
        }

        fn status_bytes(&self, poll: u32) -> Vec<u8> {
            vec![
                self.status,
                poll as u8,
                (poll >> 8) as u8,
                (poll >> 16) as u8,
                self.state,
                0,
            ]
        }

        fn error(&mut self, st: u8) {
            self.state = state::DFU_ERROR;
            self.status = st;
        }

        fn in_flash(&self, a: u32, len: usize) -> bool {
            a >= self.flash_base && (a - self.flash_base) as usize + len <= self.flash.len()
        }

        fn execute(&mut self, p: Pending) {
            match p {
                Pending::SetAddress(a) => {
                    if self.read_protected && a >= 0x1FFF_0000 {
                        self.error(status::ERR_VENDOR);
                    } else {
                        self.addr = a;
                        self.state = state::DFU_DNLOAD_IDLE;
                    }
                }
                Pending::Erase(Some(a)) => {
                    if !self.in_flash(a, 1) {
                        return self.error(status::ERR_ADDRESS);
                    }
                    let len = self.page_len(a);
                    let off = (a - self.flash_base) as usize;
                    self.flash[off..off + len as usize].fill(0xFF);
                    self.erased.insert(a, ());
                    if self.stuck_busy_after_erase {
                        self.stuck_busy_after_erase = false;
                        self.stuck = true;
                        self.state = state::DFU_DNBUSY;
                    } else {
                        self.state = state::DFU_DNLOAD_IDLE;
                    }
                }
                Pending::Erase(None) => {
                    self.flash.fill(0xFF);
                    self.state = state::DFU_DNLOAD_IDLE;
                }
                Pending::Unprotect => {
                    self.flash.fill(0xFF);
                    self.read_protected = false;
                    self.state = state::DFU_DNLOAD_IDLE;
                }
                Pending::Write(block, data) => {
                    let ts = self.transfer.unwrap_or(2048) as u32;
                    let a = self.addr + (block as u32 - 2) * ts;
                    if !self.in_flash(a, data.len()) {
                        return self.error(status::ERR_ADDRESS);
                    }
                    let off = (a - self.flash_base) as usize;
                    for (i, b) in data.iter().enumerate() {
                        if self.flash[off + i] != 0xFF {
                            return self.error(status::ERR_PROG); // not erased
                        }
                        let mut v = *b;
                        if self.corrupt_write_at == Some(a + i as u32) {
                            v ^= 0x01;
                        }
                        self.flash[off + i] = v;
                    }
                    self.state = state::DFU_DNLOAD_IDLE;
                }
            }
        }
    }

    impl DfuTransport for SimDfu {
        fn control_in(&mut self, req: u8, value: u16, length: usize) -> Result<Vec<u8>> {
            match req {
                request::GETSTATUS => {
                    self.log.push("GETSTATUS".into());
                    match self.state {
                        state::DFU_DNLOAD_SYNC => {
                            self.state = state::DFU_DNBUSY;
                            Ok(self.status_bytes(5))
                        }
                        state::DFU_DNBUSY if !self.stuck => {
                            if let Some(p) = self.pending.take() {
                                self.execute(p);
                            }
                            Ok(self.status_bytes(0))
                        }
                        _ => Ok(self.status_bytes(0)),
                    }
                }
                request::UPLOAD => {
                    self.log.push(format!("UPLOAD {value} {length}"));
                    if self.state != state::DFU_IDLE && self.state != state::DFU_UPLOAD_IDLE {
                        self.error(status::ERR_STALLEDPKT);
                        return Err(FlashError::Usb("stall".into()));
                    }
                    if self.read_protected {
                        self.error(status::ERR_VENDOR);
                        return Err(FlashError::Usb("stall".into()));
                    }
                    self.state = state::DFU_UPLOAD_IDLE;
                    let ts = self.transfer.unwrap_or(2048) as u32;
                    let a = self.addr + (value as u32 - 2) * ts;
                    if self.in_flash(a, length) {
                        let off = (a - self.flash_base) as usize;
                        Ok(self.flash[off..off + length].to_vec())
                    } else {
                        Ok(vec![0xAA; length]) // option bytes etc.
                    }
                }
                _ => Err(FlashError::Usb(format!("unexpected IN request {req}"))),
            }
        }

        fn control_out(&mut self, req: u8, value: u16, data: &[u8]) -> Result<()> {
            match req {
                request::DNLOAD => {
                    let hdr: Vec<String> =
                        data.iter().take(5).map(|b| format!("{b:02x}")).collect();
                    self.log.push(format!(
                        "DNLOAD {value} [{}] len={}",
                        hdr.join(" "),
                        data.len()
                    ));
                    if self.state != state::DFU_IDLE && self.state != state::DFU_DNLOAD_IDLE {
                        self.error(status::ERR_STALLEDPKT);
                        return Err(FlashError::Usb("stall".into()));
                    }
                    if data.is_empty() {
                        self.left = true;
                        self.state = state::DFU_MANIFEST;
                        return Ok(());
                    }
                    let p = if value == 0 {
                        let a = (data.len() >= 5)
                            .then(|| u32::from_le_bytes([data[1], data[2], data[3], data[4]]));
                        match data[0] {
                            cmd::SET_ADDRESS => Pending::SetAddress(a.unwrap_or(0)),
                            cmd::ERASE => Pending::Erase(a),
                            cmd::READ_UNPROTECT => Pending::Unprotect,
                            _ => return Err(FlashError::Usb("bad command".into())),
                        }
                    } else {
                        Pending::Write(value, data.to_vec())
                    };
                    self.pending = Some(p);
                    self.state = state::DFU_DNLOAD_SYNC;
                    Ok(())
                }
                request::CLRSTATUS => {
                    self.log.push("CLRSTATUS".into());
                    if self.stuck {
                        // First CLRSTATUS: errUNKNOWN/dfuERROR, second clears (STM32CubeProgrammer trick).
                        self.stuck = false;
                        self.error(status::ERR_UNKNOWN);
                        return Ok(());
                    }
                    if self.state != state::DFU_ERROR {
                        // Strict bootloader (STM32C5-like): CLRSTATUS outside dfuERROR stalls.
                        return Err(FlashError::Usb("stall: CLRSTATUS outside dfuERROR".into()));
                    }
                    self.state = state::DFU_IDLE;
                    self.status = status::OK;
                    Ok(())
                }
                request::ABORT => {
                    self.log.push("ABORT".into());
                    self.state = state::DFU_IDLE;
                    Ok(())
                }
                _ => Err(FlashError::Usb(format!("unexpected OUT request {req}"))),
            }
        }

        fn memory_descriptors(&mut self) -> Result<Vec<String>> {
            Ok(self.descriptors.clone())
        }

        fn transfer_size(&mut self) -> Result<Option<u16>> {
            Ok(self.transfer)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::sim::SimDfu;
    use super::*;
    use crate::flasher::hex::Segment;

    fn image() -> HexImage {
        HexImage {
            segments: vec![
                Segment {
                    address: 0x0800_0000,
                    data: (0..5000u32).map(|i| (i % 251) as u8).collect(),
                },
                Segment {
                    address: 0x0801_0000,
                    data: vec![0x5A; 100],
                },
            ],
            start_linear_address: None,
        }
    }

    fn run(sim: SimDfu, opts: DfuOptions) -> (Result<FlashReport>, SimDfu, Vec<DfuProgress>) {
        let mut dfu = DfuSe::new(sim).with_sleep(|_| {});
        let mut events = Vec::new();
        let cancel = AtomicBool::new(false);
        let r = dfu.flash(&image(), &opts, &mut |p| events.push(p), &cancel);
        (r, dfu.into_transport(), events)
    }

    fn assert_flashed(sim: &SimDfu) {
        let img = image();
        for seg in &img.segments {
            let off = (seg.address - sim.flash_base) as usize;
            assert_eq!(&sim.flash[off..off + seg.data.len()], seg.data.as_slice());
        }
    }

    #[test]
    fn flashes_verifies_and_leaves() {
        let (r, sim, events) = run(SimDfu::f405(), DfuOptions::default());
        let report = r.unwrap();
        assert_flashed(&sim);
        assert!(sim.left);
        assert_eq!(sim.erased_pages(), vec![0x0800_0000, 0x0801_0000]);
        assert_eq!(report.pages_erased, 2);
        assert_eq!(report.bytes_written, 5100);
        assert_eq!(report.transfer_size, 2048);
        assert!(report.verified);
        // Progress is monotonic and ends at 100 / Done.
        assert!(events.windows(2).all(|w| w[1].percent >= w[0].percent));
        assert_eq!(events.last().unwrap().phase, DfuPhase::Done);
        assert_eq!(events.last().unwrap().percent, 100.0);
    }

    #[test]
    fn dfu_command_sequence() {
        let (r, sim, _) = run(SimDfu::f405(), DfuOptions::default());
        r.unwrap();
        let dn: Vec<&str> = sim
            .log
            .iter()
            .filter(|l| l.starts_with("DNLOAD") || l.starts_with("UPLOAD"))
            .map(String::as_str)
            .collect();
        assert_eq!(
            dn,
            vec![
                "DNLOAD 0 [21 00 c0 ff 1f] len=5", // option bytes address (read-protection probe)
                "UPLOAD 2 16",                     // read option bytes
                "DNLOAD 0 [41 00 00 00 08] len=5", // erase 0x08000000
                "DNLOAD 0 [41 00 00 01 08] len=5", // erase 0x08010000
                "DNLOAD 0 [21 00 00 00 08] len=5", // set address seg 0
                "DNLOAD 2 [00 01 02 03 04] len=2048",
                "DNLOAD 3 [28 29 2a 2b 2c] len=2048",
                "DNLOAD 4 [50 51 52 53 54] len=904",
                "DNLOAD 0 [21 00 00 01 08] len=5", // set address seg 1
                "DNLOAD 2 [5a 5a 5a 5a 5a] len=100",
                "DNLOAD 0 [21 00 00 00 08] len=5", // verify seg 0
                "UPLOAD 2 2048",
                "UPLOAD 3 2048",
                "UPLOAD 4 904",
                "DNLOAD 0 [21 00 00 01 08] len=5", // verify seg 1
                "UPLOAD 2 100",
                "DNLOAD 0 [21 00 00 00 08] len=5", // leave: address
                "DNLOAD 0 [] len=0",               // leave: zero-length download
            ]
        );
        // The simulator stalls CLRSTATUS outside dfuERROR, so passing proves ABORT was used for
        // dnload/upload-idle (the STM32C5 fix).
        assert!(sim.log.iter().any(|l| l == "ABORT"));
    }

    #[test]
    fn full_erase_erases_every_page() {
        let opts = DfuOptions {
            erase: EraseMode::Full,
            ..Default::default()
        };
        let (r, sim, _) = run(SimDfu::f405(), opts);
        assert_eq!(r.unwrap().pages_erased, 12);
        assert_eq!(sim.erased_pages().len(), 12);
        assert_flashed(&sim);
    }

    #[test]
    fn read_protected_stops_before_erase() {
        let mut sim = SimDfu::f405();
        sim.read_protected = true;
        let (r, sim, _) = run(sim, DfuOptions::default());
        assert!(matches!(r, Err(FlashError::ReadProtected)));
        assert!(sim.erased_pages().is_empty());
        assert!(!sim.log.iter().any(|l| l.starts_with("DNLOAD 0 [41")));
    }

    #[test]
    fn unprotect_only_with_explicit_confirm() {
        let mut sim = SimDfu::f405();
        sim.read_protected = true;
        let mut dfu = DfuSe::new(sim).with_sleep(|_| {});
        assert!(dfu.is_read_protected().unwrap());
        // No confirm: refused, nothing sent.
        assert!(matches!(
            dfu.remove_read_protection(false),
            Err(FlashError::EraseNotConfirmed)
        ));
        assert!(!dfu
            .transport_mut()
            .log
            .iter()
            .any(|l| l.starts_with("DNLOAD 0 [92]")));
        dfu.remove_read_protection(true).unwrap();
        let sim = dfu.into_transport();
        assert!(sim.log.iter().any(|l| l.starts_with("DNLOAD 0 [92]")));
        assert!(!sim.read_protected);
    }

    #[test]
    fn rejects_image_outside_flash() {
        let mut dfu = DfuSe::new(SimDfu::f405()).with_sleep(|_| {});
        let img = HexImage::from_bin(0x0810_0000 - 4, vec![1; 8]);
        let r = dfu.flash(
            &img,
            &DfuOptions::default(),
            &mut |_| {},
            &AtomicBool::new(false),
        );
        assert!(matches!(
            r,
            Err(FlashError::OutOfRange {
                address: 0x080F_FFFC,
                len: 8
            })
        ));
        assert!(!dfu
            .into_transport()
            .log
            .iter()
            .any(|l| l.starts_with("DNLOAD")));
    }

    #[test]
    fn verify_mismatch_reports_address() {
        let mut sim = SimDfu::f405();
        sim.corrupt_write_at = Some(0x0800_0801);
        let (r, _, _) = run(sim, DfuOptions::default());
        match r {
            Err(FlashError::VerifyMismatch {
                address,
                expected,
                actual,
            }) => {
                assert_eq!(address, 0x0800_0801);
                assert_eq!(actual, expected ^ 1);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn handles_h7_stuck_busy_after_erase() {
        let mut sim = SimDfu::f405();
        sim.stuck_busy_after_erase = true;
        let (r, sim, _) = run(sim, DfuOptions::default());
        r.unwrap();
        assert_flashed(&sim);
        assert!(sim.log.iter().filter(|l| *l == "CLRSTATUS").count() >= 2);
    }

    #[test]
    fn cancel_stops_flash() {
        let mut dfu = DfuSe::new(SimDfu::f405()).with_sleep(|_| {});
        let cancel = AtomicBool::new(false);
        let r = dfu.flash(
            &image(),
            &DfuOptions::default(),
            &mut |p| {
                if p.phase == DfuPhase::Write {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
            &cancel,
        );
        assert!(matches!(r, Err(FlashError::Cancelled)));
        assert!(!dfu.into_transport().left);
    }

    #[test]
    fn default_transfer_size_and_no_option_bytes() {
        let mut sim = SimDfu::f405();
        sim.transfer = None; // functional descriptor missing -> 2048
        sim.descriptors.truncate(1); // no option bytes alt setting (STM32C5 ROM)
        let (r, sim, _) = run(
            sim,
            DfuOptions {
                verify: false,
                ..Default::default()
            },
        );
        let rep = r.unwrap();
        assert_eq!(rep.transfer_size, 2048);
        assert!(!rep.verified);
        assert!(!sim.log.iter().any(|l| l.starts_with("UPLOAD")));
    }

    #[test]
    fn status_parse() {
        let s = DfuStatus::parse(&[0x0B, 0x10, 0x27, 0x00, 10, 0]).unwrap();
        assert_eq!(s.status, status::ERR_VENDOR);
        assert_eq!(s.poll_timeout_ms, 10_000);
        assert_eq!(s.state, state::DFU_ERROR);
        assert!(DfuStatus::parse(&[0; 3]).is_err());
    }
}
