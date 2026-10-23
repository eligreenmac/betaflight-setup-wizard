// SPDX-License-Identifier: GPL-3.0-or-later
// Live 3D orientation view: the Euler -> scene rotation mapping (MSP pitch + = nose DOWN,
// roll + = right side down, yaw + = nose right), the 2D fallback (no WebGL, as in jsdom, or
// the renderer failing), the alignment preview turning the 3D quad, and the 30 Hz attitude
// stream running only while the step is visible.
import { mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "../src/App.vue";
import { resetDemoState } from "../src/api/fallback";
import { attitudeToEuler } from "../src/api/pending";
import { i18n, setLocale } from "../src/i18n";
import en from "../src/i18n/locales/en.json";
import he from "../src/i18n/locales/he.json";
import { installMock, uninstallMock } from "../src/mocks/install";
import { ATTITUDE_DEFAULT_HZ, ATTITUDE_MAX_HZ } from "../src/mocks/scenarios";
import type { ScenarioMock } from "../src/mocks/scenarios";
import { apply, ATTITUDE_RATE_HZ, matrixToQuat, NOSE, relativeYaw, RIGHT, sceneMatrix, sceneQuat, smoothingStep, UP, webglAvailable, type Quat, type Vec3 } from "../src/wizard/attitude3d";
import { nav, resetNav } from "../src/wizard/nav";
import { resetWizard, wizard } from "../src/wizard/state";
import { settle } from "./walk";

// three.js can't run in jsdom: replace the lazily imported renderer with a recorder.
const fake = vi.hoisted(() => ({ mode: "ok" as "ok" | "throw", targets: [] as { x: number; y: number; z: number; w: number }[], created: 0, disposed: 0 }));
vi.mock("../src/components/quad3d", () => ({
  createQuadView: () => {
    if (fake.mode === "throw") throw new Error("Error creating WebGL context.");
    fake.created++;
    return {
      setTarget: (q: { x: number; y: number; z: number; w: number }) => fake.targets.push({ ...q }),
      setReducedMotion: () => undefined,
      resize: () => undefined,
      dispose: () => void fake.disposed++,
    };
  },
}));

const near = (a: Vec3, b: Vec3) => a.forEach((x, i) => expect(x).toBeCloseTo(b[i]!, 6));
/** Rotate v by unit quaternion q. */
function rotate(q: Quat, v: Vec3): Vec3 {
  const { x, y, z, w } = q;
  const [vx, vy, vz] = v;
  const tx = 2 * (y * vz - z * vy), ty = 2 * (z * vx - x * vz), tz = 2 * (x * vy - y * vx);
  return [vx + w * tx + (y * tz - z * ty), vy + w * ty + (z * tx - x * tz), vz + w * tz + (x * ty - y * tx)];
}

describe("Euler -> 3D scene rotation", () => {
  it("level: nose away from the viewer (-Z), right side +X, top up", () => {
    const m = sceneMatrix({ roll: 0, pitch: 0, yaw: 0 });
    near(apply(m, NOSE), [0, 0, -1]);
    near(apply(m, RIGHT), [1, 0, 0]);
    near(apply(m, UP), [0, 1, 0]);
  });

  it("MSP pitch +30 (nose DOWN) tips the nose down", () => {
    const e = attitudeToEuler({ roll: 0, pitch: 30, yaw: 0 });
    const nose = apply(sceneMatrix(e), NOSE);
    expect(nose[1]).toBeCloseTo(-0.5, 6); // sin 30° below the horizon
    expect(nose[2]).toBeCloseTo(-Math.cos(Math.PI / 6), 6);
  });

  it("roll +30 (right side down) drops the right side", () => {
    const m = sceneMatrix(attitudeToEuler({ roll: 30, pitch: 0, yaw: 0 }));
    expect(apply(m, RIGHT)[1]).toBeCloseTo(-0.5, 6);
    expect(apply(m, NOSE)[1]).toBeCloseTo(0, 6);
  });

  it("yaw +90 (nose right) points the nose to the right (+X)", () => {
    near(apply(sceneMatrix(attitudeToEuler({ roll: 0, pitch: 0, yaw: 90 })), NOSE), [1, 0, 0]);
    near(apply(sceneMatrix({ roll: 0, pitch: 0, yaw: 270 }), NOSE), [-1, 0, 0]);
  });

  it("the quaternion is the same rotation as the matrix (any attitude)", () => {
    for (const e of [{ roll: 20, pitch: -35, yaw: 120 }, { roll: 170, pitch: 10, yaw: 300 }, { roll: -90, pitch: 80, yaw: 45 }, { roll: 180, pitch: 0, yaw: 180 }]) {
      const m = sceneMatrix(e);
      const q = matrixToQuat(m);
      expect(Math.hypot(q.x, q.y, q.z, q.w)).toBeCloseTo(1, 9);
      for (const v of [NOSE, RIGHT, UP]) near(rotate(q, v), apply(m, v));
    }
  });

  it("heading relative to the 'face forward' reference; smoothing is instant with reduced motion", () => {
    expect(relativeYaw({ roll: 5, pitch: 6, yaw: 30 }, 120)).toEqual({ roll: 5, pitch: 6, yaw: 270 });
    expect(smoothingStep(16, true)).toBe(1);
    const s = smoothingStep(16, false);
    expect(s).toBeGreaterThan(0);
    expect(s).toBeLessThan(1);
    expect(smoothingStep(33, false)).toBeGreaterThan(s);
  });
});

// ---- the step
let mock: ScenarioMock;
let w: VueWrapper | null = null;
const realGetContext = HTMLCanvasElement.prototype.getContext;
let visibility: DocumentVisibilityState = "visible";

function stubWebgl(): void {
  (globalThis as Record<string, unknown>).WebGLRenderingContext = function WebGLRenderingContext() {};
  HTMLCanvasElement.prototype.getContext = (() => ({})) as never;
}

beforeEach(() => {
  resetWizard();
  resetNav();
  resetDemoState();
  setLocale("en");
  fake.mode = "ok";
  fake.targets = [];
  fake.created = 0;
  fake.disposed = 0;
  visibility = "visible";
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => visibility });
});
afterEach(async () => {
  w?.unmount();
  w = null;
  await settle();
  uninstallMock();
  delete (globalThis as Record<string, unknown>).WebGLRenderingContext;
  HTMLCanvasElement.prototype.getContext = realGetContext;
  delete (document as unknown as Record<string, unknown>).visibilityState;
});

