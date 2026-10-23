//! Platform-independent driver evaluation: VID/PID tables for the devices the
//! wizard cares about, official vendor driver links, and the pure function that
//! turns "what Windows reports" into a [`DriverStatus`]. Unit-tested on any OS.

use crate::model::{DriverAction, DriverGuidance, DriverStatus, UsbDeviceKind, UsbDriverInfo};

/// MCU ROM DFU bootloaders that need WinUSB for flashing (same list as the
/// Betaflight App's `usbDevices.dfu`, src/js/protocols/devices.ts).
/// MVP flashes STM32 + AT32; the others get WinUSB too so they are ready later.
pub const DFU_IDS: &[(u16, u16, &str)] = &[
    (0x0483, 0xDF11, "STM32 DFU"),
    (0x2E3C, 0xDF11, "AT32 DFU"),
    (0x314B, 0x0106, "APM32 DFU"),
    (0x28E9, 0x0189, "GD32 DFU"),
    (0x3797, 0xDF11, "X32 DFU"),
];

/// Native MCU USB virtual COM ports (Windows 10+ binds inbox `usbser`).
pub const VCP_IDS: &[(u16, u16, &str)] = &[
    (0x0483, 0x5740, "STM32 VCP"),
    (0x2E3C, 0x5740, "AT32 VCP"),
    (0x314B, 0x5740, "APM32 VCP"),
    (0x28E9, 0x018A, "GD32 VCP"),
    (0x2E8A, 0x0009, "RP2040/RP2350 VCP"),
    (0x3797, 0x5740, "X32 VCP"),
];

/// USB-UART bridges: (vid, pid, name, index into [`guidance()`]).
/// Vendor drivers are NOT redistributable by us -> link to the official page.
pub const BRIDGE_IDS: &[(u16, u16, &str, usize)] = &[
    (0x10C4, 0xEA60, "CP210x", 0),
    (0x10C4, 0xEA61, "CP210x", 0),
    (0x10C4, 0xEA62, "CP210x", 0),
    (0x10C4, 0xEA70, "CP2105", 0),
    (0x10C4, 0xEA71, "CP2108", 0),
    (0x1A86, 0x7522, "CH340", 1),
    (0x1A86, 0x7523, "CH340", 1),
    (0x1A86, 0x7584, "CH340S", 1),
    (0x1A86, 0x5523, "CH341", 1),
    (0x1A86, 0x55D3, "CH343", 2),
    (0x1A86, 0x55D4, "CH9102", 2),
];

pub const SILABS_CP210X_URL: &str =
    "https://www.silabs.com/software-and-tools/usb-to-uart-bridge-vcp-drivers";
pub const WCH_CH341SER_URL: &str = "https://www.wch-ic.com/downloads/CH341SER_EXE.html";
pub const WCH_CH343SER_URL: &str = "https://www.wch-ic.com/downloads/CH343SER_EXE.html";

/// Official download pages, in the order referenced by [`BRIDGE_IDS`].
pub fn guidance() -> Vec<DriverGuidance> {
    let ids = |g: usize| {
        BRIDGE_IDS
            .iter()
            .filter(|b| b.3 == g)
            .map(|b| format!("{:04X}:{:04X}", b.0, b.1))
            .collect::<Vec<_>>()
    };
    vec![
        DriverGuidance {
            id: "CP210X".into(),
            vendor: "Silicon Labs".into(),
            chips: "CP210x (CP2102, CP2102N, CP2104, CP2105, CP2108, CP2109)".into(),
            usb_ids: ids(0),
            download_url: SILABS_CP210X_URL.into(),
        },
        DriverGuidance {
            id: "CH34X".into(),
            vendor: "WCH (Nanjing Qinheng)".into(),
            chips: "CH340 / CH341 (driver CH341SER)".into(),
            usb_ids: ids(1),
            download_url: WCH_CH341SER_URL.into(),
        },
        DriverGuidance {
            id: "CH343".into(),
            vendor: "WCH (Nanjing Qinheng)".into(),
            chips: "CH343 / CH9102 (driver CH343SER)".into(),
            usb_ids: ids(2),
            download_url: WCH_CH343SER_URL.into(),
        },
    ]
}

/// One USB device node as reported by the OS (SetupAPI on Windows).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawUsbDevice {
    pub vid: u16,
    pub pid: u16,
    pub instance_id: String,
    /// Bound driver service (SPDRP_SERVICE), `None` if no driver is installed.
    pub service: Option<String>,
    /// Device Manager problem code (CM_PROB_*), 0 = working.
    pub problem: u32,
}

