// $SYS monitoring: poll STATSZ / CONNZ / SUBSZ / JSZ every few seconds via
// request/reply, plus a live `$SYS.>` event tail.

import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Activity, RefreshCw } from "lucide-react";
import { Button, Callout, EmptyState, Field, Panel } from "@mactools/ui";

import * as api from "../nats-client";
import type { SysMessage } from "../nats-types";

const PINGS = [
  { label: "STATSZ", subject: "$SYS.REQ.SERVER.PING.STATSZ" },
  { label: "CONNZ", subject: "$SYS.REQ.SERVER.PING.CONNZ" },
  { label: "SUBSZ", subject: "$SYS.REQ.SERVER.PING.SUBSZ" },
  { label: "JSZ", subject: "$SYS.REQ.SERVER.PING.JSZ" },
];

export function MonitorPanel({
  connected,
  addLog,
}: {
  connected: boolean;
  addLog: (dir: "info" | "error", msg: string) => void;
}) {
  const [results, setResults] = useState<Record<string, string>>({});
  const [tail, setTail] = useState<SysMessage[]>([]);
  const [tailSubject, setTailSubject] = useState("$SYS.>");
  const [tailSid, setTailSid] = useState<string | null>(null);
  const [autoPoll, setAutoPoll] = useState(true);
  const [err, setErr] = useState("");
  const timer = useRef<number | undefined>(undefined);
  const unlisten = useRef<UnlistenFn | null>(null);

  const pollAll = async () => {
    const next: Record<string, string> = {};
    for (const p of PINGS) {
      try {
        next[p.label] = await api.sysRequest(p.subject, 2000);
      } catch (e) {
        next[p.label] = `(error: ${String(e)})`;
      }
    }
    setResults(next);
  };

  useEffect(() => {
    if (connected && autoPoll) {
      pollAll();
      timer.current = window.setInterval(pollAll, 5000);
    }
    return () => {
      clearInterval(timer.current);
      timer.current = undefined;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connected, autoPoll]);

  const startTail = async () => {
    if (tailSid) return;
    try {
      const sid = await api.sysSubscribe(tailSubject || "$SYS.>");
      setTailSid(sid);
      unlisten.current = await listen<SysMessage>("sys-message", (e) => {
        setTail((t) => [...t.slice(-200), e.payload as SysMessage]);
      });
      addLog("info", `Subscribed ${tailSubject}`);
    } catch (e) {
      setErr(String(e));
    }
  };

  const stopTail = async () => {
    unlisten.current?.();
    unlisten.current = null;
    if (tailSid) {
      invoke("client_unsubscribe", { sid: tailSid }).catch(() => {});
      setTailSid(null);
    }
  };

  useEffect(() => () => { stopTail(); }, []);

  if (!connected) {
    return (
      <div style={{ height: "100%", display: "flex", flexDirection: "column" }}>
        <EmptyState
          icon={<Activity size={18} strokeWidth={1.75} />}
          title="Not connected"
          description="Connect to a server with monitoring enabled to poll its $SYS endpoints."
        />
      </div>
    );
  }

  return (
    <div className="vt-stack" style={{ height: "100%", padding: 12, overflow: "auto" }}>
      <div style={{ display: "flex", gap: 12, alignItems: "center", flexWrap: "wrap" }}>
        <Button size="sm" variant="ghost" icon={<RefreshCw size={14} strokeWidth={1.75} />} onClick={pollAll}>
          Refresh now
        </Button>
        <label className="vt-check">
          <input type="checkbox" checked={autoPoll} onChange={(e) => setAutoPoll(e.target.checked)} />
          Auto-poll every 5 s
        </label>
      </div>

      {err && <Callout tone="danger">{err}</Callout>}

      <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(280px, 1fr))", gap: 8 }}>
        {PINGS.map((p) => (
          <Panel key={p.label} title={p.label}>
            <pre
              className="mono"
              style={{
                margin: 0,
                padding: 10,
                fontSize: "var(--fs-sm)",
                whiteSpace: "pre-wrap",
                maxHeight: 140,
                overflow: "auto",
                color: "var(--text-dim)",
              }}
            >
              {results[p.label] ? prettyJson(results[p.label]!) : "Waiting for response"}
            </pre>
          </Panel>
        ))}
      </div>

      <div style={{ borderTop: "1px solid var(--border)", paddingTop: 10, display: "flex", flexDirection: "column", gap: 8, flex: 1, minHeight: 140 }}>
        <div style={{ display: "flex", gap: 8, alignItems: "flex-end", flexWrap: "wrap" }}>
          <div style={{ flex: 1, minWidth: 200 }}>
            <Field label="Live tail subject">
              {(id) => (
                <input
                  id={id}
                  className="vt-input vt-input--mono"
                  value={tailSubject}
                  onChange={(e) => setTailSubject(e.target.value)}
                  disabled={!!tailSid}
                />
              )}
            </Field>
          </div>
          {tailSid ? (
            <Button onClick={stopTail}>Stop tail</Button>
          ) : (
            <Button variant="primary" onClick={startTail}>Start tail</Button>
          )}
        </div>
        <div
          className="mono"
          style={{
            flex: 1,
            minHeight: 100,
            maxHeight: 220,
            overflow: "auto",
            border: "1px solid var(--border)",
            borderRadius: "var(--radius)",
            padding: 8,
            fontSize: "var(--fs-sm)",
            display: "flex",
            flexDirection: "column",
          }}
        >
          {tail.length === 0 ? (
            <div style={{ fontFamily: "var(--font-sans)", display: "flex", flex: 1 }}>
              <EmptyState
                compact
                title="No events yet"
                description="Start the tail to follow $SYS traffic in real time."
              />
            </div>
          ) : (
            tail.map((m, i) => (
              <div key={i} style={{ borderBottom: "1px solid var(--border)", paddingBottom: 2 }}>
                <span style={{ color: "var(--accent-text)" }}>{m.subject}</span>{" "}
                <span style={{ color: "var(--text-muted)" }}>{m.payload.slice(0, 200)}</span>
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}

function prettyJson(s: string): string {
  try {
    return JSON.stringify(JSON.parse(s), null, 2);
  } catch {
    return s;
  }
}
