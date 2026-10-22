// Profiles as the Voltrus console web editor stores them (voltrus-console: src/lib/connect/profile-schema.ts
// `checkProfileData`, CSV from src/lib/connect/profile-csv.ts). Generated, not hand-written: the app tests below
// push each one through the real parsers and sync bookkeeping, so a web edit can never produce a profile an
// app mis-reads or marks "changed on this device". Regenerate when the web schema changes.
export interface WebProfileFixture {
  app: "modbus-poll" | "bacnet-explorer";
  name: string;
  data: Record<string, unknown>;
  /** The CSV the console exports for this profile (blocks for Modbus Poll, watch list for BACnet Explorer). */
  csv: string;
}

export const WEB_PROFILES: WebProfileFixture[] =
[
  {
    "app": "modbus-poll",
    "name": "Pump house, main \"A\" panel – kWh ✓",
    "data": {
      "transport": "tcp",
      "tcp": {
        "host": "10.0.0.5",
        "port": 502,
        "unitId": 1,
        "pollInterval": 1000
      },
      "rtu": {
        "port": "/dev/cu.usbserial-1410",
        "baudRate": 9600,
        "dataBits": 8,
        "stopBits": 1,
        "parity": "none",
        "unitId": 1
      },
      "blocks": [
        {
          "name": "Voltage, L1-L2-L3",
          "address": 2999,
          "quantity": 3,
          "type": "float32",
          "area": "holding"
        },
        {
          "name": "Run \"state\"",
          "address": 10,
          "quantity": 2,
          "type": "uint16",
          "area": "coil"
        },
        {
          "name": "Alarm inputs",
          "address": 0,
          "quantity": 8,
          "type": "uint16",
          "area": "discrete"
        },
        {
          "name": "Counter",
          "address": 100,
          "quantity": 1,
          "type": "int32",
          "area": "input"
        },
        {
          "name": "Flow",
          "address": 200,
          "quantity": 2,
          "type": "float64",
          "area": "holding"
        },
        {
          "name": "Temp x10",
          "address": 300,
          "quantity": 1,
          "type": "int16",
          "area": "holding",
          "scale": 0.1
        }
      ]
    },
    "csv": "name,address,quantity,type,area\n\"Voltage, L1-L2-L3\",2999,3,float32,holding\n\"Run \"\"state\"\"\",10,2,uint16,coil\nAlarm inputs,0,8,uint16,discrete\nCounter,100,1,int32,input\nFlow,200,2,float64,holding\nTemp x10,300,1,int16,holding\n"
  },
  {
    "app": "modbus-poll",
    "name": "Boiler RTU",
    "data": {
      "transport": "rtu",
      "tcp": {
        "host": "192.168.0.9",
        "port": 1502,
        "unitId": 3,
        "pollInterval": 250
      },
      "rtu": {
        "port": "/dev/ttyUSB0",
        "baudRate": 38400,
        "dataBits": 7,
        "stopBits": 2,
        "parity": "even",
        "unitId": 17
      },
      "blocks": [
        {
          "name": "Setpoint",
          "address": 40,
          "quantity": 1,
          "type": "uint16",
          "area": "holding",
          "register_area": "holding",
          "vendor": "x"
        }
      ]
    },
    "csv": "name,address,quantity,type,area\nSetpoint,40,1,uint16,holding\n"
  },
  {
    "app": "modbus-poll",
    "name": "Empty profile",
    "data": {
      "transport": "tcp",
      "tcp": {
        "host": "10.0.0.5",
        "port": 502,
        "unitId": 1,
        "pollInterval": 1000
      },
      "rtu": {
        "port": "/dev/cu.usbserial-1410",
        "baudRate": 9600,
        "dataBits": 8,
        "stopBits": 1,
        "parity": "none",
        "unitId": 1
      },
      "blocks": []
    },
    "csv": "name,address,quantity,type,area\n"
  },
  {
    "app": "bacnet-explorer",
    "name": "AHU-1 supply air",
    "data": {
      "device": {
        "ip": "192.168.1.20",
        "port": 47808,
        "device_id": 1001
      },
      "watch": [
        {
          "type": "analogInput",
          "instance": 1,
          "name": "Supply air temp",
          "source": "poll"
        },
        {
          "type": "binaryValue",
          "instance": 4,
          "name": "Fan, \"status\"",
          "source": "cov"
        },
        {
          "type": "multiStateValue",
          "instance": 4194302,
          "name": "Mode ✓",
          "source": "poll"
        }
      ]
    },
    "csv": "type,instance,name,source\nanalogInput,1,Supply air temp,poll\nbinaryValue,4,\"Fan, \"\"status\"\"\",cov\nmultiStateValue,4194302,Mode ✓,poll\n"
  },
  {
    "app": "bacnet-explorer",
    "name": "Bare device",
    "data": {
      "device": {
        "ip": "10.0.0.1",
        "port": 47809,
        "device_id": 0
      },
      "watch": []
    },
    "csv": "type,instance,name,source\n"
  }
]
;
