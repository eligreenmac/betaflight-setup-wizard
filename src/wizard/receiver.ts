// SPDX-License-Identifier: GPL-3.0-or-later
/** Receiver options offered by the wizard (MVP). */
import type { FcInfo, RxProtocol } from "../types/model";

/** Serial receiver protocols, most common first. */
export const SERIAL_RX_PROTOCOLS: RxProtocol[] = ["CRSF", "GHST", "SBUS", "FPORT", "IBUS", "SRXL2", "SUMD", "SPEK2048"];

/** MVP onboard (SPI) receiver: ExpressLRS only, and only when the board has one. */
export const SPI_RX_PROTOCOLS: RxProtocol[] = ["EXPRESSLRS"];

/**
 * Build options that mean "this board has an onboard SPI ExpressLRS receiver".
 * TODO(device): confirm which token(s) device_identify reports in FcInfo.buildOptions.
 */
export const ONBOARD_ELRS_BUILD_OPTIONS = ["USE_RX_EXPRESSLRS", "USE_RX_SPI_EXPRESSLRS", "RX_EXPRESSLRS"] as const;

export function hasOnboardElrs(fc: FcInfo | null): boolean {
  return !!fc && fc.buildOptions.some((o) => (ONBOARD_ELRS_BUILD_OPTIONS as readonly string[]).includes(o.toUpperCase()));
}

/** Protocols where the wizard asks "send telemetry back to the radio?" (default yes). */
export const TELEMETRY_QUESTION_PROTOCOLS: RxProtocol[] = ["CRSF", "FPORT"];

export function asksTelemetry(p: RxProtocol | null): boolean {
  return !!p && TELEMETRY_QUESTION_PROTOCOLS.includes(p);
}

/** Default telemetry answer: yes where we ask (and for onboard ELRS), otherwise off. */
export function defaultTelemetry(p: RxProtocol): boolean {
  return asksTelemetry(p) || p === "EXPRESSLRS";
}
