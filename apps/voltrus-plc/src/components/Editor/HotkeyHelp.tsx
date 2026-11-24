import { Button, Kbd } from "@mactools/ui";

export interface HotkeyEntry {
  key: string;
  label: string;
}

interface HotkeyHelpProps {
  title: string;
  entries: HotkeyEntry[];
  onClose: () => void;
}

// Shared shortcut cheat-sheet overlay for the graphical editors (FBD/SFC).
export default function HotkeyHelp({ title, entries, onClose }: HotkeyHelpProps) {
  return (
    <div className="ed-help" role="dialog" aria-label={title}>
      <div className="ed-help__title">{title}</div>
      <div className="ed-help__rows">
        {entries.map((h) => (
          <div key={h.key} className="ed-help__row">
            <span>{h.label}</span>
            <Kbd>{h.key}</Kbd>
          </div>
        ))}
      </div>
      <Button size="sm" block onClick={onClose}>
        Close
      </Button>
    </div>
  );
}
