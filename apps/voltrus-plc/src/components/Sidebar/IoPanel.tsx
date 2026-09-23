import { useState } from "react";
import { invoke } from "../../api/bridge";
import { useStore } from "../../store";
import type { IoStatus, IoReadResult } from "../../types";
import { PROTOCOLS } from "../../constants/protocols";
import {
  acceptEipImport,
  fetchEipUdts,
  filterGeneratedSt,
  type EipImportResult,
} from "../../lib/eipImport";

const IO_PROTOCOLS = PROTOCOLS.filter((p) => p.defaultPort !== undefined);

const QUICK_PRESETS = [
  { label: "ESP32 Tester", host: "192.168.1.100", port: 502, unitId: 1 },
  { label: "Local Simulator", host: "127.0.0.1", port: 502, unitId: 1 },
];

export default function IoPanel() {
  const addLog = useStore((s) => s.addLog);

  const [protocol, setProtocol] = useState("modbus");
  const [host, setHost] = useState("127.0.0.1");
  const [port, setPort] = useState(502);
  const [unitId, setUnitId] = useState(1);
  const [opcuaEndpoint, setOpcuaEndpoint] = useState("");
  const [s7Rack, setS7Rack] = useState(0);
  const [s7Slot, setS7Slot] = useState(1);
  const [mqttPrefix, setMqttPrefix] = useState("");
  const [connecting, setConnecting] = useState(false);
  const [status, setStatus] = useState<IoStatus | null>(null);

  const [testAddr, setTestAddr] = useState(0);
  const [testValue, setTestValue] = useState(0);
  const [readResult, setReadResult] = useState<IoReadResult | null>(null);
  const [testing, setTesting] = useState(false);

  // UDT import (EtherNet/IP only)
  const [eipImportOpen, setEipImportOpen] = useState(false);
  const [eipImportLoading, setEipImportLoading] = useState(false);
  const [eipImportError, setEipImportError] = useState<string | null>(null);
  const [eipImportResult, setEipImportResult] = useState<EipImportResult | null>(null);
  const [eipImportSelected, setEipImportSelected] = useState<Set<string>>(new Set());
  const [eipImportGlob, setEipImportGlob] = useState("*");
  const [eipImportAccepting, setEipImportAccepting] = useState(false);

  const handleProtocolChange = (p: string) => {
    setProtocol(p);
    const proto = IO_PROTOCOLS.find((x) => x.id === p);
    if (proto?.defaultPort !== undefined) setPort(proto.defaultPort);
  };

  const handleConnect = async () => {
    setConnecting(true);
    try {
      const result = await invoke<IoStatus>("configure_io_driver", {
        config: {
          protocol,
          host,
          port,
          unit_id: unitId,
          opcua_endpoint_url: opcuaEndpoint,
          s7_rack: s7Rack,
          s7_slot: s7Slot,
          mqtt_topic_prefix: mqttPrefix,
        },
      });
      setStatus(result);
      if (result.connected) {
        addLog("system", `${protocol.toUpperCase()} connected to ${host}:${port}`);
      } else if (result.error) {
        addLog("system", `${protocol.toUpperCase()} connection failed: ${result.error}`);
      }
    } catch (e) {
      addLog("system", `${protocol.toUpperCase()} error: ${e}`);
    } finally {
      setConnecting(false);
    }
  };

  const handleDisconnect = async () => {
    try {
      await invoke<IoStatus>("disconnect_io_driver");
      setStatus({ connected: false, protocol, host: "", port: 0, unit_id: 0, error: null });
      addLog("system", `${protocol.toUpperCase()} disconnected`);
    } catch (e) {
      addLog("system", `Disconnect error: ${e}`);
    }
  };

  const handleTestRead = async () => {
    setTesting(true);
    try {
      const result = await invoke<IoReadResult>("io_test_read", { address: testAddr });
      setReadResult(result);
      addLog("out", `Read addr[${testAddr}] = ${result.value}`);
    } catch (e) {
      addLog("system", `Read test error: ${e}`);
    } finally {
      setTesting(false);
    }
  };

  const handleTestWrite = async () => {
    setTesting(true);
    try {
      const result = await invoke<IoReadResult>("io_test_write", {
        address: testAddr,
        value: testValue,
      });
      setReadResult(result);
      if (result.success) {
        addLog("in", `Wrote ${testValue} → addr[${testAddr}]`);
      } else {
        addLog("system", `Write failed: ${result.error}`);
      }
    } catch (e) {
      addLog("system", `Write test error: ${e}`);
    } finally {
      setTesting(false);
    }
  };

  const openEipImport = async (glob: string) => {
    setEipImportOpen(true);
    setEipImportLoading(true);
    setEipImportError(null);
    try {
      const result = await fetchEipUdts(host, port, glob);
      setEipImportResult(result);
      setEipImportSelected(new Set(result.tags.map((t) => t.name)));
    } catch (e) {
      setEipImportResult(null);
      setEipImportError(String(e));
    } finally {
      setEipImportLoading(false);
    }
  };

  const handleEipImportAccept = async () => {
    if (!eipImportResult) return;
    setEipImportAccepting(true);
    try {
      await acceptEipImport(eipImportResult, eipImportSelected);
      setEipImportOpen(false);
      setEipImportResult(null);
      setEipImportError(null);
    } catch (e) {
      setEipImportError(String(e));
    } finally {
      setEipImportAccepting(false);
    }
  };

  const toggleEipImportTag = (name: string, checked: boolean) => {
    const next = new Set(eipImportSelected);
    if (checked) next.add(name);
    else next.delete(name);
    setEipImportSelected(next);
  };

  const connected = status?.connected ?? false;

  return (
    <div style={{ padding: 12, fontSize: 12 }}>
      {/* Protocol selector */}
      <div style={{ marginBottom: 8 }}>
        <label style={{ display: "block", color: "var(--text-muted)", fontSize: 11, marginBottom: 2 }}>
          Protocol
        </label>
        <select
          className="scan-cycle-input"
          style={{ width: "100%", textAlign: "left", padding: "2px 6px" }}
          value={protocol}
          onChange={(e) => handleProtocolChange(e.target.value)}
          disabled={connected}
        >
          {IO_PROTOCOLS.map((p) => (
            <option key={p.id} value={p.id}>{p.label}</option>
          ))}
        </select>
      </div>

      {/* Quick presets for Modbus */}
      {protocol === "modbus" && !connected && (
        <div style={{ marginBottom: 8 }}>
          <label style={{ display: "block", color: "var(--text-muted)", fontSize: 11, marginBottom: 2 }}>
            Quick Preset
          </label>
          <div style={{ display: "flex", gap: 4 }}>
            {QUICK_PRESETS.map((p) => (
              <button
                key={p.label}
                className="toolbar-btn"
                style={{ flex: 1, fontSize: 10, padding: "2px 4px" }}
                onClick={() => { setHost(p.host); setPort(p.port); setUnitId(p.unitId); }}
              >
                {p.label}
              </button>
            ))}
          </div>
        </div>
      )}

      {/* Host */}
      <div style={{ marginBottom: 8 }}>
        <label style={{ display: "block", color: "var(--text-muted)", fontSize: 11, marginBottom: 2 }}>
          Host
        </label>
        <input
          className="scan-cycle-input"
          style={{ width: "100%", textAlign: "left" }}
          value={host}
          onChange={(e) => setHost(e.target.value)}
          disabled={connected}
          placeholder="127.0.0.1"
        />
      </div>

      <div style={{ display: "flex", gap: 8, marginBottom: 8 }}>
        <div style={{ flex: 1 }}>
          <label style={{ display: "block", color: "var(--text-muted)", fontSize: 11, marginBottom: 2 }}>
            Port
          </label>
          <input
            className="scan-cycle-input"
            style={{ width: "100%", textAlign: "left" }}
            type="number"
            value={port}
            onChange={(e) => setPort(Number(e.target.value))}
            disabled={connected}
          />
        </div>
        {protocol === "modbus" && (
          <div style={{ flex: 1 }}>
            <label style={{ display: "block", color: "var(--text-muted)", fontSize: 11, marginBottom: 2 }}>
              Unit ID
            </label>
            <input
              className="scan-cycle-input"
              style={{ width: "100%", textAlign: "left" }}
              type="number"
              min={0} max={255}
              value={unitId}
              onChange={(e) => setUnitId(Number(e.target.value))}
              disabled={connected}
            />
          </div>
        )}
      </div>

      {/* OPC-UA specific */}
      {protocol === "opcua" && (
        <div style={{ marginBottom: 8 }}>
          <label style={{ display: "block", color: "var(--text-muted)", fontSize: 11, marginBottom: 2 }}>
            Endpoint URL (leave blank for default)
          </label>
          <input
            className="scan-cycle-input"
            style={{ width: "100%", textAlign: "left" }}
            value={opcuaEndpoint}
            onChange={(e) => setOpcuaEndpoint(e.target.value)}
            disabled={connected}
            placeholder={`opc.tcp://${host}:${port}/`}
          />
        </div>
      )}

      {/* S7 specific */}
      {protocol === "s7" && (
        <div style={{ display: "flex", gap: 8, marginBottom: 8 }}>
          <div style={{ flex: 1 }}>
            <label style={{ display: "block", color: "var(--text-muted)", fontSize: 11, marginBottom: 2 }}>
              Rack
            </label>
            <input
              className="scan-cycle-input"
              style={{ width: "100%", textAlign: "left" }}
              type="number" min={0} max={15}
              value={s7Rack}
              onChange={(e) => setS7Rack(Number(e.target.value))}
              disabled={connected}
            />
          </div>
          <div style={{ flex: 1 }}>
            <label style={{ display: "block", color: "var(--text-muted)", fontSize: 11, marginBottom: 2 }}>
              Slot
            </label>
            <input
              className="scan-cycle-input"
              style={{ width: "100%", textAlign: "left" }}
              type="number" min={0} max={15}
              value={s7Slot}
              onChange={(e) => setS7Slot(Number(e.target.value))}
              disabled={connected}
            />
          </div>
        </div>
      )}

      {/* MQTT specific */}
      {protocol === "mqtt" && (
        <div style={{ marginBottom: 8 }}>
          <label style={{ display: "block", color: "var(--text-muted)", fontSize: 11, marginBottom: 2 }}>
            Topic Prefix
          </label>
          <input
            className="scan-cycle-input"
            style={{ width: "100%", textAlign: "left" }}
            value={mqttPrefix}
            onChange={(e) => setMqttPrefix(e.target.value)}
            disabled={connected}
            placeholder="plc"
          />
        </div>
      )}

      {/* Connection status */}
      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 8 }}>
        <span
          style={{
            width: 8, height: 8, borderRadius: "50%",
            background: connected ? "var(--accent)" : "var(--text-muted)",
            boxShadow: connected ? "0 0 6px var(--accent)" : "none",
            flexShrink: 0,
          }}
        />
        <span style={{ color: connected ? "var(--accent)" : "var(--text-muted)", fontSize: 11 }}>
          {connected ? `Connected ${host}:${port}` : "Disconnected"}
        </span>
      </div>

      {status?.error && (
        <div style={{ color: "var(--danger)", fontSize: 11, marginBottom: 8 }}>
          {status.error}
        </div>
      )}

      <button
        className={`toolbar-btn ${connected ? "danger" : "primary"}`}
        style={{ width: "100%" }}
        onClick={connected ? handleDisconnect : handleConnect}
        disabled={connecting}
      >
        {connecting ? "Connecting..." : connected ? "Disconnect" : "Connect"}
      </button>

      {protocol === "enip" && (
        <button
          className="toolbar-btn"
          style={{ width: "100%", marginTop: 6 }}
          onClick={() => openEipImport(eipImportGlob)}
          disabled={eipImportLoading}
        >
          {eipImportLoading ? "Browsing..." : "Import UDTs from controller…"}
        </button>
      )}

      {connected && (
        <>
          <div style={{ marginTop: 12, color: "var(--text-muted)", fontSize: 11, fontWeight: 600 }}>
            Register Test
          </div>
          <div style={{ display: "flex", gap: 6, marginTop: 6 }}>
            <input
              className="scan-cycle-input"
              style={{ flex: 1, textAlign: "left", padding: "2px 6px" }}
              type="number"
              min={0} max={65535}
              value={testAddr}
              onChange={(e) => setTestAddr(Number(e.target.value))}
              placeholder="Address/Offset"
            />
            <button className="toolbar-btn" onClick={handleTestRead} disabled={testing} style={{ padding: "2px 8px", fontSize: 11 }}>
              Read
            </button>
          </div>
          <div style={{ display: "flex", gap: 6, marginTop: 4 }}>
            <input
              className="scan-cycle-input"
              style={{ flex: 1, textAlign: "left", padding: "2px 6px" }}
              type="number"
              value={testValue}
              onChange={(e) => setTestValue(Number(e.target.value))}
              placeholder="Value"
            />
            <button className="toolbar-btn" onClick={handleTestWrite} disabled={testing} style={{ padding: "2px 8px", fontSize: 11 }}>
              Write
            </button>
          </div>
          {readResult && (
            <div style={{ marginTop: 4, fontSize: 11, color: readResult.success ? "var(--accent)" : "var(--danger)" }}>
              {readResult.success ? `Result: ${readResult.value}` : `Error: ${readResult.error}`}
            </div>
          )}
          {testing && (
            <div style={{ marginTop: 4, fontSize: 11, color: "var(--text-muted)" }}>
              Testing...
            </div>
          )}
        </>
      )}

      {eipImportOpen && (
        <div className="modal-overlay" onClick={() => setEipImportOpen(false)}>
          <div
            className="modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="eip-import-title"
            onClick={(e) => e.stopPropagation()}
            style={{ minWidth: 420, maxWidth: 560 }}
          >
            <h3 id="eip-import-title">Import UDTs from controller</h3>
            {eipImportError && (
              <div style={{ color: "var(--danger)", fontSize: 11, marginBottom: 8, whiteSpace: "pre-wrap" }}>
                {eipImportError}
              </div>
            )}
            {eipImportLoading && (
              <div style={{ color: "var(--text-muted)", fontSize: 11, marginBottom: 8 }}>
                Browsing {host}:{port}...
              </div>
            )}
            {eipImportResult && (
              <>
                <div style={{ display: "flex", gap: 6, marginBottom: 8 }}>
                  <input
                    className="scan-cycle-input"
                    style={{ flex: 1, textAlign: "left", padding: "2px 6px" }}
                    value={eipImportGlob}
                    onChange={(e) => setEipImportGlob(e.target.value)}
                    placeholder="Tag glob, e.g. Motor*"
                  />
                  <button
                    className="toolbar-btn"
                    style={{ padding: "2px 8px", fontSize: 11 }}
                    onClick={() => openEipImport(eipImportGlob)}
                    disabled={eipImportLoading}
                  >
                    Browse
                  </button>
                </div>
                <div style={{ maxHeight: 160, overflowY: "auto", border: "1px solid var(--border)", marginBottom: 8 }}>
                  {eipImportResult.tags.map((t) => (
                    <label
                      key={t.name}
                      style={{ display: "flex", gap: 6, alignItems: "center", padding: "2px 6px", fontSize: 11, cursor: "pointer" }}
                    >
                      <input
                        type="checkbox"
                        checked={eipImportSelected.has(t.name)}
                        onChange={(e) => toggleEipImportTag(t.name, e.target.checked)}
                      />
                      <span>{t.name}</span>
                      <span style={{ color: "var(--text-muted)" }}>
                        {t.template} ({t.members.length} members)
                      </span>
                    </label>
                  ))}
                </div>
                <div style={{ color: "var(--text-muted)", fontSize: 11, marginBottom: 4 }}>
                  Preview — eip_types.st
                </div>
                <pre
                  style={{
                    maxHeight: 180,
                    overflowY: "auto",
                    fontSize: 10,
                    background: "var(--card)",
                    border: "1px solid var(--border)",
                    padding: 8,
                    margin: 0,
                    whiteSpace: "pre-wrap",
                  }}
                >
                  {filterGeneratedSt(eipImportResult.generated_st, eipImportSelected, eipImportResult.tags)}
                </pre>
              </>
            )}
            <div style={{ display: "flex", gap: 8, marginTop: 12 }}>
              <button
                className="toolbar-btn primary"
                style={{ flex: 1 }}
                disabled={!eipImportResult || eipImportSelected.size === 0 || eipImportAccepting}
                onClick={handleEipImportAccept}
              >
                {eipImportAccepting ? "Writing..." : "Accept"}
              </button>
              <button className="toolbar-btn" style={{ flex: 1 }} onClick={() => setEipImportOpen(false)}>
                Cancel
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
