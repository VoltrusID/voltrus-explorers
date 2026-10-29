import { invoke } from "@tauri-apps/api/core";
import { Button, Callout, DataTable } from "@mactools/ui";
import { RefreshCw } from "lucide-react";
import { useState } from "react";
import { hint, mono, row } from "./ui";

export default function ProgramsView() {
  const [list, setList] = useState<string[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = async () => {
    setBusy(true);
    setError(null);
    try {
      setList(await invoke<string[]>("gem_ppid_list"));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, gap: 8 }}>
      <div style={row}>
        <Button size="sm" icon={<RefreshCw size={14} strokeWidth={1.75} />} onClick={() => void load()} disabled={busy}>List process programs</Button>
        {list && <span style={hint}>{list.length} process program(s)</span>}
        <span style={{ flex: 1 }} />
        <span style={hint}>S7F19 / S7F20. Transferring program bodies is not supported.</span>
      </div>
      {error && <Callout tone="danger">{error}</Callout>}
      <div style={{ flex: 1, minHeight: 120, display: "flex", flexDirection: "column" }}>
        <DataTable
          columns={[{ key: "n", label: "#", width: "60px", align: "right" }, { key: "ppid", label: "PPID" }]}
          data={(list ?? []).map((p, i) => ({ n: i + 1, ppid: p }))}
          emptyMessage={busy ? "Reading" : "List process programs (S7F19) to see the PPIDs on the equipment"}
          renderCell={(key, r) => (key === "n" ? mono(r[key]) : <span className="mono">{String(r[key])}</span>)}
        />
      </div>
    </div>
  );
}
