import { Circle, Square } from "lucide-react";
import { Button, ProBadge, Section, useProStatus } from "@mactools/ui";

export interface CaptureState {
  path: string;
  rows: number;
}

interface Props {
  capture: CaptureState | null;
  onStart: () => void;
  onStop: () => void;
}

/** Writes every RX and TX chunk of all open ports to a CSV file as it happens. */
export function CaptureSection({ capture, onStart, onStop }: Props) {
  const { pro } = useProStatus();
  const fileName = capture?.path.split(/[\\/]/).pop();
  return (
    <Section title="Capture to file" meta={pro ? undefined : <ProBadge />}>
      {capture ? (
        <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)", lineHeight: 1.45, wordBreak: "break-all" }} title={capture.path}>
          Writing to {fileName}: {capture.rows} {capture.rows === 1 ? "row" : "rows"}
        </div>
      ) : (
        <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)", lineHeight: 1.45 }}>
          Saves all traffic of every open port to a CSV file while you work, with timestamp, port, direction, hex and text.
        </div>
      )}
      <Button
        block
        variant={capture ? "danger" : "default"}
        icon={capture ? <Square size={14} strokeWidth={1.75} /> : <Circle size={14} strokeWidth={1.75} />}
        onClick={capture ? onStop : onStart}
      >
        {capture ? "Stop capture" : "Capture to file..."}
      </Button>
    </Section>
  );
}
