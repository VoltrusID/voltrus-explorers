import React from "react";
import { useStore } from "../../store";

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
  M: "Internal Relays",
  D: "Data Registers",
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
      <div style={{ color: "var(--text-muted)", padding: "8px 0" }}>
        Compile and run the program to enable simulation.
      </div>
    );
  }

  if (compiledVariables.length === 0) {
    return (
      <div style={{ color: "var(--text-muted)", padding: "8px 0" }}>
        No compiled variables. Compile the program first.
      </div>
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

  const renderToggle = (name: string) => {
    const forced = simulationInputs.has(name);
    const value = forced ? simulationInputs.get(name) : watchMap.get(name)?.value;
    const isOn = value === true || value === 1 || value === "TRUE" || value === "1";

    return (
      <div
        key={name}
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          padding: "3px 0",
          fontSize: 11,
          fontFamily: "'SF Mono', 'Fira Code', monospace",
        }}
      >
        <button
          onClick={() => handleToggle(name)}
          style={{
            width: 32,
            height: 18,
            borderRadius: 9,
            border: "none",
            cursor: "pointer",
            background: isOn ? "var(--accent)" : "var(--border)",
            position: "relative",
            transition: "background 0.15s",
            flexShrink: 0,
          }}
          title={forced ? "Forced — click to unforce" : "Click to force toggle"}
        >
          <div
            style={{
              width: 14,
              height: 14,
              borderRadius: 7,
              background: "#fff",
              position: "absolute",
              top: 2,
              left: isOn ? 16 : 2,
              transition: "left 0.15s",
            }}
          />
        </button>
        <span style={{ color: forced ? "var(--accent)" : "var(--text-muted)", minWidth: 60 }}>
          {name}
        </span>
        <span style={{ color: isOn ? "#10b981" : "var(--text-muted)" }}>
          {isOn ? "ON" : "OFF"}
        </span>
        {forced && (
          <span style={{ fontSize: 9, color: "#f59e0b" }}>FORCED</span>
        )}
      </div>
    );
  };

  const renderOutput = (name: string) => {
    const w = watchMap.get(name);
    const isOn = w?.value === true || w?.value === 1 || w?.display_value === "TRUE" || w?.display_value === "1";
    const quality = getQuality(name);

    return (
      <div
        key={name}
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          padding: "3px 0",
          fontSize: 11,
          fontFamily: "'SF Mono', 'Fira Code', monospace",
        }}
      >
        <div
          style={{
            width: 14,
            height: 14,
            borderRadius: 3,
            background: isOn ? "#10b981" : "var(--border)",
            boxShadow: isOn ? "0 0 6px #10b98180" : "none",
            flexShrink: 0,
            transition: "background 0.15s",
          }}
        />
        <span style={{ color: "var(--text-muted)", minWidth: 60 }}>{name}</span>
        <span style={{ color: isOn ? "#10b981" : "var(--text-muted)" }}>
          {getDisplayValue(name)}
        </span>
        {quality === "forced" && (
          <span style={{ fontSize: 9, color: "#f59e0b" }}>FORCED</span>
        )}
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
        <div
          key={name}
          style={{
            display: "flex",
            alignItems: "center",
            gap: 8,
            padding: "3px 0",
            fontSize: 11,
            fontFamily: "'SF Mono', 'Fira Code', monospace",
          }}
        >
          <div
            style={{
              width: 10,
              height: 10,
              borderRadius: 2,
              background: isOn ? "#10b981" : "var(--border)",
              flexShrink: 0,
            }}
          />
          <span style={{ color: "var(--text-muted)", minWidth: 60 }}>{name}</span>
          <span>{getDisplayValue(name)}</span>
          {quality === "forced" && (
            <span style={{ fontSize: 9, color: "#f59e0b" }}>FORCED</span>
          )}
        </div>
      );
    }

    return (
      <div
        key={name}
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          padding: "3px 0",
          fontSize: 11,
          fontFamily: "'SF Mono', 'Fira Code', monospace",
        }}
      >
        <span style={{ color: "var(--text-muted)", minWidth: 60 }}>{name}</span>
        <span style={{ color: "var(--text)" }}>{getDisplayValue(name)}</span>
        {quality === "forced" && (
          <span style={{ fontSize: 9, color: "#f59e0b" }}>FORCED</span>
        )}
      </div>
    );
  };

  const renderGroup = (
    prefix: string,
    names: string[],
    renderer: (name: string) => React.ReactNode,
  ) => (
    <div key={prefix} style={{ marginBottom: 12 }}>
      <div
        style={{
          fontSize: 10,
          fontWeight: 600,
          color: "var(--text-muted)",
          textTransform: "uppercase",
          letterSpacing: 1,
          marginBottom: 4,
        }}
      >
        {PREFIX_LABELS[prefix] ?? prefix} ({names.length})
      </div>
      {names.map(renderer)}
    </div>
  );

  return (
    <div style={{ display: "flex", gap: 24, padding: "4px 0" }}>
      {/* Inputs */}
      <div style={{ flex: 1, minWidth: 0 }}>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            marginBottom: 6,
          }}
        >
          <span
            style={{
              fontSize: 11,
              fontWeight: 600,
              color: "var(--accent)",
            }}
          >
            Inputs (Toggle)
          </span>
          {simulationInputs.size > 0 && (
            <button
              className="toolbar-btn"
              style={{ fontSize: 10, padding: "2px 8px" }}
              onClick={resetSimulationInputs}
            >
              Reset All ({simulationInputs.size})
            </button>
          )}
        </div>
        {inputNames.length === 0 ? (
          <span style={{ color: "var(--text-muted)", fontSize: 11 }}>
            No X* input variables
          </span>
        ) : (
          Array.from(inputGroups.entries()).map(([prefix, names]) =>
            renderGroup(prefix, names, renderToggle),
          )
        )}
      </div>

      {/* Outputs */}
      <div style={{ flex: 1, minWidth: 0 }}>
        <div
          style={{
            fontSize: 11,
            fontWeight: 600,
            color: "#10b981",
            marginBottom: 6,
          }}
        >
          Outputs (Monitor)
        </div>
        {outputNames.length === 0 ? (
          <span style={{ color: "var(--text-muted)", fontSize: 11 }}>
            No Y* output variables
          </span>
        ) : (
          Array.from(outputGroups.entries()).map(([prefix, names]) =>
            renderGroup(prefix, names, renderOutput),
          )
        )}
      </div>

      {/* Internal */}
      <div style={{ flex: 1, minWidth: 0 }}>
        <div
          style={{
            fontSize: 11,
            fontWeight: 600,
            color: "var(--text-muted)",
            marginBottom: 6,
          }}
        >
          Internal Variables
        </div>
        {otherNames.length === 0 ? (
          <span style={{ color: "var(--text-muted)", fontSize: 11 }}>
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
