import { Button, Field, IconButton } from "@mactools/ui";
import { Plus, Trash2 } from "lucide-react";
import { useEffect } from "react";
import { useStore } from "../store";

interface Props {
  onSave: () => void;
}

export function CustomTemplateDialog({ onSave }: Props) {
  const { showCustomDialog, setShowCustomDialog, customName, setCustomName, customRegs, setCustomRegs } = useStore();

  useEffect(() => {
    if (!showCustomDialog) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") setShowCustomDialog(false); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [showCustomDialog, setShowCustomDialog]);

  if (!showCustomDialog) return null;

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Custom device template"
      style={{ position: "fixed", inset: 0, background: "var(--scrim)", display: "flex", alignItems: "center", justifyContent: "center", zIndex: 9999 }}
      onClick={() => setShowCustomDialog(false)}
    >
      <div
        style={{ background: "var(--card)", border: "1px solid var(--border)", borderRadius: "var(--radius-lg)", padding: 24, maxWidth: 420, width: "90%", maxHeight: "80vh", overflowY: "auto", boxShadow: "var(--shadow-pop)" }}
        onClick={(e) => e.stopPropagation()}
      >
        <div style={{ fontSize: "var(--fs-lg)", fontWeight: 600, color: "var(--text)", marginBottom: 12 }}>Custom device template</div>
        <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
          <Field label="Template name">
            <input className="vt-input" placeholder="My device" value={customName} onChange={(e) => setCustomName(e.target.value)} />
          </Field>
          <div style={{ fontSize: "var(--fs-xs)", fontWeight: 600, color: "var(--text-dim)", paddingTop: 4 }}>Registers</div>
          {customRegs.map((r, i) => (
            <div key={i} style={{ display: "grid", gridTemplateColumns: "1fr 50px 60px 60px 60px 22px", gap: 4, alignItems: "end" }}>
              <input className="vt-input" placeholder="Name" value={r.name} onChange={(e) => {
                const regs = [...customRegs]; regs[i] = { ...regs[i], name: e.target.value }; setCustomRegs(regs);
              }} />
              <input className="vt-input" placeholder="Unit" value={r.unit} onChange={(e) => {
                const regs = [...customRegs]; regs[i] = { ...regs[i], unit: e.target.value }; setCustomRegs(regs);
              }} />
              <input className="vt-input" placeholder="Init" type="number" value={r.initial} onChange={(e) => {
                const regs = [...customRegs]; regs[i] = { ...regs[i], initial: Number(e.target.value) }; setCustomRegs(regs);
              }} />
              <input className="vt-input" placeholder="Min" type="number" value={r.min} onChange={(e) => {
                const regs = [...customRegs]; regs[i] = { ...regs[i], min: Number(e.target.value) }; setCustomRegs(regs);
              }} />
              <input className="vt-input" placeholder="Max" type="number" value={r.max} onChange={(e) => {
                const regs = [...customRegs]; regs[i] = { ...regs[i], max: Number(e.target.value) }; setCustomRegs(regs);
              }} />
              <IconButton label="Remove register" size="sm" danger onClick={() => setCustomRegs(customRegs.filter((_, j) => j !== i))}>
                <Trash2 size={16} strokeWidth={1.75} />
              </IconButton>
            </div>
          ))}
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setCustomRegs([...customRegs, { name: "", unit: "", description: "", initial: 0, min: 0, max: 65535, drift: 1, isCounter: false, counterRate: 0 }])}
            icon={<Plus size={14} strokeWidth={1.75} />}
          >
            Add register
          </Button>
          <Button variant="primary" onClick={onSave} disabled={!customName || customRegs.length === 0}>
            Save template
          </Button>
        </div>
      </div>
    </div>
  );
}
