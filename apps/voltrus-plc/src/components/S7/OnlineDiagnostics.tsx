import { useCallback, useEffect, useState } from "react";
import { Button, Callout, EmptyState, StatusDot } from "@mactools/ui";
import { Blocks, Cpu, Gauge, Plug, RefreshCw, ScrollText, Settings2 } from "lucide-react";
import { bridgeMode } from "../../api/bridge";
import { s7, type DiagBuffer, type DiagEntry } from "../../lib/s7";
import { deviceLabel, errorText, useS7 } from "../../lib/s7Store";
import { ConnectionForm, CpuDetails, ModeBadge, ModeButtons, S7AccessNote } from "./common";

type Section = "general" | "diag" | "blocks" | "mode" | "connection";

const NAV: { group: string; items: { id: Section; label: string; icon: typeof Cpu }[] }[] = [
  {
    group: "Diagnostics",
    items: [
      { id: "general", label: "General", icon: Cpu },
      { id: "diag", label: "Diagnostic buffer", icon: ScrollText },
      { id: "blocks", label: "Program blocks", icon: Blocks },
    ],
  },
  {
    group: "Functions",
    items: [
      { id: "mode", label: "Operating mode", icon: Gauge },
      { id: "connection", label: "Connection", icon: Settings2 },
    ],
  },
];

/** The Online & diagnostics editor tab for the connected S7 CPU. */
export default function OnlineDiagnostics() {
  const { info, target, mode } = useS7();
  const online = info !== null;
  const [section, setSection] = useState<Section>(online ? "general" : "connection");

  useEffect(() => {
    if (!online) setSection("connection");
  }, [online]);

  if (bridgeMode !== "tauri") {
    return (
      <EmptyState
        icon={<Plug size={20} strokeWidth={1.75} />}
        title="Desktop app only"
        description="Going online with an S7 CPU needs a direct network connection. Use the Voltrus PLC desktop app."
      />
    );
  }

  return (
    <div className="s7-doc">
      <header className="s7-doc__header">
        <Cpu size={16} strokeWidth={1.75} />
        <span className="s7-doc__title">{deviceLabel(info, target)}</span>
        <span className="s7-muted mono">{target.host || "no address set"}</span>
        <span className="s7-doc__spacer" />
        {online ? (
          <>
            <StatusDot tone="success" label="Online" />
            <ModeBadge mode={mode} />
          </>
        ) : (
          <StatusDot tone="neutral" label="Offline" />
        )}
      </header>
      <div className="s7-od">
        <nav className="s7-od__nav" aria-label="Online and diagnostics">
          {NAV.map((g) => (
            <div key={g.group}>
              <div className="s7-od__group">{g.group}</div>
              {g.items.map((it) => {
                const Icon = it.icon;
                const disabled = !online && it.id !== "connection";
                return (
                  <button
                    key={it.id}
                    className="s7-od__item"
                    aria-current={section === it.id ? "page" : undefined}
                    disabled={disabled}
                    onClick={() => setSection(it.id)}
                  >
                    <Icon size={14} strokeWidth={1.75} /> {it.label}
                  </button>
                );
              })}
            </div>
          ))}
        </nav>
        <div className="s7-od__body">
          {section === "connection" && (
            <div className="s7-od__pane">
              <h2 className="s7-od__h">Connection</h2>
              <ConnectionForm />
              {!online && <S7AccessNote />}
            </div>
          )}
          {online && info && section === "general" && (
            <div className="s7-od__pane">
              <h2 className="s7-od__h">General</h2>
              <CpuDetails info={info} wide />
            </div>
          )}
          {online && section === "diag" && <DiagPane />}
          {online && section === "blocks" && <BlocksPane />}
          {online && section === "mode" && <ModePane />}
        </div>
      </div>
    </div>
  );
}

function eventId(e: DiagEntry) {
  return `16#${e.event_number.toString(16).toUpperCase().padStart(4, "0")}`;
}

