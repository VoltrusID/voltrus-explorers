import { invoke } from "@tauri-apps/api/core";
import { decodeRegisters, numRegisters } from "../templates";
import { IconButton } from "@mactools/ui";
import { Lock, LockOpen } from "lucide-react";
import { thStyle } from "../constants";
import { useStore } from "../store";

interface Props {
  handleSlaveRegWrite: (address: number, value: number) => void;
}

export function SlaveTable({ handleSlaveRegWrite }: Props) {
  const { slaveRegs, activeProfile, lockedRegs, setLockedRegs, slavePage, setSlavePage } = useStore();

  const PAGE_SIZE = 50;
  const slaveViewStart = activeProfile ? activeProfile.baseAddress : slavePage * PAGE_SIZE;
  const slaveViewEnd = activeProfile
    ? activeProfile.baseAddress + numRegisters(activeProfile)
    : Math.min((slavePage + 1) * PAGE_SIZE, slaveRegs.length);
  const totalPages = activeProfile ? 1 : Math.ceil(slaveRegs.length / PAGE_SIZE);
  const visibleRegs = slaveRegs.slice(slaveViewStart, slaveViewEnd);

  return (
    <div style={{ flex: 1, display: "flex", flexDirection: "column", overflow: "hidden", background: "var(--card)", border: "1px solid var(--border)", borderRadius: "var(--radius-lg)" }}>
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", padding: "6px 12px", borderBottom: "1px solid var(--border)", flexShrink: 0 }}>
        <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>
          {activeProfile
            ? `Profile range: ${slaveViewStart}-${slaveViewEnd - 1} (${slaveViewEnd - slaveViewStart} regs)`
            : `Registers ${slaveViewStart}-${slaveViewEnd - 1} of ${slaveRegs.length}`}
        </span>
        {!activeProfile && totalPages > 1 && (
          <div style={{ display: "flex", gap: 4, alignItems: "center" }}>
            <button disabled={slavePage === 0} onClick={() => setSlavePage(slavePage - 1)} style={{ padding: "2px 8px", borderRadius: 4, border: "1px solid var(--border)", background: "var(--card)", color: slavePage === 0 ? "var(--text-muted)" : "var(--text)", cursor: slavePage === 0 ? "default" : "pointer", fontSize: "var(--fs-xs)" }}>Prev</button>
            <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)" }}>{slavePage + 1}/{totalPages}</span>
            <button disabled={slavePage >= totalPages - 1} onClick={() => setSlavePage(slavePage + 1)} style={{ padding: "2px 8px", borderRadius: 4, border: "1px solid var(--border)", background: "var(--card)", color: slavePage >= totalPages - 1 ? "var(--text-muted)" : "var(--text)", cursor: slavePage >= totalPages - 1 ? "default" : "pointer", fontSize: "var(--fs-xs)" }}>Next</button>
          </div>
        )}
      </div>
      <div style={{ flex: 1, overflow: "auto" }}>
        <table style={{ width: "100%", borderCollapse: "collapse", fontSize: "var(--fs-sm)", fontFamily: 'var(--font-mono)' }}>
          <thead>
            <tr>
              {activeProfile ? (
                <>
                  <th style={thStyle}>Name</th>
                  <th style={thStyle}>Addr</th>
                  <th style={thStyle}>Raw (Hex)</th>
                  <th style={thStyle}>Decimal</th>
                  <th style={thStyle}>Type</th>
                  <th style={{ ...thStyle, textAlign: "right" }}>Decoded</th>
                  <th style={thStyle}>Lock</th>
                </>
              ) : (
                <>
                  <th style={thStyle}>Addr</th>
                  <th style={thStyle}>Hex</th>
                  <th style={thStyle}>Decimal</th>
                </>
              )}
            </tr>
          </thead>
          <tbody>
            {activeProfile ? (() => {
              const decoded = decodeRegisters(slaveRegs.slice(activeProfile.baseAddress), activeProfile);
              return decoded.map((row, idx) => (
                <tr key={row.address} style={{ borderBottom: "1px solid var(--border)" }}>
                  <td style={{ padding: "5px 12px", color: "var(--text)", fontSize: "var(--fs-xs)", fontWeight: 500, whiteSpace: "nowrap" }}>{row.name}</td>
                  <td style={{ padding: "5px 12px", color: "var(--text-muted)", fontSize: 12 }}>{row.address}</td>
                  <td style={{ padding: "5px 12px", color: "var(--text-dim)", fontSize: 11 }}>{row.raw}</td>
                  <td style={{ padding: "5px 12px", color: "var(--text-dim)", fontSize: 11 }}>{row.decimals.join(", ")}</td>
                  <td style={{ padding: "5px 12px", color: "var(--text-muted)", fontSize: "var(--fs-xs)", whiteSpace: "nowrap" }}>{row.type}</td>
                  <td style={{ padding: "5px 12px", color: "var(--accent-text)", fontSize: "var(--fs-sm)", fontWeight: 600, textAlign: "right", whiteSpace: "nowrap" }}>{row.decoded}</td>
                  <td style={{ padding: "2px 6px", textAlign: "center" }}>
                      <IconButton
                        label={lockedRegs.has(idx) ? "Unlock register" : "Lock register"}
                        size="sm"
                        pressed={lockedRegs.has(idx)}
                        onClick={async () => {
                          const newLocked = !lockedRegs.has(idx);
                          try {
                            await invoke("sim_lock_register", { registerIndex: idx, locked: newLocked });
                            setLockedRegs((() => { const next = new Set(lockedRegs); newLocked ? next.add(idx) : next.delete(idx); return next; })());
                          } catch { /* ignore */ }
                        }}
                      >
                        {lockedRegs.has(idx) ? <Lock size={16} strokeWidth={1.75} /> : <LockOpen size={16} strokeWidth={1.75} />}
                      </IconButton>
                  </td>
                </tr>
              ));
            })() : visibleRegs.map((v, idx) => {
              const i = slaveViewStart + idx;
              return (
                <tr key={i} style={{ borderBottom: "1px solid var(--border)", background: idx % 2 === 0 ? "transparent" : "var(--hover-overlay)" }}>
                  <td style={{ textAlign: "right", padding: "5px 12px", color: "var(--text-muted)", fontSize: 12 }}>{i}</td>
                  <td style={{ padding: "5px 12px", color: "var(--text-dim)", fontSize: 12 }}>0x{v.toString(16).toUpperCase().padStart(4, "0")}</td>
                  <td style={{ padding: "5px 12px" }}>
                    <input
                      className="vt-input" style={{ padding: "3px 6px", width: "100%" }}
                      value={v}
                      onChange={(e) => {
                        const num = Number(e.target.value);
                        if (!isNaN(num) && num >= 0 && num <= 65535) {
                          handleSlaveRegWrite(i, num);
                        }
                      }}
                    />
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}
