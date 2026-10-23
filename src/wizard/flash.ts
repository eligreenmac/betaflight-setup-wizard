// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Firmware update orchestration (docs/ARCHITECTURE.md §6), driven from the UI:
 * fetch -> backup -> bootloader -> wait for DFU -> flash (events) -> reconnect.
 * Kept out of the .vue file so it can be tested against the scenario mock.
 */
import { api } from "../api/commands";
import { onEvent } from "../api/events";
import { uiError, type ErrorCode, type UiError } from "../errors/codes";
import type { Backup, DfuDevice, FcInfo, FcSerialPort, FirmwareRequirements, FlashProgress } from "../types/model";

export const UPDATE_STAGES = ["download", "backup", "bootloader", "flash", "reconnect"] as const;
export type UpdateStage = (typeof UPDATE_STAGES)[number];

export interface UpdateHooks {
  onStage: (stage: UpdateStage) => void;
  onProgress: (p: FlashProgress) => void;
  /** The DFU device that will be flashed (needed to remove read protection after READ_PROTECTED). */
  onDfu?: (d: DfuDevice) => void;
  /** A settings backup was taken (reported right away so it survives a later failure). */
  onBackup?: (b: Backup) => void;
}

export interface UpdateResult {
  fc: FcInfo;
  ports: FcSerialPort[];
  path: string;
  backup: Backup | null;
  /** Board options the release does not offer, left out of the build (opaque names; show them as-is). */
  droppedOptions: string[];
}

export interface PollOptions {
  tries: number;
  intervalMs: number;
}
export const DEFAULT_POLL: PollOptions = { tries: 30, intervalMs: 500 };

const fail = (code: ErrorCode, message: string): UiError => uiError(code, message);
const sleep = (ms: number) => (ms > 0 ? new Promise<void>((r) => setTimeout(r, ms)) : Promise.resolve());

export async function pollUntil<T>(fn: () => Promise<T | null | undefined>, opts: PollOptions): Promise<T | null> {
  for (let i = 0; i < opts.tries; i++) {
    const v = await fn();
    if (v !== null && v !== undefined) return v;
    await sleep(opts.intervalMs);
  }
  return null;
}

/**
 * Runs the whole update. Re-entrant after a failure: if the board is already in DFU
 * (e.g. after FLASH_FAILED or after installing the DFU driver) it skips the reboot.
 * Throws UiError (DESIGN.md §6); DFU_NOT_FOUND / DRIVER_MISSING are raised UI-side
 * when the backend can't (yet) report them itself.
 */
export interface UpdateOptions {
  /** Erase the whole chip (offered after VERIFY_MISMATCH, as DESIGN.md §6 suggests). */
  fullErase?: boolean;
  /**
   * What the wizard's model needs in the firmware (receiver kind/protocol, OSD video system).
   * The backend turns it into the release's build options (`flasher_required_options`), so
   * option names never appear in the UI. Omitted/null: only the board's own options.
   */
  requirements?: FirmwareRequirements | null;
}

export async function runFirmwareUpdate(
  fc: FcInfo,
  release: string,
  hooks: UpdateHooks,
  poll: PollOptions = DEFAULT_POLL,
  opts: UpdateOptions = {},
): Promise<UpdateResult> {
  hooks.onStage("download");
  // REQUIRED_OPTION_UNSUPPORTED / BUILD_OPTIONS_UNAVAILABLE surface here, before anything is touched.
  const requiredOptions = opts.requirements ? await api.flasher.requiredOptions(opts.requirements, release) : [];
  const image = await api.flasher.fetchFirmware({ target: fc.boardName, release, options: fc.buildOptions, requiredOptions, offlineOnly: false });

  let dfu: DfuDevice | null = (await api.flasher.listDfuDevices())[0] ?? null;
  let backup: Backup | null = null;
  if (!dfu) {
    hooks.onStage("backup");
    if (fc.fcVariant === "BTFL") {
      backup = await api.config.backup();
      hooks.onBackup?.(backup);
    }
    hooks.onStage("bootloader");
    await api.device.rebootToBootloader();
    dfu = await pollUntil(async () => (await api.flasher.listDfuDevices())[0], poll);
  } else {
    hooks.onStage("bootloader");
  }
  if (!dfu) throw fail("DFU_NOT_FOUND", "board did not appear in DFU mode");
  if (!dfu.driverOk) throw fail("DRIVER_MISSING", `WinUSB driver missing for ${dfu.vid.toString(16)}:${dfu.pid.toString(16)}`);

  hooks.onDfu?.(dfu);
  hooks.onStage("flash");
  const unlisten = await onEvent("flasher://progress", hooks.onProgress);
  try {
    await api.flasher.flash(image, dfu.serial, { fullErase: opts.fullErase ?? false, manifest: true });
  } finally {
    unlisten();
  }

  hooks.onStage("reconnect");
  const host = await pollUntil(async () => (await api.device.listPorts()).find((p) => p.kind === "FC_VCP"), poll);
  if (!host) throw fail("BOARD_NOT_FOUND", "board did not come back as a COM port after flashing");
  await api.device.connect(host.path);
  const newFc = await api.device.identify();
  const ports = await api.config.readPorts();
  return { fc: newFc, ports, path: host.path, backup, droppedOptions: image.droppedOptions ?? [] };
}

/**
 * After READ_PROTECTED: remove the protection, which ERASES THE WHOLE CHIP (firmware and
 * settings). Only call this after the user explicitly confirmed it (ReadProtectedPanel);
 * it is never done automatically (ARCHITECTURE §2.3). The board re-enumerates in DFU,
 * so wait for it before flashing again with runFirmwareUpdate (which then skips the reboot).
 */
export async function removeReadProtection(dfuSerial: string | null, poll: PollOptions = DEFAULT_POLL): Promise<DfuDevice> {
  await api.flasher.removeReadProtection(dfuSerial, true);
  const dfu = await pollUntil(async () => (await api.flasher.listDfuDevices())[0], poll);
  if (!dfu) throw fail("DFU_NOT_FOUND", "board did not come back in DFU mode after removing read protection");
  return dfu;
}
