import { useRef, useEffect } from "react";
import type { LdElement, LdElementType } from "../../../types/ld";
import { isCoilType, getLdElementDef } from "../../../types/ld";

export const CELL_W = 110;
export const CELL_H = 48;

interface Props {
  element: LdElement;
  selected: boolean;
  editing: boolean;
  isOutputColumn?: boolean;
  onVariableChange: (v: string) => void;
  onSelect: () => void;
  onStartEdit: () => void;
  onFinishEdit: () => void;
}

function SvgSymbol({ type, blockType }: { type: LdElementType; blockType?: string }) {
  const c = "currentColor";
  if (type === "contact_no") {
    return <svg width={32} height={24} viewBox="0 0 32 24"><line x1="0" y1="12" x2="10" y2="12" stroke={c} strokeWidth="2" /><line x1="22" y1="12" x2="32" y2="12" stroke={c} strokeWidth="2" /><line x1="10" y1="3" x2="10" y2="21" stroke={c} strokeWidth="2.5" /><line x1="22" y1="3" x2="22" y2="21" stroke={c} strokeWidth="2.5" /></svg>;
  }
  if (type === "contact_nc") {
    return <svg width={32} height={24} viewBox="0 0 32 24"><line x1="0" y1="12" x2="10" y2="12" stroke={c} strokeWidth="2" /><line x1="22" y1="12" x2="32" y2="12" stroke={c} strokeWidth="2" /><line x1="10" y1="3" x2="10" y2="21" stroke={c} strokeWidth="2.5" /><line x1="22" y1="3" x2="22" y2="21" stroke={c} strokeWidth="2.5" /><line x1="10" y1="12" x2="22" y2="12" stroke={c} strokeWidth="2" /></svg>;
  }
  if (type === "contact_re") {
    return <svg width={32} height={24} viewBox="0 0 32 24"><line x1="0" y1="12" x2="10" y2="12" stroke={c} strokeWidth="2" /><line x1="22" y1="12" x2="32" y2="12" stroke={c} strokeWidth="2" /><line x1="10" y1="3" x2="10" y2="21" stroke={c} strokeWidth="2.5" /><line x1="22" y1="3" x2="22" y2="21" stroke={c} strokeWidth="2.5" /><text x="16" y="9" textAnchor="middle" fontSize="8" fill={c} fontWeight="bold">P</text></svg>;
  }
  if (type === "contact_fe") {
    return <svg width={32} height={24} viewBox="0 0 32 24"><line x1="0" y1="12" x2="10" y2="12" stroke={c} strokeWidth="2" /><line x1="22" y1="12" x2="32" y2="12" stroke={c} strokeWidth="2" /><line x1="10" y1="3" x2="10" y2="21" stroke={c} strokeWidth="2.5" /><line x1="22" y1="3" x2="22" y2="21" stroke={c} strokeWidth="2.5" /><text x="16" y="9" textAnchor="middle" fontSize="8" fill={c} fontWeight="bold">N</text></svg>;
  }
  if (type === "coil") {
    return <svg width={32} height={24} viewBox="0 0 32 24"><line x1="0" y1="12" x2="8" y2="12" stroke={c} strokeWidth="2" /><circle cx="16" cy="12" r="7" fill="none" stroke={c} strokeWidth="2" /><line x1="24" y1="12" x2="32" y2="12" stroke={c} strokeWidth="2" /></svg>;
  }
  if (type === "coil_set") {
    return <svg width={32} height={24} viewBox="0 0 32 24"><line x1="0" y1="12" x2="8" y2="12" stroke={c} strokeWidth="2" /><circle cx="16" cy="12" r="7" fill="none" stroke={c} strokeWidth="2" /><text x="16" y="15" textAnchor="middle" fontSize="9" fill={c} fontWeight="bold">S</text><line x1="24" y1="12" x2="32" y2="12" stroke={c} strokeWidth="2" /></svg>;
  }
  if (type === "coil_reset") {
    return <svg width={32} height={24} viewBox="0 0 32 24"><line x1="0" y1="12" x2="8" y2="12" stroke={c} strokeWidth="2" /><circle cx="16" cy="12" r="7" fill="none" stroke={c} strokeWidth="2" /><text x="16" y="15" textAnchor="middle" fontSize="9" fill={c} fontWeight="bold">R</text><line x1="24" y1="12" x2="32" y2="12" stroke={c} strokeWidth="2" /></svg>;
  }
  if (type === "coil_toggle") {
    return <svg width={32} height={24} viewBox="0 0 32 24"><line x1="0" y1="12" x2="8" y2="12" stroke={c} strokeWidth="2" /><circle cx="16" cy="12" r="7" fill="none" stroke={c} strokeWidth="2" /><text x="16" y="15" textAnchor="middle" fontSize="9" fill={c} fontWeight="bold">T</text><line x1="24" y1="12" x2="32" y2="12" stroke={c} strokeWidth="2" /></svg>;
  }
  if (type === "fb_block") {
    return <svg width={CELL_W * 2} height={CELL_H} viewBox={`0 0 ${CELL_W * 2} ${CELL_H}`}><rect x="4" y="2" width={CELL_W * 2 - 8} height={CELL_H - 4} fill="var(--bg)" stroke={c} strokeWidth="1.5" rx="3" /><text x={CELL_W} y="18" textAnchor="middle" fontSize="12" fill="var(--accent)" fontWeight="bold">{blockType || "TON"}</text><text x={CELL_W} y="34" textAnchor="middle" fontSize="9" fill="var(--text-muted)">instance</text><line x1="0" y1={CELL_H / 2} x2="4" y2={CELL_H / 2} stroke={c} strokeWidth="2" /><line x1={CELL_W * 2 - 4} y1={CELL_H / 2} x2={CELL_W * 2} y2={CELL_H / 2} stroke={c} strokeWidth="2" /></svg>;
  }
  return null;
}

