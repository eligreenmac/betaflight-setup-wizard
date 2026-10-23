// SPDX-License-Identifier: GPL-3.0-or-later
/** Receiver options offered by the wizard (MVP). */
import { device, type DeviceFcInfo } from "../api/device";
import type { FcInfo, RxProtocol } from "../types/model";
import type { WizardState } from "./state";
import { compareVersions } from "./version";

/** Serial receiver protocols, most common first. */
export const SERIAL_RX_PROTOCOLS: RxProtocol[] = ["CRSF", "GHST", "SBUS", "FPORT", "IBUS", "SRXL2", "SUMD", "SPEK2048"];

/** MVP onboard (SPI) receiver: ExpressLRS only, and only when the board has one. */
export const SPI_RX_PROTOCOLS: RxProtocol[] = ["EXPRESSLRS"];

/**
 * Build options that hint at an onboard SPI ExpressLRS receiver. Only a fallback:
 * MSP cannot tell whether the board *has* an SPI receiver (ARCHITECTURE §4.1).
 */
export const ONBOARD_ELRS_BUILD_OPTIONS = ["USE_RX_EXPRESSLRS", "USE_RX_SPI_EXPRESSLRS", "RX_EXPRESSLRS"] as const;

/**
 * Onboard SPI receiver? `rxSpi.present` (RX_SPI_CS assigned, from device_probe_board)
 * decides when known; before the probe (`present: null`) fall back to the ELRS pins
 * and build-option hints. config_plan still refuses non-ExpressLRS SPI protocols.
 */
export function hasOnboardElrs(fc: DeviceFcInfo | null): boolean {
  if (!fc) return false;
  if (fc.rxSpi && fc.rxSpi.present !== null) return fc.rxSpi.present;
  return !!fc.rxSpi?.expresslrsPins || fc.buildOptions.some((o) => (ONBOARD_ELRS_BUILD_OPTIONS as readonly string[]).includes(o.toUpperCase()));
}

/** device_probe_board leaves the CLI without a reboot only on 4.5+ (on 4.4 the FC reboots). */
export const PROBE_WITHOUT_REBOOT_SINCE = "4.5.0";

/**
 * Best-effort device_probe_board after identify, to learn `rxSpi.present`.
 * Skipped before 4.5 and on non-Betaflight firmware; any failure (e.g. NOT_IMPLEMENTED
 * on an older backend) keeps the identify result.
 */
export async function probeOnboardReceiver(info: FcInfo, probe: () => Promise<DeviceFcInfo> = () => device.probeBoard()): Promise<DeviceFcInfo> {
  if (info.fcVariant !== "BTFL" || compareVersions(info.firmwareVersion, PROBE_WITHOUT_REBOOT_SINCE) < 0) return info;
  try {
    return await probe();
  } catch {
    return info;
  }
}

/**
 * Protocols where the wizard asks "send telemetry back to the radio?" (default yes).
 * Same set the config engine sets telemetry for (ARCHITECTURE §5.1): CRSF, FPORT and
 * onboard SPI ExpressLRS (receiver kind SPI, protocol EXPRESSLRS).
 */
export const TELEMETRY_QUESTION_PROTOCOLS: RxProtocol[] = ["CRSF", "FPORT", "EXPRESSLRS"];

export function asksTelemetry(p: RxProtocol | null): boolean {
  return !!p && TELEMETRY_QUESTION_PROTOCOLS.includes(p);
}

/** Default telemetry answer: yes where we ask, otherwise off (ignored by the engine). */
export function defaultTelemetry(p: RxProtocol): boolean {
  return asksTelemetry(p);
}

/** The Binding step applies: ExpressLRS on SPI, or CRSF on a UART (ExpressLRS or Crossfire). */
export function bindingApplies(s: WizardState): boolean {
  const r = s.model.receiver;
  return (r.kind === "SPI" && r.protocol === "EXPRESSLRS") || (r.kind === "SERIAL" && r.protocol === "CRSF");
}

/** The receiver is ExpressLRS (onboard SPI, or a UART CRSF receiver the user says is ExpressLRS). */
export function isElrs(s: WizardState): boolean {
  const r = s.model.receiver;
  return (r.kind === "SPI" && r.protocol === "EXPRESSLRS") || (r.kind === "SERIAL" && r.protocol === "CRSF" && s.ui.crsfFamily === "ELRS");
}
