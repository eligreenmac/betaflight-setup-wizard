// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { ALL_STEPS, visibleSteps } from "../src/wizard/engine";
import { canGoTo, guards } from "../src/wizard/guards";
import { FN } from "../src/wizard/ports";
import { createWizardState, setBoard, setGpsEnabled, setReceiverKind, setReceiverProtocol, setVtxKind, type WizardState } from "../src/wizard/state";
import { asksTelemetry, hasOnboardElrs } from "../src/wizard/receiver";
import { MODES } from "../src/wizard/switches";
import type { FcInfo, FcSerialPort } from "../src/types/model";

const fc = (firmwareVersion = "4.5.1"): FcInfo => ({
  fcVariant: "BTFL",
  firmwareVersion,
  apiVersion: "1.46",
  boardName: "SPEEDYBEEF405V4",
  manufacturerId: "SPBE",
  mcu: "STM32F405",
  uid: null,
  buildOptions: [],
});
const PORTS: FcSerialPort[] = [
  { name: "VCP", identifier: 20, functionMask: FN.MSP, isConnectionPort: true },
  { name: "UART1", identifier: 0, functionMask: 0, isConnectionPort: false },
  { name: "UART2", identifier: 1, functionMask: FN.RX_SERIAL, isConnectionPort: false },
  { name: "UART4", identifier: 3, functionMask: 0, isConnectionPort: false },
];
const keys = (s: WizardState, id: string) => guards[id]!.validate(s).map((i) => i.key);

function ready(): WizardState {
  const s = createWizardState();
  setBoard(s, fc(), PORTS, "COM5");
  s.ui.firmwareDecision = "keep";
  return s;
}

