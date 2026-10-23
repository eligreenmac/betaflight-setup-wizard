// SPDX-License-Identifier: GPL-3.0-or-later
// Wire the scenario mock into every backend seam: shared commands, upcoming
// commands and events. Used by main.ts in browser demo mode and by UI tests.
import { resetBackend, setBackend } from "../api/commands";
import { resetEventSource, setEventSource } from "../api/events";
import { resetUpcomingBackend, setUpcomingBackend } from "../api/upcoming";
import { createScenarioMock, type MockOptions, type ScenarioMock } from "./scenarios";

export function installMock(opts: MockOptions = {}): ScenarioMock {
  const mock = createScenarioMock(opts);
  setBackend(mock.backend);
  setUpcomingBackend(mock.upcoming);
  setEventSource(mock.events.on);
  return mock;
}

export function uninstallMock(): void {
  resetBackend();
  resetUpcomingBackend();
  resetEventSource();
}
