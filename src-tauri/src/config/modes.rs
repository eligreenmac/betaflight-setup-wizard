//! Wizard-managed flight modes and their permanent box ids (betaflight msp/msp_box.c).

use super::firmware::Dialect;

/// (canonical name, permanent id, minimum dialect)
const MANAGED_MODES: &[(&str, u8, Dialect)] = &[
    ("ARM", 0, Dialect::V4_4),
    ("ANGLE", 1, Dialect::V4_4),
    ("HORIZON", 2, Dialect::V4_4),
    ("ALTHOLD", 3, Dialect::V2025_12), // id 3 was BARO before 2025.12
    ("ANTI GRAVITY", 4, Dialect::V4_4),
    ("POSHOLD", 11, Dialect::V2025_12), // id 11 was GPS HOLD before 2025.12
    ("BEEPER", 13, Dialect::V4_4),
    ("OSD DISABLE", 19, Dialect::V4_4),
    ("BLACKBOX", 26, Dialect::V4_4),
    ("FAILSAFE", 27, Dialect::V4_4),
    ("AIR MODE", 28, Dialect::V4_4),
    ("FLIP OVER AFTER CRASH", 35, Dialect::V4_4),
    ("PREARM", 36, Dialect::V4_4),
    ("VTX PIT MODE", 39, Dialect::V4_4),
    ("PARALYZE", 45, Dialect::V4_4),
    ("GPS RESCUE", 46, Dialect::V4_4),
    ("LAUNCH CONTROL", 49, Dialect::V4_4),
    ("BEEPER MUTE", 52, Dialect::V4_4),
];

fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Resolve a mode name ("ARM", "AIRMODE", "flip_over_after_crash", "TURTLE", ...)
/// to `(canonical name, permanent id)` for `dialect`.
pub fn resolve(name: &str, dialect: Dialect) -> Result<(&'static str, u8), String> {
    let key = match squash(name).as_str() {
        "TURTLE" | "FLIP" | "FLIPOVERAFTERCRASH" => squash("FLIP OVER AFTER CRASH"),
        "ALTITUDEHOLD" => "ALTHOLD".to_string(),
        "POSITIONHOLD" => "POSHOLD".to_string(),
        k => k.to_string(),
    };
    let (canon, id, min) = MANAGED_MODES
        .iter()
        .find(|(n, _, _)| squash(n) == key)
        .ok_or_else(|| format!("unknown or unmanaged flight mode '{name}'"))?;
    if dialect < *min {
        return Err(format!(
            "mode '{canon}' needs Betaflight {} or newer",
            min.as_str()
        ));
    }
    Ok((canon, *id))
}

/// Is this permanent id one the wizard manages (for `dialect`)?
pub fn is_managed_id(id: u8, dialect: Dialect) -> bool {
    MANAGED_MODES
        .iter()
        .any(|(_, i, min)| *i == id && dialect >= *min)
}

pub fn name_of(id: u8) -> Option<&'static str> {
    MANAGED_MODES.iter().find(|(_, i, _)| *i == id).map(|m| m.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases() {
        assert_eq!(resolve("ARM", Dialect::V4_5).unwrap().1, 0);
        assert_eq!(resolve("air_mode", Dialect::V4_5).unwrap().1, 28);
        assert_eq!(resolve("Turtle", Dialect::V4_4).unwrap().1, 35);
        assert!(resolve("ALTHOLD", Dialect::V4_5).is_err());
        assert_eq!(resolve("ALTHOLD", Dialect::V2025_12).unwrap().1, 3);
        assert!(!is_managed_id(3, Dialect::V4_5));
        assert!(resolve("MAG", Dialect::V4_5).is_err());
    }
}
