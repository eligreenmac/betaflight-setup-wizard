//! The elevated part of `system_install_dfu_driver`. The app relaunches its own exe
//! with `runas` and [`ARG`] (one UAC prompt naming the app), and `main` hands that
//! process to [`dispatch`] before any Tauri/WebView code runs. The main app process
//! never runs elevated.
//!
//! Why not run `bfsw-wdi-simple.exe --dest %TEMP%\...` directly: %TEMP% is writable by
//! the (non-elevated) user, so other code running as the user could swap the extracted
//! .inf/.cat/co-installer files between extraction and installation. The elevated
//! process instead:
//! 1. atomically creates a fresh per-run directory `%ProgramData%\BFSW-WinUSB-<run id>`
//!    with a protected DACL that only grants SYSTEM and Administrators (owner
//!    Administrators). It fails if the directory already exists. It then opens the
//!    directory without following reparse points and without delete sharing, so it
//!    cannot be renamed or replaced while in use. It refuses to continue if the
//!    directory is a reparse point, its owner/DACL is not exactly
//!    [`SECURE_DIR_SDDL`], or it is not empty;
//! 2. copies the bundled helper (`<exe dir>\drivers\`, i.e. the Tauri resource dir,
//!    never a path from the command line) into it and runs it from there, once per
//!    VID/PID, with `--dest <dir>\<vid>_<pid>`, stopping at the first failure;
//! 3. deletes the directory (only one it created and verified).
//!
//! Command line: `<app exe> --bfsw-elevated-winusb-install <vid>:<pid>:<hex(name)>...`
//! (only `[0-9a-f:]`, so no quoting issues; names are re-validated).
//!
//! Exit code: 0, the helper's libwdi `wdi_error` (negative), or one of the `EXIT_*`
//! codes below.

#[cfg(windows)]
use super::usb_drivers::wdi_simple_args;
#[cfg(windows)]
use std::path::Path;

/// Subcommand that makes the app exe act as the elevated driver installer.
pub const ARG: &str = "--bfsw-elevated-winusb-install";

pub const EXIT_NO_PROGRAMDATA: i32 = 1001;
pub const EXIT_DIR_EXISTS: i32 = 1002;
pub const EXIT_DIR_CREATE: i32 = 1003;
pub const EXIT_DIR_INSECURE: i32 = 1004;
pub const EXIT_DIR_NOT_EMPTY: i32 = 1005;
pub const EXIT_COPY_HELPER: i32 = 1006;
pub const EXIT_HELPER_START: i32 = 1007;
pub const EXIT_BAD_ARGS: i32 = 1008;
pub const EXIT_UNSUPPORTED: i32 = 1009;

/// Owner Administrators, protected DACL: full control for SYSTEM and
/// Administrators only (inherited by everything created inside).
pub const SECURE_DIR_SDDL: &str = "O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)";

/// Name prefix of the per-run directory under %ProgramData%.
pub const DIR_PREFIX: &str = "BFSW-WinUSB-";

const MAX_NAME_CHARS: usize = 128;

