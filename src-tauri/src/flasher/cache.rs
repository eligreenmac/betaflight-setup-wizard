// SPDX-License-Identifier: GPL-3.0-or-later
//! On-disk firmware cache keyed by target + release + build options (+ commit).
//!
//! Layout: `<root>/<target>/<release>/<digest16>.hex` plus `<digest16>.json` metadata.
//! The digest is SHA-256 over a canonical string with options sorted and de-duplicated, so
//! option order in the UI doesn't change the key. Writes are atomic (temp file + rename).

use crate::flasher::error::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheKey {
    pub target: String,
    pub release: String,
    /// Normalised: sorted, de-duplicated.
    pub options: Vec<String>,
    pub commit: Option<String>,
}

impl CacheKey {
    pub fn new(target: &str, release: &str, options: &[String], commit: Option<&str>) -> Self {
        let mut options: Vec<String> = options
            .iter()
            .map(|o| o.trim().to_string())
            .filter(|o| !o.is_empty())
            .collect();
        options.sort();
        options.dedup();
        CacheKey {
            target: target.to_string(),
            release: release.to_string(),
            options,
            commit: commit.map(str::to_string),
        }
    }

    fn canonical(&self) -> String {
        format!(
            "v1\ntarget={}\nrelease={}\ncommit={}\noptions={}",
            self.target,
            self.release,
            self.commit.as_deref().unwrap_or(""),
            self.options.join(",")
        )
    }

