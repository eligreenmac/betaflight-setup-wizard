// SPDX-License-Identifier: GPL-3.0-or-later
import { afterEach, describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { ALLOWED_URL_PREFIXES, isAllowedUrl, openBackupsFolder, openExternal, setLinkRuntime } from "../src/api/links";

afterEach(() => setLinkRuntime(null));

describe("links (@tauri-apps/plugin-opener)", () => {
  it("only the scoped hosts are allowed", () => {
    expect(isAllowedUrl("https://betaflight.com/docs/wiki")).toBe(true);
    expect(isAllowedUrl("https://github.com/betaflight/betaflight/issues")).toBe(true);
    expect(isAllowedUrl("https://github.com/eligreenmac/betaflight-setup-wizard/issues")).toBe(true);
    expect(isAllowedUrl("https://github.com/evil/repo")).toBe(false);
    expect(isAllowedUrl("https://betaflight.com.evil.example/")).toBe(false);
  });

  it("allows the USB-serial driver vendors (CP210x, CH34x) and nothing that only looks like them", () => {
    expect(isAllowedUrl("https://www.silabs.com/software-and-tools/usb-to-uart-bridge-vcp-drivers")).toBe(true);
    expect(isAllowedUrl("https://silabs.com/developers/usb-to-uart-bridge-vcp-drivers")).toBe(true);
    expect(isAllowedUrl("https://www.wch-ic.com/downloads/CH341SER_EXE.html")).toBe(true);
    expect(isAllowedUrl("https://wch-ic.com/downloads/CH343SER_EXE.html")).toBe(true);
    expect(isAllowedUrl("http://www.silabs.com/")).toBe(false);
    expect(isAllowedUrl("https://www.silabs.com.evil.example/")).toBe(false);
    expect(isAllowedUrl("https://evil-wch-ic.com/")).toBe(false);
  });

  it("matches the opener capability scope in src-tauri/capabilities/default.json", () => {
    const cap = JSON.parse(readFileSync(resolve(process.cwd(), "src-tauri/capabilities/default.json"), "utf8")) as {
      permissions: (string | { identifier: string; allow?: { url?: string }[] })[];
    };
    const open = cap.permissions.find((p) => typeof p !== "string" && p.identifier === "opener:allow-open-url") as { allow: { url: string }[] };
    expect(open.allow.map((a) => a.url).sort()).toEqual(ALLOWED_URL_PREFIXES.map((p) => `${p}*`).sort());
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