async function open(): Promise<VueWrapper> {
  mock = installMock({ scenario: "happy", latencyMs: 0, stepMs: 0 });
  w = mount(App, { global: { plugins: [i18n] }, attachTo: document.body });
  await settle();
  await w.get('[data-test="next"]').trigger("click");
  await w.get('[data-test="connect"]').trigger("click");
  await settle();
  nav.currentId = "calibration";
  await settle();
  return w;
}
const waitFor = async (pred: () => boolean, ms = 1500) => {
  const end = Date.now() + ms;
  while (!pred() && Date.now() < end) {
    await new Promise((r) => setTimeout(r, 20));
    await settle();
  }
  expect(pred()).toBe(true);
};
const view = () => w!.get('[data-test="attitude-view"]');

describe("3D view and fallback", () => {
  it("no WebGL (jsdom): the 2D side / rear drawings, the question still asked", async () => {
    expect(webglAvailable()).toBe(false);
    await open();
    expect(view().attributes("data-view")).toBe("2d");
    expect(w!.find('[data-test="attitude-3d"]').exists()).toBe(false);
    expect(view().findAll("svg")).toHaveLength(2);
    expect(view().text()).toContain(en.calibration.orientation.sideView);
    expect(w!.find('[data-test="attitude-reset-heading"]').exists()).toBe(false);
    expect(w!.find('[data-orientation="yes"]').exists()).toBe(true);
    mock.device.tilt = { roll: 0, pitch: -30, yaw: 0 };
    await waitFor(() => view().attributes("data-direction") === "noseDown");
  });

  it("with WebGL: the 3D quad follows the stream (nose down) and the alignment preview turns it", async () => {
    stubWebgl();
    await open();
    expect(view().attributes("data-view")).toBe("3d");
    await waitFor(() => fake.created === 1 && fake.targets.length > 0);
    expect(w!.find('[data-test="attitude-3d"] canvas').attributes("aria-label")).toBe(en.calibration.orientation.view3d);
    expect(view().findAll("svg")).toHaveLength(0);

    mock.device.tilt = { roll: 0, pitch: -30, yaw: 0 }; // frame nose down 30°
    await waitFor(() => view().attributes("data-direction") === "noseDown");
    await waitFor(() => rotate(fake.targets.at(-1)!, NOSE)[1] < -0.45);

    // Preview "arrow points right" (yaw 90) on a level quad: the 3D nose turns away from -Z.
    mock.device.tilt = { roll: 0, pitch: 0, yaw: 0 };
    await waitFor(() => Math.abs(rotate(fake.targets.at(-1)!, NOSE)[1]) < 0.01);
    await w!.get('[data-orientation="no"]').setValue(true);
    await w!.get('[data-preset="yaw90"]').trigger("click");
    await waitFor(() => Math.abs(rotate(fake.targets.at(-1)!, NOSE)[0]) > 0.99);
    expect(w!.find('[data-orientation="yes"]').exists()).toBe(true);

    nav.currentId = "beepers";
    await settle();
    expect(fake.disposed).toBe(1);
  });

  it("the renderer failing to start (WebGL blocked) falls back to 2D", async () => {
    stubWebgl();
    fake.mode = "throw";
    await open();
    await waitFor(() => view().attributes("data-view") === "2d");
    expect(view().findAll("svg")).toHaveLength(2);
  });

  it("Hebrew: the 3D view stays LTR, with Hebrew text", async () => {
    stubWebgl();
    setLocale("he");
    await open();
    expect(view().attributes("dir")).toBe("ltr");
    await waitFor(() => fake.created === 1);
    expect(w!.get('[data-test="attitude-3d"] canvas').attributes("aria-label")).toBe(he.calibration.orientation.view3d);
    expect(w!.text()).toContain(he.calibration.orientation.resetHeading);
  });
});

