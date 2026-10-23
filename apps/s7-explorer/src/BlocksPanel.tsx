import { Database, Download, RefreshCw, Trash2 } from "lucide-react";
import { Button, Callout, EmptyState, IconButton } from "@mactools/ui";
import { useBlocks, type BlockListEntry } from "./useBlocks";

interface Props {
  deviceId: string;
  connected: boolean;
}

export default function BlocksPanel({ deviceId, connected }: Props) {
  const { blocks, loading, error, uploading, refreshBlocks, handleUpload, deleteBlock } = useBlocks(deviceId, connected);

  const handleDelete = async (block: BlockListEntry) => {
    if (!confirm(`Delete ${block.block_type} ${block.block_number} from PLC?`)) return;
    await deleteBlock(block);
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
