import React from "react";
import { useStore } from "../../store";
import { Badge, Button, EmptyState } from "@mactools/ui";
import { Play, RotateCcw } from "lucide-react";

function groupByPrefix(names: string[]): Map<string, string[]> {
  const groups = new Map<string, string[]>();
  for (const name of names) {
    const match = name.match(/^[A-Za-z]+/);
    const prefix = match ? match[0] : "Other";
    const list = groups.get(prefix) ?? [];
    list.push(name);
    groups.set(prefix, list);
  }
  return groups;
}

const PREFIX_LABELS: Record<string, string> = {
  X: "Inputs",
  Y: "Outputs",
  M: "Internal relays",
  D: "Data registers",
  T: "Timers",
  C: "Counters",
};

const PREFIX_ORDER = ["X", "Y", "M", "D", "T", "C"];

export default function SimulationPanel() {
  const compiledVariables = useStore((s) => s.compiledVariables);
  const watchedVariables = useStore((s) => s.watchedVariables);
  const runtimeState = useStore((s) => s.runtimeState);
  const forceVariable = useStore((s) => s.forceVariable);
  const unforceVariable = useStore((s) => s.unforceVariable);
  const simulationInputs = useStore((s) => s.simulationInputs);
  const toggleSimulationInput = useStore((s) => s.toggleSimulationInput);
  const resetSimulationInputs = useStore((s) => s.resetSimulationInputs);

  if (runtimeState === "stopped") {
    return (
      <EmptyState
        compact
        icon={<Play size={18} strokeWidth={1.75} />}
        title="Simulation unavailable"
        description="Compile and run the program to enable simulation."
      />
    );
  }

  if (compiledVariables.length === 0) {
    return (
      <EmptyState
        compact
        icon={<Play size={18} strokeWidth={1.75} />}
        title="No compiled variables"
        description="Compile the program first to simulate its variables."
      />
    );
  }

  const watchMap = new Map(watchedVariables.map((w) => [w.name, w]));

  // Classify variables
  const inputNames: string[] = [];
  const outputNames: string[] = [];
  const otherNames: string[] = [];

  for (const v of compiledVariables) {
    const prefix = v.name.match(/^[A-Za-z]+/)?.[0] ?? "";
    if (prefix.startsWith("X")) {
      inputNames.push(v.name);
    } else if (prefix.startsWith("Y")) {
      outputNames.push(v.name);
    } else {
      otherNames.push(v.name);
    }
  }

  const isBool = (name: string) => {
    const w = watchMap.get(name);
    if (w) return w.type_name === "BOOL";
    const cv = compiledVariables.find((v) => v.name === name);
    return cv?.type_name === "BOOL";
  };

  const getDisplayValue = (name: string) => {
    const w = watchMap.get(name);
    if (w) return w.display_value;
    return "—";
  };

  const getQuality = (name: string) => watchMap.get(name)?.quality ?? "good";

  const handleToggle = toggleSimulationInput;

  const inputGroups = groupByPrefix(inputNames);
  const outputGroups = groupByPrefix(outputNames);
  const otherGroups = groupByPrefix(otherNames);

  const orderedOtherPrefixes = PREFIX_ORDER.filter((p) => otherGroups.has(p));
  for (const p of otherGroups.keys()) {
    if (!orderedOtherPrefixes.includes(p)) orderedOtherPrefixes.push(p);
  }

  const forcedTag = (
    <Badge tone="warn">Forced</Badge>
  );

  const renderToggle = (name: string) => {
    const forced = simulationInputs.has(name);
    const value = forced ? simulationInputs.get(name) : watchMap.get(name)?.value;
    const isOn = value === true || value === 1 || value === "TRUE" || value === "1";

    return (
      <div key={name} className="sim-row">
        <button
          className="sim-toggle"
          aria-pressed={isOn}
          onClick={() => handleToggle(name)}
          title={forced ? "Forced. Click to unforce" : "Click to force toggle"}
          aria-label={`Toggle ${name}`}
        />
        <span className="sim-name">{name}</span>
        <span style={{ color: isOn ? "var(--accent-text)" : "var(--text-muted)" }}>
          {isOn ? "ON" : "OFF"}
        </span>
        {forced && forcedTag}
      </div>
    );
  };

  const renderOutput = (name: string) => {
    const w = watchMap.get(name);
    const isOn = w?.value === true || w?.value === 1 || w?.display_value === "TRUE" || w?.display_value === "1";
    const quality = getQuality(name);

    return (
      <div key={name} className="sim-row">
        <div className={`sim-led ${isOn ? "is-on" : ""}`} />
        <span className="sim-name">{name}</span>
        <span style={{ color: isOn ? "var(--accent-text)" : "var(--text-muted)" }}>
          {getDisplayValue(name)}
        </span>
        {quality === "forced" && forcedTag}
      </div>
    );
  };

  const renderVariable = (name: string) => {
    const w = watchMap.get(name);
    const quality = getQuality(name);
    const isBoolVar = isBool(name);

    if (isBoolVar) {
      const isOn = w?.value === true || w?.value === 1;
      return (
        <div key={name} className="sim-row">
          <div className={`sim-pin-led ${isOn ? "is-on" : ""}`} />
          <span className="sim-name">{name}</span>
          <span style={{ color: "var(--text)" }}>{getDisplayValue(name)}</span>
          {quality === "forced" && forcedTag}
        </div>
      );
    }

    return (
      <div key={name} className="sim-row">
        <span className="sim-name">{name}</span>
        <span style={{ color: "var(--text)" }}>{getDisplayValue(name)}</span>
        {quality === "forced" && forcedTag}
      </div>
    );
  };

  const renderGroup = (
    prefix: string,
    names: string[],
    renderer: (name: string) => React.ReactNode,
  ) => (
    <div key={prefix} className="sim-group">
      <div className="vt-label" style={{ marginBottom: 4 }}>
        {PREFIX_LABELS[prefix] ?? prefix} ({names.length})
      </div>
      {names.map(renderer)}
    </div>
  );

  return (
    <div className="sim-columns">
      {/* Inputs */}
      <div className="sim-col">
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            marginBottom: 6,
          }}
        >
          <span className="vt-section__title" style={{ fontSize: "var(--fs-sm)" }}>Inputs (toggle)</span>
          {simulationInputs.size > 0 && (
            <Button
              size="sm"
              variant="ghost"
              icon={<RotateCcw size={12} strokeWidth={1.75} />}
              onClick={resetSimulationInputs}
            >
              Reset all ({simulationInputs.size})
            </Button>
          )}
        </div>
        {inputNames.length === 0 ? (
          <span style={{ color: "var(--text-muted)", fontSize: "var(--fs-xs)" }}>
            No X* input variables
          </span>
        ) : (
          Array.from(inputGroups.entries()).map(([prefix, names]) =>
            renderGroup(prefix, names, renderToggle),
          )
        )}
      </div>

      {/* Outputs */}
      <div className="sim-col">
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            marginBottom: 6,
          }}
        >
          <span className="vt-section__title" style={{ fontSize: "var(--fs-sm)" }}>Outputs (monitor)</span>
        </div>
        {outputNames.length === 0 ? (
          <span style={{ color: "var(--text-muted)", fontSize: "var(--fs-xs)" }}>
            No Y* output variables
          </span>
        ) : (
          Array.from(outputGroups.entries()).map(([prefix, names]) =>
            renderGroup(prefix, names, renderOutput),
          )
        )}
      </div>

      {/* Internal */}
      <div className="sim-col">
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            marginBottom: 6,
          }}
        >
          <span className="vt-section__title" style={{ fontSize: "var(--fs-sm)" }}>Internal variables</span>
        </div>
        {otherNames.length === 0 ? (
          <span style={{ color: "var(--text-muted)", fontSize: "var(--fs-xs)" }}>
            No internal variables
          </span>
        ) : (
          orderedOtherPrefixes.map((prefix) => {
            const names = otherGroups.get(prefix);
            return names ? renderGroup(prefix, names, renderVariable) : null;
          })
        )}
      </div>
    </div>
  );
}