/// Parse `VID_xxxx` / `PID_xxxx` out of a Windows device instance id such as
/// `USB\VID_0483&PID_DF11\205F32...` or `USB\VID_10C4&PID_EA70&MI_01\...`.
pub fn parse_vid_pid(instance_id: &str) -> Option<(u16, u16)> {
    let up = instance_id.to_ascii_uppercase();
    let hex_after = |tag: &str| -> Option<u16> {
        let i = up.find(tag)? + tag.len();
        let digits = up.get(i..i + 4)?;
        u16::from_str_radix(digits, 16).ok()
    };
    Some((hex_after("VID_")?, hex_after("PID_")?))
}

/// Classify a VID/PID: kind, friendly name, guidance index (bridges only).
pub fn classify(vid: u16, pid: u16) -> Option<(UsbDeviceKind, &'static str, Option<usize>)> {
    if let Some(d) = DFU_IDS.iter().find(|d| d.0 == vid && d.1 == pid) {
        return Some((UsbDeviceKind::Dfu, d.2, None));
    }
    if let Some(d) = VCP_IDS.iter().find(|d| d.0 == vid && d.1 == pid) {
        return Some((UsbDeviceKind::FcVcp, d.2, None));
    }
    BRIDGE_IDS
        .iter()
        .find(|d| d.0 == vid && d.1 == pid)
        .map(|d| (UsbDeviceKind::UsbUartBridge, d.2, Some(d.3)))
}

fn is_service(s: &Option<String>, name: &str) -> bool {
    s.as_deref().is_some_and(|v| v.eq_ignore_ascii_case(name))
}

/// Pure: evaluate the OS device list into the `DriverStatus` the UI consumes.
pub fn evaluate(raw: &[RawUsbDevice]) -> DriverStatus {
    let mut devices = Vec::new();
    let mut notes = Vec::new();
    for r in raw {
        let Some((kind, name, guidance_index)) = classify(r.vid, r.pid) else {
            continue;
        };
        // Composite parent node (e.g. CP2105 dual UART): the interfaces
        // (`&MI_xx`) carry the real driver, so skip the usbccgp parent.
        if is_service(&r.service, "usbccgp") {
            continue;
        }
        let bound = r.service.is_some();
        let (driver_ok, action) = match kind {
            UsbDeviceKind::Dfu => {
                if is_service(&r.service, "WinUSB") && r.problem == 0 {
                    (true, DriverAction::None)
                } else if is_service(&r.service, "WinUSB") {
                    (false, DriverAction::Replug)
                } else {
                    // No driver, or another one (e.g. ST "STTub30"/DfuSe,
                    // libusb0): libwdi rebinds it to WinUSB.
                    (false, DriverAction::InstallDfuDriver)
                }
            }
            UsbDeviceKind::FcVcp => {
                if bound && r.problem == 0 {
                    (true, DriverAction::None)
                } else {
                    (false, DriverAction::Replug)
                }
            }
            UsbDeviceKind::UsbUartBridge => {
                if bound && r.problem == 0 {
                    (true, DriverAction::None)
                } else {
                    (false, DriverAction::VendorDownload)
                }
            }
        };
        if !driver_ok {
            notes.push(format!(
                "{name} ({:04X}:{:04X}) {}: service={}, problem={}",
                r.vid,
                r.pid,
                r.instance_id,
                r.service.as_deref().unwrap_or("<none>"),
                r.problem
            ));
        }
        devices.push(UsbDriverInfo {
            vid: r.vid,
            pid: r.pid,
            name: name.to_string(),
            kind,
            instance_id: Some(r.instance_id.clone()),
            service: r.service.clone(),
            driver_ok,
            action,
            guidance_index: match action {
                DriverAction::VendorDownload => guidance_index.map(|g| g as u32),
                _ => None,
            },
        });
    }
    let dfu_winusb = devices
        .iter()
        .filter(|d| d.kind == UsbDeviceKind::Dfu)
        .all(|d| d.driver_ok);
    let vcp = devices
        .iter()
        .filter(|d| d.kind != UsbDeviceKind::Dfu)
        .all(|d| d.driver_ok);
    DriverStatus {
        dfu_winusb,
        vcp,
        notes,
        devices,
        guidance: guidance(),
    }
}

