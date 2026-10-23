// SPDX-License-Identifier: GPL-3.0-or-later
import { afterEach, describe, expect, it } from "vitest";
import { api } from "../src/api/commands";
import { onEvent } from "../src/api/events";
import { normalizeVerify, toNotImplemented, upcoming, type ApplyProgress } from "../src/api/upcoming";
import { installMock, uninstallMock } from "../src/mocks/install";
import { emptyDroneModel, type VerifyReport } from "../src/types/model";

afterEach(() => uninstallMock());

describe("upcoming contract wrappers (UI-side, TODO(shared))", () => {
  it("maps Tauri's unknown-command string to NOT_IMPLEMENTED", () => {
    expect(toNotImplemented("motor_test_start", "Command motor_test_start not found")).toMatchObject({ code: "NOT_IMPLEMENTED" });
    const appErr = { code: "SERIAL", message: "x" };
    expect(toNotImplemented("motor_test_start", appErr)).toEqual({ code: "SERIAL_ERROR", category: "Connection", message: "x" });
  });

  it("normalises verify mismatches from both the agreed {key} and current {setting} shapes", () => {
    const agreed = { ok: false, mismatches: [{ key: "gps_provider", expected: "UBLOX", actual: "NMEA" }] } as unknown as VerifyReport;
    const current: VerifyReport = { ok: false, mismatches: [{ setting: "motor_pwm_protocol", expected: "DSHOT600", actual: "DSHOT300" }] };
    expect(normalizeVerify(agreed).mismatches[0]).toEqual({ key: "gps_provider", expected: "UBLOX", actual: "NMEA" });
    expect(normalizeVerify(current).mismatches[0]!.key).toBe("motor_pwm_protocol");
    expect(normalizeVerify({ ok: true, mismatches: [] }).ok).toBe(true);
  });

  it("motor test and RC stream work against the mock; legacy backend reports NOT_IMPLEMENTED", async () => {
    const mock = installMock({ latencyMs: 0 });
    await expect(upcoming.motorTestStart({ port: "COM5", motorIndex: 2, throttle: 1100, propsOffConfirmed: false })).rejects.toMatchObject({
      code: "MOTOR_TEST_REFUSED",
      category: "Safety",
    });
    await expect(upcoming.motorTestStart({ port: "COM5", motorIndex: 2, throttle: 1200, propsOffConfirmed: true })).rejects.toMatchObject({
      code: "MOTOR_TEST_REFUSED",
    }); // over the 1150 cap: refused, not clamped
    const r = await upcoming.motorTestStart({ port: "COM5", motorIndex: 2, throttle: 1100, propsOffConfirmed: true });
    expect(r.limits).toEqual({ maxThrottle: 1150, watchdog: 500 });
    expect(mock.device.motorsSpinning.has(2)).toBe(true);
    await upcoming.motorTestKeepalive();
    await upcoming.motorTestStop();
    expect(mock.device.motorsSpinning.size).toBe(0);
    await expect(upcoming.motorTestKeepalive()).rejects.toMatchObject({ code: "MOTOR_TEST_NOT_ACTIVE" });
    uninstallMock();
    installMock({ latencyMs: 0, scenario: "legacy-backend" });
    await expect(upcoming.motorTestStop()).rejects.toMatchObject({ code: "NOT_IMPLEMENTED" });
  });

  it("config_apply emits the agreed stages in order before resolving", async () => {
    installMock({ latencyMs: 0 });
    const seen: ApplyProgress["stage"][] = [];
    await onEvent("config://apply-progress", (p) => seen.push(p.stage));
    const ports = await api.config.readPorts();
    const script = await api.config.generate(emptyDroneModel(), ports);
    const res = await api.config.apply(script);
    expect(res.saved).toBe(true);
    expect(seen).toEqual(["BACKUP", "SENDING", "SAVING", "REBOOTING", "RECONNECTED"]);
  });

  it("scenario mocks raise the agreed detailed error codes", async () => {
    installMock({ latencyMs: 0, scenario: "port-busy" });
    await expect(api.device.connect("COM5")).rejects.toMatchObject({ code: "PORT_BUSY" });
    uninstallMock();
    const m = installMock({ latencyMs: 0, scenario: "flash-fail" });
    m.device.inDfu = true;
    const image = await api.flasher.fetchFirmware({ target: "SPEEDYBEEF405V4", release: "4.5.2", options: [], offlineOnly: false });
    await expect(api.flasher.flash(image, null, { fullErase: false, manifest: true })).rejects.toMatchObject({ code: "FLASH_FAILED" });
  });
});

