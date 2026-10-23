//! Backup store: `diff all` text files with a small metadata header, kept forever at
//! `<root>/<boardUid>/<UTC YYYYMMDDTHHMMSSZ>.txt` (root = `<app_data_dir>/backups`).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{AppError, AppResult};
use crate::model::Backup;

const MAGIC: &str = "# bfsw-backup v1";

pub struct BackupStore {
    root: PathBuf,
}

/// (compact id timestamp "20261023T091500Z", RFC 3339 "2026-10-23T09:15:00Z")
pub fn utc_stamps(t: SystemTime) -> (String, String) {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let (hh, mm, ss) = (sod / 3600, (sod % 3600) / 60, sod % 60);
    (
        format!("{y:04}{m:02}{d:02}T{hh:02}{mm:02}{ss:02}Z"),
        format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z"),
    )
}

fn sanitize(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        "unknown".into()
    } else {
        out
    }
}

impl BackupStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        BackupStore { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Write `diff` (raw `diff all` output) with a metadata header.
    pub fn save(
        &self,
        diff: &str,
        board_uid: &str,
        board_name: &str,
        firmware_version: &str,
        now: SystemTime,
    ) -> AppResult<Backup> {
        let uid = sanitize(board_uid);
        let (mut stamp, created_at) = utc_stamps(now);
        let dir = self.root.join(&uid);
        fs::create_dir_all(&dir)?;
        // Never overwrite: two backups in the same second get a suffix.
        let mut path = dir.join(format!("{stamp}.txt"));
        let mut n = 1;
        while path.exists() {
            stamp = format!("{}-{n}", utc_stamps(now).0);
            path = dir.join(format!("{stamp}.txt"));
            n += 1;
        }
        let text = format!(
            "{MAGIC}\n# board_name: {board_name}\n# board_uid: {uid}\n# firmware: {firmware_version}\n# created_at: {created_at}\n\n{diff}"
        );
        fs::write(&path, text)?;
        Ok(Backup {
            id: format!("{uid}/{stamp}"),
            path: path.to_string_lossy().into_owned(),
            board_uid: uid,
            board_name: board_name.to_string(),
            firmware_version: firmware_version.to_string(),
            created_at,
        })
    }

    fn read_meta(path: &Path, id: String) -> Option<Backup> {
        let text = fs::read_to_string(path).ok()?;
        if !text.starts_with(MAGIC) {
            return None;
        }
        let field = |k: &str| {
            text.lines()
                .take(8)
                .find_map(|l| l.strip_prefix(&format!("# {k}: ")).map(str::to_string))
                .unwrap_or_default()
        };
        Some(Backup {
            id,
            path: path.to_string_lossy().into_owned(),
            board_uid: field("board_uid"),
            board_name: field("board_name"),
            firmware_version: field("firmware"),
            created_at: field("created_at"),
        })
    }

    /// All backups, newest first.
    pub fn list(&self) -> AppResult<Vec<Backup>> {
        let mut out = Vec::new();
        let Ok(boards) = fs::read_dir(&self.root) else {
            return Ok(out);
        };
        for b in boards.flatten() {
            if !b.path().is_dir() {
                continue;
            }
            let uid = b.file_name().to_string_lossy().into_owned();
            for f in fs::read_dir(b.path())?.flatten() {
                let p = f.path();
                if p.extension().and_then(|e| e.to_str()) != Some("txt") {
                    continue;
                }
                let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                if let Some(bk) = Self::read_meta(&p, format!("{uid}/{stem}")) {
                    out.push(bk);
                }
            }
        }
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
        Ok(out)
    }

    pub fn find(&self, id: &str) -> AppResult<Backup> {
        self.list()?
            .into_iter()
            .find(|b| b.id == id)
            .ok_or_else(|| AppError::Config(format!("backup '{id}' not found")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn stamps() {
        let t = UNIX_EPOCH + Duration::from_secs(1_792_746_900); // 2026-10-23T09:15:00Z
        assert_eq!(
            utc_stamps(t),
            ("20261023T091500Z".into(), "2026-10-23T09:15:00Z".into())
        );
    }

    #[test]
    fn save_list_find() {
        let dir = std::env::temp_dir().join(format!("bfsw-bk-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let store = BackupStore::new(&dir);
        let t = UNIX_EPOCH + Duration::from_secs(1_792_746_900);
        let a = store
            .save("batch start\n", "ABC/123", "X", "4.5.2", t)
            .unwrap();
        let b = store
            .save(
                "batch start\n",
                "ABC/123",
                "X",
                "4.5.2",
                t + Duration::from_secs(60),
            )
            .unwrap();
        let c = store
            .save("batch start\n", "ABC/123", "X", "4.5.2", t)
            .unwrap();
        assert_eq!(a.id, "ABC_123/20261023T091500Z");
        assert_ne!(a.id, c.id);
        let all = store.list().unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].id, b.id);
        assert_eq!(store.find(&a.id).unwrap().firmware_version, "4.5.2");
        assert!(fs::read_to_string(&a.path).unwrap().contains("batch start"));
        let _ = fs::remove_dir_all(&dir);
    }
}
