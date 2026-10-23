// SPDX-License-Identifier: GPL-3.0-or-later
//! Live attitude stream: polls MSP_ATTITUDE (108) on a worker thread, like the RC stream
//! (same stop reasons, same timeout handling). The Tauri layer forwards samples as
//! `device://attitude` and the end as `device://attitude-stopped`.
//!
//! Latest value only: a separate emitter thread forwards samples, and a sample that arrives
//! while the previous one is still being emitted replaces any older one that is waiting, so
//! a slow consumer sees fresh attitude (with gaps in `seq`), never a backlog. Polling is
//! paced from a fixed schedule; after a slow poll it restarts from now instead of bursting.
//!
//! The link stays shareable while streaming: [`AttitudeStream::take_link`] pauses polling
//! between two polls and hands the port to another (read-only MSP) caller, and
//! [`AttitudeStream::return_link`] resumes it (see `ConnectionManager::with_conn_shared`).

use crate::device::error::DeviceError;
use crate::device::msp::{codes, MspClient};
use crate::device::rc::{EventSink, RcStopReason};
use crate::device::sensors::parse_attitude;
use crate::device::serial::SerialLink;
use crate::model::Attitude;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Default when the UI passes no rate (30 Hz: smooth 3D model, ~1/4 of what a USB VCP
/// link sustains with other polling, see docs/ARCHITECTURE.md §4.3).
pub const DEFAULT_RATE_HZ: u32 = 30;
pub const MIN_RATE_HZ: u32 = 1;
/// Upper clamp (measured headroom: see docs/ARCHITECTURE.md §4.3).
pub const MAX_RATE_HZ: u32 = 50;

/// `rateHz` as the command takes it: default 30, clamped to 1..=50.
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
        Duration::from_micros(1_000_000 / self.rate_hz.clamp(MIN_RATE_HZ, MAX_RATE_HZ) as u64)
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

/// Worker -> emitter hand-over: one slot for the newest sample, plus the end.
#[derive(Default)]
struct Mailbox {
    latest: Option<Attitude>,
    stopped: Option<(RcStopReason, Option<serde_json::Value>)>,
    /// Samples overwritten before they were emitted (for logs/tests).
    dropped: u64,
}

struct Shared<S: SerialLink> {
    /// The poller's client; `None` while lent out by `take_link`.
    client: Mutex<Option<MspClient<S>>>,
    /// Set by `take_link` so the poller stays off the link until it is returned.
    wanted: AtomicBool,
    stop: AtomicBool,
    mailbox: Mutex<Mailbox>,
    wake: Condvar,
    request_timeout: Duration,
}

impl<S: SerialLink> Shared<S> {
    fn mailbox(&self) -> MutexGuard<'_, Mailbox> {
        self.mailbox.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn client(&self) -> MutexGuard<'_, Option<MspClient<S>>> {
        self.client.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn publish(&self, a: Attitude) {
        let mut m = self.mailbox();
        if m.latest.replace(a).is_some() {
            m.dropped += 1;
        }
        self.wake.notify_one();
    }

    fn finish(&self, reason: RcStopReason, error: Option<serde_json::Value>) {
        let mut m = self.mailbox();
        if reason == RcStopReason::Requested {
            m.latest = None; // nothing after an explicit stop
        }
        m.stopped = Some((reason, error));
        self.wake.notify_one();
    }
}

/// Handle to a running stream. `stop()` (or drop) ends it; `stop()` returns the link.
pub struct AttitudeStream<S: SerialLink + 'static> {
    shared: Arc<Shared<S>>,
    poller: Option<JoinHandle<bool>>,
    emitter: Option<JoinHandle<()>>,
}

