// SPDX-License-Identifier: GPL-3.0-or-later
import { createApp } from "vue";
import App from "./App.vue";
import { forceDemo, installFallback } from "./api/fallback";
import { isTauriRuntime } from "./api/runtime";
import { applyDirection, i18n } from "./i18n";
import { installMock } from "./mocks/install";
import { createScenarioMock, isScenario } from "./mocks/scenarios";
import "./styles.css";

// Demo mode:
// - outside Tauri (plain browser / `npm run dev`) or when forced, every command and
//   event goes to the scenario mock (pick a scenario with ?mock=<name>);
// - in the app, each command goes to the real backend and falls back to the mock only
//   for commands the backend does not have yet (api/fallback.ts). Either way the
//   "Demo mode" banner shows as soon as anything is simulated.
const q = new URLSearchParams(window.location.search).get("mock") ?? import.meta.env.VITE_MOCK_SCENARIO;
const scenario = isScenario(q) ? q : undefined;
if (import.meta.env.VITE_MOCK_BACKEND === "1" || !isTauriRuntime()) {
  installMock({ scenario, latencyMs: 150, stepMs: 400 });
  forceDemo();
} else {
  const mock = createScenarioMock({ scenario, latencyMs: 150, stepMs: 400 });
  installFallback({ backend: mock.backend, deviceBackend: mock.deviceBackend, events: mock.events.on });
}

applyDirection(i18n.global.locale.value);
createApp(App).use(i18n).mount("#app");
