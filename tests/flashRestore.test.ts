// SPDX-License-Identifier: GPL-3.0-or-later
// Firmware update options (keep settings / full erase) and the restore after the flash
// (Config #92 config_restore_after_flash via wizard/restore.ts): orchestration, warnings,
// the report (failed / skipped / notes), refusals, retry, the mid-wizard update, en/he.
import { mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import App from "../src/App.vue";
import { setBackend, type Backend } from "../src/api/commands";
import { isWriteCommand, resetDemoState } from "../src/api/fallback";
import { resetCatalog } from "../src/api/catalog";
import { uiError } from "../src/errors/codes";
import { i18n, setLocale } from "../src/i18n";
import { installMock, uninstallMock } from "../src/mocks/install";
import type { Scenario, ScenarioMock } from "../src/mocks/scenarios";
import { runFirmwareUpdate } from "../src/wizard/flash";
import { nav, resetNav } from "../src/wizard/nav";
import { requestReflash } from "../src/wizard/reflash";
import {
  canRetryRestore, crossLine, lineResetsSettings, refusalKind, refusalStage, RESTORE_COMMAND, RESTORE_REFUSAL_REASONS, RESTORE_SKIP_REASONS, visibleSkips, type RestoreReport,
} from "../src/wizard/restore";
import { resetWizard, wizard } from "../src/wizard/state";
import type { FcInfo } from "../src/types/model";
import en from "../src/i18n/locales/en.json";
import he from "../src/i18n/locales/he.json";
import { click, settle } from "./walk";

let w: VueWrapper | null = null;
let mock: ScenarioMock;

beforeEach(() => {
  resetDemoState();
  resetWizard();
  resetNav();
  resetCatalog();
  setLocale("en");
});
afterEach(() => {
  w?.unmount();
  w = null;
  uninstallMock();
  resetDemoState();
  document.body.innerHTML = "";
});

const poll = { tries: 3, intervalMs: 0 };
const hooks = { onStage: () => {}, onProgress: () => {} };
const configError = (reason: string, extra: object = {}) => ({ ...uiError("CONFIG_UNSUPPORTED", "configuration error"), details: { reason, backupId: "uid/20261023T040000Z", ...extra } as never });

async function boot(scenario: Scenario, stepMs = 0): Promise<VueWrapper> {
  mock = installMock({ scenario, latencyMs: 0, stepMs });
  w = mount(App, { global: { plugins: [i18n] }, attachTo: document.body });
  await settle();
  await click(w, "next");
  await click(w, "connect");
  nav.currentId = "firmware";
  await settle();
  return w;
}
async function pick(release: string): Promise<void> {
  await w!.get("#release").setValue(release);
  await settle();
}
const has = (sel: string) => w!.find(`[data-test="${sel}"]`).exists();
const checkbox = (sel: string) => (w!.get(`[data-test="${sel}"]`).element as HTMLInputElement).checked;

describe("restore helpers", () => {
  it("keys refusals on details.reason, with UNSUPPORTED_FIRMWARE, not-available and a generic fallback", () => {
    for (const r of RESTORE_REFUSAL_REASONS) expect(refusalKind(configError(r))).toBe(r);
    expect(refusalKind(configError("SOMETHING_NEW"))).toBe("GENERIC");
    expect(refusalKind(uiError("UNSUPPORTED_FIRMWARE", "x"))).toBe("UNSUPPORTED_FIRMWARE");
    expect(refusalKind(uiError("NOT_IMPLEMENTED", "x"))).toBe("NOT_AVAILABLE");
    expect(refusalKind(uiError("MIXED_BACKEND", "x"))).toBe("NOT_AVAILABLE");
    expect(refusalKind(uiError("TIMEOUT", "x"))).toBe("GENERIC");
  });

  it("BOARD_NOT_RECONNECTED says how far it got; retry only where it can help", () => {
    expect(refusalStage(configError("BOARD_NOT_RECONNECTED"))).toBe("NOTHING_SENT");
    expect(refusalStage(configError("BOARD_NOT_RECONNECTED", { stage: "RESTORING" }))).toBe("RESTORING");
    expect(refusalStage(configError("BOARD_NOT_RECONNECTED", { stage: "REBOOTING" }))).toBe("REBOOTING");
    expect(refusalStage(configError("BOARD_UID_MISMATCH"))).toBeNull();
    expect(canRetryRestore(configError("BOARD_NOT_RECONNECTED"))).toBe(true);
    expect(canRetryRestore(configError("BOARD_UID_MISMATCH"))).toBe(false);
    expect(canRetryRestore(configError("BACKUP_NOT_DIFF_ALL"))).toBe(false);
  });

  it("cross-line = a different major.minor; bookkeeping skips are never listed", () => {
    expect(crossLine("4.5.1", "4.5.2")).toBe(false);
    expect(crossLine("4.5.1", "2025.12.0-RC1")).toBe(true);
    expect(crossLine("2026.6.2", "4.4.3")).toBe(true);
    expect(crossLine("", "4.5.2")).toBe(false);
    const r: RestoreReport = {
      applied: 1, failed: [], warnings: [], saved: true, lineChanged: false, fromVersion: "4.5.1", toVersion: "4.5.2",
      skipped: [
        { line: "# version", reason: "COMMENT", detail: "" },
        { line: "batch start", reason: "SESSION_CONTROL", detail: "" },
        { line: "board_name X", reason: "BOARD_IDENTITY", detail: "" },
        { line: "set iterm_limit = 400", reason: "REMOVED_ON_TARGET", detail: "" },
      ],
    };
    expect(visibleSkips(r).map((s) => s.reason)).toEqual(["REMOVED_ON_TARGET"]);
  });

  it("a normal flash resets settings across lines whose config version differs (not 4.5 <-> 2025.12)", () => {
    expect(lineResetsSettings("4.5.1", "4.5.2")).toBe(false);
    expect(lineResetsSettings("4.5.1", "2025.12.0-RC1")).toBe(false);
    expect(lineResetsSettings("2025.12.5", "4.5.5")).toBe(false);
    expect(lineResetsSettings("4.4.3", "4.5.1")).toBe(true);
    expect(lineResetsSettings("4.5.1", "4.4.3")).toBe(true);
    expect(lineResetsSettings("2025.12.5", "2026.6.2")).toBe(true);
    expect(lineResetsSettings("4.5.1", "2027.6.0")).toBe(true); // unknown line: assume it resets
    expect(lineResetsSettings("", "4.5.2")).toBe(false);
  });

  it("the restore is a board write for the mixed-backend guard", () => {
    expect(isWriteCommand(RESTORE_COMMAND)).toBe(true);
  });
});

describe("runFirmwareUpdate options", () => {
  const fc = (): FcInfo => ({
    fcVariant: "BTFL", firmwareVersion: "4.5.1", apiVersion: "1.46", boardName: "SPEEDYBEEF405V4", manufacturerId: "SPBE", mcu: "STM32F405",
    uid: "3a0021000c51333038363834", buildOptions: [], hdOsdSupported: true, serialPorts: [],
    rxSpi: { present: null, expresslrsPins: false, protocol: null, featureEnabled: null, resources: [] },
  });

  it("keepSettings off: the backup is still taken and kept, nothing is restored", async () => {
    mock = installMock({ latencyMs: 0, stepMs: 0 });
    const stages: string[] = [];
    const r = await runFirmwareUpdate(fc(), "4.5.2", { ...hooks, onStage: (s) => stages.push(s) }, poll, { keepSettings: false });
    expect(stages).not.toContain("restore");
    expect(r.backup).not.toBeNull();
    expect(mock.device.backups.map((b) => b.backup.id)).toContain(r.backup!.id);
    expect(mock.device.restoreCalls).toEqual([]);
    expect(r.restore).toBeNull();
    expect(r.restoreBackupId).toBeNull();
  });

  it("full erase and keep settings together: a full erase is asked for, then the backup is restored", async () => {
    mock = installMock({ latencyMs: 0, stepMs: 0 });
    const flashOptions: unknown[] = [];
    const inner = mock.backend;
    setBackend((async (cmd, args) => {
      if (cmd === "flasher_flash") flashOptions.push((args as { options: unknown }).options);
      return inner(cmd, args);
    }) as Backend);
    const progress: string[] = [];
    const r = await runFirmwareUpdate(fc(), "4.5.2", { ...hooks, onRestoreProgress: (p) => progress.push(p.stage) }, poll, { fullErase: true, keepSettings: true });
    expect(flashOptions).toEqual([{ fullErase: true, manifest: true }]);
    expect(mock.device.restoreCalls).toEqual([r.backup!.id]);
    expect(progress).toEqual(["RESTORING", "RESTORING", "RESTORING", "SAVING", "REBOOTING", "RECONNECTED"]);
    expect(r.restore).toMatchObject({ saved: true, lineChanged: false, failed: [], warnings: [] });
  });

  it("a cross-line restore reports failed and skipped lines and notes", async () => {
    mock = installMock({ latencyMs: 0, stepMs: 0 });
    const r = await runFirmwareUpdate(fc(), "2025.12.0-RC1", hooks, poll);
    expect(r.restore).toMatchObject({ lineChanged: true, fromVersion: "4.5.1", toVersion: "2025.12.0-RC1", saved: true });
    expect(r.restore!.failed.length).toBe(2);
    expect(visibleSkips(r.restore!).map((s) => s.reason)).toEqual(["HARDWARE_LINE_CHANGED", "HARDWARE_LINE_CHANGED", "REMOVED_ON_TARGET", "SEMANTICS_CHANGED", "NO_EQUIVALENT_MODE"]);
    expect(r.restore!.warnings.length).toBe(1);
  });

  it("a refusal never fails the update: the firmware is installed and the backup is kept", async () => {
    mock = installMock({ scenario: "fw-restore-other-board", latencyMs: 0, stepMs: 0 });
    const r = await runFirmwareUpdate(fc(), "4.5.2", hooks, poll);
    expect(r.fc.firmwareVersion).toBe("4.5.2");
    expect(r.restore).toBeNull();
    expect(refusalKind(r.restoreError!)).toBe("BOARD_UID_MISMATCH");
    expect(r.restoreBackupId).toBe(r.backup!.id);
  });
});

describe("Firmware step: update options", () => {
  it("keep settings on and full erase off by default; each combination explains itself", async () => {
    await boot("happy");
    expect(checkbox("opt-keep")).toBe(true);
    expect(checkbox("opt-erase")).toBe(false);
    expect(w!.text()).toContain(en.flashOptions.keepHelp);
    expect(has("opt-warn-erase-keep") || has("opt-warn-erase-reset")).toBe(false);

    await w!.get('[data-test="opt-erase"]').setValue(true);
    expect(w!.get('[data-test="opt-warn-erase-keep"]').text()).toBe(en.flashOptions.warnEraseKeep);
    await w!.get('[data-test="opt-keep"]').setValue(false);
    expect(has("opt-warn-erase-keep")).toBe(false);
    expect(w!.get('[data-test="opt-warn-erase-reset"]').text()).toBe(en.flashOptions.warnEraseReset);
    expect(w!.text()).toContain(en.flashOptions.keepOffHelp);
  });

  it("the cross-line warning only when the version line changes and settings are kept", async () => {
    await boot("happy");
    expect(has("opt-warn-cross-line")).toBe(false); // 4.5.1 -> 4.5.2
    await pick("4.4.3");
    expect(w!.get('[data-test="opt-warn-cross-line"]').text()).toContain("4.4.3");
    await w!.get('[data-test="opt-keep"]').setValue(false);
    expect(has("opt-warn-cross-line")).toBe(false);
  });

  it("keep off + a version line that resets settings: says the board starts from defaults even without a full erase", async () => {
    await boot("happy");
    await w!.get('[data-test="opt-keep"]').setValue(false);
    expect(has("opt-warn-line-reset")).toBe(false); // 4.5.1 -> 4.5.2 keeps them
    await pick("4.4.3");
    const warn = w!.get('[data-test="opt-warn-line-reset"]').text();
    expect(warn).toContain("Betaflight 4.4.3 stores settings differently from 4.5.1");
    expect(warn).toContain("even without erasing the chip");
    expect(warn).toContain("Keep my settings");
    await w!.get('[data-test="opt-erase"]').setValue(true); // the full-erase warning says it already
    expect(has("opt-warn-line-reset")).toBe(false);
    expect(has("opt-warn-erase-reset")).toBe(true);
    await w!.get('[data-test="opt-erase"]').setValue(false);
    await w!.get('[data-test="opt-keep"]').setValue(true); // kept: the cross-line note instead
    expect(has("opt-warn-line-reset")).toBe(false);
    expect(has("opt-warn-cross-line")).toBe(true);
  });

  it("the help says a line change resets settings and a full erase takes up to about 30 s; the wait is shown while erasing", async () => {
    expect(en.flashOptions.keepHelp).toContain("even without erasing the chip");
    expect(en.flashOptions.keepOffHelp).toContain("Moving to another version line, or erasing the whole chip, starts from default settings");
    expect(en.flashOptions.eraseHelp).toContain("up to about 30 seconds");
    expect(he.flashOptions.eraseHelp).toContain("30");
    await boot("happy", 25);
    await w!.get('[data-test="opt-erase"]').setValue(true);
    await w!.get('[data-test="update"]').trigger("click");
    const end = Date.now() + 3000;
    while (!has("full-erase-wait") && Date.now() < end) await new Promise((r) => setTimeout(r, 5));
    expect(w!.get('[data-test="full-erase-wait"]').text()).toBe(en.flashOptions.eraseWait);
    while (!wizard.ui.flashed && Date.now() < end + 3000) await new Promise((r) => setTimeout(r, 10));
    await settle();
    expect(has("full-erase-wait")).toBe(false);
  });

  it("no full-erase wait note on a normal flash", async () => {
    await boot("happy", 25);
    await w!.get('[data-test="update"]').trigger("click");
    let sawErasing = false;
    const end = Date.now() + 3000;
    while (!wizard.ui.flashed && Date.now() < end) {
      if (w!.text().includes(en.firmware.flashStage.ERASING)) sawErasing = true;
      expect(has("full-erase-wait")).toBe(false);
      await new Promise((r) => setTimeout(r, 5));
    }
    expect(sawErasing).toBe(true);
  });

  it("a same-line update puts the settings back and says so", async () => {
    await boot("happy");
    await click(w!, "update");
    expect(wizard.ui.flashed).toBe(true);
    expect(mock.device.restoreCalls.length).toBe(1);
    expect(w!.get('[data-test="restore-done"]').text()).toContain(en.restore.doneTitle);
    expect(has("restore-failed") || has("restore-skipped") || has("restore-warnings") || has("restore-line-changed")).toBe(false);
  });

  it("a cross-line update lists failed lines verbatim, skipped lines with reasons, and notes", async () => {
    await boot("happy");
    await pick("4.4.3");
    await click(w!, "update");
    expect(has("restore-line-changed")).toBe(true);
    const failed = w!.get('[data-test="restore-failed"]');
    expect(failed.element.tagName).toBe("DETAILS");
    expect(failed.findAll("li").length).toBe(2);
    expect(failed.text()).toContain("set angle_limit = 85");
    expect(failed.text()).toContain("###ERROR IN set: INVALID VALUE### (angle_limit: 10 - 80)");
    const skipped = w!.get('[data-test="restore-skipped"]');
    expect(skipped.findAll("li").length).toBe(5);
    expect(skipped.get('[data-reason="SEMANTICS_CHANGED"]').text()).toContain(en.restore.skip.SEMANTICS_CHANGED);
    expect(skipped.text()).not.toContain("board_name");
    expect(w!.get('[data-test="restore-warnings"]').text()).toContain("re-tune D");
  });

  it("keep settings off: no restore, no restore result", async () => {
    await boot("happy");
    await w!.get('[data-test="opt-keep"]').setValue(false);
    await click(w!, "update");
    expect(wizard.ui.flashed).toBe(true);
    expect(mock.device.restoreCalls).toEqual([]);
    expect(has("restore-done") || has("restore-refused")).toBe(false);
  });

  it("refused (another board's backup): flash succeeded, backup saved, Backups list here, no retry", async () => {
    await boot("fw-restore-other-board");
    await click(w!, "update");
    expect(wizard.ui.flashed).toBe(true);
    const box = w!.get('[data-test="restore-refused"]');
    expect(box.attributes("data-kind")).toBe("BOARD_UID_MISMATCH");
    expect(box.text()).toContain(en.restore.refusedTitle);
    expect(box.text()).toContain(en.restore.refused.BOARD_UID_MISMATCH);
    expect(box.get('[data-test="restore-backup-saved"]').text()).toContain(mock.device.restoreCalls[0]);
    expect(has("restore-retry")).toBe(false);
    await click(w!, "restore-show-backups");
    expect(box.find('[data-test="backups"]').exists()).toBe(true);
  });

  it("board lost mid-restore: says nothing was saved; Try again restores", async () => {
    await boot("fw-restore-dropped");
    await click(w!, "update");
    expect(w!.get('[data-test="restore-refused"]').attributes("data-kind")).toBe("BOARD_NOT_RECONNECTED");
    expect(w!.get('[data-test="restore-refused-stage"]').text()).toBe(en.restore.refusedStage.RESTORING);
    await click(w!, "restore-retry");
    expect(mock.device.restoreCalls.length).toBe(2);
    expect(has("restore-refused")).toBe(false);
    expect(has("restore-done")).toBe(true);
  });

  it("a backend that can't restore yet: the update still succeeds and says so", async () => {
    await boot("legacy-backend");
    await click(w!, "update");
    expect(wizard.ui.flashed).toBe(true);
    expect(w!.get('[data-test="restore-refused"]').attributes("data-kind")).toBe("NOT_AVAILABLE");
  });

  it("mid-wizard update: same options; stays on the step when the settings weren't put back", async () => {
    await boot("happy");
    mock.device.restoreRefusals.push({ reason: "BACKUP_UNREADABLE" });
    expect(requestReflash(wizard, { returnTo: "osd", requirements: { osd: { videoSystem: "HD" } } })).toBe(true);
    nav.currentId = "firmware";
    await settle();
    expect(has("reflash-banner")).toBe(true);
    expect(checkbox("opt-keep")).toBe(true);
    await click(w!, "update");
    expect(nav.currentId).toBe("firmware");
    expect(w!.get('[data-test="restore-refused"]').attributes("data-kind")).toBe("BACKUP_UNREADABLE");
    expect(has("reflash-return")).toBe(true);
  });

  it("Hebrew: the options and the report read in Hebrew", async () => {
    setLocale("he");
    await boot("happy");
    expect(w!.text()).toContain(he.flashOptions.keep);
    await pick("4.4.3");
    expect(w!.text()).toContain(he.flashOptions.warnCrossLine.split("{")[0]);
    await click(w!, "update");
    expect(w!.get('[data-test="restore-skipped"]').text()).toContain(he.restore.skip.REMOVED_ON_TARGET);
  });
});

describe("restore texts", () => {
  it("en and he have the same keys, every skip reason and refusal kind has a text", () => {
    const keys = (o: object, p = ""): string[] =>
      Object.entries(o).flatMap(([k, v]) => (v && typeof v === "object" ? keys(v, `${p}${k}.`) : [`${p}${k}`]));
    expect(keys(he.flashOptions).sort()).toEqual(keys(en.flashOptions).sort());
    expect(keys(he.restore).sort()).toEqual(keys(en.restore).sort());
    for (const r of RESTORE_SKIP_REASONS) expect(en.restore.skip[r]).toBeTruthy();
    for (const k of [...RESTORE_REFUSAL_REASONS, "UNSUPPORTED_FIRMWARE", "NOT_AVAILABLE", "GENERIC"]) {
      expect(en.restore.refused[k as keyof typeof en.restore.refused]).toBeTruthy();
      expect(he.restore.refused[k as keyof typeof he.restore.refused]).toBeTruthy();
    }
  });
});
