import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Badge, Button, Callout, Field, ProBadge, openTextFile } from "@mactools/ui";
import { FileUp, Play, Repeat, Square } from "lucide-react";
import { useEffect, useState } from "react";
import type { ReplayInfo } from "./types";
import { card, hint, row, type AddLog, type RequirePro } from "./ui";

interface Props {
  connected: boolean;
  canSend: boolean;
  requirePro: RequirePro;
  addLog: AddLog;
}

const LOG_FILTER = { name: "candump log", extensions: ["log", "txt", "candump"] };

export default function ReplayView({ connected, canSend, requirePro, addLog }: Props) {
  const [file, setFile] = useState<{ path: string; text: string } | null>(null);
  const [speed, setSpeed] = useState(1);
  const [repeat, setRepeat] = useState(false);
  const [ids, setIds] = useState("");
  const [info, setInfo] = useState<ReplayInfo | null>(null);
  const [progress, setProgress] = useState<{ pos: number; total: number; pass?: number } | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const un = listen<{ done: boolean; pos?: number; total?: number; pass?: number; error?: string }>("can-replay", (e) => {
      const p = e.payload;
      if (p.total != null) setProgress({ pos: p.pos ?? 0, total: p.total, pass: p.pass });
      if (p.done) {
        setRunning(false);
        if (p.error) setError(p.error);
      }
    });
    return () => { void un.then((u) => u()); };
  }, []);

  const pick = async () => {
    try {
      const f = await openTextFile(LOG_FILTER);
      if (f) { setFile(f); setInfo(null); setProgress(null); setError(null); }
    } catch (e) {
      setError(String(e));
    }
  };

  const start = async () => {
    if (!requirePro("Trace replay") || !file) return;
    setError(null);
    const idList = ids.split(/[\s,]+/).filter(Boolean).map((s) => parseInt(s.replace(/^0x/i, ""), 16));
    if (idList.some((n) => Number.isNaN(n))) { setError("Identifiers are hex, separated by commas."); return; }
    try {
      const r = await invoke<ReplayInfo>("replay_start", { request: { text: file.text, speed, repeat, ids: idList } });
      setInfo(r);
      setRunning(true);
      addLog("info", `Replaying ${r.frames} frames (${r.durationS.toFixed(1)} s at ${speed}x) from ${file.path}`);
    } catch (e) {
      setError(String(e));
    }
  };

  const stop = async () => {
    await invoke("replay_stop");
    setRunning(false);
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8, overflow: "auto", flex: 1, minHeight: 0 }}>
      <div style={{ ...card, display: "flex", flexDirection: "column", gap: 8 }}>
        <div style={row}>
          <strong style={{ fontSize: "var(--fs-sm)" }}>Replay a candump log onto the bus <ProBadge /></strong>
        </div>
        <div style={row}>
          <Button icon={<FileUp size={14} strokeWidth={1.75} />} onClick={() => void pick()}>Open log</Button>
          <span className="mono" style={{ fontSize: "var(--fs-xs)", color: "var(--text-dim)" }}>{file ? file.path : "No file"}</span>
        </div>
        <div style={{ ...row, alignItems: "flex-end" }}>
          <Field label="Speed">
            <select className="vt-input" value={speed} onChange={(e) => setSpeed(Number(e.target.value))} disabled={running}>
              {[0.25, 0.5, 1, 2, 5, 10].map((s) => <option key={s} value={s}>{s}x</option>)}
            </select>
          </Field>
          <Field label="Only these IDs (hex, optional)">
            <input className="vt-input vt-input--mono" value={ids} placeholder="7E8, 18FEF100" onChange={(e) => setIds(e.target.value)} disabled={running} />
          </Field>
          <label style={{ display: "flex", gap: 6, alignItems: "center", fontSize: "var(--fs-sm)", paddingBottom: 6 }}>
            <input type="checkbox" checked={repeat} onChange={(e) => setRepeat(e.target.checked)} disabled={running} /> <Repeat size={14} /> Loop
          </label>
          <span style={{ flex: 1 }} />
          {running ? (
            <Button variant="danger" icon={<Square size={14} strokeWidth={1.75} />} onClick={() => void stop()}>Stop</Button>
          ) : (
            <Button variant="primary" icon={<Play size={14} strokeWidth={1.75} />} disabled={!file || !connected || !canSend} onClick={() => void start()}>Replay</Button>
          )}
        </div>
        {info && progress && (
          <div style={row}>
            <Badge tone={running ? "success" : "neutral"}>{running ? "Replaying" : "Done"}</Badge>
            <progress value={progress.pos} max={progress.total} style={{ flex: 1 }} />
            <span className="mono" style={{ fontSize: "var(--fs-xs)" }}>{progress.pos} / {progress.total}{progress.pass ? ` (pass ${progress.pass + 1})` : ""}</span>
          </div>
        )}
        {error && <Callout tone="danger">{error}</Callout>}
        <span style={hint}>
          Frames go out with the timing they were recorded with, scaled by the speed. The log is the candump format (<span className="mono">(1699999999.123456) can0 123#DEADBEEF</span>), as written by the trace's candump export, recording and Linux candump -l.
          {!connected && " Connect to a bus first."}
        </span>
      </div>
      <Callout tone="warn">Replaying onto a live machine sends real commands to it. Use a bench setup or the virtual bus unless you know what the recording contains.</Callout>
    </div>
  );
}
