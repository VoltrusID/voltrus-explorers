import type { FbdProgram } from "../types/fbd";

export interface FbdSample {
  id: string;
  name: string;
  description: string;
  category: string;
  program: FbdProgram;
}

export const FBD_SAMPLES: FbdSample[] = [
  {
    id: "motor-start-stop",
    name: "Motor Start/Stop",
    description: "Self-latching motor circuit with start/stop buttons",
    category: "Logic",
    program: {
      version: "1.0",
      programName: "MotorStartStop",
      declarations: [],
      nodes: [
        { id: "node-1", type: "input_pin", position: { x: 0, y: 0 }, data: { blockType: "INPUT_PIN", label: "Input", registerType: "X", registerNumber: 0, variableName: "X0", variableType: "BOOL", pinValues: {} } },
        { id: "node-2", type: "input_pin", position: { x: 0, y: 120 }, data: { blockType: "INPUT_PIN", label: "Input", registerType: "X", registerNumber: 1, variableName: "X1", variableType: "BOOL", pinValues: {} } },
        { id: "node-3", type: "input_pin", position: { x: 0, y: 240 }, data: { blockType: "INPUT_PIN", label: "Input", registerType: "M", registerNumber: 0, variableName: "M0", variableType: "BOOL", pinValues: {} } },
        { id: "node-4", type: "block", position: { x: 250, y: 50 }, data: { blockType: "OR", label: "OR", pinValues: {} } },
        { id: "node-5", type: "block", position: { x: 500, y: 100 }, data: { blockType: "AND", label: "AND", pinValues: {} } },
        { id: "node-6", type: "output_pin", position: { x: 750, y: 100 }, data: { blockType: "OUTPUT_PIN", label: "Output", registerType: "Y", registerNumber: 0, variableName: "Y0", variableType: "BOOL", pinValues: {} } },
      ],
      edges: [
        { id: "e1", source: "node-1", target: "node-4", sourceHandle: "OUT", targetHandle: "IN1", data: { sourcePin: "OUT", targetPin: "IN1" } },
        { id: "e2", source: "node-3", target: "node-4", sourceHandle: "OUT", targetHandle: "IN2", data: { sourcePin: "OUT", targetPin: "IN2" } },
        { id: "e3", source: "node-4", target: "node-5", sourceHandle: "OUT", targetHandle: "IN1", data: { sourcePin: "OUT", targetPin: "IN1" } },
        { id: "e4", source: "node-2", target: "node-5", sourceHandle: "OUT", targetHandle: "IN2", data: { sourcePin: "OUT", targetPin: "IN2" } },
        { id: "e5", source: "node-5", target: "node-6", sourceHandle: "OUT", targetHandle: "IN", data: { sourcePin: "OUT", targetPin: "IN" } },
      ],
      viewport: { x: 0, y: 0, zoom: 1 },
    },
  },
  {
    id: "timer-delay",
    name: "Timer Delay",
    description: "TON timer: output activates after input is TRUE for 500ms",
    category: "Timer",
    program: {
      version: "1.0",
      programName: "TimerDelay",
      declarations: [],
      nodes: [
        { id: "node-1", type: "input_pin", position: { x: 0, y: 0 }, data: { blockType: "INPUT_PIN", label: "Input", registerType: "X", registerNumber: 0, variableName: "X0", variableType: "BOOL", pinValues: {} } },
        { id: "node-2", type: "timer", position: { x: 250, y: 0 }, data: { blockType: "TON", label: "TON", instanceName: "delay_timer", pinValues: { PT: "T#500MS" } } },
        { id: "node-3", type: "output_pin", position: { x: 500, y: 0 }, data: { blockType: "OUTPUT_PIN", label: "Output", registerType: "Y", registerNumber: 0, variableName: "Y0", variableType: "BOOL", pinValues: {} } },
      ],
      edges: [
        { id: "e1", source: "node-1", target: "node-2", sourceHandle: "OUT", targetHandle: "IN", data: { sourcePin: "OUT", targetPin: "IN" } },
        { id: "e2", source: "node-2", target: "node-3", sourceHandle: "Q", targetHandle: "IN", data: { sourcePin: "Q", targetPin: "IN" } },
      ],
      viewport: { x: 0, y: 0, zoom: 1 },
    },
  },
  {
    id: "counter",
    name: "Counter",
    description: "CTU counter: counts X0 pulses, resets with X1, outputs at count 10",
    category: "Counter",
    program: {
      version: "1.0",
      programName: "Counter",
      declarations: [],
      nodes: [
        { id: "node-1", type: "input_pin", position: { x: 0, y: 0 }, data: { blockType: "INPUT_PIN", label: "Input", registerType: "X", registerNumber: 0, variableName: "X0", variableType: "BOOL", pinValues: {} } },
        { id: "node-2", type: "input_pin", position: { x: 0, y: 100 }, data: { blockType: "INPUT_PIN", label: "Input", registerType: "X", registerNumber: 1, variableName: "X1", variableType: "BOOL", pinValues: {} } },
        { id: "node-3", type: "counter", position: { x: 250, y: 30 }, data: { blockType: "CTU", label: "CTU", instanceName: "part_counter", pinValues: { PV: "10" } } },
        { id: "node-4", type: "output_pin", position: { x: 500, y: 30 }, data: { blockType: "OUTPUT_PIN", label: "Output", registerType: "Y", registerNumber: 0, variableName: "Y0", variableType: "BOOL", pinValues: {} } },
      ],
      edges: [
        { id: "e1", source: "node-1", target: "node-3", sourceHandle: "OUT", targetHandle: "CU", data: { sourcePin: "OUT", targetPin: "CU" } },
        { id: "e2", source: "node-2", target: "node-3", sourceHandle: "OUT", targetHandle: "R", data: { sourcePin: "OUT", targetPin: "R" } },
        { id: "e3", source: "node-3", target: "node-4", sourceHandle: "Q", targetHandle: "IN", data: { sourcePin: "Q", targetPin: "IN" } },
      ],
      viewport: { x: 0, y: 0, zoom: 1 },
    },
  },
  {
    id: "threshold-alarm",
    name: "Threshold Alarm",
    description: "ADD + GT: alarm when D0 + D1 > 100",
    category: "Arithmetic",
    program: {
      version: "1.0",
      programName: "ThresholdAlarm",
      declarations: [],
      nodes: [
        { id: "node-1", type: "input_pin", position: { x: 0, y: 0 }, data: { blockType: "INPUT_PIN", label: "Input", registerType: "D", registerNumber: 0, variableName: "D0", variableType: "INT", pinValues: {} } },
        { id: "node-2", type: "input_pin", position: { x: 0, y: 100 }, data: { blockType: "INPUT_PIN", label: "Input", registerType: "D", registerNumber: 1, variableName: "D1", variableType: "INT", pinValues: {} } },
        { id: "node-3", type: "block", position: { x: 250, y: 30 }, data: { blockType: "ADD", label: "ADD", pinValues: {} } },
        { id: "node-4", type: "block", position: { x: 500, y: 60 }, data: { blockType: "GT", label: "GT", pinValues: { IN2: "100" } } },
        { id: "node-5", type: "output_pin", position: { x: 750, y: 60 }, data: { blockType: "OUTPUT_PIN", label: "Output", registerType: "Y", registerNumber: 0, variableName: "Y0", variableType: "BOOL", pinValues: {} } },
      ],
      edges: [
        { id: "e1", source: "node-1", target: "node-3", sourceHandle: "OUT", targetHandle: "IN1", data: { sourcePin: "OUT", targetPin: "IN1" } },
        { id: "e2", source: "node-2", target: "node-3", sourceHandle: "OUT", targetHandle: "IN2", data: { sourcePin: "OUT", targetPin: "IN2" } },
        { id: "e3", source: "node-3", target: "node-4", sourceHandle: "OUT", targetHandle: "IN1", data: { sourcePin: "OUT", targetPin: "IN1" } },
        { id: "e4", source: "node-4", target: "node-5", sourceHandle: "OUT", targetHandle: "IN", data: { sourcePin: "OUT", targetPin: "IN" } },
      ],
      viewport: { x: 0, y: 0, zoom: 1 },
    },
  },
  {
    id: "edge-detect",
    name: "Edge Detect",
    description: "R_TRIG: one-shot pulse on rising edge of X0",
    category: "Edge",
    program: {
      version: "1.0",
      programName: "EdgeDetect",
      declarations: [],
      nodes: [
        { id: "node-1", type: "input_pin", position: { x: 0, y: 0 }, data: { blockType: "INPUT_PIN", label: "Input", registerType: "X", registerNumber: 0, variableName: "X0", variableType: "BOOL", pinValues: {} } },
        { id: "node-2", type: "block", position: { x: 250, y: 0 }, data: { blockType: "R_TRIG", label: "R_TRIG", instanceName: "trig_1", pinValues: {} } },
        { id: "node-3", type: "output_pin", position: { x: 500, y: 0 }, data: { blockType: "OUTPUT_PIN", label: "Output", registerType: "M", registerNumber: 0, variableName: "M0", variableType: "BOOL", pinValues: {} } },
      ],
      edges: [
        { id: "e1", source: "node-1", target: "node-2", sourceHandle: "OUT", targetHandle: "CLK", data: { sourcePin: "OUT", targetPin: "CLK" } },
        { id: "e2", source: "node-2", target: "node-3", sourceHandle: "Q", targetHandle: "IN", data: { sourcePin: "Q", targetPin: "IN" } },
      ],
      viewport: { x: 0, y: 0, zoom: 1 },
    },
  },
];
