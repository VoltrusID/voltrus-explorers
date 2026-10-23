import { useCallback, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ExternalLink, ImagePlus, Sparkles, X } from "lucide-react";
import { Badge, Button, Callout, EmptyState, Field, Tabs } from "./primitives";
import { SheetSelect } from "./sheetSelect";
import { ConsentSheet, useAssistant, type AssistantAccess } from "./assistantProvider";
import { fileToJpeg } from "./assistantImage";
import {
  WORD_ORDER_LABEL,
  blockSummary,
  commonWordOrder,
  failureInfo,
  isFailure,
  parseErrorCode,
  parseRegisters,
  quotaLine,
  type AnswerResult,
  type AssistantBlock,
  type AssistantContext,
  type AssistantFailure,
  type AssistantProtocol,
  type AssistantTask,
  type DecodeCandidate,
  type Diagnosis,
  type ManualResult,
  type RegisterMapResult,
} from "./assistantLogic";

export interface AssistantPanelProps {
  protocol: AssistantProtocol;
  /** Which tasks this app offers, in tab order. */
  tasks: AssistantTask[];
  /** Read when a request is sent, so the answer reflects the session as it is now. */
  getContext?: () => AssistantContext;
  /**
   * Adds the blocks the user confirmed. May return a sentence to show afterwards (e.g. "Set the word
   * order to CD AB"). Absent for apps that have no blocks.
   */
  onAddBlocks?: (blocks: AssistantBlock[]) => string | void;
}

const TAB_LABEL: Record<AssistantTask, string> = {
  lookup: "Find a device's registers",
  manual: "Read a manual page",
  diagnose: "Diagnose an error",
  decode: "Decode raw values",
  ask: "Ask about this session",
};

const TAB_SHORT: Record<AssistantTask, string> = { lookup: "Find", manual: "Manual", diagnose: "Diagnose", decode: "Decode", ask: "Ask" };

/** Tabs when they fit; a picker (a sheet on a phone) when there are more than three. */
function TaskPicker({ tasks, value, onChange }: { tasks: AssistantTask[]; value: AssistantTask; onChange: (t: AssistantTask) => void }) {
  if (tasks.length > 3)
    return (
      <SheetSelect
        sheetTitle="Assistant"
        aria-label="Assistant task"
        value={value}
        onChange={onChange}
        options={tasks.map((t) => ({ value: t, label: TAB_LABEL[t] }))}
      />
    );
  return <Tabs label="Assistant tasks" value={value} items={tasks.map((t) => ({ value: t, label: TAB_SHORT[t] }))} onChange={onChange} />;
}

