// SPDX-License-Identifier: GPL-3.0-or-later
//! Live RC channel stream: polls MSP_RC (105) at a fixed rate on a worker thread and hands
//! decoded channel values to a sink (the Tauri layer forwards them as `device://rc-channels` events).
//! Decoding follows Betaflight Configurator `MSPHelper.ts` (`MSP_RC`: N x u16 LE, N = len/2).

use crate::device::error::{DeviceError, Result};
use crate::device::msp::{codes, MspClient};
use crate::device::serial::SerialLink;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const DEFAULT_RATE_HZ: u32 = 15;
pub const MIN_RATE_HZ: u32 = 1;
pub const MAX_RATE_HZ: u32 = 50;
/// Betaflight reports at most 18 channels (MAX_SUPPORTED_RC_CHANNEL_COUNT).
pub const MAX_CHANNELS: usize = 18;

/// Decode an MSP_RC payload. A trailing odd byte is ignored.
pub fn parse_rc(p: &[u8]) -> Result<Vec<u16>> {
    if p.len() / 2 > MAX_CHANNELS {
        return Err(DeviceError::MspMalformed(format!(
            "MSP_RC with {} channels",
            p.len() / 2
        )));
    }
    Ok(p.as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RcStreamOptions {
    /// Poll rate, clamped to MIN_RATE_HZ..=MAX_RATE_HZ.
    pub rate_hz: u32,
    /// Give up after this many consecutive unanswered polls.
    pub max_consecutive_timeouts: u32,
    /// Per-poll MSP timeout in ms.
    pub request_timeout_ms: u64,
}

impl Default for RcStreamOptions {
    fn default() -> Self {
        RcStreamOptions {
            rate_hz: DEFAULT_RATE_HZ,
            max_consecutive_timeouts: 5,
            request_timeout_ms: 200,
        }
    }
}

impl RcStreamOptions {
    pub fn period(&self) -> Duration {
        Duration::from_millis(1000 / self.rate_hz.clamp(MIN_RATE_HZ, MAX_RATE_HZ) as u64)
    }
}

/// Payload of `device://rc-channels`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RcFrame {
    pub seq: u64,
    /// Milliseconds since the stream started.
    pub timestamp_ms: u64,
    /// Raw channel values (usually 1000-2000 µs), in Betaflight channel order
    /// (first four follow the FC's `map`, e.g. AETR).
    pub channels: Vec<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RcStopReason {
    Requested,
    Disconnected,
    Error,
}

/// What the sink receives. Tauri mapping: `Channels` -> `device://rc-channels`, `Stopped` -> `device://rc-stopped`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RcStreamEvent {
    Channels(RcFrame),
    #[serde(rename_all = "camelCase")]
    Stopped {
        reason: RcStopReason,
        error: Option<serde_json::Value>,
    },
}

/// Where events go (Tauri: `app.emit(...)`; tests: a channel).
pub trait EventSink<E>: Send + 'static {
    fn emit(&mut self, event: E);
}

impl<E, F: FnMut(E) + Send + 'static> EventSink<E> for F {
    fn emit(&mut self, event: E) {
        self(event)
    }
}

/// Wraps `mpsc::Sender` (closures cover most uses; this avoids a move-closure in tests).
pub struct ChannelSink<E>(pub mpsc::Sender<E>);

impl<E: Send + 'static> EventSink<E> for ChannelSink<E> {
    fn emit(&mut self, event: E) {
        let _ = self.0.send(event);
    }
}

/// Handle to a running stream. `stop()` (or drop) ends it; `stop()` returns the link.
pub struct RcStream<S: SerialLink + 'static> {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<Option<S>>>,
}

