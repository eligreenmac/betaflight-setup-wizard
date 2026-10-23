// SPDX-License-Identifier: GPL-3.0-or-later
//! [`DfuTransport`] over libusb (rusb). Mirrors what Betaflight Configurator's
//! `WebUsbDfuTransport.ts` does with WebUSB: claim interface 0, class/interface control
//! transfers, alt-setting strings as memory layouts, wTransferSize from the functional
//! descriptor.

use crate::device::usb_ids;
use crate::flasher::dfu::DfuTransport;
use crate::flasher::error::{open_error, FlashError, Result};
use crate::flasher::layout::ChipInfo;
use crate::model::DfuDevice;
use rusb::{Direction, GlobalContext, Recipient, RequestType, UsbContext};
use std::time::Duration;

const DFU_CLASS: u8 = 0xFE;
const DFU_SUBCLASS: u8 = 0x01;
const DFU_FUNCTIONAL_DESCRIPTOR: u8 = 0x21;

pub struct RusbDfuTransport {
    handle: rusb::DeviceHandle<GlobalContext>,
    interface: u8,
    pub timeout: Duration,
}

impl RusbDfuTransport {
    pub fn open_device(dev: rusb::Device<GlobalContext>) -> Result<Self> {
        let config = dev
            .active_config_descriptor()
            .or_else(|_| dev.config_descriptor(0))?;
        let interface = config
            .interfaces()
            .find(|i| {
                i.descriptors()
                    .any(|d| d.class_code() == DFU_CLASS && d.sub_class_code() == DFU_SUBCLASS)
            })
            .map(|i| i.number())
            .unwrap_or(0);
        let desc = dev.device_descriptor()?;
        let (vid, pid) = (desc.vendor_id(), desc.product_id());
        let mut handle = dev.open().map_err(|e| open_error(e, vid, pid))?;
        #[cfg(not(windows))]
        {
            // Linux: let libusb detach/reattach any kernel driver. Not supported on Windows.
            let _ = handle.set_auto_detach_kernel_driver(true);
        }
        claim_with_retry(&mut handle, interface).map_err(|e| open_error(e, vid, pid))?;
        Ok(RusbDfuTransport {
            handle,
            interface,
            timeout: Duration::from_secs(5),
        })
    }

    fn req_type(dir: Direction) -> u8 {
        rusb::request_type(dir, RequestType::Class, Recipient::Interface)
    }

    /// Reset the device (used after leaving DFU on some bootloaders).
    pub fn reset(&mut self) -> Result<()> {
        Ok(self.handle.reset()?)
    }
}

/// The OS can hold the interface briefly after re-enumeration (usbdfu.ts retries 6x / 1 s).
fn claim_with_retry(
    handle: &mut rusb::DeviceHandle<GlobalContext>,
    interface: u8,
) -> Result<(), rusb::Error> {
    let mut last = None;
    for _ in 0..6 {
        match handle.claim_interface(interface) {
            Ok(()) => return Ok(()),
            Err(rusb::Error::Busy) => {
                last = Some(rusb::Error::Busy);
                std::thread::sleep(Duration::from_secs(1));
            }
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or(rusb::Error::Busy))
}

impl Drop for RusbDfuTransport {
    fn drop(&mut self) {
        let _ = self.handle.release_interface(self.interface);
    }
}

impl DfuTransport for RusbDfuTransport {
    fn control_in(&mut self, request: u8, value: u16, length: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; length];
        let n = self.handle.read_control(
            Self::req_type(Direction::In),
            request,
            value,
            self.interface as u16,
            &mut buf,
            self.timeout,
        )?;
        buf.truncate(n);
        Ok(buf)
    }

    fn control_out(&mut self, request: u8, value: u16, data: &[u8]) -> Result<()> {
        self.handle.write_control(
            Self::req_type(Direction::Out),
            request,
            value,
            self.interface as u16,
            data,
            self.timeout,
        )?;
        Ok(())
    }

    fn memory_descriptors(&mut self) -> Result<Vec<String>> {
        let dev = self.handle.device();
        let config = dev.active_config_descriptor()?;
        let mut out = Vec::new();
        for iface in config.interfaces().filter(|i| i.number() == self.interface) {
            for alt in iface.descriptors() {
                if let Some(idx) = alt.description_string_index() {
                    // ASCII read is what the bootloaders emit; fall back to raw on failure.
                    match self.handle.read_string_descriptor_ascii(idx) {
                        Ok(s) => out.push(s),
                        Err(e) => log::warn!("cannot read alt setting string {idx}: {e}"),
                    }
                }
            }
        }
        Ok(out)
    }

    fn transfer_size(&mut self) -> Result<Option<u16>> {
        let dev = self.handle.device();
        let config = dev.active_config_descriptor()?;
        for iface in config.interfaces().filter(|i| i.number() == self.interface) {
            for alt in iface.descriptors() {
                if let Some(ts) = parse_functional_descriptor(alt.extra()) {
                    return Ok(Some(ts));
                }
            }
        }
        Ok(None)
    }
}

