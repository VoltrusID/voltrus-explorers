import { useEffect, useRef, useState } from "react";

export interface ChartSeries {
  id: string;
  name: string;
  color: string;
  points: { t: number; v: number }[];
}

export const SERIES_COLORS = ["#3b82f6", "#ef4444", "#22c55e", "#f59e0b", "#a855f7", "#ec4899", "#14b8a6", "#f97316"];

interface Props {
  series: ChartSeries[];
  /** Right edge of the time axis (ms). Freeze it to pause. */
  now: number;
  /** Visible span in seconds; 0 shows everything recorded. */
  windowSec: number;
  unit?: string;
  height?: number;
  label: string;
}

const PAD = { l: 52, r: 12, t: 10, b: 22 };

function niceStep(range: number, target: number): number {
  const raw = range / target;
  const pow = Math.pow(10, Math.floor(Math.log10(raw)));
  const f = raw / pow;
  return (f < 1.5 ? 1 : f < 3 ? 2 : f < 7 ? 5 : 10) * pow;
}

function formatTick(v: number, step: number): string {
  const decimals = Math.max(0, -Math.floor(Math.log10(step)));
  return v.toFixed(Math.min(decimals, 6));
}

function formatAge(sec: number): string {
  if (sec < 1) return "now";
  if (sec < 120) return `-${Math.round(sec)}s`;
  return `-${(sec / 60).toFixed(sec < 600 ? 1 : 0)}m`;
}

