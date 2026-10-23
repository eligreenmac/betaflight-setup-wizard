//! `vtxtable` state as Betaflight keeps it (cli.c `cliVtxTable` / `printVtxTable`,
//! identical in 4.4.3, 4.5.5 and 2025.12.5). Used to render the generator's commands,
//! parse the table out of `dump all` for verify, and simulate it in MockFc.

use crate::model::VtxTable;

/// drivers/vtx_table.h
pub const MAX_BANDS: usize = 8;
pub const MAX_CHANNELS: usize = 8;
pub const MAX_POWER_LEVELS: usize = 8;
pub const BAND_NAME_LENGTH: usize = 8;
pub const POWER_LABEL_LENGTH: usize = 3;
/// VTX_SETTINGS_MAX_FREQUENCY_MHZ
pub const MAX_FREQUENCY_MHZ: u16 = 5999;
// `vtx_power` is 1-based but its CLI range is 0..VTX_TABLE_MAX_POWER_LEVELS-1:
// model::VTX_POWER_MAX (checked against these constants in model's tests).

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Band {
    pub name: String,
    pub letter: char,
    pub factory: bool,
    pub frequencies: Vec<u16>,
}

/// The FC's table. Counts and contents follow the firmware's update rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VtxTableState {
    pub bands: usize,
    pub channels: usize,
    /// MAX_BANDS entries, each with MAX_CHANNELS frequencies.
    pub band: Vec<Band>,
    pub power_levels: usize,
    pub power_values: Vec<u16>,
    pub power_labels: Vec<String>,
}

impl Default for VtxTableState {
    fn default() -> Self {
        VtxTableState {
            bands: 0,
            channels: 0,
            band: vec![
                Band {
                    name: String::new(),
                    letter: ' ',
                    factory: false,
                    frequencies: vec![0; MAX_CHANNELS],
                };
                MAX_BANDS
            ],
            power_levels: 0,
            power_values: vec![0; MAX_POWER_LEVELS],
            power_labels: vec![String::new(); MAX_POWER_LEVELS],
        }
    }
}

