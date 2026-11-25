import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent, type MouseEvent } from "react";
import { Badge, Button, Callout, StatusDot } from "@mactools/ui";
import { Cpu, Eye, EyeOff, ScanSearch, Search, Tags, Trash2 } from "lucide-react";
import { s7 } from "../../lib/s7";
import { errorText, useS7 } from "../../lib/s7Store";
import { listDeviceDocs, readDoc, tagDocName, updateDoc, writeProjectFile, openDoc } from "../../lib/deviceProject";
import { docTitle, parseTagDoc, serializeTagDoc, type TagDoc } from "../../lib/s7Docs";
import {
  addressConflicts,
  catalogFor,
  channelsOf,
  channelTags,
  cpuFromOrderCode,
  entryOf,
  FAMILIES,
  familyFromOrderCode,
  freeSlotFor,
  parseHwDoc,
  placeModule,
  rangesOf,
  rangeText,
  serializeHwDoc,
  slotsOf,
  type CatalogEntry,
  type Family,
  type HwDoc,
  type HwModule,
  type ModuleKind,
} from "../../lib/s7Hw";
import { useStore } from "../../store";
import { ModeBadge } from "./common";

const UNIT = 54; // px per width unit on the rail
const POLL_MS = 500;

const KIND_LABEL: Record<ModuleKind, string> = {
  cpu: "CPU", ps: "Power supply", im: "Interface", di: "Digital input", dq: "Digital output",
  dio: "Digital in/out", ai: "Analog input", aq: "Analog output", cm: "Communication",
};

/** The device configuration editor: rack view, device overview, catalog and properties. */
export default function DeviceView({ path, value, onChange }: { path: string; value: string; onChange: (v: string) => void }) {
  const parsed = useMemo(() => {
    try {
      return { doc: parseHwDoc(value), error: null };
    } catch (e) {
      return { doc: null, error: errorText(e) };
    }
  }, [value]);
  if (!parsed.doc) {
    return <div className="s7-doc"><Callout tone="danger">This device configuration cannot be read: {parsed.error}</Callout></div>;
  }
  return <Device path={path} doc={parsed.doc} onChange={(d) => onChange(serializeHwDoc(d))} />;
}

/** Live I/O: byte and word values by address (`IB8` -> 0x0F, `IW96` -> 1234). */
type IoValues = Record<string, number>;

