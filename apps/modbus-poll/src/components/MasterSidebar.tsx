import { invoke } from "@tauri-apps/api/core";
import { ConnectionForm, Badge, Button, Field, IconButton, ProBadge, Section, Segmented, SheetSelect, useIsPhone, useProStatus } from "@mactools/ui";
import { ChevronDown, ChevronUp, FileDown, Pencil, Radar, RefreshCw, ScanSearch, Square, X } from "lucide-react";
import { SaveProfileAsInput } from "../SaveProfileAsInput";
import { ENDIAN_OPTS, VIRTUAL_PORT } from "../constants";
import { useStore } from "../store";

const BAUD_PRESETS = [1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200];

interface Props {
  protocol: {
    connected: boolean;
    loading: boolean;
  };
  masterFormRef: React.RefObject<Record<string, string | number> | null>;
  formKey: number;
  onConnect: (values: Record<string, string | number>) => void;
  onCancelConnect: () => void;
  onDisconnect: () => void;
  onRtuConnect: () => void;
  onScanBus: () => void;
  onSelectScanUnit: (unitId: number) => void;
  onAutoDetect: () => void;
  autoDetecting: boolean;
  /** CSV log of the polled values: the file being written, or null. */
  csvLog: { path: string | null; rows: number };
  onStartCsvLog: () => void;
  onStopCsvLog: () => void;
  saveProfileAs: (name: string) => void;
  saveProfileChanges: () => void;
  loadProfile: (name: string) => void;
  deleteProfile: (name: string) => void;
}

