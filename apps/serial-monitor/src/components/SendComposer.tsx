import { SendHorizontal } from "lucide-react";
import { Button, Segmented } from "@mactools/ui";

interface SendComposerProps {
  format: "hex" | "ascii";
  onFormatChange: (format: "hex" | "ascii") => void;
  value: string;
  onValueChange: (value: string) => void;
  onSend: () => void;
  /** True while the current mode has no open port. */
  disabled: boolean;
}

/**
 * Terminal-style input bar shown under the traffic table: format toggle,
 * payload field, send action.
 */
export function SendComposer({
  format,
  onFormatChange,
  value,
  onValueChange,
  onSend,
  disabled,
}: SendComposerProps) {
  return (
    <div
      className="send-composer"
      style={{
        display: "flex",
        alignItems: "flex-end",
        flexWrap: "wrap",
        gap: 8,
        flexShrink: 0,
      }}
    >
      <Segmented
        label="Send format"
        size="sm"
        value={format}
        onChange={onFormatChange}
        options={[
          { value: "hex", label: "HEX" },
          { value: "ascii", label: "ASCII" },
        ]}
      />
      <textarea
        className="vt-textarea vt-textarea--mono"
        style={{ flex: 1, minWidth: 0, minHeight: 32, maxHeight: 120, resize: "none" }}
        placeholder={format === "hex" ? "01 03 00 00 00 0A C5 CD" : "Enter text..."}
        value={value}
        onChange={(e) => onValueChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            onSend();
          }
        }}
        spellCheck={false}
      />
      <Button
        variant="primary"
        size="sm"
        icon={<SendHorizontal size={14} strokeWidth={1.75} />}
        onClick={onSend}
        disabled={disabled || !value}
        style={{ flexShrink: 0 }}
      >
        Send
      </Button>
    </div>
  );
}
