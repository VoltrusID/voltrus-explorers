import { useState } from "react";
import { openTextFile, saveTextFile, useProStatus } from "@mactools/ui";
import { useStore } from "./store";
import type { Device } from "./types";
import { uid, FREE_TAG_LIMIT } from "./helpers";
import { effectiveDataType, parseTagCsv, tagsToCsv, type TagCsvImport } from "./tagCsv";

/** Tag list editing of one device: the add/edit form state, CSV import and export. Shared by the desktop panel and the phone sheets. */
export function useTagsEditor(device: Device) {
  const setDeviceTags = useStore((st) => st.setDeviceTags);
  const addLog = useStore((st) => st.addLog);
  const [pendingImport, setPendingImport] = useState<(TagCsvImport & { path: string }) | null>(null);
  const [editingTagId, setEditingTagId] = useState<string | null>(null);
  const [newName, setNewName] = useState("");
  const [newAddr, setNewAddr] = useState("");
  const [newType, setNewType] = useState("DWord");
  const { pro, requirePro } = useProStatus();
  const tags = device.tags ?? [];

  const reset = () => { setEditingTagId(null); setNewName(""); setNewAddr(""); };

  /** Resolves true when the tag was added (false: the form is incomplete or the free tag limit asked for Pro). */
  const addTag = (): boolean => {
    if (!newName.trim() || !newAddr.trim()) return false;
    if (tags.length >= FREE_TAG_LIMIT && !requirePro("More than 10 live tags")) return false;
    setDeviceTags(device.id, (prev) => [
      ...prev,
      { id: uid(), name: newName.trim(), address: newAddr.trim(), data_type: effectiveDataType(newAddr, newType) },
    ]);
    setNewName(""); setNewAddr("");
    return true;
  };

  const saveTag = (): boolean => {
    if (!editingTagId || !newName.trim() || !newAddr.trim()) return false;
    setDeviceTags(device.id, (prev) =>
      prev.map((t) =>
        t.id === editingTagId ? { ...t, name: newName.trim(), address: newAddr.trim(), data_type: effectiveDataType(newAddr, newType) } : t,
      ),
    );
    reset();
    return true;
  };

  const editTag = (t: { id: string; name: string; address: string; data_type: string }) => {
    setEditingTagId(t.id);
    setNewName(t.name); setNewAddr(t.address); setNewType(t.data_type);
  };

  const removeTag = (id: string) => {
    setDeviceTags(device.id, (prev) => prev.filter((x) => x.id !== id));
  };

  const exportCsv = async () => {
    if (!requirePro("Tag list CSV export")) return;
    try {
      const path = await saveTextFile(`${device.label}-tags.csv`, tagsToCsv(tags));
      if (path) addLog("info", `[${device.label}] Exported ${tags.length} tags to ${path}`);
    } catch (e) {
      addLog("error", `[${device.label}] Tag export failed: ${String(e)}`);
    }
  };

  const chooseImportFile = async () => {
    if (!requirePro("Tag list CSV import")) return;
    try {
      const file = await openTextFile();
      if (!file) return;
      const parsed = parseTagCsv(file.text);
      parsed.errors.forEach((message) => addLog("error", `${file.path}: ${message}`));
      if (parsed.tags.length === 0) {
        addLog("error", `[${device.label}] No usable tags in ${file.path}`);
        return;
      }
      setPendingImport({ ...parsed, path: file.path });
    } catch (e) {
      addLog("error", `[${device.label}] Tag import failed: ${String(e)}`);
    }
  };

  const applyImport = (mode: "replace" | "add") => {
    if (!pendingImport) return;
    const incoming = pendingImport.tags.map((t) => ({ id: uid(), ...t }));
    let added = incoming.length;
    if (mode === "replace") {
      setDeviceTags(device.id, incoming);
    } else {
      const have = new Set(tags.map((t) => `${t.name}\u0000${t.address}`));
      const fresh = incoming.filter((t) => !have.has(`${t.name}\u0000${t.address}`));
      added = fresh.length;
      setDeviceTags(device.id, (prev) => [...prev, ...fresh]);
    }
    const skipped = pendingImport.errors.length;
    addLog(
      "info",
      `[${device.label}] Imported ${added} tags from ${pendingImport.path}${skipped > 0 ? ` (${skipped} row(s) skipped)` : ""}`,
    );
    pendingImport.notes.forEach((note) => addLog("info", note));
    setPendingImport(null);
  };

  return {
    tags, pro, requirePro,
    pendingImport, setPendingImport,
    editingTagId, newName, setNewName, newAddr, setNewAddr, newType, setNewType,
    reset, addTag, saveTag, editTag, removeTag, exportCsv, chooseImportFile, applyImport,
  };
}
