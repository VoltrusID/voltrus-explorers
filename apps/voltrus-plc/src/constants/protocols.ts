export interface Protocol {
  id: string;
  label: string;
  defaultPort?: number;
}

export const PROTOCOLS: Protocol[] = [
  { id: "gpio", label: "GPIO" },
  { id: "modbus", label: "Modbus", defaultPort: 502 },
  { id: "opcua", label: "OPC-UA", defaultPort: 4840 },
  { id: "s7", label: "S7", defaultPort: 102 },
  { id: "mqtt", label: "MQTT", defaultPort: 1883 },
  { id: "enip", label: "EtherNet/IP", defaultPort: 44818 },
];
