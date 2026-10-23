//! What apply and rollback are allowed to send to the FC.
//!
//! Everything goes through an allowlist of command verbs. A line is sent only if its first
//! word (matched case-insensitively, as Betaflight does) is in the list for that path and
//! the line is a single canonical CLI line (printable ASCII, no leading/trailing spaces, no
//! CR/LF/TAB, not a comment). Persisting, rebooting, wiping and batch control are owned by
//! `session` (`batch start` / `save` / `exit`), never by the script or the backup.
//!
//! Apply ([`validate_script`]) accepts only the verbs the generator emits. Rollback
//! ([`replay_lines`]) accepts the verbs `diff all` emits on 4.4 / 4.5 / 2025.12
//! (`printConfig` in `src/main/cli/cli.c`) with the rules documented on [`replay_lines`].

use crate::error::{AppError, AppResult};

/// Verbs `generate` emits. Add a verb here only together with the generator code that
/// emits it (a test checks every generated script against this list).
pub const APPLY_VERBS: &[&str] = &["set", "serial", "feature", "aux"];

/// Verbs `diff all` emits that restore configuration (cli.c `printConfig`, identical in
/// 4.4.3, 4.5.5 and 2025.12.5), besides the ones [`replay_lines`] handles specially.
pub const REPLAY_VERBS: &[&str] = &[
    "resource",
    "timer",
    "dma",
    "feature",
    "serial",
    "mixer",
    "mmix",
    "smix",
    "servo",
    "beeper",
    "beacon",
    "map",
    "led",
    "color",
    "mode_color",
    "aux",
    "adjrange",
    "rxrange",
    "vtxtable",
    "vtx",
    "rxfail",
    "set",
    "profile",
    "rateprofile",
];

/// Board identity lines in a `diff all`. Never replayed: `board_name`, `manufacturer_id`
/// and `signature` with an argument write the board information once (refused if it
/// differs from what the FC has), and `mcu_id` only prints the MCU UID.
pub const BOARD_IDENTITY_VERBS: &[&str] = &["board_name", "manufacturer_id", "mcu_id", "signature"];

/// Why a known-dangerous verb is refused (for the error message). `None` = simply not
/// on the allowlist.
fn danger(verb: &str) -> Option<&'static str> {
    Some(match verb {
        "save" => "persists the configuration and reboots the FC",
        "defaults" => "resets the configuration to defaults",
        "reboot" | "exit" => "reboots the FC",
        "bl" | "dfu" => "reboots the FC into the bootloader",
        "msc" => "reboots the FC into mass-storage mode",
        "batch" => "controls the CLI batch, which apply/rollback own",
        "restore" => "restores a configuration outside the batch",
        "bind_rx" | "bind_rx_spi" => "puts the receiver into bind mode",
        "diff" | "dump" => "is a read command",
        "play_sound" => "drives the beeper",
        v if v.starts_with("flash") => "writes or erases the onboard flash",
        _ => return None,
    })
}

/// Lower-cased first word of a canonical line.
fn verb(line: &str) -> String {
    line.split(' ').next().unwrap_or("").to_ascii_lowercase()
}

fn reject(what: &str, line_no: usize, line: &str, why: &str) -> AppError {
    AppError::Config(format!(
        "{what} line {line_no} `{}` was refused: {why}; nothing was sent to the FC",
        line.escape_debug()
    ))
}

/// Shape check shared by both paths.
fn shape(line: &str) -> Result<(), &'static str> {
    if line.is_empty() {
        return Err("empty line");
    }
    if line.chars().any(|c| !(' '..='~').contains(&c)) {
        return Err("contains a control or non-ASCII character (one CLI line per entry)");
    }
    if line != line.trim() {
        return Err("leading or trailing whitespace");
    }
    if line.starts_with('#') {
        return Err("comment / CLI-mode character");
    }
    Ok(())
}

fn check(line: &str, allowed: &[&str], what: &str, line_no: usize) -> AppResult<()> {
    shape(line).map_err(|why| reject(what, line_no, line, why))?;
    let v = verb(line);
    if allowed.contains(&v.as_str()) {
        return Ok(());
    }
    let why = danger(&v)
        .map(|d| format!("`{v}` {d}"))
        .unwrap_or_else(|| format!("`{v}` is not a command this path sends"));
    Err(reject(what, line_no, line, &why))
}

