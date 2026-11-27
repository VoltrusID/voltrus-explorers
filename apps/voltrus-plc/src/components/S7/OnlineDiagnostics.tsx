import { useCallback, useEffect, useState } from "react";
import { Badge, Button, Callout, EmptyState, StatusDot } from "@mactools/ui";
import { Blocks, Cpu, FileSearch, Gauge, GitCompareArrows, HardDriveDownload, Plug, RefreshCw, Save, ScrollText, Settings2 } from "lucide-react";
import { bridgeMode } from "../../api/bridge";
import { s7, type BlockDetails, type DiagBuffer, type DiagEntry } from "../../lib/s7";
import { compareBlocks, loadBaseline, saveBaseline, type Baseline, type CompareRow, type CompareStatus } from "../../lib/s7Blocks";
import { useStore } from "../../store";
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

const STATUS_TEXT: Record<CompareStatus, string> = {
  same: "Same as baseline",
  changed: "Changed since baseline",
  new: "Not in baseline",
  missing: "In baseline, not in the CPU",
};

function BlocksPane() {
  const { blocks, blocksError, readBlocks, target } = useS7();
  const projectDir = useStore((s) => s.projectDir);
  const addLog = useStore((s) => s.addLog);
  const [loading, setLoading] = useState<null | "list" | "details" | "backup">(null);
  const [details, setDetails] = useState<Record<string, BlockDetails>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [baseline, setBaseline] = useState<Baseline | null>(null);
  const [compare, setCompare] = useState(false);

  useEffect(() => { void loadBaseline(target.host).then(setBaseline); }, [target.host, projectDir]);

  const refresh = async () => {
    setLoading("list");
    await readBlocks();
    setLoading(null);
  };

  /** Upload every listed block for its details; with a folder, keep the bytes as a backup. */
  const upload = async (backup: boolean) => {
    if (!blocks?.length) return null;
    setLoading(backup ? "backup" : "details");
    const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-");
    const dir = backup && projectDir ? `${projectDir}/cpu_backup/${stamp}` : undefined;
    try {
      const res = await s7.uploadBlocks(blocks.map((b) => ({ block_type: b.block_type, block_number: b.block_number })), dir);
      const d: Record<string, BlockDetails> = {};
      const err: Record<string, string> = {};
      for (const r of res) {
        const key = `${r.block_type}${r.block_number}`;
        if (r.details) d[key] = r.details;
        if (r.error) err[key] = r.error;
      }
      setDetails(d);
      setErrors(err);
      const ok = res.filter((r) => r.details).length;
      if (dir) addLog("system", `Backed up ${res.filter((r) => r.file).length} of ${res.length} blocks to ${dir}`);
      else addLog("system", `Read the details of ${ok} of ${res.length} blocks`);
      return Object.values(d);
    } catch (e) {
      addLog("system", `Block upload failed: ${errorText(e)}`);
      return null;
    } finally {
      setLoading(null);
    }
  };

  const saveAsBaseline = async () => {
    const list = Object.keys(details).length ? Object.values(details) : await upload(false);
    if (!list?.length) return;
    const b: Baseline = { savedAt: new Date().toISOString(), host: target.host, blocks: list };
    const where = await saveBaseline(b);
    setBaseline(b);
    setCompare(true);
    addLog("system", `Saved a baseline of ${list.length} blocks (${where})`);
  };

  const runCompare = async () => {
    if (!Object.keys(details).length) await upload(false);
    setCompare(true);
  };

  const rows: CompareRow[] | null = compare && baseline ? compareBlocks(Object.values(details), baseline.blocks) : null;
  const counts = rows?.reduce((acc, r) => ({ ...acc, [r.status]: (acc[r.status] ?? 0) + 1 }), {} as Record<string, number>);
  const hasDetails = Object.keys(details).length > 0;

  const detailCells = (d: BlockDetails | undefined, key: string) => d ? (
    <>
      <td>{d.name || "–"}</td>
      <td>{d.family || "–"}</td>
      <td>{d.author || "–"}</td>
      <td className="mono">{d.version}</td>
      <td className="mono">{d.load_size}</td>
      <td className="mono">{d.code_time}</td>
      <td className="mono">16#{d.checksum.toString(16).toUpperCase().padStart(4, "0")}</td>
      <td>{d.know_how_protected ? <Badge tone="warn">Protected</Badge> : ""}</td>
    </>
  ) : (
    <td colSpan={8} className="s7-muted">{errors[key] ?? (hasDetails ? "No details" : "")}</td>
  );

  return (
    <div className="s7-od__pane s7-od__pane--fill">
      <div className="s7-od__bar">
        <h2 className="s7-od__h">Program blocks in the CPU</h2>
        <span className="s7-doc__spacer" />
        {blocks && <span className="s7-muted">{blocks.length} blocks</span>}
        <Button size="sm" variant="ghost" icon={<RefreshCw size={14} />} loading={loading === "list"} onClick={refresh}>Refresh</Button>
      </div>
      <div className="s7-row">
        <Button size="sm" icon={<FileSearch size={14} />} disabled={!blocks?.length || loading !== null} loading={loading === "details"}
          onClick={() => void upload(false)} title="Upload each block to read its header: name, author, version, checksum, time stamps">
          Read details
        </Button>
        <Button size="sm" icon={<HardDriveDownload size={14} />} disabled={!blocks?.length || loading !== null || !projectDir} loading={loading === "backup"}
          onClick={() => void upload(true)}
          title={projectDir ? "Save every block, as the CPU sends it, under cpu_backup/ in the project" : "Open a project folder to back up blocks"}>
          Back up to project
        </Button>
        <span className="vt-divider--v" />
        <Button size="sm" icon={<Save size={14} />} disabled={!blocks?.length || loading !== null} onClick={() => void saveAsBaseline()}>
          Save as baseline
        </Button>
        <Button size="sm" variant={compare ? "primary" : "default"} icon={<GitCompareArrows size={14} />}
          disabled={!baseline || loading !== null} onClick={() => (compare ? setCompare(false) : void runCompare())}
          title={baseline ? `Compare with the baseline of ${new Date(baseline.savedAt).toLocaleString()}` : "Save a baseline first"}>
          {compare ? "Hide comparison" : "Compare with baseline"}
        </Button>
      </div>
      {blocksError && <Callout tone="warn">The CPU did not list its blocks: {blocksError}. S7-1200/1500 often refuse this.</Callout>}
      {rows && counts && (
        <Callout tone={counts.changed || counts.new || counts.missing ? "warn" : "success"}>
          Against the baseline of {new Date(baseline!.savedAt).toLocaleString()}: {counts.changed ?? 0} changed,
          {" "}{counts.new ?? 0} new, {counts.missing ?? 0} missing, {counts.same ?? 0} the same.
        </Callout>
      )}
      {blocks && blocks.length === 0 && <p className="s7-muted">No blocks listed.</p>}
      {blocks && blocks.length > 0 && (
        <div className="s7-grid-wrap">
          <table className="vt-table s7-grid">
            <thead>
              <tr>
                <th className="s7-grid__mark" /><th>Block</th><th>Language</th>
                <th>Name</th><th>Family</th><th>Author</th><th>Version</th><th>Load size</th><th>Code time</th><th>Checksum</th><th />
              </tr>
            </thead>
            <tbody>
              {rows
                ? rows.map((r) => {
                    const b = r.now ?? r.then!;
                    return (
                      <tr key={r.key} className={`s7-cmp s7-cmp--${r.status}`} title={STATUS_TEXT[r.status] + (r.diffs.length ? `: ${r.diffs.join(", ")}` : "")}>
                        <td className="s7-grid__mark"><span className={`s7-mark s7-mark--cmp-${r.status}`} /></td>
                        <td className="mono">{r.key}</td>
                        <td>{b.language || "–"}</td>
                        {detailCells(r.now ?? r.then, r.key)}
                      </tr>
                    );
                  })
                : blocks.map((b) => {
                    const key = `${b.block_type}${b.block_number}`;
                    return (
                      <tr key={key}>
                        <td className="s7-grid__mark"><span className="s7-mark s7-mark--ok" title="In the CPU" /></td>
                        <td className="mono">{key}</td>
                        <td>{b.language || "–"}</td>
                        {detailCells(details[key], key)}
                      </tr>
                    );
                  })}
            </tbody>
          </table>
        </div>
      )}
      <p className="s7-muted">
        Details come from each block's header, read by uploading it; the code itself is not decoded.
        S7-1200/1500 do not allow uploads over PUT/GET. A baseline marks what changed in the CPU since.
      </p>
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
