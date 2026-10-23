import { useState } from "react";
import { Trash2 } from "lucide-react";
import { Button, Field, IconButton, ProBadge, Section, SheetSelect, useProStatus } from "@mactools/ui";
import { FREE_PROFILE_LIMIT, upsertByName, type PortProfile } from "../profiles";

interface Props {
  profiles: PortProfile[];
  setProfiles: (list: PortProfile[]) => void;
  current: Omit<PortProfile, "name">;
  onApply: (p: PortProfile) => void;
  disabled: boolean;
}

export function ProfilesSection({ profiles, setProfiles, current, onApply, disabled }: Props) {
  const { pro, requirePro } = useProStatus();
  const [name, setName] = useState("");
  const [selected, setSelected] = useState("");

  const save = () => {
    const trimmed = name.trim();
    if (!trimmed) return;
    const replacing = profiles.some((p) => p.name === trimmed);
    if (!replacing && profiles.length >= FREE_PROFILE_LIMIT && !requirePro("More than one saved port profile")) return;
    setProfiles(upsertByName(profiles, { name: trimmed, ...current }));
    setSelected(trimmed);
    setName("");
  };

  return (
    <Section title="Saved settings" meta={!pro && profiles.length >= FREE_PROFILE_LIMIT ? <ProBadge /> : undefined}>
      {profiles.length > 0 && (
        <Field label="Profile">
          {(id) => (
            <div style={{ display: "flex", gap: 6 }}>
              <SheetSelect
                id={id}
                sheetTitle="Profile"
                value={selected}
                disabled={disabled}
                options={profiles.map((p) => ({ value: p.name, label: p.name }))}
                placeholder="Select profile..."
                onChange={(v) => {
                  setSelected(v);
                  const p = profiles.find((x) => x.name === v);
                  if (p) onApply(p);
                }}
              />
              <IconButton
                label="Delete profile"
                disabled={!selected}
                onClick={() => {
                  setProfiles(profiles.filter((p) => p.name !== selected));
                  setSelected("");
                }}
              >
                <Trash2 size={16} strokeWidth={1.75} />
              </IconButton>
            </div>
          )}
        </Field>
      )}
      <Field label="Save current settings as">
        {(id) => (
          <div style={{ display: "flex", gap: 6 }}>
            <input
              id={id}
              className="vt-input"
              placeholder="Profile name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") save(); }}
            />
            <Button size="sm" disabled={!name.trim()} onClick={save}>Save</Button>
          </div>
        )}
      </Field>
    </Section>
  );
}