describe("guards map", () => {
  it("has a guard for every step id in steps.json", () => {
    for (const st of ALL_STEPS) expect(guards[st.id], st.id).toBeDefined();
    expect(Object.keys(guards).sort()).toEqual(ALL_STEPS.map((s) => s.id).sort());
  });

  it("connect and firmware block until a supported board is connected and a decision made", () => {
    const s = createWizardState();
    expect(keys(s, "connect")).toEqual(["validation.noBoard"]);
    setBoard(s, fc("4.3.2"), PORTS, "COM5");
    expect(keys(s, "connect")).toEqual([]);
    expect(keys(s, "firmware")).toEqual(["validation.firmwareUnsupported"]);
    setBoard(s, fc("4.5.1"), PORTS, "COM5");
    expect(keys(s, "firmware")).toEqual(["validation.firmwareDecide"]);
    s.ui.firmwareDecision = "keep";
    expect(keys(s, "firmware")).toEqual([]);
  });

  it("components requires a receiver", () => {
    const s = ready();
    expect(keys(s, "components")).toContain("validation.receiverRequired");
    setReceiverKind(s, "SERIAL");
    expect(keys(s, "components")).toEqual([]);
  });

  it("receiver/vtx/gps steps validate their port pickers (by name, prefilled from the board)", () => {
    const s = ready();
    setReceiverKind(s, "SERIAL");
    expect(s.model.receiver.port).toBe("UART2"); // prefilled
    expect(keys(s, "receiver")).toEqual([]);
    setGpsEnabled(s, true);
    expect(keys(s, "gps")).toEqual(["validation.portMissing"]);
    s.model.gps.port = "UART2"; // taken by receiver
    expect(keys(s, "gps")).toEqual(["validation.portConflict"]);
    s.model.gps.port = "UART4";
    expect(keys(s, "gps")).toEqual([]);
    setVtxKind(s, "DIGITAL");
    expect(keys(s, "vtx")).toEqual(["validation.portMissing"]);
    s.model.vtx.port = "UART1";
    expect(keys(s, "vtx")).toEqual([]);
  });

  it("esc requires motor confirmations only when the motor test exists", () => {
    const s = ready();
    s.ui.motorTestAvailable = false;
    expect(keys(s, "esc")).toEqual([]);
    s.ui.motorTestAvailable = true;
    expect(keys(s, "esc")).toEqual(["validation.motorOrderUnconfirmed", "validation.motorDirectionUnconfirmed"]);
    s.model.esc.protocol = "ONESHOT125";
    s.model.esc.bidirectionalDshot = true;
    expect(keys(s, "esc")).toContain("validation.bidirNeedsDshot");
  });

  it("switches: arm required, needs an off position, ELRS needs AUX1, no overlap with arm", () => {
    const s = ready();
    expect(keys(s, "switches")).toEqual([]); // default: ARM on AUX1 high
    s.model.switches = [];
    expect(keys(s, "switches")).toEqual(["validation.armRequired"]);
    s.model.switches = [{ mode: MODES.arm, auxChannel: 1, rangeMin: 900, rangeMax: 2100 }];
    expect(keys(s, "switches")).toContain("validation.armAlwaysOn");
    s.model.switches = [{ mode: MODES.arm, auxChannel: 1, rangeMin: 1700, rangeMax: 2100 }];
    s.model.receiver.protocol = "EXPRESSLRS";
    expect(keys(s, "switches")).toEqual(["validation.elrsArmAux1"]);
    s.model.receiver.protocol = "CRSF";
    s.model.switches.push({ mode: MODES.angle, auxChannel: 1, rangeMin: 1300, rangeMax: 2100 });
    expect(keys(s, "switches")).toEqual(["validation.armOverlap"]);
  });

  it("review needs a generated script; apply needs verification", () => {
    const s = ready();
    expect(keys(s, "review")).toEqual(["validation.notGenerated"]);
    s.ui.script = { commands: ["save"], warnings: [], dialect: "4.5" };
    expect(keys(s, "review")).toEqual([]);
    expect(keys(s, "apply")).toEqual(["validation.notApplied"]);
    s.ui.applyStatus = "verified";
    expect(keys(s, "apply")).toEqual([]);
    expect(guards.done!.canEnter(s)).toBe(true);
  });

  it("canGoTo: back always, forward only through leavable steps", () => {
    const s = createWizardState();
    const steps = visibleSteps(s.model);
    expect(canGoTo("welcome", "connect", s, steps)).toBe(true);
    expect(canGoTo("firmware", "welcome", s, steps)).toBe(false);
    setBoard(s, fc(), PORTS, "COM5");
    expect(canGoTo("firmware", "welcome", s, steps)).toBe(true);
    expect(canGoTo("components", "welcome", s, steps)).toBe(false);
  });

  it("changing to a different board drops port picks", () => {
    const s = ready();
    setReceiverKind(s, "SERIAL");
    s.model.receiver.port = "UART4";
    setBoard(s, { ...fc(), boardName: "MATEKH743" }, [], "COM6");
    expect(s.model.receiver.port).toBeNull();
  });

  it("onboard SPI ExpressLRS only when the board reports it", () => {
    const s = ready();
    expect(hasOnboardElrs(s.fc)).toBe(false);
    setReceiverKind(s, "SPI");
    expect(keys(s, "components")).toContain("validation.noOnboardReceiver");
    setBoard(s, { ...fc(), buildOptions: ["USE_RX_EXPRESSLRS"] }, PORTS, "COM5");
    expect(hasOnboardElrs(s.fc)).toBe(true);
    expect(keys(s, "components")).toEqual([]);
    expect(s.model.receiver.protocol).toBe("EXPRESSLRS");
    expect(keys(s, "receiver")).toEqual([]); // no UART needed
  });

  it("asks about telemetry for CRSF and FPort only, defaulting to yes", () => {
    const s = ready();
    setReceiverKind(s, "SERIAL");
    expect(s.model.receiver.protocol).toBe("CRSF");
    expect(s.model.receiver.telemetry).toBe(true);
    setReceiverProtocol(s, "SBUS");
    expect(asksTelemetry("SBUS")).toBe(false);
    expect(s.model.receiver.telemetry).toBe(false);
    setReceiverProtocol(s, "FPORT");
    expect(asksTelemetry("FPORT")).toBe(true);
    expect(s.model.receiver.telemetry).toBe(true);
  });
});
