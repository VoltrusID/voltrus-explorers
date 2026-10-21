import { Pencil } from "lucide-react";
import { Badge, EmptyState, IconButton } from "@mactools/ui";
import type { BacnetPropertyEntry, BacnetValue } from "../types";
import { bacnetValueToString, formatObjectType } from "../types";

interface PropertyInspectorProps {
  objectType: string;
  objectInstance: number;
  properties: BacnetPropertyEntry[];
  loading: boolean;
  onWrite?: (propertyName: string, value: string, dataType: string) => void;
  unitName?: string | null;
  unitSymbol?: string | null;
}

function formatPropertyName(name: string): string {
  return name
    .replace(/([a-z])([A-Z])/g, "$1 $2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1 $2")
    .replace(/^./, (s) => s.toUpperCase());
}

function getDataType(v: BacnetValue): string {
  if ("Real" in v) return "Real";
  if ("Double" in v) return "Double";
  if ("Unsigned" in v) return "Unsigned";
  if ("Boolean" in v) return "Boolean";
  if ("Enum" in v) return "Enum";
  if ("Float64" in v) return "Float64";
  if ("String" in v) return "String";
  if ("Null" in v) return "Null";
  return "Null";
}

function isWritable(
  objectType: string,
  propertyName: string
): boolean {
  if (propertyName !== "presentValue") return false;
  const WRITABLE_OBJECT_TYPES: Record<string, true> = {
    analogOutput: true,
    analogValue: true,
    binaryOutput: true,
    binaryValue: true,
    multiStateOutput: true,
    multiStateValue: true,
  };
  return WRITABLE_OBJECT_TYPES[objectType] === true;
}

export function PropertyInspector({
  objectType,
  objectInstance,
  properties,
  loading,
  onWrite,
  unitName,
  unitSymbol,
}: PropertyInspectorProps) {
  const label = formatObjectType(objectType);

  return (
    <div style={{ display: "flex", flexDirection: "column", minHeight: 0 }}>
      <div style={{ display: "flex", alignItems: "baseline", justifyContent: "space-between", padding: "2px 2px 10px" }}>
        <span style={{ fontSize: "var(--fs-md)", fontWeight: 600, color: "var(--text)" }}>
          {label} #{objectInstance}
        </span>
        <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)", fontVariantNumeric: "tabular-nums" }}>
          {properties.length} propert{properties.length === 1 ? "y" : "ies"}
        </span>
      </div>

      {loading && (
        <div style={{ display: "flex", alignItems: "center", justifyContent: "center", gap: 8, padding: 32, fontSize: "var(--fs-sm)", color: "var(--text-muted)" }}>
          <span className="vt-spinner" aria-hidden="true" />
          Loading properties…
        </div>
      )}

      {!loading && properties.length === 0 && (
        <EmptyState
          compact
          title="No properties"
          description="No properties were returned for this object."
        />
      )}

      {!loading && properties.length > 0 && (
        <div style={{ overflowY: "auto", minHeight: 0, flex: 1, background: "var(--card)", border: "1px solid var(--border)", borderRadius: "var(--radius-lg)" }}>
          <table className="vt-table">
            <tbody>
              {properties.map((entry, index) => {
                const writable = isWritable(objectType, entry.property_name);
                const value = entry.value;
                const hasValue = value && typeof value === "object";
                const isBool = hasValue && "Boolean" in value;
                const isEnum = hasValue && "Enum" in value;
                const isMultiStateEnum =
                  isEnum &&
                  entry.property_name === "presentValue" &&
                  objectType.startsWith("multiState");

                let displayValue: string;
                let tone: "success" | "neutral" | undefined;

                if (!hasValue) {
                  displayValue = "—";
                } else if (isBool) {
                  displayValue = value.Boolean ? "Active" : "Inactive";
                  tone = value.Boolean ? "success" : "neutral";
                } else if (isMultiStateEnum) {
                  displayValue = `State ${value.Enum}`;
                } else {
                  displayValue = bacnetValueToString(value);
                }

                const isPresentValue = entry.property_identifier === 85;
                const isUnitsRow = entry.property_identifier === 90;
                if (isUnitsRow && unitName) displayValue = unitName;
                if (isPresentValue && unitSymbol) displayValue = `${displayValue} ${unitSymbol}`;

                return (
                  <tr
                    key={`${entry.property_identifier}-${index}`}
                    className="property-inspector-row"
                  >
                    <td
                      style={{
                        width: "45%",
                        fontSize: "var(--fs-xs)",
                        fontWeight: 500,
                        color: "var(--text-muted)",
                        whiteSpace: "nowrap",
                        overflow: "hidden",
                        textOverflow: "ellipsis",
                        verticalAlign: "middle",
                      }}
                    >
                      {formatPropertyName(entry.property_name)}
                    </td>
                    <td
                      className="is-mono"
                      style={{ position: "relative", verticalAlign: "middle", paddingRight: writable && onWrite && hasValue ? 36 : undefined }}
                    >
                      {isBool ? <Badge tone={tone}>{displayValue}</Badge> : <span>{displayValue}</span>}
                      {writable && onWrite && hasValue && (
                        <IconButton
                          size="sm"
                          label={`Write ${entry.property_name}`}
                          className="property-inspector-edit"
                          style={{ position: "absolute", right: 6, top: "50%", transform: "translateY(-50%)" }}
                          onClick={() =>
                            onWrite(
                              entry.property_name,
                              bacnetValueToString(value),
                              getDataType(value)
                            )
                          }
                        >
                          <Pencil size={12} strokeWidth={1.75} />
                        </IconButton>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
