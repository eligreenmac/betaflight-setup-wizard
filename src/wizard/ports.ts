// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Port-role adapter between the board's serial ports (`FcSerialPort`, from
 * `config_read_ports`) and the wizard answers (`DroneModel`, ports by NAME).
 * Pure functions, no Vue, unit tested in tests/ports.test.ts.
 *
 * Rules for the UART picker:
 *  1. The connection port (USB VCP) is never offered.
 *  2. Soft serial is not offered (out of MVP scope).
 *  3. A port already chosen for another wizard role is not offered.
 *  4. A port with functions the wizard does NOT manage (e.g. a plain MSP link for a
 *     Bluetooth module, blackbox, a second receiver) is "in use" and not offered.
 *     Bits the wizard manages (RX, VTX, GPS, ESC telemetry) do not block a port:
 *     those roles are re-assigned by the wizard and stale ones are cleared on apply.
 */
import type { DroneModel, FcSerialPort } from "../types/model";

/** Betaflight serialPortFunction_e bits (same for 4.4 / 4.5 / 2025.12, see ARCHITECTURE §5). */
export const FN = {
  MSP: 1 << 0,
  GPS: 1 << 1,
  RX_SERIAL: 1 << 6,
  ESC_SENSOR: 1 << 10,
  TBS_SMARTAUDIO: 1 << 11,
  IRC_TRAMP: 1 << 13,
  VTX_MSP: 1 << 17,
} as const;
export type FunctionName = keyof typeof FN;

/** Functions whose ownership the wizard takes over (and therefore may clear). */
export const MANAGED_MASK = FN.GPS | FN.RX_SERIAL | FN.ESC_SENSOR | FN.TBS_SMARTAUDIO | FN.IRC_TRAMP | FN.VTX_MSP;

export const PORT_ROLES = ["receiver", "vtx", "gps", "escTelemetry"] as const;
export type PortRole = (typeof PORT_ROLES)[number];

/** UI-only answers that DroneModel has no field for. */
export interface RoleOptions {
  /** ESC telemetry wire soldered to a UART (DroneModel only stores the port). */
  escTelemetry: boolean;
}

/** Bits the wizard manages on a port. MSP counts only when paired with VTX_MSP (digital VTX). */
export function managedBits(mask: number): number {
  let m = mask & MANAGED_MASK;
  if (mask & FN.VTX_MSP) m |= mask & FN.MSP;
  return m;
}

export function foreignBits(mask: number): number {
  return mask & ~managedBits(mask);
}

export function functionNames(mask: number): FunctionName[] {
  return (Object.keys(FN) as FunctionName[]).filter((k) => (mask & FN[k]) !== 0);
}

export function isSoftSerial(name: string): boolean {
  return name.toUpperCase().startsWith("SOFTSERIAL");
}

export function isReserved(port: FcSerialPort): boolean {
  return port.isConnectionPort || port.name.toUpperCase() === "VCP";
}

/** Function mask the role needs given the current answers; 0 if the role needs no UART. */
export function roleMask(role: PortRole, model: DroneModel, opts: RoleOptions): number {
  switch (role) {
    case "receiver":
      return model.receiver.kind === "SERIAL" ? FN.RX_SERIAL : 0;
    case "gps":
      return model.gps.enabled ? FN.GPS : 0;
    case "escTelemetry":
      return opts.escTelemetry ? FN.ESC_SENSOR : 0;
    case "vtx":
      if (model.vtx.kind === "NONE") return 0;
      switch (model.vtx.protocol) {
        case "SMARTAUDIO":
          return FN.TBS_SMARTAUDIO;
        case "TRAMP":
          return FN.IRC_TRAMP;
        case "MSP":
          return FN.MSP | FN.VTX_MSP;
        default:
          return 0;
      }
  }
}

/** Roles that need a UART. A VTX without a chosen protocol yet still needs a port. */
export function neededRoles(model: DroneModel, opts: RoleOptions): PortRole[] {
  return PORT_ROLES.filter((r) => (r === "vtx" ? model.vtx.kind !== "NONE" : roleMask(r, model, opts) !== 0));
}

export function getRolePort(model: DroneModel, role: PortRole): string | null {
  switch (role) {
    case "receiver":
      return model.receiver.port;
    case "vtx":
      return model.vtx.port;
    case "gps":
      return model.gps.port;
    case "escTelemetry":
      return model.esc.telemetryPort;
  }
}

