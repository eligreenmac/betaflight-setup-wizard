import { describe, expect, it } from "vitest";
import { ALL_STEPS, visibleSteps } from "../src/wizard/engine";
import { stepComponents } from "../src/wizard/registry";
import { emptyDroneModel } from "../src/types/model";

describe("wizard engine", () => {
  it("hides optional steps for an empty model", () => {
    const ids = visibleSteps(emptyDroneModel()).map((s) => s.id);
    expect(ids).not.toContain("receiver");
    expect(ids).not.toContain("gps");
    expect(ids).toContain("esc");
  });

  it("shows receiver/vtx/gps steps when selected", () => {
    const m = emptyDroneModel();
    m.receiver.kind = "SERIAL";
    m.vtx.kind = "DIGITAL";
    m.gps.enabled = true;
    const ids = visibleSteps(m).map((s) => s.id);
    expect(ids).toEqual(expect.arrayContaining(["receiver", "vtx", "gps"]));
  });

  it("every step in steps.json has a registered component", () => {
    for (const s of ALL_STEPS) expect(stepComponents[s.component]).toBeDefined();
  });
});