impl<S: SerialLink + 'static> AttitudeStream<S> {
    pub fn start(
        link: S,
        opts: AttitudeStreamOptions,
        mut sink: impl EventSink<AttitudeStreamEvent>,
    ) -> Self {
        let request_timeout = Duration::from_millis(opts.request_timeout_ms);
        let mut client = MspClient::new(link);
        client.timeout = request_timeout;
        let shared = Arc::new(Shared {
            client: Mutex::new(Some(client)),
            wanted: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            mailbox: Mutex::new(Mailbox::default()),
            wake: Condvar::new(),
            request_timeout,
        });

        let sh = shared.clone();
        let emitter = std::thread::Builder::new()
            .name("attitude-emit".into())
            .spawn(move || loop {
                let ev = {
                    let mut m = sh.mailbox();
                    loop {
                        if let Some(a) = m.latest.take() {
                            break AttitudeStreamEvent::Attitude(a);
                        }
                        if let Some((reason, error)) = m.stopped.take() {
                            if m.dropped > 0 {
                                log::debug!(
                                    "attitude stream: {} stale sample(s) dropped",
                                    m.dropped
                                );
                            }
                            break AttitudeStreamEvent::Stopped { reason, error };
                        }
                        m = sh.wake.wait(m).unwrap_or_else(|p| p.into_inner());
                    }
                };
                let last = matches!(ev, AttitudeStreamEvent::Stopped { .. });
                sink.emit(ev);
                if last {
                    return;
                }
            })
            .expect("spawn attitude-emit thread");

        let sh = shared.clone();
        let poller = std::thread::Builder::new()
            .name("attitude-stream".into())
            .spawn(move || Self::poll_loop(&sh, opts))
            .expect("spawn attitude-stream thread");
        AttitudeStream {
            shared,
            poller: Some(poller),
            emitter: Some(emitter),
        }
    }

    /// Returns whether the link is still usable (false after a disconnect).
    fn poll_loop(sh: &Shared<S>, opts: AttitudeStreamOptions) -> bool {
        let period = opts.period();
        let started = Instant::now();
        let mut next = started;
        let mut seq = 0u64;
        let mut timeouts = 0u32;
        loop {
            if sh.stop.load(Ordering::SeqCst) {
                sh.finish(RcStopReason::Requested, None);
                return true;
            }
            if !sh.wanted.load(Ordering::SeqCst) {
                let mut guard = sh.client();
                // Lent out (paused): skip this tick.
                if let Some(client) = guard.as_mut() {
                    let result = client
                        .request(codes::MSP_ATTITUDE, &[])
                        .and_then(|p| parse_attitude(&p));
                    drop(guard);
                    match result {
                        Ok((roll, pitch, yaw)) => {
                            timeouts = 0;
                            sh.publish(Attitude {
                                seq,
                                timestamp_ms: started.elapsed().as_millis() as u64,
                                roll,
                                pitch,
                                yaw,
                            });
                            seq += 1;
                        }
                        Err(DeviceError::Timeout { .. })
                            if timeouts + 1 < opts.max_consecutive_timeouts =>
                        {
                            timeouts += 1
                        }
                        Err(DeviceError::Disconnected) => {
                            sh.finish(RcStopReason::Disconnected, None);
                            return false;
                        }
                        Err(e) => {
                            sh.finish(
                                RcStopReason::Error,
                                Some(crate::error::AppError::from(e).to_json()),
                            );
                            return true;
                        }
                    }
                }
            }
            next += period;
            let now = Instant::now();
            if next < now {
                next = now; // fell behind; don't burst
            }
            while Instant::now() < next && !sh.stop.load(Ordering::SeqCst) {
                std::thread::sleep((next - Instant::now()).min(Duration::from_millis(5)));
            }
        }
    }

    pub fn is_running(&self) -> bool {
        self.poller.as_ref().is_some_and(|j| !j.is_finished())
    }

    /// Pause polling after the poll in flight (if any) and take the link. `None` once the
    /// poller has ended (or the link is already out). Give it back with `return_link`.
    pub fn take_link(&self) -> Option<S> {
        if !self.is_running() {
            return None;
        }
        self.shared.wanted.store(true, Ordering::SeqCst);
        let taken = self.shared.client().take().map(MspClient::into_inner);
        if taken.is_none() {
            self.shared.wanted.store(false, Ordering::SeqCst);
        }
        taken
    }

    /// Resume polling on `link` (from `take_link`).
    pub fn return_link(&self, link: S) {
        let mut client = MspClient::new(link);
        client.timeout = self.shared.request_timeout;
        *self.shared.client() = Some(client);
        self.shared.wanted.store(false, Ordering::SeqCst);
    }

    /// Stop polling and get the link back (None if the port disconnected or the link was
    /// not returned).
    pub fn stop(mut self) -> Option<S> {
        self.shutdown()
    }

    fn shutdown(&mut self) -> Option<S> {
        self.shared.stop.store(true, Ordering::SeqCst);
        let usable = self
            .poller
            .take()
            .map(|j| j.join().unwrap_or(false))
            .unwrap_or(false);
        if let Some(j) = self.emitter.take() {
            let _ = j.join();
        }
        let link = self.shared.client().take().map(MspClient::into_inner);
        link.filter(|_| usable)
    }
}

