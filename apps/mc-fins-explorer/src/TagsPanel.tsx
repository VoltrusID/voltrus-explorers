import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Download, FileDown, FileUp, Pencil, Trash2 } from "lucide-react";
import { Button, Dialog, Field, IconButton, ProBadge, Section, SheetSelect, openTextFile, saveTextFile, useProStatus } from "@mactools/ui";
import type { Device } from "./types";
import { ADDRESS_HINT, DATA_TYPES } from "./types";
import { FREE_TAG_LIMIT, guessType, uid } from "./helpers";
import { parseTagCsv, tagsToCsv, type TagCsvImport } from "./tagCsv";
import { useStore } from "./store";

const TYPE_OPTIONS = DATA_TYPES.map((dt) => ({ value: dt, label: dt }));

interface Props {
  device: Device;
  /** Replace the tags with the simulator template of this device's protocol. */
  onLoadSim?: () => void;
}

export default function TagsPanel({ device, onLoadSim }: Props) {
  const setDeviceTags = useStore((s) => s.setDeviceTags);
  const addLog = useStore((s) => s.addLog);
  const { pro, requirePro } = useProStatus();
  const tags = device.tags;
  const [editingId, setEditingId] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [addr, setAddr] = useState("");
  const [type, setType] = useState("Int16");
  const [addrError, setAddrError] = useState<string | null>(null);
  const [pending, setPending] = useState<(TagCsvImport & { path: string }) | null>(null);

  const reset = () => { setEditingId(null); setName(""); setAddr(""); setAddrError(null); };

  /** The backend parses the address exactly as a read would. */
  const validate = async (): Promise<boolean> => {
    try {
      const kind = await invoke<string>("check_address", { protocol: device.protocol, address: addr, xyOctal: device.mc.xyOctal });
      const a = addr.trim().toUpperCase();
      if (type === "Bool" && kind === "word") {
        setAddrError(`${a} is a word. For Bool give a bit, for example ${a}.0${device.protocol === "fins" ? "0" : ""}`);
        return false;
      }
      if (type !== "Bool" && a.includes(".")) {
        setAddrError(`${a} is a single bit: use the Bool type`);
        return false;
      }
      setAddrError(null);
      return true;
    } catch (e) {
      setAddrError(String(e));
      return false;
    }
  };

  const save = async () => {
    if (!addr.trim() || !(await validate())) return;
    const tag = { name: name.trim() || addr.trim().toUpperCase(), address: addr.trim().toUpperCase(), data_type: type };
    if (editingId) {
      setDeviceTags(device.id, (prev) => prev.map((t) => (t.id === editingId ? { ...t, ...tag } : t)));
    } else {
      if (tags.length >= FREE_TAG_LIMIT && !requirePro("More than 10 live tags")) return;
      setDeviceTags(device.id, (prev) => [...prev, { id: uid(), ...tag }]);
    }
    reset();
  };

  const exportCsv = async () => {
    if (!requirePro("Tag list CSV export")) return;
    try {
      const path = await saveTextFile(`${device.label}-tags.csv`, tagsToCsv(tags));
      if (path) addLog("info", `[${device.label}] Exported ${tags.length} tags to ${path}`);
    } catch (e) {
      addLog("error", `[${device.label}] Tag export failed: ${String(e)}`);
    }
  };

  const importCsv = async () => {
    if (!requirePro("Tag list CSV import")) return;
    try {
      const file = await openTextFile();
      if (!file) return;
      const parsed = parseTagCsv(file.text);
      parsed.errors.forEach((m) => addLog("error", `${file.path}: ${m}`));
      if (parsed.tags.length === 0) {
        addLog("error", `[${device.label}] No usable tags in ${file.path}`);
        return;
      }
      setPending({ ...parsed, path: file.path });
    } catch (e) {
      addLog("error", `[${device.label}] Tag import failed: ${String(e)}`);
    }
  };

  const applyImport = (mode: "replace" | "add") => {
    if (!pending) return;
    const incoming = pending.tags.map((t) => ({ id: uid(), ...t }));
    if (mode === "replace") setDeviceTags(device.id, incoming);
    else {
      const have = new Set(tags.map((t) => `${t.name}\u0000${t.address}`));
      setDeviceTags(device.id, (prev) => [...prev, ...incoming.filter((t) => !have.has(`${t.name}\u0000${t.address}`))]);
    }
    addLog("info", `[${device.label}] Imported ${incoming.length} tags from ${pending.path}`);
    setPending(null);
  };

  return (
    <Section
      title="Tags"
      meta={pro || tags.length <= FREE_TAG_LIMIT ? (pro ? String(tags.length) : `${tags.length} of ${FREE_TAG_LIMIT}`) : `${FREE_TAG_LIMIT} of ${tags.length} polled`}
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 8, padding: 14 }}>
        {onLoadSim && (
          <Button size="sm" icon={<Download size={14} strokeWidth={1.75} />} onClick={onLoadSim} title="Replace this PLC's tags with the simulator template's points">
            Load simulator tags
          </Button>
        )}
        <div style={{ display: "flex", gap: 6 }}>
          <Button size="sm" style={{ flex: 1 }} icon={<FileUp size={14} strokeWidth={1.75} />} onClick={importCsv} title="Import Name,Address,Type rows from a CSV file">
            Import <ProBadge />
          </Button>
          <Button size="sm" style={{ flex: 1 }} icon={<FileDown size={14} strokeWidth={1.75} />} disabled={tags.length === 0} onClick={exportCsv}>
            Export <ProBadge />
          </Button>
        </div>

        <Field label="Address" error={addrError ?? undefined} hint={addrError ? undefined : ADDRESS_HINT[device.protocol]}>
          {(id) => (
            <input
              id={id}
              className="vt-input mono"
              placeholder={device.protocol === "mc" ? "D100" : "D100 or CIO0.05"}
              value={addr}
              onChange={(e) => { setAddr(e.target.value); setType((t) => guessType(device.protocol, e.target.value, t)); setAddrError(null); }}
              onKeyDown={(e) => { if (e.key === "Enter") void save(); }}
            />
          )}
        </Field>
        <div style={{ display: "flex", gap: 6 }}>
          <div style={{ flex: 1 }}>
            <Field label="Name">
              {(id) => <input id={id} className="vt-input" placeholder="Optional" value={name} onChange={(e) => setName(e.target.value)} />}
            </Field>
          </div>
          <div style={{ width: 104 }}>
            <Field label="Type">
              {(id) => <SheetSelect id={id} sheetTitle="Type" value={type} options={TYPE_OPTIONS} onChange={(v) => setType(v)} />}
            </Field>
          </div>
        </div>
        {editingId ? (
          <div style={{ display: "flex", gap: 6 }}>
            <Button size="sm" variant="primary" style={{ flex: 1 }} onClick={() => void save()}>Save</Button>
            <Button size="sm" style={{ flex: 1 }} onClick={reset}>Cancel</Button>
          </div>
        ) : (
          <Button size="sm" variant="primary" block disabled={!addr.trim()} onClick={() => void save()}>Add tag</Button>
        )}

        <div style={{ display: "flex", flexDirection: "column" }}>
          {tags.map((t) => (
            <div key={t.id} style={{ display: "flex", justifyContent: "space-between", alignItems: "center", padding: "6px 0", borderBottom: "1px solid var(--border)", fontSize: "var(--fs-sm)" }}>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ color: "var(--text)", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{t.name}</div>
                <div className="mono" style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>{t.address} &middot; {t.data_type}</div>
              </div>
              <div style={{ display: "flex", gap: 2 }}>
                <IconButton size="sm" label={`Edit ${t.name}`} onClick={() => { setEditingId(t.id); setName(t.name); setAddr(t.address); setType(t.data_type); setAddrError(null); }}>
                  <Pencil size={14} strokeWidth={1.75} />
                </IconButton>
                <IconButton size="sm" danger label={`Delete ${t.name}`} onClick={() => setDeviceTags(device.id, (prev) => prev.filter((x) => x.id !== t.id))}>
                  <Trash2 size={14} strokeWidth={1.75} />
                </IconButton>
              </div>
            </div>
          ))}
        </div>
      </div>
      <Dialog
        open={pending !== null}
        onClose={() => setPending(null)}
        title="Import tags"
        width={420}
        footer={
          <>
            <Button size="sm" onClick={() => setPending(null)}>Cancel</Button>
            <Button size="sm" onClick={() => applyImport("add")}>Add to existing</Button>
            <Button size="sm" variant="primary" onClick={() => applyImport("replace")}>Replace existing</Button>
          </>
        }
      >
        {pending && (
          <div style={{ fontSize: "var(--fs-sm)", color: "var(--text-dim)", lineHeight: 1.5 }}>
            <strong style={{ color: "var(--text)" }}>{pending.tags.length} tags</strong> found in <span className="mono">{pending.path.split(/[\\/]/).pop()}</span>.
            {pending.errors.length > 0 && ` ${pending.errors.length} row(s) will be skipped; the reasons are in the comm log.`}
          </div>
        )}
      </Dialog>
    </Section>
  );
}