/// wTransferSize from the DFU functional descriptor in an interface's extra bytes.
pub fn parse_functional_descriptor(extra: &[u8]) -> Option<u16> {
    let mut i = 0;
    while i + 2 <= extra.len() {
        let len = extra[i] as usize;
        if len < 2 || i + len > extra.len() {
            return None;
        }
        if extra[i + 1] == DFU_FUNCTIONAL_DESCRIPTOR && len >= 7 {
            return Some(u16::from_le_bytes([extra[i + 5], extra[i + 6]]));
        }
        i += len;
    }
    None
}

/// Whether a device exposes a DFU interface (or is a known DFU VID:PID; on Windows without
/// a driver the configuration descriptor may be unreadable).
fn is_dfu_device<C: UsbContext>(dev: &rusb::Device<C>, vid: u16, pid: u16) -> bool {
    if usb_ids::is_dfu(vid, pid) {
        return true;
    }
    dev.active_config_descriptor()
        .or_else(|_| dev.config_descriptor(0))
        .map(|c| {
            c.interfaces().any(|i| {
                i.descriptors()
                    .any(|d| d.class_code() == DFU_CLASS && d.sub_class_code() == DFU_SUBCLASS)
            })
        })
        .unwrap_or(false)
}

/// Read the alt-setting strings without claiming the interface.
fn alt_strings(handle: &rusb::DeviceHandle<GlobalContext>) -> Vec<String> {
    let dev = handle.device();
    let Ok(config) = dev
        .active_config_descriptor()
        .or_else(|_| dev.config_descriptor(0))
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for iface in config.interfaces() {
        for alt in iface.descriptors() {
            if alt.class_code() != DFU_CLASS || alt.sub_class_code() != DFU_SUBCLASS {
                continue;
            }
            if let Some(idx) = alt.description_string_index() {
                if let Ok(s) = handle.read_string_descriptor_ascii(idx) {
                    out.push(s);
                }
            }
        }
    }
    out
}

/// MCU family hint from the vendor and the DfuSe memory layouts. STM32 families are told
/// apart by the option-byte address (F4 0x1FFFC000, F7 0x1FFF0000, H7 0x5200xxxx,
/// G4 0x1FFF78xx); Betaflight only supports AT32F435/437 on AT32.
pub fn mcu_family_hint(vid: u16, descriptors: &[String]) -> Option<String> {
    if vid == 0x2E3C {
        return Some("AT32F435".into());
    }
    let chip = ChipInfo::from_descriptors(descriptors);
    let ob = chip.option_bytes()?.start_address;
    let family = match ob {
        0x1FFF_C000 => "STM32F4",
        0x1FFF_0000 => "STM32F7",
        0x5200_0000..=0x5200_FFFF => "STM32H7",
        0x1FFF_7800..=0x1FFF_78FF => "STM32G4",
        _ => return None,
    };
    Some(family.into())
}

/// One enumerated DFU device and the libusb handle to reopen it.
pub struct FoundDfu {
    pub device: rusb::Device<GlobalContext>,
    pub info: DfuDevice,
}

