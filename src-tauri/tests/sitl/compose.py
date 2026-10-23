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
Layer 3  settings that SITL cannot compile (DShot, GPS, OSD, SPI ELRS) with the
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
        "osd_displayport_device": "AUTO",     # osd/osd.c (MAX7456 present)
        "vcd_video_system": "AUTO",           # pg/vcd.c VIDEO_SYSTEM_AUTO
    },
    "4.4": {"dshot_idle_value": "550"},       # pg/motor.c digitalIdleOffsetValue = 550
    "4.5": {"dshot_idle_value": "550"},       # pg/motor.c digitalIdleOffsetValue = 550
    "2025.12": {},                            # motor_idle is present in SITL
}
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
