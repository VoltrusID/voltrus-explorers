import { Copy, Plug, Unplug } from "lucide-react";
import { Button, Callout, IconButton, Section, StatusDot } from "@mactools/ui";

interface VirtualPortSidebarProps {
  virtualActive: boolean;
  virtualPortPath: string | null;
  handleCreateVirtualPort: () => void;
  handleDestroyVirtualPort: () => void;
  handleCopyPath: () => void;
}

export function VirtualPortSidebar({
  virtualActive,
  virtualPortPath,
  handleCreateVirtualPort,
  handleDestroyVirtualPort,
  handleCopyPath,
}: VirtualPortSidebarProps) {
  return (
    <Section title="Virtual port">
      <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-dim)", lineHeight: 1.5 }}>
        Create a real virtual serial port (PTY) that other devices and apps can connect to.
        Use it as a loopback test or bridge.
      </div>

      {virtualActive ? (
        <>
          <Callout tone="success">
            <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
              <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                <StatusDot tone="success" />
                <span style={{ fontSize: "var(--fs-xs)", fontWeight: 600 }}>Virtual port active</span>
              </div>
              <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                <code
                  className="mono"
                  style={{
                    fontSize: "var(--fs-sm)",
                    flex: 1,
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                  }}
                >
                  {virtualPortPath}
                </code>
                <IconButton label="Copy path" size="sm" onClick={handleCopyPath}>
                  <Copy size={14} strokeWidth={1.75} />
                </IconButton>
              </div>
            </div>
          </Callout>

          <div style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)", lineHeight: 1.45 }}>
            Connect to this path from Monitor mode or external tools (e.g.{" "}
            <code className="mono">screen {virtualPortPath}</code>)
          </div>

          <Button
            block
            variant="danger"
            icon={<Unplug size={14} strokeWidth={1.75} />}
            onClick={handleDestroyVirtualPort}
          >
            Destroy virtual port
          </Button>
        </>
      ) : (
        <Button
          block
          variant="primary"
          icon={<Plug size={14} strokeWidth={1.75} />}
          onClick={handleCreateVirtualPort}
        >
          Create virtual port
        </Button>
      )}
    </Section>
  );
}
