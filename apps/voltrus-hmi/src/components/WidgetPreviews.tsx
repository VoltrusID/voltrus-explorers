import { useEffect, useRef, useState } from "react";
import { useStore } from "../store";
import type { HmiWidget } from "../types";

// ---------------------------------------------------------------------------
// SVG-based widget previews
// ---------------------------------------------------------------------------

/** Compute point on a semicircular arc (radius 40, center 50,55 in a 0-100 viewBox) */
export function arcPoint(angleDeg: number): { x: number; y: number } {
  const rad = (angleDeg * Math.PI) / 180;
  const cx = 50, cy = 55, r = 40;
  return { x: cx + r * Math.cos(rad), y: cy - r * Math.sin(rad) };
}

export function gaugeColor(fraction: number): string {
  if (fraction < 0.8) return "#22c55e";
  if (fraction < 0.95) return "#eab308";
  return "#ef4444";
}

/** Mirrors the exported runtime's ON check exactly (widgets.rs) so the
 *  designer preview never disagrees with the deployed HMI. */
export function isOn(v: unknown): boolean {
  return v === true || v === 1 || v === "1" || v === "true";
}

const REAL_TYPES = new Set(["REAL", "LREAL", "FLOAT"]);
const INT_TYPES = new Set([
  "BOOL", "BYTE", "SINT", "USINT", "INT", "UINT", "DINT", "UDINT",
  "LINT", "ULINT", "WORD", "DWORD", "LWORD", "TIME",
]);

/** Same rules as the exported runtime's fmtNum (export/widgets.rs num_mode):
 *  REAL/LREAL/FLOAT -> one decimal, the listed integer types -> whole number,
 *  untyped or unknown types (e.g. FLOAT64) -> integers as-is and fractions
 *  with one decimal. */
export function formatValue(v: unknown, dataType?: string): string {
  if (typeof v !== "number") return String(v);
  const t = (dataType ?? "").toUpperCase();
  if (REAL_TYPES.has(t)) return v.toFixed(1);
  if (INT_TYPES.has(t)) return String(Math.round(v));
  return Number.isInteger(v) ? String(v) : v.toFixed(1);
}

// Rolling chart buffer per chart widget (designer-side trend preview).
export const chartBuffers = new Map<string, number[]>();
const lastFrame = new Map<string, unknown>();
const CHART_MAX_POINTS = 50;

function polylinePoints(buf: number[]): string {
  if (buf.length < 2) return "";
  const minV = Math.min(...buf);
  const maxV = Math.max(...buf);
  const range = maxV - minV || 1;
  return buf
    .map((v, i) => {
      const px = (i / (buf.length - 1)) * 200;
      const py = 80 - ((v - minV) / range) * 76 - 2;
      return `${px.toFixed(1)},${py.toFixed(1)}`;
    })
    .join(" ");
}

export function GaugePreview({ widget, liveValue }: { widget: HmiWidget; liveValue?: unknown }) {
  const { min, max, label, variableName, dataType } = widget;
  const hasValue = liveValue !== undefined && typeof liveValue === "number";
  const fraction = hasValue
    ? Math.max(0, Math.min(1, ((liveValue as number) - min) / (max - min || 1)))
    : 0;
  const arcEndAngle = 180 - fraction * 180; // sweep left (180°) -> right (0°)

  const startPt = arcPoint(180);
  const endPt = arcPoint(arcEndAngle);

  const color = gaugeColor(fraction);

  return (
    <div className="widget-preview-gauge">
      <svg viewBox="0 0 100 60" preserveAspectRatio="xMidYMid meet">
        {/* Background arc */}
        <path d="M10,55 A40,40 0 0,1 90,55" fill="none" stroke="#444" strokeWidth="6" strokeLinecap="round" />
        {/* Value arc */}
        {fraction > 0 && (
          <path
            d={`M${startPt.x},${startPt.y} A40,40 0 0,1 ${endPt.x},${endPt.y}`}
            fill="none"
            stroke={color}
            strokeWidth="6"
            strokeLinecap="round"
          />
        )}
        {/* Value text */}
        <text x="50" y="48" textAnchor="middle" fill={widget.color} fontSize="14" fontWeight="bold" fontFamily="sans-serif">
          {hasValue ? formatValue(liveValue, dataType) : "--"}
        </text>
      </svg>
      <span className="gauge-label">{label}</span>
      {variableName && <span className="widget-var-label inline">{variableName}</span>}
    </div>
  );
}

export function IndicatorPreview({ widget, liveValue }: { widget: HmiWidget; liveValue?: unknown }) {
  // Bound without a value yet, or unbound: show OFF (never a fake ON) —
  // mirrors the runtime: dark fill + colored ring, filled + glow when ON.
  const active = liveValue !== undefined ? isOn(liveValue) : false;
  return (
    <div className="widget-preview-indicator">
      <svg viewBox="0 0 60 60" width="60%" style={{ maxHeight: "60%", overflow: "visible" }}>
        <circle
          cx="30" cy="30" r="27"
          fill={active ? widget.color : "#333"}
          stroke={widget.color}
          strokeWidth="2"
          style={active ? { filter: `drop-shadow(0 0 6px ${widget.color})` } : undefined}
        />
      </svg>
      <span className="indicator-label" style={{ fontSize: widget.fontSize }}>{widget.label}</span>
      {widget.variableName && <span className="widget-var-label inline">{widget.variableName}</span>}
    </div>
  );
}