/// Distinct (vid, pid) of DFU devices that need `system_install_dfu_driver`.
pub fn dfu_ids_needing_install(status: &DriverStatus) -> Vec<(u16, u16, String)> {
    let mut out: Vec<(u16, u16, String)> = Vec::new();
    for d in &status.devices {
        if d.kind == UsbDeviceKind::Dfu
            && d.action == DriverAction::InstallDfuDriver
            && !out.iter().any(|o| o.0 == d.vid && o.1 == d.pid)
        {
            out.push((d.vid, d.pid, d.name.clone()));
        }
    }
    out
}

/// Arguments for libwdi's `wdi-simple` (shipped as `bfsw-wdi-simple.exe`):
/// WinUSB (`-t 0`), silent, wait up to 2 min for pending installs, no logs.
pub fn wdi_simple_args(vid: u16, pid: u16, name: &str, dest: &str) -> Vec<String> {
    vec![
        "--name".into(),
        format!("{name} (Betaflight Setup Wizard)"),
        "--inf".into(),
        format!("bfsw_{vid:04x}_{pid:04x}.inf"),
        "--manufacturer".into(),
        "Betaflight Setup Wizard".into(),
        "--vid".into(),
        format!("0x{vid:04X}"),
        "--pid".into(),
        format!("0x{pid:04X}"),
        "--type".into(),
        "0".into(),
        "--dest".into(),
        dest.into(),
        "--silent".into(),
        "--timeout".into(),
        "120000".into(),
    ]
}

