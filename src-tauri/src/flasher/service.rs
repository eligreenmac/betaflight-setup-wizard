// SPDX-License-Identifier: GPL-3.0-or-later
//! Glue between the Build API client, the firmware cache, DfuSe flashing and the shared
//! model types. Blocking; the Tauri commands run it on `spawn_blocking`.

use crate::flasher::build_api::{BuildApi, HttpClient, PollOptions};
use crate::flasher::build_types::{BuildOptions, BuildRequest, TargetSummary};
use crate::flasher::cache::{CacheEntry, CacheKey, FirmwareCache};
use crate::flasher::dfu::{DfuOptions, DfuPhase, DfuProgress, DfuSe, DfuTransport, EraseMode};
use crate::flasher::error::{FlashError, Result};
use crate::flasher::hex::{parse_intel_hex, HexImage};
use crate::flasher::options;
use crate::model::{
    BoardTarget, FirmwareFormat, FirmwareImage, FirmwareRelease, FirmwareRequest,
    FirmwareRequirements, FlashOptions, FlashProgress, FlashStage,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

/// Default load address of `.bin` images (STM32/AT32 internal flash).
pub const BIN_LOAD_ADDRESS: u32 = 0x0800_0000;
/// Sent as `client` in build requests.
pub const BUILD_CLIENT: &str = concat!("betaflight-setup-wizard/", env!("CARGO_PKG_VERSION"));

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Build API + on-disk cache under `root` (`<app_data_dir>/firmware`). Target and release
/// lists are cached as JSON under `root/api/` so the wizard works offline.
pub struct FirmwareService<C: HttpClient> {
    api: BuildApi<C>,
    cache: FirmwareCache,
    root: PathBuf,
    poll: PollOptions,
}

impl<C: HttpClient> FirmwareService<C> {
    pub fn new(api: BuildApi<C>, root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let cache = FirmwareCache::new(&root)?;
        Ok(FirmwareService {
            api,
            cache,
            root,
            poll: PollOptions::default(),
        })
    }

    pub fn with_poll(mut self, poll: PollOptions) -> Self {
        self.poll = poll;
        self
    }

    fn api_cache_path(&self, name: &str) -> PathBuf {
        let safe: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.root.join("api").join(format!("{safe}.json"))
    }

    /// Online result (and refresh the stored copy), else the stored copy, else the error.
    fn online_or_cached<T: Serialize + DeserializeOwned>(
        &self,
        name: &str,
        fetch: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let path = self.api_cache_path(name);
        match fetch() {
            Ok(v) => {
                if let Some(dir) = path.parent() {
                    fs::create_dir_all(dir)?;
                }
                fs::write(&path, serde_json::to_vec(&v)?)?;
                Ok(v)
            }
            Err(e) => match fs::read(&path) {
                Ok(b) => {
                    log::warn!("Build API unavailable ({e}); using cached {name}");
                    Ok(serde_json::from_slice(&b)?)
                }
                Err(_) => Err(e),
            },
        }
    }

    pub fn list_targets(&self) -> Result<Vec<BoardTarget>> {
        let targets: Vec<TargetSummary> =
            self.online_or_cached("targets", || self.api.targets())?;
        Ok(targets
            .into_iter()
            .map(|t| BoardTarget {
                mcu: t
                    .extra
                    .get("mcu")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                target: t.target,
                manufacturer: t.manufacturer,
            })
            .collect())
    }

    pub fn list_releases(&self, target: &str) -> Result<Vec<FirmwareRelease>> {
        let r = self.online_or_cached(&format!("releases-{target}"), || {
            self.api.target_releases(target)
        })?;
        Ok(r.releases
            .into_iter()
            .filter(|r| !r.withdrawn)
            .map(|r| FirmwareRelease {
                prerelease: !r.is_stable(),
                release: r.release,
            })
            .collect())
    }

    /// The release's Build API option list (`/api/options/{release}`): online (and stored under
    /// `api/options-<release>.json`), else the stored copy. `offline_only`: the stored copy only
    /// (`NotCached` without one). No usable list -> `OptionsUnavailable` (never an empty one:
    /// that would strip every board option from the build).
    pub fn release_options(&self, release: &str, offline_only: bool) -> Result<BuildOptions> {
        let name = format!("options-{release}");
        let list: BuildOptions = if offline_only {
            let stored = fs::read(self.api_cache_path(&name))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok());
            stored.ok_or_else(|| {
                FlashError::NotCached(format!(
                    "the build options of {release} are not in the offline cache"
                ))
            })?
        } else {
            self.online_or_cached(&name, || self.api.options(release))
                .map_err(|e| FlashError::OptionsUnavailable {
                    release: release.to_string(),
                    reason: e.to_string(),
                })?
        };
        if options::offered(&list).is_empty() {
            return Err(FlashError::OptionsUnavailable {
                release: release.to_string(),
                reason: "the option list is empty".into(),
            });
        }
        Ok(list)
    }

    /// `flasher_required_options`: the options the wizard's answers need; with `release`, also
    /// checked against that release's list.
    pub fn required_options(
        &self,
        requirements: &FirmwareRequirements,
        release: Option<&str>,
    ) -> Result<Vec<String>> {
        let names = options::required_options(requirements);
        if let (Some(release), false) = (release, names.is_empty()) {
            let list = self.release_options(release, false)?;
            options::check_required(release, &names, &list)?;
        }
        Ok(names)
    }

    fn image(
        entry: &CacheEntry,
        data: &[u8],
        from_cache: bool,
        dropped_options: Vec<String>,
    ) -> FirmwareImage {
        FirmwareImage {
            request: FirmwareRequest {
                target: entry.key.target.clone(),
                release: entry.key.release.clone(),
                options: entry.key.options.clone(),
                required_options: Vec::new(),
                offline_only: false,
            },
            path: entry.path.to_string_lossy().into_owned(),
            format: FirmwareFormat::Hex,
            sha256: sha256_hex(data),
            build_key: entry.source.clone(),
            from_cache,
            dropped_options,
        }
    }

    /// Resolve the options against the release's list (ARCHITECTURE §6 "Build options"), then
    /// a cache hit, or (unless `offline_only`) request a cloud build, wait, download, validate
    /// as Intel HEX and cache it. Progress is reported as `DOWNLOADING`. Fails before any build
    /// with `OptionsUnavailable` (no option list) or `RequiredOptionUnsupported`.
    pub fn fetch(
        &self,
        req: &FirmwareRequest,
        progress: &mut dyn FnMut(FlashProgress),
        cancel: &AtomicBool,
    ) -> Result<FirmwareImage> {
        let list = self.release_options(&req.release, req.offline_only)?;
        let resolved = options::resolve(&req.release, &req.options, &req.required_options, &list)?;
        let dropped = resolved.dropped.clone();
        let key = CacheKey::new(&req.target, &req.release, &resolved.options, None);
        if let Some(entry) = self.cache.list()?.into_iter().find(|e| e.key == key) {
            if let Some(data) = self.cache.get(&key)? {
                return Ok(Self::image(&entry, &data, true, dropped));
            }
        }
        if req.offline_only {
            return Err(FlashError::NotCached(format!(
                "{} {} is not in the offline cache",
                req.target, req.release
            )));
        }
        let mut report = |fraction: f32, message: String| {
            progress(FlashProgress {
                stage: FlashStage::Downloading,
                fraction,
                message: Some(message),
            })
        };
        if !dropped.is_empty() {
            report(
                0.0,
                format!("not available in {}: {}", req.release, dropped.join(", ")),
            );
        }
        report(0.0, "requesting cloud build".into());
        let build = self.api.request_build(&BuildRequest {
            target: req.target.clone(),
            release: req.release.clone(),
            options: key.options.clone(),
            commit: None,
            client: Some(BUILD_CLIENT.into()),
        })?;
        self.api.wait_for_build(
            &build.key,
            &self.poll,
            &mut |s| report(0.1, format!("cloud build {}", s.status)),
            cancel,
        )?;
        report(0.5, "downloading firmware".into());
        let url = build
            .url
            .clone()
            .unwrap_or_else(|| format!("/api/builds/{}/hex", build.key));
        let data = self.api.download(&url)?;
        let text = std::str::from_utf8(&data).map_err(|_| FlashError::Hex {
            line: 0,
            reason: "not UTF-8 text".into(),
        })?;
        parse_intel_hex(text)?;
        let entry = self.cache.put(&key, &data, Some(&build.key))?;
        report(1.0, "firmware cached".into());
        Ok(Self::image(&entry, &data, false, dropped))
    }

    /// [`load_image`] restricted to this service's cache directory.
    pub fn load_image(&self, image: &FirmwareImage) -> Result<HexImage> {
        load_image(image, self.cache.root())
    }

    pub fn list_cache(&self) -> Result<Vec<FirmwareImage>> {
        let mut out = Vec::new();
        for e in self.cache.list()? {
            match fs::read(&e.path) {
                Ok(data) => out.push(Self::image(&e, &data, true, Vec::new())),
                Err(err) => log::warn!("skipping cache entry {}: {err}", e.path.display()),
            }
        }
        Ok(out)
    }
}

