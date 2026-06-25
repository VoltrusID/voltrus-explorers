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

export function GaugePreview({ widget, liveValue }: { widget: HmiWidget; liveValue?: unknown }) {
  const { min, max, label, variableName } = widget;
  const raw = Number(liveValue);
  const fraction = liveValue !== undefined
    ? Math.max(0, Math.min(1, (raw - min) / (max - min || 1)))
    : 0.42;
  const value = min + fraction * (max - min);
  const arcEndAngle = fraction * 180; // 0 to 180 degrees

  const startPt = arcPoint(0);   // right end (angle 0)
  const endPt = arcPoint(arcEndAngle);
  const largeArc = arcEndAngle > 90 ? 1 : 0;

  const color = gaugeColor(fraction);

  return (
    <div className="widget-preview-gauge">
      <svg viewBox="0 0 100 60" preserveAspectRatio="xMidYMid meet">
        {/* Label above arc */}
        <text x="50" y="12" textAnchor="middle" fill="#888" fontSize="7" fontFamily="sans-serif">{label}</text>
        {/* Background arc */}
        <path d="M10,55 A40,40 0 0,1 90,55" fill="none" stroke="#333" strokeWidth="6" strokeLinecap="round" />
        {/* Value arc */}
        {fraction > 0 && (
          <path
            d={`M${startPt.x},${startPt.y} A40,40 0 ${largeArc},1 ${endPt.x},${endPt.y}`}
            fill="none"
            stroke={color}
            strokeWidth="6"
            strokeLinecap="round"
          />
        )}
        {/* Value text */}
        <text x="50" y="52" textAnchor="middle" fill="white" fontSize="13" fontWeight="bold" fontFamily="sans-serif">
          {Math.round(value)}
        </text>
      </svg>
      {variableName && <span className="widget-var-label">{variableName}</span>}
    </div>
  );
}

export function IndicatorPreview({ widget, liveValue }: { widget: HmiWidget; liveValue?: unknown }) {
  const active = liveValue !== undefined
    ? Number(liveValue) > 0 || !!liveValue
    : !!widget.variableName;
  const fillColor = active ? "#4ade80" : "#666";
  const circleColor = active ? "#4ade80" : widget.color;
  return (
    <div className="widget-preview-indicator" style={{ color: circleColor }}>
      <svg viewBox="0 0 60 60" width="70%" style={{ maxHeight: "65%" }}>
        <circle
          cx="30" cy="30" r="22"
          fill={fillColor}
          stroke={widget.color}
          strokeWidth="2"
        />
      </svg>
      <span className="indicator-label">{widget.label}</span>
      {widget.variableName && <span className="widget-var-label">{widget.variableName}</span>}
    </div>
  );
}

export function ButtonPreview({ widget, liveValue }: { widget: HmiWidget; liveValue?: unknown }) {
  const isOn = liveValue !== undefined
    ? Number(liveValue) > 0 || !!liveValue
    : !!widget.variableName;
  const bg = isOn ? widget.color : "#3b82f6";
  return (
    <div className="widget-preview-button" style={{ background: bg, borderRadius: 6 }}>
      <span className="button-label" style={{ fontSize: widget.fontSize }}>{widget.label}</span>
      {widget.variableName && <span className="button-var">{widget.variableName}</span>}
    </div>
  );
}

export function SliderPreview({ widget, liveValue }: { widget: HmiWidget; liveValue?: unknown }) {
  const { min, max, color, label, variableName } = widget;
  const raw = Number(liveValue);
  const fraction = liveValue !== undefined
    ? Math.max(0, Math.min(1, (raw - min) / (max - min || 1)))
    : 0.5;
  const value = min + fraction * (max - min);
  return (
    <div className="widget-preview-slider">
      <svg viewBox="0 0 100 20" preserveAspectRatio="xMidYMid meet">
        {/* Track */}
        <line x1="5" y1="10" x2="95" y2="10" stroke="#444" strokeWidth="3" strokeLinecap="round" />
        {/* Filled portion */}
        <line x1="5" y1="10" x2={5 + fraction * 90} y2="10" stroke={color} strokeWidth="3" strokeLinecap="round" />
        {/* Thumb */}
        <circle cx={5 + fraction * 90} cy="10" r="5" fill="#fff" stroke={color} strokeWidth="1.5" />
      </svg>
      <span className="slider-value">{Math.round(value)}</span>
      {variableName && <span className="widget-var-label">{variableName}</span>}
    </div>
  );
}

export function ChartPreview({ widget, liveValue: _liveValue }: { widget: HmiWidget; liveValue?: unknown }) {
  const { label, variableName, color } = widget;
  // Placeholder sparkline polyline
  const points = "5,35 15,28 25,32 35,18 45,24 55,14 65,20 75,10 85,16 95,12";
  return (
    <div className="widget-preview-chart">
      <svg viewBox="0 0 100 50" preserveAspectRatio="xMidYMid meet">
        {/* Grid lines */}
        <line x1="5" y1="5" x2="5" y2="45" stroke="#333" strokeWidth="0.3" />
        <line x1="5" y1="45" x2="95" y2="45" stroke="#333" strokeWidth="0.3" />
        <line x1="5" y1="25" x2="95" y2="25" stroke="#333" strokeWidth="0.3" strokeDasharray="2,2" />
        <line x1="35" y1="5" x2="35" y2="45" stroke="#333" strokeWidth="0.3" strokeDasharray="2,2" />
        <line x1="65" y1="5" x2="65" y2="45" stroke="#333" strokeWidth="0.3" strokeDasharray="2,2" />
        {/* Sparkline */}
        <polyline points={points} fill="none" stroke={color} strokeWidth="1.5" strokeLinejoin="round" />
        {/* Label */}
        <text x="50" y="8" textAnchor="middle" fill="#888" fontSize="6" fontFamily="sans-serif">{label}</text>
      </svg>
      {variableName && <span className="widget-var-label">{variableName}</span>}
    </div>
  );
}

export function TextPreview({ widget, liveValue }: { widget: HmiWidget; liveValue?: unknown }) {
  const { label, variableName, color, fontSize } = widget;
  const displayValue = liveValue !== undefined ? String(liveValue) : (variableName || label);
  return (
    <div className="widget-preview-text">
      <span style={{ fontSize, color }}>{displayValue}</span>
      {variableName && <span style={{ fontSize: 9, color: "var(--text-muted)", marginTop: 2 }}>{variableName}</span>}
    </div>
  );
}
