import { useEffect, useState } from "react";
import { Download, Pause, Play, Trash2 } from "lucide-react";
import { Button, IconButton, Segmented } from "@mactools/ui";

export type WindowSec = "30" | "60" | "300" | "0";

const WINDOW_OPTIONS = [
  { value: "30", label: "30 s" },
  { value: "60", label: "1 min" },
  { value: "300", label: "5 min" },
  { value: "0", label: "All" },
] as const;

/** Wall clock that ticks four times a second and freezes while `paused`. */
export function useClock(paused: boolean): number {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (paused) return;
    setNow(Date.now());
    const id = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(id);
  }, [paused]);
  return now;
}

interface Props {
  windowSec: WindowSec;
  onWindowChange: (w: WindowSec) => void;
  paused: boolean;
  onPausedChange: (p: boolean) => void;
  onClear: () => void;
  onExport: () => void;
}

export default function ChartToolbar({ windowSec, onWindowChange, paused, onPausedChange, onClear, onExport }: Props) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
      <Segmented size="sm" label="Time window" value={windowSec} options={WINDOW_OPTIONS} onChange={onWindowChange} />
      <IconButton
        size="sm"
        outline
        label={paused ? "Resume chart" : "Pause chart"}
        pressed={paused}
        onClick={() => onPausedChange(!paused)}
      >
        {paused ? <Play size={14} strokeWidth={1.75} /> : <Pause size={14} strokeWidth={1.75} />}
      </IconButton>
      <span style={{ flex: 1 }} />
      <Button size="sm" icon={<Download size={13} strokeWidth={1.75} />} onClick={onExport}>
        Export CSV
      </Button>
      <Button size="sm" variant="ghost" icon={<Trash2 size={13} strokeWidth={1.75} />} onClick={onClear}>
        Clear
      </Button>
    </div>
  );
}

interface NumInputProps {
  value: number;
  onCommit: (n: number) => void;
  min?: number;
  width?: number;
  label: string;
  integer?: boolean;
}

/** Number box that keeps what the user is typing ("0." or "-") and commits only valid numbers. */
export function NumInput({ value, onCommit, min, width = 64, label, integer }: NumInputProps) {
  const [text, setText] = useState(String(value));
  useEffect(() => {
    setText((t) => (Number(t) === value ? t : String(value)));
  }, [value]);
  const invalid = text.trim() === "" || !Number.isFinite(Number(text)) || (min !== undefined && Number(text) < min) || (integer && !Number.isInteger(Number(text)));
  return (
    <input
      className="vt-input vt-input--sm vt-input--mono"
      style={{ width }}
      inputMode="decimal"
      aria-label={label}
      aria-invalid={invalid || undefined}
      value={text}
      onChange={(e) => {
        setText(e.target.value);
        const n = Number(e.target.value);
        if (e.target.value.trim() !== "" && Number.isFinite(n) && (min === undefined || n >= min) && (!integer || Number.isInteger(n))) onCommit(n);
      }}
      onBlur={() => setText(String(value))}
    />
  );
}