impl<S: SerialLink + 'static> Drop for AttitudeStream<S> {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::mock::MockSerial;
    use crate::device::msp::{MspEvent, MspFrame, MspParser};
    use crate::device::rc::ChannelSink;
    use std::sync::mpsc;
    use std::time::Instant;

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
        assert_eq!(effective_rate(None), 30);
        assert_eq!(effective_rate(Some(0)), 1);
        assert_eq!(effective_rate(Some(500)), 50);
        assert_eq!(effective_rate(Some(50)), 50);
        assert_eq!(effective_rate(Some(20)), 20);
        let p = |rate_hz| {
            AttitudeStreamOptions {
                rate_hz,
                ..Default::default()
            }
            .period()
        };
        assert_eq!(p(0), Duration::from_millis(1000));
        assert_eq!(p(100), Duration::from_millis(20));
        assert_eq!(p(30), Duration::from_micros(33_333));
        assert_eq!(p(10), Duration::from_millis(100));
        assert_eq!(AttitudeStreamOptions::default().rate_hz, 30);
    }

    #[test]
    fn slow_listener_gets_the_latest_sample_not_a_backlog() {
        let (tx, rx) = mpsc::channel();
        let s = AttitudeStream::start(
            fc(),
            AttitudeStreamOptions {
                rate_hz: 50,
                ..Default::default()
            },
            move |ev: AttitudeStreamEvent| {
                if matches!(ev, AttitudeStreamEvent::Attitude(_)) {
                    std::thread::sleep(Duration::from_millis(100)); // 10 Hz consumer
                }
                let _ = tx.send((Instant::now(), ev));
            },
        );
        std::thread::sleep(Duration::from_millis(700));
        let t_stop = Instant::now();
        assert!(s.stop().is_some());
        let events: Vec<(Instant, AttitudeStreamEvent)> = rx.try_iter().collect();
        let seqs: Vec<u64> = events
            .iter()
            .filter_map(|(_, e)| match e {
                AttitudeStreamEvent::Attitude(a) => Some(a.seq),
                _ => None,
            })
            .collect();
        assert!(seqs.len() >= 3 && seqs.len() <= 9, "{seqs:?}");
        assert!(seqs.windows(2).all(|w| w[1] > w[0]), "{seqs:?}");
        // Polling kept its own pace: samples were skipped, not queued.
        assert!(seqs.windows(2).any(|w| w[1] - w[0] >= 3), "{seqs:?}");
        // Stopping does not drain a backlog: at most the sample being emitted finishes.
        let (t_end, last) = events.last().unwrap();
        assert!(matches!(
            last,
            AttitudeStreamEvent::Stopped {
                reason: RcStopReason::Requested,
                ..
            }
        ));
        assert!(t_end.duration_since(t_stop) < Duration::from_millis(250));
    }

    #[test]
    fn link_can_be_borrowed_between_polls() {
        let (tx, rx) = mpsc::channel();
        let s = AttitudeStream::start(
            fc(),
            AttitudeStreamOptions {
                rate_hz: 50,
                ..Default::default()
            },
            ChannelSink(tx),
        );
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(1)),
            Ok(AttitudeStreamEvent::Attitude(_))
        ));
        let link = s.take_link().expect("link lent");
        assert!(s.take_link().is_none(), "only one borrower");
        // Paused: nothing new while the link is out.
        std::thread::sleep(Duration::from_millis(60));
        let _ = rx.try_iter().count();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(rx.try_iter().count(), 0, "polled while lent out");
        // Our own request on the borrowed link.
        let mut c = MspClient::new(link);
        let (roll, _, _) = parse_attitude(&c.request(codes::MSP_ATTITUDE, &[]).unwrap()).unwrap();
        assert_eq!(roll, -12.5);
        s.return_link(c.into_inner());
        // Resumes at the normal pace, no burst.
        let t0 = Instant::now();
        let mut n = 0;
        while t0.elapsed() < Duration::from_millis(200) {
            if let Ok(AttitudeStreamEvent::Attitude(_)) = rx.recv_timeout(Duration::from_millis(50))
            {
                n += 1;
            }
        }
        assert!((5..=13).contains(&n), "{n} samples in 200 ms at 50 Hz");
        assert!(s.stop().is_some());
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