impl VtxTableState {
    /// Apply one `vtxtable ...` line (without the verb). Err = the firmware's error text.
    pub fn apply(&mut self, args: &str) -> Result<(), String> {
        let mut t = args.split_whitespace();
        let Some(sub) = t.next() else {
            return Ok(());
        };
        let count = |t: Option<&str>, max: usize, what: &str| -> Result<usize, String> {
            match t.and_then(|x| x.parse::<i64>().ok()) {
                Some(n) if (0..=max as i64).contains(&n) => Ok(n as usize),
                _ => Err(format!("{what} NOT BETWEEN 0 AND {max}")),
            }
        };
        match sub.to_ascii_lowercase().as_str() {
            "bands" => {
                let n = count(t.next(), MAX_BANDS, "BAND COUNT")?;
                for b in self.band.iter_mut().skip(n) {
                    *b = Band {
                        frequencies: vec![0; MAX_CHANNELS],
                        letter: ' ',
                        ..Band::default()
                    };
                }
                self.bands = n;
            }
            "channels" => {
                let n = count(t.next(), MAX_CHANNELS, "CHANNEL COUNT")?;
                for b in &mut self.band {
                    for f in b.frequencies.iter_mut().skip(n) {
                        *f = 0;
                    }
                }
                self.channels = n;
            }
            "powerlevels" => {
                let n = count(t.next(), MAX_POWER_LEVELS, "POWER LEVEL COUNT")?;
                for i in n..MAX_POWER_LEVELS {
                    self.power_values[i] = 0;
                    self.power_labels[i].clear();
                }
                self.power_levels = n;
            }
            "powervalues" => {
                let v: Vec<u16> = t.map(|x| x.parse().unwrap_or(0)).collect();
                if v.len() < self.power_levels {
                    return Err(format!(
                        "NOT ENOUGH VALUES (EXPECTED {})",
                        self.power_levels
                    ));
                }
                if v.len() > self.power_levels {
                    return Err(format!("TOO MANY VALUES (EXPECTED {})", self.power_levels));
                }
                self.power_values = v;
                self.power_values.resize(MAX_POWER_LEVELS, 0);
            }
            "powerlabels" => {
                let v: Vec<String> = t
                    .map(|x| x.chars().take(POWER_LABEL_LENGTH).collect::<String>())
                    .map(|x| x.to_ascii_uppercase())
                    .collect();
                if v.len() < self.power_levels {
                    return Err(format!(
                        "NOT ENOUGH LABELS (EXPECTED {})",
                        self.power_levels
                    ));
                }
                if v.len() > self.power_levels {
                    return Err(format!("TOO MANY LABELS (EXPECTED {})", self.power_levels));
                }
                for (i, l) in v.into_iter().enumerate() {
                    self.power_labels[i] = l;
                }
            }
            "band" => {
                let idx = t.next().and_then(|x| x.parse::<usize>().ok()).unwrap_or(0);
                if idx == 0 || idx > self.bands {
                    return Err(format!("BAND NUMBER NOT BETWEEN 1 AND {}", self.bands));
                }
                let (Some(name), Some(letter)) = (t.next(), t.next()) else {
                    return Err("INVALID ARGUMENT COUNT".into());
                };
                let mut factory = false;
                let mut freqs = Vec::new();
                for (i, tok) in t.enumerate() {
                    if i == 0 && !tok.starts_with(|c: char| c.is_ascii_digit()) {
                        if tok.eq_ignore_ascii_case("FACTORY") {
                            factory = true;
                        } else if !tok.eq_ignore_ascii_case("CUSTOM") {
                            return Err(format!(
                                "INVALID FACTORY FLAG {tok} (EXPECTED FACTORY OR CUSTOM)"
                            ));
                        }
                        continue;
                    }
                    freqs.push(
                        tok.parse::<u16>()
                            .map_err(|_| format!("INVALID FREQUENCY {tok}"))?,
                    );
                }
                if freqs.len() < self.channels {
                    return Err(format!(
                        "NOT ENOUGH FREQUENCIES (EXPECTED {})",
                        self.channels
                    ));
                }
                if freqs.len() > self.channels {
                    return Err(format!("TOO MANY FREQUENCIES (EXPECTED {})", self.channels));
                }
                let b = &mut self.band[idx - 1];
                b.name = name
                    .chars()
                    .take(BAND_NAME_LENGTH)
                    .collect::<String>()
                    .to_ascii_uppercase();
                b.letter = letter.chars().next().unwrap_or(' ').to_ascii_uppercase();
                b.factory = factory;
                for (i, f) in freqs.into_iter().enumerate() {
                    b.frequencies[i] = f;
                }
            }
            _ => return Err(format!("INVALID ARGUMENT {sub}")),
        }
        Ok(())
    }

    /// The `vtxtable` block as `dump` prints it (single-spaced).
    pub fn render(&self) -> Vec<String> {
        let mut o = vec![
            format!("vtxtable bands {}", self.bands),
            format!("vtxtable channels {}", self.channels),
        ];
        for (i, b) in self.band.iter().take(self.bands).enumerate() {
            let mut l = format!(
                "vtxtable band {} {} {} {}",
                i + 1,
                b.name,
                b.letter,
                if b.factory { "FACTORY" } else { "CUSTOM" }
            );
            for f in b.frequencies.iter().take(self.channels) {
                l += &format!(" {f}");
            }
            o.push(l);
        }
        o.push(format!("vtxtable powerlevels {}", self.power_levels));
        let mut v = "vtxtable powervalues".to_string();
        let mut l = "vtxtable powerlabels".to_string();
        for i in 0..self.power_levels {
            v += &format!(" {}", self.power_values[i]);
            l += &format!(" {}", self.power_labels[i]);
        }
        o.push(v);
        o.push(l);
        o
    }

    /// The commands that load `t` (in the order the firmware needs: counts first).
    pub fn commands_for(t: &VtxTable) -> Vec<String> {
        let mut s = VtxTableState::default();
        let channels = t.bands.first().map_or(0, |b| b.frequencies.len());
        let mut cmds = vec![
            format!("vtxtable bands {}", t.bands.len()),
            format!("vtxtable channels {channels}"),
        ];
        for (i, b) in t.bands.iter().enumerate() {
            let mut l = format!(
                "vtxtable band {} {} {} {}",
                i + 1,
                b.name,
                b.letter,
                if b.factory { "FACTORY" } else { "CUSTOM" }
            );
            for f in &b.frequencies {
                l += &format!(" {f}");
            }
            cmds.push(l);
        }
        cmds.push(format!("vtxtable powerlevels {}", t.power_levels.len()));
        let mut v = "vtxtable powervalues".to_string();
        let mut l = "vtxtable powerlabels".to_string();
        for p in &t.power_levels {
            v += &format!(" {}", p.value);
            l += &format!(" {}", p.label);
        }
        cmds.push(v);
        cmds.push(l);
        debug_assert!(cmds
            .iter()
            .all(|c| s.apply(&c["vtxtable ".len()..]).is_ok()));
        cmds
    }

