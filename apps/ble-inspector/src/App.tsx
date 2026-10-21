import { useState, useCallback, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  Badge,
  Button,
  ProAboutRow,
  ProBadge,
  useProStatus,
  Dialog,
  EmptyState,
  LogPanel,
  Section,
  StatusBar,
  Tabs,
  Toolbar,
} from "@mactools/ui";
import type { LogEntry } from "@mactools/ui";
import { BluetoothSearching, ChevronDown, ChevronRight, Radar } from "lucide-react";
import appIcon from "../src-tauri/icons/128x128.png";

interface BleDevice {
  id: string;
  name: string;
  rssi: number;
  address: string;
  connected: boolean;
}

interface BleService {
  uuid: string;
  is_primary: boolean;
  characteristics: BleCharacteristic[];
}

interface BleCharacteristic {
  uuid: string;
  properties: {
    read: boolean;
    write: boolean;
    write_without_response: boolean;
    notify: boolean;
    indicate: boolean;
  };
  value: string | null;
}

const SIDEBAR_WIDTH = 300;

/** Rough RSSI-to-bars mapping for the compact signal readout. */
function signalBars(rssi: number): number {
  if (rssi >= -60) return 4;
  if (rssi >= -75) return 3;
  if (rssi >= -88) return 2;
  return 1;
}

function SignalMeter({ rssi }: { rssi: number }) {
  const bars = signalBars(rssi);
  return (
    <span
      title={`${rssi} dBm`}
      style={{ display: "inline-flex", flexDirection: "column", alignItems: "flex-end", gap: 2, flexShrink: 0 }}
    >
      <span aria-hidden="true" style={{ display: "inline-flex", alignItems: "flex-end", gap: 2 }}>
        {[4, 7, 10, 13].map((height, i) => (
          <span
            key={height}
            style={{ width: 3, height, borderRadius: 1, background: i < bars ? "var(--text-dim)" : "var(--sunken)" }}
          />
        ))}
      </span>
      <span
        style={{
          fontFamily: "var(--font-mono)",
          fontVariantNumeric: "tabular-nums",
          fontSize: "var(--fs-xs)",
          color: "var(--text-muted)",
        }}
      >
        {rssi} dBm
      </span>
    </span>
  );
}

