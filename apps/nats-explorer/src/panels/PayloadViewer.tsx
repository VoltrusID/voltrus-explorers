// Payload viewer with format switch (Text / JSON / MsgPack / Hex).
// Used in message row-detail and by KV/Object panels.

import { useEffect, useState } from "react";
import { Badge, Callout, Segmented } from "@mactools/ui";

import { decodeDetect, decodeFormat } from "../nats-client";

type Tab = "utf8" | "json" | "msgpack" | "hex";

const TABS: { id: Tab; label: string; fmt: string }[] = [
  { id: "utf8", label: "Text", fmt: "utf8" },
  { id: "json", label: "JSON", fmt: "json" },
  { id: "msgpack", label: "MsgPack", fmt: "msgpack" },
  { id: "hex", label: "Hex", fmt: "hex" },
];

export function PayloadViewer({
  payload,
  maxHeight = 240,
}: {
  payload: string;
  maxHeight?: number;
}) {
  const [tab, setTab] = useState<Tab>("utf8");
  const [detected, setDetected] = useState<string>("");
  const [out, setOut] = useState<string>("");
  const [err, setErr] = useState<string>("");

  // Auto-pick a tab from detection on first payload.
  useEffect(() => {
    let cancelled = false;
    decodeDetect(payload)
      .then((d) => {
        if (cancelled) return;
        setDetected(d);
        if (d === "json") setTab("json");
        else if (d === "msg_pack") setTab("msgpack");
        else if (d === "binary") setTab("hex");
        else setTab("utf8");
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [payload]);

  useEffect(() => {
    let cancelled = false;
    setErr("");
    decodeFormat(payload, TABS.find((t) => t.id === tab)!.fmt)
      .then((r) => !cancelled && setOut(r))
      .catch((e) => !cancelled && setErr(String(e)));
    return () => {
      cancelled = true;
    };
  }, [payload, tab]);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6, minWidth: 0 }}>
      <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
        <Segmented
          size="sm"
          label="Payload format"
          value={tab}
          onChange={setTab}
          options={TABS.map((t) => ({ value: t.id, label: t.label }))}
        />
        {detected && (
          <Badge tone="info" mono>
            {detected}
          </Badge>
        )}
      </div>
      {err ? (
        <Callout tone="danger">{err}</Callout>
      ) : (
        <pre
          className="mono"
          style={{
            margin: 0,
            padding: 8,
            maxHeight,
            overflow: "auto",
            background: "var(--sunken)",
            border: "1px solid var(--border)",
            borderRadius: "var(--radius)",
            fontSize: "var(--fs-sm)",
            whiteSpace: "pre-wrap",
            wordBreak: "break-all",
            color: "var(--text)",
          }}
        >
          {out}
        </pre>
      )}
      <input
        className="vt-input vt-input--mono vt-input--sm"
        value={payload}
        readOnly
        placeholder="(payload)"
        aria-label="Raw payload"
      />
    </div>
  );
}
