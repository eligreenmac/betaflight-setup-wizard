//! Firmware line detection and the MVP support gate (4.4.x, 4.5.x, 2025.12.x).

use crate::error::{AppError, AppResult};

/// CLI dialect. Ordered by release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Dialect {
    V4_4,
    V4_5,
    V2025_12,
}

impl Dialect {
    pub const ALL: [Dialect; 3] = [Dialect::V4_4, Dialect::V4_5, Dialect::V2025_12];

    /// Map a firmware version ("4.5.2", "2025.12.1") to its dialect, refusing
    /// anything outside the supported lines with `UNSUPPORTED_FIRMWARE`.
    pub fn from_version(version: &str) -> AppResult<Dialect> {
        let v = version.trim();
        let mut it = v.split('.');
        let major = it.next().and_then(|s| s.parse::<u32>().ok());
        let minor = it.next().and_then(|s| s.parse::<u32>().ok());
        match (major, minor) {
            (Some(4), Some(4)) => Ok(Dialect::V4_4),
            (Some(4), Some(5)) => Ok(Dialect::V4_5),
            (Some(2025), Some(12)) => Ok(Dialect::V2025_12),
            (Some(_), Some(_)) => Err(AppError::UnsupportedFirmware(format!(
                "Betaflight {v} is not supported; supported lines are 4.4.x, 4.5.x and 2025.12.x"
            ))),
            _ => Err(AppError::UnsupportedFirmware(format!(
                "cannot parse firmware version '{v}'"
            ))),
        }
    }

    /// Value used in `CliScript.dialect`.
    pub fn as_str(self) -> &'static str {
        match self {
            Dialect::V4_4 => "4.4",
            Dialect::V4_5 => "4.5",
            Dialect::V2025_12 => "2025.12",
        }
    }

    pub fn from_dialect_str(s: &str) -> Option<Dialect> {
        Dialect::ALL.into_iter().find(|d| d.as_str() == s)
    }

    /// 2025.12 prints and accepts port names in `serial` (PR #14094).
    pub fn uses_port_names(self) -> bool {
        self == Dialect::V2025_12
    }

    /// `feature RX_MSP` has no CLI name in 2025.12.
    pub fn has_rx_msp_feature_name(self) -> bool {
        self != Dialect::V2025_12
    }

    /// ESC idle setting (`dshot_idle_value` renamed to `motor_idle` in 2025.12, PR #13936).
    pub fn idle_setting(self) -> &'static str {
        match self {
            Dialect::V2025_12 => "motor_idle",
            _ => "dshot_idle_value",
        }
    }
}

/// Parsed `version` CLI output / `# Betaflight /` header of a dump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionInfo {
    pub version: String,
    pub board_name: Option<String>,
}

/// Parse e.g. `# Betaflight / STM32F405 (S405) 4.5.2 Jun  4 2025 / 09:35:49 (024f8e13d) MSP API: 1.46`
/// plus an optional `board_name: X` / `board_name X` line.
pub fn parse_version(text: &str) -> Option<VersionInfo> {
    let mut version = None;
    let mut board_name = None;
    for line in text.lines() {
        let l = line.trim().trim_start_matches('#').trim();
        if version.is_none() && l.starts_with("Betaflight") {
            version = l
                .split_whitespace()
                .find(|t| {
                    let parts: Vec<_> = t.split('.').collect();
                    parts.len() == 3
                        && parts
                            .iter()
                            .all(|p| p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty())
                })
                .map(str::to_string);
        }
        if let Some(i) = l.find("board_name") {
            let rest = l[i + "board_name".len()..].trim_start_matches([':', ' ']);
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                board_name = Some(name);
            }
        }
    }
    version.map(|version| VersionInfo {
        version,
        board_name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_lines() {
        assert_eq!(Dialect::from_version("4.4.3").unwrap(), Dialect::V4_4);
        assert_eq!(Dialect::from_version("4.5.2").unwrap(), Dialect::V4_5);
        assert_eq!(
            Dialect::from_version("2025.12.1").unwrap(),
            Dialect::V2025_12
        );
    }

    #[test]
    fn refuses_others() {
        for v in ["4.3.2", "2026.6.0", "2026.6.2", "2027.1.0", "garbage"] {
            let e = Dialect::from_version(v).unwrap_err();
            assert_eq!(e.code(), "UNSUPPORTED_FIRMWARE", "{v}");
        }
    }

    #[test]
    fn parses_version_banner() {
        let t = "# version\n# Betaflight / STM32F405 (S405) 4.5.2 Jun  4 2025 / 09:35:49 (024f8e13d) MSP API: 1.46\n# config: manufacturer_id: SPBE, board_name: SPEEDYBEEF405V4, version: 0e3b2b8c, date: 2023-04-21T01:19:31Z\n";
        let v = parse_version(t).unwrap();
        assert_eq!(v.version, "4.5.2");
        assert_eq!(v.board_name.as_deref(), Some("SPEEDYBEEF405V4"));
    }
}
