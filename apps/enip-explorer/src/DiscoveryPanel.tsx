import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Radar } from "lucide-react";
import { Button, DataTable, Dialog, Field, noteDeviceError, noteDeviceSuccess, type Column } from "@mactools/ui";

export interface EnipDevice {
  host: string;
  port: number;
  vendor_id: number;
  vendor_name: string;
  device_type: number;
  product_code: number;
  revision_major: number;
  revision_minor: number;
  status: number;
  serial_number: number;
  product_name: string;
  state: number;
}

const columns: Column[] = [
  { key: "host", label: "IP : Port", width: "150px" },
  { key: "product_name", label: "Product", width: "180px" },
  { key: "vendor_name", label: "Vendor", width: "160px" },
  { key: "device_type", label: "Type", width: "70px" },
  { key: "revision", label: "Rev", width: "70px" },
  { key: "serial", label: "Serial", width: "120px" },
];

interface Props {
  onClose: () => void;
  onConnect: (host: string, port: number) => void;
}

export function DiscoveryPanel({ onClose, onConnect }: Props) {
  const [devices, setDevices] = useState<EnipDevice[]>([]);
  const [scanning, setScanning] = useState(false);
  const [timeoutMs, setTimeoutMs] = useState(3000);

  const scan = async () => {
    setScanning(true);
    setDevices([]);
    try {
      const found = await invoke<EnipDevice[]>("discover_devices", { timeoutMs });
      found.sort((a, b) => a.host.localeCompare(b.host, undefined, { numeric: true }));
      setDevices(found);
      if (found.length > 0) noteDeviceSuccess();
    } catch {
      noteDeviceError();
    } finally {
      setScanning(false);
    }
  };

  useEffect(() => {
    scan();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const rows = devices.map((d) => ({
    host: `${d.host}:${d.port}`,
    product_name: d.product_name || "—",
    vendor_name: d.vendor_name || `Vendor ${d.vendor_id}`,
    device_type: `0x${d.device_type.toString(16).toUpperCase()}`,
    revision: `${d.revision_major}.${d.revision_minor}`,
    serial: serialHex(d.serial_number),
  }));

  return (
    <Dialog open onClose={onClose} title="Discover EtherNet/IP devices" width={760}>
      <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        <div style={{ display: "flex", alignItems: "end", gap: 8 }}>
          <Field label="Timeout (ms)">
            <input
              className="vt-input"
              style={{ width: 90 }}
              type="number"
              min={500}
              step={500}
              value={timeoutMs}
              onChange={(e) => setTimeoutMs(Number(e.target.value))}
              disabled={scanning}
            />
          </Field>
          <Button
            variant="primary"
            icon={<Radar size={14} />}
            loading={scanning}
            onClick={scan}
            style={{ marginBottom: 2 }}
          >
            {scanning ? "Scanning" : "Rescan"}
          </Button>
          <span style={{ marginLeft: "auto", fontSize: 11, color: "var(--text-muted)", alignSelf: "center" }}>
            {devices.length} device{devices.length === 1 ? "" : "s"} found
          </span>
        </div>

        <div style={{ height: 380, display: "flex", flexDirection: "column" }}>
          <DataTable
            columns={columns}
            data={rows}
            emptyMessage={scanning ? "Listening for ListIdentity replies" : "No devices. Check subnet and firewall (UDP 44818 broadcast)."}
            onRowClick={(row) => {
              const hp = String(row.host).split(":");
              onConnect(hp[0], Number(hp[1] ?? 44818));
            }}
          />
        </div>

        <div style={{ fontSize: 11, color: "var(--text-muted)", lineHeight: 1.5 }}>
          Click a device to connect. Broadcasts a ListIdentity (ENIP 0x0063) to UDP port 44818 on every network interface.
        </div>
      </div>
    </Dialog>
  );
}

function serialHex(n: number): string {
  return `0x${(n >>> 0).toString(16).padStart(8, "0").toUpperCase()}`;
}
