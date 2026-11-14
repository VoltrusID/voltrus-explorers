import { Badge, Button, EmptyState, ProBadge, pickSavePath } from "@mactools/ui";
import { invoke } from "@tauri-apps/api/core";
import { Download, Radio, Trash2 } from "lucide-react";
import { hexBytes, pcap } from "./ecat";
import type { FrameView } from "./types";
import { type AddLog, type RequirePro, dim, hint, row, scroller, th } from "./ui";

interface Props {
  frames: FrameView[];
  onClear: () => void;
  simulator: boolean;
  startedAt: number;
  pro: boolean;
  requirePro: RequirePro;
  addLog: AddLog;
}

export default function FramesView({ frames, onClear, simulator, startedAt, pro, requirePro, addLog }: Props) {
  const capture = async (n: number) => {
    try { await invoke("ecat_capture", { frames: n }); } catch (e) { addLog("error", String(e)); }
  };
  const save = async () => {
    if (!requirePro("pcap export")) return;
    const path = await pickSavePath("ethercat-capture.pcap", { name: "pcap", extensions: ["pcap"] });
    if (!path) return;
    try {
      await invoke("save_bytes", { path, bytes: Array.from(pcap(frames, startedAt)) });
      addLog("info", `${frames.length} frames saved to ${path}; Wireshark decodes them`);
    } catch (e) {
      addLog("error", String(e));
    }
  };

  if (!simulator) {
    return (
      <EmptyState
        icon={<Radio size={20} strokeWidth={1.75} />}
        title="Frame capture on a real network"
        description="EtherCAT frames on a network interface are best captured with Wireshark on the same adapter: it decodes every datagram and the CoE mailbox. The app's capture works with the simulator."
      />
    );
  }
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8, flex: 1, minHeight: 0 }}>
      <div style={row}>
        <Button size="sm" variant="primary" onClick={() => void capture(100)}>Capture 100 frames</Button>
        <Button size="sm" onClick={() => void capture(1000)}>Capture 1000</Button>
        <Badge>{frames.length} frames</Badge>
        <span style={{ flex: 1 }} />
        <Button size="sm" icon={<Download size={14} strokeWidth={1.75} />} disabled={!frames.length} onClick={() => void save()}>Save .pcap{!pro && <ProBadge />}</Button>
        <Button size="sm" variant="ghost" icon={<Trash2 size={14} strokeWidth={1.75} />} onClick={onClear}>Clear</Button>
      </div>
      {frames.length === 0 ? (
        <EmptyState
          title="No frames captured"
          description="Captures the next frames between the master and the simulated devices, both ways: the cyclic LRW, state polls, mailbox traffic when you browse objects."
        />
      ) : (
        <div style={scroller}>
          <table className="vt-table" style={{ width: "100%" }}>
            <thead>
              <tr>
                <th style={{ ...th, width: 90 }}>Time</th>
                <th style={{ ...th, width: 54 }} />
                <th style={th}>Datagrams</th>
                <th style={{ ...th, width: "34%" }}>Bytes</th>
              </tr>
            </thead>
            <tbody>
              {frames.map((f, i) => (
                <tr key={i}>
                  <td className="mono">{(f.tsUs / 1000).toFixed(3)} ms</td>
                  <td><Badge tone={f.out ? "info" : "success"}>{f.out ? "out" : "back"}</Badge></td>
                  <td className="mono" style={{ fontSize: "var(--fs-xs)" }}>{f.datagrams.map((d, j) => <div key={j}>{d}</div>)}</td>
                  <td className="mono" style={{ ...dim, fontSize: "var(--fs-xs)", wordBreak: "break-all" }} title={hexBytes(f.bytes)}>
                    {hexBytes(f.bytes.slice(14, 46))}{f.bytes.length > 46 ? " …" : ""}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      <span style={hint}>Frames going out have the master's address; frames coming back carry the working counter each device added.</span>
    </div>
  );
}
