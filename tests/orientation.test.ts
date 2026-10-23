// SPDX-License-Identifier: GPL-3.0-or-later
// Board alignment preset math (client-side preview; no live alignment command).
import { describe, expect, it } from "vitest";
import { ALIGNMENT_PRESETS, alignedAttitude, reportedAttitude, sameAlignment, tiltDirection, toEuler, toMatrix, type Euler } from "../src/wizard/orientation";

const close = (a: Euler, b: Euler) => {
  const d = (x: number, y: number) => Math.abs(((x - y + 540) % 360) - 180);
  expect(d(a.roll, b.roll)).toBeLessThan(1e-6);
  expect(d(a.pitch, b.pitch)).toBeLessThan(1e-6);
  expect(d(a.yaw, b.yaw)).toBeLessThan(1e-6);
};
const noseDown: Euler = { roll: 0, pitch: -30, yaw: 0 };
const rightDown: Euler = { roll: 30, pitch: 0, yaw: 0 };

describe("orientation math", () => {
  it("toEuler(toMatrix(e)) round-trips", () => {
    for (const e of [{ roll: 10, pitch: -20, yaw: 30 }, { roll: -170, pitch: 45, yaw: 300 }, { roll: 0, pitch: 0, yaw: 0 }]) close(toEuler(toMatrix(e)), e);
  });

  it("a correctly mounted board reports the frame's attitude", () => {
    close(reportedAttitude(noseDown, { roll: 0, pitch: 0, yaw: 0 }), noseDown);
    expect(tiltDirection(noseDown)).toBe("noseDown");
    expect(tiltDirection(rightDown)).toBe("rightDown");
    expect(tiltDirection({ roll: 5, pitch: -5, yaw: 0 })).toBe("level");
  });

  it("board turned 90° right: nose down reads as a roll, and the yaw 90 preset fixes it", () => {
    const mount = { roll: 0, pitch: 0, yaw: 90 };
    const r = reportedAttitude(noseDown, mount);
    expect(tiltDirection(r)).not.toBe("noseDown");
    expect(["leftDown", "rightDown"]).toContain(tiltDirection(r));
    const fixed = alignedAttitude(r, mount);
    expect(tiltDirection(fixed)).toBe("noseDown");
    expect(tiltDirection(alignedAttitude(reportedAttitude(rightDown, mount), mount))).toBe("rightDown");
    // A wrong preset does not fix it.
    expect(tiltDirection(alignedAttitude(r, { roll: 0, pitch: 0, yaw: 270 }))).not.toBe("noseDown");
  });

  it("upside-down mounting (roll 180) is recovered by its preset", () => {
    for (const p of ALIGNMENT_PRESETS) {
      for (const body of [noseDown, rightDown, { roll: 0, pitch: 0, yaw: 0 }]) close(alignedAttitude(reportedAttitude(body, p.align), p.align), body);
    }
    const r = reportedAttitude({ roll: 0, pitch: 0, yaw: 0 }, { roll: 180, pitch: 0, yaw: 0 });
    expect(Math.abs(r.roll)).toBeCloseTo(180);
  });

  it("exactly one preset recovers each mounting", () => {
    for (const mount of ALIGNMENT_PRESETS) {
      const ok = ALIGNMENT_PRESETS.filter((p) => {
        const a = alignedAttitude(reportedAttitude(noseDown, mount.align), p.align);
        const b = alignedAttitude(reportedAttitude(rightDown, mount.align), p.align);
        return tiltDirection(a) === "noseDown" && tiltDirection(b) === "rightDown" && Math.abs(a.roll) < 90;
      });
      expect(ok.map((p) => p.id)).toEqual([mount.id]);
    }
  });

  it("sameAlignment", () => {
    expect(sameAlignment(null, { roll: 0, pitch: 0, yaw: 0 })).toBe(false);
    expect(sameAlignment({ roll: 0, pitch: 0, yaw: 90 }, { roll: 0, pitch: 0, yaw: 90 })).toBe(true);
  });
});
