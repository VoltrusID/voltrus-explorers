import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Annotation, CaptureInfo, ChannelInfo } from "./types";
import { LEVEL_COLORS, channelColor, clampView, fmtTime, sampleTime, tickStep, zoomAt, type View } from "./logic";

export interface DecoderLane {
  id: number;
  title: string;
  rows: string[];
  /** Bumped when the decoder's results change. */
  revision: number;
}

interface Props {
  info: CaptureInfo;
  channels: ChannelInfo[];
  hidden: Set<number>;
  view: View;
  setView: (v: View) => void;
  decoders: DecoderLane[];
  cursors: [number | null, number | null];
  setCursors: (c: [number | null, number | null]) => void;
  highlight: { start: number; end: number } | null;
  onAnnotationClick?: (decoderId: number, a: Annotation) => void;
  onWidth?: (w: number) => void;
}

const LABEL_W = 132;
const RULER_H = 26;
const CH_H = 30;
const TITLE_H = 18;
const ROW_H = 20;

type Lane =
  | { kind: "channel"; ch: number; y: number; h: number }
  | { kind: "title"; dec: DecoderLane; y: number; h: number }
  | { kind: "row"; dec: DecoderLane; row: number; y: number; h: number };

function cssVar(name: string, fallback: string) {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return v || fallback;
}

function layout(channels: ChannelInfo[], hidden: Set<number>, decoders: DecoderLane[]): { lanes: Lane[]; height: number } {
  const lanes: Lane[] = [];
  let y = RULER_H;
  channels.forEach((_, ch) => {
    if (hidden.has(ch)) return;
    lanes.push({ kind: "channel", ch, y, h: CH_H });
    y += CH_H;
  });
  for (const dec of decoders) {
    y += 4;
    lanes.push({ kind: "title", dec, y, h: TITLE_H });
    y += TITLE_H;
    dec.rows.forEach((_, row) => {
      lanes.push({ kind: "row", dec, row, y, h: ROW_H });
      y += ROW_H;
    });
  }
  return { lanes, height: y + 8 };
}