export function MasterSidebar({
  protocol, masterFormRef, formKey,
  onConnect, onCancelConnect, onDisconnect, onRtuConnect, onScanBus, onSelectScanUnit, onAutoDetect, autoDetecting,
  csvLog, onStartCsvLog, onStopCsvLog,
  saveProfileAs, saveProfileChanges, loadProfile, deleteProfile,
}: Props) {
  const {
    setMode, mode, slaveRunning,
    useOneBased, setUseOneBased, displayFormat, setDisplayFormat, endianness, setEndianness,
    scanSubnet, setScanSubnet, scanResults, setScanResults, scanning, setScanning,
    transport, setTransport, tcpFormValues, setTcpFormValues,
    formKey: storeFormKey, setFormKey,
    rtuPort, setRtuPort, rtuPortManual, setRtuPortManual,
    rtuBaud, setRtuBaud, rtuDataBits, setRtuDataBits, rtuStopBits, setRtuStopBits,
    rtuParity, setRtuParity, rtuUnitId, setRtuUnitId, rtuTimeout, setRtuTimeout,
    rtuPollInterval, setRtuPollInterval,
    busScanning, setBusScanning, busScanResults, setBusScanResults,
    showBusScanner, setShowBusScanner, busScanAddr, setBusScanAddr,
    busScanArea, setBusScanArea, busScanStart, setBusScanStart, busScanEnd, setBusScanEnd,
    serialPorts,
    connProfiles, activeConnProfile, setActiveConnProfile, profileDirty,
    addLog, platform,
  } = useStore();

  const { requirePro } = useProStatus();
  const phone = useIsPhone();

  // iOS has no serial ports — Modbus TCP only.
  const isIos = platform === "ios";
  // The in-app virtual RTU slave is a pty pair: macOS only. Windows ports are COMn.
  const hasVirtualPort = platform === "macos";

  return (
    <>
      <Segmented
        label="Mode"
        block
        value={mode}
        onChange={(v) => { if (v === "slave" && !requirePro("Slave simulator")) return; setMode(v); }}
        options={[
          { value: "master", label: "Master" },
          { value: "slave", label: <>Slave <ProBadge /></>, title: slaveRunning ? "Slave server running" : undefined },
        ]}
      />

      {/* Connection (primary task) */}
      <Section title="Connection">
        {!isIos && (
          <div style={{ marginBottom: 10 }}>
            <Segmented
              label="Transport"
              block
              size="sm"
              value={transport}
              onChange={(v) => { if (v === "rtu" && !requirePro("Modbus RTU (serial)")) return; setTransport(v); }}
              options={[
                { value: "tcp", label: "TCP" },
                { value: "rtu", label: <>RTU <ProBadge /></> },
              ]}
            />
          </div>
        )}
        {transport === "tcp" || isIos ? (
          <ConnectionForm
            key={formKey}
            fields={[
              { name: "host", label: "IP address", type: "text", default: "127.0.0.1" },
              { name: "port", label: "Port", type: "number", default: 502 },
              { name: "unitId", label: "Unit ID", type: "number", default: 1 },
              { name: "pollInterval", label: "Poll interval (ms)", type: "number", default: 1000 },
            ]}
            initialValues={tcpFormValues}
            onSubmit={(vals) => { if (masterFormRef.current) masterFormRef.current = vals; onConnect(vals); }}
            onChange={(vals) => setTcpFormValues(vals)}
            onDisconnect={onDisconnect}
            onCancel={onCancelConnect}
            connected={protocol.connected}
            loading={protocol.loading}
          />
        ) : (
          <div className="vt-rows" style={{ display: "flex", flexDirection: "column", gap: 8 }}>
            <Field label="Port">
              <div style={{ display: "flex", gap: 4 }}>
                {rtuPortManual ? (
                  <input
                    className="vt-input vt-input--mono"
                    style={{ flex: 1 }}
                    placeholder={platform === "windows" ? "COM3" : "/dev/cu.usbserial-0001"}
                    value={rtuPort}
                    onChange={(e) => setRtuPort(e.target.value)}
                    disabled={protocol.connected}
                  />
                ) : (
                  <SheetSelect
                    sheetTitle="Serial port"
                    style={{ flex: 1 }}
                    value={rtuPort}
                    onChange={setRtuPort}
                    disabled={protocol.connected}
                    options={[
                      { value: "", label: "Select port…" },
                      ...(hasVirtualPort ? [{ value: VIRTUAL_PORT, label: "In-app virtual slave" }] : []),
                      ...serialPorts.map((p) => ({ value: p, label: p })),
                    ]}
                  />
                )}
                <IconButton
                  label={rtuPortManual ? "Switch to port list" : "Enter port path manually"}
                  size="sm"
                  onClick={() => { setRtuPortManual(!rtuPortManual); setRtuPort(""); }}
                  disabled={protocol.connected}
                >
                  {rtuPortManual ? <ChevronDown size={16} strokeWidth={1.75} /> : <Pencil size={16} strokeWidth={1.75} />}
                </IconButton>
              </div>
            </Field>
            <Field label="Baud rate">
              <div style={{ display: "flex", gap: 4 }}>
                {!BAUD_PRESETS.includes(rtuBaud) ? (
                  <>
                    <input
                      type="number"
                      className="vt-input vt-input--mono"
                      style={{ flex: 1 }}
                      value={rtuBaud || ""}
                      placeholder="Custom baud"
                      onChange={(e) => setRtuBaud(Number(e.target.value))}
                      disabled={protocol.connected || rtuPort === VIRTUAL_PORT}
                    />
                    <IconButton
                      label="Back to preset baud rates"
                      size="sm"
                      onClick={() => setRtuBaud(9600)}
                      disabled={protocol.connected || rtuPort === VIRTUAL_PORT}
                    >
                      <X size={16} strokeWidth={1.75} />
                    </IconButton>
                  </>
                ) : (
                  <SheetSelect
                    sheetTitle="Baud rate"
                    style={{ flex: 1 }}
                    value={rtuBaud}
                    onChange={(v) => setRtuBaud(v === "custom" ? 0 : Number(v))}
                    disabled={protocol.connected || rtuPort === VIRTUAL_PORT}
                    options={[...BAUD_PRESETS.map((b) => ({ value: b as string | number, label: String(b) })), { value: "custom", label: "Custom…" }]}
                  />
                )}
              </div>
            </Field>
            <div className="vt-field-grid" style={{ display: "flex", gap: 4 }}>
              <div style={{ flex: 1 }}><Field stacked label="Data bits">
                <SheetSelect
                  sheetTitle="Data bits"
                  value={rtuDataBits}
                  onChange={setRtuDataBits}
                  disabled={protocol.connected || rtuPort === VIRTUAL_PORT}
                  options={[{ value: 7, label: "7" }, { value: 8, label: "8" }]}
                />
              </Field></div>
              <div style={{ flex: 1 }}><Field stacked label="Stop bits">
                <SheetSelect
                  sheetTitle="Stop bits"
                  value={rtuStopBits}
                  onChange={setRtuStopBits}
                  disabled={protocol.connected || rtuPort === VIRTUAL_PORT}
                  options={[{ value: 1, label: "1" }, { value: 2, label: "2" }]}
                />
              </Field></div>
              <div style={{ flex: 1 }}><Field stacked label="Parity">
                <SheetSelect
                  sheetTitle="Parity"
                  value={rtuParity}
                  onChange={setRtuParity}
                  disabled={protocol.connected || rtuPort === VIRTUAL_PORT}
                  options={[{ value: "none", label: "None" }, { value: "even", label: "Even" }, { value: "odd", label: "Odd" }]}
                />
              </Field></div>
            </div>
            <Field label="Unit ID">
              <input className="vt-input" type="number" min={0} max={247} value={rtuUnitId} onChange={(e) => setRtuUnitId(Number(e.target.value))} disabled={protocol.connected} />
            </Field>
            <div className="vt-field-grid" style={{ display: "flex", gap: 4 }}>
              <div style={{ flex: 1 }}><Field stacked label="Timeout (ms)">
                <input className="vt-input" type="number" value={rtuTimeout} onChange={(e) => setRtuTimeout(Number(e.target.value))} min={100} step={100} disabled={protocol.connected} />
              </Field></div>
              <div style={{ flex: 1 }}><Field stacked label="Poll (ms)">
                <input className="vt-input" type="number" value={rtuPollInterval} onChange={(e) => setRtuPollInterval(Number(e.target.value))} min={50} step={50} disabled={protocol.connected} />
              </Field></div>
            </div>
            <div style={{ display: "flex", gap: 4 }}>
              {!protocol.connected ? (
                <>
                  <Button variant="primary" style={{ flex: 1 }} onClick={onRtuConnect}>Connect RTU</Button>
                  <Button variant="ghost" onClick={onAutoDetect} disabled={protocol.connected || !rtuPort || rtuPort === VIRTUAL_PORT || autoDetecting} icon={<Radar size={14} strokeWidth={1.75} />}>
                    {autoDetecting ? "Detecting…" : "Auto-detect"}
                  </Button>
                </>
              ) : (
                <Button variant="danger" style={{ flex: 1 }} onClick={onDisconnect}>Disconnect</Button>
              )}
            </div>
            {/* Bus Scanner */}
            {protocol.connected && (
              <BusScanner
                busScanning={busScanning}
                busScanResults={busScanResults}
                busScanAddr={busScanAddr}
                busScanArea={busScanArea}
                busScanStart={busScanStart}
                busScanEnd={busScanEnd}
                showBusScanner={showBusScanner}
                onSelectUnit={onSelectScanUnit}
                onScan={onScanBus}
                onSetBusScanAddr={setBusScanAddr}
                onSetBusScanArea={setBusScanArea}
                onSetBusScanStart={setBusScanStart}
                onSetBusScanEnd={setBusScanEnd}
                onToggleShow={() => setShowBusScanner(!showBusScanner)}
              />
            )}
          </div>
        )}
      </Section>

      {/* Logging */}
      <Section title="Log to CSV" meta={<ProBadge />}>
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          {csvLog.path ? (
            <>
              <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-dim)" }}>
                Logging to <span className="mono" title={csvLog.path}>{csvLog.path.split(/[\\/]/).pop()}</span>
                <br />
                {csvLog.rows.toLocaleString()} values written
              </div>
              <Button block size="sm" variant="danger" onClick={onStopCsvLog} icon={<Square size={14} strokeWidth={1.75} />}>Stop logging</Button>
            </>
          ) : (
            <>
              <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
                Writes every polled value of this connection to a CSV file you choose.
              </div>
              <Button block size="sm" onClick={onStartCsvLog} disabled={!protocol.connected} title={protocol.connected ? undefined : "Connect first"} icon={<FileDown size={14} strokeWidth={1.75} />}>
                Start logging…
              </Button>
            </>
          )}
        </div>
      </Section>

      {/* Connection profiles */}
      <Section title="Profiles">
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <SheetSelect
            sheetTitle="Profiles"
            value={activeConnProfile ?? ""}
            onChange={(name) => {
              if (name) loadProfile(name);
              else { setActiveConnProfile(null); }
            }}
            disabled={protocol.connected}
            options={[{ value: "", label: "New profile" }, ...connProfiles.map((p) => ({ value: p.name, label: p.name }))]}
          />

          {activeConnProfile && (
            <div style={{ display: "flex", alignItems: "center", gap: 6, justifyContent: "space-between" }}>
              <span style={{ fontSize: "var(--fs-sm)", fontWeight: 600, color: "var(--text)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {activeConnProfile}
              </span>
              <Badge tone={profileDirty ? "warn" : "success"}>{profileDirty ? "Modified" : "Saved"}</Badge>
            </div>
          )}

          {profileDirty && activeConnProfile ? (
            <div style={{ display: "flex", gap: 4 }}>
              <Button size="sm" variant="primary" style={{ flex: 2 }} onClick={saveProfileChanges}>Save</Button>
              <Button size="sm" style={{ flex: 2 }} onClick={() => { setActiveConnProfile(null); }}>Save as new</Button>
              <Button size="sm" variant="ghost" style={{ flex: 1 }} onClick={() => loadProfile(activeConnProfile)}>Revert</Button>
            </div>
          ) : activeConnProfile && !profileDirty ? (
            <div style={{ display: "flex", gap: 4 }}>
              <Button size="sm" style={{ flex: 1 }} onClick={() => { setActiveConnProfile(null); }}>Save as new</Button>
              <Button size="sm" variant="danger" style={{ flex: 1 }} onClick={() => deleteProfile(activeConnProfile)}>Delete</Button>
            </div>
          ) : (
            <div style={{ display: "flex", gap: 4 }}>
              <SaveProfileAsInput onSave={saveProfileAs} existingNames={connProfiles.map((p) => p.name)} />
            </div>
          )}
        </div>
      </Section>

      {/* Display options */}
      <Section title="Display">
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          <label className="vt-check">
            <input type="checkbox" checked={useOneBased} onChange={(e) => setUseOneBased(e.target.checked)} />
            1-based addresses
          </label>
          {phone ? (
            // A phone picks these from a sheet, like every other choice.
            <>
              <Field label="Value format">
                <SheetSelect
                  sheetTitle="Value format"
                  value={displayFormat}
                  onChange={setDisplayFormat}
                  options={[{ value: "dec", label: "Decimal" }, { value: "hex", label: "Hexadecimal" }, { value: "bin", label: "Binary" }]}
                />
              </Field>
              <Field label="Word order" hint={protocol.connected ? "Disconnect to change the word order." : undefined}>
                <SheetSelect
                  sheetTitle="Word order"
                  value={endianness}
                  onChange={setEndianness}
                  disabled={protocol.connected}
                  options={ENDIAN_OPTS.map((e) => ({ value: e.value, label: e.label, hint: e.hint }))}
                />
              </Field>
            </>
          ) : (
            <>
              <Field stacked label="Value format">
                <Segmented
                  label="Value format"
                  block
                  size="sm"
                  value={displayFormat}
                  onChange={(v) => setDisplayFormat(v)}
                  options={[
                    { value: "dec", label: "Dec" },
                    { value: "hex", label: "Hex" },
                    { value: "bin", label: "Bin" },
                  ]}
                />
              </Field>
              <Field stacked label="Word order">
                <Segmented
                  label="Word order"
                  block
                  size="sm"
                  value={endianness}
                  onChange={(v) => setEndianness(v)}
                  // The order is applied when the connection is made.
                  options={ENDIAN_OPTS.map((e) => ({ value: e.value, label: e.label, disabled: protocol.connected, title: protocol.connected ? "Disconnect to change the word order" : undefined }))}
                />
              </Field>
            </>
          )}
        </div>
      </Section>

      {/* Network scan */}
      <Section title="Network scan" meta={<ProBadge />}>
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <div style={{ display: "flex", gap: 4 }}>
            <input
              className="vt-input vt-input--mono"
              style={{ flex: 1 }}
              placeholder="192.168.1 or 192.168.1.0/24"
              value={scanSubnet}
              onChange={(e) => setScanSubnet(e.target.value)}
            />
            <Button
              size="sm"
              variant="primary"
              onClick={async () => {
                if (!scanSubnet || !requirePro("Network scan")) return;
                setScanning(true);
                setScanResults([]);
                addLog("info", `Scanning ${scanSubnet.trim()} on port 502...`);
                try {
                  const results = await invoke<{ ip: string; port_open: boolean; hostname: string | null }[]>("scan_network", { subnet: scanSubnet, port: 502 });
                  const open = results.filter((r) => r.port_open);
                  setScanResults(results);
                  addLog("info", `Scan complete: ${open.length} of ${results.length} hosts answered on port 502`);
                } catch (e) {
                  addLog("error", `Scan failed: ${String(e)}`);
                }
                setScanning(false);
              }}
              disabled={scanning || !scanSubnet}
              icon={<ScanSearch size={14} strokeWidth={1.75} />}
            >
              {scanning ? "Scanning…" : "Scan"}
            </Button>
          </div>
          {scanResults.filter((r) => r.port_open).length > 0 && (
            <div style={{ display: "flex", flexDirection: "column", gap: 2, maxHeight: 120, overflowY: "auto" }}>
              {scanResults.filter((r) => r.port_open).map((r) => (
                <button
                  key={r.ip}
                  className="vt-pick"
                  onClick={() => { setTcpFormValues({ ...tcpFormValues, host: r.ip }); if (masterFormRef.current) masterFormRef.current.host = r.ip; setFormKey(storeFormKey + 1); addLog("info", `Selected ${r.ip}`); }}
                  style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 8, padding: "4px 8px", borderRadius: "var(--radius-sm)", border: "1px solid var(--border)", background: "transparent", cursor: "pointer", fontSize: "var(--fs-xs)", color: "var(--text)" }}
                >
                  <span className="mono" style={{ color: "var(--accent-text)" }}>{r.ip}</span>
                  <span style={{ color: "var(--text-muted)" }}>{r.hostname || "Unknown"}</span>
                </button>
              ))}
            </div>
          )}
        </div>
      </Section>
    </>
  );
}

