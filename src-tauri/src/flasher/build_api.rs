// SPDX-License-Identifier: GPL-3.0-or-later
//! Betaflight cloud build API client. Ported from Betaflight Configurator `src/js/BuildApi.ts`
//! and the polling logic of `useCloudBuild.ts` (GPL-3.0-or-later).
//!
//! **Blocking.** Uses `reqwest::blocking` behind the [`HttpClient`] trait so the flasher has no
//! async runtime and tests can mock HTTP. In Tauri call it from `tauri::async_runtime::spawn_blocking`
//! (the same thread model as the USB flashing). Do not call blocking reqwest from inside a tokio
//! async context directly; it panics.

use crate::flasher::build_types::*;
use crate::flasher::error::{FlashError, Result};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const DEFAULT_BASE_URL: &str = "https://build.betaflight.com";

/// Minimal HTTP surface the client needs.
pub trait HttpClient: Send + Sync {
    /// GET; returns (status, body).
    fn get(&self, url: &str) -> Result<(u16, Vec<u8>)>;
    /// POST JSON; returns (status, body).
    fn post_json(&self, url: &str, body: &Value) -> Result<(u16, Vec<u8>)>;
}

/// `reqwest::blocking` implementation; sends `X-CFG-VER` like the configurator.
pub struct ReqwestClient {
    client: reqwest::blocking::Client,
}

impl ReqwestClient {
    /// `app_version` goes in `X-CFG-VER` (see DESIGN.md open questions).
    pub fn new(app_version: &str) -> Result<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Ok(v) = reqwest::header::HeaderValue::from_str(app_version) {
            headers.insert("X-CFG-VER", v);
        }
        let client = reqwest::blocking::Client::builder()
            .user_agent(concat!(
                "betaflight-setup-wizard/",
                env!("CARGO_PKG_VERSION")
            ))
            .default_headers(headers)
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| FlashError::Http(e.to_string()))?;
        Ok(ReqwestClient { client })
    }
}

fn http_err(e: reqwest::Error) -> FlashError {
    FlashError::Http(e.to_string())
}

impl HttpClient for ReqwestClient {
    fn get(&self, url: &str) -> Result<(u16, Vec<u8>)> {
        let r = self.client.get(url).send().map_err(http_err)?;
        let status = r.status().as_u16();
        Ok((status, r.bytes().map_err(http_err)?.to_vec()))
    }

    fn post_json(&self, url: &str, body: &Value) -> Result<(u16, Vec<u8>)> {
        let r = self.client.post(url).json(body).send().map_err(http_err)?;
        let status = r.status().as_u16();
        Ok((status, r.bytes().map_err(http_err)?.to_vec()))
    }
}

/// Polling parameters for [`BuildApi::wait_for_build`] (configurator: 5 s poll, 120 s default).
#[derive(Debug, Clone)]
pub struct PollOptions {
    pub interval: Duration,
    pub default_timeout: Duration,
}

impl Default for PollOptions {
    fn default() -> Self {
        PollOptions {
            interval: Duration::from_secs(5),
            default_timeout: Duration::from_secs(120),
        }
    }
}

pub struct BuildApi<C: HttpClient> {
    http: C,
    base_url: String,
    sleep: Box<dyn Fn(Duration) + Send + Sync>,
}

impl BuildApi<ReqwestClient> {
    pub fn with_reqwest(app_version: &str) -> Result<Self> {
        Ok(Self::new(
            ReqwestClient::new(app_version)?,
            DEFAULT_BASE_URL,
        ))
    }
}

impl<C: HttpClient> BuildApi<C> {
    pub fn new(http: C, base_url: &str) -> Self {
        BuildApi {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            sleep: Box::new(std::thread::sleep),
        }
    }

    pub fn with_sleep(mut self, sleep: impl Fn(Duration) + Send + Sync + 'static) -> Self {
        self.sleep = Box::new(sleep);
        self
    }

    #[cfg(test)]
    pub(crate) fn http(&self) -> &C {
        &self.http
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn ok(status: u16) -> bool {
        matches!(status, 200..=202)
    }

    fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = self.url(path);
        let (status, body) = self.http.get(&url)?;
        if !Self::ok(status) {
            return Err(FlashError::HttpStatus { status, url });
        }
        Ok(serde_json::from_slice(&body)?)
    }

