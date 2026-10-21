import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { Database, Download, RefreshCw, Trash2 } from "lucide-react";
import { Button, Callout, EmptyState, IconButton } from "@mactools/ui";

interface BlockListEntry {
  block_type: string;
  block_number: number;
}

interface Props {
  deviceId: string;
  connected: boolean;
}

export default function BlocksPanel({ deviceId, connected }: Props) {
  const [blocks, setBlocks] = useState<BlockListEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [uploading, setUploading] = useState<number | null>(null);

  const refreshBlocks = useCallback(async () => {
    if (!connected) return;
    setLoading(true);
    setError("");
    try {
      const result = await invoke<BlockListEntry[]>("list_blocks", { deviceId });
      setBlocks(result);
    } catch (e) {
      setError(String(e));
      setBlocks([]);
    } finally {
      setLoading(false);
    }
  }, [connected, deviceId]);

  useEffect(() => {
    if (connected) refreshBlocks();
    else {
      setBlocks([]);
      setError("");
    }
  }, [connected, refreshBlocks]);

  const handleUpload = async (block: BlockListEntry) => {
    setUploading(block.block_number);
    try {
      const filePath = await save({
        defaultPath: `${block.block_type}_${block.block_number}.mc7`,
        filters: [{ name: "S7 Block", extensions: ["mc7", "blk", "s7"] }],
      });
      if (!filePath) {
        setUploading(null);
        return;
      }
      await invoke("upload_block_to_file", {
        deviceId,
        blockType: block.block_type.split("/")[0],
        blockNumber: block.block_number,
        filePath,
      });
    } catch (e) {
      setError(String(e));
    } finally {
      setUploading(null);
    }
  };

  const handleDelete = async (block: BlockListEntry) => {
    if (!confirm(`Delete ${block.block_type} ${block.block_number} from PLC?`)) return;
    try {
      await invoke("delete_plc_block", {
        deviceId,
        blockType: block.block_type.split("/")[0],
        blockNumber: block.block_number,
      });
      setBlocks((prev) => prev.filter((b) => b !== block));
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div style={{ padding: 12, overflow: "auto", height: "100%", display: "flex", flexDirection: "column", gap: 12 }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", flexShrink: 0 }}>
        <span style={{ fontSize: "var(--fs-sm)", fontWeight: 600, color: "var(--text)" }}>
          Block manager {blocks.length > 0 && `(${blocks.length} blocks)`}
        </span>
        <Button
          size="sm"
          variant="ghost"
          icon={<RefreshCw size={14} strokeWidth={1.75} style={loading ? { animation: "vt-spin 700ms linear infinite" } : undefined} />}
          disabled={!connected || loading}
          onClick={refreshBlocks}
        >
          {loading ? "Loading…" : "Refresh"}
        </Button>
      </div>

      {error && <Callout tone="danger">{error}</Callout>}

      {!connected && (
        <EmptyState
          icon={<Database size={18} strokeWidth={1.75} />}
          title="Not connected"
          description="Connect to a PLC to manage blocks."
        />
      )}

      {connected && blocks.length === 0 && !loading && (
        <EmptyState
          compact
          icon={<Database size={18} strokeWidth={1.75} />}
          title="No blocks found"
          description="Click Refresh to scan the PLC."
        />
      )}

      {blocks.length > 0 && (
        <div style={{ overflowX: "auto", flexShrink: 0 }}>
          <table className="vt-table">
            <thead>
              <tr>
                <th style={{ width: 70 }}>Type</th>
                <th style={{ width: 80 }}>Number</th>
                <th style={{ width: 96, textAlign: "right" }}>Actions</th>
              </tr>
            </thead>
            <tbody>
              {blocks.map((block, i) => (
                <tr key={`${block.block_type}-${block.block_number}-${i}`}>
                  <td className="mono" style={{ color: "var(--accent-text)", fontWeight: 600 }}>{block.block_type}</td>
                  <td className="mono">{block.block_number}</td>
                  <td style={{ textAlign: "right" }}>
                    <div style={{ display: "inline-flex", gap: 4 }}>
                      <IconButton
                        size="sm"
                        outline
                        label={`Save ${block.block_type} ${block.block_number} to file`}
                        disabled={uploading !== null}
                        onClick={() => handleUpload(block)}
                      >
                        <Download size={14} strokeWidth={1.75} />
                      </IconButton>
                      <IconButton
                        size="sm"
                        danger
                        label={`Delete ${block.block_type} ${block.block_number} from PLC`}
                        disabled={uploading !== null}
                        onClick={() => handleDelete(block)}
                      >
                        <Trash2 size={14} strokeWidth={1.75} />
                      </IconButton>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