// Bus Scanner sub-component
function BusScanner({ busScanning, busScanResults, busScanAddr, busScanArea, busScanStart, busScanEnd, showBusScanner, onSelectUnit, onScan, onSetBusScanAddr, onSetBusScanArea, onSetBusScanStart, onSetBusScanEnd, onToggleShow }: {
  busScanning: boolean;
  busScanResults: { unit_id: number; value: number }[];
  busScanAddr: number;
  busScanArea: string;
  busScanStart: number;
  busScanEnd: number;
  showBusScanner: boolean;
  onSelectUnit: (unitId: number) => void;
  onScan: () => void;
  onSetBusScanAddr: (v: number) => void;
  onSetBusScanArea: (v: string) => void;
  onSetBusScanStart: (v: number) => void;
  onSetBusScanEnd: (v: number) => void;
  onToggleShow: () => void;
}) {
  const { addLog } = useStore();
  const thStyle: React.CSSProperties = { padding: "3px 6px", textAlign: "left" as const, fontSize: "var(--fs-xs)", color: "var(--text-muted)", fontWeight: 600, borderBottom: "1px solid var(--border)" };

  return (
    <div style={{ borderTop: "1px solid var(--border)", paddingTop: 8, marginTop: 4 }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 6 }}>
        <span style={{ fontSize: "var(--fs-xs)", fontWeight: 500, color: "var(--text-dim)" }}>Bus scanner</span>
        <IconButton
          label={showBusScanner ? "Collapse bus scanner" : "Expand bus scanner"}
          size="sm"
          pressed={showBusScanner}
          onClick={onToggleShow}
        >
          {showBusScanner ? <ChevronUp size={16} strokeWidth={1.75} /> : <ChevronDown size={16} strokeWidth={1.75} />}
        </IconButton>
      </div>
      {showBusScanner && (
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <div className="vt-field-grid" style={{ display: "flex", gap: 4 }}>
            <div style={{ flex: 1 }}><Field stacked label="Register address">
              <input className="vt-input" type="number" value={busScanAddr} onChange={(e) => onSetBusScanAddr(Number(e.target.value))} disabled={busScanning} />
            </Field></div>
            <div style={{ flex: 1 }}><Field stacked label="Area">
              <SheetSelect
                sheetTitle="Register area"
                value={busScanArea}
                onChange={onSetBusScanArea}
                disabled={busScanning}
                options={[{ value: "holding", label: "Holding" }, { value: "input", label: "Input" }, { value: "coil", label: "Coil" }, { value: "discrete", label: "Discrete" }]}
              />
            </Field></div>
          </div>
          <div className="vt-field-grid" style={{ display: "flex", gap: 4 }}>
            <div style={{ flex: 1 }}><Field stacked label="Start ID">
              <input className="vt-input" type="number" value={busScanStart} onChange={(e) => onSetBusScanStart(Number(e.target.value))} min={1} max={247} disabled={busScanning} />
            </Field></div>
            <div style={{ flex: 1 }}><Field stacked label="End ID">
              <input className="vt-input" type="number" value={busScanEnd} onChange={(e) => onSetBusScanEnd(Number(e.target.value))} min={1} max={247} disabled={busScanning} />
            </Field></div>
          </div>
          {busScanning ? (
            <Button variant="danger" size="sm" onClick={async () => { try { await invoke("stop_scan_bus", { connId: useStore.getState().activeSessionId }); } catch (e) { addLog("error", `Could not stop the scan: ${String(e)}`); } }}>
              Stop scan
            </Button>
          ) : (
            <Button variant="primary" size="sm" onClick={onScan} icon={<RefreshCw size={14} strokeWidth={1.75} />}>
              Scan bus
            </Button>
          )}
          {busScanResults.length > 0 && (
            <div style={{ fontSize: "var(--fs-xs)", maxHeight: 200, overflowY: "auto", border: "1px solid var(--border)", borderRadius: "var(--radius-sm)" }}>
              <table className="vt-table" style={{ width: "100%", borderCollapse: "collapse" }}>
                <thead>
                  <tr>
                    <th style={thStyle}>ID</th>
                    <th style={thStyle}>Value</th>
                    <th style={thStyle}>Hex</th>
                  </tr>
                </thead>
                <tbody>
                  {busScanResults.map((r) => (
                    <tr key={r.unit_id} onClick={() => onSelectUnit(r.unit_id)} title="Poll this unit ID" style={{ cursor: "pointer" }}>
                      <td style={{ padding: "3px 6px", textAlign: "right", fontSize: "var(--fs-xs)" }}>{r.unit_id}</td>
                      <td style={{ padding: "3px 6px", fontSize: "var(--fs-xs)" }}>{r.value}</td>
                      <td className="mono" style={{ padding: "3px 6px", fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>0x{r.value.toString(16).toUpperCase().padStart(4, "0")}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
