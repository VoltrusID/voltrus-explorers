import type { LdProgram, LdElement } from "../types/ld";
import { makeLdElement, makeLdPath, makeLdRung } from "../types/ld";

export interface LdSample {
  id: string;
  name: string;
  description: string;
  category: string;
  program: LdProgram;
}

function el(type: LdElement["type"], variable: string, overrides?: Partial<LdElement>): LdElement {
  return makeLdElement(type, { variable, ...overrides });
}

export const LD_SAMPLES: LdSample[] = [
  {
    id: "ld-motor-start-stop",
    name: "Motor Start/Stop",
    description: "Self-latching circuit: X0 start, X1 stop, Y0 motor",
    category: "Logic",
    program: {
      version: "2.0",
      programName: "MotorStartStop",
      rungs: [
        makeLdRung([
          makeLdPath([
            el("contact_no", "X0", { registerType: "X" }),
            el("contact_nc", "X1", { registerType: "X" }),
            el("coil", "Y0", { registerType: "Y" }),
          ]),
          makeLdPath([
            el("contact_no", "Y0", { registerType: "Y" }),
            el("contact_nc", "X1", { registerType: "X" }),
            el("coil", "Y0", { registerType: "Y" }),
          ]),
        ]),
      ],
    },
  },
  {
    id: "ld-timer-delay",
    name: "Timer Delay",
    description: "X0 input triggers TON timer, Y0 output after 500ms",
    category: "Timer",
    program: {
      version: "2.0",
      programName: "TimerDelay",
      rungs: [
        makeLdRung([
          makeLdPath([
            el("contact_no", "X0", { registerType: "X" }),
            el("fb_block", "", { blockType: "TON", instanceName: "delay", pinValues: { PT: "T#500MS" } }),
          ]),
        ]),
        makeLdRung([
          makeLdPath([
            el("contact_no", "delay.Q", { registerType: "M" }),
            el("coil", "Y0", { registerType: "Y" }),
          ]),
        ]),
      ],
    },
  },
  {
    id: "ld-counter",
    name: "Counter",
    description: "Count X0 pulses, output Y0 when count reaches 10",
    category: "Counter",
    program: {
      version: "2.0",
      programName: "Counter",
      rungs: [
        makeLdRung([
          makeLdPath([
            el("contact_no", "X0", { registerType: "X" }),
            el("fb_block", "", { blockType: "CTU", instanceName: "cnt", pinValues: { PV: "10", R: "FALSE" } }),
          ]),
        ]),
        makeLdRung([
          makeLdPath([
            el("contact_no", "cnt.Q", { registerType: "M" }),
            el("coil", "Y0", { registerType: "Y" }),
          ]),
        ]),
      ],
    },
  },
  {
    id: "ld-alarm-latch",
    name: "Alarm Latch",
    description: "Sensors X0-X2 trigger SET M0, X3 acknowledges RESET M0",
    category: "Logic",
    program: {
      version: "2.0",
      programName: "AlarmLatch",
      rungs: [
        // Rung 1: X0 OR X1 OR X2 -> SET M0
        makeLdRung([
          makeLdPath([
            el("contact_no", "X0", { registerType: "X" }),
            el("coil_set", "M0"),
          ]),
          makeLdPath([
            el("contact_no", "X1", { registerType: "X" }),
            el("coil_set", "M0"),
          ]),
          makeLdPath([
            el("contact_no", "X2", { registerType: "X" }),
            el("coil_set", "M0"),
          ]),
        ]),
        // Rung 2: M0 -> Y0 (alarm output)
        makeLdRung([
          makeLdPath([
            el("contact_no", "M0"),
            el("coil", "Y0", { registerType: "Y" }),
          ]),
        ]),
        // Rung 3: X3 -> RESET M0
        makeLdRung([
          makeLdPath([
            el("contact_no", "X3", { registerType: "X" }),
            el("coil_reset", "M0"),
          ]),
        ]),
      ],
    },
  },
];