/// Resolve `path` (symlinks, `..`) and require it to be a file inside `root` (also
/// resolved). Returns the canonical path; anything else is `PathOutsideCache`.
pub fn ensure_in_cache(root: &Path, path: &Path) -> Result<PathBuf> {
    let outside = || FlashError::PathOutsideCache(path.display().to_string());
    let root = fs::canonicalize(root).map_err(|_| outside())?;
    let canonical = fs::canonicalize(path).map_err(|_| outside())?;
    if canonical == root || !canonical.starts_with(&root) || !canonical.is_file() {
        return Err(outside());
    }
    Ok(canonical)
}

/// Load the image the UI passes back: it must be a file in the firmware cache `root`
/// (see [`ensure_in_cache`]) and the file we produced (sha256).
pub fn load_image(image: &FirmwareImage, root: &Path) -> Result<HexImage> {
    let path = ensure_in_cache(root, Path::new(&image.path))?;
    let data = fs::read(&path)?;
    let actual = sha256_hex(&data);
    if !actual.eq_ignore_ascii_case(&image.sha256) {
        return Err(FlashError::ChecksumMismatch {
            expected: image.sha256.clone(),
            actual,
        });
    }
    match image.format {
        FirmwareFormat::Hex => {
            let text = std::str::from_utf8(&data).map_err(|_| FlashError::Hex {
                line: 0,
                reason: "not UTF-8 text".into(),
            })?;
            parse_intel_hex(text)
        }
        FirmwareFormat::Bin => Ok(HexImage::from_bin(BIN_LOAD_ADDRESS, data)),
        FirmwareFormat::Uf2 => Err(FlashError::Usb(
            "UF2 images are flashed by copying to the RP2040 drive, not over DFU".into(),
        )),
    }
}

