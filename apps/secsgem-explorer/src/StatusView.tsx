import { Button, Callout, DataTable, Field, IconButton, ProBadge } from "@mactools/ui";
import { Plus, RefreshCw, X } from "lucide-react";
import { useMemo } from "react";
import { liveSelectionAllowed } from "./storage";
import type { useStatusVars } from "./useStatusVars";
import { hint, mono, row, type RequirePro } from "./ui";

type Sv = ReturnType<typeof useStatusVars>;

interface Props {
  sv: Sv;
  pro: boolean;
  requirePro: RequirePro;
  trending: Set<string>;
  trendLabel: (id: number) => string;
  onToggleTrend: (label: string) => void;
}

export default function StatusView({ sv, pro, requirePro, trending, trendLabel, onToggleTrend }: Props) {
  const { names, values, selected } = sv;
  const data = useMemo(
    () =>
      names.map((n) => ({
        id: n.id,
        sel: selected.has(n.id),
        name: n.name,
        value: values.get(n.id)?.text ?? "",
        kind: values.get(n.id)?.kind ?? "",
        units: n.units,
        numeric: values.get(n.id)?.number != null,
      })),
    [names, values, selected],
  );

  const toggleLive = () => {
    if (sv.live) return sv.setLive(false);
    if (!liveSelectionAllowed(selected.size, pro) && !requirePro("More than 10 live status variables")) return;
    sv.setLive(true);
  };

  const select = (id: number) => {
    if (!selected.has(id) && !liveSelectionAllowed(selected.size + 1, pro) && sv.live && !requirePro("More than 10 live status variables")) return;
    sv.toggle(id);
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, gap: 8 }}>
      <div style={row}>
        <Button size="sm" icon={<RefreshCw size={14} strokeWidth={1.75} />} onClick={() => void sv.loadNames()} disabled={sv.busy}>
          {names.length ? "Reload names" : "Load names"}
        </Button>
        <Button size="sm" onClick={() => void sv.readSelected(names.map((n) => n.id))} disabled={sv.busy || names.length === 0}>
          Read all
        </Button>
        <Button size="sm" onClick={() => void sv.readSelected()} disabled={sv.busy || selected.size === 0}>
          Read selected ({selected.size})
        </Button>
        <Button size="sm" variant={sv.live ? "primary" : undefined} onClick={toggleLive} disabled={selected.size === 0 && !sv.live}>
          {sv.live ? "Stop live poll" : "Live poll selected"}
        </Button>
        <Field label="Interval (ms)">
          <input
            className="vt-input vt-input--mono"
            style={{ width: 90 }}
            type="number"
            min={250}
            step={250}
            value={sv.intervalMs}
            onChange={(e) => sv.setIntervalMs(Number(e.target.value))}
          />
        </Field>
        <span style={{ flex: 1 }} />
        {!pro && <span style={hint}>Free: {10} live variables. Trends <ProBadge /></span>}
      </div>
      {sv.error && <Callout tone="danger">{sv.error}</Callout>}
      <div style={{ flex: 1, minHeight: 160, display: "flex", flexDirection: "column" }}>
        <DataTable
          columns={[
            { key: "sel", label: "", width: "44px", align: "center" },
            { key: "id", label: "SVID", width: "80px", align: "right" },
            { key: "name", label: "Name" },
            { key: "value", label: "Value", align: "right" },
            { key: "kind", label: "Type", width: "60px" },
            { key: "units", label: "Units", width: "90px" },
            { key: "trend", label: "Trend", width: "56px", align: "center" },
          ]}
          data={data}
          emptyMessage={sv.busy ? "Reading" : "Load names (S1F11) to list the status variables"}
          renderCell={(key, r) => {
            const id = Number(r.id);
            if (key === "sel") {
              return <input type="checkbox" aria-label={`Select SVID ${id}`} checked={Boolean(r.sel)} onChange={() => select(id)} />;
            }
            if (key === "trend") {
              if (!r.numeric) return null;
              const label = trendLabel(id);
              const active = trending.has(label);
              return (
                <IconButton
                  size="sm"
                  label={active ? `Remove ${label} from trend` : `Add ${label} to trend`}
                  pressed={active}
                  onClick={(e) => {
                    e.stopPropagation();
                    if (!active && !requirePro("Live trends")) return;
                    if (!selected.has(id)) sv.toggle(id);
                    onToggleTrend(label);
                  }}
                >
                  {active ? <X size={14} strokeWidth={1.75} /> : <Plus size={14} strokeWidth={1.75} />}
                </IconButton>
              );
            }
            return key === "value" || key === "id" ? mono(r[key]) : String(r[key] ?? "");
          }}
        />
      </div>
    </div>
  );
}
