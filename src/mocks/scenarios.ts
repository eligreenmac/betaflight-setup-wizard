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
 * Pick a scenario with `?mock=<name>` or VITE_MOCK_SCENARIO.
 */
import type { Backend, CommandMap, CommandName } from "../api/commands";
import { createMockBackend } from "../api/mock";
import { createEmitter, type Emitter } from "../api/events";
import type { DeviceBackend, DeviceCommand, DeviceCommandMap, DeviceFcInfo } from "../api/device";
import type { Backup, ClearedPort, CliScript, DroneModel, FcSerialPort, FlashStage, SerialFunction } from "../types/model";
import { FN, isReserved, plannedClearings, targetMask, type FunctionName } from "../wizard/ports";
import { uiError, type ErrorCode, type UiError } from "../errors/codes";
import { DEFAULT_MAX_TEST_THROTTLE, DEFAULT_MOTOR_WATCHDOG_MS, HARD_MAX_TEST_THROTTLE } from "../wizard/motorTest";

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
  /** The #38 device_* commands (src/api/device.ts) until they are in the shared CommandMap. */
  deviceBackend: DeviceBackend;
  events: Emitter;
  /** Inspectable device state (tests). */
  device: MockDevice;
}

interface MockDevice {
  firmwareVersion: string;
  ports: FcSerialPort[];
  inDfu: boolean;
  dfuDriverOk: boolean;
  flashFailuresLeft: number;
  verifyFailuresLeft: number;
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
    dfuDriverOk: scenario !== "driver-missing",
    flashFailuresLeft: scenario === "flash-fail" ? 1 : 0,
    verifyFailuresLeft: scenario === "verify-mismatch" ? 1 : 0,
    motorsSpinning: new Set(),
    motorLog: [],
    motorFailNext: null,
    armed: false,
    rcTimer: null,
    probed: false,
    backups: [],
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
              service: device.dfuDriverOk ? "WinUSB" : null,
              driverOk: device.dfuDriverOk,
              action: device.dfuDriverOk ? "NONE" : "INSTALL_DFU_DRIVER",
              guidanceIndex: null,
            },
          ]
        : [],
      guidance: [],
    }),
    system_install_dfu_driver: async () => {
      await sleep(latency * 2);
      device.dfuDriverOk = true;
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
    },
    device_disconnect: () => undefined,
    device_identify: () => fcInfo(),
    device_cli_exec: ({ line }) => `# ${line}\n`,
    device_reboot_to_bootloader: () => {
      device.inDfu = true;
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

  function fcInfo(): DeviceFcInfo {
    const elrs = scenario === "onboard-elrs";
    return {
      fcVariant: "BTFL",
      firmwareVersion: device.firmwareVersion,
      apiVersion: device.firmwareVersion.startsWith("4.3") ? "1.44" : "1.46",
      boardName: "SPEEDYBEEF405V4",
      manufacturerId: "SPBE",
      mcu: "STM32F405",
      uid: UID,
      buildOptions: ["CRSF", "SBUS", "GPS", "VTX", "DSHOT_BITBANG", ...(elrs ? ["USE_RX_EXPRESSLRS"] : [])],
      serialPorts: device.ports.map((p) => ({ ...p })),
      // present stays null until device_probe_board ran (ARCHITECTURE §4.1).
      rxSpi: device.probed
        ? { present: elrs, expresslrsPins: elrs, protocol: elrs ? "EXPRESSLRS" : null, featureEnabled: elrs, resources: elrs ? ["RX_SPI_CS", "RX_SPI_EXTI", "RX_SPI_BIND", "RX_SPI_LED"] : [] }
        : { present: null, expresslrsPins: elrs, protocol: null, featureEnabled: null, resources: [] },
    };
  }

  // ---- motor test / RC stream (device_*, #38). Same gate order as ARCHITECTURE §4.2.
  const deviceHandlers: { [K in DeviceCommand]: (a: DeviceCommandMap[K][0]) => DeviceCommandMap[K][1] } = {
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

  const inner = createMockBackend(handlers);
  const backend: Backend = (async (cmd: CommandName, args: never) => {
    await sleep(latency);
    return inner(cmd, args);
  }) as Backend;

  const deviceBackend: DeviceBackend = (async (cmd: DeviceCommand, args: never) => {
    await sleep(latency / 2);
    return (deviceHandlers[cmd] as (a: unknown) => unknown)(args);
  }) as DeviceBackend;

  return { backend, deviceBackend, events, device };
}

const TO_SERIAL_FUNCTION: Partial<Record<FunctionName, SerialFunction>> = {
  MSP: "MSP",
  GPS: "GPS",
  RX_SERIAL: "RX_SERIAL",
  ESC_SENSOR: "ESC_SENSOR",
  TBS_SMARTAUDIO: "VTX_SMARTAUDIO",
  IRC_TRAMP: "VTX_TRAMP",
  VTX_MSP: "VTX_MSP",
};

/** Rough stand-in for the config engine (generate / plan): enough for the Review screen. */
export function generateCli(model: DroneModel, ports: FcSerialPort[]): CliScript {
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
  if (model.vtx.kind === "DIGITAL") commands.push("set osd_displayport_device = MSP", "set vcd_video_system = HD");
  commands.push(`set motor_pwm_protocol = ${model.esc.protocol}`, `set dshot_bidir = ${model.esc.bidirectionalDshot ? "ON" : "OFF"}`);
  if (model.esc.motorPoles) commands.push(`set motor_poles = ${model.esc.motorPoles}`);
  commands.push(`set yaw_motors_reversed = ${model.esc.propsOut ? "ON" : "OFF"}`);
  if (model.esc.telemetryPort) commands.push("feature ESC_SENSOR");
  // Like config::modes: stable names only, mapped to permanent box ids here (backend side).
  const boxIds: Record<string, number> = { ARM: 0, ANGLE: 1, ALTHOLD: 3, POSHOLD: 11, BEEPER: 13, "FLIP OVER AFTER CRASH": 35 };
  model.switches.forEach((s, i) => {
    const id = boxIds[s.mode];
    if (id === undefined) throw { code: "CONFIG", message: `unknown or unmanaged flight mode '${s.mode}'` };
    commands.push(`aux ${i} ${id} ${s.auxChannel} ${s.rangeMin} ${s.rangeMax} 0 0`);
  });
  commands.push("save");
  return { commands, warnings, dialect: "4.5", cleared };
}
