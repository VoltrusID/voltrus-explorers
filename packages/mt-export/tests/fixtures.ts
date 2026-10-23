import type { BacnetTagList, ModbusTagList } from "../src";

/** Every area and type, a unit, a scale, a read-only tag, and two neighbouring tags that fold into one read. */
export const MODBUS_LIST: ModbusTagList = {
  protocol: "modbus",
  source: { name: "PM5560 meter", transport: "tcp", host: "192.168.1.50", port: 502, unitId: 3, pollMs: 1000, wordOrder: "big" },
  tags: [
    { name: "Voltage_3000", area: "holding", address: 3000, type: "float32", scale: 1, unit: "V", description: "L1-N voltage", writable: true },
    { name: "Voltage_3002", area: "holding", address: 3002, type: "float32", scale: 1, unit: "V", writable: true },
    { name: "Counter_10", area: "holding", address: 10, type: "uint16", scale: 1, writable: true },
    { name: "Temperature_20", area: "holding", address: 20, type: "int16", scale: 0.1, unit: "°C", writable: true },
    { name: "Energy_100", area: "input", address: 100, type: "int32", scale: 1, unit: "kWh", writable: false },
    { name: "Flow_200", area: "input", address: 200, type: "float64", scale: 1, writable: false },
    { name: "Run_state_0", area: "coil", address: 0, type: "bool", scale: 1, description: "Run, \"auto\" mode", writable: true },
    { name: "Door_5", area: "discrete", address: 5, type: "bool", scale: 1, writable: false },
  ],
};

export const MODBUS_LITTLE: ModbusTagList = {
  ...MODBUS_LIST,
  source: { ...MODBUS_LIST.source, name: "Inverter", wordOrder: "little", pollMs: 2500 },
  tags: MODBUS_LIST.tags.slice(0, 5),
};

/** Addresses the SCADA's 5-digit references would shift: 30000 and below, and every area above it up to 65535. */
export const MODBUS_HIGH: ModbusTagList = {
  protocol: "modbus",
  source: { name: "Big map", transport: "tcp", host: "10.1.2.3", port: 1502, unitId: 1, pollMs: 2000, wordOrder: "swapped" },
  tags: [
    { name: "Edge_30000", area: "holding", address: 30000, type: "uint16", scale: 1, writable: true },
    { name: "Float_35000", area: "holding", address: 35000, type: "float32", scale: 0.5, unit: "kW", writable: true },
    { name: "Count_40000", area: "holding", address: 40000, type: "int32", scale: 1, writable: true },
    { name: "Input_30001", area: "input", address: 30001, type: "int16", scale: 1, writable: false },
    { name: "Input_65534", area: "input", address: 65534, type: "uint16", scale: 1, writable: false },
    { name: "Coil_40000", area: "coil", address: 40000, type: "bool", scale: 1, writable: true },
    { name: "Discrete_50000", area: "discrete", address: 50000, type: "bool", scale: 1, writable: false },
    { name: "Low_5", area: "holding", address: 5, type: "float64", scale: 1, writable: true },
  ],
};

export const MODBUS_RTU: ModbusTagList = {
  protocol: "modbus",
  source: {
    name: "Pump RTU", transport: "rtu", host: "", port: 502, unitId: 7, pollMs: 500, wordOrder: "swapped",
    serial: { port: "/dev/ttyUSB0", baudRate: 19200, dataBits: 8, stopBits: 1, parity: "even" },
  },
  tags: [
    { name: "Speed_0", area: "holding", address: 0, type: "uint16", scale: 1, writable: true },
    { name: "Power_1", area: "holding", address: 1, type: "float32", scale: 1, writable: true },
  ],
};

export const BACNET_LIST: BacnetTagList = {
  protocol: "bacnet",
  source: { name: "AHU-1", host: "192.168.1.60", port: 47808, deviceId: 1001, pollMs: 5000 },
  tags: [
    { name: "Zone temp", objectType: "analogInput", instance: 1, property: "presentValue", unit: "°C", description: "Zone temperature", writable: false },
    { name: "Fan cmd", objectType: "binaryOutput", instance: 2, property: "presentValue", writable: true },
    { name: "Mode", objectType: "multiStateValue", instance: 3, property: "presentValue", writable: true },
  ],
};

/** The points of `crates/mt-bacnet/examples/sim_headless.rs`, for validating the BACnet flow against a running device. */
export const BACNET_SIM_LIST: BacnetTagList = {
  protocol: "bacnet",
  source: { name: "Sim", host: "127.0.0.1", port: 47808, deviceId: 1234, pollMs: 1000 },
  tags: [
    { name: "Supply temp", objectType: "analogInput", instance: 1, property: "presentValue", unit: "°C", writable: false },
    { name: "Voltage", objectType: "analogValue", instance: 2, property: "presentValue", unit: "V", writable: false },
    { name: "Fan status", objectType: "binaryInput", instance: 3, property: "presentValue", writable: false },
    { name: "Mode", objectType: "multiStateValue", instance: 4, property: "presentValue", writable: false },
    { name: "Valve", objectType: "analogOutput", instance: 5, property: "presentValue", writable: true },
    { name: "Supply temp name", objectType: "analogInput", instance: 1, property: "objectName", writable: false },
  ],
};
