import { useEffect, useRef } from "react";

interface Props {
  points: number[];
  color?: string;
  width?: number;
  height?: number;
}

/** Minimal canvas sparkline for a live numeric series. */
export function Sparkline({ points, color = "#4f8cff", width = 240, height = 40 }: Props) {
  const ref = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = width * dpr;
    canvas.height = height * dpr;
    ctx.scale(dpr, dpr);
    ctx.clearRect(0, 0, width, height);

    const data = points.length > 1 ? points : [0, 0];
    const min = Math.min(...data);
    const max = Math.max(...data);
    const span = max - min || 1;
    const stepX = width / (data.length - 1);

    ctx.beginPath();
    data.forEach((v, i) => {
      const x = i * stepX;
      const y = height - 2 - ((v - min) / span) * (height - 4);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    });
    ctx.strokeStyle = color;
    ctx.lineWidth = 1.5;
    ctx.stroke();
  }, [points, color, width, height]);

  return <canvas ref={ref} style={{ width, height, display: "block" }} />;
}
