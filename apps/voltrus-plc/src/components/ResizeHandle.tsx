import { useCallback, useRef, useEffect } from "react";

interface ResizeHandleProps {
  axis: "x" | "y";
  onDrag: (delta: number) => void;
}

export default function ResizeHandle({ axis, onDrag }: ResizeHandleProps) {
  const dragging = useRef(false);
  const start = useRef(0);

  const onMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      dragging.current = true;
      start.current = axis === "x" ? e.clientX : e.clientY;
      document.body.style.cursor = axis === "x" ? "col-resize" : "row-resize";
      document.body.style.userSelect = "none";

      const move = (ev: MouseEvent) => {
        if (!dragging.current) return;
        const pos = axis === "x" ? ev.clientX : ev.clientY;
        onDrag(pos - start.current);
        start.current = pos;
      };

      const up = () => {
        dragging.current = false;
        document.body.style.cursor = "";
        document.body.style.userSelect = "";
        document.removeEventListener("mousemove", move);
        document.removeEventListener("mouseup", up);
      };

      document.addEventListener("mousemove", move);
      document.addEventListener("mouseup", up);
    },
    [axis, onDrag]
  );

  return (
    <div
      className={`resize-handle resize-handle-${axis}`}
      role="separator"
      aria-orientation={axis === "x" ? "vertical" : "horizontal"}
      onMouseDown={onMouseDown}
    />
  );
}
