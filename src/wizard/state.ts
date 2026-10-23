// Shared reactive wizard state (one instance per app). Steps read/write `model`.

import { reactive } from "vue";
import { emptyDroneModel, type DroneModel, type FcInfo, type FcSerialPort } from "../types/model";

export interface WizardState {
  model: DroneModel;
  fc: FcInfo | null;
  ports: FcSerialPort[];
  connectedPath: string | null;
}

export const wizard = reactive<WizardState>({
  model: emptyDroneModel(),
  fc: null,
  ports: [],
  connectedPath: null,
});