    /// Full hex SHA-256 of the canonical form.
    pub fn digest(&self) -> String {
        let h = Sha256::digest(self.canonical().as_bytes());
        h.iter().fold(String::with_capacity(64), |mut s, b| {
            use std::fmt::Write;
            let _ = write!(s, "{b:02x}");
            s
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheEntry {
    pub key: CacheKey,
    pub digest: String,
    pub file_name: String,
    pub size: u64,
    /// Unix seconds.
    pub stored_at: u64,
    /// Build key or URL it came from, if known.
    pub source: Option<String>,
    #[serde(skip)]
    pub path: PathBuf,
}

pub struct FirmwareCache {
    root: PathBuf,
}

/// Keep path components safe on Windows and Unix.
fn sanitize(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let t = t.trim_matches('.').to_string();
    if t.is_empty() {
        "_".into()
    } else {
        t
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl FirmwareCache {
    /// `root` is e.g. the Tauri app cache dir + "/firmware".
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(FirmwareCache { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn dir_for(&self, key: &CacheKey) -> PathBuf {
        self.root
            .join(sanitize(&key.target))
            .join(sanitize(&key.release))
    }

    pub fn path_for(&self, key: &CacheKey) -> PathBuf {
        self.dir_for(key)
            .join(format!("{}.hex", &key.digest()[..16]))
    }

    fn meta_path(&self, key: &CacheKey) -> PathBuf {
        self.path_for(key).with_extension("json")
    }

    /// Cached firmware bytes, if present and its metadata matches the key.
    pub fn get(&self, key: &CacheKey) -> Result<Option<Vec<u8>>> {
        let meta = self.meta_path(key);
        let Ok(m) = fs::read(&meta) else {
            return Ok(None);
        };
        let entry: CacheEntry = match serde_json::from_slice(&m) {
            Ok(e) => e,
            Err(_) => return Ok(None),
        };
        if entry.key != *key {
            return Ok(None); // digest prefix collision
        }
        match fs::read(self.path_for(key)) {
            Ok(b) if b.len() as u64 == entry.size => Ok(Some(b)),
            Ok(_) => Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn put(&self, key: &CacheKey, data: &[u8], source: Option<&str>) -> Result<CacheEntry> {
        let dir = self.dir_for(key);
        fs::create_dir_all(&dir)?;
        let path = self.path_for(key);
        atomic_write(&path, data)?;
        let entry = CacheEntry {
            key: key.clone(),
            digest: key.digest(),
            file_name: path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default(),
            size: data.len() as u64,
            stored_at: now_secs(),
            source: source.map(str::to_string),
            path: path.clone(),
        };
        atomic_write(&self.meta_path(key), &serde_json::to_vec_pretty(&entry)?)?;
        Ok(entry)
    }

    pub fn remove(&self, key: &CacheKey) -> Result<bool> {
        let mut removed = false;
        for p in [self.path_for(key), self.meta_path(key)] {
            match fs::remove_file(&p) {
                Ok(()) => removed = true,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(removed)
    }

    /// All valid entries, newest first.
    pub fn list(&self) -> Result<Vec<CacheEntry>> {
        let mut out = Vec::new();
        for t in read_dirs(&self.root)? {
            for r in read_dirs(&t)? {
                for f in fs::read_dir(&r)?.flatten() {
                    let p = f.path();
                    if p.extension().and_then(|e| e.to_str()) != Some("json") {
                        continue;
                    }
                    let Ok(bytes) = fs::read(&p) else { continue };
                    let Ok(mut e) = serde_json::from_slice::<CacheEntry>(&bytes) else {
                        continue;
                    };
                    e.path = p.with_extension("hex");
                    if e.path.exists() {
                        out.push(e);
                    }
                }
            }
        }
        out.sort_by(|a, b| {
            b.stored_at
                .cmp(&a.stored_at)
                .then_with(|| a.digest.cmp(&b.digest))
        });
        Ok(out)
    }

    /// Delete entries older than `max_age`, then the oldest beyond `max_entries`.
    /// Returns how many were removed.
    pub fn prune(&self, max_age: Option<Duration>, max_entries: Option<usize>) -> Result<usize> {
        let now = now_secs();
        let mut removed = 0;
        let mut keep = Vec::new();
        for e in self.list()? {
            if max_age.is_some_and(|a| now.saturating_sub(e.stored_at) > a.as_secs()) {
                removed += self.remove(&e.key)? as usize;
            } else {
                keep.push(e);
            }
        }
        if let Some(n) = max_entries {
            for e in keep.iter().skip(n) {
                removed += self.remove(&e.key)? as usize;
            }
        }
        Ok(removed)
    }
}

fn read_dirs(p: &Path) -> Result<Vec<PathBuf>> {
    let mut v = Vec::new();
    match fs::read_dir(p) {
        Ok(rd) => {
            for e in rd.flatten() {
                if e.path().is_dir() {
                    v.push(e.path());
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(v)
}

fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&tmp, data)?;
    // On Windows rename fails if the target exists.
    #[cfg(windows)]
    {
        let _ = fs::remove_file(path);
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn key_is_order_insensitive() {
        let a = CacheKey::new("STM32F405", "4.5.1", &opts(&["B", "A", "A"]), None);
        let b = CacheKey::new("STM32F405", "4.5.1", &opts(&["A", "B"]), None);
        assert_eq!(a, b);
        assert_eq!(a.digest(), b.digest());
        assert_eq!(a.digest().len(), 64);
        assert_ne!(
            a.digest(),
            CacheKey::new("STM32F405", "4.5.2", &opts(&["A", "B"]), None).digest()
        );
        assert_ne!(
            a.digest(),
            CacheKey::new("STM32F405", "4.5.1", &opts(&["A"]), None).digest()
        );
        assert_ne!(
            a.digest(),
            CacheKey::new("STM32F405", "4.5.1", &opts(&["A", "B"]), Some("abc")).digest()
        );
    }

    #[test]
    fn put_get_list_remove() {
        let dir = tempfile::tempdir().unwrap();
        let c = FirmwareCache::new(dir.path().join("fw")).unwrap();
        let k = CacheKey::new(
            "SPEEDYBEEF405V4",
            "4.5.1",
            &opts(&["USE_SERIALRX_CRSF"]),
            None,
        );
        assert_eq!(c.get(&k).unwrap(), None);
        let e = c.put(&k, b":00000001FF\n", Some("abc")).unwrap();
        assert!(e
            .path
            .starts_with(dir.path().join("fw/SPEEDYBEEF405V4/4.5.1")));
        assert_eq!(c.get(&k).unwrap().as_deref(), Some(&b":00000001FF\n"[..]));
        let l = c.list().unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].key, k);
        assert_eq!(l[0].source.as_deref(), Some("abc"));
        // Truncated file is a miss.
        fs::write(c.path_for(&k), b"x").unwrap();
        assert_eq!(c.get(&k).unwrap(), None);
        assert!(c.remove(&k).unwrap());
        assert!(c.list().unwrap().is_empty());
        assert!(!c.remove(&k).unwrap());
    }

    #[test]
    fn sanitizes_paths_and_prunes() {
        let dir = tempfile::tempdir().unwrap();
        let c = FirmwareCache::new(dir.path()).unwrap();
        let evil = CacheKey::new("../..\\x", "4.5:1", &[], None);
        let e = c.put(&evil, b"1", None).unwrap();
        assert!(e.path.starts_with(dir.path()));
        for i in 0..3 {
            c.put(
                &CacheKey::new("T", &format!("4.5.{i}"), &[], None),
                b"22",
                None,
            )
            .unwrap();
        }
        assert_eq!(c.list().unwrap().len(), 4);
        assert_eq!(c.prune(None, Some(2)).unwrap(), 2);
        assert_eq!(c.list().unwrap().len(), 2);
        assert_eq!(c.prune(Some(Duration::from_secs(3600)), None).unwrap(), 0);
    }
}
