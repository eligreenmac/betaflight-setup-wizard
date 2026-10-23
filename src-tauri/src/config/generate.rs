//! DroneModel -> firmware-specific CLI commands (pure; golden-tested).

use std::collections::BTreeMap;

use super::catalog;
use super::firmware::Dialect;
use super::modes;
use super::osd;
use super::serial_functions::{self as sf, check_serial_config, functions_in};
use super::snapshot::{AuxSlot, PortState, Snapshot};
use super::vtx_table::{self, VtxTableState};
use crate::error::{AppError, AppResult};
use crate::model::{
    Airmode, ClearedPort, CliScript, DigitalSystem, DroneModel, ElrsDomain, EscProtocol,
    GpsProvider, OsdVideoSystem, ReceiverKind, RxProtocol, SbasMode, VtxKind, VtxProtocol,
    BOARD_ALIGNMENT_MAX, BOARD_ALIGNMENT_MIN,
};

fn cfg_err(msg: impl Into<String>) -> AppError {
    AppError::Config(msg.into())
}

fn on_off(b: bool) -> &'static str {
    if b {
        "ON"
    } else {
        "OFF"
    }
}

fn serialrx_name(p: RxProtocol) -> Option<&'static str> {
    Some(match p {
        RxProtocol::Crsf => "CRSF",
        RxProtocol::Ghst => "GHST",
        RxProtocol::Sbus => "SBUS",
        RxProtocol::Fport => "FPORT",
        RxProtocol::Ibus => "IBUS",
        RxProtocol::Srxl2 => "SRXL2",
        RxProtocol::Spek1024 => "SPEK1024",
        RxProtocol::Spek2048 => "SPEK2048",
        RxProtocol::Sumd => "SUMD",
        RxProtocol::Expresslrs
        | RxProtocol::FrskyD
        | RxProtocol::FrskyX
        | RxProtocol::FrskyXLbt => return None,
    })
}

fn domain_name(d: ElrsDomain) -> &'static str {
    match d {
        ElrsDomain::Au433 => "AU433",
        ElrsDomain::Au915 => "AU915",
        ElrsDomain::Eu433 => "EU433",
        ElrsDomain::Eu868 => "EU868",
        ElrsDomain::In866 => "IN866",
        ElrsDomain::Fcc915 => "FCC915",
        ElrsDomain::Ism2400 => "ISM2400",
        ElrsDomain::Ce2400 => "CE2400",
    }
}

fn is_2g4(name: &str) -> bool {
    matches!(name, "ISM2400" | "CE2400")
}

fn esc_name(p: EscProtocol) -> &'static str {
    match p {
        EscProtocol::Dshot150 => "DSHOT150",
        EscProtocol::Dshot300 => "DSHOT300",
        EscProtocol::Dshot600 => "DSHOT600",
        EscProtocol::Oneshot125 => "ONESHOT125",
        EscProtocol::Multishot => "MULTISHOT",
        EscProtocol::Pwm => "PWM",
    }
}

/// Format a `serial` command for `dialect`.
pub fn serial_cmd(p: &PortState, mask: u32, dialect: Dialect) -> String {
    let id = if dialect.uses_port_names() {
        p.name.clone()
    } else {
        p.identifier.to_string()
    };
    format!(
        "serial {id} {mask} {} {} {} {}",
        p.bauds[0], p.bauds[1], p.bauds[2], p.bauds[3]
    )
}

struct Ctx<'a> {
    model: &'a DroneModel,
    dialect: Dialect,
    snapshot: Option<&'a Snapshot>,
    cmds: Vec<String>,
    warnings: Vec<String>,
}

impl Ctx<'_> {
    fn push(&mut self, s: impl Into<String>) {
        self.cmds.push(s.into());
    }
}

/// Generate the CLI script for `model` against the FC's current `ports` and, when
/// available (config_plan / verify), the full `dump all` snapshot.
pub fn generate_for(
    model: &DroneModel,
    dialect: Dialect,
    ports: &[PortState],
    snapshot: Option<&Snapshot>,
) -> AppResult<CliScript> {
    let mut cx = Ctx {
        model,
        dialect,
        snapshot,
        cmds: Vec::new(),
        warnings: Vec::new(),
    };
    let cleared = serial_section(&mut cx, ports)?;
    receiver_section(&mut cx)?;
    gps_section(&mut cx);
    esc_section(&mut cx)?;
    vtx_section(&mut cx)?;
    craft_name_section(&mut cx)?;
    aux_section(&mut cx)?;
    beeper_section(&mut cx)?;
    sensors_section(&mut cx)?;
    alignment_section(&mut cx)?;
    osd_section(&mut cx)?;
    Ok(CliScript {
        commands: cx.cmds,
        warnings: cx.warnings,
        dialect: dialect.as_str().to_string(),
        cleared,
    })
}