export function setRolePort(model: DroneModel, role: PortRole, name: string | null): void {
  switch (role) {
    case "receiver":
      model.receiver.port = name;
      break;
    case "vtx":
      model.vtx.port = name;
      break;
    case "gps":
      model.gps.port = name;
      break;
    case "escTelemetry":
      model.esc.telemetryPort = name;
      break;
  }
}

/** Drop ports for roles that no longer need one (e.g. GPS switched off). */
export function clearUnneededRolePorts(model: DroneModel, opts: RoleOptions): void {
  const needed = neededRoles(model, opts);
  for (const r of PORT_ROLES) if (!needed.includes(r)) setRolePort(model, r, null);
}

export type ExclusionReason = "reserved_usb" | "softserial_mvp" | "assigned_elsewhere" | "in_use";

/** Why `port` cannot be offered for `role`, or null if it can. */
export function exclusionReason(port: FcSerialPort, role: PortRole, model: DroneModel, opts: RoleOptions): ExclusionReason | null {
  if (isReserved(port)) return "reserved_usb";
  if (isSoftSerial(port.name)) return "softserial_mvp";
  const needed = neededRoles(model, opts);
  if (needed.some((r) => r !== role && getRolePort(model, r) === port.name)) return "assigned_elsewhere";
  if (foreignBits(port.functionMask) !== 0) return "in_use";
  return null;
}

export function freePortsFor(role: PortRole, ports: readonly FcSerialPort[], model: DroneModel, opts: RoleOptions): FcSerialPort[] {
  return ports.filter((p) => exclusionReason(p, role, model, opts) === null);
}

/** Ports hidden from the picker, with the reason (for the "why isn't my port listed?" hint). */
export function excludedPortsFor(
  role: PortRole,
  ports: readonly FcSerialPort[],
  model: DroneModel,
  opts: RoleOptions,
): { port: FcSerialPort; reason: ExclusionReason }[] {
  return ports.flatMap((port) => {
    const reason = exclusionReason(port, role, model, opts);
    return reason ? [{ port, reason }] : [];
  });
}

/**
 * Pre-fill empty role ports from the board's current config: a port whose managed
 * functions are exactly what the role needs (re-running the wizard on a set-up quad).
 * Only fills roles that have no port yet. Returns the roles that were filled.
 */
export function prefillFromBoard(ports: readonly FcSerialPort[], model: DroneModel, opts: RoleOptions): PortRole[] {
  const filled: PortRole[] = [];
  for (const role of neededRoles(model, opts)) {
    if (getRolePort(model, role) !== null) continue;
    const want = roleMask(role, model, opts);
    if (want === 0) continue;
    const hit = ports.find(
      (p) => !isReserved(p) && !isSoftSerial(p.name) && managedBits(p.functionMask) === want && exclusionReason(p, role, model, opts) === null,
    );
    if (hit) {
      setRolePort(model, role, hit.name);
      filled.push(role);
    }
  }
  return filled;
}

export interface PortClearing {
  port: string;
  /** Managed functions that will be removed from this port. */
  functions: FunctionName[];
}

/**
 * Managed functions that apply will REMOVE because the role moved or was switched off.
 * Only the wizard-managed roles (RX, VTX, GPS, ESC telemetry) are ever cleared;
 * unrelated functions on the same port are kept. Shown first on the Review screen.
 */
export function plannedClearings(ports: readonly FcSerialPort[], model: DroneModel, opts: RoleOptions): PortClearing[] {
  const out: PortClearing[] = [];
  for (const p of ports) {
    if (isReserved(p)) continue;
    const have = managedBits(p.functionMask);
    if (have === 0) continue;
    let keep = 0;
    for (const role of neededRoles(model, opts)) if (getRolePort(model, role) === p.name) keep |= roleMask(role, model, opts);
    const clear = have & ~keep;
    if (clear !== 0) out.push({ port: p.name, functions: functionNames(clear) });
  }
  return out;
}

/** The mask each port should end up with after apply (for the mock and for tests). */
export function targetMask(port: FcSerialPort, model: DroneModel, opts: RoleOptions): number {
  if (isReserved(port)) return port.functionMask;
  let mask = port.functionMask & ~managedBits(port.functionMask);
  for (const role of neededRoles(model, opts)) if (getRolePort(model, role) === port.name) mask |= roleMask(role, model, opts);
  return mask;
}
