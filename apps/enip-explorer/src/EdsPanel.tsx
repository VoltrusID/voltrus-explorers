import { useState } from "react";
import { FileText, Trash2 } from "lucide-react";
import { Button, Callout, Dialog, openTextFile } from "@mactools/ui";
import { parseEds, saveEds, clearEds, type Eds } from "./eds";

interface Props {
  eds: Eds | null;
  onImport: (eds: Eds) => void;
  onClear: () => void;
  onClose: () => void;
}

export function EdsPanel({ eds, onImport, onClear, onClose }: Props) {
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  const onPick = async () => {
    setError("");
    setBusy(true);
    try {
      const file = await openTextFile({ name: "EDS", extensions: ["eds", "txt"] });
      if (!file) return;
      const name = file.path.split(/[\\/]/).pop() ?? "device.eds";
      const parsed = parseEds(file.text, name);
      if (parsed.params.length === 0 && parsed.assemblies.length === 0) {
        setError("No [Param]/[Assembly] sections found. Is this a valid EDS file?");
      } else {
        saveEds(parsed);
        onImport(parsed);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open onClose={onClose} title="EDS import" width={460}>
      <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        <div style={{ fontSize: 12, color: "var(--text-muted)", lineHeight: 1.5 }}>
          Import a device <code className="mono">.eds</code> file to show friendly parameter names ("Motor Speed") in
          the CIP tree instead of raw class/instance/attribute. Parses <strong>[Param]</strong>,{" "}
          <strong>[Assembly]</strong> and <strong>[Enum]</strong> sections.
        </div>

        {eds ? (
          <div style={{ padding: 10, border: "1px solid var(--border)", borderRadius: 6, background: "var(--bg)", display: "flex", flexDirection: "column", gap: 8 }}>
            <div>
              <div style={{ fontSize: 12, color: "var(--text)" }}>{eds.fileName}</div>
              <div style={{ fontSize: 11, color: "var(--text-muted)", marginTop: 4 }}>
                {eds.params.length} params · {eds.assemblies.length} assemblies · {eds.enums.length} enums
              </div>
            </div>
            <div>
              <Button
                variant="danger"
                size="sm"
                icon={<Trash2 size={14} />}
                onClick={() => {
                  clearEds();
                  onClear();
                }}
              >
                Remove EDS
              </Button>
            </div>
          </div>
        ) : (
          <div>
            <Button size="sm" icon={<FileText size={14} />} loading={busy} onClick={onPick}>
              Choose EDS file
            </Button>
          </div>
        )}

        {error && (
          <Callout tone="danger">
            <span style={{ fontSize: 12 }}>{error}</span>
          </Callout>
        )}
      </div>
    </Dialog>
  );
}