function DeviceRow({
  device,
  selected,
  onConnect,
}: {
  device: BleDevice;
  selected: boolean;
  onConnect: (device: BleDevice) => void;
}) {
  const label = device.name || "(unknown)";
  const address = device.address || device.id.substring(0, 17);
  return (
    <button
      type="button"
      className="ble-device"
      aria-pressed={selected}
      onClick={() => onConnect(device)}
      title={`Connect to ${label}`}
    >
      <span style={{ display: "flex", alignItems: "center", gap: 6 }}>
        <span
          style={{
            flex: 1,
            minWidth: 0,
            fontSize: "var(--fs-sm)",
            fontWeight: 500,
            color: "var(--text)",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {label}
        </span>
        {device.connected && <Badge tone="success">Connected</Badge>}
      </span>
      <span style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 2 }}>
        <span
          className="mono"
          style={{
            fontSize: "var(--fs-xs)",
            color: "var(--text-muted)",
            flex: 1,
            minWidth: 0,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {address}
        </span>
        <SignalMeter rssi={device.rssi} />
      </span>
    </button>
  );
}

function App() {
  const [log, setLog] = useState<LogEntry[]>([]);
  const [scanning, setScanning] = useState(false);
  const [devices, setDevices] = useState<BleDevice[]>([]);
  const [selectedDevice, setSelectedDevice] = useState<BleDevice | null>(null);
  const [services, setServices] = useState<BleService[]>([]);
  const [expandedServices, setExpandedServices] = useState<Set<string>>(new Set());
  const [selectedChar, setSelectedChar] = useState<BleCharacteristic | null>(null);
  const [writeValue, setWriteValue] = useState("");
  const [showInfo, setShowInfo] = useState(false);
  const [isMobile, setIsMobile] = useState(window.innerWidth < 768);
  const [mobileTab, setMobileTab] = useState<"scan" | "device" | "log">("scan");
  const { requirePro, status: proStatus } = useProStatus();

  const addLog = useCallback((direction: LogEntry["direction"], message: string) => {
    setLog((prev) => [...prev.slice(-499), { timestamp: Date.now(), direction, message }]);
  }, []);

  useEffect(() => {
    const onResize = () => setIsMobile(window.innerWidth < 768);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  // Poll scanning state and device list
  useEffect(() => {
    const interval = setInterval(async () => {
      try {
        const isActive = await invoke<boolean>("is_scanning");
        setScanning(isActive);
        const devs = await invoke<BleDevice[]>("get_devices");
        setDevices(devs);
      } catch {}
    }, 500);
    return () => clearInterval(interval);
  }, []);

  // Listen for BLE notification events from backend
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    (async () => {
      unlisten = await listen<{ type: string; device_id: string; char_uuid: string; value: string }>(
        "ble-event",
        (event) => {
          const payload = event.payload;
          if (payload.type === "characteristic_notify") {
            addLog("rx", `Notify ${payload.char_uuid}: ${payload.value}`);
          }
        },
      );
    })();
    return () => { unlisten?.(); };
  }, [addLog]);

  const handleStartScan = useCallback(async () => {
    try {
      await invoke("start_scan");
      setScanning(true);
      addLog("tx", "BLE scan started");
    } catch (e) {
      addLog("error", `Scan start failed: ${String(e)}`);
    }
  }, [addLog]);

  const handleStopScan = useCallback(async () => {
    try {
      await invoke("stop_scan");
      setScanning(false);
      addLog("rx", "BLE scan stopped");
    } catch (e) {
      addLog("error", `Scan stop failed: ${String(e)}`);
    }
  }, [addLog]);

  const handleConnect = useCallback(async (device: BleDevice) => {
    try {
      await invoke("connect_device", { id: device.id });
      setSelectedDevice({ ...device, connected: true });
      addLog("info", `Connected to ${device.name || device.id}`);
      const svcs = await invoke<BleService[]>("discover_services", { id: device.id });
      setServices(svcs);
      setExpandedServices(svcs.length > 0 ? new Set([svcs[0].uuid]) : new Set());
    } catch (e) {
      addLog("error", `Connect failed: ${String(e)}`);
    }
  }, [addLog]);

  const handleDisconnect = useCallback(async () => {
    if (!selectedDevice) return;
    try {
      await invoke("disconnect_device", { id: selectedDevice.id });
      setSelectedDevice(null);
      setServices([]);
      setExpandedServices(new Set());
      setSelectedChar(null);
      addLog("info", `Disconnected from ${selectedDevice.name}`);
    } catch (e) {
      addLog("error", `Disconnect failed: ${String(e)}`);
    }
  }, [selectedDevice, addLog]);

  const handleReadChar = useCallback(async (char: BleCharacteristic) => {
    if (!selectedDevice) return;
    try {
      const value = await invoke<string>("read_characteristic", {
        deviceId: selectedDevice.id,
        charUuid: char.uuid,
      });
      setSelectedChar({ ...char, value });
      addLog("rx", `Read ${char.uuid}: ${value}`);
    } catch (e) {
      addLog("error", `Read failed: ${String(e)}`);
    }
  }, [selectedDevice, addLog]);

  const handleWriteChar = useCallback(async () => {
    if (!selectedDevice || !selectedChar || !writeValue) return;
    if (!requirePro("Write characteristics")) return;
    try {
      await invoke("write_characteristic", {
        deviceId: selectedDevice.id,
        charUuid: selectedChar.uuid,
        value: writeValue,
      });
      addLog("tx", `Write to ${selectedChar.uuid}: ${writeValue}`);
      setWriteValue("");
    } catch (e) {
      addLog("error", `Write failed: ${String(e)}`);
    }
  }, [selectedDevice, selectedChar, writeValue, addLog, requirePro]);

  const handleSubscribe = useCallback(async (char: BleCharacteristic) => {
    if (!selectedDevice) return;
    if (!requirePro("Notifications")) return;
    try {
      await invoke("subscribe_characteristic", {
        deviceId: selectedDevice.id,
        charUuid: char.uuid,
      });
      addLog("info", `Subscribed to notifications on ${char.uuid}`);
    } catch (e) {
      addLog("error", `Subscribe failed: ${String(e)}`);
    }
  }, [selectedDevice, addLog, requirePro]);

  // UUID shortener
  const shortUuid = (uuid: string) => {
    if (uuid.length === 36) {
      return `0x${uuid.substring(4, 8).toUpperCase()}`;
    }
    return uuid;
  };

  const toggleService = (uuid: string) => {
    setExpandedServices((prev) => {
      const next = new Set(prev);
      if (next.has(uuid)) {
        next.delete(uuid);
      } else {
        next.add(uuid);
      }
      return next;
    });
  };

  const scanButton = !scanning ? (
    <Button
      variant="primary"
      block
      icon={<Radar size={14} strokeWidth={1.75} />}
      onClick={handleStartScan}
    >
      Start scan
    </Button>
  ) : (
    <Button
      variant="danger"
      block
      icon={<span className="vt-spinner" aria-hidden="true" />}
      onClick={handleStopScan}
    >
      Stop scan
    </Button>
  );

  const deviceCountLabel = `${devices.length} device${devices.length === 1 ? "" : "s"} found`;

  const deviceList = (compact: boolean) =>
    devices.length === 0 ? (
      <EmptyState
        compact
        title="No devices found"
        description={
          scanning
            ? "Scanning is running. Devices appear here as they respond."
            : "Start a scan to discover nearby Bluetooth LE devices."
        }
      />
    ) : (
      <div
        style={
          compact
            ? { display: "flex", flexDirection: "column", gap: 4 }
            : { display: "flex", flexDirection: "column", gap: 4, flex: 1, minHeight: 0, overflowY: "auto" }
        }
      >
        {devices.map((d) => (
          <DeviceRow
            key={d.id}
            device={d}
            selected={selectedDevice?.id === d.id}
            onConnect={handleConnect}
          />
        ))}
      </div>
    );

  /** Shared GATT services/characteristics tree (desktop and mobile device view). */
  const renderServices = (compact: boolean) => (
    <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
      {services.map((svc) => {
        const open = expandedServices.has(svc.uuid);
        return (
          <div key={svc.uuid} className="vt-panel">
            <button
              type="button"
              className="ble-svc"
              aria-expanded={open}
              onClick={() => toggleService(svc.uuid)}
              title={svc.uuid}
            >
              {open ? (
                <ChevronDown size={14} strokeWidth={1.75} aria-hidden="true" />
              ) : (
                <ChevronRight size={14} strokeWidth={1.75} aria-hidden="true" />
              )}
              <span style={{ fontSize: "var(--fs-sm)", fontWeight: 600, color: "var(--text)" }}>Service</span>
              <span className="mono" style={{ color: "var(--text-dim)" }}>{shortUuid(svc.uuid)}</span>
              <span style={{ marginLeft: "auto", fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
                {svc.is_primary ? "Primary" : "Secondary"}
              </span>
            </button>
            {open && (
              <div>
                {svc.characteristics.map((ch) => (
                  <div key={ch.uuid} className="ble-char">
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div
                        className="mono"
                        title={ch.uuid}
                        style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
                      >
                        {ch.uuid}
                      </div>
                      <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)", marginTop: 2 }}>
                        {[
                          ch.properties.read && "Read",
                          ch.properties.write && "Write",
                          ch.properties.write_without_response && "Write without response",
                          ch.properties.notify && "Notify",
                          ch.properties.indicate && "Indicate",
                        ]
                          .filter(Boolean)
                          .join(" · ")}
                      </div>
                      {ch.value && (
                        <div
                          className="mono"
                          style={{ color: "var(--text-dim)", marginTop: 2, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
                        >
                          {ch.value}
                        </div>
                      )}
                    </div>
                    <div style={{ display: "flex", alignItems: "center", gap: 4, flexShrink: 0 }}>
                      {ch.properties.read && (
                        <Button size="sm" onClick={() => handleReadChar(ch)}>
                          Read
                        </Button>
                      )}
                      {ch.properties.notify && (
                        <Button size="sm" onClick={() => handleSubscribe(ch)}>
                          Notify <ProBadge />
                        </Button>
                      )}
                      {!compact && ch.properties.write && (
                        <>
                          <input
                            className="vt-input vt-input--sm vt-input--mono"
                            style={{ width: 110 }}
                            placeholder="Hex value"
                            aria-label={`Value to write to ${ch.uuid}`}
                            value={selectedChar?.uuid === ch.uuid ? writeValue : ""}
                            onChange={(e) => {
                              setSelectedChar(ch);
                              setWriteValue(e.target.value);
                            }}
                            onKeyDown={(e) => {
                              if (e.key === "Enter") handleWriteChar();
                            }}
                          />
                          <Button
                            size="sm"
                            onClick={() => {
                              setSelectedChar(ch);
                              setTimeout(() => handleWriteChar(), 0);
                            }}
                          >
                            Write <ProBadge />
                          </Button>
                        </>
                      )}
                    </div>
                  </div>
                ))}
                {svc.characteristics.length === 0 && (
                  <div style={{ padding: "0 10px 10px 32px", fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
                    No characteristics
                  </div>
                )}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );

  const deviceHeader = (
    <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start", gap: 12 }}>
      <div style={{ minWidth: 0 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span
            style={{
              fontSize: "var(--fs-lg)",
              fontWeight: 600,
              color: "var(--text)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {selectedDevice?.name || "Unknown device"}
          </span>
          <Badge tone="success">Connected</Badge>
        </div>
        <div className="mono" style={{ color: "var(--text-muted)", marginTop: 2 }}>
          {selectedDevice?.address || selectedDevice?.id}
        </div>
      </div>
      <Button variant="danger" size="sm" onClick={handleDisconnect}>
        Disconnect
      </Button>
    </div>
  );

  const isConnected = selectedDevice?.connected ?? false;

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh", background: "var(--bg)", color: "var(--text)" }}>
      <Toolbar
        title="BLE Inspector"
        icon={appIcon}
        connected={isConnected}
        onInfo={() => setShowInfo(true)}
      />

      {isMobile ? (
        <div style={{ flex: 1, overflow: "hidden", display: "flex", flexDirection: "column" }}>
          <div style={{ flex: 1, overflowY: "auto", padding: 16, minHeight: 0 }}>
            {mobileTab === "scan" && (
              <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
                {scanButton}
                <Section title="Devices" meta={deviceCountLabel}>
                  {deviceList(true)}
                </Section>
              </div>
            )}
            {mobileTab === "device" &&
              (selectedDevice ? (
                <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
                  {deviceHeader}
                  {services.length === 0 ? (
                    <EmptyState
                      compact
                      title="No services discovered"
                      description="Discovery may still be running, or the device exposes no GATT services."
                    />
                  ) : (
                    renderServices(true)
                  )}
                </div>
              ) : (
                <EmptyState
                  compact
                  title="No device selected"
                  description="Connect to a device to inspect its GATT services."
                />
              ))}
            {mobileTab === "log" && (
              <LogPanel entries={log} beforeExport={() => requirePro("Log export")} onExport={(f) => addLog("info", `Log exported to ${f}`)} onClear={() => setLog([])} />
            )}
          </div>
          <nav
            style={{
              padding: "6px 12px",
              paddingBottom: "calc(6px + env(safe-area-inset-bottom, 0px))",
              background: "var(--surface)",
              borderTop: "1px solid var(--border)",
              flexShrink: 0,
            }}
          >
            <Tabs
              value={mobileTab}
              onChange={setMobileTab}
              label="Mobile sections"
              className="ble-mtabs"
              items={[
                { value: "scan", label: "Scan" },
                { value: "device", label: "Device" },
                { value: "log", label: "Log" },
              ]}
            />
          </nav>
        </div>
      ) : (
        <div style={{ display: "flex", flex: 1, overflow: "hidden" }}>
          <aside
            className="vt-sidebar"
            style={{ width: SIDEBAR_WIDTH, flexShrink: 0, borderRight: "1px solid var(--border)" }}
          >
            <Section title="Scanner">
              {scanButton}
              <p className="vt-hint" style={{ margin: 0 }}>
                {scanning ? "Scanning for nearby devices" : "Discover Bluetooth LE devices nearby"}
              </p>
            </Section>
            <Section title="Devices" meta={deviceCountLabel} style={{ flex: 1, minHeight: 0 }}>
              {deviceList(false)}
            </Section>
          </aside>

          <main style={{ display: "flex", flex: 1, flexDirection: "column", overflow: "hidden", minWidth: 0 }}>
            <div style={{ flex: 1, overflowY: "auto", padding: 16, minHeight: 0 }}>
              {selectedDevice ? (
                <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
                  {deviceHeader}
                  {services.length === 0 ? (
                    <EmptyState
                      compact
                      title="No services discovered"
                      description="Discovery may still be running, or the device exposes no GATT services."
                    />
                  ) : (
                    renderServices(false)
                  )}
                </div>
              ) : (
                <div style={{ display: "flex", alignItems: "center", justifyContent: "center", minHeight: "100%" }}>
                  <EmptyState
                    icon={<BluetoothSearching size={20} strokeWidth={1.75} aria-hidden="true" />}
                    title="No device selected"
                    description="Start a scan and click a device to connect and inspect its GATT services."
                  />
                </div>
              )}
            </div>
            <LogPanel entries={log} beforeExport={() => requirePro("Log export")} onExport={(f) => addLog("info", `Log exported to ${f}`)} onClear={() => setLog([])} />
          </main>
        </div>
      )}

      {!isMobile && (
        <StatusBar connected={isConnected}>
          <span className="vt-statusbar__item">{scanning ? "Scanning" : deviceCountLabel}</span>
        </StatusBar>
      )}
      <Dialog open={showInfo} onClose={() => setShowInfo(false)} title="BLE Inspector" footer={
        <>
          {/* No outbound source link in App Store builds: review treats links to builds that unlock paid
              features elsewhere as steering away from in-app purchase. */}
          {proStatus && !proStatus.store && (
            <Button variant="ghost" onClick={() => invoke("open_url", { url: "https://github.com/VoltrusID/voltrus-explorers" })}>
              Source on GitHub
            </Button>
          )}
          <Button onClick={() => setShowInfo(false)}>Close</Button>
        </>
      }>
        <div style={{ paddingTop: 12 }}>
          <p style={{ margin: "0 0 12px", fontSize: "var(--fs-md)", color: "var(--text)" }}>
            Bluetooth Low Energy device scanner and GATT inspector.
          </p>
          {[
            ["Scan", "Discover nearby BLE devices with signal strength (RSSI)."],
            ["Connect", "Connect to devices and browse their GATT services and characteristics."],
            ["Inspect", "Read characteristic values for free. Pro adds writing values, notification subscriptions and log export."],
          ].map(([heading, text]) => (
            <div key={heading} style={{ marginBottom: 10 }}>
              <div style={{ fontSize: "var(--fs-sm)", fontWeight: 600, color: "var(--text)" }}>{heading}</div>
              <div style={{ fontSize: "var(--fs-sm)", color: "var(--text-dim)" }}>{text}</div>
            </div>
          ))}
          <p style={{ margin: 0, fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
            Part of the <strong style={{ color: "var(--accent-text)" }}>Voltrus</strong> suite by JoyoDigitama
          </p>
          <div style={{ marginTop: 12 }}>
            <ProAboutRow />
          </div>
        </div>
      </Dialog>
    </div>
  );
}

export default App;
