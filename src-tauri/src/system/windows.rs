//! Windows-only parts: enumerate present USB device nodes with their bound
//! driver service (SetupAPI + cfgmgr32, no elevation needed) and run the
//! libwdi helper elevated via `ShellExecuteExW("runas")` (one UAC prompt).

use super::usb_drivers::{parse_vid_pid, RawUsbDevice};
use crate::error::{AppError, AppResult};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_Status, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo,
    SetupDiGetClassDevsW, SetupDiGetDeviceInstanceIdW, SetupDiGetDeviceRegistryPropertyW,
    CR_SUCCESS, DIGCF_ALLCLASSES, DIGCF_PRESENT, DN_HAS_PROBLEM, HDEVINFO, SPDRP_SERVICE,
    SP_DEVINFO_DATA,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_CANCELLED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
use windows_sys::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// Closes the SetupAPI device info set on drop.
struct DevInfoSet(HDEVINFO);
impl Drop for DevInfoSet {
    fn drop(&mut self) {
        // SAFETY: handle came from SetupDiGetClassDevsW and is destroyed once.
        unsafe {
            SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

/// All present device nodes under the `USB` enumerator with a VID/PID.
pub fn list_usb_devices() -> AppResult<Vec<RawUsbDevice>> {
    let enumerator = wide("USB");
    // SAFETY: valid NUL-terminated enumerator, null GUID/HWND are allowed
    // with DIGCF_ALLCLASSES.
    let set = unsafe {
        SetupDiGetClassDevsW(
            std::ptr::null(),
            enumerator.as_ptr(),
            std::ptr::null_mut(),
            DIGCF_ALLCLASSES | DIGCF_PRESENT,
        )
    };
    if set == -1 {
        return Err(AppError::Driver(format!(
            "SetupDiGetClassDevsW failed (error {})",
            unsafe { GetLastError() }
        )));
    }
    let set = DevInfoSet(set);
    let mut out = Vec::new();
    for index in 0u32.. {
        // SAFETY: zeroed POD struct with cbSize set, as the API requires.
        let mut data: SP_DEVINFO_DATA = unsafe { std::mem::zeroed() };
        data.cbSize = std::mem::size_of::<SP_DEVINFO_DATA>() as u32;
        if unsafe { SetupDiEnumDeviceInfo(set.0, index, &mut data) } == 0 {
            break; // ERROR_NO_MORE_ITEMS (or any error): stop enumerating
        }
        let mut id_buf = [0u16; 512];
        let mut needed = 0u32;
        // SAFETY: buffer and its length in u16 units are passed together.
        let ok = unsafe {
            SetupDiGetDeviceInstanceIdW(
                set.0,
                &data,
                id_buf.as_mut_ptr(),
                id_buf.len() as u32,
                &mut needed,
            )
        };
        if ok == 0 {
            continue;
        }
        let instance_id = from_wide(&id_buf);
        let Some((vid, pid)) = parse_vid_pid(&instance_id) else {
            continue;
        };

        let mut svc_buf = [0u16; 256];
        let mut reg_type = 0u32;
        // SAFETY: buffer size passed in bytes, as SetupAPI expects.
        let has_service = unsafe {
            SetupDiGetDeviceRegistryPropertyW(
                set.0,
                &data,
                SPDRP_SERVICE,
                &mut reg_type,
                svc_buf.as_mut_ptr().cast(),
                std::mem::size_of_val(&svc_buf) as u32,
                &mut needed,
            )
        } != 0;
        let service = has_service
            .then(|| from_wide(&svc_buf))
            .filter(|s| !s.is_empty());

        let mut status = 0u32;
        let mut problem = 0u32;
        // SAFETY: DevInst comes from a valid SP_DEVINFO_DATA.
        let cr = unsafe { CM_Get_DevNode_Status(&mut status, &mut problem, data.DevInst, 0) };
        let problem = if cr == CR_SUCCESS && status & DN_HAS_PROBLEM != 0 {
            problem
        } else {
            0
        };
        out.push(RawUsbDevice {
            vid,
            pid,
            instance_id,
            service,
            problem,
        });
    }
    Ok(out)
}

/// `<System32>\WindowsPowerShell\v1.0\powershell.exe`, located via
/// `GetSystemDirectoryW` (not PATH or environment variables, which the user controls).
pub fn powershell_exe() -> AppResult<PathBuf> {
    let mut buf = [0u16; 260];
    // SAFETY: buffer and its length in u16 units are passed together.
    let n = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n >= buf.len() {
        return Err(AppError::Driver(format!(
            "GetSystemDirectoryW failed (error {})",
            unsafe { GetLastError() }
        )));
    }
    let exe = PathBuf::from(String::from_utf16_lossy(&buf[..n]))
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    if exe.is_file() {
        Ok(exe)
    } else {
        Err(AppError::Driver(format!(
            "Windows PowerShell not found at {}",
            exe.display()
        )))
    }
}

/// Run `exe args` elevated (UAC prompt), hidden, from `cwd`; wait up to
/// `timeout_ms` and return the process exit code.
pub fn run_elevated(exe: &Path, args: &str, cwd: &Path, timeout_ms: u32) -> AppResult<i32> {
    let verb = wide("runas");
    let file = wide(exe);
    let params = wide(args);
    let dir = wide(cwd);
    // SAFETY: zeroed POD struct; all string pointers outlive the call.
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = params.as_ptr();
    info.lpDirectory = dir.as_ptr();
    info.nShow = SW_HIDE;
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        let err = unsafe { GetLastError() };
        return Err(if err == ERROR_CANCELLED {
            AppError::Driver("administrator permission was declined (UAC)".into())
        } else {
            AppError::Driver(format!("could not start driver helper (error {err})"))
        });
    }
    if info.hProcess.is_null() {
        return Err(AppError::Driver("driver helper did not start".into()));
    }
    let wait = unsafe { WaitForSingleObject(info.hProcess, timeout_ms) };
    let mut code = 0u32;
    let got = unsafe { GetExitCodeProcess(info.hProcess, &mut code) } != 0;
    unsafe {
        CloseHandle(info.hProcess);
    }
    match wait {
        WAIT_OBJECT_0 if got => Ok(code as i32),
        WAIT_TIMEOUT => Err(AppError::Driver(
            "driver installation timed out; check Device Manager".into(),
        )),
        _ => Err(AppError::Driver(
            "could not read the driver helper result".into(),
        )),
    }
}