    /// State after sending `commands_for(t)` to a fresh table (for verify).
    pub fn from_model(t: &VtxTable) -> VtxTableState {
        let mut s = VtxTableState::default();
        for c in Self::commands_for(t) {
            let _ = s.apply(&c["vtxtable ".len()..]);
        }
        s
    }

    /// Same content as the FC would report, ignoring entries beyond the counts.
    pub fn same_as(&self, other: &VtxTableState) -> bool {
        self.render() == other.render()
    }
}

/// Validate a model table against the firmware limits. Err = message for CONFIG.
pub fn validate(t: &VtxTable) -> Result<(), String> {
    if t.bands.is_empty() || t.bands.len() > MAX_BANDS {
        return Err(format!(
            "VTX table: 1-{MAX_BANDS} bands, got {}",
            t.bands.len()
        ));
    }
    let channels = t.bands[0].frequencies.len();
    if channels == 0 || channels > MAX_CHANNELS {
        return Err(format!(
            "VTX table: 1-{MAX_CHANNELS} channels per band, got {channels}"
        ));
    }
    for b in &t.bands {
        let name_ok = !b.name.is_empty()
            && b.name.len() <= BAND_NAME_LENGTH
            && b.name.chars().all(|c| c.is_ascii_graphic());
        if !name_ok {
            return Err(format!(
                "VTX table: band name {:?} must be 1-{BAND_NAME_LENGTH} printable characters without spaces",
                b.name
            ));
        }
        let mut letter = b.letter.chars();
        if !matches!((letter.next(), letter.next()), (Some(c), None) if c.is_ascii_alphanumeric()) {
            return Err(format!(
                "VTX table: band {}: letter must be one character",
                b.name
            ));
        }
        if b.frequencies.len() != channels {
            return Err(format!(
                "VTX table: band {} has {} channels, expected {channels} (same for every band)",
                b.name,
                b.frequencies.len()
            ));
        }
        if let Some(f) = b.frequencies.iter().find(|&&f| f > MAX_FREQUENCY_MHZ) {
            return Err(format!(
                "VTX table: band {}: {f} MHz is above {MAX_FREQUENCY_MHZ}",
                b.name
            ));
        }
    }
    let n = t.power_levels.len();
    if n == 0 || n > MAX_POWER_LEVELS {
        return Err(format!(
            "VTX table: 1-{MAX_POWER_LEVELS} power levels, got {n}"
        ));
    }
    for p in &t.power_levels {
        let ok = !p.label.is_empty()
            && p.label.len() <= POWER_LABEL_LENGTH
            && p.label.chars().all(|c| c.is_ascii_graphic());
        if !ok {
            return Err(format!(
                "VTX table: power label {:?} must be 1-{POWER_LABEL_LENGTH} printable characters without spaces",
                p.label
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dump_format_and_round_trips() {
        let mut s = VtxTableState::default();
        for l in [
            "bands 2",
            "channels 3",
            "band 1 BOSCAM_A A FACTORY 5865 5845 5825",
            "band 2 raceband r CUSTOM  5658 5695 5732",
            "powerlevels 2",
            "powervalues 14 20",
            "powerlabels 25 100",
        ] {
            s.apply(l).unwrap();
        }
        assert_eq!(
            s.render(),
            [
                "vtxtable bands 2",
                "vtxtable channels 3",
                "vtxtable band 1 BOSCAM_A A FACTORY 5865 5845 5825",
                "vtxtable band 2 RACEBAND R CUSTOM 5658 5695 5732",
                "vtxtable powerlevels 2",
                "vtxtable powervalues 14 20",
                "vtxtable powerlabels 25 100",
            ]
        );
        assert!(s.apply("band 3 X X 1 2 3").is_err());
        assert!(s.apply("band 1 X X 1 2").is_err());
        assert!(s.apply("powervalues 1 2 3").is_err());
    }
}
