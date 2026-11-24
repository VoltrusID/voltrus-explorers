# ESP32 Protocol Tester

Hardware test target for MacTools industrial protocol desktop apps. Runs on an ESP32 DevKit, providing simulated devices for each protocol.

## Quick Start

```bash
# One-time ESP-IDF setup (if not installed)
git clone -b v5.4.1 --recursive https://github.com/espressif/esp-idf.git ~/esp/esp-idf
cd ~/esp/esp-idf && ./install.sh esp32

# Build & flash
cd firmware/esp32-tester
source ~/esp/esp-idf/export.sh
idf.py menuconfig    # Set WiFi SSID/password, select protocol
idf.py flash monitor
```

## Supported Protocols

| Protocol | Port | Status | MacTools App |
|---|---|---|---|
| Modbus TCP Slave | 502 | Implemented | modbus-poll |
| Modbus TCP Master | 502 | Implemented | modbus-poll |
| Modbus RTU Slave | — (UART) | Implemented | modbus-poll |
| MQTT Client | 1883 | Implemented | mqtt-explorer |
| S7 Server | 102 | Implemented | s7-explorer |
| EtherNet/IP Server | 44818 | Implemented | enip-explorer |
| OPC UA Server | 4840 | Implemented | opcua-explorer |
| BACnet Device | 47808 (UDP) | Implemented | bacnet-explorer |
| DNP3 Outstation | 20000 | Implemented | dnp3-explorer |
| Serial Bridge | — (UART) | Implemented | serial-monitor |

## Switching Protocols

```bash
idf.py menuconfig
# Navigate to: ESP32 Protocol Tester → Select Protocol
# Pick one, then:
idf.py build flash monitor
```

Only one protocol runs at a time. Default: Modbus TCP Slave.

## Protocol Details

### Modbus TCP Slave
- Slave ID: 1 (configurable)
- 100 holding registers (0-99), 100 coils (0-99)
- Register 0: auto-incrementing counter (1s interval)
- Register 1: random value (0-999)
- Coil 0: controls onboard LED (GPIO2)
- Function codes: FC01-FC06, FC0F, FC10

### Modbus TCP Master
- Connects to configurable IP:port
- Polls holding registers every 2s
- Toggles coil 0 on target each cycle

### Modbus RTU Slave
- UART2 via MAX485 transceiver
- 9600-115200 baud (configurable)
- Same register/coil model as TCP Slave

### MQTT Client
- Connects to configurable broker
- Publishes sensor data to `esp32/sensor/#` every 5s
- Subscribes `esp32/control/#` for LED control

### S7 Server
- S7comm on TCP port 102
- Rack/Slot configurable (default: 0/1)
- DB1 with simulated counter and analog values
- ReadVar/WriteVar support

### EtherNet/IP Server
- CIP on TCP port 44818
- Identity object + assembly instances
- ListIdentity, RegisterSession, ForwardOpen

### OPC UA Server
- Binary protocol on TCP port 4840
- Variable nodes with simulated values
- HEL/ACK/OPN/MSG layers

### BACnet Device
- BACnet/IP on UDP port 47808
- Device object (ID: 100)
- Analog Value + Binary Value objects
- ReadProperty, Who-Is/I-Am

### DNP3 Outstation
- TCP port 20000
- Analog + binary input points
- Simulated values

### Serial Bridge
- Raw UART passthrough USB ↔ RS485
- Configurable baud rate
- For serial-monitor testing

## Wiring

### WiFi Protocols (no wiring needed)
All TCP/UDP protocols work over WiFi. No external hardware required.

### RS485 Protocols (MAX485 module needed)

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

| Function | GPIO | Notes |
|---|---|---|
| Onboard LED | 2 | Coil 0 / control topics |
| RS485 TX | 17 | UART2 |
| RS485 RX | 16 | UART2 |
| RS485 DE/RE | 4 | Active-high for transmit |

## Project Structure

```
firmware/esp32-tester/
├── CMakeLists.txt              # Top-level ESP-IDF project
├── main/
│   ├── main.c                  # Entry point: WiFi → protocol launch
│   ├── wifi_station.c/h        # WiFi STA connection manager
│   ├── protocol_launcher.c/h   # Menuconfig-based protocol selector
│   └── Kconfig.projbuild       # Protocol selection + WiFi config
├── components/
│   ├── modbus_tcp_slave/       # Full implementation
│   ├── modbus_tcp_master/      # Full implementation
│   ├── modbus_rtu_slave/       # Full implementation
│   ├── mqtt_client/            # Full implementation
│   ├── s7_server/              # Full implementation
│   ├── enip_server/            # Full implementation
│   ├── opcua_server/           # Full implementation
│   ├── bacnet_device/          # Full implementation
│   ├── dnp3_outstation/        # Full implementation
│   └── serial_bridge/          # Full implementation
└── doc/
    └── wiring.md               # Pin mappings & diagrams
```

## Configuration Reference

All settings available via `idf.py menuconfig` under **ESP32 Protocol Tester**:

| Setting | Default | Description |
|---|---|---|
| WiFi SSID | YOUR_WIFI | Your network name |
| WiFi Password | YOUR_PASSWORD | Your network password |
| Modbus Slave Port | 502 | TCP listen port |
| Modbus Slave ID | 1 | Unit identifier |
| Modbus Master Target IP | 192.168.1.100 | Server to connect to |
| Modbus Master Poll Interval | 2000ms | Register read interval |
| Modbus RTU Baud | 9600 | UART speed |
| MQTT Broker URL | mqtt://broker.hivemq.com:1883 | Broker address |
| S7 Port | 102 | TCP listen port |
| ENIP Port | 44818 | TCP listen port |
| OPC UA Port | 4840 | TCP listen port |
| BACnet Port | 47808 | UDP listen port |
| BACnet Device ID | 100 | Device instance |
| DNP3 Port | 20000 | TCP listen port |
| DNP3 Address | 10 | Outstation address |
| Serial Baud | 9600 | Bridge UART speed |

## Development

```bash
# Add a new protocol component
mkdir -p components/my_protocol/{include,src}
# Create CMakeLists.txt, Kconfig, header, source
# Add to main/CMakeLists.txt REQUIRES list
# Add to main/Kconfig.projbuild choice list
# Add to main/protocol_launcher.c
idf.py build flash monitor
```
