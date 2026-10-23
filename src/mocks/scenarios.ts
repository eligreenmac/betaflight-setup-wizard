// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Scenario mock of the whole backend for browser demo mode and UI tests.
 * Simulates a SpeedyBee F405 V4 on Betaflight 4.5.1 (latest stable 4.5.2) with:
 *   UART2 = serial RX, UART3 = MSP + VTX_MSP (digital VTX), UART6 = plain MSP (foreign).
 *
 * Covers the shared contract (src/api/commands.ts) via createMockBackend() and the
 * agreed-but-not-yet-shared commands/events (src/api/upcoming.ts).
 *
 * Pick a scenario with `?mock=<name>` or VITE_MOCK_SCENARIO.
 */
import type { Backend, CommandMap, CommandName } from "../api/commands";
import { createMockBackend } from "../api/mock";
import { createEmitter, type Emitter } from "../api/events";
import type { UpcomingBackend, UpcomingCommand, UpcomingCommandMap } from "../api/upcoming";
import type { Backup, DroneModel, FcSerialPort, FlashStage } from "../types/model";
import { FN, isReserved, plannedClearings, targetMask } from "../wizard/ports";
import { ROLE_TO_PLAN } from "../wizard/plan";
import { uiError, type ErrorCode, type UiError } from "../errors/codes";
import { DEFAULT_MAX_TEST_THROTTLE, DEFAULT_MOTOR_WATCHDOG_MS, HARD_MAX_TEST_THROTTLE } from "../api/upcoming";

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
  upcoming: UpcomingBackend;
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
  /** Force the next motor_test_* call to fail with this error (tests). */
  motorFailNext: UiError | null;
  armed: boolean;
  rcTimer: ReturnType<typeof setInterval> | null;
  backups: { backup: Backup; ports: FcSerialPort[] }[];
}

