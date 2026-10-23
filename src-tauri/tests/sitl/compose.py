#!/usr/bin/env python3
"""Compose the per-board MockFc fixtures from public, GPL-3.0 sources.

    tests/sitl/compose.py            # rewrites tests/fixtures/{4.4,4.5,2025.12}/*.dump.txt

Layer 1  real `dump all` captured from Betaflight SITL of the same release
         (tests/fixtures/sitl/<tag>/dump_all.txt): banner, section layout, the full
         feature-name list, every setting name and its firmware default.
Layer 2  the board definition: betaflight/unified-targets .config (4.4) or
         betaflight/config config.h (4.5, 2025.12), copied verbatim under
         tests/fixtures/boards/: board_name, manufacturer_id, MCU, pin resources,
         which UARTs exist, default serial functions/features/settings.
Layer 3  settings and commands that SITL cannot compile (DShot, GPS, OSD, SPI ELRS,
         beeper / beacon) with the
         defaults from the firmware sources of the same tag (OVERLAY below cites them).

Nothing here is a hardware recording; see tests/fixtures/README.md.
"""
import os, re

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "fixtures")

RELEASES = {"4.4": "4.4.3", "4.5": "4.5.5", "2025.12": "2025.12.5"}
BOARDS = {
    "speedybeef405v4": ("SPBE-SPEEDYBEEF405V4.config", "SPEEDYBEEF405V4"),
    "crazybeef4sx1280": ("HAMO-CRAZYBEEF4SX1280.config", "CRAZYBEEF4SX1280"),
}
MCU_CODE = {"STM32F405": "S405", "STM32F411": "S411"}

# Layer 3. Values and their source (path in betaflight/betaflight at the tag).
OVERLAY = {
    "all": {
        "dshot_bidir": "OFF",                 # pg/motor.c DEFAULT_DSHOT_TELEMETRY = OFF
        "gps_provider": "UBLOX",              # pg/gps.c .provider = GPS_UBLOX
        "gps_auto_config": "ON",              # pg/gps.c .autoConfig = GPS_AUTOCONFIG_ON
        "gps_sbas_mode": "NONE",              # pg/gps.c .sbasMode = SBAS_NONE
        "gps_ublox_use_galileo": "OFF",       # pg/gps.c .gps_ublox_use_galileo = false
        "vcd_video_system": "AUTO",           # pg/vcd.c VIDEO_SYSTEM_AUTO
        "displayport_msp_col_adjust": "0",    # io/displayport_msp.c pgResetFn: colAdjust 0
        "displayport_msp_row_adjust": "0",    # (USE_MSP_DISPLAYPORT, defined with USE_OSD)
    },
    # osd/osd.c pgResetFn_osdConfig. 4.4: displayPortDevice AUTO, canvas HD 53x20 under
    # USE_OSD_HD. 4.5 / 2025.12: USE_OSD_HD (target/common_pre.h, both SD and HD) -> MSP,
    # 53x20. 2025.12 has no osd_canvas_* CLI settings (#ifdef OSD_CANVAS_SIZE_DEBUG).
    "4.4": {"dshot_idle_value": "550",        # pg/motor.c digitalIdleOffsetValue = 550
            "osd_displayport_device": "AUTO", "osd_canvas_width": "53", "osd_canvas_height": "20"},
    "4.5": {"dshot_idle_value": "550",        # pg/motor.c digitalIdleOffsetValue = 550
            "osd_displayport_device": "MSP", "osd_canvas_width": "53", "osd_canvas_height": "20"},
    "2025.12": {"osd_displayport_device": "MSP"},  # motor_idle is present in SITL
}

# io/beeper.c beeperTable (dump order; `ALL` is not printed). Default beeper_off_flags = 0
# (all on); beacon (USE_DSHOT): DSHOT_BEACON_ALLOWED_MODES = RX_LOST | RX_SET, both off by
# default (DEFAULT_DSHOT_BEACON_OFF_FLAGS). SITL has neither command (no USE_BEEPER).
BEEPERS_44 = ("GYRO_CALIBRATED RX_LOST RX_LOST_LANDING DISARMING ARMING ARMING_GPS_FIX "
              "ARMING_GPS_NO_FIX BAT_CRIT_LOW BAT_LOW GPS_STATUS RX_SET ACC_CALIBRATION "
              "ACC_CALIBRATION_FAIL READY_BEEP MULTI_BEEPS DISARM_REPEAT ARMED SYSTEM_INIT ON_USB "
              "BLACKBOX_ERASE CRASH_FLIP CAM_CONNECTION_OPEN CAM_CONNECTION_CLOSE "
              "RC_SMOOTHING_INIT_FAIL").split()
