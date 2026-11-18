import { invoke } from "@tauri-apps/api/core";
import { Badge, Button, Callout, Field, ProBadge, openTextFile } from "@mactools/ui";
import { FileUp, Play, Repeat, Square } from "lucide-react";
import { TRACE_FILE_FILTER, type ReplayState } from "./replay";
import type { ReplayInfo } from "./types";
import { card, hint, row, type AddLog, type RequirePro } from "./ui";

interface Props {
  connected: boolean;
  canSend: boolean;
  requirePro: RequirePro;
  addLog: AddLog;
  state: ReplayState;
  update: (patch: Partial<ReplayState>) => void;
}

export default function ReplayView({ connected, canSend, requirePro, addLog, state, update }: Props) {
  const { file, speed, repeat, ids, info, progress, running, error, stopped } = state;

  const pick = async () => {
    try {
      const f = await openTextFile(TRACE_FILE_FILTER);
      if (f) update({ file: f, info: null, progress: null, error: null, stopped: null });
    } catch (e) {
      update({ error: String(e) });
    }
  };

  const start = async () => {
    if (!requirePro("Trace replay") || !file) return;
    const idList = ids.split(/[\s,]+/).filter(Boolean).map((s) => parseInt(s.replace(/^0x/i, ""), 16));
    if (idList.some((n) => Number.isNaN(n))) { update({ error: "Identifiers are hex, separated by commas." }); return; }
    // Running before the call returns: a short replay can end (and say so) before the answer arrives.
    update({ error: null, stopped: null, progress: null, running: true });
    try {
      const r = await invoke<ReplayInfo>("replay_start", { request: { text: file.text, speed, repeat, ids: idList } });
      update({ info: r });
      addLog("info", `Replaying ${r.frames} frames (${r.durationS.toFixed(1)} s at ${speed}x) from ${file.path}`);
      for (const n of r.notes ?? []) addLog("info", n);
    } catch (e) {
      update({ error: String(e), running: false });
    }
  };

  const stop = async () => {
    await invoke("replay_stop");
    update({ running: false });
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8, overflow: "auto", flex: 1, minHeight: 0 }}>
      <div style={{ ...card, display: "flex", flexDirection: "column", gap: 8 }}>
        <div style={row}>
          <strong style={{ fontSize: "var(--fs-sm)" }}>Replay a trace onto the bus <ProBadge /></strong>
        </div>
        <div style={row}>
          <Button icon={<FileUp size={14} strokeWidth={1.75} />} onClick={() => void pick()} disabled={running}>Open log</Button>
          <span className="mono" style={{ fontSize: "var(--fs-xs)", color: "var(--text-dim)" }}>{file ? file.path : "No file"}</span>
        </div>
        <div style={{ ...row, alignItems: "flex-end" }}>
          <Field label="Speed">
            <select className="vt-input" value={speed} onChange={(e) => update({ speed: Number(e.target.value) })} disabled={running}>
              {[0.25, 0.5, 1, 2, 5, 10].map((s) => <option key={s} value={s}>{s}x</option>)}
            </select>
          </Field>
          <Field label="Only these IDs (hex, optional)">
            <input className="vt-input vt-input--mono" value={ids} placeholder="7E8, 18FEF100" onChange={(e) => update({ ids: e.target.value })} disabled={running} />
          </Field>
          <label style={{ display: "flex", gap: 6, alignItems: "center", fontSize: "var(--fs-sm)", paddingBottom: 6 }}>
            <input type="checkbox" checked={repeat} onChange={(e) => update({ repeat: e.target.checked })} disabled={running} /> <Repeat size={14} /> Loop
          </label>
          <span style={{ flex: 1 }} />
          {running ? (
            <Button variant="danger" icon={<Square size={14} strokeWidth={1.75} />} onClick={() => void stop()}>Stop</Button>
          ) : (
            <Button variant="primary" icon={<Play size={14} strokeWidth={1.75} />} disabled={!file || !connected || !canSend} onClick={() => void start()}>Replay</Button>
          )}
        </div>
        {(running || (info && progress)) && (
          <div style={row}>
            <Badge tone={running ? "success" : "neutral"}>{running ? "Replaying" : stopped ? "Stopped" : "Done"}</Badge>
            <progress value={progress?.pos ?? 0} max={progress?.total ?? info?.frames ?? 1} style={{ flex: 1 }} />
            {progress && (
              <span className="mono" style={{ fontSize: "var(--fs-xs)" }}>{progress.pos} / {progress.total}{progress.pass ? ` (pass ${progress.pass + 1})` : ""}</span>
            )}
          </div>
        )}
        {stopped && !running && <Callout tone="warn">{stopped}</Callout>}
        {error && <Callout tone="danger">{error}</Callout>}
        <span style={hint}>
          Frames go out with the timing they were recorded with, scaled by the speed; times that go backwards count as no wait. The log can be a
          candump log (<span className="mono">(1699999999.123456) can0 123#DEADBEEF</span>, as the trace's candump export, recording and Linux candump -l write
          it), candump's screen output, a Vector ASC or PCAN TRC trace, or this app's CSV export. Looped, a pass lasts at least 10 ms.
          {!connected && " Connect to a bus first."}
        </span>
      </div>
      <Callout tone="warn">Replaying onto a live machine sends real commands to it. Use a bench setup or the virtual bus unless you know what the recording contains.</Callout>
    </div>
  );
}