/// Apply: every line of a planned script must be a generator verb. Called before
/// anything is sent. Errors are CONFIG and name the offending line (1-based).
pub fn validate_script(lines: &[String]) -> AppResult<()> {
    for (i, l) in lines.iter().enumerate() {
        check(l, APPLY_VERBS, "script", i + 1)?;
    }
    Ok(())
}

/// Rollback: the lines to replay from a stored `diff all`, validated. Called before
/// anything is sent; any line outside these rules rejects the whole backup (CONFIG,
/// naming the backup line).
///
/// Each line is trimmed of CR and surrounding spaces (Betaflight prints e.g. `signature `
/// with a trailing space); tabs and other control characters are rejected.
///
/// Dropped (session sends its own `batch start`, `save` or `exit`):
/// - blank lines and `#` comments (version banner, section headings, `# name: ...`);
/// - the echoed `diff all` command, if it is the first line;
/// - `batch start`, only before the first command;
/// - a final `save`, then a final `batch end` before it (`diff all` ends with `save` on
///   4.4 / 4.5 / 2025.12; older captures with `batch end` + `save`);
/// - `board_name`, `manufacturer_id`, `mcu_id`, `signature` (any arguments) – see
///   [`BOARD_IDENTITY_VERBS`].
///
/// Kept:
/// - `defaults nosave`, exactly, and only as the first command (`diff all` lists only
///   non-default values, so the restore resets RAM to the board defaults first; nothing
///   persists unless the whole replay succeeds and session saves). A backup without it
///   is rejected (not a `diff all`).
/// - every line whose verb is in [`REPLAY_VERBS`], in order.
///
/// Everything else is rejected, including `save` / `batch` / `defaults` anywhere else.
pub fn replay_lines(text: &str) -> AppResult<Vec<String>> {
    const WHAT: &str = "backup";
    let mut cmds: Vec<(usize, &str)> = text
        .lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.trim_end_matches('\r').trim_matches(' ')))
        .filter(|(_, l)| !l.is_empty() && !l.starts_with('#'))
        .collect();
    if cmds
        .first()
        .is_some_and(|(_, l)| l.eq_ignore_ascii_case("diff all"))
    {
        cmds.remove(0);
    }
    if cmds
        .first()
        .is_some_and(|(_, l)| l.eq_ignore_ascii_case("batch start"))
    {
        cmds.remove(0);
    }
    if cmds
        .last()
        .is_some_and(|(_, l)| l.eq_ignore_ascii_case("save"))
    {
        cmds.pop();
    }
    if cmds
        .last()
        .is_some_and(|(_, l)| l.eq_ignore_ascii_case("batch end"))
    {
        cmds.pop();
    }
    cmds.retain(|(_, l)| !BOARD_IDENTITY_VERBS.contains(&verb(l).as_str()));

    match cmds.first() {
        Some((_, l)) if *l == "defaults nosave" => {}
        Some((n, l)) => {
            return Err(reject(
                WHAT,
                *n,
                l,
                "the first command of a `diff all` backup must be `defaults nosave`",
            ))
        }
        None => {
            return Err(AppError::Config(
                "backup has no commands (not a `diff all`); nothing was sent to the FC".into(),
            ))
        }
    }
    let mut out = vec!["defaults nosave".to_string()];
    for (n, l) in &cmds[1..] {
        check(l, REPLAY_VERBS, WHAT, *n)?;
        out.push(l.to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    pub const FORBIDDEN: &[&str] = &[
        "save",
        "SAVE",
        "Save noreboot",
        "save noreboot",
        "defaults",
        "defaults nosave",
        "defaults bare",
        "defaults show",
        "reboot",
        "exit",
        "bl",
        "bl rom",
        "dfu",
        "flash_erase",
        "flash_write 0 x",
        "flash_scan",
        "msc",
        "msc utc",
        "batch start",
        "batch end",
        "restore",
        "bind_rx",
        "bind_rx_spi",
        "diff",
        "diff all",
        "dump all",
        "play_sound",
        "play_sound 3",
        "get gyro",
        "status",
        "name Quad",
        "resource MOTOR 1 NONE",
        "mmix reset",
        "",
        " set a = 1",
        "set a = 1 ",
        "set a = 1\nsave",
        "set a = 1\rsave",
        "set\ta = 1",
        "# comment",
        "#",
        "set name = caf\u{e9}",
    ];

    #[test]
    fn apply_allowlist_rejects_everything_else() {
        for bad in FORBIDDEN {
            let lines = s(&["set motor_poles = 14", bad, "feature GPS"]);
            let e = validate_script(&lines).unwrap_err();
            assert_eq!(e.code(), "CONFIG", "{bad:?}");
            let msg = e.to_string();
            assert!(msg.contains("line 2"), "{bad:?}: {msg}");
            assert!(
                msg.contains(&bad.escape_debug().to_string()),
                "{bad:?}: {msg}"
            );
        }
        assert!(validate_script(&s(&[
            "set motor_poles = 14",
            "serial UART1 64 115200 57600 0 115200",
            "feature -GPS",
            "aux 0 0 0 1700 2100 0 0",
        ]))
        .is_ok());
    }

    #[test]
    fn danger_messages() {
        let e = validate_script(&s(&["save noreboot"]))
            .unwrap_err()
            .to_string();
        assert!(e.contains("persists"), "{e}");
        let e = validate_script(&s(&["flash_erase"]))
            .unwrap_err()
            .to_string();
        assert!(e.contains("flash"), "{e}");
        let e = validate_script(&s(&["status"])).unwrap_err().to_string();
        assert!(e.contains("not a command this path sends"), "{e}");
    }

    const DIFF_45: &str = "diff all\r\n\r\n# version\r\n# Betaflight / STM32F405 (S405) 4.5.1 Nov 30 2024 / 10:46:31 (77d01ba3b) MSP API: 1.46\r\n# config rev: 1a2b3c4\r\n\r\n# start the command batch\r\nbatch start\r\n\r\n# reset configuration to default settings\r\ndefaults nosave\r\n\r\nboard_name SPEEDYBEEF405V4\r\nmanufacturer_id SPBE\r\nmcu_id 0032002d3132510c35333638\r\nsignature \r\n\r\n# name: Cine\r\n\r\n# resources\r\nresource MOTOR 5 NONE\r\n\r\n# timer\r\ntimer A08 AF1\r\n\r\n# dma\r\ndma ADC 1 1\r\n\r\n# feature\r\nfeature -RX_PARALLEL_PWM\r\nfeature GPS\r\n\r\n# serial\r\nserial 1 64 115200 57600 0 115200\r\n\r\n# beeper\r\nbeeper -ON_USB\r\n\r\n# beacon\r\nbeacon RX_LOST\r\n\r\n# map\r\nmap TAER1234\r\n\r\n# mixer\r\nmixer QUADX\r\n\r\nmmix reset\r\n\r\n# led\r\nled 0 0,0::C:0\r\n\r\n# color\r\ncolor 0 0,0,0\r\n\r\n# mode_color\r\nmode_color 0 0 1\r\n\r\n# aux\r\naux 0 0 0 1700 2100 0 0\r\n\r\n# adjrange\r\nadjrange 0 0 0 900 900 0 0 0 0 0\r\n\r\n# rxrange\r\nrxrange 0 1000 2000\r\n\r\n# vtxtable\r\nvtxtable bands 5\r\nvtxtable band 1 BOSCAM_A A FACTORY 5865 5845 5825 5805 5785 5765 5745 5725\r\nvtxtable powerlevels 2\r\nvtxtable powervalues 25 200\r\nvtxtable powerlabels 25 200\r\n\r\n# vtx\r\nvtx 0 0 0 0 0 900 900\r\n\r\n# rxfail\r\nrxfail 0 a\r\n\r\n# master\r\nset craft_name = Cine\r\n\r\nprofile 0\r\n\r\n# profile 0\r\nset p_pitch = 50\r\n\r\n# restore original profile selection\r\nprofile 0\r\n\r\nrateprofile 0\r\n\r\n# rateprofile 0\r\nset roll_rc_rate = 10\r\n\r\n# restore original rateprofile selection\r\nrateprofile 0\r\n\r\n# save configuration\r\nsave\r\n# ";

    #[test]
    fn replay_keeps_exactly_the_config_lines() {
        let got = replay_lines(DIFF_45).unwrap();
        assert_eq!(
            got,
            s(&[
                "defaults nosave",
                "resource MOTOR 5 NONE",
                "timer A08 AF1",
                "dma ADC 1 1",
                "feature -RX_PARALLEL_PWM",
                "feature GPS",
                "serial 1 64 115200 57600 0 115200",
                "beeper -ON_USB",
                "beacon RX_LOST",
                "map TAER1234",
                "mixer QUADX",
                "mmix reset",
                "led 0 0,0::C:0",
                "color 0 0,0,0",
                "mode_color 0 0 1",
                "aux 0 0 0 1700 2100 0 0",
                "adjrange 0 0 0 900 900 0 0 0 0 0",
                "rxrange 0 1000 2000",
                "vtxtable bands 5",
                "vtxtable band 1 BOSCAM_A A FACTORY 5865 5845 5825 5805 5785 5765 5745 5725",
                "vtxtable powerlevels 2",
                "vtxtable powervalues 25 200",
                "vtxtable powerlabels 25 200",
                "vtx 0 0 0 0 0 900 900",
                "rxfail 0 a",
                "set craft_name = Cine",
                "profile 0",
                "set p_pitch = 50",
                "profile 0",
                "rateprofile 0",
                "set roll_rc_rate = 10",
                "rateprofile 0",
            ])
        );
        // Older captures end with `batch end` (+ `save`); no echo line.
        let old = "batch start\ndefaults nosave\nset a = 1\nbatch end\nsave\n";
        assert_eq!(
            replay_lines(old).unwrap(),
            s(&["defaults nosave", "set a = 1"])
        );
    }

    #[test]
    fn replay_rejects_tampered_backups() {
        let base = "batch start\ndefaults nosave\nset a = 1\n";
        for (bad, line) in [
            ("save\n", 4),
            ("save noreboot\n", 4),
            ("defaults nosave\n", 4),
            ("defaults\n", 4),
            ("exit\n", 4),
            ("reboot\n", 4),
            ("bl\n", 4),
            ("dfu\n", 4),
            ("msc\n", 4),
            ("flash_erase\n", 4),
            ("batch start\n", 4),
            ("restore\n", 4),
            ("bind_rx\n", 4),
            ("bind_rx_spi\n", 4),
            ("dump\n", 4),
            ("diff all\n", 4),
            ("play_sound\n", 4),
            ("get a\n", 4),
            ("set a = 1\tsave\n", 4),
        ] {
            let text = format!("{base}{bad}set b = 2\nsave\n");
            let e = replay_lines(&text).unwrap_err();
            assert_eq!(e.code(), "CONFIG", "{bad:?}");
            assert!(
                e.to_string().contains(&format!("line {line}")),
                "{bad:?}: {e}"
            );
        }
        // `batch end` / `save` mid-file are not trailing -> rejected.
        let e = replay_lines("defaults nosave\nbatch end\nset a = 1\n").unwrap_err();
        assert!(e.to_string().contains("line 2"), "{e}");
        let e = replay_lines("defaults nosave\nset a = 1\nsave\nbatch end\n").unwrap_err();
        assert!(e.to_string().contains("line 3"), "{e}");
        // `defaults nosave` must come first; a plain diff (no defaults) is not accepted.
        let e = replay_lines("batch start\nset a = 1\ndefaults nosave\n").unwrap_err();
        assert!(e.to_string().contains("line 2"), "{e}");
        assert!(replay_lines("set a = 1\n").is_err());
        assert!(replay_lines("defaults nosave save\n").is_err());
        assert!(replay_lines("# only comments\n\n").is_err());
        // Identity lines are dropped, never sent.
        let got =
            replay_lines("defaults nosave\nboard_name OTHER\nsignature 00ff\nset a = 1\n").unwrap();
        assert_eq!(got, s(&["defaults nosave", "set a = 1"]));
    }
}