/// Desired managed bits per port name, after validating port usage.
fn desired_functions(model: &DroneModel, ports: &[PortState]) -> AppResult<BTreeMap<String, u32>> {
    let mut want: BTreeMap<String, u32> = BTreeMap::new();
    let mut owner: BTreeMap<String, &'static str> = BTreeMap::new();
    let mut assign = |port: &Option<String>, bits: u32, role: &'static str| -> AppResult<()> {
        let Some(name) = port else {
            return Err(cfg_err(format!("{role}: a port is required")));
        };
        if name.starts_with("SOFTSERIAL") {
            return Err(cfg_err(format!(
                "{role}: soft serial ({name}) is not supported; use a hardware UART"
            )));
        }
        match ports.iter().find(|p| &p.name == name) {
            None => {
                return Err(cfg_err(format!(
                    "{role}: port {name} does not exist on this FC"
                )))
            }
            Some(p) if p.name == "VCP" => {
                return Err(cfg_err(format!(
                    "{role}: {name} is the USB connection port"
                )))
            }
            _ => {}
        }
        if let Some(prev) = owner.insert(name.clone(), role) {
            return Err(cfg_err(format!(
                "{name} is assigned to both {prev} and {role}"
            )));
        }
        *want.entry(name.clone()).or_default() |= bits;
        Ok(())
    };
    if model.receiver.kind == ReceiverKind::Serial {
        assign(&model.receiver.port, sf::RX_SERIAL, "receiver")?;
    }
    if !matches!(model.vtx.kind, VtxKind::None) {
        if let Some(p) = model.vtx.protocol {
            let bits = match p {
                VtxProtocol::Smartaudio => sf::VTX_SMARTAUDIO,
                VtxProtocol::Tramp => sf::VTX_TRAMP,
                VtxProtocol::Msp => sf::VTX_MSP | sf::MSP,
            };
            assign(&model.vtx.port, bits, "VTX")?;
        }
    }
    if model.gps.enabled {
        assign(&model.gps.port, sf::GPS, "GPS")?;
    }
    if model.esc.telemetry_port.is_some() {
        assign(&model.esc.telemetry_port, sf::ESC_SENSOR, "ESC telemetry")?;
    }
    Ok(want)
}

