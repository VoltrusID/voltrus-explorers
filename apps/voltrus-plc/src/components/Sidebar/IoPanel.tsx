import { useEffect, useState } from "react";
import { invoke } from "../../api/bridge";
import { useStore } from "../../store";
import { Button, Field, StatusDot } from "@mactools/ui";
import { Plug, Unplug } from "lucide-react";
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

  // Close the UDT import dialog on Escape.
  useEffect(() => {
    if (!eipImportOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setEipImportOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [eipImportOpen]);

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
    <div style={{ display: "flex", flexDirection: "column", gap: 10, fontSize: "var(--fs-sm)" }}>
      <Field label="Protocol" htmlFor="io-protocol">
        <select
          id="io-protocol"
          className="vt-select"
          value={protocol}
          onChange={(e) => handleProtocolChange(e.target.value)}
          disabled={connected}
        >
          {IO_PROTOCOLS.map((p) => (
            <option key={p.id} value={p.id}>{p.label}</option>
          ))}
        </select>
      </Field>

      {/* Quick presets for Modbus */}
      {protocol === "modbus" && !connected && (
        <Field label="Quick preset">
          <div style={{ display: "flex", gap: 4 }}>
            {QUICK_PRESETS.map((p) => (
              <Button
                key={p.label}
                size="sm"
                variant="ghost"
                style={{ flex: 1 }}
                onClick={() => { setHost(p.host); setPort(p.port); setUnitId(p.unitId); }}
              >
                {p.label}
              </Button>
            ))}
          </div>
        </Field>
      )}

      <Field label="Host" htmlFor="io-host">
        <input
          id="io-host"
          className="vt-input vt-input--mono"
          value={host}
          onChange={(e) => setHost(e.target.value)}
          disabled={connected}
          placeholder="127.0.0.1"
        />
      </Field>

      <div className="vt-field-row">
        <Field label="Port" htmlFor="io-port">
          <input
            id="io-port"
            className="vt-input"
            type="number"
            value={port}
            onChange={(e) => setPort(Number(e.target.value))}
            disabled={connected}
          />
        </Field>
        {protocol === "modbus" && (
          <Field label="Unit ID" htmlFor="io-unit">
            <input
              id="io-unit"
              className="vt-input"
              type="number"
              min={0} max={255}
              value={unitId}
              onChange={(e) => setUnitId(Number(e.target.value))}
              disabled={connected}
            />
          </Field>
        )}
      </div>

      {/* OPC-UA specific */}
      {protocol === "opcua" && (
        <Field label="Endpoint URL (leave blank for default)" htmlFor="io-endpoint">
          <input
            id="io-endpoint"
            className="vt-input vt-input--mono"
            value={opcuaEndpoint}
            onChange={(e) => setOpcuaEndpoint(e.target.value)}
            disabled={connected}
            placeholder={`opc.tcp://${host}:${port}/`}
          />
        </Field>
      )}

      {/* S7 specific */}
      {protocol === "s7" && (
        <div className="vt-field-row">
          <Field label="Rack" htmlFor="io-rack">
            <input
              id="io-rack"
              className="vt-input"
              type="number" min={0} max={15}
              value={s7Rack}
              onChange={(e) => setS7Rack(Number(e.target.value))}
              disabled={connected}
            />
          </Field>
          <Field label="Slot" htmlFor="io-slot">
            <input
              id="io-slot"
              className="vt-input"
              type="number" min={0} max={15}
              value={s7Slot}
              onChange={(e) => setS7Slot(Number(e.target.value))}
              disabled={connected}
            />
          </Field>
        </div>
      )}

      {/* MQTT specific */}
      {protocol === "mqtt" && (
        <Field label="Topic prefix" htmlFor="io-topic-prefix">
          <input
            id="io-topic-prefix"
            className="vt-input vt-input--mono"
            value={mqttPrefix}
            onChange={(e) => setMqttPrefix(e.target.value)}
            disabled={connected}
            placeholder="plc"
          />
        </Field>
      )}

      {/* Connection status */}
      <div style={{ display: "flex", alignItems: "center", gap: 8, fontSize: "var(--fs-xs)" }}>
        <StatusDot tone={connected ? "success" : "neutral"} label={connected ? "Connected" : "Disconnected"} />
        <span style={{ color: connected ? "var(--accent-text)" : "var(--text-muted)", fontFamily: "var(--font-mono)" }}>
          {connected ? `${host}:${port}` : "Disconnected"}
        </span>
      </div>

      {status?.error && (
        <div className="vt-callout vt-callout--danger" role="alert">
          {status.error}
        </div>
      )}

      <Button
        variant={connected ? "danger" : "primary"}
        block
        icon={connected ? <Unplug size={14} strokeWidth={1.75} /> : <Plug size={14} strokeWidth={1.75} />}
        loading={connecting}
        onClick={connected ? handleDisconnect : handleConnect}
        disabled={connecting}
      >
        {connecting ? "Connecting..." : connected ? "Disconnect" : "Connect"}
      </Button>

      {protocol === "enip" && (
        <Button
          variant="default"
          block
          onClick={() => openEipImport(eipImportGlob)}
          disabled={eipImportLoading}
        >
          {eipImportLoading ? "Browsing..." : "Import UDTs from controller"}
        </Button>
      )}

      {connected && (
        <Field label="Register test">
          <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
            <div style={{ display: "flex", gap: 6 }}>
              <input
                className="vt-input vt-input--sm vt-input--mono"
                type="number"
                min={0} max={65535}
                value={testAddr}
                onChange={(e) => setTestAddr(Number(e.target.value))}
                placeholder="Address"
                aria-label="Test address"
              />
              <Button size="sm" variant="default" onClick={handleTestRead} disabled={testing}>
                Read
              </Button>
            </div>
            <div style={{ display: "flex", gap: 6 }}>
              <input
                className="vt-input vt-input--sm vt-input--mono"
                type="number"
                value={testValue}
                onChange={(e) => setTestValue(Number(e.target.value))}
                placeholder="Value"
                aria-label="Test value"
              />
              <Button size="sm" variant="default" onClick={handleTestWrite} disabled={testing}>
                Write
              </Button>
            </div>
            {readResult && (
              <div style={{ fontSize: "var(--fs-xs)", color: readResult.success ? "var(--accent-text)" : "var(--danger)", fontFamily: "var(--font-mono)" }}>
                {readResult.success ? `Result: ${readResult.value}` : `Error: ${readResult.error}`}
              </div>
            )}
            {testing && (
              <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
                Testing...
              </div>
            )}
          </div>
        </Field>
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
              <div className="vt-callout vt-callout--danger" role="alert" style={{ marginBottom: 8, whiteSpace: "pre-wrap" }}>
                {eipImportError}
              </div>
            )}
            {eipImportLoading && (
              <div style={{ color: "var(--text-muted)", fontSize: "var(--fs-xs)", marginBottom: 8, fontFamily: "var(--font-mono)" }}>
                Browsing {host}:{port}...
              </div>
            )}
            {eipImportResult && (
              <>
                <div style={{ display: "flex", gap: 6, marginBottom: 8 }}>
                  <input
                    className="vt-input"
                    value={eipImportGlob}
                    onChange={(e) => setEipImportGlob(e.target.value)}
                    placeholder="Tag glob, e.g. Motor*"
                    aria-label="Tag glob filter"
                  />
                  <Button
                    size="md"
                    variant="default"
                    onClick={() => openEipImport(eipImportGlob)}
                    disabled={eipImportLoading}
                  >
                    Browse
                  </Button>
                </div>
                <div style={{ maxHeight: 160, overflowY: "auto", border: "1px solid var(--border)", borderRadius: "var(--radius)", marginBottom: 8 }}>
                  {eipImportResult.tags.map((t) => (
                    <label
                      key={t.name}
                      className="vt-check"
                      style={{ padding: "3px 8px", display: "flex" }}
                    >
                      <input
                        type="checkbox"
                        checked={eipImportSelected.has(t.name)}
                        onChange={(e) => toggleEipImportTag(t.name, e.target.checked)}
                      />
                      <span style={{ fontFamily: "var(--font-mono)", fontSize: "var(--fs-xs)" }}>{t.name}</span>
                      <span style={{ color: "var(--text-muted)", fontSize: "var(--fs-xs)" }}>
                        {t.template} ({t.members.length} members)
                      </span>
                    </label>
                  ))}
                </div>
                <div style={{ color: "var(--text-muted)", fontSize: "var(--fs-xs)", marginBottom: 4 }}>
                  Preview: eip_types.st
                </div>
                <pre
                  style={{
                    maxHeight: 180,
                    overflowY: "auto",
                    fontSize: "var(--fs-xs)",
                    fontFamily: "var(--font-mono)",
                    background: "var(--sunken)",
                    border: "1px solid var(--border)",
                    borderRadius: "var(--radius)",
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
              <Button
                variant="primary"
                style={{ flex: 1 }}
                loading={eipImportAccepting}
                disabled={!eipImportResult || eipImportSelected.size === 0 || eipImportAccepting}
                onClick={handleEipImportAccept}
              >
                {eipImportAccepting ? "Writing..." : "Accept"}
              </Button>
              <Button variant="default" style={{ flex: 1 }} onClick={() => setEipImportOpen(false)}>
                Cancel
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