pub fn dfu_options(o: &FlashOptions) -> DfuOptions {
    DfuOptions {
        erase: if o.full_erase {
            EraseMode::Full
        } else {
            EraseMode::Pages
        },
        verify: true,
        leave: o.manifest,
        check_read_protection: true,
    }
}

pub fn map_progress(p: DfuProgress) -> FlashProgress {
    let fraction = |done: u64, total: u64| {
        if total == 0 {
            0.0
        } else {
            (done as f32 / total as f32).clamp(0.0, 1.0)
        }
    };
    let (stage, fraction) = match p.phase {
        DfuPhase::Detect => (FlashStage::Erasing, 0.0),
        DfuPhase::Erase => (FlashStage::Erasing, fraction(p.done, p.total)),
        DfuPhase::Write => (FlashStage::Writing, fraction(p.done, p.total)),
        DfuPhase::Verify => (FlashStage::Verifying, fraction(p.done, p.total)),
        DfuPhase::Leave => (FlashStage::Rebooting, 0.0),
        DfuPhase::Done => (FlashStage::Done, 1.0),
    };
    FlashProgress {
        stage,
        fraction,
        message: None,
    }
}

/// Flash over any transport. Read protection is never removed here: a protected chip fails
/// with `ReadProtected` before anything is erased.
pub fn flash<T: DfuTransport>(
    dfu: &mut DfuSe<T>,
    image: &HexImage,
    options: &FlashOptions,
    progress: &mut dyn FnMut(FlashProgress),
    cancel: &AtomicBool,
) -> Result<()> {
    let report = dfu.flash(
        image,
        &dfu_options(options),
        &mut |p| progress(map_progress(p)),
        cancel,
    )?;
    log::info!("flash done: {report:?}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flasher::build_api::tests::MockHttp;
    use crate::flasher::dfu::sim::SimDfu;
    use crate::flasher::hex::to_intel_hex;
    use crate::flasher::options::tests::{OPTIONS_2025_12_2, OPTIONS_4_4_3, OPTIONS_4_5_2};

    fn svc(http: MockHttp, dir: &Path) -> FirmwareService<MockHttp> {
        FirmwareService::new(
            BuildApi::new(http, "https://build.example").with_sleep(|_| {}),
            dir,
        )
        .unwrap()
    }

    fn req(offline: bool) -> FirmwareRequest {
        FirmwareRequest {
            target: "STM32F405".into(),
            release: "4.5.2".into(),
            options: vec!["USE_GPS".into()],
            required_options: vec![],
            offline_only: offline,
        }
    }

    fn hex() -> String {
        to_intel_hex(&HexImage::from_bin(
            0x0800_0000,
            (0..600u32).map(|i| i as u8).collect(),
        ))
    }

    #[test]
    fn targets_and_releases_fall_back_to_cached_copy_offline() {
        let dir = tempfile::tempdir().unwrap();
        let online = svc(
            MockHttp::default()
                .reply(200, r#"[{"target":"STM32F405","manufacturer":"BTFL","mcu":"STM32F405"}]"#)
                .reply(200, r#"{"releases":[{"release":"4.5.2","type":"Stable"},{"release":"2025.12.0-RC1","type":"ReleaseCandidate"},{"release":"4.5.0","type":"Stable","withdrawn":true}]}"#),
            dir.path(),
        );
        let t = online.list_targets().unwrap();
        assert_eq!(t[0].mcu.as_deref(), Some("STM32F405"));
        let r = online.list_releases("STM32F405").unwrap();
        assert_eq!(r.len(), 2);
        assert!(!r[0].prerelease && r[1].prerelease);

        let offline = svc(MockHttp::default(), dir.path()); // every request -> HTTP 500
        assert_eq!(offline.list_targets().unwrap()[0].target, "STM32F405");
        assert_eq!(offline.list_releases("STM32F405").unwrap().len(), 2);
        assert!(offline.list_releases("OTHER").is_err());
    }

    #[test]
    fn fetch_builds_downloads_caches_then_hits_cache() {
        let dir = tempfile::tempdir().unwrap();
        let s = svc(
            MockHttp::default()
                .reply(200, OPTIONS_4_5_2)
                .reply(201, r#"{"key":"k1","url":"/api/builds/k1/hex"}"#)
                .reply(200, r#"{"status":"success"}"#)
                .reply(200, &hex()),
            dir.path(),
        );
        let mut stages = Vec::new();
        let img = s
            .fetch(
                &req(false),
                &mut |p| stages.push(p.fraction),
                &AtomicBool::new(false),
            )
            .unwrap();
        assert!(!img.from_cache);
        assert_eq!(img.build_key.as_deref(), Some("k1"));
        assert_eq!(img.sha256, sha256_hex(hex().as_bytes()));
        assert_eq!(stages.last(), Some(&1.0));
        assert!(s.api.base_url().ends_with("example"));
        assert_eq!(
            s.api_calls()[0],
            "GET https://build.example/api/options/4.5.2"
        );
        assert!(s.api_calls()[1].contains(r#""client":"betaflight-setup-wizard/"#));
        assert!(img.dropped_options.is_empty());

        let again = s
            .fetch(&req(true), &mut |_| {}, &AtomicBool::new(false))
            .unwrap();
        assert!(again.from_cache);
        assert_eq!(again.path, img.path);
        assert_eq!(again.build_key.as_deref(), Some("k1"));
        assert_eq!(s.list_cache().unwrap().len(), 1);
        assert_eq!(s.load_image(&again).unwrap().bytes_total(), 600);
    }

    #[test]
    fn offline_only_miss_and_invalid_download() {
        let dir = tempfile::tempdir().unwrap();
        let s = svc(MockHttp::default(), dir.path());
        let e = s
            .fetch(&req(true), &mut |_| {}, &AtomicBool::new(false))
            .unwrap_err();
        assert!(matches!(e, FlashError::NotCached(_)));
        assert!(
            s.api_calls().is_empty(),
            "offline-only must not touch the network"
        );

        let s = svc(
            MockHttp::default()
                .reply(200, OPTIONS_4_5_2)
                .reply(201, r#"{"key":"k2"}"#)
                .reply(200, r#"{"status":"success"}"#)
                .reply(200, "<html>oops</html>"),
            dir.path(),
        );
        assert!(matches!(
            s.fetch(&req(false), &mut |_| {}, &AtomicBool::new(false)),
            Err(FlashError::Hex { .. })
        ));
        assert!(
            s.list_cache().unwrap().is_empty(),
            "invalid firmware is not cached"
        );
    }

    fn req_with(release: &str, board: &[&str], required: &[&str]) -> FirmwareRequest {
        FirmwareRequest {
            target: "STM32F405".into(),
            release: release.into(),
            options: board.iter().map(|o| o.to_string()).collect(),
            required_options: required.iter().map(|o| o.to_string()).collect(),
            offline_only: false,
        }
    }

    /// The JSON body of the build request (the POST call).
    fn posted_options(s: &FirmwareService<MockHttp>) -> Vec<String> {
        let calls = s.api_calls();
        let post = calls
            .iter()
            .find(|c| c.starts_with("POST "))
            .expect("a build request");
        let body: serde_json::Value =
            serde_json::from_str(post.splitn(3, ' ').nth(2).unwrap()).unwrap();
        serde_json::from_value(body["options"].clone()).unwrap()
    }

    fn build_replies(http: MockHttp) -> MockHttp {
        http.reply(201, r#"{"key":"k","url":"/api/builds/k/hex"}"#)
            .reply(200, r#"{"status":"success"}"#)
            .reply(200, &hex())
    }

    #[test]
    fn fetch_filters_board_options_and_adds_required_ones() {
        let dir = tempfile::tempdir().unwrap();
        let s = svc(
            build_replies(MockHttp::default().reply(200, OPTIONS_4_5_2)),
            dir.path(),
        );
        let board = [
            "USE_SERIALRX_CRSF",
            "USE_DSHOT",
            "USE_OSD_SD",
            "USE_WING",
            "USE_ALTITUDE_HOLD",
        ];
        let mut messages = Vec::new();
        let img = s
            .fetch(
                &req_with("4.5.2", &board, &["USE_OSD_HD"]),
                &mut |p| messages.extend(p.message),
                &AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(img.dropped_options, ["USE_WING", "USE_ALTITUDE_HOLD"]);
        assert!(
            messages[0].contains("USE_WING, USE_ALTITUDE_HOLD"),
            "{messages:?}"
        );
        let mut sent = posted_options(&s);
        sent.sort();
        assert_eq!(
            sent,
            ["USE_DSHOT", "USE_OSD_HD", "USE_OSD_SD", "USE_SERIALRX_CRSF"]
        );
        assert_eq!(img.request.options, sent, "cache key = resolved options");

        // Offline: the stored option list is reused and the same request hits the cache.
        let offline = svc(MockHttp::default(), dir.path());
        let mut again = req_with("4.5.2", &board, &["USE_OSD_HD"]);
        again.offline_only = true;
        let hit = offline
            .fetch(&again, &mut |_| {}, &AtomicBool::new(false))
            .unwrap();
        assert!(hit.from_cache);
        assert_eq!(hit.dropped_options, ["USE_WING", "USE_ALTITUDE_HOLD"]);
        assert!(offline.api_calls().is_empty());
    }

    #[test]
    fn fetch_without_option_list_fails_and_never_builds() {
        for (status, body) in [(503, ""), (200, "<html>maintenance</html>"), (200, "{}")] {
            let dir = tempfile::tempdir().unwrap();
            let s = svc(MockHttp::default().reply(status, body), dir.path());
            let e = s
                .fetch(
                    &req_with("4.5.2", &["USE_DSHOT"], &[]),
                    &mut |_| {},
                    &AtomicBool::new(false),
                )
                .unwrap_err();
            assert!(
                matches!(&e, FlashError::OptionsUnavailable { release, .. } if release == "4.5.2"),
                "{status} {body}: {e:?}"
            );
            assert_eq!(
                crate::error::AppError::from(e).code(),
                "BUILD_OPTIONS_UNAVAILABLE"
            );
            assert!(
                s.api_calls().iter().all(|c| !c.starts_with("POST ")),
                "no build without the option list"
            );
        }
        // Network error (transport) too: MockHttp answers 500 when it has no reply left.
        let dir = tempfile::tempdir().unwrap();
        let s = svc(MockHttp::default(), dir.path());
        assert!(matches!(
            s.fetch(
                &req_with("4.4.3", &[], &["USE_OSD_HD"]),
                &mut |_| {},
                &AtomicBool::new(false)
            ),
            Err(FlashError::OptionsUnavailable { .. })
        ));
    }

    #[test]
    fn fetch_refuses_a_required_option_the_release_lacks() {
        let dir = tempfile::tempdir().unwrap();
        let s = svc(MockHttp::default().reply(200, OPTIONS_4_4_3), dir.path());
        let e = s
            .fetch(
                &req_with("4.4.3", &["USE_DSHOT"], &["USE_RACE_PRO"]),
                &mut |_| {},
                &AtomicBool::new(false),
            )
            .unwrap_err();
        let v = serde_json::to_value(crate::error::AppError::from(e)).unwrap();
        assert_eq!(v["code"], "REQUIRED_OPTION_UNSUPPORTED");
        assert_eq!(
            v["details"],
            serde_json::json!({"release": "4.4.3", "options": ["USE_RACE_PRO"]})
        );
        assert_eq!(s.api_calls().len(), 1, "only the option list was fetched");
    }

    #[test]
    fn fetch_for_a_2025_12_board_on_older_releases() {
        let board: Vec<String> =
            options::offered(&serde_json::from_str(OPTIONS_2025_12_2).unwrap())
                .into_iter()
                .collect();
        let board: Vec<&str> = board.iter().map(String::as_str).collect();
        for (release, list, dropped) in [
            (
                "4.5.2",
                OPTIONS_4_5_2,
                &[
                    "USE_ALTITUDE_HOLD",
                    "USE_POSITION_HOLD",
                    "USE_SOFTSERIAL",
                    "USE_WING",
                ][..],
            ),
            (
                "4.4.3",
                OPTIONS_4_4_3,
                &[
                    "USE_ALTITUDE_HOLD",
                    "USE_POSITION_HOLD",
                    "USE_RACE_PRO",
                    "USE_SOFTSERIAL",
                    "USE_WING",
                ][..],
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let s = svc(
                build_replies(MockHttp::default().reply(200, list)),
                dir.path(),
            );
            let img = s
                .fetch(
                    &req_with(release, &board, &[]),
                    &mut |_| {},
                    &AtomicBool::new(false),
                )
                .unwrap();
            assert_eq!(img.dropped_options, dropped, "{release}");
            let sent = posted_options(&s);
            assert!(dropped.iter().all(|d| !sent.iter().any(|o| o == d)));
            assert_eq!(sent.len() + dropped.len(), board.len());
        }
    }

    #[test]
    fn required_options_command_checks_the_release() {
        use crate::model::{OsdRequirement, ReceiverKind, ReceiverRequirement, RxProtocol};
        let reqs = FirmwareRequirements {
            receiver: Some(ReceiverRequirement {
                kind: ReceiverKind::Serial,
                protocol: Some(RxProtocol::Sbus),
            }),
            osd: Some(OsdRequirement {
                video_system: Some(crate::model::OsdVideoSystem::Hd),
            }),
        };
        let dir = tempfile::tempdir().unwrap();
        let s = svc(MockHttp::default().reply(200, OPTIONS_4_4_3), dir.path());
        assert_eq!(
            s.required_options(&reqs, None).unwrap(),
            ["USE_SERIALRX_SBUS", "USE_OSD_HD"]
        );
        assert!(s.api_calls().is_empty(), "no release: no network");
        assert_eq!(
            s.required_options(&reqs, Some("4.4.3")).unwrap(),
            ["USE_SERIALRX_SBUS", "USE_OSD_HD"]
        );
        // An unreachable API with no stored list.
        let s = svc(MockHttp::default(), tempfile::tempdir().unwrap().path());
        assert!(matches!(
            s.required_options(&reqs, Some("4.5.2")),
            Err(FlashError::OptionsUnavailable { .. })
        ));
    }

    #[test]
    fn load_image_rejects_modified_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("fw.hex");
        fs::write(&p, hex()).unwrap();
        let mut img = FirmwareImage {
            request: req(true),
            path: p.to_string_lossy().into_owned(),
            format: FirmwareFormat::Hex,
            sha256: sha256_hex(hex().as_bytes()),
            build_key: None,
            from_cache: true,
            dropped_options: vec![],
        };
        assert!(load_image(&img, dir.path()).is_ok());
        img.sha256 = "00".repeat(32);
        assert!(matches!(
            load_image(&img, dir.path()),
            Err(FlashError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn load_image_only_accepts_files_inside_the_cache() {
        let cache = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let root = cache.path().join("firmware");
        fs::create_dir_all(root.join("STM32F405")).unwrap();
        let inside = root.join("STM32F405").join("fw.hex");
        let outside = other.path().join("evil.hex");
        fs::write(&inside, hex()).unwrap();
        fs::write(&outside, hex()).unwrap();
        let img = |p: &Path| FirmwareImage {
            request: req(true),
            path: p.to_string_lossy().into_owned(),
            format: FirmwareFormat::Hex,
            sha256: sha256_hex(hex().as_bytes()),
            build_key: None,
            from_cache: true,
            dropped_options: vec![],
        };
        let rejected = |p: &Path| {
            let e = load_image(&img(p), &root).unwrap_err();
            assert!(matches!(e, FlashError::PathOutsideCache(_)), "{p:?}: {e:?}");
            assert_eq!(crate::error::AppError::from(e).code(), "INVALID_ARGUMENT");
        };

        assert!(load_image(&img(&inside), &root).is_ok());
        // `..` is resolved before the check.
        assert!(load_image(&img(&root.join("STM32F405/../STM32F405/fw.hex")), &root).is_ok());
        rejected(&outside);
        // `..` escape to an existing file (both tempdirs share the temp dir as parent).
        let dotdot = root
            .join("..")
            .join("..")
            .join(other.path().file_name().unwrap())
            .join("evil.hex");
        assert!(dotdot.is_file());
        rejected(&dotdot);
        rejected(&root.join("STM32F405")); // a directory
        rejected(&root); // the root itself
        rejected(&root.join("missing.hex"));
        rejected(Path::new("relative/fw.hex"));
        // A sibling dir whose name merely starts with the root's name.
        let sibling = cache.path().join("firmware-evil");
        fs::create_dir_all(&sibling).unwrap();
        fs::write(sibling.join("fw.hex"), hex()).unwrap();
        rejected(&sibling.join("fw.hex"));

        #[cfg(unix)]
        {
            // A symlink inside the cache pointing outside it.
            let link = root.join("STM32F405").join("link.hex");
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            rejected(&link);
            // A symlinked directory inside the cache pointing outside it.
            let dir_link = root.join("escape");
            std::os::unix::fs::symlink(other.path(), &dir_link).unwrap();
            rejected(&dir_link.join("evil.hex"));
            // A symlink to a file inside the cache is fine.
            let ok_link = root.join("ok.hex");
            std::os::unix::fs::symlink(&inside, &ok_link).unwrap();
            assert!(load_image(&img(&ok_link), &root).is_ok());
        }
    }

    #[test]
    fn flash_reports_shared_stages_and_refuses_protected_chip() {
        let image = parse_intel_hex(&hex()).unwrap();
        let opts = FlashOptions {
            full_erase: false,
            manifest: true,
        };
        let mut dfu = DfuSe::new(SimDfu::f405()).with_sleep(|_| {});
        let mut seen = Vec::new();
        flash(
            &mut dfu,
            &image,
            &opts,
            &mut |p| seen.push(p.stage),
            &AtomicBool::new(false),
        )
        .unwrap();
        let has = |s: FlashStage| {
            seen.iter()
                .any(|x| std::mem::discriminant(x) == std::mem::discriminant(&s))
        };
        assert!(has(FlashStage::Erasing) && has(FlashStage::Writing) && has(FlashStage::Verifying));
        assert!(has(FlashStage::Rebooting) && matches!(seen.last(), Some(FlashStage::Done)));
        assert!(dfu.transport_mut().left);

        let mut sim = SimDfu::f405();
        sim.read_protected = true;
        let mut dfu = DfuSe::new(sim).with_sleep(|_| {});
        let e = flash(
            &mut dfu,
            &image,
            &opts,
            &mut |_| {},
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(matches!(e, FlashError::ReadProtected));
        assert!(
            dfu.transport_mut().erased_pages().is_empty(),
            "nothing erased on a protected chip"
        );
    }

    impl FirmwareService<MockHttp> {
        fn api_calls(&self) -> Vec<String> {
            self.api.http().calls.lock().unwrap().clone()
        }
    }
}
