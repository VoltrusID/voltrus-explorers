import type { SfcProgram } from "../types/sfc";

export interface SfcSample {
  id: string;
  name: string;
  description: string;
  category: string;
  program: SfcProgram;
}

export const SFC_SAMPLES: SfcSample[] = [
  {
    id: "sfc-traffic-light",
    name: "Traffic Light",
    description: "3-step cycle: Green → Yellow → Red with timer transitions",
    category: "Timer",
    program: {
      version: "1.0",
      programName: "TrafficLight",
      nodes: [
        // Green (initial)
        { id: "s1", type: "initial_step", position: { x: 200, y: 0 }, data: { elementType: "initial_step", stepName: "Green", actions: [{ qualifier: "N", variableName: "GreenLamp" }], pinValues: {} } },
        { id: "t1", type: "transition", position: { x: 200, y: 120 }, data: { elementType: "transition", condition: "T1.Q", pinValues: {} } },
        // Yellow
        { id: "s2", type: "step", position: { x: 200, y: 240 }, data: { elementType: "step", stepName: "Yellow", actions: [{ qualifier: "N", variableName: "YellowLamp" }], pinValues: {} } },
        { id: "t2", type: "transition", position: { x: 200, y: 360 }, data: { elementType: "transition", condition: "T2.Q", pinValues: {} } },
        // Red
        { id: "s3", type: "step", position: { x: 200, y: 480 }, data: { elementType: "step", stepName: "Red", actions: [{ qualifier: "N", variableName: "RedLamp" }], pinValues: {} } },
        { id: "t3", type: "transition", position: { x: 200, y: 600 }, data: { elementType: "transition", condition: "T3.Q", pinValues: {} } },
        // Jump back to Green
        { id: "j1", type: "jump", position: { x: 200, y: 720 }, data: { elementType: "jump", targetStepName: "Green", pinValues: {} } },
      ],
      edges: [
        { id: "e1", source: "s1", target: "t1", sourceHandle: "B", targetHandle: "T" },
        { id: "e2", source: "t1", target: "s2", sourceHandle: "B", targetHandle: "T" },
        { id: "e3", source: "s2", target: "t2", sourceHandle: "B", targetHandle: "T" },
        { id: "e4", source: "t2", target: "s3", sourceHandle: "B", targetHandle: "T" },
        { id: "e5", source: "s3", target: "t3", sourceHandle: "B", targetHandle: "T" },
        { id: "e6", source: "t3", target: "j1", sourceHandle: "B", targetHandle: "T" },
      ],
      viewport: { x: 0, y: 0, zoom: 0.8 },
    },
  },
  {
    id: "sfc-motor-control",
    name: "Motor Control",
    description: "4-step sequence: Idle → Starting → Running → Stopping",
    category: "Logic",
    program: {
      version: "1.0",
      programName: "MotorControl",
      nodes: [
        { id: "s1", type: "initial_step", position: { x: 200, y: 0 }, data: { elementType: "initial_step", stepName: "Idle", actions: [], pinValues: {} } },
        { id: "t1", type: "transition", position: { x: 200, y: 120 }, data: { elementType: "transition", condition: "StartButton", pinValues: {} } },
        { id: "s2", type: "step", position: { x: 200, y: 240 }, data: { elementType: "step", stepName: "Starting", actions: [{ qualifier: "S", variableName: "Motor" }, { qualifier: "P1", variableName: "Counter", expression: "Counter := Counter + 1" }], pinValues: {} } },
        { id: "t2", type: "transition", position: { x: 200, y: 360 }, data: { elementType: "transition", condition: "MotorReady", pinValues: {} } },
        { id: "s3", type: "step", position: { x: 200, y: 480 }, data: { elementType: "step", stepName: "Running", actions: [{ qualifier: "N", variableName: "Motor" }, { qualifier: "N", variableName: "RunLamp" }], pinValues: {} } },
        { id: "t3", type: "transition", position: { x: 200, y: 600 }, data: { elementType: "transition", condition: "StopButton", pinValues: {} } },
        { id: "s4", type: "step", position: { x: 200, y: 720 }, data: { elementType: "step", stepName: "Stopping", actions: [{ qualifier: "R", variableName: "Motor" }], pinValues: {} } },
        { id: "t4", type: "transition", position: { x: 200, y: 840 }, data: { elementType: "transition", condition: "MotorStopped", pinValues: {} } },
        { id: "j1", type: "jump", position: { x: 200, y: 960 }, data: { elementType: "jump", targetStepName: "Idle", pinValues: {} } },
      ],
      edges: [
        { id: "e1", source: "s1", target: "t1", sourceHandle: "B", targetHandle: "T" },
        { id: "e2", source: "t1", target: "s2", sourceHandle: "B", targetHandle: "T" },
        { id: "e3", source: "s2", target: "t2", sourceHandle: "B", targetHandle: "T" },
        { id: "e4", source: "t2", target: "s3", sourceHandle: "B", targetHandle: "T" },
        { id: "e5", source: "s3", target: "t3", sourceHandle: "B", targetHandle: "T" },
        { id: "e6", source: "t3", target: "s4", sourceHandle: "B", targetHandle: "T" },
        { id: "e7", source: "s4", target: "t4", sourceHandle: "B", targetHandle: "T" },
        { id: "e8", source: "t4", target: "j1", sourceHandle: "B", targetHandle: "T" },
      ],
      viewport: { x: 0, y: 0, zoom: 0.6 },
    },
  },
  {
    id: "sfc-batch-process",
    name: "Batch Process",
    description: "4-step batch: Fill → Heat → Stir → Drain with sensor transitions",
    category: "Process",
    program: {
      version: "1.0",
      programName: "BatchProcess",
      nodes: [
        { id: "s1", type: "initial_step", position: { x: 200, y: 0 }, data: { elementType: "initial_step", stepName: "Fill", actions: [{ qualifier: "N", variableName: "FillValve" }], pinValues: {} } },
        { id: "t1", type: "transition", position: { x: 200, y: 120 }, data: { elementType: "transition", condition: "Level >= 80", pinValues: {} } },
        { id: "s2", type: "step", position: { x: 200, y: 240 }, data: { elementType: "step", stepName: "Heat", actions: [{ qualifier: "N", variableName: "Heater" }], pinValues: {} } },
        { id: "t2", type: "transition", position: { x: 200, y: 360 }, data: { elementType: "transition", condition: "Temp >= 60", pinValues: {} } },
        { id: "s3", type: "step", position: { x: 200, y: 480 }, data: { elementType: "step", stepName: "Stir", actions: [{ qualifier: "N", variableName: "StirMotor" }], pinValues: {} } },
        { id: "t3", type: "transition", position: { x: 200, y: 600 }, data: { elementType: "transition", condition: "MixTimer.Q", pinValues: {} } },
        { id: "s4", type: "step", position: { x: 200, y: 720 }, data: { elementType: "step", stepName: "Drain", actions: [{ qualifier: "N", variableName: "DrainValve" }], pinValues: {} } },
        { id: "t4", type: "transition", position: { x: 200, y: 840 }, data: { elementType: "transition", condition: "Level <= 5", pinValues: {} } },
        { id: "j1", type: "jump", position: { x: 200, y: 960 }, data: { elementType: "jump", targetStepName: "Fill", pinValues: {} } },
      ],
      edges: [
        { id: "e1", source: "s1", target: "t1", sourceHandle: "B", targetHandle: "T" },
        { id: "e2", source: "t1", target: "s2", sourceHandle: "B", targetHandle: "T" },
        { id: "e3", source: "s2", target: "t2", sourceHandle: "B", targetHandle: "T" },
        { id: "e4", source: "t2", target: "s3", sourceHandle: "B", targetHandle: "T" },
        { id: "e5", source: "s3", target: "t3", sourceHandle: "B", targetHandle: "T" },
        { id: "e6", source: "t3", target: "s4", sourceHandle: "B", targetHandle: "T" },
        { id: "e7", source: "s4", target: "t4", sourceHandle: "B", targetHandle: "T" },
        { id: "e8", source: "t4", target: "j1", sourceHandle: "B", targetHandle: "T" },
      ],
      viewport: { x: 0, y: 0, zoom: 0.6 },
    },
  },
  {
    id: "sfc-conveyor-branch",
    name: "Conveyor Branch",
    description: "OR divergence: normal path vs jam detection with alarm",
    category: "Branch",
    program: {
      version: "1.0",
      programName: "ConveyorBranch",
      nodes: [
        // Idle (initial)
        { id: "s1", type: "initial_step", position: { x: 200, y: 0 }, data: { elementType: "initial_step", stepName: "Idle", actions: [], pinValues: {} } },
        { id: "t1", type: "transition", position: { x: 200, y: 120 }, data: { elementType: "transition", condition: "StartButton", pinValues: {} } },
        // Running
        { id: "s2", type: "step", position: { x: 200, y: 240 }, data: { elementType: "step", stepName: "Running", actions: [{ qualifier: "N", variableName: "ConveyorMotor" }], pinValues: {} } },
        // OR Divergence
        { id: "od1", type: "or_diverge", position: { x: 200, y: 360 }, data: { elementType: "or_diverge", pinValues: {} } },
        // Path 1: Normal (left)
        { id: "t2", type: "transition", position: { x: 80, y: 480 }, data: { elementType: "transition", condition: "NOT JamSensor", pinValues: {} } },
        // Path 2: Jam detected (right)
        { id: "t3", type: "transition", position: { x: 320, y: 480 }, data: { elementType: "transition", condition: "JamSensor", pinValues: {} } },
        // OR Convergence
        { id: "oc1", type: "or_converge", position: { x: 200, y: 600 }, data: { elementType: "or_converge", pinValues: {} } },
        // After convergence → Alarm step
        { id: "s4", type: "step", position: { x: 200, y: 720 }, data: { elementType: "step", stepName: "Alarm", actions: [{ qualifier: "S", variableName: "AlarmLamp" }], pinValues: {} } },
        { id: "t4", type: "transition", position: { x: 200, y: 840 }, data: { elementType: "transition", condition: "ResetButton", pinValues: {} } },
        { id: "s5", type: "step", position: { x: 200, y: 960 }, data: { elementType: "step", stepName: "Reset", actions: [{ qualifier: "R", variableName: "AlarmLamp" }], pinValues: {} } },
        { id: "t5", type: "transition", position: { x: 200, y: 1080 }, data: { elementType: "transition", condition: "TRUE", pinValues: {} } },
        { id: "j1", type: "jump", position: { x: 200, y: 1200 }, data: { elementType: "jump", targetStepName: "Idle", pinValues: {} } },
      ],
      edges: [
        { id: "e1", source: "s1", target: "t1", sourceHandle: "B", targetHandle: "T" },
        { id: "e2", source: "t1", target: "s2", sourceHandle: "B", targetHandle: "T" },
        { id: "e3", source: "s2", target: "od1", sourceHandle: "B", targetHandle: "T" },
        // OR Diverge → transitions
        { id: "e4", source: "od1", target: "t2", sourceHandle: "B0", targetHandle: "T" },
        { id: "e5", source: "od1", target: "t3", sourceHandle: "B1", targetHandle: "T" },
        // Transitions → OR Converge
        { id: "e6", source: "t2", target: "oc1", sourceHandle: "B", targetHandle: "T0" },
        { id: "e7", source: "t3", target: "oc1", sourceHandle: "B", targetHandle: "T1" },
        // Convergence → Alarm → Reset → Jump
        { id: "e8", source: "oc1", target: "s4", sourceHandle: "B", targetHandle: "T" },
        { id: "e9", source: "s4", target: "t4", sourceHandle: "B", targetHandle: "T" },
        { id: "e10", source: "t4", target: "s5", sourceHandle: "B", targetHandle: "T" },
        { id: "e11", source: "s5", target: "t5", sourceHandle: "B", targetHandle: "T" },
        { id: "e12", source: "t5", target: "j1", sourceHandle: "B", targetHandle: "T" },
      ],
      viewport: { x: 0, y: 0, zoom: 0.5 },
    },
  },
];
