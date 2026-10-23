// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * UI-side typed wrappers for contract items that were agreed but are NOT yet in
 * src/api/commands.ts / src/types/model.ts (another agent is adding them in a
 * `shared` PR). Nothing here changes the shared contract.
 *
 * TODO(shared): when the shared PR lands, move these entries into CommandMap /
 * model.ts, delete this file and point imports at `api.*`. Search for
 * "TODO(shared)" to find every call site.
 *
 * Until then every wrapper degrades gracefully: if the running backend does not
 * know a command, the call rejects with { code: "NOT_IMPLEMENTED" } and the UI
 * shows a disabled control with guidance instead of failing.
 */
import { invoke } from "@tauri-apps/api/core";
import type { AppError, Backup, CliScript, DroneModel, FcSerialPort, VerifyReport } from "../types/model";

// ---- types (proposed shapes; confirm in the shared PR) -------------------------

/** Stages emitted by config_apply on `config://apply-progress` (agreed). */
export const APPLY_STAGES = ["BACKUP", "SENDING", "SAVING", "REBOOTING", "RECONNECTED"] as const;
export type ApplyStage = (typeof APPLY_STAGES)[number];
export interface ApplyProgress {
  stage: ApplyStage;
  /** 0..1 within the stage, or null if unknown. */
  fraction: number | null;
  message: string | null;
}

/** Payload of `device://rc-channels` while rc_channels_stream is enabled. */
export interface RcChannels {
  /** Raw channel values in µs (1000..2000), channel 1 first. */
  channels: number[];
}

/** Agreed error codes that are not in error.rs yet. */
export const UPCOMING_ERROR_CODES = [
  "BOARD_NOT_FOUND",
  "PORT_BUSY",
  "DRIVER_MISSING",
  "DFU_NOT_FOUND",
  "FLASH_FAILED",
  "VERIFY_MISMATCH",
] as const;

export const UPCOMING_EVENTS = {
  applyProgress: "config://apply-progress",
  rcChannels: "device://rc-channels",
} as const;

/** Wizard-managed roles, as named in a config plan. */
export const PLAN_ROLES = ["RX", "VTX", "GPS", "ESC_TELEMETRY"] as const;
export type PlanRole = (typeof PLAN_ROLES)[number];

/** One port the plan will clear managed roles from (board-reported port NAME). */
export interface PlanClearing {
  port: string;
  roles: PlanRole[];
}

/** Read-only result of config_plan: what apply WOULD do, nothing is written. */
export interface ConfigPlan {
  commands: string[];
  warnings: string[];
  dialect: string;
  clearings: PlanClearing[];
}

export const planToScript = (p: ConfigPlan): CliScript => ({ commands: p.commands, warnings: p.warnings, dialect: p.dialect });

export interface UpcomingCommandMap {
  /** Read-only preview of apply: CLI commands plus the managed roles/ports it clears. */
  config_plan: [{ model: DroneModel; ports: FcSerialPort[] }, ConfigPlan];
  /** Backups for the connected board (%APPDATA%/<app>/backups/<board-uid>/), newest first. */
  config_list_backups: [Record<string, never>, Backup[]];
  /** Spin one motor (0-based index, Betaflight order) at a low test throttle (percent, 0..100). */
  motor_test_start: [{ motor: number; throttlePercent: number }, void];
  /** Stop all motors. Must be safe to call at any time. */
  motor_test_stop: [Record<string, never>, void];
  /** Start/stop streaming RC channels as `device://rc-channels` events. */
  rc_channels_stream: [{ enabled: boolean }, void];
}
export type UpcomingCommand = keyof UpcomingCommandMap;
export type UpcomingBackend = <K extends UpcomingCommand>(cmd: K, args: UpcomingCommandMap[K][0]) => Promise<UpcomingCommandMap[K][1]>;

// ---- verify result normalisation ---------------------------------------------

/**
 * Agreed verify shape is `{ ok, mismatches: [{ key, expected, actual }] }`, while
 * model.ts still says `Mismatch.setting`. Accept both until the shared PR lands.
 */
export interface VerifyMismatch {
  key: string;
  expected: string;
  actual: string;
}
export interface VerifyOutcome {
  ok: boolean;
  mismatches: VerifyMismatch[];
}
export function normalizeVerify(r: VerifyReport): VerifyOutcome {
  const mismatches = (r.mismatches ?? []).map((m) => {
    const raw = m as unknown as { key?: string; setting?: string; expected?: unknown; actual?: unknown };
    return { key: String(raw.key ?? raw.setting ?? "?"), expected: String(raw.expected ?? ""), actual: String(raw.actual ?? "") };
  });
  return { ok: r.ok && mismatches.length === 0, mismatches };
}

// ---- backend -----------------------------------------------------------------

/** Tauri reports unknown commands as a plain string; map that to NOT_IMPLEMENTED. */
export function toNotImplemented(cmd: string, e: unknown): unknown {
  if (typeof e === "string" && /not found|unknown command/i.test(e)) {
    return { code: "NOT_IMPLEMENTED", message: `backend has no ${cmd} yet` } satisfies AppError;
  }
  return e;
}

const tauriBackend: UpcomingBackend = async (cmd, args) => {
  try {
    return (await invoke(cmd, args as Record<string, unknown>)) as never;
  } catch (e) {
    throw toNotImplemented(cmd, e);
  }
};

let backend: UpcomingBackend = tauriBackend;
export function setUpcomingBackend(b: UpcomingBackend): void {
  backend = b;
}
export function resetUpcomingBackend(): void {
  backend = tauriBackend;
}

/** TODO(shared): becomes `api.config.plan` / `api.config.listBackups` / `api.device.motorTest*` / `api.device.rcChannelsStream`. */
export const upcoming = {
  configPlan: (model: DroneModel, ports: FcSerialPort[]) => backend("config_plan", { model, ports }),
  configListBackups: () => backend("config_list_backups", {} as Record<string, never>),
  motorTestStart: (motor: number, throttlePercent: number) => backend("motor_test_start", { motor, throttlePercent }),
  motorTestStop: () => backend("motor_test_stop", {} as Record<string, never>),
  rcChannelsStream: (enabled: boolean) => backend("rc_channels_stream", { enabled }),
};
