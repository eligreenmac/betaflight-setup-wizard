// SPDX-License-Identifier: GPL-3.0-or-later
//! Glue between the Build API client, the firmware cache, DfuSe flashing and the shared
//! model types. Blocking; the Tauri commands run it on `spawn_blocking`.

use crate::flasher::build_api::{BuildApi, HttpClient, PollOptions};
use crate::flasher::build_types::{BuildRequest, TargetSummary};
use crate::flasher::cache::{CacheEntry, CacheKey, FirmwareCache};
use crate::flasher::dfu::{DfuOptions, DfuPhase, DfuProgress, DfuSe, DfuTransport, EraseMode};
use crate::flasher::error::{FlashError, Result};
use crate::flasher::hex::{parse_intel_hex, HexImage};
use crate::model::{
    BoardTarget, FirmwareFormat, FirmwareImage, FirmwareRelease, FirmwareRequest, FlashOptions,
    FlashProgress, FlashStage,
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

    fn image(entry: &CacheEntry, data: &[u8], from_cache: bool) -> FirmwareImage {
        FirmwareImage {
            request: FirmwareRequest {
                target: entry.key.target.clone(),
                release: entry.key.release.clone(),
                options: entry.key.options.clone(),
                offline_only: false,
            },
            path: entry.path.to_string_lossy().into_owned(),
            format: FirmwareFormat::Hex,
            sha256: sha256_hex(data),
            build_key: entry.source.clone(),
            from_cache,
        }
    }

    /// Cache hit, or (unless `offline_only`) request a cloud build, wait, download, validate
    /// as Intel HEX and cache it. Progress is reported as `DOWNLOADING`.
    pub fn fetch(
        &self,
        req: &FirmwareRequest,
        progress: &mut dyn FnMut(FlashProgress),
        cancel: &AtomicBool,
    ) -> Result<FirmwareImage> {
        let key = CacheKey::new(&req.target, &req.release, &req.options, None);
        if let Some(entry) = self.cache.list()?.into_iter().find(|e| e.key == key) {
            if let Some(data) = self.cache.get(&key)? {
                return Ok(Self::image(&entry, &data, true));
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
        Ok(Self::image(&entry, &data, false))
    }

    pub fn list_cache(&self) -> Result<Vec<FirmwareImage>> {
        let mut out = Vec::new();
        for e in self.cache.list()? {
            match fs::read(&e.path) {
                Ok(data) => out.push(Self::image(&e, &data, true)),
                Err(err) => log::warn!("skipping cache entry {}: {err}", e.path.display()),
            }
        }
        Ok(out)
    }
}

/// Load the image the UI passes back, checking it is the file we produced (sha256).
pub fn load_image(image: &FirmwareImage) -> Result<HexImage> {
    let data = fs::read(Path::new(&image.path))?;
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
        assert!(s.api_calls()[0].contains(r#""client":"betaflight-setup-wizard/"#));

        let again = s
            .fetch(&req(true), &mut |_| {}, &AtomicBool::new(false))
            .unwrap();
        assert!(again.from_cache);
        assert_eq!(again.path, img.path);
        assert_eq!(again.build_key.as_deref(), Some("k1"));
        assert_eq!(s.list_cache().unwrap().len(), 1);
        assert_eq!(load_image(&again).unwrap().bytes_total(), 600);
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
        };
        assert!(load_image(&img).is_ok());
        img.sha256 = "00".repeat(32);
        assert!(matches!(
            load_image(&img),
            Err(FlashError::ChecksumMismatch { .. })
        ));
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