function DiagPane() {
  const [diag, setDiag] = useState<DiagBuffer | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [selected, setSelected] = useState(0);

  const read = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setDiag(await s7.diagBuffer());
      setSelected(0);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void read(); }, [read]);

  const entry = diag?.entries[selected];
  return (
    <div className="s7-od__pane s7-od__pane--fill">
      <div className="s7-od__bar">
        <h2 className="s7-od__h">Diagnostic buffer</h2>
        <span className="s7-doc__spacer" />
        {diag && <span className="s7-muted">{diag.entries.length} of {diag.entry_count} entries</span>}
        <Button size="sm" variant="ghost" icon={<RefreshCw size={14} />} loading={loading} onClick={read}>Refresh</Button>
      </div>
      {error && <Callout tone="danger">{error}</Callout>}
      {diag && diag.entries.length === 0 && <p className="s7-muted">The diagnostic buffer is empty.</p>}
      {diag && diag.entries.length > 0 && (
        <>
          <div className="s7-grid-wrap">
            <table className="vt-table s7-grid">
              <thead>
                <tr><th className="s7-grid__no">No.</th><th>Date and time</th><th>Event</th><th>Event ID</th></tr>
              </thead>
              <tbody>
                {diag.entries.map((e, i) => (
                  <tr key={i} aria-selected={i === selected} onClick={() => setSelected(i)}>
                    <td className="s7-grid__no">{i + 1}</td>
                    <td className="mono">{e.timestamp}</td>
                    <td>{e.event_text}</td>
                    <td className="mono">{eventId(e)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {entry && (
            <section className="s7-od__detail" aria-label="Event details">
              <div className="s7-od__detail-title">Details on event {selected + 1}</div>
              <p>{entry.event_text}</p>
              <dl className="s7-kv s7-kv--wide">
                <div><dt>Event ID</dt><dd className="mono">{eventId(entry)}</dd></div>
                <div><dt>Time</dt><dd className="mono">{entry.timestamp}</dd></div>
                {entry.ob_number > 0 && <div><dt>OB</dt><dd className="mono">OB{entry.ob_number}</dd></div>}
                {entry.priority_class > 0 && <div><dt>Priority class</dt><dd className="mono">{entry.priority_class}</dd></div>}
              </dl>
            </section>
          )}
        </>
      )}
    </div>
  );
}

function BlocksPane() {
  const { blocks, blocksError, readBlocks } = useS7();
  const [loading, setLoading] = useState(false);
  const refresh = async () => {
    setLoading(true);
    await readBlocks();
    setLoading(false);
  };
  return (
    <div className="s7-od__pane s7-od__pane--fill">
      <div className="s7-od__bar">
        <h2 className="s7-od__h">Program blocks in the CPU</h2>
        <span className="s7-doc__spacer" />
        {blocks && <span className="s7-muted">{blocks.length} blocks</span>}
        <Button size="sm" variant="ghost" icon={<RefreshCw size={14} />} loading={loading} onClick={refresh}>Refresh</Button>
      </div>
      {blocksError && <Callout tone="warn">The CPU did not list its blocks: {blocksError}. S7-1200/1500 often refuse this.</Callout>}
      {blocks && blocks.length === 0 && <p className="s7-muted">No blocks listed.</p>}
      {blocks && blocks.length > 0 && (
        <div className="s7-grid-wrap">
          <table className="vt-table s7-grid">
            <thead>
              <tr><th className="s7-grid__mark" /><th>Block</th><th>Type</th><th>Number</th><th>Language</th></tr>
            </thead>
            <tbody>
              {blocks.map((b) => (
                <tr key={`${b.block_type}${b.block_number}`}>
                  <td className="s7-grid__mark"><span className="s7-mark s7-mark--ok" title="In the CPU" /></td>
                  <td className="mono">{b.block_type}{b.block_number}</td>
                  <td>{b.block_type}</td>
                  <td className="mono">{b.block_number}</td>
                  <td>{b.language || "–"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      <p className="s7-muted">Block code is not uploaded: this view lists what the CPU holds.</p>
    </div>
  );
}

function ModePane() {
  const { mode, error } = useS7();
  const lamp = mode === "RUN" ? "run" : mode === "STOP" ? "stop" : "unknown";
  return (
    <div className="s7-od__pane">
      <h2 className="s7-od__h">Operating mode</h2>
      <div className="s7-mode">
        <div className="s7-mode__lamps" aria-label={`CPU is in ${mode ?? "an unknown mode"}`}>
          <span className={`s7-lamp ${lamp === "run" ? "s7-lamp--run" : ""}`}>RUN</span>
          <span className={`s7-lamp ${lamp === "stop" ? "s7-lamp--stop" : ""}`}>STOP</span>
        </div>
        <div>
          <div className="s7-mode__now">CPU is in <strong>{mode ?? "an unknown mode"}</strong></div>
          <ModeButtons />
        </div>
      </div>
      {error && <Callout tone="danger">{error}</Callout>}
      <p className="s7-muted">
        Both actions ask for confirmation. RUN offers a warm or cold restart.
      </p>
    </div>
  );
}
