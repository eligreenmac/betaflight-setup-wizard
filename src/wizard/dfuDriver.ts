// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Installing WinUSB for the DFU bootloader (system_install_dfu_driver) rebinds the
 * device. When another driver is bound (DriverStatus.devices[].replacesDriver, e.g. ST's
 * STTub30 used by STM32CubeProgrammer / DfuSe), those tools may stop seeing the board,
 * so the install needs an explicit confirmation first. Nothing is installed without it.
 */
import { api } from "../api/commands";
import type { UsbDriverInfo } from "../types/model";

export type DfuDriverInstall =
  | { installed: true }
  /** Not installed: these devices would lose their current driver; ask first. */
  | { installed: false; replaces: UsbDriverInfo[] };

/** DFU devices whose current driver WinUSB would replace (fresh status from the backend). */
export async function driversToReplace(): Promise<UsbDriverInfo[]> {
  const status = await api.system.driverStatus();
  return status.devices.filter((d) => d.replacesDriver);
}

/**
 * Install the DFU driver, unless it would replace another driver and the user has not
 * confirmed that (`confirmedReplace`). Errors from the backend are passed through.
 */
export async function installDfuDriverSafely(confirmedReplace: boolean): Promise<DfuDriverInstall> {
  const replaces = await driversToReplace();
  if (replaces.length && !confirmedReplace) return { installed: false, replaces };
  await api.system.installDfuDriver();
  return { installed: true };
}

/** Driver names for the warning ("STTub30"), unknown ones as null. */
export function replacedDriverNames(replaces: UsbDriverInfo[]): string[] {
  return [...new Set(replaces.map((d) => d.service).filter((s): s is string => !!s))];
}