function Device({ path, doc, onChange }: { path: string; doc: HwDoc; onChange: (d: HwDoc) => void }) {
  const { info, mode, monitoring: monitoringMap, setMonitoring } = useS7();
  const addLog = useStore((s) => s.addLog);
  const online = info !== null;
  const monitoring = online && !!monitoringMap[path];
  const [selected, setSelected] = useState<number | null>(doc.modules.find((m) => entryOf(doc.family, m.catalogId)?.kind === "cpu")?.slot ?? null);
  const [panel, setPanel] = useState<"catalog" | "properties">("catalog");
  const [values, setValues] = useState<IoValues>({});
  const [ioError, setIoError] = useState<string | null>(null);
  const docRef = useRef(doc);
  docRef.current = doc;
  const reading = useRef(false);

  const module = doc.modules.find((m) => m.slot === selected) ?? null;
  const conflicts = addressConflicts(doc);

  const select = (slot: number | null) => {
    setSelected(slot);
    if (slot !== null && doc.modules.some((m) => m.slot === slot)) setPanel("properties");
  };

  const place = (entry: CatalogEntry, slot?: number) => {
    const target = slot ?? (selected !== null && slotsOf(doc.family).find((s) => s.slot === selected)?.accepts.includes(entry.kind) && !doc.modules.some((m) => m.slot === selected) ? selected : freeSlotFor(doc, entry));
    if (target === undefined || target === null) {
      addLog("system", `No free slot takes a ${entry.label}`);
      return;
    }
    onChange(placeModule(doc, entry, target));
    // Stay in the catalog so several modules can go in one after another.
    setSelected(target);
  };

  const patchModule = (slot: number, p: Partial<HwModule>) =>
    onChange({ ...doc, modules: doc.modules.map((m) => (m.slot === slot ? { ...m, ...p } : m)) });

  const remove = (slot: number) => {
    onChange({ ...doc, modules: doc.modules.filter((m) => m.slot !== slot) });
    setSelected(null);
    setPanel("catalog");
  };

  // Live I/O: one BYTE read per digital byte, one INT per analog channel.
  const tick = useCallback(async () => {
    if (reading.current) return;
    const d = docRef.current;
    const rows: { address: string; dataType: string }[] = [];
    for (const m of d.modules) {
      for (const c of channelsOf(d.family, m)) {
        if (c.dataType === "Int") rows.push({ address: c.address, dataType: "INT" });
        else {
          const byte = `${c.address[0]}B${c.address.slice(1).split(".")[0]}`;
          if (!rows.some((r) => r.address === byte)) rows.push({ address: byte, dataType: "BYTE" });
        }
      }
    }
    if (rows.length === 0) return;
    reading.current = true;
    try {
      const res = await s7.readWatch(rows);
      const next: IoValues = {};
      res.forEach((r, i) => {
        if (r.value === null) return;
        next[rows[i].address] = rows[i].dataType === "BYTE" ? parseInt(r.value.replace("16#", ""), 16) : Number(r.value);
      });
      setValues(next);
      const firstError = res.find((r) => r.error)?.error ?? null;
      setIoError(firstError);
    } catch (e) {
      setIoError(errorText(e));
    } finally {
      reading.current = false;
    }
  }, []);

  useEffect(() => {
    if (!monitoring) {
      setValues({});
      return;
    }
    void tick();
    const id = window.setInterval(tick, POLL_MS);
    return () => window.clearInterval(id);
  }, [monitoring, tick]);

  const readIdentity = () => {
    const cpu = info?.cpu;
    if (!cpu) {
      addLog("system", "The CPU did not give its identity");
      return;
    }
    const family = familyFromOrderCode(cpu.order_code) ?? doc.family;
    const base: HwDoc = family === doc.family ? doc : { ...doc, family, modules: [] };
    const slot = slotsOf(family).find((s) => s.accepts.includes("cpu"))!.slot;
    const existing = base.modules.find((m) => m.slot === slot);
    // An order number this catalog does not know keeps the CPU already in the rack.
    const entry = cpuFromOrderCode(family, cpu.order_code, cpu.module_type)
      ?? (existing ? entryOf(family, existing.catalogId) : undefined)
      ?? catalogFor(family).find((e) => e.kind === "cpu")!;
    let next = existing?.catalogId === entry.id ? base : placeModule(base, entry, slot);
    next = {
      ...next,
      modules: next.modules.map((m) => (m.slot === slot
        ? { ...m, name: cpu.as_name || cpu.module_name || m.name, articleNo: cpu.order_code || m.articleNo, firmware: cpu.firmware_version || m.firmware }
        : m)),
    };
    onChange(next);
    setSelected(slot);
    setPanel("properties");
    addLog("system", `Read the CPU identity: ${cpu.module_type || entry.label}, ${cpu.order_code}, firmware ${cpu.firmware_version}`);
  };

  const createTags = async (modules: HwModule[]) => {
    const rows = channelTags(doc, modules);
    if (rows.length === 0) {
      addLog("system", "These modules have no channels with addresses");
      return;
    }
    const cpuName = doc.modules.find((m) => entryOf(doc.family, m.catalogId)?.kind === "cpu")?.name || "PLC";
    const base = `${cpuName.replace(/[^A-Za-z0-9_]+/g, "_")}_IO`;
    try {
      const existing = listDeviceDocs("tags").find((t) => docTitle(t.name) === base);
      if (existing) {
        const tagDoc = parseTagDoc(await readDoc(existing.path));
        const byAddress = new Map(tagDoc.rows.map((r) => [r.address.toUpperCase(), r]));
        let added = 0;
        for (const r of rows) {
          if (byAddress.has(r.address.toUpperCase())) continue;
          tagDoc.rows.push(r);
          added++;
        }
        await updateDoc(existing.path, serializeTagDoc(tagDoc));
        await openDoc(existing.path);
        addLog("system", `Added ${added} channel tag(s) to ${base}`);
      } else {
        const tagDoc: TagDoc = { format: "voltrus-s7-tags", version: 1, kind: "tags", rows };
        await writeProjectFile(tagDocName(base), serializeTagDoc(tagDoc));
        addLog("system", `Created ${base} with ${rows.length} channel tag(s)`);
      }
    } catch (e) {
      addLog("system", `Could not create the channel tags: ${errorText(e)}`);
    }
  };

  const cpuModule = doc.modules.find((m) => entryOf(doc.family, m.catalogId)?.kind === "cpu");

  return (
    <div className="s7-doc">
      <header className="s7-doc__header">
        <Cpu size={16} strokeWidth={1.75} />
        <span className="s7-doc__title">{cpuModule?.name ?? "Device"}</span>
        <span className="s7-muted">Device configuration · {doc.family} · {doc.modules.length} module(s)</span>
        <span className="s7-doc__spacer" />
        {online ? (
          <>
            <StatusDot tone="success" label={`Online: ${info.target.host}`} />
            <ModeBadge mode={mode} />
          </>
        ) : (
          <StatusDot tone="neutral" label="Offline" />
        )}
      </header>
      <div className="s7-doc__toolbar">
        <label className="s7-row s7-dbno">
          Family
          <select className="vt-select vt-select--sm" value={doc.family}
            onChange={(e) => {
              const family = e.target.value as Family;
              if (doc.modules.length && !window.confirm(`Change to ${family}? The modules of this rack are removed.`)) return;
              onChange({ ...doc, family, modules: [] });
              setSelected(null);
            }}>
            {FAMILIES.map((f) => <option key={f} value={f}>{f}</option>)}
          </select>
        </label>
        <span className="vt-divider--v" />
        <Button size="sm" icon={<ScanSearch size={14} />} disabled={!online} onClick={readIdentity}
          title={online ? "Put the connected CPU, with its order number and firmware, into the rack" : "Go online first"}>
          Read CPU from device
        </Button>
        <Button size="sm" variant={monitoring ? "primary" : "default"} disabled={!online || doc.modules.length === 0}
          icon={monitoring ? <EyeOff size={14} /> : <Eye size={14} />}
          onClick={() => setMonitoring(path, !monitoring)}>
          {monitoring ? "Stop monitoring I/O" : "Monitor I/O"}
        </Button>
        <Button size="sm" variant="ghost" icon={<Tags size={14} />} disabled={doc.modules.length === 0}
          onClick={() => void createTags(doc.modules)}>
          Create tags for all channels
        </Button>
      </div>
      {conflicts.map((c) => <Callout key={c} tone="danger">{c}</Callout>)}
      {monitoring && ioError && <Callout tone="warn">Some I/O could not be read: {ioError}</Callout>}

      <div className="s7-dv">
        <div className="s7-dv__main">
          <div className="s7-rack-wrap" onClick={() => select(null)}>
            <Rack doc={doc} selected={selected} onSelect={select} onDrop={place}
              online={online} mode={mode} values={monitoring ? values : null} />
          </div>
          <div className="s7-dv__overview">
            <div className="s7-od__detail-title">Device overview</div>
            <div className="s7-grid-wrap">
              <table className="vt-table s7-grid">
                <thead>
                  <tr><th>Module</th><th>Slot</th><th>I address</th><th>Q address</th><th>Type</th><th>Article no.</th><th>Firmware</th><th>Comment</th></tr>
                </thead>
                <tbody>
                  {[...doc.modules].sort((a, b) => a.slot - b.slot).map((m) => {
                    const e = entryOf(doc.family, m.catalogId);
                    const r = rangesOf(doc.family, m);
                    return (
                      <tr key={m.slot} aria-selected={m.slot === selected} onClick={() => select(m.slot)}>
                        <td>{m.name}</td>
                        <td className="mono">{m.slot}</td>
                        <td className="mono">{rangeText(r.i)}</td>
                        <td className="mono">{rangeText(r.q)}</td>
                        <td>{e?.label ?? m.catalogId}</td>
                        <td className="mono">{m.articleNo ?? ""}</td>
                        <td className="mono">{m.firmware ?? ""}</td>
                        <td>{m.comment ?? ""}</td>
                      </tr>
                    );
                  })}
                  {doc.modules.length === 0 && (
                    <tr><td colSpan={8} className="s7-muted">Empty rack: add a CPU from the catalog, or go online and read it from the device.</td></tr>
                  )}
                </tbody>
              </table>
            </div>
          </div>
        </div>
        <aside className="s7-dv__side">
          <div className="vt-tabs s7-dv__tabs">
            <button className="vt-tab" aria-selected={panel === "catalog"} onClick={() => setPanel("catalog")}>Catalog</button>
            <button className="vt-tab" aria-selected={panel === "properties"} onClick={() => setPanel("properties")}>Properties</button>
          </div>
          {panel === "catalog" ? (
            <Catalog family={doc.family} onAdd={(e) => place(e)} />
          ) : module ? (
            <Properties doc={doc} module={module} values={monitoring ? values : null}
              onPatch={(p) => patchModule(module.slot, p)} onRemove={() => remove(module.slot)}
              onCreateTags={() => void createTags([module])} />
          ) : (
            <p className="s7-muted s7-dv__pad">Select a module in the rack.</p>
          )}
        </aside>
      </div>
    </div>
  );
}

