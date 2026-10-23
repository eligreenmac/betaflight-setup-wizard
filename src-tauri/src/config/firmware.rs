//! Firmware line detection and the MVP support gate (4.4.x, 4.5.x, 2025.12.x, 2026.6.x).
//!
//! Single source of truth for which Betaflight release lines the app accepts:
//! [`SUPPORTED_LINES`], [`is_supported`] and [`supported_lines_label`]. Device/UI code should
//! call these rather than keep their own version lists.

use crate::error::{AppError, AppResult};

/// Supported `(major, minor)` release lines, oldest first. Any patch release of a line is
/// accepted; everything else (older than 4.4, newer than 2026.6, unknown lines) is refused.
pub const SUPPORTED_LINES: &[(u16, u16)] = &[(4, 4), (4, 5), (2025, 12), (2026, 6)];

/// `(major, minor)` of a version string such as "2026.6.2" / "4.5" (patch and any suffix
/// after the minor are ignored); `None` when unparseable.
fn major_minor(version: &str) -> Option<(u16, u16)> {
    let mut it = version.trim().split('.');
    let major = it.next()?.parse::<u16>().ok()?;
    let minor = it.next()?.parse::<u16>().ok()?;
    Some((major, minor))
}

/// True when `version` ("2026.6.2", "4.5.5", ...) belongs to one of [`SUPPORTED_LINES`].
/// Unparseable input is `false`.
pub fn is_supported(version: &str) -> bool {
    major_minor(version).is_some_and(|mm| SUPPORTED_LINES.contains(&mm))
}

/// Human-readable list of the supported lines, e.g. "4.4.x, 4.5.x, 2025.12.x and 2026.6.x".
pub fn supported_lines_label() -> String {
    let items: Vec<String> = SUPPORTED_LINES
        .iter()
        .map(|(ma, mi)| format!("{ma}.{mi}.x"))
        .collect();
    match items.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

/// CLI dialect. Ordered by release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Dialect {
    V4_4,
    V4_5,
    V2025_12,
    V2026_6,
}

impl Dialect {
    pub const ALL: [Dialect; 4] = [
        Dialect::V4_4,
        Dialect::V4_5,
        Dialect::V2025_12,
        Dialect::V2026_6,
    ];

    /// `(major, minor)` release line of this dialect (an entry of [`SUPPORTED_LINES`]).
    pub fn line(self) -> (u16, u16) {
        match self {
            Dialect::V4_4 => (4, 4),
            Dialect::V4_5 => (4, 5),
            Dialect::V2025_12 => (2025, 12),
            Dialect::V2026_6 => (2026, 6),
        }
    }

    /// Map a firmware version ("4.5.2", "2026.6.2") to its dialect, refusing
    /// anything outside [`SUPPORTED_LINES`] with `UNSUPPORTED_FIRMWARE`.
    pub fn from_version(version: &str) -> AppResult<Dialect> {
        let v = version.trim();
        let Some(mm) = major_minor(v) else {
            return Err(AppError::UnsupportedFirmware(format!(
                "cannot parse firmware version '{v}'"
            )));
        };
        Dialect::ALL
            .into_iter()
            .find(|d| d.line() == mm)
            .ok_or_else(|| {
                AppError::UnsupportedFirmware(format!(
                    "Betaflight {v} is not supported; supported lines are {}",
                    supported_lines_label()
                ))
            })
    }

    /// Value used in `CliScript.dialect`.
    pub fn as_str(self) -> &'static str {
        match self {
            Dialect::V4_4 => "4.4",
            Dialect::V4_5 => "4.5",
            Dialect::V2025_12 => "2025.12",
            Dialect::V2026_6 => "2026.6",
        }
    }

    pub fn from_dialect_str(s: &str) -> Option<Dialect> {
        Dialect::ALL.into_iter().find(|d| d.as_str() == s)
    }

    /// 2025.12+ prints and accepts port names in `serial` (PR #14094).
    pub fn uses_port_names(self) -> bool {
        self >= Dialect::V2025_12
    }

    /// `feature RX_MSP` has no CLI name in 2025.12; 2026.6 restores it (cli.c featureNames,
    /// next to the new RX_UDP).
    pub fn has_rx_msp_feature_name(self) -> bool {
        self != Dialect::V2025_12
    }

    /// ESC idle setting (`dshot_idle_value` renamed to `motor_idle` in 2025.12, PR #13936).
    pub fn idle_setting(self) -> &'static str {
        if self >= Dialect::V2025_12 {
            "motor_idle"
        } else {
            "dshot_idle_value"
        }
    }

    /// 2026.6 per-profile battery settings (`battery_profile N` sections in `dump all`) and
    /// the flight-plan `waypoint` section.
    pub fn has_battery_profiles(self) -> bool {
        self >= Dialect::V2026_6
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
        for v in ["2026.6.0", "2026.6.2", " 2026.6.9 "] {
            assert_eq!(Dialect::from_version(v).unwrap(), Dialect::V2026_6, "{v}");
        }
    }

    #[test]
    fn dialects_cover_supported_lines() {
        let lines: Vec<_> = Dialect::ALL.iter().map(|d| d.line()).collect();
        assert_eq!(lines, SUPPORTED_LINES);
        for d in Dialect::ALL {
            assert_eq!(Dialect::from_dialect_str(d.as_str()), Some(d));
        }
    }

    #[test]
    fn is_supported_versions() {
        for v in [
            "4.4.0",
            "4.4.3",
            "4.5.5",
            "2025.12.5",
            "2026.6.0",
            "2026.6.2",
            "2026.6.0-RC1",
            "2026.6",
        ] {
            assert!(is_supported(v), "{v}");
        }
        for v in [
            "4.3.2",
            "4.6.0",
            "2025.6.0",
            "2026.12.0",
            "2026.12.0-alpha",
            "2026.12.1-RC1",
            "2027.1.0",
            "2026",
            "",
            "garbage",
            "x.6.2",
            "2026.y.2",
            "-4.4.0",
        ] {
            assert!(!is_supported(v), "{v}");
        }
    }

    #[test]
    fn label_and_message() {
        assert_eq!(
            supported_lines_label(),
            "4.4.x, 4.5.x, 2025.12.x and 2026.6.x"
        );
        let e = Dialect::from_version("2026.12.0").unwrap_err();
        assert!(
            e.to_string()
                .contains("4.4.x, 4.5.x, 2025.12.x and 2026.6.x"),
            "{e}"
        );
    }

    #[test]
    fn dialect_flags() {
        use Dialect::*;
        assert!(!V4_5.uses_port_names() && V2025_12.uses_port_names() && V2026_6.uses_port_names());
        assert!(V4_5.has_rx_msp_feature_name());
        assert!(!V2025_12.has_rx_msp_feature_name());
        assert!(V2026_6.has_rx_msp_feature_name());
        assert_eq!(V4_5.idle_setting(), "dshot_idle_value");
        assert_eq!(V2026_6.idle_setting(), "motor_idle");
        assert!(V2026_6.has_battery_profiles() && !V2025_12.has_battery_profiles());
    }

    #[test]
    fn refuses_others() {
        for v in ["4.3.2", "4.6.0", "2026.12.0", "2027.1.0", "garbage"] {
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
