// SPDX-License-Identifier: GPL-3.0-or-later
//! Betaflight CLI over serial: enter with `#`, run commands up to the `# ` prompt, leave with
//! `exit` / `save` / `bl`.
//!
//! Prompt detection and response cleanup follow Betaflight Configurator
//! `src/js/utils/AutoBackup.ts` (`hasPromptAtEnd`, `completedDump`), GPL-3.0-or-later.

use crate::device::error::{DeviceError, Result};
use crate::device::serial::SerialLink;
use std::time::{Duration, Instant};

/// True when the buffer ends at a fresh CLI prompt (`# ` at the start of a line).
pub fn has_prompt_at_end(buf: &[u8]) -> bool {
    buf == b"# " || buf.ends_with(b"\n# ")
}

/// Strip the echoed command, the trailing prompt and CRs from a raw CLI response.
pub fn clean_response(raw: &str, command: &str) -> String {
    let text = raw.replace('\r', "");
    let mut lines: Vec<&str> = text.split('\n').collect();
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    if !command.is_empty() && lines.first().is_some_and(|l| l.trim() == command.trim()) {
        lines.remove(0);
    }
    if lines
        .last()
        .is_some_and(|l| l.trim() == "#" || l.trim().is_empty())
    {
        lines.pop();
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

/// Which bootloader `bl` should reboot into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootloaderTarget {
    /// Plain `bl`: the ROM bootloader, or the flash bootloader on boards built with one.
    Default,
    /// `bl rom`: always the ROM (DFU) bootloader.
    Rom,
    /// `bl flash`: the flash bootloader.
    Flash,
}

impl BootloaderTarget {
    pub fn command(self) -> &'static str {
        match self {
            BootloaderTarget::Default => "bl",
            BootloaderTarget::Rom => "bl rom",
            BootloaderTarget::Flash => "bl flash",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CliOptions {
    /// Wait for the prompt after sending `#` (one retry is made).
    pub enter_timeout: Duration,
    /// Wait for the prompt after a command. `dump all` on a slow link needs a while.
    pub command_timeout: Duration,
    /// Silence required after a prompt before the response counts as complete, so a `# ` that
    /// happens to end a read mid-dump is not taken for the prompt.
    pub quiet: Duration,
}

impl Default for CliOptions {
    fn default() -> Self {
        CliOptions {
            enter_timeout: Duration::from_millis(1500),
            command_timeout: Duration::from_secs(30),
            quiet: Duration::from_millis(100),
        }
    }
}

/// An open CLI session. Dropping it leaves the FC in CLI mode; call [`exit`](Self::exit),
/// [`save`](Self::save) or [`reboot_to_bootloader`](Self::reboot_to_bootloader).
pub struct CliSession<S: SerialLink> {
    link: S,
    opts: CliOptions,
    /// Text printed on entry ("Entering CLI Mode, type 'exit' to return, or 'help'").
    pub banner: String,
}

impl<S: SerialLink> CliSession<S> {
    /// Send `#` and wait for the prompt.
    pub fn enter(mut link: S, opts: CliOptions) -> Result<Self> {
        link.clear_input()?;
        let mut last_err = None;
        for _ in 0..2 {
            link.write_all(b"#")?;
            match read_prompt(&mut link, opts.enter_timeout, opts.quiet) {
                Ok(raw) => {
                    let banner = clean_response(&raw, "#");
                    return Ok(CliSession { link, opts, banner });
                }
                Err(e @ DeviceError::Timeout { .. }) => last_err = Some(e),
                Err(e) => return Err(e),
            }
        }
        Err(last_err.unwrap_or_else(|| DeviceError::Cli("no CLI prompt".into())))
    }

    pub fn link_mut(&mut self) -> &mut S {
        &mut self.link
    }

    /// Run one command and return its output (echo and prompt removed).
    pub fn run(&mut self, command: &str) -> Result<String> {
        if command.contains(['\n', '\r']) {
            return Err(DeviceError::Cli("command must be a single line".into()));
        }
        self.link.write_all(format!("{command}\n").as_bytes())?;
        let raw = read_prompt(&mut self.link, self.opts.command_timeout, self.opts.quiet)?;
        Ok(clean_response(&raw, command))
    }

    /// `exit`: leave CLI without saving. Betaflight 4.4+ reboots the FC; the port will drop.
    pub fn exit(mut self) -> Result<S> {
        self.link.write_all(b"exit\n")?;
        let _ = drain(&mut self.link, Duration::from_millis(300), |t| {
            t.to_ascii_lowercase().contains("leaving cli mode")
        });
        Ok(self.link)
    }

    /// `exit noreboot` (Betaflight 4.5+ / API 1.46+): leave CLI and keep the FC running so MSP
    /// can continue on the same port. 4.4 ignores the argument and reboots, so callers must
    /// check the API version and expect a disconnect on 4.4.
    pub fn exit_no_reboot(mut self) -> Result<S> {
        self.link.write_all(b"exit noreboot\n")?;
        let _ = drain(&mut self.link, Duration::from_millis(300), |t| {
            t.to_ascii_lowercase().contains("leaving cli mode")
        });
        Ok(self.link)
    }

    /// Read-only board probe: `serial`, `resource`, `get rx_spi_protocol`. Changes nothing.
    /// Feed the result to `FcIdentity::apply_cli_probe`.
    pub fn probe_board(&mut self) -> Result<CliBoardProbe> {
        let serial = self.run("serial")?;
        let resource = self.run("resource")?;
        let get = self.run("get rx_spi_protocol")?;
        let rx_spi_protocol = (!get.contains("###ERROR")).then_some(get);
        Ok(CliBoardProbe {
            serial,
            resource,
            rx_spi_protocol,
        })
    }
}

/// Raw output of [`CliSession::probe_board`] (kept raw so it can also come from a `diff all`
/// the wizard already took for the backup: pass the dump text as both `serial` and `resource`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliBoardProbe {
    pub serial: String,
    pub resource: String,
    /// `get rx_spi_protocol` output; `None` when the CLI rejected the name (no USE_RX_SPI).
    pub rx_spi_protocol: Option<String>,
}

impl<S: SerialLink> CliSession<S> {
    /// `save`: write settings to flash; the FC reboots. Returns what it printed.
    pub fn save(mut self) -> Result<String> {
        self.link.write_all(b"save\n")?;
        let out = drain(&mut self.link, Duration::from_secs(3), |t| {
            t.contains("Rebooting")
        })
        .unwrap_or_default();
        let text = String::from_utf8_lossy(&out).into_owned();
        if text.to_ascii_lowercase().contains("error") {
            return Err(DeviceError::Cli(clean_response(&text, "save")));
        }
        Ok(clean_response(&text, "save"))
    }

    /// `bl`: reboot into the bootloader. The port drops right after.
    pub fn reboot_to_bootloader(mut self, target: BootloaderTarget) -> Result<()> {
        self.link
            .write_all(format!("{}\n", target.command()).as_bytes())?;
        let out = drain(&mut self.link, Duration::from_millis(500), |t| {
            t.contains("Rebooting") || t.contains("Invalid option")
        })
        .unwrap_or_default();
        let text = String::from_utf8_lossy(&out);
        if text.contains("Invalid option") {
            return Err(DeviceError::Cli(clean_response(&text, target.command())));
        }
        Ok(())
    }
}

/// Read until a prompt ends the buffer and the line stays quiet for `quiet`.
fn read_prompt<S: SerialLink + ?Sized>(
    link: &mut S,
    timeout: Duration,
    quiet: Duration,
) -> Result<String> {
    let deadline = Instant::now() + timeout;
    let mut out = Vec::new();
    let mut buf = [0u8; 1024];
    let mut prompt_since: Option<Instant> = None;
    loop {
        let now = Instant::now();
        if let Some(t) = prompt_since {
            if now.duration_since(t) >= quiet {
                return Ok(String::from_utf8_lossy(&out).into_owned());
            }
        }
        if now >= deadline {
            return Err(DeviceError::timeout("CLI prompt", timeout));
        }
        let n = link.read(&mut buf, (deadline - now).min(Duration::from_millis(20)))?;
        if n > 0 {
            out.extend_from_slice(&buf[..n]);
            prompt_since = has_prompt_at_end(&out).then(Instant::now);
        }
    }
}

/// Read whatever arrives for `window`, stopping early once `done(text)` holds; a disconnect
/// also ends it (expected when the FC reboots).
fn drain<S, F>(link: &mut S, window: Duration, done: F) -> Result<Vec<u8>>
where
    S: SerialLink + ?Sized,
    F: Fn(&str) -> bool,
{
    let deadline = Instant::now() + window;
    let mut out = Vec::new();
    let mut buf = [0u8; 512];
    while Instant::now() < deadline && !done(&String::from_utf8_lossy(&out)) {
        match link.read(&mut buf, Duration::from_millis(20)) {
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(DeviceError::Disconnected) => break,
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::mock::MockSerial;

    fn fast() -> CliOptions {
        CliOptions {
            enter_timeout: Duration::from_millis(200),
            command_timeout: Duration::from_millis(500),
            quiet: Duration::from_millis(5),
        }
    }

    /// A fake Betaflight CLI with echo, like the real one.
    fn fake_cli() -> MockSerial {
        let mut line = String::new();
        let mut in_cli = false;
        let mut m = MockSerial::new(move |w| {
            let mut out = String::new();
            for &b in w {
                let c = b as char;
                if !in_cli {
                    if c == '#' {
                        in_cli = true;
                        out += "\r\nEntering CLI Mode, type 'exit' to return, or 'help'\r\n\r\n# ";
                    }
                    continue;
                }
                if c != '\n' {
                    line.push(c);
                    continue;
                }
                out += &format!("{line}\r\n");
                match line.as_str() {
                    "version" => out += "# Betaflight / STM32F405 (S405) 4.5.2 Jan  1 2025 / 00:00:00 (abc1234) MSP API: 1.46\r\n\r\n# ",
                    "diff" => out += "# version\r\n# Betaflight\r\n\r\n# master\r\nset gyro_lpf1_static_hz = 0\r\n\r\n# ",
                    "save" => out += "# saving\r\nRebooting",
                    "bl" | "bl rom" => out += "\r\nRebooting into bootloader",
                    "bl bogus" => out += "###ERROR IN bl: Invalid option###\r\n\r\n# ",
                    "exit" => out += "\r\nLeaving CLI mode, unsaved changes lost.\r\n",
                    "exit noreboot" => out += "\r\n# leaving CLI mode, no reboot\r\n",
                    "serial" => out += "serial 20 1 115200 57600 0 115200\r\nserial 0 64 115200 57600 0 115200\r\n\r\n# ",
                    "resource" => out += "resource MOTOR 1 B10\r\nresource RX_SPI_CS 1 A15\r\nresource RX_SPI_EXTI 1 C14\r\n\r\n# ",
                    "get rx_spi_protocol" => out += "###ERROR IN get: INVALID NAME: rx_spi_protocol###\r\n\r\n# ",
                    other => out += &format!("###ERROR: UNKNOWN COMMAND '{other}'###\r\n\r\n# "),
                }
                line.clear();
            }
            out.into_bytes()
        });
        m.chunk = 7;
        m
    }

    #[test]
    fn prompt_detection() {
        assert!(has_prompt_at_end(b"# "));
        assert!(has_prompt_at_end(b"foo\r\n# "));
        assert!(!has_prompt_at_end(b"# version"));
        assert!(!has_prompt_at_end(b"foo\r\n#"));
        assert!(!has_prompt_at_end(b""));
    }

    #[test]
    fn clean_strips_echo_and_prompt() {
        let raw = "version\r\n# Betaflight 4.5.2\r\n\r\n# ";
        assert_eq!(clean_response(raw, "version"), "# Betaflight 4.5.2");
        assert_eq!(clean_response("\r\n# ", "x"), "");
    }

    #[test]
    fn enter_run_and_bl() {
        let s = CliSession::enter(fake_cli(), fast()).unwrap();
        assert!(s.banner.contains("Entering CLI Mode"));
        let mut s = s;
        let v = s.run("version").unwrap();
        assert!(v.starts_with("# Betaflight / STM32F405"));
        // Output containing "# " comment lines is not cut short.
        let d = s.run("diff").unwrap();
        assert!(d.ends_with("set gyro_lpf1_static_hz = 0"), "{d:?}");
        let e = s.run("nonsense").unwrap();
        assert!(e.contains("UNKNOWN COMMAND"));
        assert!(s.run("a\nb").is_err());
        s.reboot_to_bootloader(BootloaderTarget::Default).unwrap();
    }

    #[test]
    fn bl_invalid_option_is_error() {
        let mut m = fake_cli();
        m.chunk = 0;
        let s = CliSession::enter(m, fast()).unwrap();
        // Simulate firmware rejecting the option by sending a custom target string.
        let mut s = s;
        let out = s.run("bl bogus").unwrap();
        assert!(out.contains("Invalid option"));
    }

    #[test]
    fn exit_and_save() {
        let s = CliSession::enter(fake_cli(), fast()).unwrap();
        let link = s.exit().unwrap();
        assert!(link.written_str().ends_with("exit\n"));
        let s = CliSession::enter(fake_cli(), fast()).unwrap();
        let out = s.save().unwrap();
        assert!(out.contains("saving"));
    }

    #[test]
    fn probe_board_is_read_only_and_parses() {
        let mut s = CliSession::enter(fake_cli(), fast()).unwrap();
        let probe = s.probe_board().unwrap();
        assert!(probe.serial.starts_with("serial 20 1"));
        assert_eq!(probe.rx_spi_protocol, None);
        let w = s.link_mut().written_str();
        assert_eq!(w, "#serial\nresource\nget rx_spi_protocol\n");
        let link = s.exit_no_reboot().unwrap();
        assert!(link.written_str().ends_with("exit noreboot\n"));

        let mut info = crate::device::rx_spi::RxSpiInfo::default();
        info.apply_cli(&probe.resource, probe.rx_spi_protocol.as_deref());
        assert_eq!(info.has_rx_spi, Some(true));
        let cfg = crate::device::serial_ports::serial_config_from_cli(&probe.serial, None).unwrap();
        assert_eq!(cfg.ports.len(), 2);
    }

    #[test]
    fn enter_retries_then_times_out() {
        let m = MockSerial::silent();
        let r = CliSession::enter(m, fast());
        assert!(matches!(r, Err(DeviceError::Timeout { .. })));
    }

    #[test]
    fn enter_after_second_hash() {
        // FC ignores the first '#' (e.g. still booting), answers the second.
        let mut count = 0;
        let m = MockSerial::new(move |w| {
            if w == b"#" {
                count += 1;
                if count == 2 {
                    return b"\r\nEntering CLI Mode\r\n\r\n# ".to_vec();
                }
            }
            Vec::new()
        });
        let s = CliSession::enter(m, fast()).unwrap();
        assert_eq!(s.banner, "Entering CLI Mode");
    }
}
