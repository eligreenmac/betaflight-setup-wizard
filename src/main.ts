// SPDX-License-Identifier: GPL-3.0-or-later
import { createApp } from "vue";
import App from "./App.vue";
import { isTauriRuntime } from "./api/runtime";
import { applyDirection, i18n } from "./i18n";
import { installMock } from "./mocks/install";
import { isScenario } from "./mocks/scenarios";
import "./styles.css";

// Demo mode: outside Tauri (plain browser / `npm run dev`) or when forced, every
// command and event goes to the scenario mock. Pick a scenario with ?mock=<name>.
const forced = import.meta.env.VITE_MOCK_BACKEND === "1";
const demo = forced || !isTauriRuntime();
if (demo) {
  const q = new URLSearchParams(window.location.search).get("mock") ?? import.meta.env.VITE_MOCK_SCENARIO;
  installMock({ scenario: isScenario(q) ? q : undefined, latencyMs: 150, stepMs: 400 });
}

applyDirection(i18n.global.locale.value);
createApp(App, { demo }).use(i18n).mount("#app");