/// Human message for a non-zero exit code (libwdi codes included).
pub fn exit_message(code: i32) -> String {
    match code {
        EXIT_NO_PROGRAMDATA => "could not locate %ProgramData%".into(),
        EXIT_DIR_EXISTS => "the driver work folder already exists; retry".into(),
        EXIT_DIR_CREATE => "could not create the protected driver work folder".into(),
        EXIT_DIR_INSECURE => {
            "the driver work folder is not admin-only (unexpected owner/ACL or a link)".into()
        }
        EXIT_DIR_NOT_EMPTY => "the driver work folder was tampered with (not empty)".into(),
        EXIT_COPY_HELPER => "could not copy the driver helper into its work folder".into(),
        EXIT_HELPER_START => "could not start the driver helper".into(),
        EXIT_BAD_ARGS => "invalid arguments for the elevated driver installer".into(),
        EXIT_UNSUPPORTED => "driver installation is only supported on Windows".into(),
        c => format!("{} (libwdi {c})", super::usb_drivers::wdi_error_message(c)),
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_NAME_CHARS
        && !name.chars().any(char::is_control)
}

/// `vid:pid:hex(utf8 name)`, all lowercase hex.
pub fn encode_device(vid: u16, pid: u16, name: &str) -> String {
    let hex: String = name.bytes().map(|b| format!("{b:02x}")).collect();
    format!("{vid:04x}:{pid:04x}:{hex}")
}

pub fn decode_device(s: &str) -> Option<(u16, u16, String)> {
    let mut parts = s.split(':');
    let (v, p, h) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || v.len() != 4 || p.len() != 4 || h.len() % 2 != 0 {
        return None;
    }
    let lower_hex = |x: &str| x.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if !lower_hex(v) || !lower_hex(p) || !lower_hex(h) {
        return None;
    }
    let bytes = (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&h[i..i + 2], 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    let name = String::from_utf8(bytes).ok()?;
    valid_name(&name).then_some((
        u16::from_str_radix(v, 16).ok()?,
        u16::from_str_radix(p, 16).ok()?,
        name,
    ))
}

/// Parameters for relaunching the app exe elevated. Names that would not survive
/// validation are replaced by a generic one.
pub fn params(devices: &[(u16, u16, String)]) -> String {
    let mut out = String::from(ARG);
    for (vid, pid, name) in devices {
        let name: String = name
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_NAME_CHARS)
            .collect();
        let name = if name.is_empty() {
            "DFU device".into()
        } else {
            name
        };
        out.push(' ');
        out.push_str(&encode_device(*vid, *pid, &name));
    }
    out
}

/// Called first thing in `main`: `Some(exit code)` if this process was started as
/// the elevated installer (the caller must exit with it), `None` to start the app.
pub fn dispatch<I: IntoIterator<Item = String>>(args: I) -> Option<i32> {
    let mut args = args.into_iter().skip(1);
    if args.next().as_deref() != Some(ARG) {
        return None;
    }
    let devices: Option<Vec<_>> = args.map(|a| decode_device(&a)).collect();
    let Some(devices) = devices else {
        return Some(EXIT_BAD_ARGS);
    };
    Some(run(&devices))
}

#[cfg(not(windows))]
fn run(_devices: &[(u16, u16, String)]) -> i32 {
    EXIT_UNSUPPORTED
}

#[cfg(windows)]
fn run(devices: &[(u16, u16, String)]) -> i32 {
    let Some(helper) = std::env::current_exe().ok().and_then(|e| {
        e.parent()
            .map(|d| d.join("drivers").join(super::HELPER_EXE))
    }) else {
        return EXIT_COPY_HELPER;
    };
    let Some(base) = win::program_data() else {
        return EXIT_NO_PROGRAMDATA;
    };
    install(&base, &run_id(), &helper, devices)
}

/// Per-run directory suffix (time + pid, hex).
pub fn run_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("{:x}{:x}", nanos, std::process::id())
}