describe("attitude stream rate and lifetime", () => {
  it("asks for 30 Hz on entry, stops on leave, pauses while the window is hidden", async () => {
    expect(ATTITUDE_RATE_HZ).toBe(30);
    await open();
    await waitFor(() => mock.device.attitudeTimer !== null);
    expect(mock.device.attitudeRates).toEqual([30]);

    visibility = "hidden";
    document.dispatchEvent(new Event("visibilitychange"));
    await waitFor(() => mock.device.attitudeTimer === null);
    visibility = "visible";
    document.dispatchEvent(new Event("visibilitychange"));
    await waitFor(() => mock.device.attitudeTimer !== null);
    expect(mock.device.attitudeRates).toEqual([30, 30]);

    nav.currentId = "beepers";
    await settle();
    expect(mock.device.attitudeTimer).toBeNull();
    // Not restarted by a visibility change after leaving.
    visibility = "hidden";
    document.dispatchEvent(new Event("visibilitychange"));
    visibility = "visible";
    document.dispatchEvent(new Event("visibilitychange"));
    await settle();
    expect(mock.device.attitudeTimer).toBeNull();
    expect(mock.device.attitudeRates).toEqual([30, 30]);
  });

  it("mock follows Device #94: default 30 Hz, clamped 1..50", async () => {
    mock = installMock({ scenario: "happy", latencyMs: 0, stepMs: 0 });
    await mock.backend("device_connect", { path: "COM5" });
    const rate = async (hz: number | null) => {
      await mock.backend("device_attitude_stream_start", { rateHz: hz });
      await mock.backend("device_attitude_stream_stop", {});
      return mock.device.attitudeHz;
    };
    expect([ATTITUDE_DEFAULT_HZ, ATTITUDE_MAX_HZ]).toEqual([30, 50]);
    expect(await rate(null)).toBe(30);
    expect(await rate(50)).toBe(50);
    expect(await rate(80)).toBe(50);
    expect(await rate(0)).toBe(1);
  });

  it("gaps in seq (latest value only) are fine: the view shows the newest sample", async () => {
    await open();
    await waitFor(() => mock.device.attitudeTimer !== null);
    clearInterval(mock.device.attitudeTimer!); // only the samples below
    mock.events.emit("device://attitude", { seq: 1000, timestampMs: 1, roll: 0, pitch: 30, yaw: 0 }); // nose down
    mock.events.emit("device://attitude", { seq: 1007, timestampMs: 2, roll: 30, pitch: 0, yaw: 0 }); // 6 skipped: right side down
    await settle();
    expect(w!.get('[data-test="attitude-view"]').attributes("data-direction")).toBe("rightDown");
  });

  it("'Face forward again' makes the current heading the forward direction", async () => {
    stubWebgl();
    await open();
    await waitFor(() => fake.targets.length > 0);
    mock.device.tilt = { roll: 0, pitch: 0, yaw: 90 };
    await waitFor(() => rotate(fake.targets.at(-1)!, NOSE)[0] > 0.99);
    await w!.get('[data-test="attitude-reset-heading"]').trigger("click");
    await settle();
    await waitFor(() => rotate(fake.targets.at(-1)!, NOSE)[2] < -0.99);
    expect(wizard.ui.orientationOk).toBeNull();
  });
});

it("sceneQuat matches sceneMatrix", () => {
  const e = { roll: 12, pitch: -7, yaw: 33 };
  near(rotate(sceneQuat(e), NOSE), apply(sceneMatrix(e), NOSE));
});
