import { useState } from "react";
import { Button } from "@mactools/ui";

export function SaveProfileAsInput({ onSave, existingNames }: { onSave: (name: string) => void; existingNames: string[] }) {
  const [name, setName] = useState("");
  const [show, setShow] = useState(false);
  if (!show) {
    return (
      <Button size="sm" variant="ghost" style={{ flex: 1 }} onClick={() => setShow(true)}>Save as profile…</Button>
    );
  }
  const valid = name.trim() && !existingNames.includes(name.trim());
  return (
    <div style={{ display: "flex", gap: 4, width: "100%" }}>
      <input
        className="vt-input"
        style={{ flex: 1 }}
        placeholder="Profile name"
        value={name}
        autoFocus
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => { if (e.key === "Enter" && valid) { onSave(name.trim()); setName(""); setShow(false); } if (e.key === "Escape") setShow(false); }}
      />
      <Button size="sm" variant="primary" disabled={!valid} onClick={() => { if (valid) { onSave(name.trim()); setName(""); setShow(false); } }}>Save</Button>
      <Button size="sm" variant="ghost" onClick={() => setShow(false)}>Cancel</Button>
    </div>
  );
}