/// Steps 1-3 of the module docs with explicit inputs (tested on Windows CI, whose
/// runners are elevated).
#[cfg(windows)]
pub fn install(base: &Path, run_id: &str, helper: &Path, devices: &[(u16, u16, String)]) -> i32 {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let dir = base.join(format!("{DIR_PREFIX}{run_id}"));
    if let Err(code) = win::create_secure_dir(&dir) {
        return code;
    }
    // From here on the directory is ours; the guard keeps it open (no delete
    // sharing, so it cannot be renamed/replaced) and removes it on every exit path,
    // but only once it has been verified.
    let guard = match win::SecureDir::open_verified(&dir) {
        Ok(g) => g,
        Err(code) => return code,
    };
    let exe = dir.join(super::HELPER_EXE);
    if std::fs::copy(helper, &exe).is_err() {
        return EXIT_COPY_HELPER;
    }
    for (vid, pid, name) in devices {
        let dest = dir.join(format!("{vid:04x}_{pid:04x}"));
        let status = std::process::Command::new(&exe)
            .args(wdi_simple_args(*vid, *pid, name, &dest.to_string_lossy()))
            .current_dir(&dir)
            .creation_flags(CREATE_NO_WINDOW)
            .status();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => return s.code().unwrap_or(EXIT_HELPER_START),
            Err(_) => return EXIT_HELPER_START,
        }
    }
    drop(guard);
    0
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, LocalFree, ERROR_ALREADY_EXISTS, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW,
        ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
        SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        SECURITY_ATTRIBUTES,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateDirectoryW, CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
        OPEN_EXISTING, READ_CONTROL,
    };
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_ProgramData, SHGetKnownFolderPath};

    fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
        s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
    }

    unsafe fn take_wstr(p: *const u16) -> String {
        let mut n = 0;
        while *p.add(n) != 0 {
            n += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
    }

    /// `%ProgramData%` from the shell (not the environment, which the unelevated
    /// caller controls).
    pub fn program_data() -> Option<PathBuf> {
        let mut p: *mut u16 = std::ptr::null_mut();
        // SAFETY: valid GUID pointer, out pointer freed with CoTaskMemFree.
        let hr =
            unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramData, 0, std::ptr::null_mut(), &mut p) };
        let out = (hr >= 0 && !p.is_null()).then(|| PathBuf::from(unsafe { take_wstr(p) }));
        unsafe { CoTaskMemFree(p.cast()) };
        out.filter(|d| d.is_absolute())
    }

    /// `CreateDirectoryW` with [`SECURE_DIR_SDDL`]; fails if it already exists.
    pub fn create_secure_dir(dir: &Path) -> Result<(), i32> {
        let sddl = wide(SECURE_DIR_SDDL);
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: NUL-terminated SDDL; sd freed with LocalFree.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(EXIT_DIR_CREATE);
        }
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd,
            bInheritHandle: 0,
        };
        let path = wide(dir);
        // SAFETY: valid path and security attributes.
        let created = unsafe { CreateDirectoryW(path.as_ptr(), &sa) } != 0;
        let err = unsafe { GetLastError() };
        unsafe { LocalFree(sd) };
        match (created, err) {
            (true, _) => Ok(()),
            (false, ERROR_ALREADY_EXISTS) => Err(EXIT_DIR_EXISTS),
            _ => Err(EXIT_DIR_CREATE),
        }
    }

    /// Open handle to a verified work directory; closes and deletes it on drop.
    pub struct SecureDir {
        handle: HANDLE,
        path: PathBuf,
    }

    impl SecureDir {
        pub fn open_verified(dir: &Path) -> Result<Self, i32> {
            let path = wide(dir);
            // SAFETY: valid path; no FILE_SHARE_DELETE, reparse points not followed.
            let handle = unsafe {
                CreateFileW(
                    path.as_ptr(),
                    READ_CONTROL | FILE_READ_ATTRIBUTES,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                    std::ptr::null_mut(),
                )
            };
            if handle == INVALID_HANDLE_VALUE {
                return Err(EXIT_DIR_INSECURE);
            }
            let unverified = Unverified(handle);
            // SAFETY: zeroed POD out struct.
            let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
            if unsafe { GetFileInformationByHandle(handle, &mut info) } == 0
                || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
                || info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
            {
                return Err(EXIT_DIR_INSECURE);
            }
            match sddl_of(handle) {
                Some(s) if normalize(&s) == SECURE_DIR_SDDL => {}
                other => {
                    eprintln!("work folder security: {other:?}");
                    return Err(EXIT_DIR_INSECURE);
                }
            }
            let empty = std::fs::read_dir(dir).map(|mut it| it.next().is_none());
            if !matches!(empty, Ok(true)) {
                return Err(EXIT_DIR_NOT_EMPTY);
            }
            std::mem::forget(unverified);
            Ok(Self {
                handle,
                path: dir.to_path_buf(),
            })
        }
    }

    impl Drop for SecureDir {
        fn drop(&mut self) {
            // SAFETY: handle from CreateFileW, closed once.
            unsafe { CloseHandle(self.handle) };
            // remove_dir_all does not follow links/junctions inside the directory.
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// Closes the handle (without deleting anything) if verification fails.
    struct Unverified(HANDLE);
    impl Drop for Unverified {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    /// The SDDL may carry the auto-inherited flag ("PAI"); it changes nothing here.
    fn normalize(s: &str) -> String {
        s.replace("D:PAI(", "D:P(")
    }

    fn sddl_of(handle: HANDLE) -> Option<String> {
        let info = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: valid handle; sd freed with LocalFree.
        let rc = unsafe {
            GetSecurityInfo(
                handle,
                SE_FILE_OBJECT,
                info,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut sd,
            )
        };
        if rc != 0 {
            return None;
        }
        let mut s: *mut u16 = std::ptr::null_mut();
        // SAFETY: sd from GetSecurityInfo; s freed with LocalFree.
        let ok = unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                sd,
                SDDL_REVISION_1,
                info,
                &mut s,
                std::ptr::null_mut(),
            )
        } != 0;
        let out = (ok && !s.is_null()).then(|| unsafe { take_wstr(s) });
        unsafe {
            LocalFree(s.cast());
            LocalFree(sd);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_args_round_trip_and_are_shell_safe() {
        let devs = vec![
            (0x0483, 0xDF11, "STM32 BOOTLOADER".to_string()),
            (0x2E3C, 0xDF11, "a\"'; & del C:\\ % $x \u{05e9}".to_string()),
        ];
        let p = params(&devs);
        assert!(p.starts_with(ARG));
        let rest = &p[ARG.len()..];
        assert!(rest
            .bytes()
            .all(|b| matches!(b, b' ' | b':' | b'0'..=b'9' | b'a'..=b'f')));
        let args =
            std::iter::once("app.exe".to_string()).chain(rest.split_whitespace().map(String::from));
        let decoded: Vec<_> = args.skip(1).map(|a| decode_device(&a).unwrap()).collect();
        assert_eq!(decoded, devs);
    }

    #[test]
    fn names_are_sanitized_and_bad_args_rejected() {
        let p = params(&[(1, 2, "\u{7}\n".into())]);
        assert_eq!(
            decode_device(p.split(' ').nth(1).unwrap()).unwrap().2,
            "DFU device"
        );
        let long = "x".repeat(500);
        let p = params(&[(1, 2, long)]);
        assert_eq!(
            decode_device(p.split(' ').nth(1).unwrap()).unwrap().2.len(),
            128
        );
        for bad in [
            "",
            "0483:df11",
            "0483:df11:",
            "483:df11:41",
            "0483:DF11:41",
            "0483:df11:4",
            "0483:df11:41:00",
            "0483:df11:0a",
            "0483:df11:ff",
            "0483:df11:zz",
        ] {
            assert!(decode_device(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn dispatch_only_handles_its_own_arg() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(dispatch(a(&["app.exe"])), None);
        assert_eq!(dispatch(a(&["app.exe", "--foo"])), None);
        assert_eq!(dispatch(a(&["app.exe", "x", ARG])), None);
        assert_eq!(
            dispatch(a(&["app.exe", ARG, "nonsense"])),
            Some(EXIT_BAD_ARGS)
        );
        #[cfg(not(windows))]
        assert_eq!(dispatch(a(&["app.exe", ARG])), Some(EXIT_UNSUPPORTED));
    }

    #[test]
    fn messages() {
        assert!(exit_message(EXIT_DIR_INSECURE).contains("admin-only"));
        assert!(exit_message(-14).contains("cancelled by user"));
        let id = run_id();
        assert!(!id.is_empty() && id.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    /// Runs the real elevated-path logic (GitHub's Windows runners are elevated):
    /// secure dir creation + verification, helper copy, exit-code propagation,
    /// cleanup, and refusal of a pre-existing dir. `whoami.exe` stands in for the
    /// helper.
    #[cfg(windows)]
    #[test]
    fn install_runs_on_windows() {
        let sys = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let whoami = std::path::PathBuf::from(format!(r"{sys}\System32\whoami.exe"));
        let base = win::program_data().expect("ProgramData");
        let id = run_id();
        let dir = base.join(format!("{DIR_PREFIX}{id}"));

        assert_eq!(install(&base, &id, &whoami, &[]), 0);
        assert!(!dir.exists());
        // whoami rejects wdi-simple's args -> non-zero propagated, dir still removed.
        let code = install(&base, &id, &whoami, &[(0x0483, 0xDF11, "STM32 DFU".into())]);
        assert!(code != 0 && code < 1000, "unexpected exit {code}");
        assert!(!dir.exists());
        // Missing helper -> copy error, dir removed.
        let missing = base.join("does-not-exist.exe");
        assert_eq!(install(&base, &id, &missing, &[]), EXIT_COPY_HELPER);
        assert!(!dir.exists());
        // Pre-existing dir (e.g. planted with a weak ACL) is refused and left alone.
        std::fs::create_dir_all(&dir).expect("mkdir");
        assert_eq!(install(&base, &id, &whoami, &[]), EXIT_DIR_EXISTS);
        assert!(dir.exists(), "a dir we did not create is left alone");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
