// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * TODO(shared): exact copy of the DroneModel batch-2 types proposed in shared PR #65
 * (src/types/model.ts on `shared/model-batch2`). src/types/model.ts is a shared file and
 * is not edited here; when #65 lands, import these from "../types/model" and delete this
 * file. Do not change anything here that #65 does not have.
 */
import type { DroneModel } from "./model";

export type Airmode = "ALWAYS" | "SWITCH" | "OFF";
/** Stable names in `switches[].mode` for the optional modes (config::modes canonical names). */
export type OptionalMode = "PREARM" | "FLIP OVER AFTER CRASH" | "AIR MODE";

/** 4.4/4.5 CLI names. 2025.12: CRASH_FLIP is written as CRASHFLIP; RC_SMOOTHING_INIT_FAIL doesn't exist (warning). */
export const BEEPER_NAMES = [
  "GYRO_CALIBRATED", "RX_LOST", "RX_LOST_LANDING", "DISARMING", "ARMING", "ARMING_GPS_FIX",
  "ARMING_GPS_NO_FIX", "BAT_CRIT_LOW", "BAT_LOW", "GPS_STATUS", "RX_SET", "ACC_CALIBRATION",
  "ACC_CALIBRATION_FAIL", "READY_BEEP", "MULTI_BEEPS", "DISARM_REPEAT", "ARMED", "SYSTEM_INIT",
  "ON_USB", "BLACKBOX_ERASE", "CRASH_FLIP", "CAM_CONNECTION_OPEN", "CAM_CONNECTION_CLOSE",
  "RC_SMOOTHING_INIT_FAIL",
] as const;
export type BeeperName = (typeof BEEPER_NAMES)[number];
export type DshotBeaconCondition = "RX_LOST" | "RX_SET";
/** Full state: `beeper -X` for listed, `beeper X` for every other condition the firmware has. */
export interface Beepers {
  disabled: BeeperName[];
  /** `beacon -X` for listed, `beacon X` for the other; null = keep. */
  dshotBeaconDisabled?: DshotBeaconCondition[] | null;
}

export interface BoardAlignment { roll: number; pitch: number; yaw: number }

/** vcd_video_system. Grid: PAL 30x16, NTSC 30x13, AUTO 30x13, HD 53x20. */
export type OsdVideoSystem = "AUTO" | "PAL" | "NTSC" | "HD";
/** Betaflight osd_items_e name without OSD_, e.g. MAIN_BATT_VOLTAGE; per-firmware list from config_osd_default_layout. */
export type OsdElementName = string;
export interface OsdElement {
  name: OsdElementName;
  /** Visible in all 3 OSD profiles, or none. */
  enabled: boolean;
  /** null/absent = default layout position. Must fit the grid (CONFIG error otherwise). */
  x?: number | null;
  y?: number | null;
}
export interface Osd {
  /** null = keep the FC's video system (grid from the FC). */
  videoSystem?: OsdVideoSystem | null;
  /** Elements not listed keep the FC's state. */
  elements: OsdElement[];
}
/** config_osd_default_layout(videoSystem, firmwareVersion) */
export interface OsdLayout {
  videoSystem: OsdVideoSystem;
  cols: number;
  rows: number;
  elements: { name: OsdElementName; x: number; y: number; enabled: boolean }[];
}

/** The batch-2 optional fields of DroneModel (#65). Everything optional; null = keep. */
export interface DroneModelBatch2 {
  /** ALWAYS = feature AIRMODE; SWITCH = needs an "AIR MODE" entry in `switches`; OFF; null = keep. */
  airmode?: Airmode | null;
  /** Optional modes whose FC ranges are removed; optional modes neither here nor in `switches` are kept. */
  removeModes?: OptionalMode[];
  /** null = keep. */
  beepers?: Beepers | null;
  /** true = AUTO (a specific chip on the FC is kept), false = NONE, null = keep. Gyro is always on. */
  sensors?: { acc?: boolean | null; baro?: boolean | null; mag?: boolean | null } | null;
  /** Whole degrees, -180..360 (= what device_sensors() reports); null = keep. */
  boardAlignment?: BoardAlignment | null;
  /** null = keep the FC's OSD. */
  osd?: Osd | null;
}
/** DroneModel as #65 defines it. */
export type DroneModelNext = DroneModel & DroneModelBatch2;
