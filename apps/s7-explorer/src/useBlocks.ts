import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { writeFile } from "@tauri-apps/plugin-fs";

export interface BlockListEntry {
  block_type: string;
  block_number: number;
}

/** The program blocks of one connected PLC: the list, saving a block to a file, deleting a block. */
export function useBlocks(deviceId: string, connected: boolean) {
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

  /** Asks where to save the block, then uploads it from the PLC and writes it to that file. Resolves true when a file was written. */
  const handleUpload = async (block: BlockListEntry): Promise<boolean> => {
    setUploading(block.block_number);
    try {
      const filePath = await save({
        defaultPath: `${block.block_type}_${block.block_number}.mc7`,
        filters: [{ name: "S7 Block", extensions: ["mc7", "blk", "s7"] }],
      });
      if (!filePath) return false;
      // The bytes go through the fs plugin, like every other export: on iOS the panel hands back a URL the
      // webview side is allowed to write, a plain std::fs::write from Rust is not.
      const data = await invoke<number[]>("upload_block", {
        deviceId,
        blockType: block.block_type.split("/")[0],
        blockNumber: block.block_number,
      });
      await writeFile(filePath, Uint8Array.from(data));
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    } finally {
      setUploading(null);
    }
  };

  /** Deletes the block from the PLC. The caller confirms first. Resolves true when it is gone. */
  const deleteBlock = async (block: BlockListEntry): Promise<boolean> => {
    try {
      await invoke("delete_plc_block", {
        deviceId,
        blockType: block.block_type.split("/")[0],
        blockNumber: block.block_number,
      });
      setBlocks((prev) => prev.filter((b) => b !== block));
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    }
  };

  return { blocks, loading, error, setError, uploading, refreshBlocks, handleUpload, deleteBlock };
}
