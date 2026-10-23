// In-memory mock backend for Vitest and browser-only UI development
// (`VITE_MOCK_BACKEND=1 npm run dev`). Extend per workstream as needed.

import type { Backend, CommandMap, CommandName } from "./commands";

type Handlers = { [K in CommandName]?: (args: CommandMap[K][0]) => CommandMap[K][1] | Promise<CommandMap[K][1]> };

export const defaultMockHandlers: Handlers = {
  system_app_info: () => ({ version: "0.1.0-mock", os: "mock", cacheDir: "/tmp" }),
  device_list_ports: () => [
    { path: "COM5", vid: 0x0483, pid: 0x5740, manufacturer: "Betaflight", product: "STM32 VCP", kind: "FC_VCP" },
  ],
  device_connect: () => undefined,
  device_identify: () => ({
    fcVariant: "BTFL", firmwareVersion: "4.5.2", apiVersion: "1.46", boardName: "SPEEDYBEEF405V4",
    manufacturerId: "SPBE", mcu: "STM32F405", uid: null, buildOptions: [],
    serialPorts: [
      { name: "VCP", identifier: 20, functionMask: 1, isConnectionPort: true },
      { name: "UART1", identifier: 0, functionMask: 0, isConnectionPort: false },
      { name: "UART2", identifier: 1, functionMask: 64, isConnectionPort: false },
    ],
    rxSpi: { present: null, expresslrsPins: false, protocol: null, featureEnabled: null, resources: [] },
  }),
  config_read_ports: () => [
    { name: "VCP", identifier: 20, functionMask: 1, isConnectionPort: true },
    { name: "UART1", identifier: 0, functionMask: 0, isConnectionPort: false },
    { name: "UART2", identifier: 1, functionMask: 64, isConnectionPort: false },
    { name: "UART3", identifier: 2, functionMask: 0, isConnectionPort: false },
    { name: "UART4", identifier: 3, functionMask: 0, isConnectionPort: false },
  ],
};

export function createMockBackend(overrides: Handlers = {}): Backend {
  const handlers: Handlers = { ...defaultMockHandlers, ...overrides };
  return (async (cmd: CommandName, args: unknown) => {
    const h = handlers[cmd] as ((a: unknown) => unknown) | undefined;
    if (!h) throw { code: "NOT_IMPLEMENTED", message: `mock: ${cmd}` };
    return h(args);
  }) as Backend;
}
