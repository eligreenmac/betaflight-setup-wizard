// SPDX-License-Identifier: GPL-3.0-or-later
//! Serial transport behind a trait, so MSP/CLI logic can run against a mock.

use crate::device::error::{DeviceError, Result};
use std::time::{Duration, Instant};

/// A byte pipe to a flight controller. Implemented by [`SystemSerial`] and `mock::MockSerial`.
pub trait SerialLink: Send {
    /// Write the whole buffer.
    fn write_all(&mut self, data: &[u8]) -> Result<()>;
    /// Read what is available, waiting at most `timeout`. `Ok(0)` means nothing arrived.
    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize>;
    /// Drop anything buffered on the input side.
    fn clear_input(&mut self) -> Result<()>;
    /// Port name, for logs and errors.
    fn name(&self) -> &str;
    /// A second, independent write handle to the same port, used by the motor-test
    /// emergency stop to write MSP_SET_MOTOR without waiting for the thread that owns
    /// this link. `None` if the port cannot be duplicated.
    fn try_clone_writer(&self) -> Option<Box<dyn std::io::Write + Send>> {
        None
    }
}

impl<T: SerialLink + ?Sized> SerialLink for Box<T> {
    fn write_all(&mut self, data: &[u8]) -> Result<()> {
        (**self).write_all(data)
    }
    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize> {
        (**self).read(buf, timeout)
    }
    fn clear_input(&mut self) -> Result<()> {
        (**self).clear_input()
    }
    fn name(&self) -> &str {
        (**self).name()
    }
    fn try_clone_writer(&self) -> Option<Box<dyn std::io::Write + Send>> {
        (**self).try_clone_writer()
    }
}

/// Open parameters. Betaflight's USB VCP ignores the baud rate; UART bridges (CP210x, CH340)
/// need the configured MSP rate, 115200 by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SerialConfig {
    pub baud_rate: u32,
    /// Assert DTR on open. Some bridges need it; harmless on VCP.
    pub dtr: bool,
}

impl Default for SerialConfig {
    fn default() -> Self {
        SerialConfig {
            baud_rate: 115_200,
            dtr: true,
        }
    }
}

/// A real OS serial port (`serialport` crate).
pub struct SystemSerial {
    port: Box<dyn serialport::SerialPort>,
    name: String,
}

impl SystemSerial {
    pub fn open(name: &str, config: SerialConfig) -> Result<Self> {
        let path = normalize_port_name(name);
        let mut port = serialport::new(path, config.baud_rate)
            .timeout(Duration::from_millis(100))
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(serialport::StopBits::One)
            .flow_control(serialport::FlowControl::None)
            .open()?;
        if config.dtr {
            // Not every driver supports it; ignore failures.
            let _ = port.write_data_terminal_ready(true);
        }
        Ok(SystemSerial {
            port,
            name: name.to_string(),
        })
    }

    /// Close explicitly (dropping also closes).
    pub fn close(self) {
        drop(self);
    }
}

impl SerialLink for SystemSerial {
    fn write_all(&mut self, data: &[u8]) -> Result<()> {
        use std::io::Write;
        self.port.write_all(data).map_err(map_io)?;
        self.port.flush().map_err(map_io)
    }

    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize> {
        use std::io::Read;
        self.port
            .set_timeout(timeout.max(Duration::from_millis(1)))?;
        match self.port.read(buf) {
            Ok(n) => Ok(n),
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Ok(0),
            Err(e) => Err(map_io(e)),
        }
    }

    fn clear_input(&mut self) -> Result<()> {
        self.port.clear(serialport::ClearBuffer::Input)?;
        Ok(())
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn try_clone_writer(&self) -> Option<Box<dyn std::io::Write + Send>> {
        match self.port.try_clone() {
            Ok(p) => Some(Box::new(p)),
            Err(e) => {
                log::warn!(
                    "{}: cannot duplicate port handle for emergency stop: {e}",
                    self.name
                );
                None
            }
        }
    }
}

fn map_io(e: std::io::Error) -> DeviceError {
    use std::io::ErrorKind::*;
    match e.kind() {
        BrokenPipe | NotConnected | UnexpectedEof | ConnectionReset => DeviceError::Disconnected,
        _ => DeviceError::Io(e),
    }
}

/// On Windows, COM ports above COM9 must be opened as `\\.\COM10`. Other OSes: unchanged.
#[cfg(windows)]
pub fn normalize_port_name(name: &str) -> String {
    if name.starts_with(r"\\.\") {
        name.to_string()
    } else if name.to_ascii_uppercase().starts_with("COM") {
        format!(r"\\.\{name}")
    } else {
        name.to_string()
    }
}

#[cfg(not(windows))]
pub fn normalize_port_name(name: &str) -> String {
    name.to_string()
}

/// Read until `done(buffer)` is true or `timeout` passes. Returns the bytes read.
pub fn read_until<S, F>(link: &mut S, mut done: F, timeout: Duration, what: &str) -> Result<Vec<u8>>
where
    S: SerialLink + ?Sized,
    F: FnMut(&[u8]) -> bool,
{
    let deadline = Instant::now() + timeout;
    let mut out = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        if done(&out) {
            return Ok(out);
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(DeviceError::timeout(what, timeout));
        }
        let n = link.read(&mut buf, (deadline - now).min(Duration::from_millis(50)))?;
        out.extend_from_slice(&buf[..n]);
    }
}
