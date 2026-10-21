import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { DeviceInspection } from "../types";

interface DeviceInspectorProps {
  loading: boolean;
}

function fmt(value: unknown): string {
  if (value === null || value === undefined) return "—";
  return String(value);
}

const card: React.CSSProperties = {
  background: "var(--card)",
  border: "1px solid var(--border)",
  borderRadius: "var(--radius-lg)",
  padding: 14,
  marginBottom: 10,
};

const cardTitle: React.CSSProperties = {
  fontSize: "var(--fs-sm)",
  fontWeight: 600,
  color: "var(--text)",
  margin: "0 0 8px",
};

const fieldRow: React.CSSProperties = {
  display: "flex",
  alignItems: "baseline",
  gap: 12,
  padding: "3px 0",
};

const fieldLabel: React.CSSProperties = {
  color: "var(--text-dim)",
  fontSize: "var(--fs-sm)",
  minWidth: 120,
  flexShrink: 0,
};

const fieldValue: React.CSSProperties = {
  color: "var(--text)",
  fontSize: "var(--fs-md)",
  fontWeight: 500,
  overflowWrap: "anywhere",
};

const objectNameStyle: React.CSSProperties = {
  fontSize: "var(--fs-lg)",
  fontWeight: 600,
  color: "var(--text)",
  margin: "0 0 8px",
};

function FieldRow({ label, value }: { label: string; value: unknown }) {
  return (
    <div style={fieldRow}>
      <span style={fieldLabel}>{label}</span>
      <span style={fieldValue}>{fmt(value)}</span>
    </div>
  );
}

export function DeviceInspector(props: DeviceInspectorProps) {
  const [data, setData] = useState<DeviceInspection | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function fetchInspection() {
      try {
        const result = await invoke<DeviceInspection>("inspect_device_cmd");
        if (!cancelled) {
          setData(result);
          setError(null);
        }
      } catch (e) {
        if (!cancelled) {
          setError(e instanceof Error ? e.message : String(e));
        }
      }
    }

    fetchInspection();

    return () => {
      cancelled = true;
    };
  }, []);

  if (error) {
    return (
      <div style={{ ...card, color: "var(--text-dim)", fontSize: "var(--fs-sm)" }}>
        Inspection failed: {error}
      </div>
    );
  }

  if (!data || props.loading) {
    return (
      <div style={{ display: "flex", alignItems: "center", justifyContent: "center", gap: 8, padding: "24px 0", fontSize: "var(--fs-sm)", color: "var(--text-muted)" }}>
        <span className="vt-spinner" aria-hidden="true" />
        Inspecting device…
      </div>
    );
  }

  return (
    <div>
      {/* Identity */}
      <div style={card}>
        <h3 style={cardTitle}>Identity</h3>
        <div style={objectNameStyle}>{fmt(data.object_name)}</div>
        <FieldRow
          label="Vendor"
          value={
            data.vendor_name !== null
              ? `${data.vendor_name} (${fmt(data.vendor_id)})`
              : null
          }
        />
        <FieldRow label="Model" value={data.model_name} />
        <FieldRow label="Firmware" value={data.firmware_revision} />
        <FieldRow
          label="App software"
          value={data.application_software_version}
        />
      </div>

      {/* Protocol */}
      <div style={card}>
        <h3 style={cardTitle}>Protocol</h3>
        <FieldRow
          label="Protocol"
          value={
            data.protocol_version !== null && data.protocol_revision !== null
              ? `${data.protocol_version}.${data.protocol_revision}`
              : null
          }
        />
        <FieldRow label="Max APDU" value={data.max_apdu_length} />
        <FieldRow label="Segmentation" value={data.segmentation_supported} />
        <FieldRow
          label="APDU timeout"
          value={
            data.apdu_timeout !== null ? `${data.apdu_timeout} ms` : null
          }
        />
        <FieldRow label="Retries" value={data.number_of_apdu_retries} />
        <FieldRow label="Services" value={data.services_supported} />
        <FieldRow label="Object types" value={data.object_types_supported} />
      </div>

      {/* Status */}
      <div style={card}>
        <h3 style={cardTitle}>Status</h3>
        <FieldRow label="Description" value={data.description} />
        <FieldRow label="Location" value={data.location} />
        <FieldRow label="System status" value={data.system_status} />
        <FieldRow label="Object count" value={data.object_count} />
        <FieldRow label="Local time" value={data.local_time} />
        <FieldRow label="Local date" value={data.local_date} />
        <FieldRow label="Database rev" value={data.database_revision} />
      </div>
    </div>
  );
}