BEEPERS = {"4.4": BEEPERS_44, "4.5": BEEPERS_44,
           "2025.12": [("CRASHFLIP" if b == "CRASH_FLIP" else b) for b in BEEPERS_44
                       if b != "RC_SMOOTHING_INIT_FAIL"]}

# cli/settings.c `osd_<name>_pos` per release (all with USE_OSD; the board builds have the
# defaults of target/common_pre.h). gps_lap_* need USE_GPS_LAP_TIMER (a cloud-build option,
# not in common_pre.h); lidar_dist needs USE_RANGEFINDER (2025.12, flash >= 1024 KB).
OSD_44 = ("vbat rssi link_quality link_tx_power rssi_dbm rsnr tim_1 tim_2 remaining_time_estimate "
          "flymode anti_gravity g_force throttle vtx_channel crosshairs ah_sbar ah current mah_drawn "
          "wh_drawn motor_diag craft_name pilot_name gps_speed gps_lon gps_lat gps_sats home_dir "
          "home_dist flight_dist compass_bar altitude pid_roll pid_pitch pid_yaw debug power "
          "pidrate_profile warnings avg_cell_voltage pit_ang rol_ang battery_usage disarmed nheading "
          "up_down_reference ready_mode nvario esc_tmp esc_rpm esc_rpm_freq rtc_date_time "
          "adjustment_range flip_arrow core_temp log_status stick_overlay_left stick_overlay_right "
          "rate_profile_name pid_profile_name profile_name rcchannels camera_frame efficiency "
          "total_flights aux sys_goggle_voltage sys_vtx_voltage sys_bitrate sys_delay sys_distance "
          "sys_lq sys_goggle_dvr sys_vtx_dvr sys_warnings sys_vtx_temp sys_fan_speed").split()


def osd_pos(x, y):  # osd/osd.h OSD_POS: x bits 0-4 + bit 10, y bits 5-9
    return (x & 31) | ((x << 5) & 0x400) | ((y & 31) << 5)


def osd_defaults(fw, flash_kb):
    """osd/osd.c pgResetFn_osdElementConfig: every element hidden at (midCol - 5, midRow);
    WARNINGS visible in all 3 OSD profiles (bits 11-13) at (midCol - 6, midRow + 3)
    (2025.12: midCol - OSD_WARNINGS_PREFFERED_SIZE / 2 = midCol - 6); a few fixed ones.
    4.4 uses the SD centre when USE_OSD_SD is defined; 4.5 / 2025.12 the HD centre when
    USE_OSD_HD is defined (both are, on these builds)."""
    mid_col, mid_row = (15, 7) if fw == "4.4" else (26, 10)
    names = list(OSD_44)
    if fw == "2025.12":
        names.insert(names.index("power"), "debug2")
        if flash_kb >= 1024:
            names.append("lidar_dist")
    special = {
        "warnings": osd_pos(mid_col - 6, mid_row + 3) | 0x3800,
        "crosshairs": osd_pos(mid_col - 2, mid_row - 1),
        "ah": osd_pos(mid_col - 1, mid_row - 5),
        "ah_sbar": osd_pos(mid_col - 1, mid_row - 1),
        "camera_frame": osd_pos(mid_col - 12, mid_row - 6),
        "up_down_reference": osd_pos(mid_col - 2, mid_row - 1),
    }
    return {f"osd_{n}_pos": str(special.get(n, osd_pos(mid_col - 5, mid_row))) for n in names}
# pg/motor.c: 4.4 defaults to DISABLED (kept from SITL); 4.5 uses DEFAULT_MOTOR_DSHOT_SPEED
# (target/STM32F411/target.h: DSHOT300) else DSHOT600; 2025.12 DSHOT600.
MOTOR_PROTOCOL = {("4.5", "STM32F405"): "DSHOT600", ("4.5", "STM32F411"): "DSHOT300",
                  ("2025.12", "STM32F405"): "DSHOT600", ("2025.12", "STM32F411"): "DSHOT600"}
ELRS = {  # pg/rx_spi_expresslrs.c: domain 0, rateIndex 0, modelId 0xFF, switchMode 0
    "rx_spi_protocol": "EXPRESSLRS",          # RX_SPI_DEFAULT_PROTOCOL in the board file
    "expresslrs_domain": "ISM2400",           # lookupTableFreqDomain[0] for SX1280-only builds
    "expresslrs_rate_index": "0",
    "expresslrs_model_id": "255",
    "expresslrs_uid": "0,0,0,0,0,0",
}
ELRS_SWITCH = {"4.4": "WIDE", "4.5": "WIDE"}  # lookupTableSwitchMode[0]; removed in 2025.12

FN = {"MSP": 1, "RX_SERIAL": 64, "ESC_SENSOR": 1024}