/** Index of the last point with t <= x (or -1). */
function lastAtOrBefore(points: { t: number }[], x: number): number {
  let lo = 0;
  let hi = points.length - 1;
  let ans = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (points[mid].t <= x) {
      ans = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  return ans;
}

/** Canvas line chart with millisecond time resolution, auto Y scale and a hover read-out. */
export default function LineChart({ series, now, windowSec, unit, height = 240, label }: Props) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [width, setWidth] = useState(600);
  const [hoverX, setHoverX] = useState<number | null>(null);

  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setWidth(Math.max(200, el.clientWidth)));
    ro.observe(el);
    setWidth(Math.max(200, el.clientWidth));
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(height * dpr);
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);

    const css = getComputedStyle(canvas);
    const colGrid = css.getPropertyValue("--border").trim() || "#444";
    const colText = css.getPropertyValue("--text-muted").trim() || "#888";
    const colHover = css.getPropertyValue("--text-dim").trim() || "#aaa";
    const font = css.getPropertyValue("--font-mono").trim() || "monospace";
    ctx.font = `11px ${font}`;

    let tMin = windowSec > 0 ? now - windowSec * 1000 : Infinity;
    if (windowSec === 0) {
      for (const s of series) if (s.points.length > 0) tMin = Math.min(tMin, s.points[0].t);
      if (!Number.isFinite(tMin)) tMin = now - 1000;
    }
    const tSpan = Math.max(1, now - tMin);

    let vMin = Infinity;
    let vMax = -Infinity;
    for (const s of series) {
      for (const p of s.points) {
        if (p.t < tMin || p.t > now) continue;
        if (p.v < vMin) vMin = p.v;
        if (p.v > vMax) vMax = p.v;
      }
    }
    const hasData = Number.isFinite(vMin);
    if (!hasData) {
      vMin = 0;
      vMax = 1;
    } else if (vMin === vMax) {
      vMin -= 1;
      vMax += 1;
    } else {
      const pad = (vMax - vMin) * 0.08;
      vMin -= pad;
      vMax += pad;
    }

    const w = width - PAD.l - PAD.r;
    const h = height - PAD.t - PAD.b;
    const xOf = (t: number) => PAD.l + ((t - tMin) / tSpan) * w;
    const yOf = (v: number) => PAD.t + (1 - (v - vMin) / (vMax - vMin)) * h;

    // Horizontal grid and Y labels.
    const step = niceStep(vMax - vMin, 5);
    ctx.strokeStyle = colGrid;
    ctx.fillStyle = colText;
    ctx.lineWidth = 1;
    ctx.textAlign = "right";
    ctx.textBaseline = "middle";
    for (let v = Math.ceil(vMin / step) * step; v <= vMax; v += step) {
      const y = Math.round(yOf(v)) + 0.5;
      ctx.beginPath();
      ctx.moveTo(PAD.l, y);
      ctx.lineTo(width - PAD.r, y);
      ctx.stroke();
      ctx.fillText(formatTick(v, step), PAD.l - 6, y);
    }

    // Time labels.
    ctx.textBaseline = "top";
    const ticks = Math.max(2, Math.min(6, Math.floor(w / 90)));
    for (let i = 0; i <= ticks; i++) {
      const frac = i / ticks;
      const x = PAD.l + frac * w;
      ctx.textAlign = i === 0 ? "left" : i === ticks ? "right" : "center";
      ctx.fillText(formatAge((tSpan / 1000) * (1 - frac)), x, height - PAD.b + 6);
    }

    // Series.
    ctx.save();
    ctx.beginPath();
    ctx.rect(PAD.l, PAD.t, w, h);
    ctx.clip();
    ctx.lineWidth = 1.5;
    ctx.lineJoin = "round";
    for (const s of series) {
      ctx.strokeStyle = s.color;
      ctx.beginPath();
      let started = false;
      for (const p of s.points) {
        if (p.t > now) break;
        const x = xOf(p.t);
        const y = yOf(p.v);
        if (!started) {
          ctx.moveTo(x, y);
          started = true;
        } else ctx.lineTo(x, y);
      }
      ctx.stroke();
      if (s.points.length === 1) {
        ctx.fillStyle = s.color;
        ctx.fillRect(xOf(s.points[0].t) - 2, yOf(s.points[0].v) - 2, 4, 4);
      }
    }
    ctx.restore();

    // Hover read-out.
    if (hoverX !== null && hoverX >= PAD.l && hoverX <= width - PAD.r) {
      const tHover = tMin + ((hoverX - PAD.l) / w) * tSpan;
      ctx.strokeStyle = colHover;
      ctx.setLineDash([3, 3]);
      ctx.beginPath();
      ctx.moveTo(Math.round(hoverX) + 0.5, PAD.t);
      ctx.lineTo(Math.round(hoverX) + 0.5, PAD.t + h);
      ctx.stroke();
      ctx.setLineDash([]);
      const rows: { color: string; text: string }[] = [];
      for (const s of series) {
        const i = lastAtOrBefore(s.points, tHover);
        if (i < 0) continue;
        const p = s.points[i];
        ctx.fillStyle = s.color;
        ctx.beginPath();
        ctx.arc(xOf(p.t), yOf(p.v), 3, 0, Math.PI * 2);
        ctx.fill();
        rows.push({ color: s.color, text: `${s.name}: ${Number(p.v.toPrecision(7))}${unit ? ` ${unit}` : ""}` });
      }
      if (rows.length > 0) {
        const header = new Date(tHover).toLocaleTimeString("en-US", {
          hour12: false,
          hour: "2-digit",
          minute: "2-digit",
          second: "2-digit",
          fractionalSecondDigits: 1,
        });
        const lines = [{ color: colText, text: header }, ...rows];
        const boxW = Math.max(...lines.map((l) => ctx.measureText(l.text).width)) + 24;
        const boxH = lines.length * 15 + 8;
        const bx = hoverX + 12 + boxW > width - PAD.r ? hoverX - 12 - boxW : hoverX + 12;
        const by = PAD.t + 4;
        ctx.fillStyle = css.getPropertyValue("--card").trim() || "#222";
        ctx.strokeStyle = colGrid;
        ctx.fillRect(bx, by, boxW, boxH);
        ctx.strokeRect(bx + 0.5, by + 0.5, boxW, boxH);
        ctx.textAlign = "left";
        ctx.textBaseline = "middle";
        lines.forEach((l, i) => {
          const y = by + 4 + i * 15 + 7;
          ctx.fillStyle = l.color;
          if (i > 0) ctx.fillRect(bx + 6, y - 3, 6, 6);
          ctx.fillStyle = i === 0 ? colText : css.getPropertyValue("--text").trim() || "#eee";
          ctx.fillText(l.text, bx + (i > 0 ? 17 : 6), y);
        });
      }
    }

    if (!hasData) {
      ctx.fillStyle = colText;
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.fillText("No samples in this window", PAD.l + w / 2, PAD.t + h / 2);
    }
  }, [series, now, windowSec, width, height, hoverX, unit]);

  return (
    <div ref={wrapRef} style={{ width: "100%" }}>
      <canvas
        ref={canvasRef}
        role="img"
        aria-label={label}
        style={{ width, height, display: "block" }}
        onMouseMove={(e) => setHoverX(e.clientX - e.currentTarget.getBoundingClientRect().left)}
        onMouseLeave={() => setHoverX(null)}
      />
    </div>
  );
}
