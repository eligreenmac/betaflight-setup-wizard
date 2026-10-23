// SPDX-License-Identifier: GPL-3.0-or-later
//! In-memory [`SerialLink`] for tests (enabled with `cfg(test)` or the `mock` feature).

use crate::device::error::{DeviceError, Result};
use crate::device::serial::SerialLink;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

type Responder = Box<dyn FnMut(&[u8]) -> Vec<u8> + Send>;

/// A scripted serial port: every write is recorded and passed to a responder whose return value
/// is queued for reading. Reads may be split into small chunks to exercise partial-frame paths.
pub struct MockSerial {
    pub written: Vec<u8>,
    rx: VecDeque<u8>,
    responder: Responder,
    /// Max bytes returned per `read` (0 = unlimited).
    pub chunk: usize,
    /// When true, I/O fails with `Disconnected` (simulates the FC rebooting).
    pub disconnected: bool,
    /// Disconnect after the next write whose bytes contain this needle.
    pub disconnect_after_write_containing: Option<Vec<u8>>,
    /// Shared switch a test can flip from another thread to simulate unplugging.
    pub disconnect_switch: Option<Arc<AtomicBool>>,
}

impl MockSerial {
    pub fn new(responder: impl FnMut(&[u8]) -> Vec<u8> + Send + 'static) -> Self {
        MockSerial {
            written: Vec::new(),
            rx: VecDeque::new(),
            responder: Box::new(responder),
            chunk: 0,
            disconnected: false,
            disconnect_after_write_containing: None,
            disconnect_switch: None,
        }
    }

    /// A port that never answers.
    pub fn silent() -> Self {
        Self::new(|_| Vec::new())
    }

    /// Attach a shared unplug switch (see `disconnect_switch`).
    pub fn with_disconnect_switch(mut self, switch: Arc<AtomicBool>) -> Self {
        self.disconnect_switch = Some(switch);
        self
    }

    fn is_disconnected(&self) -> bool {
        self.disconnected
            || self
                .disconnect_switch
                .as_ref()
                .is_some_and(|s| s.load(Ordering::SeqCst))
    }

    pub fn written_str(&self) -> String {
        String::from_utf8_lossy(&self.written).into_owned()
    }
}

impl SerialLink for MockSerial {
    fn write_all(&mut self, data: &[u8]) -> Result<()> {
        if self.is_disconnected() {
            return Err(DeviceError::Disconnected);
        }
        self.written.extend_from_slice(data);
        let reply = (self.responder)(data);
        self.rx.extend(reply);
        if let Some(needle) = &self.disconnect_after_write_containing {
            if data.windows(needle.len()).any(|w| w == needle.as_slice()) {
                self.disconnected = true;
            }
        }
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize> {
        if self
            .disconnect_switch
            .as_ref()
            .is_some_and(|s| s.load(Ordering::SeqCst))
        {
            self.rx.clear(); // unplugged: pending bytes are lost
        }
        if self.rx.is_empty() {
            if self.is_disconnected() {
                return Err(DeviceError::Disconnected);
            }
            std::thread::sleep(timeout.min(Duration::from_millis(1)));
            return Ok(0);
        }
        let mut max = buf.len().min(self.rx.len());
        if self.chunk > 0 {
            max = max.min(self.chunk);
        }
        for b in buf.iter_mut().take(max) {
            *b = self.rx.pop_front().unwrap_or_default();
        }
        Ok(max)
    }

    fn clear_input(&mut self) -> Result<()> {
        self.rx.clear();
        Ok(())
    }

    fn name(&self) -> &str {
        "mock"
    }
}
