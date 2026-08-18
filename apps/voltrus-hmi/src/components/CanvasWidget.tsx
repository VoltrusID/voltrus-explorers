import { useRef, useCallback } from "react";
import type { HmiWidget } from "../types";
import { useStore } from "../store";
import {
  GaugePreview,
  IndicatorPreview,
  ButtonPreview,
  SliderPreview,
  ChartPreview,
  TextPreview,
} from "./WidgetPreviews";

export function CanvasWidget({
  widget,
  selected,
  onSelect,
  onDragStart,
  onMove,
  onResize,
  zoom,
  snapEnabled,
  dragOverTag,
  onDragOverTag,
  onDragLeaveTag,
  onDropTag,
}: {
  widget: HmiWidget;
  selected: boolean;
  onSelect: () => void;
  /** Called once at drag/resize start — pushes the single undo checkpoint. */
  onDragStart: () => void;
  onMove: (x: number, y: number) => void;
  onResize: (w: number, h: number) => void;
  zoom: number;
  snapEnabled: boolean;
  dragOverTag: boolean;
  onDragOverTag: (e: React.DragEvent) => void;
  onDragLeaveTag: (e: React.DragEvent) => void;
  onDropTag: (e: React.DragEvent) => void;
}) {
  const elRef = useRef<HTMLDivElement>(null);
  const dragState = useRef<{
    type: "move" | "resize";
    handle: string;
    startX: number;
    startY: number;
    origX: number;
    origY: number;
    origW: number;
    origH: number;
  } | null>(null);

  // Subscribe to only this widget's live value — returns a primitive, so the
  // widget re-renders when its value changes, not on every PLC frame.
  const liveValue = useStore((s) =>
    widget.variableName
      ? s.plcVariables.find((v) => v.name === widget.variableName)?.value
      : undefined
  );

  const snap = useCallback(
    (val: number) => (snapEnabled ? Math.round(val / 10) * 10 : val),
    [snapEnabled]
  );

  const handleMouseDown = useCallback(
    (e: React.MouseEvent, type: "move" | "resize", handle: string) => {
      e.stopPropagation();
      e.preventDefault();
      onSelect();
      // One checkpoint per gesture; the per-mousemove updates skip the stack.
      onDragStart();
      dragState.current = {
        type,
        handle,
        startX: e.clientX,
        startY: e.clientY,
        origX: widget.x,
        origY: widget.y,
        origW: widget.width,
        origH: widget.height,
      };

      const handleMouseMove = (e: MouseEvent) => {
        if (!dragState.current) return;
        const dx = (e.clientX - dragState.current.startX) / zoom;
        const dy = (e.clientY - dragState.current.startY) / zoom;

        if (dragState.current.type === "move") {
          onMove(snap(dragState.current.origX + dx), snap(dragState.current.origY + dy));
        } else {
          let newW = dragState.current.origW;
          let newH = dragState.current.origH;
          let newX = dragState.current.origX;
          let newY = dragState.current.origY;
          const h = dragState.current.handle;

          // Resize logic per handle
          if (h.includes("e")) newW = dragState.current.origW + dx;
          if (h.includes("w")) { newW = dragState.current.origW - dx; newX = dragState.current.origX + dx; }
          if (h.includes("s")) newH = dragState.current.origH + dy;
          if (h.includes("n")) { newH = dragState.current.origH - dy; newY = dragState.current.origY + dy; }

          newW = Math.max(30, newW);
          newH = Math.max(30, newH);

          onMove(snap(newX), snap(newY));
          onResize(newW, newH);
        }
      };

      const handleMouseUp = () => {
        dragState.current = null;
        document.removeEventListener("mousemove", handleMouseMove);
        document.removeEventListener("mouseup", handleMouseUp);
      };

      document.addEventListener("mousemove", handleMouseMove);
      document.addEventListener("mouseup", handleMouseUp);
    },
    [widget, onSelect, onDragStart, onMove, onResize, zoom, snap]
  );

  const renderPreview = () => {
    switch (widget.type) {
      case "gauge":
        return <GaugePreview widget={widget} liveValue={liveValue} />;
      case "indicator":
        return <IndicatorPreview widget={widget} liveValue={liveValue} />;
      case "button":
        return <ButtonPreview widget={widget} liveValue={liveValue} />;
      case "slider":
        return <SliderPreview widget={widget} liveValue={liveValue} />;
      case "chart":
        return <ChartPreview widget={widget} liveValue={liveValue} />;
      case "text":
        return <TextPreview widget={widget} liveValue={liveValue} />;
    }
  };

  const resizeHandles = ["nw", "n", "ne", "e", "se", "s", "sw", "w"] as const;

  return (
    <div
      ref={elRef}
      className={`canvas-widget ${selected ? "selected" : ""} ${dragOverTag ? "drag-over-tag" : ""}`}
      style={{
        left: widget.x,
        top: widget.y,
        width: widget.width,
        height: widget.height,
        zIndex: widget.zIndex ?? 0,
      }}
      onMouseDown={(e) => handleMouseDown(e, "move", "")}
      onDragOver={onDragOverTag}
      onDragLeave={onDragLeaveTag}
      onDrop={onDropTag}
    >
      {renderPreview()}

      {/* Binding status dot */}
      <div className={`widget-binding-dot ${widget.variableName ? "bound" : "unbound"}`} title={widget.variableName || "Not bound"} />

      {/* Variable name label at bottom */}
      {widget.variableName && !["gauge", "indicator", "slider", "chart", "text"].includes(widget.type) && (
        <span className="widget-var-label">{widget.variableName}</span>
      )}

      {/* Resize handles */}
      {selected &&
        resizeHandles.map((h) => (
          <div
            key={h}
            className={`resize-handle resize-handle-${h}`}
            onMouseDown={(e) => handleMouseDown(e, "resize", h)}
          />
        ))}
    </div>
  );
}
