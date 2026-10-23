// SPDX-License-Identifier: GPL-3.0-or-later
//! Live attitude stream: polls MSP_ATTITUDE (108) on a worker thread, like the RC stream
//! (same stop reasons, same timeout handling). The Tauri layer forwards samples as
//! `device://attitude` and the end as `device://attitude-stopped`.

use crate::device::error::DeviceError;
use crate::device::msp::{codes, MspClient};
use crate::device::rc::{EventSink, RcStopReason};
use crate::device::sensors::parse_attitude;
use crate::device::serial::SerialLink;
use crate::model::Attitude;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const DEFAULT_RATE_HZ: u32 = 10;
pub const MIN_RATE_HZ: u32 = 1;
pub const MAX_RATE_HZ: u32 = 30;

/// `rateHz` as the command takes it: default 10, clamped to 1..=30.
pub fn effective_rate(rate_hz: Option<u32>) -> u32 {
    rate_hz
        .unwrap_or(DEFAULT_RATE_HZ)
        .clamp(MIN_RATE_HZ, MAX_RATE_HZ)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttitudeStreamOptions {
    /// Poll rate, clamped to MIN_RATE_HZ..=MAX_RATE_HZ.
    pub rate_hz: u32,
    /// Give up after this many consecutive unanswered polls.
    pub max_consecutive_timeouts: u32,
    /// Per-poll MSP timeout in ms.
    pub request_timeout_ms: u64,
}

impl Default for AttitudeStreamOptions {
    fn default() -> Self {
        AttitudeStreamOptions {
            rate_hz: DEFAULT_RATE_HZ,
            max_consecutive_timeouts: 5,
            request_timeout_ms: 200,
        }
    }
}

impl AttitudeStreamOptions {
    pub fn period(&self) -> Duration {
        Duration::from_millis(1000 / self.rate_hz.clamp(MIN_RATE_HZ, MAX_RATE_HZ) as u64)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AttitudeStreamEvent {
    Attitude(Attitude),
    Stopped {
        reason: RcStopReason,
        error: Option<serde_json::Value>,
    },
}

/// Handle to a running stream. `stop()` (or drop) ends it; `stop()` returns the link.
pub struct AttitudeStream<S: SerialLink + 'static> {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<Option<S>>>,
}

impl<S: SerialLink + 'static> AttitudeStream<S> {
    pub fn start(
        link: S,
        opts: AttitudeStreamOptions,
        mut sink: impl EventSink<AttitudeStreamEvent>,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let join = std::thread::Builder::new()
            .name("attitude-stream".into())
            .spawn(move || {
                let mut client = MspClient::new(link);
                client.timeout = Duration::from_millis(opts.request_timeout_ms);
                let period = opts.period();
                let started = Instant::now();
                let mut next = started;
                let mut seq = 0u64;
                let mut timeouts = 0u32;
                loop {
                    if flag.load(Ordering::SeqCst) {
                        sink.emit(AttitudeStreamEvent::Stopped {
                            reason: RcStopReason::Requested,
                            error: None,
                        });
                        return Some(client.into_inner());
                    }
                    let result = client
                        .request(codes::MSP_ATTITUDE, &[])
                        .and_then(|p| parse_attitude(&p));
                    match result {
                        Ok((roll, pitch, yaw)) => {
                            timeouts = 0;
                            sink.emit(AttitudeStreamEvent::Attitude(Attitude {
                                seq,
                                timestamp_ms: started.elapsed().as_millis() as u64,
                                roll,
                                pitch,
                                yaw,
                            }));
                            seq += 1;
                        }
                        Err(DeviceError::Timeout { .. })
                            if timeouts + 1 < opts.max_consecutive_timeouts =>
                        {
                            timeouts += 1
                        }
                        Err(DeviceError::Disconnected) => {
                            sink.emit(AttitudeStreamEvent::Stopped {
                                reason: RcStopReason::Disconnected,
                                error: None,
                            });
                            return None;
                        }
                        Err(e) => {
                            sink.emit(AttitudeStreamEvent::Stopped {
                                reason: RcStopReason::Error,
                                error: Some(crate::error::AppError::from(e).to_json()),
                            });
                            return Some(client.into_inner());
                        }
                    }
                    next += period;
                    let now = Instant::now();
                    if next < now {
                        next = now; // fell behind; don't burst
                    }
                    while Instant::now() < next && !flag.load(Ordering::SeqCst) {
                        std::thread::sleep((next - Instant::now()).min(Duration::from_millis(10)));
                    }
                }
            })
            .expect("spawn attitude-stream thread");
        AttitudeStream {
            stop,
            join: Some(join),
        }
    }

    pub fn is_running(&self) -> bool {
        self.join.as_ref().is_some_and(|j| !j.is_finished())
    }

    /// Stop polling and get the link back (None if the port disconnected).
    pub fn stop(mut self) -> Option<S> {
        self.stop.store(true, Ordering::SeqCst);
        self.join.take().and_then(|j| j.join().ok().flatten())
    }
}

impl<S: SerialLink + 'static> Drop for AttitudeStream<S> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::mock::MockSerial;
    use crate::device::msp::{MspEvent, MspFrame, MspParser};
    use crate::device::rc::ChannelSink;
    use std::sync::mpsc;

    fn fc() -> MockSerial {
        let mut parser = MspParser::default();
        MockSerial::new(move |w| {
            let mut out = Vec::new();
            for ev in parser.feed(w) {
                if let MspEvent::Frame(f) = ev {
                    if f.code == codes::MSP_ATTITUDE {
                        let p: Vec<u8> = [-125i16, 47, 271]
                            .iter()
                            .flat_map(|v| v.to_le_bytes())
                            .collect();
                        out.extend(MspFrame::response(f.version, f.code, &p).encode().unwrap());
                    }
                }
            }
            out
        })
    }

    #[test]
    fn rate_is_clamped() {
        assert_eq!(effective_rate(None), 10);
        assert_eq!(effective_rate(Some(0)), 1);
        assert_eq!(effective_rate(Some(500)), 30);
        assert_eq!(effective_rate(Some(20)), 20);
        let p = |rate_hz| {
            AttitudeStreamOptions {
                rate_hz,
                ..Default::default()
            }
            .period()
        };
        assert_eq!(p(0), Duration::from_millis(1000));
        assert_eq!(p(100), Duration::from_millis(33));
        assert_eq!(p(10), Duration::from_millis(100));
    }

    #[test]
    fn streams_degrees_and_stops() {
        let (tx, rx) = mpsc::channel();
        let s = AttitudeStream::start(
            fc(),
            AttitudeStreamOptions {
                rate_hz: 30,
                ..Default::default()
            },
            ChannelSink(tx),
        );
        for want_seq in 0..3 {
            match rx.recv_timeout(Duration::from_secs(1)).unwrap() {
                AttitudeStreamEvent::Attitude(a) => {
                    assert_eq!(a.seq, want_seq);
                    assert_eq!((a.roll, a.pitch, a.yaw), (-12.5, 4.7, 271.0));
                }
                other => panic!("{other:?}"),
            }
        }
        assert!(s.is_running());
        assert!(s.stop().is_some());
        assert_eq!(
            rx.try_iter().last().unwrap(),
            AttitudeStreamEvent::Stopped {
                reason: RcStopReason::Requested,
                error: None
            }
        );
    }

    #[test]
    fn stops_on_disconnect_and_on_silence() {
        let unplug = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let s = AttitudeStream::start(
            fc().with_disconnect_switch(unplug.clone()),
            AttitudeStreamOptions::default(),
            ChannelSink(tx),
        );
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(1)),
            Ok(AttitudeStreamEvent::Attitude(_))
        ));
        unplug.store(true, Ordering::SeqCst);
        let stopped = rx
            .iter()
            .find(|e| matches!(e, AttitudeStreamEvent::Stopped { .. }))
            .unwrap();
        assert_eq!(
            stopped,
            AttitudeStreamEvent::Stopped {
                reason: RcStopReason::Disconnected,
                error: None
            }
        );
        assert!(s.stop().is_none());

        let (tx, rx) = mpsc::channel();
        let opts = AttitudeStreamOptions {
            rate_hz: 30,
            max_consecutive_timeouts: 3,
            request_timeout_ms: 20,
        };
        let s = AttitudeStream::start(MockSerial::silent(), opts, ChannelSink(tx));
        match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
            AttitudeStreamEvent::Stopped {
                reason: RcStopReason::Error,
                error: Some(e),
            } => assert_eq!(e["code"], "SERIAL"),
            other => panic!("{other:?}"),
        }
        assert!(s.stop().is_some());
    }
}
