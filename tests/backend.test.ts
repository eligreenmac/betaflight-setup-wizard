// SPDX-License-Identifier: GPL-3.0-or-later
import { afterEach, describe, expect, it } from "vitest";
import { api, EVENTS } from "../src/api/commands";
import { onEvent } from "../src/api/events";
import { toUiError } from "../src/errors/codes";
import { installMock, uninstallMock } from "../src/mocks/install";
import { emptyDroneModel, type ConfigStage } from "../src/types/model";

afterEach(() => uninstallMock());

describe("device contract (#38) against the scenario mock", () => {
  it("maps Tauri's unknown-command string to NOT_IMPLEMENTED and keeps AppError details", () => {
    expect(toUiError("Command device_motor_test_start not found")).toMatchObject({ code: "NOT_IMPLEMENTED" });
    expect(toUiError({ code: "SERIAL", message: "x" })).toEqual({ code: "SERIAL_ERROR", category: "Connection", message: "x" });
    expect(toUiError({ code: "PORT_BUSY", message: "x", details: { port: "COM5", heldBy: "APP" } })).toMatchObject({ details: { heldBy: "APP" } });
    expect(toUiError({ code: "ERASE_CONFIRMATION_REQUIRED", message: "x" }).code).toBe("INTERNAL");
  });

  it("motor test and RC stream work against the mock; legacy backend reports NOT_IMPLEMENTED", async () => {
    const mock = installMock({ latencyMs: 0 });
    await expect(api.device.motorTestStart(2, 1100, false)).rejects.toMatchObject({ code: "MOTOR_TEST_REFUSED", category: "Safety" });
    await expect(api.device.motorTestStart(2, 1200, true)).rejects.toMatchObject({ code: "MOTOR_TEST_REFUSED" }); // over the 1150 cap: refused, not clamped
    const r = await api.device.motorTestStart(2, 1100, true, { maxThrottle: 1150, watchdogMs: 50 });
    expect(r.limits).toEqual({ maxThrottle: 1150, watchdogMs: 100 }); // watchdog clamped 100..2000
    expect(r.outputConfig).toMatchObject({ motorCount: 4, stopValue: 1000, feature3d: false });
    expect(mock.device.motorsSpinning.has(2)).toBe(true);
    await api.device.motorTestKeepalive();
    // Motor test and RC stream share the manager's connection.
    await expect(api.device.rcStreamStart(15)).rejects.toMatchObject({ code: "PORT_BUSY", details: { heldBy: "APP" } });
    await api.device.motorTestStop();
    expect(mock.device.motorsSpinning.size).toBe(0);
    await expect(api.device.motorTestKeepalive()).rejects.toMatchObject({ code: "MOTOR_TEST_NOT_ACTIVE" });
    const stopped: string[] = [];
    await onEvent(EVENTS.rcStopped, (p) => stopped.push(p.reason));
    await api.device.rcStreamStart(null);
    await api.device.rcStreamStop();
    expect(stopped).toEqual(["REQUESTED"]);
    uninstallMock();
    installMock({ latencyMs: 0, scenario: "legacy-backend" });
    await expect(api.device.motorTestStop()).rejects.toMatchObject({ code: "NOT_IMPLEMENTED" });
    await expect(api.device.rcStreamStart(15)).rejects.toMatchObject({ code: "NOT_IMPLEMENTED" });
  });

  it("device_probe_board fills rxSpi.present; the wizard probes only on 4.5+ and keeps identify on failure", async () => {
    const { probeOnboardReceiver, hasOnboardElrs } = await import("../src/wizard/receiver");
    installMock({ latencyMs: 0, scenario: "onboard-elrs" });
    const id = await api.device.identify();
    expect(id.rxSpi.present).toBeNull();
    expect(id.serialPorts.map((p) => p.name)).toContain("UART2");
    const probed = await probeOnboardReceiver(id);
    expect(probed.rxSpi).toMatchObject({ present: true, resources: expect.arrayContaining(["RX_SPI_CS"]) });
    expect(hasOnboardElrs(probed)).toBe(true);
    let calls = 0;
    const probe = async () => (calls++, probed);
    expect(await probeOnboardReceiver({ ...id, firmwareVersion: "4.4.3" }, probe)).toMatchObject({ firmwareVersion: "4.4.3" });
    expect(calls).toBe(0); // leaving the CLI reboots 4.4
    uninstallMock();
    installMock({ latencyMs: 0, scenario: "legacy-backend" });
    expect(await probeOnboardReceiver(id)).toBe(id);
  });

  it("config_apply emits config://progress stages in order and returns its backup", async () => {
    installMock({ latencyMs: 0 });
    const seen: ConfigStage[] = [];
    await onEvent(EVENTS.configProgress, (p) => seen.push(p.stage));
    const script = await api.config.plan(emptyDroneModel());
    const res = await api.config.apply(script);
    expect(res).toMatchObject({ saved: true, reconnected: true, errors: [] });
    expect(res.backup).toMatchObject({ boardUid: "3a0021000c51333038363834" });
    expect(seen).toEqual(["BACKUP", "SENDING", "SAVING", "REBOOTING", "RECONNECTED"]);
  });

  it("scenario mocks raise the agreed detailed error codes", async () => {
    installMock({ latencyMs: 0, scenario: "port-busy" });
    await expect(api.device.connect("COM5")).rejects.toMatchObject({ code: "PORT_BUSY", details: { port: "COM5", heldBy: "EXTERNAL" } });
    uninstallMock();
    const m = installMock({ latencyMs: 0, scenario: "flash-fail" });
    m.device.inDfu = true;
    const image = await api.flasher.fetchFirmware({ target: "SPEEDYBEEF405V4", release: "4.5.2", options: [], offlineOnly: false });
    await expect(api.flasher.flash(image, null, { fullErase: false, manifest: true })).rejects.toMatchObject({ code: "FLASH_FAILED" });
  });
});

