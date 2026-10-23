// SPDX-License-Identifier: GPL-3.0-or-later
import { afterEach, describe, expect, it } from "vitest";
import { isAllowedUrl, openBackupsFolder, openExternal, setLinkRuntime } from "../src/api/links";

afterEach(() => setLinkRuntime(null));

describe("links (@tauri-apps/plugin-opener)", () => {
  it("only the scoped hosts are allowed", () => {
    expect(isAllowedUrl("https://betaflight.com/docs/wiki")).toBe(true);
    expect(isAllowedUrl("https://github.com/betaflight/betaflight/issues")).toBe(true);
    expect(isAllowedUrl("https://github.com/eligreenmac/betaflight-setup-wizard/issues")).toBe(true);
    expect(isAllowedUrl("https://github.com/evil/repo")).toBe(false);
    expect(isAllowedUrl("https://betaflight.com.evil.example/")).toBe(false);
  });

  it("in the app, uses the opener plugin's openUrl", async () => {
    const calls: string[] = [];
    setLinkRuntime({ inApp: () => true, openUrl: async (u) => void calls.push(u) });
    expect(await openExternal("https://betaflight.com/")).toEqual({ opened: true, target: "https://betaflight.com/" });
    expect(calls).toEqual(["https://betaflight.com/"]);
  });

  it("falls back to showing the link when opening fails", async () => {
    setLinkRuntime({ inApp: () => true, openUrl: async () => Promise.reject("not allowed") });
    expect(await openExternal("https://betaflight.com/")).toEqual({ opened: false, target: "https://betaflight.com/" });
  });

  it("never opens a URL outside the scope", async () => {
    const calls: string[] = [];
    setLinkRuntime({ inApp: () => true, openUrl: async (u) => void calls.push(u) });
    expect((await openExternal("https://example.com/")).opened).toBe(false);
    expect(calls).toEqual([]);
  });

  it("backups folder outside the app returns the path to show", async () => {
    setLinkRuntime({ inApp: () => false });
    const r = await openBackupsFolder("abc");
    expect(r.opened).toBe(false);
    expect(r.target).toMatch(/backups\\abc$/);
  });

  it("opens <config_backups_dir>/<boardUid> in the app", async () => {
    const opened: string[] = [];
    setLinkRuntime({
      inApp: () => true,
      backupsDir: async () => "C:\\Users\\p\\AppData\\Roaming\\io.github.eligreenmac.bfsw\\backups",
      join: async (...p) => p.join("\\"),
      openPath: async (p) => void opened.push(p),
    });
    const r = await openBackupsFolder("uid1");
    expect(r).toEqual({ opened: true, target: opened[0] });
    expect(opened[0]).toMatch(/bfsw\\backups\\uid1$/);
  });
});