export default function ElementCell({ element, selected, editing, isOutputColumn, onVariableChange, onSelect, onStartEdit, onFinishEdit }: Props) {
  const inputRef = useRef<HTMLInputElement>(null);
  const def = getLdElementDef(element.type, element.blockType);

  useEffect(() => {
    if (editing && inputRef.current) {
      inputRef.current.focus();
      inputRef.current.select();
    }
  }, [editing]);

  const isFb = element.type === "fb_block";

  return (
    <div
      onClick={onSelect}
      onDoubleClick={(e) => { e.stopPropagation(); onStartEdit(); }}
      style={{
        width: isFb ? CELL_W * 2 : CELL_W,
        height: CELL_H,
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        border: selected ? "2px solid var(--accent)" : "1px solid transparent",
        borderRadius: 4,
        cursor: "pointer",
        background: selected ? "rgba(22, 163, 74, 0.08)" : "var(--bg)",
        transition: "border-color 0.1s",
        flexShrink: 0,
        gap: 2,
      }}
    >
      <SvgSymbol type={element.type} blockType={element.blockType} />
      {editing ? (
        <input
          ref={inputRef}
          defaultValue={element.variable || element.instanceName || ""}
          onChange={(e) => onVariableChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === "Escape") {
              e.preventDefault();
              e.stopPropagation();
              onFinishEdit();
            }
          }}
          style={{
            width: 64,
            fontSize: 10,
            textAlign: "center",
            background: "var(--bg)",
            color: "var(--text)",
            border: "1px solid var(--accent)",
            borderRadius: 3,
            padding: "1px 4px",
            outline: "none",
            fontFamily: '"SF Mono", monospace',
          }}
        />
      ) : (
        <span style={{
          fontSize: 10,
          color: isCoilType(element.type) ? "var(--accent)" : "var(--text)",
          fontFamily: '"SF Mono", monospace',
          maxWidth: 90,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}>
          {element.variable || element.instanceName || def?.symbol || "\u2014"}
        </span>
      )}
    </div>
  );
}