fn serial_section(cx: &mut Ctx, ports: &[PortState]) -> AppResult<Vec<ClearedPort>> {
    let model = cx.model;
    let want = desired_functions(model, ports)?;
    // Receiver kind NONE means "leave the receiver alone".
    let managed = if model.receiver.kind == ReceiverKind::None {
        sf::MANAGED & !sf::RX_SERIAL
    } else {
        sf::MANAGED
    };
    let mut cleared = Vec::new();
    let mut after: Vec<(String, u32)> = Vec::new();
    let mut lines = Vec::new();
    for p in ports {
        let old = p.function_mask;
        if p.name == "VCP" {
            after.push((p.name.clone(), old));
            continue;
        }
        let desired = want.get(&p.name).copied().unwrap_or(0);
        let mut new = (old & !managed) | desired;
        // MSP DisplayPort moved away: drop MSP only if the port was exactly MSP|VTX_MSP.
        if old & sf::VTX_MSP != 0 && desired & sf::VTX_MSP == 0 && old == sf::MSP | sf::VTX_MSP {
            new &= !sf::MSP;
        }
        let removed = old & !new & (managed | sf::MSP);
        if removed != 0 {
            cleared.push(ClearedPort {
                port: p.name.clone(),
                functions: functions_in(removed),
                function_mask_before: old,
                function_mask_after: new,
            });
        }
        if new != old {
            let mut q = p.clone();
            if desired & sf::GPS != 0 && old & sf::GPS == 0 {
                q.bauds[1] = "115200".into();
            }
            lines.push(serial_cmd(&q, new, cx.dialect));
        }
        after.push((p.name.clone(), new));
    }
    let serialrx = (model.receiver.kind == ReceiverKind::Serial)
        .then_some(model.receiver.protocol)
        .flatten();
    check_serial_config(&after, serialrx).map_err(|why| {
        cfg_err(format!(
            "the resulting serial port setup would be rejected by Betaflight: {why}"
        ))
    })?;
    for c in &cleared {
        cx.warnings.push(format!(
            "{}: removing {}",
            c.port,
            c.functions
                .iter()
                .map(|f| format!("{f:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    cx.cmds.extend(lines);
    Ok(cleared)
}

fn receiver_section(cx: &mut Ctx) -> AppResult<()> {
    let r = &cx.model.receiver;
    let rx_msp = cx.dialect.has_rx_msp_feature_name();
    match r.kind {
        ReceiverKind::None => return Ok(()),
        ReceiverKind::Ppm | ReceiverKind::Msp => {
            return Err(cfg_err(
                "PPM and MSP receivers are not supported by the setup wizard",
            ))
        }
        ReceiverKind::Serial => {
            let proto = r
                .protocol
                .ok_or_else(|| cfg_err("serial receiver: protocol is required"))?;
            let name = serialrx_name(proto)
                .ok_or_else(|| cfg_err(format!("{proto:?} is not a serial receiver protocol")))?;
            cx.push("feature RX_SERIAL");
            cx.push("feature -RX_SPI");
            cx.push("feature -RX_PPM");
            cx.push("feature -RX_PARALLEL_PWM");
            if rx_msp {
                cx.push("feature -RX_MSP");
            }
            cx.push(format!("set serialrx_provider = {name}"));
            cx.push(format!("set serialrx_inverted = {}", on_off(r.inverted)));
            cx.push(format!(
                "set serialrx_halfduplex = {}",
                on_off(r.half_duplex)
            ));
            if r.elrs_uid.is_some() || r.elrs_domain.is_some() {
                cx.warnings
                    .push("ELRS UID/domain are only used for onboard SPI receivers; set them on the receiver itself".into());
            }
            if r.telemetry && matches!(proto, RxProtocol::Crsf | RxProtocol::Fport) {
                cx.push("feature TELEMETRY");
            }
        }
        ReceiverKind::Spi => {
            if !matches!(r.protocol, Some(RxProtocol::Expresslrs)) {
                return Err(cfg_err(
                    "only onboard SPI ExpressLRS receivers are supported by the setup wizard",
                ));
            }
            if r.port.is_some() {
                return Err(cfg_err("SPI receiver must not have a port"));
            }
            if let Some(s) = cx.snapshot {
                let has_hw = s.resource_assigned("RX_SPI_CS")
                    && s.settings.contains_key("rx_spi_protocol")
                    && s.settings.contains_key("expresslrs_uid");
                if !has_hw {
                    return Err(cfg_err(
                        "this flight controller has no onboard SPI ExpressLRS receiver (or the firmware was built without it)",
                    ));
                }
                if let (Some(want), Some(cur)) = (r.elrs_domain, s.setting("expresslrs_domain")) {
                    let want = domain_name(want);
                    if cur.eq_ignore_ascii_case("NONE")
                        || is_2g4(want) != is_2g4(&cur.to_ascii_uppercase())
                    {
                        return Err(cfg_err(format!(
                            "ELRS domain {want} is not supported by this receiver chip (current domain {cur})"
                        )));
                    }
                }
            }
            cx.push("feature RX_SPI");
            cx.push("feature -RX_SERIAL");
            cx.push("feature -RX_PPM");
            cx.push("feature -RX_PARALLEL_PWM");
            if rx_msp {
                cx.push("feature -RX_MSP");
            }
            cx.push("set rx_spi_protocol = EXPRESSLRS");
            if let Some(uid) = r.elrs_uid {
                let uid = uid.iter().map(u8::to_string).collect::<Vec<_>>().join(",");
                cx.push(format!("set expresslrs_uid = {uid}"));
            }
            if let Some(d) = r.elrs_domain {
                cx.push(format!("set expresslrs_domain = {}", domain_name(d)));
            }
            if r.telemetry {
                cx.push("feature TELEMETRY");
            }
        }
    }
    Ok(())
}

fn gps_section(cx: &mut Ctx) {
    let g = &cx.model.gps;
    if g.enabled {
        let provider = match g.provider.unwrap_or(GpsProvider::Ublox) {
            GpsProvider::Ublox => "UBLOX",
            GpsProvider::Nmea => "NMEA",
        };
        cx.push("feature GPS");
        cx.push(format!("set gps_provider = {provider}"));
        cx.push("set gps_auto_config = ON");
        if let Some(sbas) = g.sbas_mode {
            cx.push(format!("set gps_sbas_mode = {}", sbas_name(sbas)));
        }
        match (g.galileo, provider) {
            (Some(on), "UBLOX") => cx.push(format!("set gps_ublox_use_galileo = {}", on_off(on))),
            (Some(_), _) => cx
                .warnings
                .push("Galileo can only be selected for a UBLOX GPS; not written".into()),
            (None, _) => {}
        }
    } else {
        cx.push("feature -GPS");
    }
}

fn sbas_name(m: SbasMode) -> &'static str {
    match m {
        SbasMode::Auto => "AUTO",
        SbasMode::Egnos => "EGNOS",
        SbasMode::Waas => "WAAS",
        SbasMode::Msas => "MSAS",
        SbasMode::Gagan => "GAGAN",
        SbasMode::None => "NONE",
    }
}

fn esc_section(cx: &mut Ctx) -> AppResult<()> {
    let e = &cx.model.esc;
    cx.push(if e.telemetry_port.is_some() {
        "feature ESC_SENSOR"
    } else {
        "feature -ESC_SENSOR"
    });
    cx.push(format!("set motor_pwm_protocol = {}", esc_name(e.protocol)));
    let dshot = matches!(
        e.protocol,
        EscProtocol::Dshot150 | EscProtocol::Dshot300 | EscProtocol::Dshot600
    );
    cx.push(format!(
        "set dshot_bidir = {}",
        on_off(dshot && e.bidirectional_dshot)
    ));
    if e.bidirectional_dshot && !dshot {
        cx.warnings
            .push("bidirectional DShot needs a DShot protocol; it was turned off".into());
    }
    if let Some(p) = e.motor_poles {
        cx.push(format!("set motor_poles = {p}"));
    }
    cx.push(format!("set yaw_motors_reversed = {}", on_off(e.props_out)));
    if let Some(idle) = e.idle {
        if idle > 2000 {
            return Err(cfg_err(format!(
                "motor idle {idle} is out of range (0-2000, in 0.01 %)"
            )));
        }
        if cx.dialect == Dialect::V2025_12 {
            cx.push(format!("set motor_idle = {idle}"));
        } else if dshot {
            cx.push(format!("set dshot_idle_value = {idle}"));
        } else {
            // 4.4 / 4.5 derive idle from min_throttle for analog protocols.
            cx.warnings.push(
                "motor idle is a DShot setting on this firmware (dshot_idle_value); not written"
                    .into(),
            );
        }
    }
    if let Some(order) = &e.motor_order {
        // motor_output_reordering has MAX_SUPPORTED_MOTORS entries; keep the FC's length.
        let len = cx
            .snapshot
            .and_then(|s| s.setting("motor_output_reordering"))
            .map_or(8, |v| v.split(',').count());
        let mut seen = vec![false; order.len()];
        let perm = !order.is_empty()
            && order.len() <= len
            && order.iter().all(|&o| {
                let i = o as usize;
                i < seen.len() && !std::mem::replace(&mut seen[i], true)
            });
        if !perm {
            return Err(cfg_err(format!(
                "motor order {order:?} must be a permutation of 0..{} (at most {len} motors)",
                order.len()
            )));
        }
        let full: Vec<String> = (0..len)
            .map(|i| order.get(i).map_or(i, |&o| o as usize).to_string())
            .collect();
        cx.push(format!("set motor_output_reordering = {}", full.join(",")));
    }
    Ok(())
}

/// `set craft_name`: see `model::craft_name_valid` (the schema's pattern); "" clears it (`-`).
fn craft_name_section(cx: &mut Ctx) -> AppResult<()> {
    let Some(name) = cx.model.extras.craft_name.as_deref() else {
        return Ok(());
    };
    if !crate::model::craft_name_valid(name) {
        return Err(cfg_err(format!(
            "craft name {name:?} must be 1-16 printable ASCII characters other than # and ?, \
             without leading/trailing spaces, and not just \"-\""
        )));
    }
    cx.push(if name.is_empty() {
        "set craft_name = -".to_string()
    } else {
        format!("set craft_name = {name}")
    });
    Ok(())
}

fn vtx_section(cx: &mut Ctx) -> AppResult<()> {
    vtx_table_section(cx)?;
    // With an OSD video system in the model, osd_section writes these two settings.
    let osd_owns = cx
        .model
        .osd
        .as_ref()
        .is_some_and(|o| o.video_system.is_some());
    match cx.model.vtx.kind {
        VtxKind::None => {}
        VtxKind::Analog => {
            if !osd_owns {
                cx.push("set osd_displayport_device = AUTO");
                cx.push("set vcd_video_system = AUTO");
            }
        }
        VtxKind::Digital => {
            if !osd_owns {
                cx.push("set osd_displayport_device = MSP");
                cx.push("set vcd_video_system = HD");
            }
            if matches!(cx.model.vtx.digital_system, Some(DigitalSystem::Dji))
                && !matches!(cx.model.vtx.protocol, Some(VtxProtocol::Msp))
            {
                cx.warnings
                    .push("digital VTX without an MSP port: the OSD will not work".into());
            }
        }
    }
    Ok(())
}

/// `vtxtable ...` then `vtx_band` / `vtx_channel` / `vtx_power` (1-based into the table
/// being written, or the FC's current table when the model keeps it).
fn vtx_table_section(cx: &mut Ctx) -> AppResult<()> {
    let v = &cx.model.vtx;
    if v.table.is_none() && v.band.is_none() && v.channel.is_none() && v.power.is_none() {
        return Ok(());
    }
    if matches!(v.kind, VtxKind::None) {
        return Err(cfg_err(
            "VTX table / band / channel / power are set but the model has no VTX",
        ));
    }
    // Firmware ranges (cli/settings.c), whether or not the table is known.
    for (what, val, max) in [
        ("band", v.band, crate::model::VTX_BAND_MAX),
        ("channel", v.channel, crate::model::VTX_CHANNEL_MAX),
        ("power level", v.power, crate::model::VTX_POWER_MAX),
    ] {
        if let Some(n) = val.filter(|&n| n == 0 || n > max) {
            return Err(cfg_err(format!(
                "VTX {what} {n}: the CLI accepts 1-{max} (vtx_{} range)",
                what.split(' ').next().unwrap()
            )));
        }
    }
    let table: Option<VtxTableState> = match &v.table {
        Some(t) => {
            vtx_table::validate(t).map_err(cfg_err)?;
            Some(VtxTableState::from_model(t))
        }
        None => cx.snapshot.and_then(|s| s.vtxtable.clone()),
    };
    let check = |what: &str, val: u8, max: usize| -> AppResult<()> {
        if val == 0 || usize::from(val) > max {
            return Err(cfg_err(format!(
                "VTX {what} {val} is outside the VTX table (1-{max})"
            )));
        }
        Ok(())
    };
    if let Some(t) = &table {
        if let Some(b) = v.band {
            check("band", b, t.bands)?;
        }
        if let Some(c) = v.channel {
            check("channel", c, t.channels)?;
        }
        if let Some(p) = v.power {
            check("power level", p, t.power_levels)?;
        }
    } else if v.band.is_some() || v.channel.is_some() || v.power.is_some() {
        cx.warnings.push(
            "VTX band/channel/power written without a known VTX table; check it on the FC".into(),
        );
    }
    if let Some(t) = &v.table {
        cx.cmds.extend(VtxTableState::commands_for(t));
    }
    if let Some(b) = v.band {
        cx.push(format!("set vtx_band = {b}"));
    }
    if let Some(c) = v.channel {
        cx.push(format!("set vtx_channel = {c}"));
    }
    if let Some(p) = v.power {
        cx.push(format!("set vtx_power = {p}"));
    }
    Ok(())
}

/// Which optional modes (AIR MODE, FLIP OVER AFTER CRASH, PREARM) the model manages, and
/// the `feature AIRMODE` line. Validates `airmode` against the AIR MODE switch.
fn airmode_and_optional(cx: &mut Ctx, switch_ids: &[u8]) -> AppResult<Vec<u8>> {
    let model = cx.model;
    let dialect = cx.dialect;
    let mut managed: Vec<u8> = switch_ids
        .iter()
        .copied()
        .filter(|id| modes::is_optional_id(*id))
        .collect();
    for name in &model.remove_modes {
        let (canon, id) = modes::resolve(name, dialect).map_err(cfg_err)?;
        if switch_ids.contains(&id) {
            return Err(cfg_err(format!(
                "mode '{canon}' is both in removeModes and assigned to a switch"
            )));
        }
        if !modes::is_optional_id(id) {
            return Err(cfg_err(format!(
                "removeModes takes only PREARM, FLIP OVER AFTER CRASH and AIR MODE, not '{canon}' (other managed modes are removed when they are not in switches)"
            )));
        }
        // No range for it on the FC: nothing to remove (a no-op, not an error).
        managed.push(id);
    }
    let air_switch = switch_ids.contains(&modes::AIR_MODE_ID);
    match model.airmode {
        Some(Airmode::Always) | Some(Airmode::Off) if air_switch => {
            return Err(cfg_err(
                "airmode is ALWAYS/OFF but an AIR MODE switch is assigned; use airmode SWITCH",
            ));
        }
        Some(Airmode::Switch) if !air_switch => {
            return Err(cfg_err(
                "airmode SWITCH needs an AIR MODE entry in switches",
            ));
        }
        Some(Airmode::Always) => {
            cx.push("feature AIRMODE");
            managed.push(modes::AIR_MODE_ID);
        }
        Some(Airmode::Off) => {
            cx.push("feature -AIRMODE");
            managed.push(modes::AIR_MODE_ID);
        }
        Some(Airmode::Switch) => cx.push("feature -AIRMODE"),
        None if air_switch => {
            let on = cx
                .snapshot
                .and_then(|s| s.features.get("AIRMODE").copied())
                .unwrap_or(true);
            if on {
                cx.warnings.push(
                    "an AIR MODE switch has no effect while feature AIRMODE is on; set airmode to SWITCH"
                        .into(),
                );
            }
        }
        None => {}
    }
    managed.sort_unstable();
    managed.dedup();
    Ok(managed)
}

fn aux_section(cx: &mut Ctx) -> AppResult<()> {
    let dialect = cx.dialect;
    let switches = &cx.model.switches;
    let mut switch_ids = Vec::with_capacity(switches.len());
    for sw in switches {
        let (_, id) = modes::resolve(&sw.mode, dialect).map_err(cfg_err)?;
        switch_ids.push(id);
    }
    let optional = airmode_and_optional(cx, &switch_ids)?;
    let is_managed = |id: u8| {
        modes::is_managed_id(id, dialect) && (!modes::is_optional_id(id) || optional.contains(&id))
    };
    let existing: Vec<AuxSlot> = match cx.snapshot {
        Some(s) => s.aux.clone(),
        None => Vec::new(),
    };
    let slot = |i: u8| {
        existing
            .iter()
            .find(|a| a.slot == i)
            .copied()
            .unwrap_or(AuxSlot::empty(i))
    };
    let mut freed = Vec::new();
    let mut unused = Vec::new();
    let mut kept = 0;
    for i in 0..AuxSlot::COUNT {
        let a = slot(i);
        if !a.is_used() {
            unused.push(i);
        } else if is_managed(a.mode_id) {
            freed.push(i);
        } else {
            kept += 1;
        }
    }
    // Lowest slot numbers first, so verify (which regenerates against the applied FC)
    // assigns the same slots again.
    let mut available: Vec<u8> = freed.iter().chain(unused.iter()).copied().collect();
    available.sort_unstable();
    if switches.len() > available.len() {
        return Err(cfg_err(format!(
            "{} switch assignments need more than the {} free aux slots",
            switches.len(),
            available.len()
        )));
    }
    let mut out: BTreeMap<u8, String> = BTreeMap::new();
    for ((sw, &id), &slot_no) in switches.iter().zip(&switch_ids).zip(available.iter()) {
        if sw.aux_channel > 13 {
            return Err(cfg_err(format!(
                "{}: AUX{} does not exist",
                sw.mode,
                sw.aux_channel + 1
            )));
        }
        let ok_range = |v: u16| (900..=2100).contains(&v) && v.is_multiple_of(25);
        if !(ok_range(sw.range_min) && ok_range(sw.range_max) && sw.range_min < sw.range_max) {
            return Err(cfg_err(format!(
                "{}: invalid range {}..{}",
                sw.mode, sw.range_min, sw.range_max
            )));
        }
        let a = AuxSlot {
            slot: slot_no,
            mode_id: id,
            aux_channel: sw.aux_channel,
            range_min: sw.range_min,
            range_max: sw.range_max,
            logic: 0,
            linked_to: 0,
        };
        out.insert(slot_no, a.to_cli());
    }
    for &f in &freed {
        out.entry(f).or_insert_with(|| AuxSlot::empty(f).to_cli());
    }
    if kept > 0 {
        cx.warnings.push(format!(
            "kept {kept} aux slot(s) used by modes the wizard does not manage"
        ));
    }
    cx.cmds.extend(out.into_values());
    Ok(())
}

/// `beeper [-]NAME` for every condition the line has (table order), then `beacon [-]NAME`.
/// A build without the command (no `beeper` / `beacon` lines in its dump) gets a warning.
fn beeper_section(cx: &mut Ctx) -> AppResult<()> {
    let Some(b) = &cx.model.beepers else {
        return Ok(());
    };
    let dialect = cx.dialect;
    if cx.snapshot.is_some_and(|s| s.beepers.is_none()) {
        cx.warnings
            .push("this firmware build has no beeper command; beeper settings not written".into());
    } else {
        for d in &b.disabled {
            if catalog::beeper_cli_name(*d, dialect).is_none() {
                cx.warnings.push(format!(
                    "beeper condition {} does not exist on Betaflight {}; ignored",
                    serde_json::to_value(d)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_string))
                        .unwrap_or_default(),
                    dialect.as_str()
                ));
            }
        }
        for c in catalog::BEEPERS {
            if let Some(name) = catalog::beeper_cli_name(c, dialect) {
                let off = if b.disabled.contains(&c) { "-" } else { "" };
                cx.push(format!("beeper {off}{name}"));
            }
        }
    }
    if let Some(off) = &b.dshot_beacon_disabled {
        if cx.snapshot.is_some_and(|s| s.beacons.is_none()) {
            cx.warnings.push(
                "this firmware build has no DShot beacon (beacon command); not written".into(),
            );
        } else {
            for c in catalog::BEACONS {
                let neg = if off.contains(&c) { "-" } else { "" };
                cx.push(format!("beacon {neg}{}", catalog::beacon_cli_name(c)));
            }
        }
    }
    Ok(())
}

/// acc / baro / mag `*_hardware`: true = AUTO (a specific chip already set is kept),
/// false = NONE. A sensor the build lacks: CONFIG when enabling, nothing when disabling.
fn sensors_section(cx: &mut Ctx) -> AppResult<()> {
    let Some(sn) = cx.model.sensors.clone() else {
        return Ok(());
    };
    let dialect = cx.dialect;
    if sn.acc == Some(false) {
        let needs_acc = ["ANGLE", "HORIZON", "ALTHOLD", "POSHOLD", "GPS RESCUE"];
        for sw in &cx.model.switches {
            if let Ok((canon, _)) = modes::resolve(&sw.mode, dialect) {
                if needs_acc.contains(&canon) {
                    return Err(cfg_err(format!(
                        "the accelerometer is turned off but {canon} (which needs it) is on a switch"
                    )));
                }
            }
        }
    }
    if sn.baro == Some(false)
        && cx
            .model
            .switches
            .iter()
            .any(|sw| modes::resolve(&sw.mode, dialect).is_ok_and(|(c, _)| c == "ALTHOLD"))
    {
        cx.warnings
            .push("ALTHOLD without a barometer holds altitude on GPS only".into());
    }
    for (what, key, want) in [
        ("accelerometer", "acc_hardware", sn.acc),
        ("barometer", "baro_hardware", sn.baro),
        ("magnetometer", "mag_hardware", sn.mag),
    ] {
        let Some(on) = want else { continue };
        let current = match cx.snapshot {
            Some(s) => match s.setting(key) {
                Some(v) => Some(v.to_ascii_uppercase()),
                None if on => {
                    return Err(cfg_err(format!(
                        "this firmware build has no {what} support ({key})"
                    )))
                }
                None => continue,
            },
            None => None,
        };
        let value = match (on, current.as_deref()) {
            (false, _) => "NONE".to_string(),
            (true, None | Some("AUTO")) => "AUTO".to_string(),
            (true, Some("NONE")) => {
                cx.warnings.push(if key == "acc_hardware" {
                    "accelerometer turned on: calibrate it (level, Configurator or stick command) before flying in self-level modes".to_string()
                } else {
                    format!("{what} turned on (AUTO): it is detected at the next boot; without one fitted it stays off")
                });
                "AUTO".to_string()
            }
            (true, Some(chip)) => chip.to_string(),
        };
        cx.push(format!("set {key} = {value}"));
    }
    Ok(())
}

fn alignment_section(cx: &mut Ctx) -> AppResult<()> {
    let Some(a) = cx.model.board_alignment else {
        return Ok(());
    };
    for (axis, v) in [("roll", a.roll), ("pitch", a.pitch), ("yaw", a.yaw)] {
        if !(BOARD_ALIGNMENT_MIN..=BOARD_ALIGNMENT_MAX).contains(&v) {
            return Err(cfg_err(format!(
                "board alignment {axis} {v} is out of range ({BOARD_ALIGNMENT_MIN}..{BOARD_ALIGNMENT_MAX} degrees)"
            )));
        }
        cx.push(format!("set align_board_{axis} = {v}"));
    }
    Ok(())
}

/// `feature OSD`, displayport / video system (+ HD canvas), then one `osd_<el>_pos` per
/// listed element (settings order), visible in all 3 OSD profiles when enabled.
fn osd_section(cx: &mut Ctx) -> AppResult<()> {
    let Some(o) = cx.model.osd.clone() else {
        return Ok(());
    };
    let dialect = cx.dialect;
    let snap = cx.snapshot;
    // Without a snapshot (preview / goldens) assume the firmware line's stock build.
    let has = |k: &str| snap.is_none_or(|s| s.has_setting(k));
    if !has("osd_displayport_device") {
        return Err(cfg_err(
            "this firmware build has no OSD (osd_displayport_device is missing)",
        ));
    }
    let vtx = cx.model.vtx.kind;
    let fc_vs = snap
        .and_then(|s| s.setting("vcd_video_system"))
        .and_then(osd::parse_video_system);
    let vs = match (o.video_system, vtx) {
        (Some(v), _) => v,
        (None, VtxKind::Digital) => OsdVideoSystem::Hd,
        (None, VtxKind::Analog) => OsdVideoSystem::Auto,
        (None, VtxKind::None) => match fc_vs {
            Some(v) => v,
            None => {
                cx.warnings.push(
                    "OSD video system unknown; element positions checked against the 30x13 AUTO grid"
                        .into(),
                );
                OsdVideoSystem::Auto
            }
        },
    };
    if vs == OsdVideoSystem::Hd && !has("displayport_msp_col_adjust") {
        return Err(cfg_err(
            "HD OSD needs MSP DisplayPort, which this firmware build does not have",
        ));
    }
    if o.video_system.is_some() && !has("vcd_video_system") {
        return Err(cfg_err(
            "this firmware build cannot select the OSD video system (vcd_video_system is missing)",
        ));
    }
    match (vtx, vs) {
        (VtxKind::Digital, OsdVideoSystem::Pal | OsdVideoSystem::Ntsc | OsdVideoSystem::Auto) => {
            cx.warnings.push(format!(
                "digital VTX with an analog OSD video system ({}): the OSD uses the 30-column SD grid",
                osd::video_system_name(vs)
            ))
        }
        (VtxKind::Analog, OsdVideoSystem::Hd) => cx.warnings.push(
            "analog VTX with an HD OSD: the analog OSD chip is not used; only an MSP DisplayPort device shows it"
                .into(),
        ),
        _ => {}
    }
    cx.push("feature OSD");
    if let Some(v) = o.video_system {
        let dp = if v == OsdVideoSystem::Hd || matches!(vtx, VtxKind::Digital) {
            "MSP"
        } else {
            "AUTO"
        };
        cx.push(format!("set osd_displayport_device = {dp}"));
        cx.push(format!(
            "set vcd_video_system = {}",
            osd::video_system_name(v)
        ));
    }
    let (cols, rows) = vs.grid();
    // osd_canvas_* exist on 4.4 / 4.5; 2025.12 hides them (OSD_CANVAS_SIZE_DEBUG) and
    // takes the canvas from the HD device.
    let canvas = match snap {
        Some(s) => s.has_setting("osd_canvas_width"),
        None => dialect < Dialect::V2025_12,
    };
    if vs == OsdVideoSystem::Hd && canvas {
        cx.push(format!("set osd_canvas_width = {cols}"));
        cx.push(format!("set osd_canvas_height = {rows}"));
    }
    let mut lines: Vec<(usize, String)> = Vec::new();
    for e in &o.elements {
        let cli = osd::cli_name(&e.name, dialect).map_err(cfg_err)?;
        let idx = osd::ELEMENTS
            .iter()
            .position(|(_, c, _)| *c == cli)
            .unwrap_or(usize::MAX);
        if lines.iter().any(|(i, _)| *i == idx) {
            return Err(cfg_err(format!("OSD element {} is listed twice", e.name)));
        }
        let key = format!("osd_{cli}_pos");
        if !has(&key) {
            return Err(cfg_err(format!(
                "OSD element {} is not in this firmware build ({key} is missing)",
                e.name
            )));
        }
        let stable = osd::ELEMENTS[idx].0;
        let (dx, dy, _) = osd::default_position(stable, cols, rows);
        let (x, y) = (e.x.unwrap_or(dx), e.y.unwrap_or(dy));
        if x >= cols || y >= rows {
            return Err(cfg_err(format!(
                "OSD element {} at ({x}, {y}) is outside the {} grid ({cols}x{rows})",
                e.name,
                osd::video_system_name(vs)
            )));
        }
        let current = snap
            .and_then(|s| s.setting(&key))
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(0);
        lines.push((
            idx,
            format!("set {key} = {}", osd::encode(x, y, e.enabled, current)),
        ));
    }
    lines.sort_by_key(|(i, _)| *i);
    cx.cmds.extend(lines.into_iter().map(|(_, l)| l));
    Ok(())
}