    pub fn targets(&self) -> Result<Vec<TargetSummary>> {
        self.get_json("/api/targets")
    }

    pub fn target_releases(&self, target: &str) -> Result<TargetReleases> {
        self.get_json(&format!("/api/targets/{}", enc(target)))
    }

    pub fn target_detail(&self, release: &str, target: &str) -> Result<TargetDetail> {
        self.get_json(&format!("/api/builds/{}/{}", enc(release), enc(target)))
    }

    pub fn options(&self, release: &str) -> Result<BuildOptions> {
        self.get_json(&format!("/api/options/{}", enc(release)))
    }

    /// USB device filters (`/api/app/devices`), applicable via `UsbIdTable::apply_device_filters`.
    pub fn device_filters(&self) -> Result<Value> {
        self.get_json("/api/app/devices")
    }

    pub fn request_build(&self, req: &BuildRequest) -> Result<BuildResponse> {
        let url = self.url("/api/builds");
        let (status, body) = self.http.post_json(&url, &serde_json::to_value(req)?)?;
        if !Self::ok(status) {
            return Err(FlashError::HttpStatus { status, url });
        }
        Ok(serde_json::from_slice(&body)?)
    }

    pub fn build_status(&self, key: &str) -> Result<BuildStatus> {
        self.get_json(&format!("/api/builds/{}/status", enc(key)))
    }

    pub fn build_log_url(&self, key: &str) -> String {
        self.url(&format!("/api/builds/{}/log", enc(key)))
    }

    /// Poll until "success" (Ok) or another terminal status (BuildFailed). `on_status` sees every
    /// poll result (for UI).
    pub fn wait_for_build(
        &self,
        key: &str,
        opts: &PollOptions,
        on_status: &mut dyn FnMut(&BuildStatus),
        cancel: &AtomicBool,
    ) -> Result<BuildStatus> {
        let started = Instant::now();
        let mut timeout = opts.default_timeout;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(FlashError::Cancelled);
            }
            let s = self.build_status(key)?;
            on_status(&s);
            if let Some(t) = s.time_out {
                timeout = Duration::from_secs(t);
            }
            if s.is_success() {
                return Ok(s);
            }
            if !s.is_queued() {
                return Err(FlashError::BuildFailed {
                    key: key.to_string(),
                    status: s.status,
                });
            }
            if started.elapsed() >= timeout {
                return Err(FlashError::Timeout(format!(
                    "cloud build {key} after {}s",
                    timeout.as_secs()
                )));
            }
            (self.sleep)(opts.interval);
        }
    }

    /// Download firmware by API-relative (`/api/builds/...`) or absolute URL; retries once after
    /// 4 s like the configurator.
    pub fn download(&self, url_or_path: &str) -> Result<Vec<u8>> {
        let url = if url_or_path.starts_with("http://") || url_or_path.starts_with("https://") {
            url_or_path.to_string()
        } else {
            self.url(url_or_path)
        };
        let mut last = None;
        for attempt in 0..2 {
            if attempt > 0 {
                (self.sleep)(Duration::from_secs(4));
            }
            match self.http.get(&url) {
                Ok((s, body)) if Self::ok(s) && !body.is_empty() => return Ok(body),
                Ok((status, _)) => {
                    last = Some(FlashError::HttpStatus {
                        status,
                        url: url.clone(),
                    })
                }
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or(FlashError::Http("download failed".into())))
    }
}

