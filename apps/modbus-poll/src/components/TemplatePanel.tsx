import {
  BUILTIN_PROFILES,
  numRegisters,
} from "../templates";
import { Button, Section, SheetSelect } from "@mactools/ui";
import { Pause, Play, Plus } from "lucide-react";
import { useStore } from "../store";

interface Props {
  applyProfile: (profile: import("../templates").DeviceProfile) => void;
  toggleSimulation: () => void;
  saveCustomTemplate: () => void;
}

export function TemplatePanel({ applyProfile, toggleSimulation, saveCustomTemplate }: Props) {
  const {
    activeProfile, simValues, simRunning, simSpeed, setSimSpeed,
    customTemplates, showCustomDialog, setShowCustomDialog,
    customName, setCustomName, customRegs, setCustomRegs,
  } = useStore();

  return (
    <>
      <Section title="Device template">
      <SheetSelect
        sheetTitle="Device template"
        value={activeProfile?.id ?? ""}
        onChange={(id) => {
          if (id === "__custom__") { setShowCustomDialog(true); return; }
          const all = [...BUILTIN_PROFILES, ...customTemplates];
          const p = all.find((t) => t.id === id);
          if (p) applyProfile(p);
        }}
        options={[
          { value: "", label: "Select device…" },
          ...BUILTIN_PROFILES.map((p) => ({ value: p.id, label: `${p.manufacturer} ${p.model}`, hint: p.description, group: "Built-in" })),
          ...customTemplates.map((p) => ({ value: p.id, label: p.model, hint: p.description, group: "Custom" })),
          { value: "__custom__", label: "New custom…" },
        ]}
      />

      {activeProfile && (
        <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)", lineHeight: 1.5 }}>
          {activeProfile.description}
          <br />Base address: {activeProfile.baseAddress} | {activeProfile.registers.length} params, {numRegisters(activeProfile)} u16 regs
        </div>
      )}

      {activeProfile && useStore.getState().slaveRunning && (
        <div style={{ display: "flex", gap: 4, alignItems: "center" }}>
          <Button
            onClick={toggleSimulation}
            variant={simRunning ? "danger" : "primary"}
            size="sm"
            style={{ flex: 1 }}
            icon={simRunning ? <Pause size={14} strokeWidth={1.75} /> : <Play size={14} strokeWidth={1.75} />}
          >
            {simRunning ? "Pause" : "Simulate"}
          </Button>
          <SheetSelect
            sheetTitle="Update speed"
            className="vt-select vt-select--narrow"
            style={{ width: 70 }}
            value={simSpeed}
            onChange={setSimSpeed}
            options={[{ value: 500, label: "0.5s" }, { value: 1000, label: "1s" }, { value: 2000, label: "2s" }, { value: 5000, label: "5s" }]}
          />
        </div>
      )}

      {activeProfile && simValues.length > 0 && (
        <div style={{ marginTop: 4 }}>
          <div style={{ fontSize: "var(--fs-xs)", fontWeight: 600, color: "var(--text)", marginBottom: 6, paddingBottom: 4, borderBottom: "1px solid var(--border)" }}>
            Live values
          </div>
          {activeProfile.registers.map((r, i) => (
            <div key={i} style={{ display: "flex", justifyContent: "space-between", padding: "3px 0", borderBottom: "1px solid var(--border)", fontSize: "var(--fs-xs)" }}>
              <span style={{ color: "var(--text-dim)" }}>{r.name}</span>
              <span style={{ color: "var(--text)", fontFamily: "var(--font-mono)" }}>
                {r.isCounter
                  ? simValues[i].toLocaleString(undefined, { maximumFractionDigits: 3 })
                  : simValues[i].toFixed(r.drift < 0.01 ? 3 : r.drift < 0.1 ? 2 : 1)}
                {r.unit ? ` ${r.unit}` : ""}
              </span>
            </div>
          ))}
        </div>
      )}
      </Section>
    </>
  );
}
