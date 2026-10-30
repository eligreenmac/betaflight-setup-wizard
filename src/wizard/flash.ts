// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Firmware update orchestration (docs/ARCHITECTURE.md §6), driven from the UI:
 * fetch -> backup -> bootloader -> wait for DFU -> flash (events) -> reconnect -> restore.
 *
 * Two independent options (both may be on): `fullErase` wipes the whole chip before writing
 * (flasher_flash FlashOptions.fullErase, a verified mass erase); `keepSettings` (default on)
 * puts the settings backup back afterwards (config_restore_after_flash, wizard/restore.ts).
 * A Betaflight board is always backed up before the reboot, whatever the options, and the
 * backup is kept. A restore that is refused or fails never fails the update: the firmware
 * is installed, the result says the settings weren't restored, and the backup stays listed.
 * Kept out of the .vue file so it can be tested against the scenario mock.
 */
import { api } from "../api/commands";
import { onEvent } from "../api/events";
import { toUiError, uiError, type ErrorCode, type UiError } from "../errors/codes";
import type { Backup, DfuDevice, FcInfo, FcSerialPort, FirmwareRequirements, FlashProgress } from "../types/model";
import { restoreAfterFlash, type RestoreProgress, type RestoreReport } from "./restore";

export const UPDATE_STAGES = ["download", "backup", "bootloader", "flash", "reconnect", "restore"] as const;
export type UpdateStage = (typeof UPDATE_STAGES)[number];

export interface UpdateHooks {
  onStage: (stage: UpdateStage) => void;
  onProgress: (p: FlashProgress) => void;
  /** The DFU device that will be flashed (needed to remove read protection after READ_PROTECTED). */
  onDfu?: (d: DfuDevice) => void;
  /** A settings backup was taken (reported right away so it survives a later failure). */
  onBackup?: (b: Backup) => void;
  /** config://progress while the settings are put back (RESTORING line n/total, SAVING, REBOOTING, RECONNECTED). */
  onRestoreProgress?: (p: RestoreProgress) => void;
}

export interface UpdateResult {
  fc: FcInfo;
  ports: FcSerialPort[];
  path: string;
  backup: Backup | null;
  /** Board options the release does not offer, left out of the build (opaque names; show them as-is). */
  droppedOptions: string[];
  /** What the restore did (keepSettings on and it ran); null otherwise. */
  restore: RestoreReport | null;
  /** keepSettings on but the settings were not put back (refusal / failure). The flash still succeeded. */
  restoreError: UiError | null;
  /** The backup the restore used or would have used; null = keepSettings off, or no backup to restore. */
  restoreBackupId: string | null;
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
  /** Put the backed-up settings back after the update (default true). Independent of fullErase. */
  keepSettings?: boolean;
  /**
   * A backup this same update already took (an earlier attempt that failed after the
   * reboot, e.g. VERIFY_MISMATCH -> full erase): the board is in DFU now, so no new one.
   */
  previousBackup?: Backup | null;
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
  let newFc = await api.device.identify();
  let ports = await api.config.readPorts();
  const result: UpdateResult = { fc: newFc, ports, path: host.path, backup, droppedOptions: image.droppedOptions ?? [], restore: null, restoreError: null, restoreBackupId: null };

  const source = backup ?? opts.previousBackup ?? null;
  if ((opts.keepSettings ?? true) && source) {
    hooks.onStage("restore");
    result.restoreBackupId = source.id;
    const r = await restoreSettings(source.id, hooks);
    result.restore = r.report;
    result.restoreError = r.error;
    if (r.report) {
      // The restore rebooted the board and changed its settings: read them again.
      try {
        newFc = await api.device.identify();
        ports = await api.config.readPorts();
        Object.assign(result, { fc: newFc, ports });
      } catch {
        // Keep what was read right after the flash; the next step reads the board again anyway.
      }
    }
  }
  return result;
}

/** config_restore_after_flash with its progress; never throws (a refusal is returned as `error`). */
export async function restoreSettings(backupId: string, hooks: Pick<UpdateHooks, "onRestoreProgress">): Promise<{ report: RestoreReport | null; error: UiError | null }> {
  const unlisten = await onEvent("config://progress", (p) => hooks.onRestoreProgress?.(p as RestoreProgress));
  try {
    return { report: await restoreAfterFlash(backupId), error: null };
  } catch (e) {
    return { report: null, error: toUiError(e) };
  } finally {
    unlisten();
  }
}

/**
 * "Try again" after a restore that did not happen (e.g. BOARD_NOT_RECONNECTED): reconnect if
 * the board is not answering, then restore the same backup. Never throws.
 */
export async function retryRestore(
  backupId: string,
  hooks: Pick<UpdateHooks, "onRestoreProgress">,
  poll: PollOptions = DEFAULT_POLL,
): Promise<{ report: RestoreReport | null; error: UiError | null; fc: FcInfo | null; ports: FcSerialPort[] | null; path: string | null }> {
  let path: string | null = null;
  try {
    await api.device.identify();
  } catch {
    const host = await pollUntil(async () => (await api.device.listPorts()).find((p) => p.kind === "FC_VCP"), poll);
    if (!host) return { report: null, error: fail("BOARD_NOT_FOUND", "board not found"), fc: null, ports: null, path: null };
    try {
      await api.device.connect(host.path);
      path = host.path;
    } catch (e) {
      return { report: null, error: toUiError(e), fc: null, ports: null, path: null };
    }
  }
  const r = await restoreSettings(backupId, hooks);
  let fc: FcInfo | null = null;
  let ports: FcSerialPort[] | null = null;
  try {
    fc = await api.device.identify();
    ports = await api.config.readPorts();
  } catch {
    // Reported through the restore result; the board is read again on the next step.
  }
  return { ...r, fc, ports, path };
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