impl<S: SerialLink + 'static> RcStream<S> {
    pub fn start(link: S, opts: RcStreamOptions, mut sink: impl EventSink<RcStreamEvent>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let join = std::thread::Builder::new()
            .name("rc-stream".into())
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
                        sink.emit(RcStreamEvent::Stopped {
                            reason: RcStopReason::Requested,
                            error: None,
                        });
                        return Some(client.into_inner());
                    }
                    let result = client
                        .request(codes::MSP_RC, &[])
                        .and_then(|p| parse_rc(&p));
                    match result {
                        Ok(channels) => {
                            timeouts = 0;
                            sink.emit(RcStreamEvent::Channels(RcFrame {
                                seq,
                                timestamp_ms: started.elapsed().as_millis() as u64,
                                channels,
                            }));
                            seq += 1;
                        }
                        Err(DeviceError::Timeout { .. })
                            if timeouts + 1 < opts.max_consecutive_timeouts =>
                        {
                            timeouts += 1
                        }
                        Err(DeviceError::Disconnected) => {
                            sink.emit(RcStreamEvent::Stopped {
                                reason: RcStopReason::Disconnected,
                                error: None,
                            });
                            return None;
                        }
                        Err(e) => {
                            sink.emit(RcStreamEvent::Stopped {
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
                    // Sleep in slices so stop() is responsive.
                    while Instant::now() < next && !flag.load(Ordering::SeqCst) {
                        std::thread::sleep((next - Instant::now()).min(Duration::from_millis(10)));
                    }
                }
            })
            .expect("spawn rc-stream thread");
        RcStream {
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

impl<S: SerialLink + 'static> Drop for RcStream<S> {
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
    use std::sync::atomic::AtomicU16;

    fn fc(throttle: Arc<AtomicU16>) -> MockSerial {
        let mut parser = MspParser::default();
        MockSerial::new(move |w| {
            let mut out = Vec::new();
            for ev in parser.feed(w) {
                if let MspEvent::Frame(f) = ev {
                    if f.code == codes::MSP_RC {
                        let ch = [
                            1500u16,
                            1500,
                            throttle.load(Ordering::SeqCst),
                            1500,
                            1000,
                            2000,
                            1500,
                            1500,
                        ];
                        let p: Vec<u8> = ch.iter().flat_map(|v| v.to_le_bytes()).collect();
                        out.extend(MspFrame::response(f.version, f.code, &p).encode().unwrap());
                    }
                }
            }
            out
        })
    }

    #[test]
    fn decodes_channels() {
        assert_eq!(
            parse_rc(&[0xDC, 0x05, 0xE8, 0x03, 0x01]).unwrap(),
            vec![1500, 1000]
        );
        assert!(parse_rc(&[]).unwrap().is_empty());
        assert!(parse_rc(&[0; 40]).is_err());
        assert_eq!(
            RcStreamOptions {
                rate_hz: 500,
                ..Default::default()
            }
            .period(),
            Duration::from_millis(20)
        );
        assert_eq!(
            RcStreamOptions {
                rate_hz: 0,
                ..Default::default()
            }
            .period(),
            Duration::from_millis(1000)
        );
    }

    #[test]
    fn start_stream_stop() {
        let thr = Arc::new(AtomicU16::new(1000));
        let (tx, rx) = mpsc::channel();
        let s = RcStream::start(
            fc(thr.clone()),
            RcStreamOptions {
                rate_hz: 50,
                ..Default::default()
            },
            ChannelSink(tx),
        );
        let first = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        match first {
            RcStreamEvent::Channels(f) => {
                assert_eq!(f.seq, 0);
                assert_eq!(f.channels.len(), 8);
                assert_eq!(f.channels[2], 1000);
            }
            other => panic!("{other:?}"),
        }
        thr.store(1234, Ordering::SeqCst);
        let saw = (0..20).any(|_| matches!(rx.recv_timeout(Duration::from_secs(1)), Ok(RcStreamEvent::Channels(ref f)) if f.channels[2] == 1234));
        assert!(saw);
        assert!(s.is_running());
        let link = s.stop().expect("link returned");
        let last = rx.try_iter().last().unwrap();
        assert_eq!(
            last,
            RcStreamEvent::Stopped {
                reason: RcStopReason::Requested,
                error: None
            }
        );
        assert!(link.written.len() > 10);
    }

    #[test]
    fn stops_on_disconnect_and_on_silence() {
        let unplug = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let s = RcStream::start(
            fc(Arc::new(AtomicU16::new(1000))).with_disconnect_switch(unplug.clone()),
            RcStreamOptions::default(),
            ChannelSink(tx),
        );
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(1)),
            Ok(RcStreamEvent::Channels(_))
        ));
        unplug.store(true, Ordering::SeqCst);
        let stopped = rx
            .iter()
            .find(|e| matches!(e, RcStreamEvent::Stopped { .. }))
            .unwrap();
        assert_eq!(
            stopped,
            RcStreamEvent::Stopped {
                reason: RcStopReason::Disconnected,
                error: None
            }
        );
        assert!(s.stop().is_none());

        let (tx, rx) = mpsc::channel();
        let opts = RcStreamOptions {
            rate_hz: 50,
            max_consecutive_timeouts: 3,
            request_timeout_ms: 20,
        };
        let s = RcStream::start(MockSerial::silent(), opts, ChannelSink(tx));
        match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
            RcStreamEvent::Stopped {
                reason: RcStopReason::Error,
                error: Some(e),
            } => assert_eq!(e["code"], "SERIAL"),
            other => panic!("{other:?}"),
        }
        assert!(s.stop().is_some());
    }

    #[test]
    fn event_json_shape() {
        let e = RcStreamEvent::Channels(RcFrame {
            seq: 3,
            timestamp_ms: 150,
            channels: vec![1500, 1000],
        });
        assert_eq!(
            serde_json::to_string(&e).unwrap(),
            r#"{"type":"channels","seq":3,"timestampMs":150,"channels":[1500,1000]}"#
        );
    }
}
