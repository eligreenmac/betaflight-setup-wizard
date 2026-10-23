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
    expect(toNotImplemented("motor_test_start", appErr)).toBe(appErr);
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
    await upcoming.motorTestStart(2, 5);
    expect(mock.device.motorsSpinning.has(2)).toBe(true);
    await upcoming.motorTestStop();
    expect(mock.device.motorsSpinning.size).toBe(0);
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
