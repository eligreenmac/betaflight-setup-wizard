import { afterEach, describe, expect, it } from "vitest";
import { api, resetBackend, setBackend } from "../src/api/commands";
import { createMockBackend } from "../src/api/mock";

describe("api wrapper", () => {
  afterEach(() => resetBackend());

  it("routes calls to the mock backend", async () => {
    setBackend(createMockBackend());
    const fc = await api.device.identify();
    expect(fc.boardName).toBe("SPEEDYBEEF405V4");
  });

  it("rejects unimplemented commands with an AppError shape", async () => {
    setBackend(createMockBackend());
    await expect(api.flasher.listTargets()).rejects.toMatchObject({ code: "NOT_IMPLEMENTED" });
  });
});
