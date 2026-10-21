import { Button, Callout, EmptyState, Field, IconButton, Section, useProStatus } from "@mactools/ui";
import { ChevronDown, ChevronUp, Download, Pencil, Plus, Trash2, Upload } from "lucide-react";
import { AREAS, FREE_VALUES, TYPES } from "../constants";
import { blockStep, isBitArea, validateBlock } from "../helpers";
import type { ReadBlock } from "../types";
import { displayAddr } from "../utils/format";
import { valueCapNotice } from "../limits";
import { useStore } from "../store";

interface Props {
  onExportCsv: () => void;
  onImportCsv: () => void;
}

export function BlocksPanel({ onExportCsv, onImportCsv }: Props) {
  const {
    blocks, setBlocks,
    editingBlock, setEditingBlock,
    newName, setNewName, newAddr, setNewAddr, newQty, setNewQty,
    newType, setNewType, newArea, setNewArea,
    useOneBased, addLog,
  } = useStore();
  const { pro, requirePro, openUpsell } = useProStatus();
  const capNotice = valueCapNotice(blocks, pro);
  const totalValues = (list: ReadBlock[]) => list.reduce((n, b) => n + b.quantity, 0);
  const overFreeCap = (list: ReadBlock[]) => !pro && totalValues(list) > FREE_VALUES && !requirePro(`More than ${FREE_VALUES} polled values`);

  /** The block described by the form, or null (with the reason logged) if it can not be polled. */
  const blockFromForm = (): ReadBlock | null => {
    const block: ReadBlock = {
      name: newName.trim(),
      address: newAddr.trim() === "" ? NaN : Number(newAddr),
      quantity: newQty.trim() === "" ? NaN : Number(newQty),
      // Coils and discrete inputs are single bits; they have no register type.
      type: isBitArea(newArea) ? "uint16" : newType,
      area: newArea,
    };
    const problem = validateBlock(block);
    if (problem) {
      addLog("error", `Block not saved: ${problem}`);
      return null;
    }
    return block;
  };

  const addBlock = () => {
    const block = blockFromForm();
    if (!block) return;
    if (overFreeCap([...blocks, block])) return;
    setBlocks([...blocks, block]);
    setNewName("");
    setNewAddr("");
    setNewQty("");
  };

  const startEditBlock = (index: number) => {
    const b = blocks[index];
    setNewName(b.name);
    setNewAddr(String(b.address));
    setNewQty(String(b.quantity));
    setNewType(b.type);
    setNewArea(b.area);
    setEditingBlock(index);
  };

  const saveEditBlock = () => {
    if (editingBlock === null) return;
    const block = blockFromForm();
    if (!block) return;
    const next = blocks.map((b, i) => i === editingBlock ? block : b);
    if (overFreeCap(next)) return;
    setBlocks(next);
    setEditingBlock(null);
    setNewName("");
    setNewAddr("");
    setNewQty("");
  };

  const cancelEditBlock = () => {
    setEditingBlock(null);
    setNewName("");
    setNewAddr("");
    setNewQty("");
  };

  const removeBlock = (index: number) => {
    setBlocks(blocks.filter((_, i) => i !== index));
  };

  const moveBlock = (index: number, direction: -1 | 1) => {
    const next = [...blocks];
    const target = index + direction;
    if (target < 0 || target >= next.length) return;
    [next[index], next[target]] = [next[target], next[index]];
    setBlocks(next);
  };

  return (
    <Section title="Read blocks" meta={blocks.length > 0 ? `${blocks.length}` : undefined}>
      {capNotice && (
        <div style={{ marginBottom: 8 }}>
          <Callout tone="warn">
            {capNotice}{" "}
            <button type="button" className="vt-btn vt-btn--sm vt-btn--ghost" onClick={() => openUpsell(`More than ${FREE_VALUES} polled values`)}>Unlock Pro</button>
          </Callout>
        </div>
      )}
      <div style={{ display: "flex", gap: 4, marginBottom: 8 }}>
        <Button size="sm" variant="ghost" onClick={onImportCsv} icon={<Upload size={14} strokeWidth={1.75} />}>Import CSV</Button>
        {blocks.length > 0 && (
          <Button size="sm" variant="ghost" onClick={onExportCsv} icon={<Download size={14} strokeWidth={1.75} />}>Export CSV</Button>
        )}
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
        {blocks.map((b, i) => {
          const step = blockStep(b);
          const endAddr = b.address + (b.quantity - 1) * step + step - 1;
          if (editingBlock === i) {
            return (
              <div key={i} style={{ padding: "6px 0", borderBottom: "1px solid var(--border)", display: "flex", flexDirection: "column", gap: 4 }}>
                <input className="vt-input" placeholder="Block name" value={newName} onChange={(e) => setNewName(e.target.value)} />
                <div style={{ display: "flex", gap: 4 }}>
                  <input className="vt-input" style={{ width: 60, flexShrink: 0 }} placeholder="Start" value={newAddr} onChange={(e) => setNewAddr(e.target.value)} />
                  <input className="vt-input" style={{ width: 50, flexShrink: 0 }} placeholder="Qty" value={newQty} onChange={(e) => setNewQty(e.target.value)} />
                </div>
                <div style={{ display: "flex", gap: 4 }}>
                  <select className="vt-select" style={{ flex: 1 }} value={isBitArea(newArea) ? "uint16" : newType} disabled={isBitArea(newArea)} onChange={(e) => setNewType(e.target.value)}>
                    {TYPES.map((t) => <option key={t} value={t}>{t}</option>)}
                  </select>
                  <select className="vt-select" style={{ flex: 1 }} value={newArea} onChange={(e) => setNewArea(e.target.value)}>
                    {AREAS.map((a) => <option key={a} value={a}>{a}</option>)}
                  </select>
                </div>
                <div style={{ display: "flex", gap: 4 }}>
                  <Button size="sm" variant="primary" style={{ flex: 1 }} onClick={saveEditBlock}>Save</Button>
                  <Button size="sm" variant="ghost" style={{ flex: 1 }} onClick={cancelEditBlock}>Cancel</Button>
                </div>
              </div>
            );
          }
          return (
            <div
              key={i}
              style={{ display: "grid", gridTemplateColumns: "minmax(0, 1fr) auto", alignItems: "center", columnGap: 6, rowGap: 2, padding: "6px 0", borderBottom: i < blocks.length - 1 ? "1px solid var(--border)" : "none" }}
            >
              <div style={{ fontSize: "var(--fs-sm)", color: "var(--text)", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{b.name}</div>
              <div style={{ display: "flex", gap: 2 }}>
                <IconButton label={`Move ${b.name} up`} size="sm" onClick={() => moveBlock(i, -1)} disabled={i === 0}>
                  <ChevronUp size={14} strokeWidth={1.75} />
                </IconButton>
                <IconButton label={`Move ${b.name} down`} size="sm" onClick={() => moveBlock(i, 1)} disabled={i === blocks.length - 1}>
                  <ChevronDown size={14} strokeWidth={1.75} />
                </IconButton>
                <IconButton label={`Edit ${b.name}`} size="sm" onClick={() => startEditBlock(i)}>
                  <Pencil size={14} strokeWidth={1.75} />
                </IconButton>
                <IconButton label={`Delete ${b.name}`} size="sm" danger onClick={() => removeBlock(i)}>
                  <Trash2 size={14} strokeWidth={1.75} />
                </IconButton>
              </div>
              <div className="mono" style={{ gridColumn: "1 / -1", fontSize: "var(--fs-xs)", color: "var(--text-muted)", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                {displayAddr(b.address, b.area, useOneBased)}-{displayAddr(endAddr, b.area, useOneBased)} · {b.quantity}x {b.type} · {b.area}
              </div>
            </div>
          );
        })}
        {blocks.length === 0 && (
          <EmptyState
            compact
            title="No read blocks"
            description="Add a block below to start polling registers."
          />
        )}
        {editingBlock === null && (
          <div style={{ display: "flex", flexDirection: "column", gap: 4, borderTop: "1px solid var(--border)", paddingTop: 8, marginTop: 4 }}>
            <Field label="New block">
              <input className="vt-input" placeholder="Block name" value={newName} onChange={(e) => setNewName(e.target.value)} />
            </Field>
            <div style={{ display: "flex", gap: 4 }}>
              <div style={{ width: 76 }}>
                <Field label="Start">
                  <input className="vt-input" placeholder="Address" value={newAddr} onChange={(e) => setNewAddr(e.target.value)} />
                </Field>
              </div>
              <div style={{ width: 64 }}>
                <Field label="Qty">
                  <input className="vt-input" placeholder="Count" value={newQty} onChange={(e) => setNewQty(e.target.value)} />
                </Field>
              </div>
            </div>
            <div style={{ display: "flex", gap: 4 }}>
              <select className="vt-select" style={{ flex: 1 }} value={isBitArea(newArea) ? "uint16" : newType} disabled={isBitArea(newArea)} onChange={(e) => setNewType(e.target.value)} aria-label="Register type">
                {TYPES.map((t) => <option key={t} value={t}>{t}</option>)}
              </select>
              <select className="vt-select" style={{ flex: 1 }} value={newArea} onChange={(e) => setNewArea(e.target.value)} aria-label="Register area">
                {AREAS.map((a) => <option key={a} value={a}>{a}</option>)}
              </select>
            </div>
            <Button variant="primary" onClick={addBlock} icon={<Plus size={14} strokeWidth={1.75} />}>Add block</Button>
          </div>
        )}
      </div>
    </Section>
  );
}