export default function Waveform({ info, channels, hidden, view, setView, decoders, cursors, setCursors, highlight, onAnnotationClick, onWidth }: Props) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [width, setWidth] = useState(800);
  const [summary, setSummary] = useState<{ data: Uint8Array; start: number; end: number; buckets: number } | null>(null);
  const [anns, setAnns] = useState<Map<number, Annotation[]>>(new Map());
  const [hover, setHover] = useState<{ x: number; y: number } | null>(null);
  const drag = useRef<{ kind: "pan"; x0: number; start0: number; moved: boolean; shift: boolean } | { kind: "cursor"; which: 0 | 1 } | null>(null);
  const reqId = useRef(0);

  const plotW = Math.max(width - LABEL_W, 10);
  const { lanes, height } = useMemo(() => layout(channels, hidden, decoders), [channels, hidden, decoders]);

  // ── Size ──
  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setWidth(el.clientWidth));
    ro.observe(el);
    setWidth(el.clientWidth);
    return () => ro.disconnect();
  }, []);
  useEffect(() => { onWidth?.(plotW); }, [plotW, onWidth]);

  // ── Data for the visible window ──
  const decKey = decoders.map((d) => `${d.id}:${d.revision}`).join(",");
  useEffect(() => {
    const id = ++reqId.current;
    const start = Math.floor(view.start);
    const end = Math.min(info.samples, Math.ceil(view.start + view.spp * plotW) + 1);
    const buckets = Math.max(1, Math.min(plotW, end - start));
    const t = setTimeout(async () => {
      try {
        const data = await invoke<number[]>("la_waveform", { start, end, buckets });
        const per = await Promise.all(
          decoders.map((d) => invoke<Annotation[]>("la_annotations", { id: d.id, start, end, minWidth: view.spp * 3 }).catch(() => [] as Annotation[]).then((a): [number, Annotation[]] => [d.id, a])),
        );
        if (id !== reqId.current) return;
        setSummary({ data: Uint8Array.from(data), start, end, buckets });
        setAnns(new Map(per));
      } catch {
        /* capture replaced meanwhile */
      }
    }, 0);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view.start, view.spp, plotW, info, decKey]);

  // ── Drawing ──
  const draw = useCallback(() => {
    const cv = canvasRef.current;
    if (!cv) return;
    const dpr = window.devicePixelRatio || 1;
    cv.width = Math.round(width * dpr);
    cv.height = Math.round(height * dpr);
    cv.style.width = `${width}px`;
    cv.style.height = `${height}px`;
    const g = cv.getContext("2d")!;
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    const bg = cssVar("--bg", "#0b0b0a");
    const card = cssVar("--card", "#151514");
    const border = cssVar("--border", "#252422");
    const text = cssVar("--text", "#ece9e3");
    const muted = cssVar("--text-muted", "#8a867f");
    g.fillStyle = bg;
    g.fillRect(0, 0, width, height);
    const xOf = (s: number) => LABEL_W + (s - view.start) / view.spp;
    const font = (px: number, weight = 400) => `${weight} ${px}px ui-sans-serif, system-ui, -apple-system, sans-serif`;
    const mono = (px: number) => `${px}px ui-monospace, SFMono-Regular, Menlo, monospace`;

    // Ruler and grid.
    const sPerPx = view.spp / info.sampleRate;
    const step = tickStep(sPerPx);
    const t0 = sampleTime(info, view.start);
    const t1 = t0 + plotW * sPerPx;
    g.font = mono(10);
    g.textBaseline = "middle";
    for (let t = Math.ceil(t0 / step) * step; t <= t1; t += step) {
      const x = LABEL_W + (t - t0) / sPerPx;
      g.strokeStyle = border;
      g.lineWidth = 1;
      g.beginPath();
      g.moveTo(Math.round(x) + 0.5, RULER_H - 6);
      g.lineTo(Math.round(x) + 0.5, height);
      g.stroke();
      g.fillStyle = muted;
      g.fillText(fmtTime(Math.abs(t) < step / 1e6 ? 0 : t), x + 3, RULER_H / 2);
    }
    g.strokeStyle = border;
    g.beginPath();
    g.moveTo(0, RULER_H - 0.5);
    g.lineTo(width, RULER_H - 0.5);
    g.stroke();

    // Highlighted range (selected table row).
    if (highlight) {
      const a = Math.max(xOf(highlight.start), LABEL_W);
      const b = Math.min(xOf(highlight.end), width);
      if (b > a) {
        g.fillStyle = "rgba(62,142,247,0.10)";
        g.fillRect(a, RULER_H, Math.max(b - a, 1), height - RULER_H);
      }
    }

    // Channels.
    const s = summary;
    for (const lane of lanes) {
      if (lane.kind !== "channel") continue;
      const color = channelColor(channels[lane.ch], lane.ch);
      const hi = lane.y + 6;
      const lo = lane.y + lane.h - 6;
      g.strokeStyle = border;
      g.beginPath();
      g.moveTo(LABEL_W, lane.y + lane.h - 0.5);
      g.lineTo(width, lane.y + lane.h - 0.5);
      g.stroke();
      if (!s) continue;
      const bw = (s.end - s.start) / s.buckets / view.spp;
      const off = (s.start - view.start) / view.spp;
      g.strokeStyle = color;
      g.fillStyle = color + "55";
      g.lineWidth = 1.5;
      g.beginPath();
      let prev = -1;
      for (let i = 0; i < s.buckets; i++) {
        const v = s.data[lane.ch * s.buckets + i];
        const x0 = LABEL_W + off + i * bw;
        const x1 = x0 + bw;
        if (x1 < LABEL_W || x0 > width) {
          prev = v;
          continue;
        }
        if (v === 2) {
          g.fillRect(x0, hi, Math.max(bw, 1), lo - hi);
          g.moveTo(x0, hi);
          g.lineTo(x1, hi);
          g.moveTo(x0, lo);
          g.lineTo(x1, lo);
        } else {
          const y = v === 1 ? hi : lo;
          if (prev !== -1 && prev !== v) {
            g.moveTo(x0, hi);
            g.lineTo(x0, lo);
          }
          g.moveTo(x0, y);
          g.lineTo(x1, y);
        }
        prev = v;
      }
      g.stroke();
    }

    // Decoder rows.
    for (const lane of lanes) {
      if (lane.kind === "title") continue;
      if (lane.kind !== "row") continue;
      const list = anns.get(lane.dec.id) ?? [];
      const top = lane.y + 2;
      const h = lane.h - 4;
      for (const a of list) {
        if (a.row !== lane.row) continue;
        const x0 = Math.max(xOf(a.start), LABEL_W);
        const x1 = Math.min(xOf(a.end), width);
        if (x1 < LABEL_W || x0 > width) continue;
        const w = Math.max(x1 - x0, 1);
        const c = LEVEL_COLORS[a.level];
        if (!a.text) {
          // A dense block of boxes too narrow to draw one by one.
          g.fillStyle = c + "66";
          g.fillRect(x0, top + h / 2 - 2, w, 4);
          continue;
        }
        g.fillStyle = c + "30";
        g.strokeStyle = c;
        g.lineWidth = 1;
        g.beginPath();
        g.roundRect(x0 + 0.5, top + 0.5, Math.max(w - 1, 1), h - 1, 4);
        g.fill();
        if (w > 3) g.stroke();
        g.font = mono(10.5);
        const label = g.measureText(a.text).width < w - 8 ? a.text : g.measureText(a.short).width < w - 6 ? a.short : "";
        if (label) {
          g.fillStyle = text;
          g.fillText(label, x0 + Math.max(4, (w - g.measureText(label).width) / 2), top + h / 2 + 0.5);
        }
      }
    }

    // Trigger.
    if (info.trigger != null) {
      const x = xOf(info.trigger);
      if (x >= LABEL_W && x <= width) {
        g.strokeStyle = "#f08c00";
        g.lineWidth = 1.5;
        g.setLineDash([4, 3]);
        g.beginPath();
        g.moveTo(x, RULER_H);
        g.lineTo(x, height);
        g.stroke();
        g.setLineDash([]);
        g.fillStyle = "#f08c00";
        g.beginPath();
        g.moveTo(x - 5, RULER_H - 9);
        g.lineTo(x + 5, RULER_H - 9);
        g.lineTo(x, RULER_H - 2);
        g.fill();
      }
    }

    // Cursors.
    cursors.forEach((c, i) => {
      if (c == null) return;
      const x = xOf(c);
      if (x < LABEL_W || x > width) return;
      const col = i === 0 ? "#22d3ee" : "#e879f9";
      g.strokeStyle = col;
      g.lineWidth = 1.5;
      g.beginPath();
      g.moveTo(x, RULER_H);
      g.lineTo(x, height);
      g.stroke();
      g.fillStyle = col;
      g.beginPath();
      g.roundRect(x - 8, 2, 16, 16, 3);
      g.fill();
      g.fillStyle = "#000";
      g.font = font(10, 700);
      g.fillText(i === 0 ? "A" : "B", x - 3.5, 10.5);
    });

    // Label gutter on top of everything.
    g.fillStyle = card;
    g.fillRect(0, 0, LABEL_W, height);
    g.strokeStyle = border;
    g.beginPath();
    g.moveTo(LABEL_W - 0.5, 0);
    g.lineTo(LABEL_W - 0.5, height);
    g.stroke();
    g.font = font(10, 600);
    g.fillStyle = muted;
    g.fillText(info.trigger != null ? "t = 0 at trigger" : "t from start", 8, RULER_H / 2);
    for (const lane of lanes) {
      if (lane.kind === "channel") {
        const color = channelColor(channels[lane.ch], lane.ch);
        g.fillStyle = color;
        g.beginPath();
        g.roundRect(8, lane.y + lane.h / 2 - 5, 10, 10, 2);
        g.fill();
        g.fillStyle = muted;
        g.font = mono(10);
        g.fillText(`D${lane.ch}`, 24, lane.y + lane.h / 2);
        g.fillStyle = text;
        g.font = font(12, 500);
        const name = channels[lane.ch]?.name ?? "";
        let n = name;
        while (n.length > 1 && g.measureText(n).width > LABEL_W - 52) n = n.slice(0, -1);
        g.fillText(n === name ? n : n + "…", 46, lane.y + lane.h / 2);
      } else if (lane.kind === "title") {
        g.fillStyle = card;
        g.fillRect(0, lane.y, width, lane.h);
        g.fillStyle = muted;
        g.font = font(11, 600);
        g.fillText(lane.dec.title, 8, lane.y + lane.h / 2);
      } else if (lane.kind === "row") {
        g.fillStyle = muted;
        g.font = font(10.5);
        g.fillText(lane.dec.rows[lane.row], 18, lane.y + lane.h / 2);
      }
    }
  }, [width, height, view, info, plotW, lanes, summary, anns, channels, cursors, highlight]);

  useEffect(() => { draw(); }, [draw]);
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: light)");
    mq.addEventListener("change", draw);
    return () => mq.removeEventListener("change", draw);
  }, [draw]);

  // ── Interaction ──
  const sampleAt = (x: number) => view.start + (x - LABEL_W) * view.spp;
  const pos = (e: { clientX: number; clientY: number }) => {
    const r = canvasRef.current!.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  };
  const laneAt = (y: number) => lanes.find((l) => y >= l.y && y < l.y + l.h);

  /** Places a cursor, snapped to the nearest edge of the channel under the pointer when close. */
  const placeCursor = async (which: 0 | 1, x: number, y: number) => {
    let s = Math.round(sampleAt(x));
    const lane = laneAt(y);
    if (lane?.kind === "channel") {
      try {
        const [n, p] = await Promise.all([
          invoke<number | null>("la_edge", { channel: lane.ch, from: Math.max(s, 0), forward: true }),
          invoke<number | null>("la_edge", { channel: lane.ch, from: Math.max(s, 0), forward: false }),
        ]);
        const near = [n, p].filter((e): e is number => e != null).sort((a, b) => Math.abs(a - s) - Math.abs(b - s))[0];
        if (near != null && Math.abs(near - s) / view.spp <= 8) s = near;
      } catch { /* no capture */ }
    }
    s = Math.min(Math.max(s, 0), info.samples);
    const next: [number | null, number | null] = [...cursors];
    next[which] = s;
    setCursors(next);
  };

  const onMouseDown = (e: React.MouseEvent) => {
    const { x } = pos(e);
    if (x < LABEL_W) return;
    for (const which of [0, 1] as const) {
      const c = cursors[which];
      if (c != null && Math.abs(LABEL_W + (c - view.start) / view.spp - x) < 5) {
        drag.current = { kind: "cursor", which };
        return;
      }
    }
    drag.current = { kind: "pan", x0: x, start0: view.start, moved: false, shift: e.shiftKey || e.altKey };
  };

  const onMouseMove = (e: React.MouseEvent) => {
    const p = pos(e);
    setHover(p);
    const d = drag.current;
    if (!d) return;
    if (d.kind === "cursor") {
      const next: [number | null, number | null] = [...cursors];
      next[d.which] = Math.min(Math.max(Math.round(sampleAt(p.x)), 0), info.samples);
      setCursors(next);
      return;
    }
    if (Math.abs(p.x - d.x0) > 3) d.moved = true;
    if (d.moved) setView(clampView({ start: d.start0 - (p.x - d.x0) * view.spp, spp: view.spp }, info.samples, plotW));
  };

  const onMouseUp = (e: React.MouseEvent) => {
    const d = drag.current;
    drag.current = null;
    if (!d || d.kind !== "pan" || d.moved) return;
    const p = pos(e);
    const lane = laneAt(p.y);
    if (lane?.kind === "row" && onAnnotationClick) {
      const s = sampleAt(p.x);
      const hit = (anns.get(lane.dec.id) ?? []).find((a) => a.row === lane.row && a.text && s >= a.start && s < a.end);
      if (hit) {
        onAnnotationClick(lane.dec.id, hit);
        return;
      }
    }
    void placeCursor(d.shift ? 1 : 0, p.x, p.y);
  };

  // Wheel: zoom (vertical) and pan (horizontal or with Shift). Not passive, so the page does not scroll sideways.
  useEffect(() => {
    const cv = canvasRef.current;
    if (!cv) return;
    const onWheel = (e: WheelEvent) => {
      const r = cv.getBoundingClientRect();
      const x = e.clientX - r.left;
      if (x < LABEL_W) return;
      const horizontal = e.shiftKey || Math.abs(e.deltaX) > Math.abs(e.deltaY);
      if (horizontal) {
        e.preventDefault();
        const dx = e.shiftKey ? e.deltaY : e.deltaX;
        setView(clampView({ start: view.start + dx * view.spp, spp: view.spp }, info.samples, plotW));
      } else if (e.ctrlKey || e.metaKey || !e.altKey) {
        e.preventDefault();
        setView(zoomAt(view, Math.pow(1.0018, -e.deltaY * (e.deltaMode === 1 ? 30 : 1)), x - LABEL_W, info.samples, plotW));
      }
    };
    cv.addEventListener("wheel", onWheel, { passive: false });
    return () => cv.removeEventListener("wheel", onWheel);
  }, [view, info.samples, plotW, setView]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    const mid = plotW / 2;
    if (e.key === "+" || e.key === "=") setView(zoomAt(view, 2, mid, info.samples, plotW));
    else if (e.key === "-" || e.key === "_") setView(zoomAt(view, 0.5, mid, info.samples, plotW));
    else if (e.key === "ArrowLeft") setView(clampView({ start: view.start - plotW * view.spp * 0.2, spp: view.spp }, info.samples, plotW));
    else if (e.key === "ArrowRight") setView(clampView({ start: view.start + plotW * view.spp * 0.2, spp: view.spp }, info.samples, plotW));
    else if (e.key === "Home") setView(clampView({ start: 0, spp: view.spp }, info.samples, plotW));
    else if (e.key === "End") setView(clampView({ start: info.samples, spp: view.spp }, info.samples, plotW));
    else if (e.key === "f" || e.key === "F") setView(clampView({ start: 0, spp: Infinity }, info.samples, plotW));
    else return;
    e.preventDefault();
  };

  // Hover readout: time under the pointer, or the full text of the annotation under it.
  let tip: string | null = null;
  if (hover && hover.x >= LABEL_W) {
    const s = sampleAt(hover.x);
    const lane = laneAt(hover.y);
    if (lane?.kind === "row") {
      const a = (anns.get(lane.dec.id) ?? []).find((x) => x.row === lane.row && x.text && s >= x.start && s < x.end);
      if (a) tip = a.text;
    }
    tip ??= `${fmtTime(sampleTime(info, s))} · sample ${Math.max(0, Math.floor(s)).toLocaleString()}`;
  }

  return (
    <div ref={wrapRef} style={{ position: "relative", flex: 1, minHeight: 0, overflowY: "auto", overflowX: "hidden", borderRadius: "var(--radius-lg)", border: "1px solid var(--border)" }}>
      <canvas
        ref={canvasRef}
        tabIndex={0}
        aria-label="Waveform. Scroll to zoom, drag to pan, click to place cursor A, Shift-click for cursor B. Keys: plus, minus, arrows, F to fit."
        style={{ display: "block", cursor: drag.current?.kind === "pan" && drag.current.moved ? "grabbing" : "crosshair", outline: "none" }}
        onMouseDown={onMouseDown}
        onMouseMove={onMouseMove}
        onMouseUp={onMouseUp}
        onMouseLeave={() => { setHover(null); drag.current = null; }}
        onKeyDown={onKeyDown}
      />
      {tip && hover && (
        <div
          className="mono"
          style={{
            position: "absolute",
            left: Math.min(hover.x + 12, width - 260),
            top: hover.y + 14,
            pointerEvents: "none",
            background: "var(--card)",
            border: "1px solid var(--border-light)",
            borderRadius: 6,
            padding: "3px 7px",
            fontSize: 11,
            color: "var(--text)",
            maxWidth: 420,
            whiteSpace: "nowrap",
            overflow: "hidden",
            textOverflow: "ellipsis",
            boxShadow: "var(--shadow-pop)",
          }}
        >
          {tip}
        </div>
      )}
    </div>
  );
}