/// Quote one argument for a Windows command line (CommandLineToArgvW rules).
pub fn quote_windows_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.chars().any(|c| c == ' ' || c == '\t' || c == '"') {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

/// Human message for a libwdi `wdi_error` exit code.
pub fn wdi_error_message(code: i32) -> &'static str {
    match code {
        0 => "success",
        -1 => "input/output error",
        -2 => "invalid parameter",
        -3 => "access denied (insufficient permissions)",
        -4 => "no such device (it may have been disconnected)",
        -5 => "entity not found",
        -6 => "resource busy, or API call already running",
        -7 => "operation timed out",
        -8 => "overflow",
        -9 => "another installation is pending",
        -10 => "system call interrupted",
        -11 => "could not acquire resource (insufficient memory, etc.)",
        -12 => "operation not supported or unimplemented on this platform",
        -13 => "entity already exists",
        -14 => "cancelled by user",
        -15 => "couldn't run installer with required privileges",
        -16 => "attempted to run the 32 bit installer on 64 bit",
        -17 => "bad inf syntax",
        -18 => "missing cat file",
        -19 => "system policy prevents the installation of unsigned drivers",
        _ => "other error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(id: &str, service: Option<&str>, problem: u32) -> RawUsbDevice {
        let (vid, pid) = parse_vid_pid(id).expect("vid/pid");
        RawUsbDevice {
            vid,
            pid,
            instance_id: id.into(),
            service: service.map(Into::into),
            problem,
        }
    }

    #[test]
    fn parses_instance_ids() {
        assert_eq!(
            parse_vid_pid(r"USB\VID_0483&PID_DF11\205F32A35430"),
            Some((0x0483, 0xDF11))
        );
        assert_eq!(
            parse_vid_pid(r"usb\vid_10c4&pid_ea70&mi_01\6&1"),
            Some((0x10C4, 0xEA70))
        );
        assert_eq!(parse_vid_pid(r"USB\ROOT_HUB30\4&1"), None);
        assert_eq!(parse_vid_pid(r"USB\VID_04"), None);
    }

    #[test]
    fn empty_system_is_all_ok_with_guidance() {
        let s = evaluate(&[]);
        assert!(s.dfu_winusb && s.vcp);
        assert!(s.devices.is_empty());
        assert_eq!(s.guidance.len(), 3);
        assert!(s.guidance[0].usb_ids.contains(&"10C4:EA60".to_string()));
        assert!(s.guidance[1].usb_ids.contains(&"1A86:7523".to_string()));
        assert!(s
            .guidance
            .iter()
            .all(|g| g.download_url.starts_with("https://")));
    }

    #[test]
    fn dfu_without_driver_needs_install() {
        let s = evaluate(&[dev(r"USB\VID_0483&PID_DF11\1", None, 28)]);
        assert!(!s.dfu_winusb);
        assert!(s.vcp);
        assert_eq!(s.devices[0].action, DriverAction::InstallDfuDriver);
        assert_eq!(s.devices[0].kind, UsbDeviceKind::Dfu);
        assert_eq!(
            dfu_ids_needing_install(&s),
            vec![(0x0483, 0xDF11, "STM32 DFU".into())]
        );
    }

    #[test]
    fn dfu_with_winusb_is_ok_and_other_driver_is_replaced() {
        let s = evaluate(&[
            dev(r"USB\VID_2E3C&PID_DF11\1", Some("WINUSB"), 0),
            dev(r"USB\VID_0483&PID_DF11\2", Some("STTub30"), 0),
        ]);
        assert!(s.devices[0].driver_ok);
        assert_eq!(s.devices[1].action, DriverAction::InstallDfuDriver);
        assert!(!s.dfu_winusb);
        // already-bound AT32 is not reinstalled
        assert_eq!(dfu_ids_needing_install(&s).len(), 1);
    }

    #[test]
    fn winusb_with_problem_asks_for_replug() {
        let s = evaluate(&[dev(r"USB\VID_0483&PID_DF11\1", Some("WinUSB"), 43)]);
        assert_eq!(s.devices[0].action, DriverAction::Replug);
        assert!(dfu_ids_needing_install(&s).is_empty());
    }

    #[test]
    fn bridges_without_driver_point_to_vendor() {
        let s = evaluate(&[
            dev(r"USB\VID_10C4&PID_EA60\0001", None, 28),
            dev(r"USB\VID_1A86&PID_7523\5&1", Some("CH341SER_A64"), 0),
            dev(r"USB\VID_1A86&PID_55D4\5&2", None, 28),
        ]);
        assert!(!s.vcp);
        assert!(s.dfu_winusb);
        let d = &s.devices;
        assert_eq!(d[0].action, DriverAction::VendorDownload);
        assert_eq!(
            s.guidance[d[0].guidance_index.unwrap() as usize].id,
            "CP210X"
        );
        assert!(d[1].driver_ok);
        assert_eq!(d[1].guidance_index, None);
        assert_eq!(
            s.guidance[d[2].guidance_index.unwrap() as usize].id,
            "CH343"
        );
        assert_eq!(s.notes.len(), 2);
    }

    #[test]
    fn composite_parent_and_unknown_devices_are_skipped() {
        let s = evaluate(&[
            dev(r"USB\VID_10C4&PID_EA70\1", Some("usbccgp"), 0),
            dev(r"USB\VID_10C4&PID_EA70&MI_00\2", Some("silabser"), 0),
            dev(r"USB\VID_046D&PID_C52B\3", Some("HidUsb"), 0),
        ]);
        assert_eq!(s.devices.len(), 1);
        assert!(s.devices[0].driver_ok);
    }

    #[test]
    fn vcp_without_driver_asks_for_replug() {
        let s = evaluate(&[dev(r"USB\VID_0483&PID_5740\1", None, 28)]);
        assert!(!s.vcp);
        assert_eq!(s.devices[0].action, DriverAction::Replug);
    }

    #[test]
    fn wdi_args_and_quoting() {
        let a = wdi_simple_args(0x0483, 0xDF11, "STM32 DFU", r"C:\Users\A B\Temp\x");
        assert!(a.windows(2).any(|w| w[0] == "--vid" && w[1] == "0x0483"));
        assert!(a.windows(2).any(|w| w[0] == "--pid" && w[1] == "0xDF11"));
        assert!(a.windows(2).any(|w| w[0] == "--type" && w[1] == "0"));
        assert_eq!(quote_windows_arg("plain"), "plain");
        assert_eq!(quote_windows_arg(r"C:\A B\"), r#""C:\A B\\""#);
        assert_eq!(quote_windows_arg(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(quote_windows_arg(""), r#""""#);
        assert_eq!(wdi_error_message(-14), "cancelled by user");
    }

    #[test]
    fn tables_have_no_duplicate_ids() {
        let mut all: Vec<(u16, u16)> = DFU_IDS.iter().map(|d| (d.0, d.1)).collect();
        all.extend(VCP_IDS.iter().map(|d| (d.0, d.1)));
        all.extend(BRIDGE_IDS.iter().map(|d| (d.0, d.1)));
        let n = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), n);
        assert!(BRIDGE_IDS.iter().all(|b| b.3 < guidance().len()));
    }
}