function bitOn(values: IoValues | null, address: string): boolean | null {
  if (!values) return null;
  const [area, rest] = [address[0], address.slice(1)];
  const [byte, bit] = rest.split(".").map(Number);
  const v = values[`${area}B${byte}`];
  return v === undefined ? null : ((v >> bit) & 1) === 1;
}

function Rack({ doc, selected, onSelect, onDrop, online, mode, values }: {
  doc: HwDoc;
  selected: number | null;
  onSelect: (slot: number) => void;
  onDrop: (entry: CatalogEntry, slot: number) => void;
  online: boolean;
  mode: string | null;
  values: IoValues | null;
}) {
  const [over, setOver] = useState<number | null>(null);
  const slots = slotsOf(doc.family);
  const bySlot = new Map(doc.modules.map((m) => [m.slot, m]));

  const drop = (e: DragEvent, slot: number) => {
    e.preventDefault();
    setOver(null);
    const id = e.dataTransfer.getData("text/x-s7-module");
    const entry = entryOf(doc.family, id);
    const info = slots.find((s) => s.slot === slot);
    if (entry && info?.accepts.includes(entry.kind)) onDrop(entry, slot);
  };

  return (
    <div className="s7-rack" role="group" aria-label={`${doc.family} rack`}>
      <div className="s7-rack__label">Rack 0</div>
      <div className="s7-rack__slots">
        {slots.map((s) => {
          const m = bySlot.get(s.slot);
          const e = m ? entryOf(doc.family, m.catalogId) : undefined;
          const width = (e?.width ?? 1) * UNIT;
          return (
            <div key={s.slot} className="s7-slot" style={{ width }}
              onDragOver={(ev) => { ev.preventDefault(); setOver(s.slot); }}
              onDragLeave={() => setOver((o) => (o === s.slot ? null : o))}
              onDrop={(ev) => drop(ev, s.slot)}>
              <div className="s7-slot__no">{s.label}</div>
              {m && e ? (
                <Module module={m} entry={e} family={doc.family} selected={selected === s.slot}
                  onClick={(ev) => { ev.stopPropagation(); onSelect(s.slot); }}
                  online={online} mode={mode} values={values} />
              ) : (
                <button className={`s7-slot__empty ${over === s.slot ? "is-over" : ""} ${selected === s.slot ? "is-selected" : ""}`}
                  aria-label={`Empty slot ${s.label}`}
                  onClick={(ev) => { ev.stopPropagation(); onSelect(s.slot); }} />
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function Module({ module: m, entry: e, family, selected, onClick, online, mode, values }: {
  module: HwModule;
  entry: CatalogEntry;
  family: Family;
  selected: boolean;
  onClick: (ev: MouseEvent) => void;
  online: boolean;
  mode: string | null;
  values: IoValues | null;
}) {
  const channels = channelsOf(family, m).filter((c) => c.kind === "di" || c.kind === "dq");
  const isCpu = e.kind === "cpu";
  const narrow = e.width === 1;
  return (
    <button className={`s7-module s7-module--${e.kind} ${selected ? "is-selected" : ""}`} onClick={onClick}
      title={`${m.name}: ${e.label}${m.articleNo ? ` (${m.articleNo})` : ""}`}>
      <span className="s7-module__band" />
      <span className="s7-module__leds">
        {isCpu ? (
          <>
            <Led label="RUN" on={online && mode === "RUN"} tone="run" />
            <Led label="STOP" on={online && mode === "STOP"} tone="stop" />
            <Led label="ERR" on={false} tone="err" />
          </>
        ) : (
          <Led label="DIAG" on={online} tone="run" />
        )}
      </span>
      <span className={`s7-module__name ${narrow ? "is-vertical" : ""}`}>{narrow ? e.label.split(" ×")[0] : e.label}</span>
      {channels.length > 0 && (
        <span className="s7-module__channels" style={{ gridTemplateColumns: `repeat(${Math.min(2, Math.ceil(channels.length / 8))}, 1fr)` }}>
          {channels.slice(0, 32).map((c) => {
            const on = bitOn(values, c.address);
            return <span key={c.address} className={`s7-ch ${on ? `s7-ch--on s7-ch--${c.kind}` : ""}`} title={`${c.label} ${c.address}${on === null ? "" : on ? " = 1" : " = 0"}`} />;
          })}
        </span>
      )}
      <span className="s7-module__foot">{m.name}</span>
    </button>
  );
}

function Led({ label, on, tone }: { label: string; on: boolean; tone: "run" | "stop" | "err" }) {
  return (
    <span className="s7-led-row">
      <span className={`s7-led ${on ? `s7-led--${tone}` : ""}`} />
      <span className="s7-led-label">{label}</span>
    </span>
  );
}

function Catalog({ family, onAdd }: { family: Family; onAdd: (e: CatalogEntry) => void }) {
  const [q, setQ] = useState("");
  const entries = catalogFor(family).filter((e) => e.label.toLowerCase().includes(q.toLowerCase()));
  const groups = new Map<ModuleKind, CatalogEntry[]>();
  for (const e of entries) groups.set(e.kind, [...(groups.get(e.kind) ?? []), e]);
  return (
    <div className="s7-catalog">
      <label className="s7-catalog__search">
        <Search size={14} />
        <input className="vt-input vt-input--sm" placeholder="Search the catalog" value={q} onChange={(e) => setQ(e.target.value)} />
      </label>
      <div className="s7-catalog__list">
        {[...groups.entries()].map(([kind, list]) => (
          <div key={kind}>
            <div className="s7-od__group">{KIND_LABEL[kind]}</div>
            {list.map((e) => (
              <div key={e.id} className="s7-catalog__item" draggable
                onDragStart={(ev) => { ev.dataTransfer.setData("text/x-s7-module", e.id); ev.dataTransfer.effectAllowed = "copy"; }}
                onDoubleClick={() => onAdd(e)}>
                <span className={`s7-catalog__swatch s7-module--${e.kind}`} />
                <span className="s7-catalog__label">{e.label}</span>
                <button className="s7-catalog__add" onClick={() => onAdd(e)} aria-label={`Add ${e.label}`}>Add</button>
              </div>
            ))}
          </div>
        ))}
      </div>
      <p className="s7-muted s7-dv__pad">Drag a module onto a slot, or click Add to put it in the selected or the first free slot.</p>
    </div>
  );
}

function Properties({ doc, module: m, values, onPatch, onRemove, onCreateTags }: {
  doc: HwDoc;
  module: HwModule;
  values: IoValues | null;
  onPatch: (p: Partial<HwModule>) => void;
  onRemove: () => void;
  onCreateTags: () => void;
}) {
  const e = entryOf(doc.family, m.catalogId);
  const channels = channelsOf(doc.family, m);
  const r = rangesOf(doc.family, m);
  const numberField = (label: string, key: "iStart" | "qStart", show: boolean) => show && (
    <label className="s7-prop">
      <span>{label}</span>
      <input className="vt-input vt-input--sm vt-input--mono" inputMode="numeric" value={m[key] ?? ""}
        onChange={(ev) => { const d = ev.target.value.replace(/\D/g, ""); onPatch({ [key]: d ? Number(d) : undefined }); }} />
    </label>
  );
  const isCpu = e?.kind === "cpu";
  return (
    <div className="s7-props">
      <div className="s7-props__head">
        <span className={`s7-catalog__swatch s7-module--${e?.kind ?? "cm"}`} />
        <strong>{e?.label ?? m.catalogId}</strong>
        <Badge tone="neutral" mono>Slot {m.slot}</Badge>
      </div>
      <label className="s7-prop"><span>Name</span>
        <input className="vt-input vt-input--sm" value={m.name} onChange={(ev) => onPatch({ name: ev.target.value })} />
      </label>
      <label className="s7-prop"><span>Article no.</span>
        <input className="vt-input vt-input--sm vt-input--mono" value={m.articleNo ?? ""} placeholder="6ES7 …"
          onChange={(ev) => onPatch({ articleNo: ev.target.value || undefined })} />
      </label>
      <label className="s7-prop"><span>Firmware</span>
        <input className="vt-input vt-input--sm vt-input--mono" value={m.firmware ?? ""} placeholder="V4.6"
          onChange={(ev) => onPatch({ firmware: ev.target.value || undefined })} />
      </label>
      {numberField(isCpu ? "DI start byte" : "Input start byte", "iStart", isCpu ? !!e?.di : r.i.length > 0 || !!(e?.di || e?.ai))}
      {numberField(isCpu ? "DQ start byte" : "Output start byte", "qStart", isCpu ? !!e?.dq : r.q.length > 0 || !!(e?.dq || e?.aq))}
      <label className="s7-prop"><span>Comment</span>
        <input className="vt-input vt-input--sm" value={m.comment ?? ""} onChange={(ev) => onPatch({ comment: ev.target.value || undefined })} />
      </label>
      {channels.length > 0 && (
        <>
          <div className="s7-od__group">Channels</div>
          <div className="s7-props__channels">
            {channels.map((c) => {
              const v = c.dataType === "Bool" ? bitOn(values, c.address) : values?.[c.address];
              return (
                <div key={c.address} className="s7-props__ch">
                  <span>{c.label}</span>
                  <span className="mono">{c.address}</span>
                  <span className="mono s7-props__val">{v === null || v === undefined ? "" : typeof v === "boolean" ? (v ? "1" : "0") : v}</span>
                </div>
              );
            })}
          </div>
        </>
      )}
      <div className="s7-row">
        <Button size="sm" icon={<Tags size={14} />} disabled={channels.length === 0} onClick={onCreateTags}>Create tags</Button>
        <Button size="sm" variant="danger" icon={<Trash2 size={14} />} onClick={onRemove}>Remove</Button>
      </div>
    </div>
  );
}
