# ESP32 Protocol Tester — Wiring Guide

## Pin Assignments

| Function | GPIO | Connect To | Notes |
|---|---|---|---|
| Onboard LED | 2 | Internal | Modbus coil 0 controls this |
| RS485 TX | 17 | MAX485 DI | UART2 |
| RS485 RX | 16 | MAX485 RO | UART2 |
| RS485 DE/RE | 4 | MAX485 DE + RE | Tied together, active-high for TX |

## MAX485 Wiring (for Modbus RTU / Serial Bridge)

```
ESP32 DevKit          MAX485 Module
─────────────         ─────────────
GPIO17 (TX2) ──────── DI
GPIO16 (RX2) ──────── RO
GPIO4        ──────── DE + RE
3V3          ──────── VCC
GND          ──────── GND
                       A  ──── RS485 Bus (+)
                       B  ──── RS485 Bus (-)
```

## Protocols by Connection Type

### WiFi only (no wiring needed)
- Modbus TCP Slave (port 502)
- Modbus TCP Master
- MQTT Client
- S7 Server (port 102)
- EtherNet/IP Server (port 44818)
- OPC UA Server (port 4840)
- BACnet Device (UDP port 47808)
- DNP3 Outstation (port 20000)

### RS485 required (MAX485 module)
- Modbus RTU Slave
- Serial Bridge

## Quick Start

1. Flash firmware: `idf.py flash monitor`
2. Set WiFi credentials: `idf.py menuconfig` → WiFi Configuration
3. Default protocol: Modbus TCP Slave on port 502
4. Test with MacTools modbus-poll app → connect to ESP32's IP