def pin(p):  # PB6 -> B06
    return f"{p[1]}{int(p[2:]):02d}"


def board_from_config_h(text):
    d = dict(re.findall(r"^#define\s+(\w+)\s+([^\s/]+)", text, re.M))
    flags = set(re.findall(r"^#define\s+(\w+)\s*$", text, re.M))
    res = []
    simple = {"BEEPER_PIN": "BEEPER 1", "LED_STRIP_PIN": "LED_STRIP 1", "ADC_VBAT_PIN": "ADC_BATT 1",
              "ADC_CURR_PIN": "ADC_CURR 1", "ADC_RSSI_PIN": "ADC_RSSI 1", "GYRO_1_CS_PIN": "GYRO_CS 1",
              "GYRO_1_EXTI_PIN": "GYRO_EXTI 1", "MAX7456_SPI_CS_PIN": "OSD_CS 1",
              "FLASH_CS_PIN": "FLASH_CS 1", "SDCARD_SPI_CS_PIN": "SDCARD_CS 1",
              "RX_SPI_CS_PIN": "RX_SPI_CS 1", "RX_SPI_EXTI_PIN": "RX_SPI_EXTI 1",
              "RX_SPI_BIND_PIN": "RX_SPI_BIND 1", "RX_SPI_LED_PIN": "RX_SPI_LED 1",
              "RX_SPI_EXPRESSLRS_RESET_PIN": "RX_SPI_EXPRESSLRS_RESET 1",
              "RX_SPI_EXPRESSLRS_BUSY_PIN": "RX_SPI_EXPRESSLRS_BUSY 1"}
    for i in range(1, 9):
        if f"MOTOR{i}_PIN" in d:
            res.append((f"MOTOR {i}", pin(d[f"MOTOR{i}_PIN"])))
    for kind, tag in (("TX", "SERIAL_TX"), ("RX", "SERIAL_RX")):
        for i in range(1, 11):
            if f"UART{i}_{kind}_PIN" in d:
                res.append((f"{tag} {i}", pin(d[f"UART{i}_{kind}_PIN"])))
    for k, name in simple.items():
        if k in d:
            res.append((name, pin(d[k])))
    uarts = sorted({int(m) for m in re.findall(r"^#define\s+UART(\d+)_[TR]X_PIN", text, re.M)})
    serial = {}
    for macro, fn in (("MSP_UART", "MSP"), ("SERIALRX_UART", "RX_SERIAL"), ("ESC_SENSOR_UART", "ESC_SENSOR")):
        if macro in d:
            n = int(re.search(r"(\d+)$", d[macro]).group(1))
            serial[n] = serial.get(n, 0) | FN[fn]
    rx_feature = "RX_SPI" if d.get("DEFAULT_RX_FEATURE") == "FEATURE_RX_SPI" else "RX_SERIAL"
    return {
        "board_name": d["BOARD_NAME"], "manufacturer_id": d["MANUFACTURER_ID"],
        "mcu": d["FC_TARGET_MCU"], "resources": res, "uarts": uarts, "serial": serial,
        # config/feature.c: DEFAULT_FEATURES(0) | DEFAULT_RX_FEATURE | ANTI_GRAVITY | AIRMODE
        "features": {rx_feature}, "settings": {}, "spi_rx": "USE_RX_SPI" in flags,
    }


def board_from_unified(text):
    mcu = re.search(r"^#mcu (\S+)", text, re.M)
    mcu = mcu.group(1) if mcu else "STM32" + re.search(r"STM32(F\d+)", text).group(1)
    res = [(f"{a} {b}", c) for a, b, c in re.findall(r"^resource (\S+) (\d+) (\S+)", text, re.M)]
    uarts = sorted({int(b) for a, b, c in re.findall(r"^resource (SERIAL_[TR]X) (\d+) (\S+)", text, re.M)})
    serial = {int(i) + 1: int(f) for i, f in re.findall(r"^serial (\d+) (\d+) ", text, re.M)}
    feats = set(re.findall(r"^feature (\w+)", text, re.M))
    spi = "USE_RX_SPI" in text
    # unified targets inherit DEFAULT_RX_FEATURE from the generic MCU target: RX_SPI on the
    # SX1280 targets (RX_SPI_DEFAULT_PROTOCOL), RX_SERIAL otherwise.
    feats.add("RX_SPI" if spi else "RX_SERIAL")
    sets = dict(re.findall(r"^set (\w+) = (.*)$", text, re.M))
    return {
        "board_name": re.search(r"^board_name (\S+)", text, re.M).group(1),
        "manufacturer_id": re.search(r"^manufacturer_id (\S+)", text, re.M).group(1),
        "mcu": mcu, "resources": res, "uarts": uarts, "serial": serial,
        "features": feats, "settings": sets, "spi_rx": spi,
    }