/** Errors in the DESIGN.md §6 UiError shape. NOT_IMPLEMENTED mimics the Phase 0 stubs. */
const err = (code: ErrorCode, message: string): UiError => uiError(code, message);
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
    backups: [],
  };
  const snapshot = (): Backup => {
    const createdAt = new Date(Date.UTC(2026, 9, 23, 4, 0, device.backups.length)).toISOString();
    const backup: Backup = {
      path: `C:\\Users\\pilot\\AppData\\Roaming\\io.github.eligreenmac.bfsw\\backups\\${UID}\\${createdAt.replace(/[:.]/g, "-")}.txt`,
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
    system_driver_status: () => ({ dfuWinusb: device.dfuDriverOk, vcp: true, notes: [] }),
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
      if (scenario === "port-busy") throw err("PORT_BUSY", `${path}: Access is denied. (os error 5)`);
      if (path !== "COM5") throw err("SERIAL_ERROR", `${path}: no MSP response`);
    },
    device_disconnect: () => undefined,
    device_identify: () => ({
      fcVariant: "BTFL",
      firmwareVersion: device.firmwareVersion,
      apiVersion: device.firmwareVersion.startsWith("4.3") ? "1.44" : "1.46",
      boardName: "SPEEDYBEEF405V4",
      manufacturerId: "SPBE",
      mcu: "STM32F405",
      uid: UID,
      buildOptions: ["CRSF", "SBUS", "GPS", "VTX", "DSHOT_BITBANG", ...(scenario === "onboard-elrs" ? ["USE_RX_EXPRESSLRS"] : [])],
    }),
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
      for (const stage of ["BACKUP", "SENDING", "SAVING", "REBOOTING", "RECONNECTED"] as const) {
        if (stage === "BACKUP") snapshot();
        events.emit("config://apply-progress", { stage, fraction: 0, message: null });
        await sleep(stepMs);
      }
      // Reflect the serial changes on the "board" so a re-read shows them.
      for (const line of script.commands) {
        const m = /^serial (\d+) (\d+)/.exec(line);
        const port = m ? device.ports.find((p) => p.identifier === Number(m[1])) : undefined;
        if (port && m) port.functionMask = Number(m[2]);
      }
      return { applied: script.commands.length, errors: [], saved: true };
    },
    config_verify: () => {
      if (device.verifyFailuresLeft > 0) {
        device.verifyFailuresLeft--;
        // Agreed shape uses `key` (model.ts still says `setting`); see normalizeVerify().
        return { ok: false, mismatches: [{ key: "serialrx_provider", expected: "CRSF", actual: "SBUS" }] } as unknown as CommandMap["config_verify"][1];
      }
      return { ok: true, mismatches: [] };
    },
    config_rollback: async ({ backup }) => {
      await sleep(stepMs * 3);
      if (scenario === "rollback-fail") throw err("CLI_ERROR", "###ERROR### in line 12: serial 2 131073 115200 57600 0 115200");
      const hit = device.backups.find((b) => b.backup.path === backup.path);
      if (!hit) throw err("IO_ERROR", `backup file not found: ${backup.path}`);
      device.ports = hit.ports.map((p) => ({ ...p }));
      return { applied: 42, errors: [], saved: true };
    },
  };

  async function stages(list: FlashStage[]) {
    for (const stage of list) {
      for (const fraction of [0, 0.5, 1]) {
        events.emit("flasher://progress", { stage, fraction, message: null });
        await sleep(stepMs / 3);
      }
    }
  }

  const inner = createMockBackend(handlers);
  const backend: Backend = (async (cmd: CommandName, args: never) => {
    await sleep(latency);
    return inner(cmd, args);
  }) as Backend;

  const upcomingHandlers: { [K in UpcomingCommand]: (a: UpcomingCommandMap[K][0]) => UpcomingCommandMap[K][1] } = {
    config_plan: ({ model, ports }) => {
      const opts = { escTelemetry: model.esc.telemetryPort !== null };
      const clearings = plannedClearings(ports, model, opts).map((c) => ({ port: c.port, roles: c.roles.map((r) => ROLE_TO_PLAN[r]) }));
      return { ...generateCli(model, ports), clearings };
    },
    config_list_backups: () => device.backups.map((b) => b.backup),
    motor_test_start: ({ motorIndex, throttle, propsOffConfirmed, limits }) => {
      device.motorLog.push(`start ${motorIndex} ${throttle}`);
      motorFault();
      const lim = { maxThrottle: Math.min(limits?.maxThrottle ?? DEFAULT_MAX_TEST_THROTTLE, HARD_MAX_TEST_THROTTLE), watchdog: limits?.watchdog ?? DEFAULT_MOTOR_WATCHDOG_MS };
      // Same gate order as device::motor::check_gate (DESIGN.md §5).
      if (!propsOffConfirmed) throw err("MOTOR_TEST_REFUSED", "props-off not confirmed");
      if (device.armed) throw err("BOARD_ARMED", "flight controller is armed");
      if (motorIndex !== null && (motorIndex < 0 || motorIndex > 3)) throw err("MOTOR_TEST_REFUSED", `motor index ${motorIndex} >= motor count 4`);
      if (throttle < 1000 || throttle > lim.maxThrottle) throw err("MOTOR_TEST_REFUSED", `throttle ${throttle} outside 1000..${lim.maxThrottle}`);
      device.motorsSpinning.clear();
      if (motorIndex === null) [0, 1, 2, 3].forEach((m) => device.motorsSpinning.add(m));
      else device.motorsSpinning.add(motorIndex);
      return { outputConfig: { motorCount: 4, stopValue: 1000, protocol: "DSHOT600" }, limits: lim };
    },
    motor_test_keepalive: () => {
      device.motorLog.push("keepalive");
      motorFault();
      if (device.motorsSpinning.size === 0) throw err("MOTOR_TEST_NOT_ACTIVE", "no motor test running");
    },
    motor_test_stop: () => {
      device.motorLog.push("stop");
      device.motorsSpinning.clear();
      motorFault();
    },
    rc_channels_stream_start: ({ rateHz }) => {
      if (device.rcTimer) clearInterval(device.rcTimer);
      const hz = Math.min(50, Math.max(1, rateHz ?? 15));
      let seq = 0;
      device.rcTimer = setInterval(() => {
        seq++;
        // Sticks centred; AUX1 toggles every ~3 s, AUX2 cycles low/mid/high every ~2 s.
        const t = (seq / hz) * 1000;
        const aux1 = Math.floor(t / 3000) % 2 === 0 ? 1000 : 2000;
        const aux2 = [1000, 1500, 2000][Math.floor(t / 2000) % 3]!;
        events.emit("rc://channels", { seq, timestampMs: Math.round(t), channels: [1500, 1500, 1000, 1500, aux1, aux2, 1000, 1000] });
      }, 1000 / hz);
    },
    rc_channels_stream_stop: () => {
      if (device.rcTimer) clearInterval(device.rcTimer);
      device.rcTimer = null;
      events.emit("rc://stopped", { reason: "requested", error: null });
    },
  };
  function motorFault(): void {
    const e = device.motorFailNext;
    if (e) {
      device.motorFailNext = null;
      throw e;
    }
  }
  const upcoming: UpcomingBackend = (async (cmd: UpcomingCommand, args: never) => {
    await sleep(latency / 2);
    if (scenario === "legacy-backend") throw err("NOT_IMPLEMENTED", `mock: ${cmd}`);
    return (upcomingHandlers[cmd] as (a: unknown) => unknown)(args);
  }) as UpcomingBackend;

  return { backend, upcoming, events, device };
}