const dim = { fontSize: "var(--fs-sm)", color: "var(--text-dim)", lineHeight: 1.5 } as const;
const col = { display: "flex", flexDirection: "column", gap: 10 } as const;
const row = { display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" } as const;

function openLink(url: string) {
  void invoke("connect_open_url", { url }).catch(() => {});
}

/** The Assistant: gate (sign in, subscribe, consent), quota line and one view per task. */
export function AssistantPanel(props: AssistantPanelProps) {
  const a = useAssistant();
  const [consentOpen, setConsentOpen] = useState(false);
  const [task, setTask] = useState<AssistantTask>(props.tasks[0] ?? "ask");
  // Lives here so the conversation survives switching tabs.
  const [chat, setChat] = useState<{ role: "user" | "assistant"; content: string }[]>([]);
  const active = props.tasks.includes(task) ? task : props.tasks[0];

  if (a.access === "hidden") return null;
  if (a.access !== "ready") return <Upsell access={a.access} onOpen={a.openConnect} />;
  if (!a.consented) {
    return (
      <div style={col}>
        <EmptyState
          icon={<Sparkles size={22} strokeWidth={1.75} />}
          title="Assistant"
          description="Looks up register maps, turns manual pages into read blocks, explains errors and decodes raw values. It reads and suggests; it never writes to a device."
          action={
            <Button variant="primary" onClick={() => setConsentOpen(true)}>
              Review and continue
            </Button>
          }
        />
        <ConsentSheet
          open={consentOpen}
          onClose={() => setConsentOpen(false)}
          onAgree={() => {
            a.grantConsent();
            setConsentOpen(false);
          }}
        />
      </div>
    );
  }
  const line = quotaLine(a.quota);
  return (
    <div style={col}>
      {props.tasks.length > 1 && <TaskPicker tasks={props.tasks} value={active} onChange={setTask} />}
      {active === "lookup" && <LookupView {...props} />}
      {active === "manual" && <ManualView {...props} />}
      {active === "diagnose" && <DiagnoseView {...props} />}
      {active === "decode" && <DecodeView />}
      {active === "ask" && <AskView {...props} chat={chat} setChat={setChat} />}
      {line && <div style={{ ...dim, fontSize: "var(--fs-xs)" }}>{line}</div>}
    </div>
  );
}

function Upsell({ access, onOpen }: { access: AssistantAccess; onOpen: () => void }) {
  return (
    <EmptyState
      icon={<Sparkles size={22} strokeWidth={1.75} />}
      title="Assistant"
      description={
        access === "signin"
          ? "Sign in to Voltrus Connect to use the Assistant: it finds register maps, reads manual pages, explains errors and decodes values."
          : "The Assistant is included with Voltrus Connect. It finds register maps, reads manual pages, explains errors and decodes values."
      }
      action={
        <Button variant="primary" onClick={onOpen}>
          Voltrus Connect
        </Button>
      }
    />
  );
}

// ── Shared pieces ──

/** One request at a time: busy flag, the failure to show, the last result. */
function useTask<T>(task: AssistantTask) {
  const { run, openConnect } = useAssistant();
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<AssistantFailure | null>(null);
  const [result, setResult] = useState<T | null>(null);
  const send = useCallback(
    async (payload: Record<string, unknown>): Promise<T | null> => {
      setBusy(true);
      setFailure(null);
      try {
        const out = await run<T>(task, payload);
        setResult(out);
        return out;
      } catch (e) {
        setResult(null);
        setFailure(isFailure(e) ? e : { status: 0, code: "error", message: String(e) });
        return null;
      } finally {
        setBusy(false);
      }
    },
    [run, task],
  );
  const clear = useCallback(() => {
    setResult(null);
    setFailure(null);
  }, []);
  const error = failure ? (
    <FailureNote failure={failure} onConnect={openConnect} />
  ) : null;
  return { busy, result, send, clear, error };
}

function FailureNote({ failure, onConnect }: { failure: AssistantFailure; onConnect: () => void }) {
  const info = failureInfo(failure);
  return (
    <Callout tone={info.kind === "info" ? "danger" : "warn"}>
      <div style={col}>
        <span>{info.text}</span>
        {(info.kind === "signin" || info.kind === "upgrade") && (
          <div>
            <Button size="sm" onClick={onConnect}>
              Voltrus Connect
            </Button>
          </div>
        )}
      </div>
    </Callout>
  );
}

const CONFIDENCE_TONE = { high: "success", medium: "warn", low: "neutral" } as const;

function SourceLine({ block }: { block: AssistantBlock }) {
  const s = block.source;
  if (!s) return null;
  if (s.origin === "web" && s.url) {
    const url = s.url;
    return (
      <button
        type="button"
        onClick={() => openLink(url)}
        style={{ ...dim, background: "none", border: 0, padding: 0, textAlign: "left", cursor: "pointer", textDecoration: "underline" }}
      >
        <ExternalLink size={11} style={{ verticalAlign: "-1px", marginRight: 4 }} />
        {s.vendor ? "Manufacturer" : "Third-party page"}: {s.title}
        {s.page ? `, ${s.page}` : ""}
      </button>
    );
  }
  return <span style={dim}>From your text: “{s.quote.length > 90 ? `${s.quote.slice(0, 90)}…` : s.quote}”</span>;
}

/** The review list: every block is a proposal the user can untick; nothing is added until "Add". */
function BlockProposals({
  blocks,
  onAddBlocks,
  notes,
}: {
  blocks: AssistantBlock[];
  onAddBlocks?: (blocks: AssistantBlock[]) => string | void;
  notes?: string;
}) {
  const [skipped, setSkipped] = useState<Set<number>>(new Set());
  const [done, setDone] = useState<string | null>(null);
  const chosen = blocks.filter((_, i) => !skipped.has(i));
  const order = commonWordOrder(chosen);
  if (blocks.length === 0) return <Callout tone="neutral">{notes ?? "No register table found in that."}</Callout>;
  return (
    <div style={col}>
      <div style={dim}>Review these before adding. Untick anything that looks wrong.</div>
      <ul style={{ listStyle: "none", margin: 0, padding: 0, display: "flex", flexDirection: "column", gap: 8 }}>
        {blocks.map((b, i) => (
          <li key={`${b.name}-${b.address}-${i}`} style={{ display: "flex", gap: 8, alignItems: "flex-start" }}>
            <input
              type="checkbox"
              aria-label={`Add ${b.name}`}
              checked={!skipped.has(i)}
              disabled={done !== null}
              onChange={(e) => {
                const next = new Set(skipped);
                if (e.target.checked) next.delete(i);
                else next.add(i);
                setSkipped(next);
              }}
              style={{ marginTop: 3 }}
            />
            <div style={{ display: "flex", flexDirection: "column", gap: 2, minWidth: 0, flex: 1 }}>
              <div style={row}>
                <strong>{b.name}</strong>
                <Badge tone={CONFIDENCE_TONE[b.confidence]}>{b.confidence}</Badge>
              </div>
              <span style={dim}>{blockSummary(b)}</span>
              {b.note && <span style={dim}>{b.note}</span>}
              <SourceLine block={b} />
            </div>
          </li>
        ))}
      </ul>
      {notes && <div style={dim}>{notes}</div>}
      {order && (
        <Callout tone="info">
          These registers use word order {WORD_ORDER_LABEL[order]}. Set it in the connection settings after adding.
        </Callout>
      )}
      {done ? (
        <Callout tone="success">{done}</Callout>
      ) : (
        onAddBlocks && (
          <div>
            <Button
              variant="primary"
              disabled={chosen.length === 0}
              onClick={() => setDone(onAddBlocks(chosen) || `Added ${chosen.length} ${chosen.length === 1 ? "block" : "blocks"}.`)}
            >
              Add {chosen.length === blocks.length ? "these blocks" : `${chosen.length} of ${blocks.length} blocks`}
            </Button>
          </div>
        )
      )}
    </div>
  );
}

function Photo({ file, onFile }: { file: File | null; onFile: (f: File | null) => void }) {
  const input = useRef<HTMLInputElement>(null);
  return (
    <div style={row}>
      <input
        ref={input}
        type="file"
        accept="image/*"
        hidden
        onChange={(e) => {
          onFile(e.target.files?.[0] ?? null);
          e.target.value = "";
        }}
      />
      <Button size="sm" icon={<ImagePlus size={14} />} onClick={() => input.current?.click()}>
        {file ? "Change photo" : "Add a photo or screenshot"}
      </Button>
      {file && (
        <span style={{ ...dim, display: "inline-flex", alignItems: "center", gap: 4 }}>
          {file.name}
          <button type="button" aria-label="Remove photo" onClick={() => onFile(null)} style={{ background: "none", border: 0, cursor: "pointer", color: "inherit", padding: 0 }}>
            <X size={13} />
          </button>
        </span>
      )}
    </div>
  );
}

const area = (rows = 5) => ({ className: "vt-textarea vt-textarea--mono", rows });

// ── Find registers ──

function LookupView({ onAddBlocks }: AssistantPanelProps) {
  const [maker, setMaker] = useState("");
  const [model, setModel] = useState("");
  const [text, setText] = useState("");
  const t = useTask<RegisterMapResult>("lookup");
  const go = (withText: boolean) =>
    t.send({ model: model.trim(), manufacturer: maker.trim(), ...(withText && text.trim() ? { source_text: text } : {}) });
  const needsManual = t.result?.needs_manual === true;
  return (
    <div style={col}>
      <div style={dim}>Name the device. If its register map is already known you get it straight away, with where it came from.</div>
      <Field label="Manufacturer">
        {(id) => <input id={id} className="vt-input" value={maker} onChange={(e) => setMaker(e.target.value)} placeholder="Schneider Electric" />}
      </Field>
      <Field label="Model">
        {(id) => <input id={id} className="vt-input" value={model} onChange={(e) => setModel(e.target.value)} placeholder="PM5560" />}
      </Field>
      <div>
        <Button variant="primary" loading={t.busy} disabled={model.trim().length < 2} onClick={() => go(false)}>
          Find registers
        </Button>
      </div>
      {t.error}
      {needsManual && (
        <>
          <Callout tone="info">{t.result?.notes}</Callout>
          <Field label="Register table from the manual" hint="Paste the rows as they are. Each block will quote the line it came from.">
            {(id) => <textarea id={id} {...area(6)} value={text} onChange={(e) => setText(e.target.value)} />}
          </Field>
          <div>
            <Button variant="primary" loading={t.busy} disabled={text.trim().length < 20} onClick={() => go(true)}>
              Read these registers
            </Button>
          </div>
        </>
      )}
      {t.result && !needsManual && <BlockProposals blocks={t.result.blocks} onAddBlocks={onAddBlocks} notes={t.result.notes} />}
    </div>
  );
}

// ── From manual ──

function ManualView({ onAddBlocks }: AssistantPanelProps) {
  const [text, setText] = useState("");
  const [file, setFile] = useState<File | null>(null);
  const [prep, setPrep] = useState<string | null>(null);
  const t = useTask<ManualResult>("manual");
  const go = async () => {
    setPrep(null);
    try {
      const image = file ? await fileToJpeg(file) : undefined;
      await t.send({ ...(text.trim() ? { text } : {}), ...(image ? { image } : {}) });
    } catch (e) {
      setPrep(e instanceof Error ? e.message : String(e));
    }
  };
  return (
    <div style={col}>
      <div style={dim}>Paste a register table, or add a photo or screenshot of the page. You review the blocks before anything is added.</div>
      <Field label="Register table">
        {(id) => <textarea id={id} {...area(6)} value={text} onChange={(e) => setText(e.target.value)} placeholder={"3028  Current A  2  Float32  A\n3030  Current B  2  Float32  A"} />}
      </Field>
      <Photo file={file} onFile={setFile} />
      {file && <div style={dim}>A photo uses one of your monthly photo requests.</div>}
      <div>
        <Button variant="primary" loading={t.busy} disabled={!text.trim() && !file} onClick={go}>
          Read it
        </Button>
      </div>
      {prep && <Callout tone="danger">{prep}</Callout>}
      {t.error}
      {t.result && <BlockProposals blocks={t.result.blocks} onAddBlocks={onAddBlocks} notes={t.result.notes} />}
    </div>
  );
}

// ── Diagnose ──

function DiagnoseView({ protocol, getContext }: AssistantPanelProps) {
  const ctx = getContext?.();
  const [kind, setKind] = useState<"exception" | "timeout" | "other">(ctx?.error?.kind ?? "timeout");
  const [code, setCode] = useState(ctx?.error?.code !== undefined ? String(ctx.error.code) : "");
  const [message, setMessage] = useState(ctx?.error?.message ?? "");
  const t = useTask<Diagnosis>("diagnose");
  const go = () => {
    const now = getContext?.();
    const parsed = kind === "exception" ? parseErrorCode(code) : undefined;
    void t.send({
      protocol,
      error: { kind, ...(parsed !== undefined ? { code: parsed } : {}), ...(message.trim() ? { message: message.trim() } : {}) },
      ...(now?.config !== undefined ? { config: now.config } : {}),
      ...(now?.block !== undefined ? { block: now.block } : {}),
      ...(now?.log?.length ? { log: now.log } : {}),
    });
  };
  return (
    <div style={col}>
      <div style={dim}>Explains an error or a timeout using your current connection settings and the last log lines.</div>
      <Field label="What happened">
        {(id) => (
          <select id={id} className="vt-select" value={kind} onChange={(e) => setKind(e.target.value as typeof kind)}>
            <option value="exception">The device answered with an error</option>
            <option value="timeout">No answer (timeout)</option>
            <option value="other">Something else</option>
          </select>
        )}
      </Field>
      {kind === "exception" && (
        <Field label="Error code" hint="For Modbus, the exception code such as 02 or 0x0B.">
          {(id) => <input id={id} className="vt-input" value={code} onChange={(e) => setCode(e.target.value)} placeholder="02" inputMode="numeric" />}
        </Field>
      )}
      <Field label="Message (optional)">
        {(id) => <input id={id} className="vt-input" value={message} onChange={(e) => setMessage(e.target.value)} />}
      </Field>
      <div>
        <Button variant="primary" loading={t.busy} onClick={go}>
          Diagnose
        </Button>
      </div>
      {t.error}
      {t.result && <DiagnosisView d={t.result} />}
    </div>
  );
}

function DiagnosisView({ d }: { d: Diagnosis }) {
  return (
    <div style={col}>
      <div>{d.summary}</div>
      <div style={col}>
        {d.causes.map((c) => (
          <div key={c.title} style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            <div style={row}>
              <strong>{c.title}</strong>
              <Badge tone={CONFIDENCE_TONE[c.likelihood]}>{c.likelihood}</Badge>
            </div>
            <span style={dim}>{c.detail}</span>
          </div>
        ))}
      </div>
      {d.checks.length > 0 && (
        <div>
          <strong>Check</strong>
          <ul style={{ margin: "4px 0 0", paddingLeft: 18, listStyle: "disc", ...dim }}>
            {d.checks.map((c) => (
              <li key={c}>{c}</li>
            ))}
          </ul>
        </div>
      )}
      {d.suggestions.length > 0 && (
        <div>
          <strong>Settings to try</strong>
          <ul style={{ margin: "4px 0 0", paddingLeft: 18, listStyle: "disc", ...dim }}>
            {d.suggestions.map((s) => (
              <li key={`${s.setting}-${String(s.to)}`}>
                {s.setting.replace(/_/g, " ")}: {s.from !== undefined ? `${String(s.from)} → ` : ""}
                {String(s.to)}. {s.reason}
              </li>
            ))}
          </ul>
          <div style={{ ...dim, marginTop: 4 }}>You change these yourself; nothing is applied for you.</div>
        </div>
      )}
    </div>
  );
}

// ── Decode ──

function DecodeView() {
  const [regs, setRegs] = useState("");
  const [label, setLabel] = useState("");
  const [unit, setUnit] = useState("");
  const [expected, setExpected] = useState("");
  const [problem, setProblem] = useState<string | null>(null);
  const t = useTask<{ candidates: DecodeCandidate[] }>("decode");
  const go = () => {
    const parsed = parseRegisters(regs);
    if (typeof parsed === "string") return setProblem(parsed);
    setProblem(null);
    void t.send({
      registers: parsed,
      ...(label.trim() ? { label: label.trim() } : {}),
      ...(unit.trim() ? { unit: unit.trim() } : {}),
      ...(expected.trim() ? { expected: expected.trim() } : {}),
    });
  };
  return (
    <div style={col}>
      <div style={dim}>Not sure how a value is encoded? Enter the raw registers and what the value should be.</div>
      <Field label="Raw registers" error={problem} hint="Decimal or hex, e.g. 16840 0 or 0x41C8 0x0000.">
        {(id) => <input id={id} className="vt-input vt-input--mono" value={regs} onChange={(e) => setRegs(e.target.value)} placeholder="0x41C8 0x0000" />}
      </Field>
      <Field label="What is it (optional)">
        {(id) => <input id={id} className="vt-input" value={label} onChange={(e) => setLabel(e.target.value)} placeholder="Voltage L1" />}
      </Field>
      <div style={row}>
        <Field label="Unit">
          {(id) => <input id={id} className="vt-input" value={unit} onChange={(e) => setUnit(e.target.value)} placeholder="V" style={{ width: 90 }} />}
        </Field>
        <Field label="Expected range">
          {(id) => <input id={id} className="vt-input" value={expected} onChange={(e) => setExpected(e.target.value)} placeholder="200 to 250" style={{ width: 150 }} />}
        </Field>
      </div>
      <div>
        <Button variant="primary" loading={t.busy} onClick={go}>
          Decode
        </Button>
      </div>
      {t.error}
      {t.result &&
        (t.result.candidates.length === 0 ? (
          <Callout tone="neutral">None of the usual encodings gives a sensible value for these registers.</Callout>
        ) : (
          <ul style={{ listStyle: "none", margin: 0, padding: 0, display: "flex", flexDirection: "column", gap: 10 }}>
            {t.result.candidates.map((c, i) => (
              <li key={i} style={{ display: "flex", flexDirection: "column", gap: 2 }}>
                <div style={row}>
                  <strong style={{ fontVariantNumeric: "tabular-nums" }}>
                    {c.value}
                    {unit.trim() ? ` ${unit.trim()}` : ""}
                  </strong>
                  <Badge tone={CONFIDENCE_TONE[c.confidence]}>{c.confidence}</Badge>
                </div>
                <span style={dim}>
                  {c.type}, word order {WORD_ORDER_LABEL[c.word_order]}
                  {c.scale !== 1 ? `, scale ${c.scale}` : ""}, starting at register {c.register_index + 1}
                </span>
                <span style={dim}>{c.reason}</span>
              </li>
            ))}
          </ul>
        ))}
    </div>
  );
}

// ── Ask ──

function AskView({
  protocol,
  getContext,
  chat,
  setChat,
}: AssistantPanelProps & {
  chat: { role: "user" | "assistant"; content: string }[];
  setChat: (c: { role: "user" | "assistant"; content: string }[]) => void;
}) {
  const [question, setQuestion] = useState("");
  const [followUps, setFollowUps] = useState<string[]>([]);
  const t = useTask<AnswerResult>("ask");
  const ask = async (q: string) => {
    const now = getContext?.();
    const history = chat.slice(-6);
    const next = [...chat, { role: "user" as const, content: q }];
    setChat(next);
    setQuestion("");
    setFollowUps([]);
    const out = await t.send({
      protocol,
      question: q,
      history,
      ...(now?.config !== undefined ? { config: now.config } : {}),
      ...(now?.log?.length ? { log: now.log } : {}),
    });
    if (out) {
      setChat([...next, { role: "assistant" as const, content: out.answer }]);
      setFollowUps(out.follow_ups);
    }
  };
  return (
    <div style={col}>
      <div style={dim}>Ask about this session. The Assistant sees your connection settings and the last log lines, never passwords or addresses.</div>
      {chat.length > 0 && (
        <div style={col} aria-live="polite">
          {chat.map((m, i) => (
            <div
              key={i}
              style={{
                alignSelf: m.role === "user" ? "flex-end" : "flex-start",
                maxWidth: "92%",
                padding: "6px 10px",
                borderRadius: 10,
                background: m.role === "user" ? "var(--accent-soft, var(--bg-elev, rgba(127,127,127,.15)))" : "var(--bg-elev, rgba(127,127,127,.1))",
                whiteSpace: "pre-wrap",
              }}
            >
              {m.content}
            </div>
          ))}
        </div>
      )}
      {t.error}
      {followUps.length > 0 && (
        <div style={row}>
          {followUps.map((f) => (
            <Button key={f} size="sm" onClick={() => void ask(f)}>
              {f}
            </Button>
          ))}
        </div>
      )}
      <Field label="Your question">
        {(id) => (
          <textarea
            id={id}
            className="vt-textarea"
            rows={3}
            value={question}
            onChange={(e) => setQuestion(e.target.value)}
            placeholder="Why do my float32 values look wrong?"
          />
        )}
      </Field>
      <div style={row}>
        <Button variant="primary" loading={t.busy} disabled={question.trim().length === 0} onClick={() => void ask(question.trim())}>
          Ask
        </Button>
        {chat.length > 0 && (
          <Button
            variant="ghost"
            onClick={() => {
              setChat([]);
              setFollowUps([]);
              t.clear();
            }}
          >
            New conversation
          </Button>
        )}
      </div>
    </div>
  );
}
