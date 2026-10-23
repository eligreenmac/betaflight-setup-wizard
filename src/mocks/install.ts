// SPDX-License-Identifier: GPL-3.0-or-later
// Wire the scenario mock into every backend seam: commands and events. Used by main.ts in browser demo mode and by UI tests.
import { resetBackend, setBackend } from "../api/commands";
import { resetDeviceBackend, setDeviceBackend } from "../api/device";
import { resetEventSource, setEventSource } from "../api/events";
import { createScenarioMock, type MockOptions, type ScenarioMock } from "./scenarios";

export function installMock(opts: MockOptions = {}): ScenarioMock {
  const mock = createScenarioMock(opts);
  setBackend(mock.backend);
  setDeviceBackend(mock.deviceBackend);
  setEventSource(mock.events.on);
  return mock;
}

export function uninstallMock(): void {
  resetBackend();
  resetDeviceBackend();
  resetEventSource();
}