def port_line(fw, name, fn):
    if fw == "2025.12":
        ident = name
    else:
        ident = {"VCP": "20", "SOFTSERIAL1": "30"}.get(name) or str(int(name[4:]) - 1)
    return f"serial {ident} {fn} 115200 57600 0 115200"


def compose(fw, board_key):
    tag = RELEASES[fw]
    sitl = open(os.path.join(ROOT, "sitl", tag, "dump_all.txt")).read()
    ut_file, cfg_dir = BOARDS[board_key]
    if fw == "4.4":
        src = f"boards/unified-targets/{ut_file}"
        b = board_from_unified(open(os.path.join(ROOT, src)).read())
    else:
        src = f"boards/config/{cfg_dir}/config.h"
        b = board_from_config_h(open(os.path.join(ROOT, src)).read())

    settings = dict(OVERLAY["all"])
    settings.update(OVERLAY[fw])
    if (fw, b["mcu"]) in MOTOR_PROTOCOL:
        settings["motor_pwm_protocol"] = MOTOR_PROTOCOL[(fw, b["mcu"])]
    if b["spi_rx"]:
        settings.update(ELRS)
        if fw in ELRS_SWITCH:
            settings["expresslrs_switch_mode"] = ELRS_SWITCH[fw]
    settings.update(osd_defaults(fw, 1024 if b["mcu"] == "STM32F405" else 512))
    settings.update(b["settings"])

    out, section, master_done = [], None, False
    lines = sitl.split("\n")
    for ln in lines:
        if ln.startswith("dump all"):
            continue
        if ln.startswith("# Betaflight / SITL (SITL)"):
            out.append(ln.replace("SITL (SITL)", f"{b['mcu']} ({MCU_CODE.get(b['mcu'], 'S4xx')})"))
            out.append(f"# COMPOSED FIXTURE (not a hardware recording): Betaflight {tag} SITL `dump all`"
                       f" + {src}; see tests/fixtures/README.md")
            continue
        if ln.startswith("board_name"):
            out.append(f"board_name {b['board_name']}")
            continue
        if ln.startswith("manufacturer_id"):
            out.append(f"manufacturer_id {b['manufacturer_id']}")
            continue
        if ln.startswith("mcu_id") or ln.startswith("signature"):
            out.append(ln)
            if ln.startswith("signature"):
                out.append("")
                out.append("# resources")
                out += [f"resource {k} {v}" for k, v in b["resources"]]
            continue
        if ln.startswith("# "):
            section = ln[2:].strip()
            if section == "map":
                out.append("# beeper")
                out += [f"beeper {n}" for n in BEEPERS[fw]]
                out += ["", "# beacon", "beacon -RX_LOST", "beacon -RX_SET", ""]
        if section == "feature" and ln.startswith("feature "):
            name = ln.split()[1].lstrip("-")
            on = name in b["features"] or name in ("AIRMODE", "ANTI_GRAVITY")
            # SITL prints the always-on features a second time; keep one line per name.
            if any(o.lstrip("feature -") == name and o.startswith("feature") for o in out[-40:]):
                continue
            out.append(f"feature {'' if on else '-'}{name}")
            continue
        if section == "serial" and ln.startswith("serial "):
            if ln.startswith("serial UART1 ") or ln.startswith("serial 0 "):
                out.append(port_line(fw, "VCP", 1))
                for n in b["uarts"]:
                    out.append(port_line(fw, f"UART{n}", b["serial"].get(n, 0)))
                if "SOFTSERIAL" in b["features"]:
                    # the board enables soft serial by default -> SOFTSERIAL1 (id 30) is listed
                    out.append(port_line(fw, "SOFTSERIAL1", 0))
            continue
        if section == "master" and ln.startswith("set "):
            k = ln[4:].split(" = ")[0]
            if k in settings:
                out.append(f"set {k} = {settings.pop(k)}")
            else:
                out.append(ln)
            continue
        if section == "master" and ln == "" and not master_done and out and out[-1].startswith("set "):
            # settings SITL cannot compile, appended at the end of the master section
            for k in sorted(settings):
                out.append(f"set {k} = {settings[k]}")
            settings.clear()
            master_done = True
        out.append(ln)
    text = "\n".join(out).rstrip("# \n") + "\n"
    return text


def main():
    for fw in RELEASES:
        for board in BOARDS:
            path = os.path.join(ROOT, fw, f"{board}.dump.txt")
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w") as f:
                f.write(compose(fw, board))
            print("wrote", os.path.relpath(path, ROOT))


if __name__ == "__main__":
    main()