describe("firmware update orchestration", () => {
  const poll = { tries: 3, intervalMs: 0 };
  const fc = { fcVariant: "BTFL", firmwareVersion: "4.5.1", apiVersion: "1.46", boardName: "SPEEDYBEEF405V4", manufacturerId: null, mcu: null, uid: null, buildOptions: [] };

  it("runs fetch, backup, bootloader, flash, reconnect and ends on the new version", async () => {
    const { runFirmwareUpdate } = await import("../src/wizard/flash");
    installMock({ latencyMs: 0 });
    const stages: string[] = [];
    const flash: string[] = [];
    const res = await runFirmwareUpdate(fc, "4.5.2", { onStage: (s) => stages.push(s), onProgress: (p) => flash.push(p.stage) }, poll);
    expect(stages).toEqual(["download", "backup", "bootloader", "flash", "reconnect"]);
    expect(flash.at(-1)).toBe("DONE");
    expect(res.fc.firmwareVersion).toBe("4.5.2");
    expect(res.backup).not.toBeNull();
  });

  it("stops with DRIVER_MISSING, then resumes from DFU after the driver is installed", async () => {
    const { runFirmwareUpdate } = await import("../src/wizard/flash");
    installMock({ latencyMs: 0, scenario: "driver-missing" });
    const hooks = { onStage: () => {}, onProgress: () => {} };
    await expect(runFirmwareUpdate(fc, "4.5.2", hooks, poll)).rejects.toMatchObject({ code: "DRIVER_MISSING" });
    await api.system.installDfuDriver();
    const res = await runFirmwareUpdate(fc, "4.5.2", hooks, poll);
    expect(res.fc.firmwareVersion).toBe("4.5.2");
  });
});

describe("flash verify mismatch (DESIGN.md §6)", () => {
  it("raises VERIFY_MISMATCH (category Flash) and succeeds when retried with full erase", async () => {
    const { runFirmwareUpdate } = await import("../src/wizard/flash");
    installMock({ latencyMs: 0, stepMs: 0, scenario: "flash-verify" });
    await api.device.connect("COM5");
    const fc = await api.device.identify();
    const hooks = { onStage: () => undefined, onProgress: () => undefined };
    const poll = { tries: 3, intervalMs: 0 };
    await expect(runFirmwareUpdate(fc, "4.5.2", hooks, poll)).rejects.toMatchObject({ code: "VERIFY_MISMATCH", category: "Flash" });
    const r = await runFirmwareUpdate(fc, "4.5.2", hooks, poll, { fullErase: true });
    expect(r.fc.firmwareVersion).toBe("4.5.2");
  });
});

describe("review plan, backups and rollback", () => {
  it("uses config_plan when present, else falls back to generate + UI clearings", async () => {
    const { fetchPlan } = await import("../src/wizard/plan");
    const m = emptyDroneModel(); // nothing selected: UART2 (RX) and UART3 (VTX) get cleared
    installMock({ latencyMs: 0 });
    const ports = await api.config.readPorts();
    const a = await fetchPlan(m, ports, { escTelemetry: false });
    expect(a.source).toBe("backend");
    expect(a.plan.clearings).toEqual([
      { port: "UART2", roles: ["RX"] },
      { port: "UART3", roles: ["VTX"] },
    ]);
    uninstallMock();
    installMock({ latencyMs: 0, scenario: "legacy-backend" });
    const b = await fetchPlan(m, ports, { escTelemetry: false });
    expect(b.source).toBe("wizard");
    expect(b.plan.clearings).toEqual(a.plan.clearings);
    expect(b.plan.commands).toEqual(a.plan.commands);
  });

  it("lists backups (backend or session fallback) and restores one with config_rollback", async () => {
    const { listBackups, restoreBackup, canRestoreOnto } = await import("../src/wizard/backups");
    const mock = installMock({ latencyMs: 0 });
    const before = await api.config.backup();
    mock.device.ports.find((p) => p.name === "UART2")!.functionMask = 0;
    const list = await listBackups([]);
    expect(list.source).toBe("backend");
    expect(list.backups[0]).toEqual(before);
    expect(canRestoreOnto(before, "SPEEDYBEEF405V4")).toBe(true);
    expect(canRestoreOnto(before, "MATEKH743")).toBe(false);
    const res = await restoreBackup(before);
    expect(res.ports.find((p) => p.name === "UART2")!.functionMask).not.toBe(0);
    uninstallMock();
    installMock({ latencyMs: 0, scenario: "legacy-backend" });
    expect((await listBackups([before])).source).toBe("session");
    uninstallMock();
    installMock({ latencyMs: 0, scenario: "rollback-fail" });
    await expect(restoreBackup(before)).rejects.toMatchObject({ code: "CLI_ERROR", category: "Protocol" });
  });
});