/** Rough stand-in for the config engine's generate(): enough for the Review screen. */
export function generateCli(model: DroneModel, ports: FcSerialPort[]) {
  const opts = { escTelemetry: model.esc.telemetryPort !== null };
  const commands: string[] = [];
  const warnings: string[] = [];
  for (const c of plannedClearings(ports, model, opts)) warnings.push(`clearing ${c.functions.join("+")} from ${c.port}`);
  for (const p of ports) {
    if (isReserved(p)) continue;
    const mask = targetMask(p, model, opts);
    if (mask !== p.functionMask) commands.push(`serial ${p.identifier} ${mask} 115200 57600 0 115200`);
  }
  if (model.receiver.kind === "SERIAL" && model.receiver.protocol) {
    commands.push("feature RX_SERIAL", `set serialrx_provider = ${model.receiver.protocol}`);
    commands.push(`set serialrx_inverted = ${model.receiver.inverted ? "ON" : "OFF"}`, `set serialrx_halfduplex = ${model.receiver.halfDuplex ? "ON" : "OFF"}`);
  }
  if (model.receiver.kind === "SPI" && model.receiver.protocol) commands.push("feature RX_SPI", `set rx_spi_protocol = ${model.receiver.protocol}`);
  if (model.gps.enabled) commands.push("feature GPS", `set gps_provider = ${model.gps.provider ?? "UBLOX"}`);
  if (model.vtx.kind === "DIGITAL") commands.push("set osd_displayport_device = MSP", "set vcd_video_system = HD");
  commands.push(`set motor_pwm_protocol = ${model.esc.protocol}`, `set dshot_bidir = ${model.esc.bidirectionalDshot ? "ON" : "OFF"}`);
  if (model.esc.motorPoles) commands.push(`set motor_poles = ${model.esc.motorPoles}`);
  commands.push(`set yaw_motors_reversed = ${model.esc.propsOut ? "ON" : "OFF"}`);
  if (model.esc.telemetryPort) commands.push("feature ESC_SENSOR");
  const boxIds: Record<string, number> = { ARM: 0, ANGLE: 1, BARO: 3, ALTHOLD: 3, BEEPER: 13, "GPS HOLD": 11, "POS HOLD": 11, "FLIP OVER AFTER CRASH": 35 };
  model.switches.forEach((s, i) => commands.push(`aux ${i} ${boxIds[s.mode] ?? 0} ${s.auxChannel} ${s.rangeMin} ${s.rangeMax} 0 0`));
  commands.push("save");
  return { commands, warnings, dialect: "4.5" };
}