/// All USB devices in DFU mode. A device we can see but not open on Windows is listed with
/// `driver_ok = false` (no serial, no layout) so the UI can offer the driver installer.
pub fn list_dfu() -> Result<Vec<FoundDfu>> {
    let mut out = Vec::new();
    for dev in rusb::devices()?.iter() {
        let Ok(desc) = dev.device_descriptor() else {
            continue;
        };
        let (vid, pid) = (desc.vendor_id(), desc.product_id());
        if !is_dfu_device(&dev, vid, pid) {
            continue;
        }
        let info = match dev.open() {
            Ok(handle) => {
                let serial = handle
                    .read_serial_number_string_ascii(&desc)
                    .ok()
                    .filter(|s| !s.is_empty());
                let descriptors = alt_strings(&handle);
                DfuDevice {
                    vid,
                    pid,
                    serial,
                    mcu_family: mcu_family_hint(vid, &descriptors),
                    flash_descriptor: descriptors.into_iter().find(|d| d.contains("Flash")),
                    driver_ok: true,
                }
            }
            Err(e) => {
                let driver_ok =
                    !matches!(open_error(e, vid, pid), FlashError::DriverMissing { .. });
                log::warn!("cannot open DFU device {vid:04x}:{pid:04x}: {e}");
                DfuDevice {
                    vid,
                    pid,
                    serial: None,
                    mcu_family: None,
                    flash_descriptor: None,
                    driver_ok,
                }
            }
        };
        out.push(FoundDfu { device: dev, info });
    }
    Ok(out)
}

/// Choose the DFU device with serial `serial`, or the only one present.
pub fn select(mut found: Vec<FoundDfu>, serial: Option<&str>) -> Result<FoundDfu> {
    match serial {
        Some(s) => {
            let i = found
                .iter()
                .position(|f| f.info.serial.as_deref() == Some(s))
                .ok_or_else(|| FlashError::DfuNotFound(format!("no DFU device with serial {s}")))?;
            Ok(found.swap_remove(i))
        }
        None => match found.len() {
            0 => Err(FlashError::DfuNotFound("no device in DFU mode".into())),
            1 => Ok(found.remove(0)),
            n => Err(FlashError::DfuNotFound(format!(
                "{n} DFU devices present; pass dfuSerial to pick one"
            ))),
        },
    }
}

/// Open the selected device for flashing. A device without a driver fails with
/// `DriverMissing {vid, pid}` before anything is sent.
pub fn open_selected(f: FoundDfu) -> Result<RusbDfuTransport> {
    if !f.info.driver_ok {
        return Err(FlashError::DriverMissing {
            vid: f.info.vid,
            pid: f.info.pid,
            detail: "no WinUSB driver bound".into(),
        });
    }
    RusbDfuTransport::open_device(f.device)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn functional_descriptor() {
        // bLength 9, type 0x21, attrs 0x0B, detach 255, transfer 2048, bcdDFU 0x011A
        let d = [9, 0x21, 0x0B, 0xFF, 0x00, 0x00, 0x08, 0x1A, 0x01];
        assert_eq!(parse_functional_descriptor(&d), Some(2048));
        let with_other = [
            3, 0x24, 0, 9, 0x21, 0x0B, 0xFF, 0x00, 0x00, 0x04, 0x1A, 0x01,
        ];
        assert_eq!(parse_functional_descriptor(&with_other), Some(1024));
        assert_eq!(parse_functional_descriptor(&[]), None);
        assert_eq!(parse_functional_descriptor(&[0, 0x21]), None);
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn family_from_option_bytes() {
        let f405 = s(&[
            "@Internal Flash  /0x08000000/04*016Kg,01*064Kg,07*128Kg",
            "@Option Bytes  /0x1FFFC000/01*016 e",
        ]);
        assert_eq!(mcu_family_hint(0x0483, &f405).as_deref(), Some("STM32F4"));
        let f722 = s(&[
            "@Internal Flash  /0x08000000/04*016Kg,01*064Kg,03*128Kg",
            "@Option Bytes  /0x1FFF0000/01*032 e",
        ]);
        assert_eq!(mcu_family_hint(0x0483, &f722).as_deref(), Some("STM32F7"));
        let h743 = s(&[
            "@Internal Flash   /0x08000000/16*128Kg",
            "@Option Bytes   /0x5200201C/01*128 e",
        ]);
        assert_eq!(mcu_family_hint(0x0483, &h743).as_deref(), Some("STM32H7"));
        assert_eq!(mcu_family_hint(0x2E3C, &[]).as_deref(), Some("AT32F435"));
        assert_eq!(
            mcu_family_hint(0x0483, &s(&["@Internal Flash  /0x08000000/04*016Kg"])),
            None
        );
    }

    #[test]
    fn select_by_serial_or_single() {
        assert!(matches!(
            select(Vec::new(), None),
            Err(FlashError::DfuNotFound(_))
        ));
        assert!(matches!(
            select(Vec::new(), Some("X")),
            Err(FlashError::DfuNotFound(_))
        ));
    }
}
