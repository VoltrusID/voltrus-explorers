import { useEffect, useMemo, useState } from "react";
import { Button, Dialog, Field } from "@mactools/ui";
import { decodedPretty, toHex, type Decoded, type PayloadInfo } from "./payload";
import { topicMatches } from "./sessions";

const hint: React.CSSProperties = {
  margin: 0,
  fontSize: "var(--fs-xs)",
  color: "var(--text-muted)",
  lineHeight: 1.4,
};

const preStyle: React.CSSProperties = {
  margin: 0,
  padding: 8,
  maxHeight: 360,
  overflow: "auto",
  whiteSpace: "pre-wrap",
  wordBreak: "break-word",
  fontFamily: "var(--font-mono)",
  fontSize: "var(--fs-xs)",
  background: "var(--card)",
  border: "1px solid var(--border)",
  borderRadius: "var(--radius-sm)",
};

/** Full decoded tree of one message, with the raw bytes underneath. */
export function PayloadInspector({
  topic,
  info,
  onClose,
}: {
  topic: string;
  info: PayloadInfo & { decoded: Decoded };
  onClose: () => void;
}) {
  const label = info.decoded.kind === "sparkplug" ? "Sparkplug B" : "Protobuf";
  const text = decodedPretty(info.decoded);
  return (
    <Dialog
      open
      onClose={onClose}
      title={`${label} payload`}
      width={620}
      footer={
        <>
          <Button size="sm" variant="ghost" onClick={() => void navigator.clipboard.writeText(text)}>
            Copy
          </Button>
          <Button size="sm" onClick={onClose}>Close</Button>
        </>
      }
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        <span style={{ ...hint, fontFamily: "var(--font-mono)" }}>{topic}</span>
        <pre style={{ ...preStyle, color: info.decoded.error ? "var(--danger, #ef4444)" : undefined }}>{text}</pre>
        <Field label="Raw bytes">
          <pre style={{ ...preStyle, maxHeight: 100 }}>{toHex(info)}</pre>
        </Field>
      </div>
    </Dialog>
  );
}

/**
 * Publish an empty retained message on each topic so the broker forgets its
 * stored message. Lists only topics whose retained delivery this session saw.
 */
export function ClearRetainedDialog({
  open,
  onClose,
  candidates,
  defaultFilter,
  onClear,
}: {
  open: boolean;
  onClose: () => void;
  /** Topics with a retained message seen on the active connection. */
  candidates: string[];
  defaultFilter: string;
  /** Resolves with how many topics were cleared. */
  onClear: (topics: string[]) => Promise<number>;
}) {
  const [filter, setFilter] = useState("#");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      setFilter(defaultFilter || "#");
      setResult(null);
    }
  }, [open, defaultFilter]);

  const matching = useMemo(
    () => candidates.filter((t) => filter.trim() === "" || topicMatches(filter.trim(), t)),
    [candidates, filter],
  );

  const run = async () => {
    setBusy(true);
    try {
      const n = await onClear(matching);
      setResult(`Cleared ${n} of ${matching.length} retained message${matching.length === 1 ? "" : "s"}.`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Clear retained messages"
      width={460}
      footer={
        <>
          <Button size="sm" variant="ghost" onClick={onClose}>Close</Button>
          <Button size="sm" variant="danger" disabled={busy || matching.length === 0} onClick={() => void run()}>
            Clear {matching.length} retained
          </Button>
        </>
      }
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        <p style={hint}>
          Publishes an empty retained message on each topic below, which makes the broker delete the stored message.
          Only topics whose retained message was delivered on this connection are listed, so subscribe to the filter
          first. This changes data on the broker and cannot be undone.
        </p>
        <Field label="Topic filter">
          <input
            className="vt-input vt-input--sm vt-input--mono"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="#"
          />
        </Field>
        <pre style={{ ...preStyle, maxHeight: 200 }}>
          {matching.length === 0 ? "No retained messages match." : matching.join("\n")}
        </pre>
        {result && <p style={hint} role="status">{result}</p>}
      </div>
    </Dialog>
  );
}
