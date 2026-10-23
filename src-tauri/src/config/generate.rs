//! DroneModel -> firmware-specific CLI commands (pure; golden-tested).

use std::collections::BTreeMap;

use super::firmware::Dialect;
use super::md5::elrs_uid;
use super::modes;
use super::serial_functions::{self as sf, check_serial_config, functions_in};
use super::snapshot::{AuxSlot, PortState, Snapshot};
use crate::error::{AppError, AppResult};
use crate::model::{
    ClearedPort, CliScript, DigitalSystem, DroneModel, ElrsDomain, EscProtocol, GpsProvider,
    ReceiverKind, RxProtocol, VtxKind, VtxProtocol,
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
    esc_section(&mut cx);
    vtx_section(&mut cx);
    aux_section(&mut cx)?;
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
            if r.elrs_binding_phrase.is_some() || r.elrs_domain.is_some() {
                cx.warnings
                    .push("ELRS binding phrase/domain are only used for onboard SPI receivers; set them on the receiver itself".into());
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
            if let Some(phrase) = r.elrs_binding_phrase.as_deref().filter(|p| !p.is_empty()) {
                let uid = elrs_uid(phrase)
                    .iter()
                    .map(u8::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
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
    } else {
        cx.push("feature -GPS");
    }
}

fn esc_section(cx: &mut Ctx) {
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
}

fn vtx_section(cx: &mut Ctx) {
    match cx.model.vtx.kind {
        VtxKind::None => {}
        VtxKind::Analog => {
            cx.push("set osd_displayport_device = AUTO");
            cx.push("set vcd_video_system = AUTO");
        }
        VtxKind::Digital => {
            cx.push("set osd_displayport_device = MSP");
            cx.push("set vcd_video_system = HD");
            if matches!(cx.model.vtx.digital_system, Some(DigitalSystem::Dji))
                && !matches!(cx.model.vtx.protocol, Some(VtxProtocol::Msp))
            {
                cx.warnings
                    .push("digital VTX without an MSP port: the OSD will not work".into());
            }
        }
    }
}

fn aux_section(cx: &mut Ctx) -> AppResult<()> {
    let dialect = cx.dialect;
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
        } else if modes::is_managed_id(a.mode_id, dialect) {
            freed.push(i);
        } else {
            kept += 1;
        }
    }
    let available: Vec<u8> = freed.iter().chain(unused.iter()).copied().collect();
    let switches = &cx.model.switches;
    if switches.len() > available.len() {
        return Err(cfg_err(format!(
            "{} switch assignments need more than the {} free aux slots",
            switches.len(),
            available.len()
        )));
    }
    let mut out: BTreeMap<u8, String> = BTreeMap::new();
    for (sw, &slot_no) in switches.iter().zip(available.iter()) {
        let (_, id) = modes::resolve(&sw.mode, dialect).map_err(cfg_err)?;
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
    for &f in freed.iter().skip(switches.len()) {
        out.insert(f, AuxSlot::empty(f).to_cli());
    }
    if kept > 0 {
        cx.warnings.push(format!(
            "kept {kept} aux slot(s) used by modes the wizard does not manage"
        ));
    }
    cx.cmds.extend(out.into_values());
    Ok(())
}