export function ButtonPreview({ widget }: { widget: HmiWidget }) {
  // Same look as the exported runtime button (dark fill, colored border/text).
  return (
    <div className="widget-preview-button" style={{ color: widget.color }}>
      <span className="button-label" style={{ fontSize: widget.fontSize }}>{widget.label}</span>
      {widget.variableName && <span className="button-var">{widget.variableName}</span>}
    </div>
  );
}

export function SliderPreview({ widget, liveValue }: { widget: HmiWidget; liveValue?: unknown }) {
  const { min, max, color, label, variableName, dataType, fontSize } = widget;
  const hasValue = liveValue !== undefined && typeof liveValue === "number";
  const raw = Number(liveValue);
  const fraction = hasValue
    ? Math.max(0, Math.min(1, (raw - min) / (max - min || 1)))
    : 0;
  return (
    <div className="widget-preview-slider">
      <span className="slider-label">{label}</span>
      <svg viewBox="0 0 100 12" preserveAspectRatio="none" style={{ width: "90%", height: 12 }}>
        {/* Track */}
        <line x1="5" y1="6" x2="95" y2="6" stroke="#555" strokeWidth="3" strokeLinecap="round" />
        {/* Filled portion */}
        <line x1="5" y1="6" x2={5 + fraction * 90} y2="6" stroke={color} strokeWidth="3" strokeLinecap="round" />
      </svg>
      <span className="slider-value" style={{ fontSize, color }}>
        {hasValue ? formatValue(liveValue, dataType) : "--"}
      </span>
      {variableName && <span className="widget-var-label inline">{variableName}</span>}
    </div>
  );
}

export function ChartPreview({ widget }: { widget: HmiWidget; liveValue?: unknown }) {
  const { id, label, variableName, color } = widget;
  const [points, setPoints] = useState("");
  const prevVariable = useRef<string | null>(null);
  // Every PLC frame replaces plcVariables, so this changes per frame even when
  // the value does not — constant signals still advance the trend, like the
  // runtime chart.
  const frame = useStore((s) => (variableName ? s.plcVariables : null));

  useEffect(() => {
    // Rebinding starts a fresh trend; reset before appending so the first
    // sample for the new variable is kept.
    if (prevVariable.current !== null && prevVariable.current !== variableName) {
      chartBuffers.delete(id);
      lastFrame.delete(id);
    }
    prevVariable.current = variableName;

    const buf = chartBuffers.get(id) ?? [];
    // One sample per frame: re-renders, remounts (screen switches) and
    // StrictMode effect replays must not append the same frame twice.
    if (variableName && frame && lastFrame.get(id) !== frame) {
      lastFrame.set(id, frame);
      const v = frame.find((x) => x.name === variableName)?.value;
      if (typeof v === "number") {
        buf.push(v);
        if (buf.length > CHART_MAX_POINTS) buf.shift();
        chartBuffers.set(id, buf);
      }
    }
    setPoints(polylinePoints(buf));
  }, [id, variableName, frame]);

  // Drop the buffer when the widget is deleted (not when its screen is merely
  // switched away — the widget still exists then).
  useEffect(
    () => () => {
      const exists = useStore
        .getState()
        .screens.some((sc) => sc.widgets.some((w) => w.id === id));
      if (!exists) {
        chartBuffers.delete(id);
        lastFrame.delete(id);
      }
    },
    [id]
  );

  return (
    <div className="widget-preview-chart">
      <span className="chart-label">{label}</span>
      <div className="chart-plot">
        {points ? (
          <svg viewBox="0 0 200 80" preserveAspectRatio="none">
            <polyline points={points} fill="none" stroke={color} strokeWidth="1.5" strokeLinejoin="round" vectorEffect="non-scaling-stroke" />
          </svg>
        ) : (
          <span className="chart-empty">{variableName ? "Waiting for data" : "Not bound"}</span>
        )}
      </div>
      {variableName && <span className="widget-var-label inline">{variableName}</span>}
    </div>
  );
}

export function TextPreview({ widget, liveValue }: { widget: HmiWidget; liveValue?: unknown }) {
  const { label, variableName, color, fontSize } = widget;
  // Matches the runtime: unbound text shows its label, bound text shows the
  // value ("--" until the first frame arrives).
  const displayValue = !variableName
    ? label
    : liveValue !== undefined
      ? formatValue(liveValue, widget.dataType)
      : "--";
  return (
    <div className="widget-preview-text">
      <span style={{ fontSize, color, lineHeight: 1.2 }}>{displayValue}</span>
      {variableName && <span className="widget-var-label inline">{variableName}</span>}
    </div>
  );
}
