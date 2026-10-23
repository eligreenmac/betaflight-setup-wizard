// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Scenario mock of the whole backend for browser demo mode and UI tests.
 * Simulates a SpeedyBee F405 V4 on Betaflight 4.5.1 (latest stable 4.5.2) with:
 *   UART2 = serial RX, UART3 = MSP + VTX_MSP (digital VTX), UART6 = plain MSP (foreign).
 *
 * Covers the shared contract (src/api/commands.ts) via createMockBackend(), including
 * the device_motor_test_* / device_rc_stream_* commands of #38 (no port argument: the
 * connection manager holds the port; a motor test and the RC stream exclude each
 * other with PORT_BUSY {heldBy: "APP"}). The `legacy-backend` scenario behaves like a
 * Phase 0 backend: config_plan, config_list_backups, config_backups_dir,
 * config_restore, config_elrs_uid, device_probe_board and the motor/RC commands
 * answer NOT_IMPLEMENTED.
 *
 * Sensors / calibration / attitude (Device #63, api/pending.ts adapter): gyro, acc
 * and baro present (baro disabled), no mag. `no-sensors`: no acc and no baro.
 * `calib-fail`: the first accelerometer calibration fails. `unsaved-ram`: an earlier apply
 * left unsaved changes in RAM, so device_acc_calibrate answers PENDING_UNSAVED_CHANGES
 * {reason: "CLI_COMMAND"} (before any other check) until device_reboot; a reconnect never
 * clears it. `unsaved-ram-reboot-fail`: the same, and the first device_reboot is not
 * confirmed (PENDING_UNSAVED_CHANGES); pressing it again works. `device.rebootFailure =
 * "lost"` (tests): the board does not come back (SERIAL). `device.connectUnsavedFails`
 * (tests): device_connect answers PENDING_UNSAVED_CHANGES (its reboot of a board left dirty
 * was not confirmed). `mounted-yaw90`: the board is
 * mounted turned 90° to the right, so tilting the nose down shows as a roll until the
 * user picks the "arrow points right" (yaw 90) alignment. The attitude stream follows
 * `device.tilt` (tests) or, when null, tilts by itself: level, nose down, level, right
 * side down, every 2 s.
 *
 * HD OSD (FcInfo.hdOsdSupported, TODO(shared)): true by default. `osd-hd-unsupported`: false
 * (HD disabled in the OSD step). `osd-hd-unknown`: null (HD allowed, with a note).
 * `osd-hd-plan-error`: null, but the firmware has no MSP DisplayPort, so config_plan refuses
 * an HD OSD with #67's CONFIG error. `osd-sd-only`: no field, build options say "OSD (SD)".
 *
 * Pick a scenario with `?mock=<name>` or VITE_MOCK_SCENARIO.
 */
import type { Backend, CommandMap, CommandName } from "../api/commands";
import { createMockBackend } from "../api/mock";
import { createEmitter, type Emitter } from "../api/events";
import type { Backup, ClearedPort, CliScript, FcInfo, FcSerialPort, FirmwareCatalog, FlashStage, SerialFunction } from "../types/model";
import { FN, isReserved, plannedClearings, targetMask, type FunctionName } from "../wizard/ports";
import { uiError, type ErrorCode, type UiError } from "../errors/codes";
import { BEEPER_NAMES, type OsdLayout, type OsdVideoSystem } from "../types/model";
import type { PendingCommandName } from "../api/pending";
import type { ExtendedDroneModel, BoardAlignment } from "../wizard/modelExt";
import { reportedWithAlignment, type Euler } from "../wizard/orientation";
import { DEFAULT_MAX_TEST_THROTTLE, DEFAULT_MOTOR_WATCHDOG_MS, HARD_MAX_TEST_THROTTLE } from "../wizard/motorTest";
import { compareVersions } from "../wizard/version";

export const SCENARIOS = [
  "happy",
  "no-board",
  "port-busy",
  "driver-missing",
  "old-firmware",
  "flash-fail",
  "verify-mismatch",
  "slow",
  "legacy-backend",
  "onboard-elrs",
  "fw-2025",
  "rollback-fail",
  "flash-verify",
  "read-protected",
  "driver-replace",
  "no-sensors",
  "calib-fail",
  "mounted-yaw90",
  "unsaved-ram",
  "unsaved-ram-reboot-fail",
  "osd-sd-only",
  "osd-hd-unsupported",
  "osd-hd-unknown",
  "osd-hd-plan-error",
] as const;
export type Scenario = (typeof SCENARIOS)[number];

export interface MockOptions {
  scenario?: Scenario;
  /** Per-call latency in ms (0 = resolve immediately; used by tests). */
  latencyMs?: number;
  /** Delay between progress events in ms. */
  stepMs?: number;
}

export interface ScenarioMock {
  backend: Backend;
  events: Emitter;
  /** Inspectable device state (tests). */
  device: MockDevice;
}

interface MockDevice {
  firmwareVersion: string;
  ports: FcSerialPort[];
  inDfu: boolean;
  dfuDriverOk: boolean;
  /** ST's STTub30 (STM32CubeProgrammer / DfuSe) is bound to the DFU device: WinUSB would replace it. */
  dfuOtherDriver: string | null;
  /** system_install_dfu_driver calls (tests). */
  driverInstalls: number;
  flashFailuresLeft: number;
  verifyFailuresLeft: number;
  /** MCU read protection set (flasher_flash refuses until it is removed). */
  readProtected: boolean;
  /** flasher_remove_read_protection calls, with their confirmErase flag (tests). */
  unprotectLog: boolean[];
  motorsSpinning: Set<number>;
  /** Calls seen by the mock, in order (tests assert on stop / keepalive traffic). */
  motorLog: string[];
  /** Force the next device_motor_test_* call to fail with this error (tests). */
  motorFailNext: UiError | null;
  /** device_probe_board ran (fills FcInfo.rxSpi.present). */
  probed: boolean;
  armed: boolean;
  rcTimer: ReturnType<typeof setInterval> | null;
  backups: { backup: Backup; ports: FcSerialPort[] }[];
  sensors: { gyro: { present: boolean }; acc: { present: boolean; enabled: boolean }; baro: { present: boolean; enabled: boolean }; mag: { present: boolean; enabled: boolean } };
  accCalFailuresLeft: number;
  /** An earlier apply left unsaved changes in RAM; cleared only by a confirmed device_reboot. */
  unsavedRam: boolean;
  /** The next N device_reboot calls fail, as `rebootFailure` says. */
  rebootFailuresLeft: number;
  /** unconfirmed = PENDING_UNSAVED_CHANGES (flag still set) / MSP; lost = SERIAL (board did not come back). */
  rebootFailure: "unconfirmed" | "lost";
  /** device_connect fails with PENDING_UNSAVED_CHANGES (its reboot of a dirty board was not confirmed). */
  connectUnsavedFails: boolean;
  reboots: number;
  /** Successful device_acc_calibrate calls (tests). */
  accCalibrations: number;
  /** How the board is mounted on the frame (the truth the alignment check should find). */
  mount: BoardAlignment;
  /** align_board_* currently stored on the FC (reported by device_sensors). */
  alignment: BoardAlignment;
  accCalibrated: boolean;
  /** Simulated frame tilt; null = automatic demo cycle. */
  tilt: Euler | null;
  attitudeTimer: ReturnType<typeof setInterval> | null;
}

/** Errors in the DESIGN.md §6 UiError shape. NOT_IMPLEMENTED mimics the Phase 0 stubs. */
const err = (code: ErrorCode, message: string): UiError => uiError(code, message);
const portBusy = (port: string, heldBy: "APP" | "EXTERNAL", message: string): UiError => ({ ...uiError("PORT_BUSY", message), details: { port, heldBy } });
const UID = "3a0021000c51333038363834";

export function isScenario(s: string | null | undefined): s is Scenario {
  return !!s && (SCENARIOS as readonly string[]).includes(s);
}

export function createScenarioMock(opts: MockOptions = {}): ScenarioMock {
  const scenario = opts.scenario ?? "happy";
  const latency = opts.latencyMs ?? (scenario === "slow" ? 1500 : 250);
  const stepMs = opts.stepMs ?? (latency === 0 ? 0 : scenario === "slow" ? 900 : 350);
  const sleep = (ms: number) => (ms > 0 ? new Promise<void>((r) => setTimeout(r, ms)) : Promise.resolve());
  const events = createEmitter();

  const device: MockDevice = {
    firmwareVersion: scenario === "old-firmware" ? "4.3.2" : scenario === "fw-2025" ? "2025.12.0" : "4.5.1",
    // 4.x: UART1 = 0; 2025.12+: UART1 = 51. The UI must only use the names.
    ports: [
      { name: "VCP", identifier: 20, functionMask: FN.MSP, isConnectionPort: true },
      ...[0, FN.RX_SERIAL, FN.MSP | FN.VTX_MSP, 0, 0, FN.MSP].map((functionMask, i) => ({
        name: `UART${i + 1}`,
        identifier: (scenario === "fw-2025" ? 51 : 0) + i,
        functionMask,
        isConnectionPort: false,
      })),
      { name: "SOFTSERIAL1", identifier: 30, functionMask: 0, isConnectionPort: false },
    ],
    inDfu: false,
    dfuDriverOk: scenario !== "driver-missing" && scenario !== "driver-replace",
    dfuOtherDriver: scenario === "driver-replace" ? "STTub30" : null,
    driverInstalls: 0,
    flashFailuresLeft: scenario === "flash-fail" ? 1 : 0,
    verifyFailuresLeft: scenario === "verify-mismatch" ? 1 : 0,
    readProtected: scenario === "read-protected",
    unprotectLog: [],
    motorsSpinning: new Set(),
    motorLog: [],
    motorFailNext: null,
    armed: false,
    rcTimer: null,
    probed: false,
    backups: [],
    sensors:
      scenario === "no-sensors"
        ? // Set to AUTO but nothing found.
          { gyro: { present: true }, acc: { present: false, enabled: true }, baro: { present: false, enabled: true }, mag: { present: false, enabled: true } }
        : // A disabled sensor is never initialised, so never present (#63 §4.3).
          { gyro: { present: true }, acc: { present: true, enabled: true }, baro: { present: false, enabled: false }, mag: { present: false, enabled: true } },
    accCalFailuresLeft: scenario === "calib-fail" ? 1 : 0,
    unsavedRam: scenario === "unsaved-ram" || scenario === "unsaved-ram-reboot-fail",
    rebootFailuresLeft: scenario === "unsaved-ram-reboot-fail" ? 1 : 0,
    rebootFailure: "unconfirmed",
    connectUnsavedFails: false,
    reboots: 0,
    accCalibrations: 0,
    mount: scenario === "mounted-yaw90" ? { roll: 0, pitch: 0, yaw: 90 } : { roll: 0, pitch: 0, yaw: 0 },
    alignment: { roll: 0, pitch: 0, yaw: 0 },
    accCalibrated: false,
    tilt: null,
    attitudeTimer: null,
  };
  const snapshot = (): Backup => {
    const createdAt = new Date(Date.UTC(2026, 9, 23, 4, 0, device.backups.length)).toISOString();
    const stamp = createdAt.replace(/[-:]/g, "").replace(/\.\d+Z$/, "Z");
    const backup: Backup = {
      id: `${UID}/${stamp}`,
      boardUid: UID,
      path: `C:\\Users\\pilot\\AppData\\Roaming\\io.github.eligreenmac.bfsw\\backups\\${UID}\\${stamp}.txt`,
      boardName: "SPEEDYBEEF405V4",
      firmwareVersion: device.firmwareVersion,
      createdAt,
    };
    device.backups.unshift({ backup, ports: device.ports.map((p) => ({ ...p })) });
    return backup;
  };

  type H = { [K in CommandName]?: (args: CommandMap[K][0]) => CommandMap[K][1] | Promise<CommandMap[K][1]> };
  const handlers: H = {
    system_app_info: () => ({ version: "0.1.0-mock", os: "mock", cacheDir: "/tmp/bfsw" }),
    system_driver_status: () => ({
      dfuWinusb: device.dfuDriverOk,
      vcp: true,
      notes: [],
      devices: device.inDfu
        ? [
            {
              vid: 0x0483,
              pid: 0xdf11,
              name: "STM32 BOOTLOADER",
              kind: "DFU",
              instanceId: "USB\\VID_0483&PID_DF11\\MOCKDFU1",
              service: device.dfuDriverOk ? "WinUSB" : device.dfuOtherDriver,
              driverOk: device.dfuDriverOk,
              action: device.dfuDriverOk ? "NONE" : "INSTALL_DFU_DRIVER",
              guidanceIndex: null,
              // Only the driver-replace scenario has another driver (STTub30) bound.
              replacesDriver: !device.dfuDriverOk && device.dfuOtherDriver !== null,
            },
          ]
        : [],
      guidance: [],
    }),
    system_install_dfu_driver: async () => {
      await sleep(latency * 2);
      device.driverInstalls++;
      device.dfuDriverOk = true;
      device.dfuOtherDriver = null;
    },
    device_list_ports: () =>
      scenario === "no-board" || device.inDfu
        ? []
        : [
            { path: "COM5", vid: 0x0483, pid: 0x5740, manufacturer: "Betaflight", product: "STM32 Virtual COM Port", kind: "FC_VCP" },
            { path: "COM1", vid: null, pid: null, manufacturer: null, product: "Communications Port", kind: "UNKNOWN" },
          ],
    device_connect: ({ path }) => {
      if (scenario === "port-busy") throw portBusy(path, "EXTERNAL", `${path}: Access is denied. (os error 5)`);
      if (path !== "COM5") throw err("SERIAL_ERROR", `${path}: no MSP response`);
      // A plain reconnect does not clear unsaved RAM changes (only device_reboot does).
      if (device.connectUnsavedFails)
        throw { ...err("PENDING_UNSAVED_CHANGES", "the board was left with unsaved changes and its reboot could not be confirmed"), details: { reason: "CLI_COMMAND" } };
    },
    device_disconnect: () => undefined,
    device_identify: () => fcInfo(),
    device_cli_exec: ({ line }) => `# ${line}\n`,
    device_reboot_to_bootloader: () => {
      device.inDfu = true;
    },
    // "Reboot board" (#68/#69): resolves once the reboot is confirmed and clears the unsaved
    // flag. Not confirmed: PENDING_UNSAVED_CHANGES while the flag is set (else MSP); the board
    // did not come back: SERIAL. Pressing it again is safe.
    device_reboot: async () => {
      await sleep(stepMs * 2);
      if (device.rebootFailuresLeft > 0) {
        device.rebootFailuresLeft--;
        if (device.rebootFailure === "lost") throw err("SERIAL_ERROR", "COM5: the board did not come back after the reboot");
        if (device.unsavedRam)
          throw { ...err("PENDING_UNSAVED_CHANGES", "the reboot was not confirmed; the flight controller may still have unsaved changes (CLI command)"), details: { reason: "CLI_COMMAND" } };
        throw err("MSP_BAD_RESPONSE", "MSP_REBOOT was not acknowledged");
      }
      device.reboots++;
      device.unsavedRam = false;
    },
    flasher_list_targets: () => [{ target: "SPEEDYBEEF405V4", manufacturer: "SPBE", mcu: "STM32F405" }],
    flasher_list_releases: () => [
      { release: "2025.12.0-RC1", prerelease: true },
      { release: "4.5.2", prerelease: false },
      { release: "4.5.1", prerelease: false },
      { release: "4.4.3", prerelease: false },
    ],
    flasher_list_dfu_devices: () =>
      device.inDfu
        ? [{ vid: 0x0483, pid: 0xdf11, serial: "MOCKDFU1", mcuFamily: "STM32F4", flashDescriptor: "@Internal Flash  /0x08000000/04*016Kg,01*064Kg,07*128Kg", driverOk: device.dfuDriverOk }]
        : [],
    flasher_fetch_firmware: async ({ request }) => {
      await stages(["DOWNLOADING"]);
      return { request, path: `/tmp/bfsw/${request.target}_${request.release}.hex`, format: "HEX", sha256: "0".repeat(64), buildKey: "mock", fromCache: false };
    },
    flasher_list_cache: () => [],
    flasher_flash: async ({ image, options }) => {
      if (!device.inDfu) throw err("DFU_NOT_FOUND", "no DFU device");
      if (!device.dfuDriverOk) throw err("DRIVER_MISSING", "WinUSB driver not installed for 0483:df11");
      // Checked before anything is erased (ARCHITECTURE §2.3).
      if (device.readProtected) throw err("READ_PROTECTED", "option bytes: RDP level 1");
      if (device.flashFailuresLeft > 0) {
        device.flashFailuresLeft--;
        await stages(["ERASING", "WRITING"]);
        events.emit("flasher://progress", { stage: "FAILED", fraction: 0.4, message: "write failed at 0x08020000" });
        device.inDfu = false;
        throw err("FLASH_FAILED", "DFU write failed at 0x08020000");
      }
      if (scenario === "flash-verify" && !options.fullErase) {
        await stages(["ERASING", "WRITING", "VERIFYING"]);
        throw err("VERIFY_MISMATCH", "verify failed at 0x08000010: wrote 0x01, read 0x02");
      }
      await stages(["ERASING", "WRITING", "VERIFYING", "REBOOTING"]);
      device.inDfu = false;
      device.firmwareVersion = image.request.release;
      events.emit("flasher://progress", { stage: "DONE", fraction: 1, message: null });
    },
    config_read_ports: () => device.ports.map((p) => ({ ...p })),
    config_backup: () => snapshot(),
    config_generate: ({ model, ports }) => generateCli(model, ports),
    config_apply: async ({ script }) => {
      let backup: Backup | null = null;
      for (const stage of ["BACKUP", "SENDING", "SAVING", "REBOOTING", "RECONNECTED"] as const) {
        if (stage === "BACKUP") backup = snapshot();
        events.emit("config://progress", { stage, fraction: 0, message: null });
        await sleep(stepMs);
      }
      // Reflect the serial changes on the "board" so a re-read shows them.
      for (const line of script.commands) {
        const m = /^serial (\d+) (\d+)/.exec(line);
        const port = m ? device.ports.find((p) => p.identifier === Number(m[1])) : undefined;
        if (port && m) port.functionMask = Number(m[2]);
      }
      return { applied: script.commands.length, errors: [], saved: true, backup, reconnected: true };
    },
    config_verify: () => {
      if (device.verifyFailuresLeft > 0) {
        device.verifyFailuresLeft--;
        return { ok: false, mismatches: [{ setting: "serialrx_provider", expected: "CRSF", actual: "SBUS" }] };
      }
      return { ok: true, mismatches: [] };
    },
    config_rollback: ({ backup }) => restore(backup.id),
    config_plan: ({ model }) => {
      legacy("config_plan");
      if (model.receiver.kind === "SPI" && model.receiver.protocol !== "EXPRESSLRS")
        throw { code: "CONFIG", message: `SPI receiver protocol ${model.receiver.protocol} is not supported` }; // error.rs AppError::Config
      if (model.receiver.kind === "SPI" && scenario !== "onboard-elrs") throw { code: "CONFIG", message: "no onboard SPI receiver (resource RX_SPI_CS not assigned)" };
      // config::generate (#67): an HD OSD on firmware without MSP DisplayPort.
      const osd = (model as { osd?: { videoSystem?: string | null } | null }).osd;
      const hdOsd = !!osd && (osd.videoSystem === "HD" || (!osd.videoSystem && model.vtx.kind === "DIGITAL"));
      if (hdOsd && NO_DISPLAYPORT.includes(scenario))
        throw { code: "CONFIG", message: "configuration error: HD OSD needs MSP DisplayPort, which this firmware build does not have" };
      return generateCli(model, device.ports);
    },
    config_list_backups: () => {
      legacy("config_list_backups");
      return device.backups.map((b) => b.backup);
    },
    config_backups_dir: () => {
      legacy("config_backups_dir");
      return "C:\\Users\\pilot\\AppData\\Roaming\\io.github.eligreenmac.bfsw\\backups";
    },
    config_restore: ({ backupId }) => {
      legacy("config_restore");
      return restore(backupId);
    },
    config_elrs_uid: ({ phrase }) => {
      legacy("config_elrs_uid");
      // Not the real MD5-based derivation; just 6 deterministic bytes for the demo.
      return Array.from({ length: 6 }, (_, i) => [...phrase].reduce((h, c) => (h * 31 + c.charCodeAt(0) + i) & 0xff, 7));
    },
    // Shared #66 (real in config, #67): demo answers only. Pure (no board I/O), so safe next to a real board.
    config_firmware_catalog: ({ firmwareVersion }) => mockFirmwareCatalog(firmwareVersion),
    config_osd_default_layout: ({ videoSystem, firmwareVersion }) => {
      legacy("config_osd_default_layout");
      if (!["AUTO", "PAL", "NTSC", "HD"].includes(videoSystem)) throw err("CONFIG_UNSUPPORTED", `unknown video system ${String(videoSystem)}`);
      return mockOsdLayout(videoSystem, firmwareVersion);
    },
  };

  function legacy(cmd: string): void {
    if (scenario === "legacy-backend") throw err("NOT_IMPLEMENTED", `mock: ${cmd}`);
  }
  async function restore(backupId: string) {
    await sleep(stepMs * 3);
    if (scenario === "rollback-fail") throw err("CLI_ERROR", "###ERROR### in line 12: serial 2 131073 115200 57600 0 115200");
    const hit = device.backups.find((b) => b.backup.id === backupId);
    if (!hit) throw err("IO_ERROR", `backup not found: ${backupId}`);
    const safety = snapshot(); // safety backup first (ARCHITECTURE §5.1)
    device.ports = hit.ports.map((p) => ({ ...p }));
    return { applied: 42, errors: [], saved: true, backup: safety, reconnected: true };
  }

  async function stages(list: FlashStage[]) {
    for (const stage of list) {
      for (const fraction of [0, 0.5, 1]) {
        events.emit("flasher://progress", { stage, fraction, message: null });
        await sleep(stepMs / 3);
      }
    }
  }

  function fcInfo(): FcInfo {
    const elrs = scenario === "onboard-elrs";
    return {
      fcVariant: "BTFL",
      firmwareVersion: device.firmwareVersion,
      apiVersion: device.firmwareVersion.startsWith("4.3") ? "1.44" : "1.46",
      boardName: "SPEEDYBEEF405V4",
      manufacturerId: "SPBE",
      mcu: "STM32F405",
      uid: UID,
      buildOptions: ["CRSF", "SBUS", "GPS", "VTX", "DSHOT_BITBANG", ...(elrs ? ["USE_RX_EXPRESSLRS"] : []), ...(scenario === "osd-sd-only" ? ["USE_OSD_SD"] : [])],
      // TODO(shared): Device's FcInfo.hdOsdSupported (boolean | null), read via wizard/osd.ts hdOsdSupport.
      // Absent for the old backends (legacy-backend, osd-sd-only: build-options heuristic).
      ...(scenario === "legacy-backend" || scenario === "osd-sd-only"
        ? {}
        : { hdOsdSupported: scenario === "osd-hd-unsupported" ? false : scenario === "osd-hd-unknown" || scenario === "osd-hd-plan-error" ? null : true }),
      serialPorts: device.ports.map((p) => ({ ...p })),
      // present stays null until device_probe_board ran (ARCHITECTURE §4.1).
      rxSpi: device.probed
        ? { present: elrs, expresslrsPins: elrs, protocol: elrs ? "EXPRESSLRS" : null, featureEnabled: elrs, resources: elrs ? ["RX_SPI_CS", "RX_SPI_EXTI", "RX_SPI_BIND", "RX_SPI_LED"] : [] }
        : { present: null, expresslrsPins: elrs, protocol: null, featureEnabled: null, resources: [] },
    };
  }

  // ---- motor test / RC stream (device_*, #38). Same gate order as ARCHITECTURE §4.2.
  const deviceHandlers: H = {
    device_probe_board: () => {
      legacy("device_probe_board");
      device.probed = true;
      return fcInfo();
    },
    device_motor_test_start: ({ motorIndex, throttle, propsOffConfirmed, limits }) => {
      legacy("device_motor_test_start");
      device.motorLog.push(`start ${motorIndex} ${throttle}`);
      motorFault();
      if (device.rcTimer) throw portBusy("COM5", "APP", "RC stream running");
      if (device.attitudeTimer) throw portBusy("COM5", "APP", "attitude stream running");
      const lim = {
        maxThrottle: Math.min(limits?.maxThrottle ?? DEFAULT_MAX_TEST_THROTTLE, HARD_MAX_TEST_THROTTLE),
        watchdogMs: Math.min(2000, Math.max(100, limits?.watchdogMs ?? DEFAULT_MOTOR_WATCHDOG_MS)),
      };
      if (!propsOffConfirmed) throw err("MOTOR_TEST_REFUSED", "props-off not confirmed");
      if (device.armed) throw err("BOARD_ARMED", "flight controller is armed");
      if (motorIndex !== null && (motorIndex < 0 || motorIndex > 3)) throw err("MOTOR_TEST_REFUSED", `motor index ${motorIndex} >= motor count 4`);
      if (throttle < 1000 || throttle > lim.maxThrottle) throw err("MOTOR_TEST_REFUSED", `throttle ${throttle} outside 1000..${lim.maxThrottle}`);
      device.motorsSpinning.clear();
      if (motorIndex === null) [0, 1, 2, 3].forEach((m) => device.motorsSpinning.add(m));
      else device.motorsSpinning.add(motorIndex);
      return {
        outputConfig: { protocol: "DSHOT600", digital: true, feature3d: false, mincommand: 1000, motorCount: 4, stopValue: 1000 },
        limits: lim,
      };
    },
    device_motor_test_keepalive: () => {
      legacy("device_motor_test_keepalive");
      device.motorLog.push("keepalive");
      motorFault();
      if (device.motorsSpinning.size === 0) throw err("MOTOR_TEST_NOT_ACTIVE", "no motor test running");
    },
    device_motor_test_stop: () => {
      legacy("device_motor_test_stop");
      device.motorLog.push("stop");
      device.motorsSpinning.clear();
      motorFault();
    },
    device_rc_stream_start: ({ rateHz }) => {
      legacy("device_rc_stream_start");
      if (device.motorsSpinning.size > 0) throw portBusy("COM5", "APP", "motor test running");
      if (device.attitudeTimer) throw portBusy("COM5", "APP", "attitude stream running");
      if (device.rcTimer) clearInterval(device.rcTimer);
      const hz = Math.min(50, Math.max(1, rateHz ?? 15));
      let seq = 0;
      device.rcTimer = setInterval(() => {
        seq++;
        // Sticks centred; AUX1 toggles every ~3 s, AUX2 cycles low/mid/high every ~2 s.
        const t = (seq / hz) * 1000;
        const aux1 = Math.floor(t / 3000) % 2 === 0 ? 1000 : 2000;
        const aux2 = [1000, 1500, 2000][Math.floor(t / 2000) % 3]!;
        events.emit("device://rc-channels", { seq, timestampMs: Math.round(t), channels: [1500, 1500, 1000, 1500, aux1, aux2, 1000, 1000] });
      }, 1000 / hz);
    },
    flasher_remove_read_protection: async ({ confirmErase }) => {
      legacy("flasher_remove_read_protection");
      device.unprotectLog.push(confirmErase);
      if (!confirmErase) throw { code: "ERASE_CONFIRMATION_REQUIRED", message: "confirmErase must be true" };
      if (!device.inDfu) throw err("DFU_NOT_FOUND", "no DFU device");
      await stages(["ERASING"]);
      // Mass erase: no firmware left, the chip comes back in the bootloader.
      device.readProtected = false;
      device.inDfu = true;
    },
    device_rc_stream_stop: () => {
      legacy("device_rc_stream_stop");
      if (device.rcTimer) clearInterval(device.rcTimer);
      device.rcTimer = null;
      events.emit("device://rc-stopped", { reason: "REQUESTED", error: null });
    },
  };
  function motorFault(): void {
    const e = device.motorFailNext;
    if (e) {
      device.motorFailNext = null;
      throw e;
    }
  }

  // ---- Device #63 (api/pending.ts adapter): sensors, acc calibration, attitude.
  const autoTilt = (ms: number): Euler => {
    const phase = Math.floor(ms / 2000) % 4;
    return phase === 1 ? { roll: 0, pitch: -30, yaw: 0 } : phase === 3 ? { roll: 30, pitch: 0, yaw: 0 } : { roll: 0, pitch: 0, yaw: 0 };
  };
  const pendingHandlers: Record<PendingCommandName, (args: never) => unknown> = {
    device_sensors: () => {
      legacy("device_sensors");
      // SensorStatus (#63). Extra keys are allowed; the UI must ignore them.
      const hw = (s: { present: boolean; enabled: boolean }, chip: string) => ({ ...s, configuredHardware: s.enabled ? "AUTO" : "NONE", detectedHardware: s.present ? chip : null });
      const s = device.sensors;
      return {
        gyro: { present: s.gyro.present, enabled: true, configuredHardware: null, detectedHardware: "BMI270" },
        acc: hw(s.acc, "BMI270"),
        baro: hw(s.baro, "DPS310"),
        mag: hw(s.mag, "QMC5883"),
        accCalibrated: s.acc.present ? device.accCalibrated : null,
        alignment: { boardRollDeg: device.alignment.roll, boardPitchDeg: device.alignment.pitch, boardYawDeg: device.alignment.yaw, gyroAlignment: "CW270" },
        rangefinder: { present: false, enabled: false },
      };
    },
    device_acc_calibrate: async () => {
      legacy("device_acc_calibrate");
      // Checked first (#68): calibration saves the whole RAM config to EEPROM.
      if (device.unsavedRam)
        throw { ...err("PENDING_UNSAVED_CHANGES", "the flight controller may have unsaved changes (CLI command); reboot the board first"), details: { reason: "CLI_COMMAND" } };
      if (device.armed) throw err("BOARD_ARMED", "flight controller is armed");
      const acc = device.sensors.acc;
      if (!acc.present || !acc.enabled) throw { ...err("ACC_NOT_AVAILABLE", "no usable accelerometer"), details: { present: acc.present, enabled: acc.enabled } };
      await sleep(stepMs * 3);
      if (device.accCalFailuresLeft > 0) {
        device.accCalFailuresLeft--;
        throw { ...err("BOARD_MOVING", "the board is moving (23 deg/s > 8 deg/s); keep it level and still and calibrate again"), details: { maxRateDps: 23, limitDps: 8, during: true } };
      }
      device.accCalibrations++;
      device.accCalibrated = true;
      return { durationMs: stepMs * 3, completion: "CALIBRATING_FLAG", maxGyroRateDps: 0.6 };
    },
    device_get_attitude: () => {
      legacy("device_get_attitude");
      const a = reportedWithAlignment(device.tilt ?? autoTilt(Date.now()), device.mount, device.alignment);
      return { seq: 0, timestampMs: 0, roll: a.roll, pitch: a.pitch === 0 ? 0 : -a.pitch, yaw: a.yaw };
    },
    device_attitude_stream_start: ({ rateHz }: { rateHz: number | null }) => {
      legacy("device_attitude_stream_start");
      if (device.motorsSpinning.size > 0) throw portBusy("COM5", "APP", "motor test running");
      if (device.rcTimer) throw portBusy("COM5", "APP", "RC stream running");
      if (device.attitudeTimer) clearInterval(device.attitudeTimer);
      const hz = Math.min(30, Math.max(1, rateHz ?? 10));
      let seq = 0;
      device.attitudeTimer = setInterval(() => {
        seq++;
        const ms = Math.round((seq / hz) * 1000);
        const a = reportedWithAlignment(device.tilt ?? autoTilt(ms), device.mount, device.alignment);
        // MSP_ATTITUDE signs: pitch + = nose DOWN.
        events.emit("device://attitude", { seq, timestampMs: ms, roll: a.roll, pitch: a.pitch === 0 ? 0 : -a.pitch, yaw: a.yaw });
      }, 1000 / hz);
    },
    device_attitude_stream_stop: () => {
      legacy("device_attitude_stream_stop");
      if (device.attitudeTimer) {
        clearInterval(device.attitudeTimer);
        events.emit("device://attitude-stopped", { reason: "REQUESTED", error: null });
      }
      device.attitudeTimer = null;
    },
  };

  const inner = createMockBackend({ ...handlers, ...deviceHandlers });
  // Motor test / RC stream calls answer faster (the 200 ms keepalive must keep up).
  const fast = (cmd: string) => cmd.startsWith("device_motor_test_") || cmd.startsWith("device_rc_stream_") || cmd.startsWith("device_attitude_stream_");
  const backend: Backend = (async (cmd: CommandName, args: never) => {
    await sleep(fast(cmd) ? latency / 2 : latency);
    const p = pendingHandlers[cmd as unknown as PendingCommandName];
    if (p) return p(args);
    return inner(cmd, args);
  }) as Backend;

  return { backend, events, device };
}

/** Scenarios whose firmware has no MSP DisplayPort: config_plan refuses an HD OSD (osd-hd-plan-error: the board reports "unknown"). */
const NO_DISPLAYPORT: readonly string[] = ["osd-sd-only", "osd-hd-unsupported", "osd-hd-plan-error"];

const TO_SERIAL_FUNCTION: Partial<Record<FunctionName, SerialFunction>> = {
  MSP: "MSP",
  GPS: "GPS",
  RX_SERIAL: "RX_SERIAL",
  ESC_SENSOR: "ESC_SENSOR",
  TBS_SMARTAUDIO: "VTX_SMARTAUDIO",
  IRC_TRAMP: "VTX_TRAMP",
  VTX_MSP: "VTX_MSP",
};

const isV2025 = (fw: string | null | undefined) => !!fw && compareVersions(fw, "2025.12.0") >= 0;

/**
 * OSD element names (osd_items_e without OSD_) with their `osd_<cli>_pos` stem. A subset
 * of Betaflight's list (4.4: 77, 4.5: 80, 2025.12: 82), enough for the demo.
 */
export const MOCK_OSD_CLI: Record<string, string> = {
  RSSI_VALUE: "rssi", MAIN_BATT_VOLTAGE: "vbat", CROSSHAIRS: "crosshairs", ITEM_TIMER_1: "tim_1",
  ITEM_TIMER_2: "tim_2", FLYMODE: "flymode", CRAFT_NAME: "craft_name", THROTTLE_POS: "throttle",
  CURRENT_DRAW: "current", MAH_DRAWN: "mah_drawn", GPS_SPEED: "gps_speed", GPS_SATS: "gps_sats",
  ALTITUDE: "altitude", ROLL_PIDS: "pid_roll", WARNINGS: "warnings", AVG_CELL_VOLTAGE: "avg_cell_voltage",
  DISARMED: "disarmed", HOME_DIR: "home_dir", HOME_DIST: "home_dist", ESC_TMP: "esc_tmp",
  LINK_QUALITY: "link_quality", RSSI_DBM_VALUE: "rssi_dbm", GPS_LAP_TIME_CURRENT: "gps_lap_time_current",
  DEBUG2: "debug2", LIDAR_DIST: "lidar_dist",
};

/**
 * Demo config_firmware_catalog(firmwareVersion) (shared #66 FirmwareCatalog).
 * 2025.12 has no RC_SMOOTHING_INIT_FAIL, adds DEBUG2 / LIDAR_DIST and manages ALTHOLD / POSHOLD.
 * Unknown / empty version = the 4.5 lists.
 */
export function mockFirmwareCatalog(firmwareVersion: string): FirmwareCatalog {
  const v2025 = isV2025(firmwareVersion);
  const v44 = !!firmwareVersion && compareVersions(firmwareVersion, "4.5.0") < 0;
  return {
    beepers: BEEPER_NAMES.filter((n) => !(v2025 && n === "RC_SMOOTHING_INIT_FAIL")),
    beaconEvents: ["RX_LOST", "RX_SET"],
    osdElements: Object.keys(MOCK_OSD_CLI).filter((n) => !(v44 && n === "GPS_LAP_TIME_CURRENT") && (v2025 || (n !== "DEBUG2" && n !== "LIDAR_DIST"))),
    modes: ["ARM", "ANGLE", "HORIZON", ...(v2025 ? ["ALTHOLD", "POSHOLD"] : []), "BEEPER", "AIR MODE", "FLIP OVER AFTER CRASH", "PREARM", "GPS RESCUE"],
  };
}

/** Rough stand-in for the config engine (generate / plan): enough for the Review screen. */
/**
 * Plausible default layouts (stand-in for config_osd_default_layout): PAL 30x16,
 * NTSC / AUTO 30x13 (AUTO fits either camera), HD 53x20. Includes an element the UI does
 * not offer (ROLL_PIDS) and only elements of that firmware line.
 */
export function mockOsdLayout(videoSystem: OsdVideoSystem, firmwareVersion = ""): OsdLayout {
  const [cols, rows] = videoSystem === "HD" ? [53, 20] : videoSystem === "PAL" ? [30, 16] : [30, 13];
  const c = Math.floor(cols / 2);
  const m = Math.floor(rows / 2);
  const e = (name: string, x: number, y: number, enabled: boolean) => ({ name, x, y, enabled });
  const known = new Set(mockFirmwareCatalog(firmwareVersion).osdElements);
  return {
    videoSystem,
    cols,
    rows,
    elements: [
      e("LINK_QUALITY", 1, 1, true),
      e("RSSI_VALUE", 1, 2, false),
      e("RSSI_DBM_VALUE", 1, 3, false),
      e("ITEM_TIMER_2", cols - 7, 1, true),
      e("GPS_SATS", cols - 8, 2, true),
      e("FLYMODE", cols - 6, 3, false),
      e("ITEM_TIMER_1", cols - 7, 4, false),
      e("GPS_SPEED", cols - 9, 5, false),
      e("HOME_DIR", c, 1, true),
      e("HOME_DIST", c - 2, 2, true),
      e("CROSSHAIRS", c - 1, m, false),
      e("DISARMED", c - 4, m + 2, false),
      e("ALTITUDE", 1, m, false),
      e("WARNINGS", c - 6, rows - 5, true),
      e("ESC_TMP", 1, rows - 4, false),
      e("MAIN_BATT_VOLTAGE", 1, rows - 3, false),
      e("AVG_CELL_VOLTAGE", 1, rows - 2, true),
      e("CURRENT_DRAW", cols - 8, rows - 4, false),
      e("THROTTLE_POS", cols - 8, rows - 3, false),
      e("MAH_DRAWN", cols - 8, rows - 2, true),
      e("CRAFT_NAME", c - 4, rows - 1, true),
      e("ROLL_PIDS", 2, 6, false),
      e("DEBUG2", 2, 7, false),
    ].filter((x) => known.has(x.name)),
  };
}

/** BF 4.4+ `osd_*_pos` value: x (bits 0-4 + bit 10), y (bits 5-9), `enabled` = visible in all 3 OSD profiles (bits 11-13). */
export function osdPosValue(x: number, y: number, visible: boolean): number {
  return (x & 0x1f) | (((x >> 5) & 1) << 10) | ((y & 0x1f) << 5) | (visible ? 0x7 << 11 : 0);
}

export function generateCli(model: ExtendedDroneModel, ports: FcSerialPort[]): CliScript {
  const opts = { escTelemetry: model.esc.telemetryPort !== null };
  const commands: string[] = [];
  const warnings: string[] = [];
  const cleared: ClearedPort[] = plannedClearings(ports, model, opts).map((c) => {
    const p = ports.find((x) => x.name === c.port)!;
    return {
      port: c.port,
      functions: c.functions.map((f) => TO_SERIAL_FUNCTION[f]).filter((f): f is SerialFunction => !!f),
      functionMaskBefore: p.functionMask,
      functionMaskAfter: targetMask(p, model, opts),
    };
  });
  for (const p of ports) {
    if (isReserved(p)) continue;
    const mask = targetMask(p, model, opts);
    if (mask !== p.functionMask) commands.push(`serial ${p.identifier} ${mask} 115200 57600 0 115200`);
  }
  if (model.receiver.kind === "SERIAL" && model.receiver.protocol) {
    commands.push("feature -RX_SPI", "feature RX_SERIAL", `set serialrx_provider = ${model.receiver.protocol}`);
    commands.push(`set serialrx_inverted = ${model.receiver.inverted ? "ON" : "OFF"}`, `set serialrx_halfduplex = ${model.receiver.halfDuplex ? "ON" : "OFF"}`);
  }
  if (model.receiver.kind === "SPI" && model.receiver.protocol) commands.push("feature -RX_SERIAL", "feature RX_SPI", `set rx_spi_protocol = ${model.receiver.protocol}`);
  if (model.receiver.kind === "SPI" && model.receiver.elrsUid?.length === 6) commands.push(`set expresslrs_uid = ${model.receiver.elrsUid.join(",")}`);
  const p = model.receiver.protocol;
  if (model.receiver.telemetry && (p === "CRSF" || p === "FPORT" || (model.receiver.kind === "SPI" && p === "EXPRESSLRS"))) commands.push("feature TELEMETRY");
  if (model.gps.enabled) commands.push("feature GPS", `set gps_provider = ${model.gps.provider ?? "UBLOX"}`);
  if (model.vtx.kind === "DIGITAL" && !model.osd) commands.push("set osd_displayport_device = MSP", "set vcd_video_system = HD");
  commands.push(`set motor_pwm_protocol = ${model.esc.protocol}`, `set dshot_bidir = ${model.esc.bidirectionalDshot ? "ON" : "OFF"}`);
  if (model.esc.motorPoles) commands.push(`set motor_poles = ${model.esc.motorPoles}`);
  commands.push(`set yaw_motors_reversed = ${model.esc.propsOut ? "ON" : "OFF"}`);
  if (model.esc.telemetryPort) commands.push("feature ESC_SENSOR");
  // Like config::modes: stable names only, mapped to permanent box ids here (backend side).
  const boxIds: Record<string, number> = { ARM: 0, ANGLE: 1, HORIZON: 2, ALTHOLD: 3, POSHOLD: 11, BEEPER: 13, AIRMODE: 28, FLIPOVERAFTERCRASH: 35, PREARM: 36, GPSRESCUE: 46 };
  model.switches.forEach((s, i) => {
    const id = boxIds[s.mode.toUpperCase().replace(/[^A-Z0-9]/g, "")];
    if (id === undefined) throw { code: "CONFIG", message: `unknown or unmanaged flight mode '${s.mode}'` };
    commands.push(`aux ${i} ${id} ${s.auxChannel} ${s.rangeMin} ${s.rangeMax} 0 0`);
  });
  // #65 fields (src/types/model.ts); absent / null = keep the board's value.
  const fw = model.fc.firmwareVersion || "";
  (model.removeModes ?? []).forEach((m, i) => {
    // The demo FC has no ranges for these: reset a spare slot like the engine would (no-op otherwise).
    const id = boxIds[m.replace(/[^A-Z0-9]/g, "")];
    if (id !== undefined) commands.push(`aux ${model.switches.length + i} 0 0 900 900 0 0`);
  });
  if (model.airmode) commands.push(model.airmode === "ALWAYS" ? "feature AIRMODE" : "feature -AIRMODE");
  for (const k of ["acc", "baro", "mag"] as const) {
    const v = model.sensors?.[k];
    if (typeof v === "boolean") commands.push(`set ${k}_hardware = ${v ? "AUTO" : "NONE"}`);
  }
  if (model.boardAlignment) for (const k of ["roll", "pitch", "yaw"] as const) commands.push(`set align_board_${k} = ${model.boardAlignment[k]}`);
  if (model.beepers) {
    const off = new Set<string>(model.beepers.disabled);
    for (const n of mockFirmwareCatalog(fw).beepers) {
      const cli = isV2025(fw) && n === "CRASH_FLIP" ? "CRASHFLIP" : n;
      commands.push(off.has(n) ? `beeper -${cli}` : `beeper ${cli}`);
    }
    if (model.beepers.dshotBeaconDisabled)
      for (const e of ["RX_LOST", "RX_SET"]) commands.push(model.beepers.dshotBeaconDisabled.includes(e as never) ? `beacon -${e}` : `beacon ${e}`);
  }
  // #65 `osd` field (wizard/osd.ts); absent = keep the board's OSD.
  if (model.osd) {
    const vs = model.osd.videoSystem ?? "AUTO";
    commands.push("feature OSD");
    if (model.osd.videoSystem) commands.push(`set vcd_video_system = ${vs}`);
    if (vs === "HD") commands.push("set osd_displayport_device = MSP", "set osd_canvas_width = 53", "set osd_canvas_height = 20");
    else if (model.osd.videoSystem) commands.push("set osd_displayport_device = AUTO");
    const layout = mockOsdLayout(vs, fw);
    for (const el of model.osd.elements) {
      const d = layout.elements.find((l) => l.name === el.name);
      if (!d) throw err("CONFIG_UNSUPPORTED", `OSD element ${el.name} is not in this firmware`);
      const x = el.x ?? d.x;
      const y = el.y ?? d.y;
      if (x < 0 || y < 0 || x >= layout.cols || y >= layout.rows) throw err("CONFIG_UNSUPPORTED", `osd ${el.name} outside the grid`);
      commands.push(`set osd_${MOCK_OSD_CLI[el.name]}_pos = ${osdPosValue(x, y, el.enabled)}`);
    }
  }
  commands.push("save");
  return { commands, warnings, dialect: "4.5", cleared };
}