/// Percent-encode a path segment (targets/releases are simple, but be safe).
fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    #[derive(Default)]
    pub(crate) struct MockHttp {
        replies: Mutex<VecDeque<(u16, Vec<u8>)>>,
        pub(crate) calls: Mutex<Vec<String>>,
    }

    impl MockHttp {
        pub(crate) fn reply(self, status: u16, body: &str) -> Self {
            self.replies
                .lock()
                .unwrap()
                .push_back((status, body.as_bytes().to_vec()));
            self
        }
        fn next(&self, call: String) -> Result<(u16, Vec<u8>)> {
            self.calls.lock().unwrap().push(call);
            Ok(self
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or((500, Vec::new())))
        }
    }

    impl HttpClient for MockHttp {
        fn get(&self, url: &str) -> Result<(u16, Vec<u8>)> {
            self.next(format!("GET {url}"))
        }
        fn post_json(&self, url: &str, body: &Value) -> Result<(u16, Vec<u8>)> {
            self.next(format!("POST {url} {body}"))
        }
    }

    fn api(http: MockHttp) -> BuildApi<MockHttp> {
        BuildApi::new(http, "https://build.example/").with_sleep(|_| {})
    }

    #[test]
    fn get_endpoints() {
        let a = api(MockHttp::default()
            .reply(200, r#"[{"target":"STM32F405","manufacturer":"BTFL"}]"#)
            .reply(200, r#"{"releases":[{"release":"4.5.1","type":"Stable"}]}"#)
            .reply(200, r#"{"generalOptions":[]}"#)
            .reply(404, ""));
        assert_eq!(a.targets().unwrap()[0].target, "STM32F405");
        assert_eq!(
            a.target_releases("STM32F405").unwrap().releases[0].release,
            "4.5.1"
        );
        a.options("4.5.1").unwrap();
        assert!(matches!(
            a.target_detail("4.5.1", "NOPE"),
            Err(FlashError::HttpStatus { status: 404, .. })
        ));
        assert_eq!(
            *a.http.calls.lock().unwrap(),
            vec![
                "GET https://build.example/api/targets",
                "GET https://build.example/api/targets/STM32F405",
                "GET https://build.example/api/options/4.5.1",
                "GET https://build.example/api/builds/4.5.1/NOPE",
            ]
        );
    }

    #[test]
    fn request_and_poll_build() {
        let a = api(MockHttp::default()
            .reply(
                201,
                r#"{"key":"abc","url":"/api/builds/abc/hex","file":"fw.hex"}"#,
            )
            .reply(200, r#"{"status":"queued","timeOut":300}"#)
            .reply(200, r#"{"status":"queued"}"#)
            .reply(200, r#"{"status":"success"}"#)
            .reply(500, "")
            .reply(200, ":00000001FF\n"));
        let req = BuildRequest {
            target: "T".into(),
            release: "4.5.1".into(),
            options: vec!["USE_SERIALRX_CRSF".into()],
            commit: None,
            client: None,
        };
        let r = a.request_build(&req).unwrap();
        assert_eq!(r.key, "abc");
        let mut seen = Vec::new();
        let s = a
            .wait_for_build(
                &r.key,
                &PollOptions::default(),
                &mut |s| seen.push(s.status.clone()),
                &AtomicBool::new(false),
            )
            .unwrap();
        assert!(s.is_success());
        assert_eq!(seen, vec!["queued", "queued", "success"]);
        // First download attempt fails, retry succeeds.
        assert_eq!(
            a.download(r.url.as_deref().unwrap()).unwrap(),
            b":00000001FF\n"
        );
        let calls = a.http.calls.lock().unwrap();
        assert!(calls[0].starts_with(r#"POST https://build.example/api/builds {"options":["USE_SERIALRX_CRSF"],"release":"4.5.1","target":"T"}"#));
        assert_eq!(calls[5], "GET https://build.example/api/builds/abc/hex");
    }

    #[test]
    fn build_failure_and_timeout() {
        let a = api(MockHttp::default().reply(200, r#"{"status":"failure"}"#));
        let r = a.wait_for_build(
            "k",
            &PollOptions::default(),
            &mut |_| {},
            &AtomicBool::new(false),
        );
        assert!(
            matches!(r, Err(FlashError::BuildFailed { ref status, .. }) if status == "failure")
        );

        let a = api(MockHttp::default().reply(200, r#"{"status":"queued","timeOut":0}"#));
        let r = a.wait_for_build(
            "k",
            &PollOptions::default(),
            &mut |_| {},
            &AtomicBool::new(false),
        );
        assert!(matches!(r, Err(FlashError::Timeout(_))));
    }

    #[test]
    fn encodes_path_segments() {
        assert_eq!(enc("4.5.1"), "4.5.1");
        assert_eq!(enc("a b/c"), "a%20b%2Fc");
    }
}
