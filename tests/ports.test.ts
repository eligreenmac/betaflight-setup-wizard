// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import {
  exclusionReason,
  FN,
  foreignBits,
  freePortsFor,
  managedBits,
  neededRoles,
  plannedClearings,
  prefillFromBoard,
  targetMask,
  type RoleOptions,
} from "../src/wizard/ports";
import { emptyDroneModel, type DroneModel, type FcSerialPort } from "../src/types/model";

const port = (name: string, identifier: number, functionMask = 0, isConnectionPort = false): FcSerialPort => ({ name, identifier, functionMask, isConnectionPort });

// 4.5 identifiers; 2025.12+ would be UART1=51, the adapter never relies on them.
const PORTS: FcSerialPort[] = [
  port("VCP", 20, FN.MSP, true),
  port("UART1", 0),
  port("UART2", 1, FN.RX_SERIAL),
  port("UART3", 2, FN.MSP | FN.VTX_MSP),
  port("UART4", 3),
  port("UART6", 5, FN.MSP), // plain MSP link: not managed by the wizard
  port("SOFTSERIAL1", 30),
];

const noTlm: RoleOptions = { escTelemetry: false };

function model(patch: (m: DroneModel) => void = () => {}): DroneModel {
  const m = emptyDroneModel();
  patch(m);
  return m;
}
const names = (ps: FcSerialPort[]) => ps.map((p) => p.name);

describe("managed vs foreign function bits", () => {
  it("counts MSP as managed only together with VTX_MSP", () => {
    expect(managedBits(FN.MSP | FN.VTX_MSP)).toBe(FN.MSP | FN.VTX_MSP);
    expect(managedBits(FN.MSP)).toBe(0);
    expect(foreignBits(FN.MSP)).toBe(FN.MSP);
    expect(foreignBits(FN.GPS | FN.RX_SERIAL)).toBe(0);
  });
});

describe("free-port filter", () => {
  const m = model((m) => {
    m.receiver.kind = "SERIAL";
    m.receiver.protocol = "CRSF";
    m.gps.enabled = true;
    m.vtx.kind = "DIGITAL";
    m.vtx.protocol = "MSP";
  });

  it("never offers the USB connection port, soft serial or ports with foreign functions", () => {
    const free = names(freePortsFor("gps", PORTS, m, noTlm));
    expect(free).not.toContain("VCP");
    expect(free).not.toContain("SOFTSERIAL1");
    expect(free).not.toContain("UART6");
    expect(exclusionReason(PORTS[0]!, "gps", m, noTlm)).toBe("reserved_usb");
    expect(exclusionReason(PORTS[6]!, "receiver", m, noTlm)).toBe("softserial_mvp");
    expect(exclusionReason(PORTS[5]!, "gps", m, noTlm)).toBe("in_use");
  });

  it("offers ports holding managed roles (they get re-assigned / cleared)", () => {
    expect(names(freePortsFor("gps", PORTS, m, noTlm))).toEqual(["UART1", "UART2", "UART3", "UART4"]);
  });

  it("hides a port already picked for another role, but not for the role that owns it", () => {
    const picked = model((x) => Object.assign(x, structuredClone(m)));
    picked.receiver.port = "UART2";
    expect(names(freePortsFor("gps", PORTS, picked, noTlm))).not.toContain("UART2");
    expect(names(freePortsFor("receiver", PORTS, picked, noTlm))).toContain("UART2");
    expect(exclusionReason(PORTS[2]!, "gps", picked, noTlm)).toBe("assigned_elsewhere");
  });

  it("ignores ports of roles that are switched off", () => {
    const picked = model((x) => Object.assign(x, structuredClone(m)));
    picked.gps.enabled = false;
    picked.gps.port = "UART4";
    expect(names(freePortsFor("receiver", PORTS, picked, noTlm))).toContain("UART4");
  });
});

describe("needed roles and prefill", () => {
  it("derives needed roles from the model and the ESC telemetry option", () => {
    const m = model((m) => {
      m.receiver.kind = "SPI";
      m.vtx.kind = "ANALOG";
    });
    expect(neededRoles(m, noTlm)).toEqual(["vtx"]);
    expect(neededRoles(m, { escTelemetry: true })).toEqual(["vtx", "escTelemetry"]);
  });

  it("prefills from the board's existing exact matches by NAME", () => {
    const m = model((m) => {
      m.receiver.kind = "SERIAL";
      m.receiver.protocol = "CRSF";
      m.vtx.kind = "DIGITAL";
      m.vtx.protocol = "MSP";
    });
    expect(prefillFromBoard(PORTS, m, noTlm)).toEqual(["receiver", "vtx"]);
    expect(m.receiver.port).toBe("UART2");
    expect(m.vtx.port).toBe("UART3");
  });

  it("does not overwrite a port the user already chose", () => {
    const m = model((m) => {
      m.receiver.kind = "SERIAL";
      m.receiver.port = "UART4";
    });
    prefillFromBoard(PORTS, m, noTlm);
    expect(m.receiver.port).toBe("UART4");
  });
});

describe("clearing managed roles on apply", () => {
  it("clears only wizard-managed roles from ports that lost them", () => {
    const m = model((m) => {
      m.receiver.kind = "SERIAL";
      m.receiver.port = "UART4"; // receiver moved from UART2
      // VTX switched off: UART3's MSP+VTX_MSP is cleared. UART6's plain MSP is untouched.
    });
    expect(plannedClearings(PORTS, m, noTlm)).toEqual([
      { port: "UART2", functions: ["RX_SERIAL"] },
      { port: "UART3", functions: ["MSP", "VTX_MSP"] },
    ]);
    expect(targetMask(PORTS[4]!, m, noTlm)).toBe(FN.RX_SERIAL);
    expect(targetMask(PORTS[5]!, m, noTlm)).toBe(FN.MSP);
    expect(targetMask(PORTS[0]!, m, noTlm)).toBe(FN.MSP);
  });

  it("plans nothing when the board already matches", () => {
    const m = model((m) => {
      m.receiver.kind = "SERIAL";
      m.receiver.port = "UART2";
      m.vtx.kind = "DIGITAL";
      m.vtx.protocol = "MSP";
      m.vtx.port = "UART3";
    });
    expect(plannedClearings(PORTS, m, noTlm)).toEqual([]);
  });

  it("clears a changed VTX protocol on the same port", () => {
    const m = model((m) => {
      m.vtx.kind = "ANALOG";
      m.vtx.protocol = "SMARTAUDIO";
      m.vtx.port = "UART3";
    });
    expect(plannedClearings(PORTS, m, noTlm)).toEqual([
      { port: "UART2", functions: ["RX_SERIAL"] },
      { port: "UART3", functions: ["MSP", "VTX_MSP"] },
    ]);
    expect(targetMask(PORTS[3]!, m, noTlm)).toBe(FN.TBS_SMARTAUDIO);
  });
});
