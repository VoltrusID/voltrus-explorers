import { useCallback, useRef, useEffect } from "react";

interface UseResizeOptions {
  direction: "horizontal" | "vertical";
  min: number;
  max: number;
  defaultSize: number;
  onResize?: (size: number) => void;
}

export function useResize(options: UseResizeOptions) {
  const { direction, min, max, defaultSize, onResize } = options;
  const sizeRef = useRef(defaultSize);
  const dragging = useRef(false);
  const startPos = useRef(0);
  const startSize = useRef(0);
  const elementRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    sizeRef.current = defaultSize;
  }, [defaultSize]);

  const onMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      dragging.current = true;
      startPos.current = direction === "horizontal" ? e.clientX : e.clientY;
      startSize.current = sizeRef.current;
      document.body.style.cursor = direction === "horizontal" ? "col-resize" : "row-resize";
      document.body.style.userSelect = "none";

      const onMouseMove = (ev: MouseEvent) => {
        if (!dragging.current) return;
        const delta =
          direction === "horizontal"
            ? ev.clientX - startPos.current
            : ev.clientY - startPos.current;
        const newSize = Math.min(max, Math.max(min, startSize.current + delta));
        sizeRef.current = newSize;
        if (elementRef.current) {
          elementRef.current.style.setProperty(
            "--resize-size",
            `${newSize}px`
          );
        }
        onResize?.(newSize);
      };

      const onMouseUp = () => {
        dragging.current = false;
        document.body.style.cursor = "";
        document.body.style.userSelect = "";
        document.removeEventListener("mousemove", onMouseMove);
        document.removeEventListener("mouseup", onMouseUp);
      };

      document.addEventListener("mousemove", onMouseMove);
      document.addEventListener("mouseup", onMouseUp);
    },
    [direction, min, max, onResize]
  );

  return { onMouseDown, elementRef };
}