describe("firmware update orchestration", () => {
  const poll = { tries: 3, intervalMs: 0 };
  const fc = { fcVariant: "BTFL", firmwareVersion: "4.5.1", apiVersion: "1.46", boardName: "SPEEDYBEEF405V4", manufacturerId: null, mcu: null, uid: null, buildOptions: [], hdOsdSupported: null, serialPorts: [], rxSpi: { present: null, expresslrsPins: false, protocol: null, featureEnabled: null, resources: [] } };

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

describe("review plan, backups and restore", () => {
  it("uses config_plan's script (cleared ports first); without config_plan only a preview, apply blocked", async () => {
    const { fetchPlan } = await import("../src/wizard/plan");
    const m = emptyDroneModel(); // nothing selected: UART2 (RX) and UART3 (VTX) get cleared
    installMock({ latencyMs: 0 });
    const ports = await api.config.readPorts();
    const a = await fetchPlan(m, ports);
    expect(a.status).toBe("ready");
    if (a.status !== "ready") return;
    expect(a.script.cleared).toEqual([
      { port: "UART2", functions: ["RX_SERIAL"], functionMaskBefore: 64, functionMaskAfter: 0 },
      // old MSP DisplayPort port (exactly MSP|VTX_MSP): MSP goes too (ARCHITECTURE §5.1)
      { port: "UART3", functions: ["MSP", "VTX_MSP"], functionMaskBefore: 1 | (1 << 17), functionMaskAfter: 0 },
    ]);
    uninstallMock();
    installMock({ latencyMs: 0, scenario: "legacy-backend" });
    const b = await fetchPlan(m, ports);
    expect(b.status).toBe("unavailable");
    expect(b.status === "unavailable" && b.preview?.commands).toEqual(a.script.commands);
  });

  it("an unsupported receiver comes back as CONFIG and maps to CONFIG_UNSUPPORTED", async () => {
    const { fetchPlan } = await import("../src/wizard/plan");
    installMock({ latencyMs: 0 });
    const m = emptyDroneModel();
    m.receiver = { ...m.receiver, kind: "SPI", protocol: "FRSKY_X" };
    const e = await fetchPlan(m, []).catch((x: unknown) => x);
    expect(e).toMatchObject({ code: "CONFIG" }); // as error.rs sends it
    expect(toUiError(e).code).toBe("CONFIG_UNSUPPORTED");
  });

  it("lists this board's backups (backend or session fallback) and restores one with config_restore", async () => {
    const { listBackups, restoreBackup, canRestoreOnto } = await import("../src/wizard/backups");
    const UID = "3a0021000c51333038363834";
    const mock = installMock({ latencyMs: 0 });
    const before = await api.config.backup();
    expect(before).toMatchObject({ boardUid: UID });
    expect(before.id).toBeTruthy();
    mock.device.backups.push({ backup: { ...before, id: "other/1", boardUid: "other" }, ports: [] });
    mock.device.ports.find((p) => p.name === "UART2")!.functionMask = 0;
    const list = await listBackups(UID, []);
    expect(list.source).toBe("backend");
    expect(list.backups).toEqual([before]); // other board filtered out
    expect(await listBackups(null, [])).toMatchObject({ backups: [] });
    expect(canRestoreOnto(before, UID)).toBe(true);
    expect(canRestoreOnto(before, "other")).toBe(false);
    const res = await restoreBackup(before);
    expect(res.result.backup).not.toBeNull(); // safety backup first
    expect(res.ports.find((p) => p.name === "UART2")!.functionMask).not.toBe(0);
    uninstallMock();
    installMock({ latencyMs: 0, scenario: "legacy-backend" });
    expect(await listBackups(UID, [before])).toEqual({ backups: [before], source: "session" });
    await expect(restoreBackup(before)).rejects.toMatchObject({ code: "NOT_IMPLEMENTED" });
    uninstallMock();
    installMock({ latencyMs: 0, scenario: "rollback-fail" });
    await expect(restoreBackup(before)).rejects.toMatchObject({ code: "CLI_ERROR", category: "Protocol" });
  });
});
