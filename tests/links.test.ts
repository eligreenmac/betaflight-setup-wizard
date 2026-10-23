// SPDX-License-Identifier: GPL-3.0-or-later
import { afterEach, describe, expect, it } from "vitest";
import { isAllowedUrl, openBackupsFolder, openExternal, setLinkRuntime } from "../src/api/links";

afterEach(() => setLinkRuntime(null));

describe("links (opener plugin optional)", () => {
  it("only the scoped hosts are allowed", () => {
    expect(isAllowedUrl("https://betaflight.com/docs/wiki")).toBe(true);
    expect(isAllowedUrl("https://github.com/betaflight/betaflight/issues")).toBe(true);
    expect(isAllowedUrl("https://github.com/eligreenmac/betaflight-setup-wizard/issues")).toBe(true);
    expect(isAllowedUrl("https://github.com/evil/repo")).toBe(false);
    expect(isAllowedUrl("https://betaflight.com.evil.example/")).toBe(false);
  });

  it("in the app, uses plugin:opener|open_url", async () => {
    const calls: [string, unknown][] = [];
    setLinkRuntime({ inApp: () => true, invoke: async (c, a) => void calls.push([c, a]) });
    expect(await openExternal("https://betaflight.com/")).toEqual({ opened: true, target: "https://betaflight.com/" });
    expect(calls).toEqual([["plugin:opener|open_url", { url: "https://betaflight.com/" }]]);
  });

  it("falls back to showing the link when the plugin/capability is missing", async () => {
    setLinkRuntime({ inApp: () => true, invoke: async () => Promise.reject("plugin opener not found") });
    expect(await openExternal("https://betaflight.com/")).toEqual({ opened: false, target: "https://betaflight.com/" });
  });

  it("never opens a URL outside the scope", async () => {
    const calls: string[] = [];
    setLinkRuntime({ inApp: () => true, invoke: async (c) => void calls.push(c) });
    expect((await openExternal("https://example.com/")).opened).toBe(false);
    expect(calls).toEqual([]);
  });

  it("backups folder outside the app returns the path to show", async () => {
    setLinkRuntime({ inApp: () => false });
    const r = await openBackupsFolder("abc");
    expect(r.opened).toBe(false);
    expect(r.target).toMatch(/backups\\abc$/);
  });
});
