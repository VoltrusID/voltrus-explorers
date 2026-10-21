import { useState } from "react";
import { Bookmark, Repeat, SendHorizontal, Trash2 } from "lucide-react";
import { Button, IconButton, ProBadge, Segmented, useProStatus } from "@mactools/ui";
import type { SendMacro } from "../profiles";

interface SendComposerProps {
  format: "hex" | "ascii";
  onFormatChange: (format: "hex" | "ascii") => void;
  value: string;
  onValueChange: (value: string) => void;
  onSend: () => void;
  /** True while the current mode has no open port. */
  disabled: boolean;
  /** Send-macro and repeat-send support (Pro). Omit to hide the row. */
  macros?: SendMacro[];
  onMacrosChange?: (list: SendMacro[]) => void;
  repeating?: boolean;
  onRepeatingChange?: (on: boolean) => void;
  repeatMs?: number;
  onRepeatMsChange?: (ms: number) => void;
}

/**
 * Terminal-style input bar shown under the traffic table: format toggle,
 * payload field, send action.
 */
export function SendComposer({
  format,
  onFormatChange,
  value,
  onValueChange,
  onSend,
  disabled,
  macros,
  onMacrosChange,
  repeating = false,
  onRepeatingChange,
  repeatMs = 1000,
  onRepeatMsChange,
}: SendComposerProps) {
  const { pro, requirePro } = useProStatus();
  const [macroName, setMacroName] = useState("");
  const [macroSel, setMacroSel] = useState("");
  const extras = macros && onMacrosChange;

  const saveMacro = () => {
    const name = macroName.trim();
    if (!name || !value || !macros || !onMacrosChange) return;
    if (!requirePro("Send macros")) return;
    onMacrosChange([...macros.filter((m) => m.name !== name), { name, data: value, format }]);
    setMacroSel(name);
    setMacroName("");
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6, flexShrink: 0 }}>
    {extras && (
      <div style={{ display: "flex", alignItems: "center", flexWrap: "wrap", gap: 8, fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
        <select
          className="vt-select"
          aria-label="Send macro"
          style={{ width: 160 }}
          value={macroSel}
          onChange={(e) => {
            setMacroSel(e.target.value);
            const m = macros.find((x) => x.name === e.target.value);
            if (m) { onFormatChange(m.format); onValueChange(m.data); }
          }}
        >
          <option value="">{macros.length ? "Macros..." : "No macros yet"}</option>
          {macros.map((m) => <option key={m.name} value={m.name}>{m.name}</option>)}
        </select>
        <IconButton
          label="Delete macro"
          size="sm"
          disabled={!macroSel}
          onClick={() => { onMacrosChange(macros.filter((m) => m.name !== macroSel)); setMacroSel(""); }}
        >
          <Trash2 size={14} strokeWidth={1.75} />
        </IconButton>
        <input
          className="vt-input"
          aria-label="Macro name"
          placeholder="Macro name"
          style={{ width: 130 }}
          value={macroName}
          onChange={(e) => setMacroName(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter") saveMacro(); }}
        />
        <Button size="sm" icon={<Bookmark size={14} strokeWidth={1.75} />} disabled={!macroName.trim() || !value} onClick={saveMacro}>
          Save macro <ProBadge />
        </Button>
        <span style={{ flex: 1 }} />
        <Button
          size="sm"
          variant={repeating ? "danger" : "default"}
          icon={<Repeat size={14} strokeWidth={1.75} />}
          disabled={!repeating && (disabled || !value)}
          onClick={() => {
            if (repeating) { onRepeatingChange?.(false); return; }
            if (requirePro("Repeat send")) onRepeatingChange?.(true);
          }}
        >
          {repeating ? "Stop repeat" : <>Repeat {pro ? "" : <ProBadge />}</>}
        </Button>
        <input
          className="vt-input"
          aria-label="Repeat interval in milliseconds"
          type="number"
          min={10}
          style={{ width: 80 }}
          value={repeatMs}
          onChange={(e) => {
            const v = parseInt(e.target.value, 10);
            if (!isNaN(v)) onRepeatMsChange?.(Math.max(10, v));
          }}
        />
        <span>ms</span>
      </div>
    )}
    <div
      className="send-composer"
      style={{
        display: "flex",
        alignItems: "flex-end",
        flexWrap: "wrap",
        gap: 8,
        flexShrink: 0,
      }}
    >
      <Segmented
        label="Send format"
        size="sm"
        value={format}
        onChange={onFormatChange}
        options={[
          { value: "hex", label: "HEX" },
          { value: "ascii", label: "ASCII" },
        ]}
      />
      <textarea
        className="vt-textarea vt-textarea--mono"
        style={{ flex: 1, minWidth: 0, minHeight: 32, maxHeight: 120, resize: "none" }}
        placeholder={format === "hex" ? "01 03 00 00 00 0A C5 CD" : "Enter text..."}
        value={value}
        onChange={(e) => onValueChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            onSend();
          }
        }}
        spellCheck={false}
      />
      <Button
        variant="primary"
        size="sm"
        icon={<SendHorizontal size={14} strokeWidth={1.75} />}
        onClick={onSend}
        disabled={disabled || !value}
        style={{ flexShrink: 0 }}
      >
        Send
      </Button>
    </div>
    </div>
  );
}
